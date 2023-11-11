use embedded_io::{Error as _, ErrorType};
use embedded_io_async::{BufRead, Read};
use heapless::Vec;

use crate::headers::{ContentType, KeepAlive, TransferEncoding};
use crate::reader::BufferingReader;
use crate::request::Method;
use crate::Error;

/// Type representing a parsed HTTP response.
#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Response<'resp, 'buf, C>
where
    C: Read,
{
    conn: &'resp mut C,
    /// The method used to create the response.
    method: Method,
    /// The HTTP response status code.
    pub status: Status,
    /// The HTTP response content type.
    pub content_type: Option<ContentType>,
    /// The content length.
    pub content_length: Option<usize>,
    /// The transfer encoding.
    pub transfer_encoding: heapless::Vec<TransferEncoding, 4>,
    /// The keep-alive parameters.
    pub keep_alive: Option<KeepAlive>,
    header_buf: &'buf mut [u8],
    header_len: usize,
    raw_body_read: usize,
}

impl<'resp, 'buf, C> Response<'resp, 'buf, C>
where
    C: Read,
{
    // Read at least the headers from the connection.
    pub async fn read(conn: &'resp mut C, method: Method, header_buf: &'buf mut [u8]) -> Result<Self, Error> {
        let mut header_len = 0;
        let mut pos = 0;
        while pos < header_buf.len() {
            let n = conn.read(&mut header_buf[pos..]).await.map_err(|e| {
                /*warn!(
                    "error {:?}, but read data from socket:  {:?}",
                    defmt::Debug2Format(&e),
                    defmt::Debug2Format(&core::str::from_utf8(&buf[..pos])),
                );*/
                e.kind()
            })?;

            if n == 0 {
                return Err(Error::ConnectionAborted);
            }

            pos += n;

            // Look for header end
            let mut headers = [httparse::EMPTY_HEADER; 64];
            let mut response = httparse::Response::new(&mut headers);
            let parse_status = response.parse(&header_buf[..pos]).map_err(|_| Error::Codec)?;
            if parse_status.is_complete() {
                header_len = parse_status.unwrap();
                break;
            }
        }

        if header_len == 0 {
            // Unable to completely read header
            return Err(Error::BufferTooSmall);
        }

        // Parse status and known headers
        let mut headers = [httparse::EMPTY_HEADER; 64];
        let mut response = httparse::Response::new(&mut headers);
        response.parse(&header_buf[..header_len]).unwrap();

        let status = response.code.unwrap().into();
        let mut content_type = None;
        let mut content_length = None;
        let mut transfer_encoding = Vec::new();
        let mut keep_alive = None;

        for header in response.headers {
            if header.name.eq_ignore_ascii_case("content-type") {
                content_type.replace(header.value.into());
            } else if header.name.eq_ignore_ascii_case("content-length") {
                content_length = Some(
                    core::str::from_utf8(header.value)
                        .map_err(|_| Error::Codec)?
                        .parse::<usize>()
                        .map_err(|_| Error::Codec)?,
                );
            } else if header.name.eq_ignore_ascii_case("transfer-encoding") {
                transfer_encoding
                    .push(header.value.try_into().map_err(|_| Error::Codec)?)
                    .map_err(|_| Error::Codec)?;
            } else if header.name.eq_ignore_ascii_case("keep-alive") {
                keep_alive.replace(header.value.try_into().map_err(|_| Error::Codec)?);
            }
        }

        // The number of bytes that we have read into the body part of the response
        let raw_body_read = pos - header_len;

        if let Some(content_length) = content_length {
            if content_length < raw_body_read {
                // We have more into the body then what is specified in content_length
                return Err(Error::Codec);
            }
        }

        Ok(Response {
            conn,
            method,
            status,
            content_type,
            content_length,
            transfer_encoding,
            keep_alive,
            header_buf,
            header_len,
            raw_body_read,
        })
    }

    /// Get the response headers
    pub fn headers(&self) -> HeaderIterator {
        let mut iterator = HeaderIterator(0, [httparse::EMPTY_HEADER; 64]);
        let mut response = httparse::Response::new(&mut iterator.1);
        response.parse(&self.header_buf[..self.header_len]).unwrap();

        iterator
    }

    /// Get the response body
    pub fn body(self) -> ResponseBody<'resp, 'buf, C> {
        let reader_hint = if self.method == Method::HEAD {
            // Head requests does not have a body so we return an empty reader
            ReaderHint::Empty
        } else if let Some(content_length) = self.content_length {
            ReaderHint::FixedLength(content_length)
        } else if self.transfer_encoding.contains(&TransferEncoding::Chunked) {
            ReaderHint::Chunked
        } else {
            ReaderHint::ToEnd
        };

        // Move the body part of the bytes in the header buffer to the beginning of the buffer.
        self.header_buf
            .copy_within(self.header_len..self.header_len + self.raw_body_read, 0);

        ResponseBody {
            conn: self.conn,
            reader_hint,
            body_buf: self.header_buf,
            raw_body_read: self.raw_body_read,
        }
    }
}

pub struct HeaderIterator<'a>(usize, [httparse::Header<'a>; 64]);

impl<'a> Iterator for HeaderIterator<'a> {
    type Item = (&'a str, &'a [u8]);

    fn next(&mut self) -> Option<Self::Item> {
        let result = self.1.get(self.0);

        self.0 += 1;

        result.map(|h| (h.name, h.value))
    }
}

/// Response body
///
/// This type contains the original header buffer provided to `read_headers`,
/// now renamed to `body_buf`, the number of read body bytes that are available
/// in `body_buf`, and a reader to be used for reading the remaining body.
pub struct ResponseBody<'resp, 'buf, C>
where
    C: Read,
{
    conn: &'resp mut C,
    reader_hint: ReaderHint,
    /// The number of raw bytes read from the body and available in the beginning of `body_buf`.
    raw_body_read: usize,
    /// The buffer initially provided to read the header.
    pub body_buf: &'buf mut [u8],
}

enum ReaderHint {
    Empty,
    FixedLength(usize),
    Chunked,
    ToEnd, // https://www.rfc-editor.org/rfc/rfc7230#section-3.3.3 pt. 7: Until end of connection
}

impl<'resp, 'buf, C> ResponseBody<'resp, 'buf, C>
where
    C: Read,
{
    pub fn reader(self) -> BodyReader<BufferingReader<'resp, 'buf, C>> {
        let raw_body = BufferingReader::new(self.body_buf, self.raw_body_read, self.conn);

        match self.reader_hint {
            ReaderHint::Empty => BodyReader::Empty,
            ReaderHint::FixedLength(content_length) => BodyReader::FixedLength(FixedLengthBodyReader {
                raw_body,
                remaining: content_length,
            }),
            ReaderHint::Chunked => BodyReader::Chunked(ChunkedBodyReader {
                raw_body,
                chunk_remaining: ChunkState::NoChunk,
            }),
            ReaderHint::ToEnd => BodyReader::ToEnd(raw_body),
        }
    }
}

impl<'resp, 'buf, C> ResponseBody<'resp, 'buf, C>
where
    C: Read,
{
    /// Read the entire body into the buffer originally provided [`Response::read()`].
    /// This requires that this original buffer is large enough to contain the entire body.
    ///
    /// This is not valid for chunked responses as it requires that the body bytes over-read
    /// while parsing the http response header would be available for the body reader.
    /// For this case, or if the original buffer is not large enough, use
    /// [`BodyReader::read_to_end()`] instead from the reader returned by [`ResponseBody::reader()`].
    pub async fn read_to_end(self) -> Result<&'buf mut [u8], Error> {
        // We can only read responses with Content-Length header to end using the body_buf buffer,
        // as any other response would require the body reader to know the entire body.
        match self.reader_hint {
            ReaderHint::Empty => Ok(&mut []),
            ReaderHint::FixedLength(content_length) => {
                // Read into the buffer after the portion that was already received when parsing the header
                self.conn
                    .read_exact(&mut self.body_buf[self.raw_body_read..content_length])
                    .await?;

                Ok(&mut self.body_buf[..content_length])
            }
            ReaderHint::Chunked => Err(Error::Codec),
            ReaderHint::ToEnd => {
                let mut body_len = self.raw_body_read;
                loop {
                    let len = self
                        .conn
                        .read(&mut self.body_buf[body_len..])
                        .await
                        .map_err(|e| e.kind())?;
                    if len == 0 {
                        break;
                    }
                    body_len += len;
                }

                Ok(&mut self.body_buf[..body_len])
            }
        }
    }

    /// Discard the entire body
    ///
    /// Returns the number of discarded body bytes
    pub async fn discard(self) -> Result<usize, Error> {
        self.reader().discard().await
    }
}

/// A body reader
pub enum BodyReader<B> {
    Empty,
    FixedLength(FixedLengthBodyReader<B>),
    Chunked(ChunkedBodyReader<B>),
    ToEnd(B),
}

impl<B> BodyReader<B>
where
    B: Read,
{
    /// Read the entire body
    pub async fn read_to_end(&mut self, buf: &mut [u8]) -> Result<usize, Error> {
        let mut len = 0;
        while len < buf.len() {
            match self.read(&mut buf[len..]).await {
                Ok(0) => break,
                Ok(n) => len += n,
                Err(e) => return Err(e),
            }
        }

        let is_done = match self {
            BodyReader::Empty => true,
            BodyReader::FixedLength(reader) => {
                if reader.remaining > 0 {
                    warn!("FixedLength: {} bytes remained", reader.remaining);
                }
                reader.remaining == 0
            }
            BodyReader::Chunked(reader) => reader.chunk_remaining == ChunkState::Empty,
            BodyReader::ToEnd(_) => true,
        };

        if is_done {
            Ok(len)
        } else {
            Err(Error::BufferTooSmall)
        }
    }

    async fn discard(&mut self) -> Result<usize, Error> {
        let mut body_len = 0;
        let mut buf = [0; 128];
        loop {
            let buf = self.read(&mut buf).await?;
            if buf == 0 {
                break;
            }
            body_len += buf;
        }

        Ok(body_len)
    }
}

impl<B> ErrorType for BodyReader<B> {
    type Error = Error;
}

impl<B> Read for BodyReader<B>
where
    B: Read,
{
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        match self {
            BodyReader::Empty => Ok(0),
            BodyReader::FixedLength(reader) => reader.read(buf).await,
            BodyReader::Chunked(reader) => reader.read(buf).await,
            BodyReader::ToEnd(conn) => conn.read(buf).await.map_err(|e| Error::Network(e.kind())),
        }
    }
}

impl<B> BufRead for BodyReader<B>
where
    B: BufRead + Read,
{
    async fn fill_buf(&mut self) -> Result<&[u8], Self::Error> {
        match self {
            BodyReader::Empty => Ok(&[]),
            BodyReader::FixedLength(reader) => reader.fill_buf().await,
            BodyReader::Chunked(reader) => reader.fill_buf().await,
            BodyReader::ToEnd(conn) => conn.fill_buf().await.map_err(|e| Error::Network(e.kind())),
        }
    }

    fn consume(&mut self, amt: usize) {
        match self {
            BodyReader::Empty => {}
            BodyReader::FixedLength(reader) => reader.consume(amt),
            BodyReader::Chunked(reader) => reader.consume(amt),
            BodyReader::ToEnd(conn) => conn.consume(amt),
        }
    }
}

/// Fixed length response body reader
pub struct FixedLengthBodyReader<B> {
    raw_body: B,
    remaining: usize,
}

impl<C> ErrorType for FixedLengthBodyReader<C> {
    type Error = Error;
}

impl<C> Read for FixedLengthBodyReader<C>
where
    C: Read,
{
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Error> {
        if self.remaining == 0 {
            return Ok(0);
        }

        let read = self.raw_body.read(buf).await.map_err(|e| Error::Network(e.kind()))?;
        self.remaining -= read;

        Ok(read)
    }
}

impl<C> BufRead for FixedLengthBodyReader<C>
where
    C: BufRead + Read,
{
    async fn fill_buf(&mut self) -> Result<&[u8], Self::Error> {
        if self.remaining == 0 {
            return Ok(&[]);
        }

        let loaded = self
            .raw_body
            .fill_buf()
            .await
            .map_err(|e| Error::Network(e.kind()))
            .map(|data| &data[..data.len().min(self.remaining)])?;

        if loaded.is_empty() {
            return Err(Error::ConnectionAborted);
        }

        Ok(loaded)
    }

    fn consume(&mut self, amt: usize) {
        let amt = amt.min(self.remaining);
        self.remaining -= amt;
        self.raw_body.consume(amt)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ChunkState {
    NoChunk,
    NotEmpty(u32),
    Empty,
}

impl ChunkState {
    fn consume(&mut self, amt: usize) -> usize {
        if let ChunkState::NotEmpty(remaining) = self {
            let consumed = (amt as u32).min(*remaining);
            *remaining -= consumed;
            consumed as usize
        } else {
            0
        }
    }

    fn len(self) -> usize {
        if let ChunkState::NotEmpty(len) = self {
            len as usize
        } else {
            0
        }
    }
}

/// Chunked response body reader
pub struct ChunkedBodyReader<B> {
    raw_body: B,
    chunk_remaining: ChunkState,
}

impl<C> ChunkedBodyReader<C>
where
    C: Read,
{
    async fn read_next_chunk_length(&mut self) -> Result<(), Error> {
        let mut header_buf = [0; 8 + 2]; // 32 bit hex + \r + \n
        let mut total_read = 0;

        'read_size: loop {
            let mut byte = 0;
            self.raw_body
                .read_exact(core::slice::from_mut(&mut byte))
                .await
                .map_err(|e| Error::from(e).kind())?;

            if byte != b'\n' {
                header_buf[total_read] = byte;
                total_read += 1;

                if total_read == header_buf.len() {
                    return Err(Error::Codec);
                }
            } else {
                if total_read == 0 || header_buf[total_read - 1] != b'\r' {
                    return Err(Error::Codec);
                }
                break 'read_size;
            }
        }

        let hex_digits = total_read - 1;

        // Prepend hex with zeros
        let mut hex = [b'0'; 8];
        hex[8 - hex_digits..].copy_from_slice(&header_buf[..hex_digits]);

        let mut bytes = [0; 4];
        hex::decode_to_slice(hex, &mut bytes).map_err(|_| Error::Codec)?;

        let chunk_length = u32::from_be_bytes(bytes);

        debug!("Chunk length: {}", chunk_length);

        self.chunk_remaining = match chunk_length {
            0 => ChunkState::Empty,
            other => ChunkState::NotEmpty(other),
        };

        Ok(())
    }

    async fn read_chunk_end(&mut self) -> Result<(), Error> {
        // All chunks are terminated with a \r\n
        let mut newline_buf = [0; 2];
        self.raw_body.read_exact(&mut newline_buf).await?;

        if newline_buf != [b'\r', b'\n'] {
            return Err(Error::Codec);
        }
        Ok(())
    }

    /// Handles chunk boundary and returns the number of bytes in the current (or new) chunk.
    async fn handle_chunk_boundary(&mut self) -> Result<usize, Error> {
        match self.chunk_remaining {
            ChunkState::NoChunk => self.read_next_chunk_length().await?,

            ChunkState::NotEmpty(0) => {
                // The current chunk is currently empty, advance into a new chunk...
                self.read_chunk_end().await?;
                self.read_next_chunk_length().await?;
            }

            ChunkState::NotEmpty(_) => {}

            ChunkState::Empty => return Ok(0),
        }

        if self.chunk_remaining == ChunkState::Empty {
            // Read final chunk termination
            self.read_chunk_end().await?;
        }

        Ok(self.chunk_remaining.len())
    }
}

impl<C> ErrorType for ChunkedBodyReader<C> {
    type Error = Error;
}

impl<C> Read for ChunkedBodyReader<C>
where
    C: Read,
{
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Error> {
        let remaining = self.handle_chunk_boundary().await?;
        let max_len = buf.len().min(remaining);

        let len = self
            .raw_body
            .read(&mut buf[..max_len])
            .await
            .map_err(|e| Error::Network(e.kind()))?;

        self.chunk_remaining.consume(len);

        Ok(len)
    }
}

impl<C> BufRead for ChunkedBodyReader<C>
where
    C: BufRead + Read,
{
    async fn fill_buf(&mut self) -> Result<&[u8], Self::Error> {
        let remaining = self.handle_chunk_boundary().await?;

        let buf = self.raw_body.fill_buf().await.map_err(|e| Error::Network(e.kind()))?;

        let len = buf.len().min(remaining);

        Ok(&buf[..len])
    }

    fn consume(&mut self, amt: usize) {
        let consumed = self.chunk_remaining.consume(amt);
        self.raw_body.consume(consumed);
    }
}

/// HTTP status types
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Status {
    Ok = 200,
    Created = 201,
    Accepted = 202,
    NoContent = 204,
    PartialContent = 206,
    MovedPermanently = 301,
    Found = 302,
    SeeOther = 303,
    NotModified = 304,
    TemporaryRedirect = 307,
    PermanentRedirect = 308,
    BadRequest = 400,
    Unauthorized = 401,
    Forbidden = 403,
    NotFound = 404,
    MethodNotAllowed = 405,
    Conflict = 409,
    UnsupportedMediaType = 415,
    RangeNotSatisfiable = 416,
    TooManyRequests = 429,
    InternalServerError = 500,
    BadGateway = 502,
    ServiceUnavailable = 503,
    GatewayTimeout = 504,
    Unknown = 0,
}

impl Status {
    pub fn is_informational(&self) -> bool {
        let status = *self as u16;
        (100..=199).contains(&status)
    }

    pub fn is_successful(&self) -> bool {
        let status = *self as u16;
        (200..=299).contains(&status)
    }

    pub fn is_redirection(&self) -> bool {
        let status = *self as u16;
        (300..=399).contains(&status)
    }

    pub fn is_client_error(&self) -> bool {
        let status = *self as u16;
        (400..=499).contains(&status)
    }

    pub fn is_server_error(&self) -> bool {
        let status = *self as u16;
        (500..=599).contains(&status)
    }
}

impl From<u16> for Status {
    fn from(from: u16) -> Status {
        match from {
            200 => Status::Ok,
            201 => Status::Created,
            202 => Status::Accepted,
            204 => Status::NoContent,
            206 => Status::PartialContent,
            301 => Status::MovedPermanently,
            302 => Status::Found,
            303 => Status::SeeOther,
            304 => Status::NotModified,
            307 => Status::TemporaryRedirect,
            308 => Status::PermanentRedirect,
            400 => Status::BadRequest,
            401 => Status::Unauthorized,
            403 => Status::Forbidden,
            404 => Status::NotFound,
            405 => Status::MethodNotAllowed,
            409 => Status::Conflict,
            415 => Status::UnsupportedMediaType,
            416 => Status::RangeNotSatisfiable,
            429 => Status::TooManyRequests,
            500 => Status::InternalServerError,
            502 => Status::BadGateway,
            503 => Status::ServiceUnavailable,
            504 => Status::GatewayTimeout,
            n => {
                warn!("Unknown status code: {:?}", n);
                Status::Unknown
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use core::convert::Infallible;

    use embedded_io::ErrorType;
    use embedded_io_async::Read;

    use crate::{
        reader::BufferingReader,
        request::Method,
        response::{ChunkState, ChunkedBodyReader, Response},
        Error,
    };

    #[tokio::test]
    async fn can_read_with_content_length_with_same_buffer() {
        let mut conn = FakeSingleReadConnection::new(b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\n\r\nHELLO WORLD");
        let mut response_buf = [0; 200];
        let response = Response::read(&mut conn, Method::GET, &mut response_buf).await.unwrap();

        let body = response.body().read_to_end().await.unwrap();

        assert_eq!(b"HELLO WORLD", body);
        assert!(conn.is_exhausted());
    }

    #[tokio::test]
    async fn can_read_with_content_length_to_other_buffer() {
        let mut conn = FakeSingleReadConnection::new(b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\n\r\nHELLO WORLD");
        let mut header_buf = [0; 200];
        let response = Response::read(&mut conn, Method::GET, &mut header_buf).await.unwrap();

        let mut body_buf = [0; 200];
        let len = response.body().reader().read_to_end(&mut body_buf).await.unwrap();

        assert_eq!(b"HELLO WORLD", &body_buf[..len]);
        assert!(conn.is_exhausted());
    }

    #[tokio::test]
    async fn can_discard_with_content_length() {
        let mut conn = FakeSingleReadConnection::new(b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\n\r\nHELLO WORLD");
        let mut response_buf = [0; 200];
        let response = Response::read(&mut conn, Method::GET, &mut response_buf).await.unwrap();

        assert_eq!(11, response.body().discard().await.unwrap());
        assert!(conn.is_exhausted());
    }

    #[tokio::test]
    async fn incorrect_fragment_length_does_not_panic() {
        let mut conn = FakeSingleReadConnection::new(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n\n\r\nHELLO WORLD\r\n0\r\n\r\n",
        );
        let mut header_buf = [0; 200];

        let response = Response::read(&mut conn, Method::GET, &mut header_buf).await.unwrap();

        let error = response.body().read_to_end().await.unwrap_err();

        assert!(matches!(error, Error::Codec));
    }

    #[tokio::test]
    async fn can_read_with_chunked_encoding() {
        let mut conn = FakeSingleReadConnection::new(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nB\r\nHELLO WORLD\r\n0\r\n\r\n",
        );
        let mut header_buf = [0; 200];
        let response = Response::read(&mut conn, Method::GET, &mut header_buf).await.unwrap();

        let mut body_buf = [0; 200];
        let len = response.body().reader().read_to_end(&mut body_buf).await.unwrap();

        assert_eq!(b"HELLO WORLD", &body_buf[..len]);
        assert!(conn.is_exhausted());
    }

    #[tokio::test]
    async fn can_read_with_chunked_encoding_empty_body() {
        let mut conn = FakeSingleReadConnection::new(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n");
        let mut header_buf = [0; 200];
        let response = Response::read(&mut conn, Method::GET, &mut header_buf).await.unwrap();

        let mut body_buf = [0; 200];
        let len = response.body().reader().read_to_end(&mut body_buf).await.unwrap();

        assert_eq!(0, len);
        assert!(conn.is_exhausted());
    }

    #[tokio::test]
    async fn can_discard_with_chunked_encoding() {
        let mut conn = FakeSingleReadConnection::new(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nB\r\nHELLO WORLD\r\n0\r\n\r\n",
        );
        let mut header_buf = [0; 200];
        let response = Response::read(&mut conn, Method::GET, &mut header_buf).await.unwrap();

        assert_eq!(11, response.body().discard().await.unwrap());
        assert!(conn.is_exhausted());
    }

    #[tokio::test]
    async fn can_read_to_end_of_connection_with_same_buffer() {
        let mut conn = FakeSingleReadConnection::new(b"HTTP/1.1 200 OK\r\n\r\nHELLO WORLD");
        let mut header_buf = [0; 200];
        let response = Response::read(&mut conn, Method::GET, &mut header_buf).await.unwrap();

        let body = response.body().read_to_end().await.unwrap();

        assert_eq!(b"HELLO WORLD", body);
        assert!(conn.is_exhausted());
    }

    #[tokio::test]
    async fn can_read_to_end_of_connection_to_other_buffer() {
        let mut conn = FakeSingleReadConnection::new(b"HTTP/1.1 200 OK\r\n\r\nHELLO WORLD");
        let mut header_buf = [0; 200];
        let response = Response::read(&mut conn, Method::GET, &mut header_buf).await.unwrap();

        let mut body_buf = [0; 200];
        let len = response.body().reader().read_to_end(&mut body_buf).await.unwrap();

        assert_eq!(b"HELLO WORLD", &body_buf[..len]);
        assert!(conn.is_exhausted());
    }

    #[tokio::test]
    async fn can_discard_to_end_of_connection() {
        let mut conn = FakeSingleReadConnection::new(b"HTTP/1.1 200 OK\r\n\r\nHELLO WORLD");
        let mut header_buf = [0; 200];
        let response = Response::read(&mut conn, Method::GET, &mut header_buf).await.unwrap();

        assert_eq!(11, response.body().discard().await.unwrap());
        assert!(conn.is_exhausted());
    }

    #[tokio::test]
    async fn chunked_body_reader_can_read_preloaded() {
        let mut read_buffer: Vec<u8> = Vec::new();
        read_buffer.extend_from_slice(b"1\r\nX\r\n10\r\nYYYYYYYYYYYYYYYY\r\n0\r\n\r\n");
        let preloaded = read_buffer.len();
        let mut empty_body = [0; 0].as_slice();
        let mut reader = ChunkedBodyReader {
            raw_body: BufferingReader::new(&mut read_buffer, preloaded, &mut empty_body),
            chunk_remaining: ChunkState::NoChunk,
        };

        let mut body = [0; 17];
        reader.read_exact(&mut body).await.unwrap();

        assert_eq!(0, reader.read(&mut body).await.unwrap());
        assert_eq!(0, reader.read(&mut body).await.unwrap());
        assert_eq!(b"XYYYYYYYYYYYYYYYY", &body);
    }

    #[tokio::test]
    async fn chunked_body_reader_can_read_with_large_buffer() {
        let mut raw_body = b"1\r\nX\r\n10\r\nYYYYYYYYYYYYYYYY\r\n0\r\n\r\n".as_slice();
        let mut read_buffer = [0; 128];
        let mut reader = ChunkedBodyReader {
            raw_body: BufferingReader::new(&mut read_buffer, 0, &mut raw_body),
            chunk_remaining: ChunkState::NoChunk,
        };

        let mut body = [0; 17];
        reader.read_exact(&mut body).await.unwrap();

        assert_eq!(0, reader.read(&mut body).await.unwrap());
        assert_eq!(0, reader.read(&mut body).await.unwrap());
        assert_eq!(b"XYYYYYYYYYYYYYYYY", &body);
    }

    #[tokio::test]
    async fn chunked_body_reader_can_read_with_tiny_buffer() {
        let mut raw_body = b"1\r\nX\r\n10\r\nYYYYYYYYYYYYYYYY\r\n0\r\n\r\n".as_slice();
        let mut read_buffer = [0; 128];
        let mut reader = ChunkedBodyReader {
            raw_body: BufferingReader::new(&mut read_buffer, 0, &mut raw_body),
            chunk_remaining: ChunkState::NoChunk,
        };

        let mut body = heapless::Vec::<u8, 17>::new();
        for _ in 0..17 {
            let mut buf = [0; 1];
            assert_eq!(1, reader.read(&mut buf).await.unwrap());
            body.push(buf[0]).unwrap();
        }

        let mut buf = [0; 1];
        assert_eq!(0, reader.read(&mut buf).await.unwrap());
        assert_eq!(0, reader.read(&mut buf).await.unwrap());
        assert_eq!(b"XYYYYYYYYYYYYYYYY", &body);
    }

    struct FakeSingleReadConnection {
        response: &'static [u8],
        offset: usize,
    }

    impl FakeSingleReadConnection {
        pub fn new(response: &'static [u8]) -> Self {
            Self { response, offset: 0 }
        }

        pub fn is_exhausted(&self) -> bool {
            self.offset == self.response.len()
        }
    }

    impl ErrorType for FakeSingleReadConnection {
        type Error = Infallible;
    }

    impl Read for FakeSingleReadConnection {
        async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
            if self.is_exhausted() || buf.is_empty() {
                return Ok(0);
            }

            buf[0] = self.response[self.offset];
            self.offset += 1;
            return Ok(1);
        }
    }
}
