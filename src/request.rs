use crate::headers::ContentType;
/// Low level API for encoding requests and decoding responses.
use crate::Error;
use core::fmt::Write as _;
use core::mem::size_of;
use embedded_io::{Error as _, ErrorType};
use embedded_io_async::Write;
use heapless::String;

/// A read only HTTP request type
pub struct Request<'req, B>
where
    B: RequestBody,
{
    pub(crate) method: Method,
    pub(crate) base_path: Option<&'req str>,
    pub(crate) path: &'req str,
    pub(crate) auth: Option<Auth<'req>>,
    pub(crate) host: Option<&'req str>,
    pub(crate) body: Option<B>,
    pub(crate) content_type: Option<ContentType>,
    pub(crate) extra_headers: Option<&'req [(&'req str, &'req str)]>,
}

impl Default for Request<'_, ()> {
    fn default() -> Self {
        Self {
            method: Method::GET,
            base_path: None,
            path: "/",
            auth: None,
            host: None,
            body: None,
            content_type: None,
            extra_headers: None,
        }
    }
}

/// A HTTP request builder.
pub trait RequestBuilder<'req, B>
where
    B: RequestBody,
{
    type WithBody<T: RequestBody>: RequestBuilder<'req, T>;

    /// Set optional headers on the request.
    fn headers(self, headers: &'req [(&'req str, &'req str)]) -> Self;
    /// Set the path of the HTTP request.
    fn path(self, path: &'req str) -> Self;
    /// Set the data to send in the HTTP request body.
    fn body<T: RequestBody>(self, body: T) -> Self::WithBody<T>;
    /// Set the host header.
    fn host(self, host: &'req str) -> Self;
    /// Set the content type header for the request.
    fn content_type(self, content_type: ContentType) -> Self;
    /// Set the basic authentication header for the request.
    fn basic_auth(self, username: &'req str, password: &'req str) -> Self;
    /// Return an immutable request.
    fn build(self) -> Request<'req, B>;
}

/// Request authentication scheme.
pub enum Auth<'a> {
    Basic { username: &'a str, password: &'a str },
}

impl<'req> Request<'req, ()> {
    /// Create a new http request.
    #[allow(clippy::new_ret_no_self)]
    pub fn new(method: Method, path: &'req str) -> DefaultRequestBuilder<'req, ()> {
        DefaultRequestBuilder(Request {
            method,
            path,
            ..Default::default()
        })
    }

    /// Create a new GET http request.
    pub fn get(path: &'req str) -> DefaultRequestBuilder<'req, ()> {
        Self::new(Method::GET, path)
    }

    /// Create a new POST http request.
    pub fn post(path: &'req str) -> DefaultRequestBuilder<'req, ()> {
        Self::new(Method::POST, path)
    }

    /// Create a new PUT http request.
    pub fn put(path: &'req str) -> DefaultRequestBuilder<'req, ()> {
        Self::new(Method::PUT, path)
    }

    /// Create a new DELETE http request.
    pub fn delete(path: &'req str) -> DefaultRequestBuilder<'req, ()> {
        Self::new(Method::DELETE, path)
    }

    /// Create a new HEAD http request.
    pub fn head(path: &'req str) -> DefaultRequestBuilder<'req, ()> {
        Self::new(Method::HEAD, path)
    }
}

impl<'req, B> Request<'req, B>
where
    B: RequestBody,
{
    /// Write request header to the I/O stream
    pub async fn write_header<C>(&self, c: &mut C) -> Result<(), Error>
    where
        C: Write,
    {
        write_str(c, self.method.as_str()).await?;
        write_str(c, " ").await?;
        if let Some(base_path) = self.base_path {
            write_str(c, base_path.trim_end_matches('/')).await?;
            if !self.path.starts_with('/') {
                write_str(c, "/").await?;
            }
        }
        write_str(c, self.path).await?;
        write_str(c, " HTTP/1.1\r\n").await?;

        if let Some(auth) = &self.auth {
            match auth {
                Auth::Basic { username, password } => {
                    use base64::engine::{general_purpose, Engine as _};

                    let mut combined: String<128> = String::new();
                    write!(combined, "{}:{}", username, password).map_err(|_| Error::Codec)?;
                    let mut authz = [0; 256];
                    let authz_len = general_purpose::STANDARD
                        .encode_slice(combined.as_bytes(), &mut authz)
                        .map_err(|_| Error::Codec)?;
                    write_str(c, "Authorization: Basic ").await?;
                    write_str(c, unsafe { core::str::from_utf8_unchecked(&authz[..authz_len]) }).await?;
                    write_str(c, "\r\n").await?;
                }
            }
        }
        if let Some(host) = &self.host {
            write_header(c, "Host", host).await?;
        }
        if let Some(content_type) = &self.content_type {
            write_header(c, "Content-Type", content_type.as_str()).await?;
        }
        if let Some(body) = self.body.as_ref() {
            if let Some(len) = body.len() {
                let mut s: String<32> = String::new();
                write!(s, "{}", len).map_err(|_| Error::Codec)?;
                write_header(c, "Content-Length", s.as_str()).await?;
            } else {
                write_header(c, "Transfer-Encoding", "chunked").await?;
            }
        }
        if let Some(extra_headers) = self.extra_headers {
            for (header, value) in extra_headers.iter() {
                write_header(c, header, value).await?;
            }
        }
        write_str(c, "\r\n").await?;
        trace!("Header written");
        Ok(())
    }
}

pub struct DefaultRequestBuilder<'req, B>(Request<'req, B>)
where
    B: RequestBody;

impl<'req, B> RequestBuilder<'req, B> for DefaultRequestBuilder<'req, B>
where
    B: RequestBody,
{
    type WithBody<T: RequestBody> = DefaultRequestBuilder<'req, T>;

    fn headers(mut self, headers: &'req [(&'req str, &'req str)]) -> Self {
        self.0.extra_headers.replace(headers);
        self
    }

    fn path(mut self, path: &'req str) -> Self {
        self.0.path = path;
        self
    }

    fn body<T: RequestBody>(self, body: T) -> Self::WithBody<T> {
        DefaultRequestBuilder(Request {
            method: self.0.method,
            base_path: self.0.base_path,
            path: self.0.path,
            auth: self.0.auth,
            host: self.0.host,
            body: Some(body),
            content_type: self.0.content_type,
            extra_headers: self.0.extra_headers,
        })
    }

    fn host(mut self, host: &'req str) -> Self {
        self.0.host.replace(host);
        self
    }

    fn content_type(mut self, content_type: ContentType) -> Self {
        self.0.content_type.replace(content_type);
        self
    }

    fn basic_auth(mut self, username: &'req str, password: &'req str) -> Self {
        self.0.auth.replace(Auth::Basic { username, password });
        self
    }

    fn build(self) -> Request<'req, B> {
        self.0
    }
}

#[derive(Debug, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
/// HTTP request methods
pub enum Method {
    /// GET
    GET,
    /// PUT
    PUT,
    /// POST
    POST,
    /// DELETE
    DELETE,
    /// HEAD
    HEAD,
}

impl Method {
    /// str representation of method
    pub fn as_str(&self) -> &str {
        match self {
            Method::POST => "POST",
            Method::PUT => "PUT",
            Method::GET => "GET",
            Method::DELETE => "DELETE",
            Method::HEAD => "HEAD",
        }
    }
}

async fn write_str<C: Write>(c: &mut C, data: &str) -> Result<(), Error> {
    c.write_all(data.as_bytes()).await.map_err(|e| e.kind())?;
    Ok(())
}

async fn write_header<C: Write>(c: &mut C, key: &str, value: &str) -> Result<(), Error> {
    write_str(c, key).await?;
    write_str(c, ": ").await?;
    write_str(c, value).await?;
    write_str(c, "\r\n").await?;
    Ok(())
}

/// The request body
#[allow(clippy::len_without_is_empty)]
pub trait RequestBody {
    /// Get the length of the body if known
    ///
    /// If the length is known, then it will be written in the `Content-Length` header,
    /// chunked encoding will be used otherwise.
    fn len(&self) -> Option<usize> {
        None
    }

    /// Write the body to the provided writer
    async fn write<W: Write>(&self, writer: &mut W) -> Result<(), W::Error>;
}

impl RequestBody for () {
    fn len(&self) -> Option<usize> {
        None
    }

    async fn write<W: Write>(&self, _writer: &mut W) -> Result<(), W::Error> {
        Ok(())
    }
}

impl RequestBody for &[u8] {
    fn len(&self) -> Option<usize> {
        Some(<[u8]>::len(self))
    }

    async fn write<W: Write>(&self, writer: &mut W) -> Result<(), W::Error> {
        writer.write_all(self).await
    }
}

impl<T> RequestBody for Option<T>
where
    T: RequestBody,
{
    fn len(&self) -> Option<usize> {
        self.as_ref().map(|inner| inner.len()).unwrap_or_default()
    }

    async fn write<W: Write>(&self, writer: &mut W) -> Result<(), W::Error> {
        if let Some(inner) = self.as_ref() {
            inner.write(writer).await
        } else {
            Ok(())
        }
    }
}

pub struct FixedBodyWriter<C: Write>(C, usize);

impl<C> FixedBodyWriter<C>
where
    C: Write,
{
    pub fn new(conn: C) -> Self {
        Self(conn, 0)
    }

    pub fn written(&self) -> usize {
        self.1
    }
}

impl<C> ErrorType for FixedBodyWriter<C>
where
    C: Write,
{
    type Error = C::Error;
}

impl<C> Write for FixedBodyWriter<C>
where
    C: Write,
{
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        let written = self.0.write(buf).await?;
        self.1 += written;
        Ok(written)
    }

    async fn write_all(&mut self, buf: &[u8]) -> Result<(), Self::Error> {
        self.0.write_all(buf).await?;
        self.1 += buf.len();
        Ok(())
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        self.0.flush().await
    }
}

const fn hex_chars(number: usize) -> u32 {
    if number == 0 {
        1
    } else {
        (usize::BITS - number.leading_zeros()).div_ceil(4)
    }
}

fn write_chunked_header(buf: &mut [u8], chunk_len: usize) -> usize {
    let mut hex = [0; 2 * size_of::<usize>()];
    hex::encode_to_slice(chunk_len.to_be_bytes(), &mut hex).unwrap();
    let leading_zeros = hex.iter().position(|x| *x != b'0').unwrap_or_default();
    let hex_chars = hex.len() - leading_zeros;
    buf[..hex_chars].copy_from_slice(&hex[leading_zeros..]);
    buf[hex_chars..hex_chars + 2].copy_from_slice(b"\r\n");
    hex_chars + 2
}

pub struct ChunkedBodyWriter<C: Write>(C);

impl<C> ChunkedBodyWriter<C>
where
    C: Write,
{
    pub fn new(conn: C) -> Self {
        Self(conn)
    }

    pub async fn terminate(&mut self) -> Result<(), C::Error> {
        self.0.write_all(b"0\r\n\r\n").await
    }
}

impl<C> ErrorType for ChunkedBodyWriter<C>
where
    C: Write,
{
    type Error = embedded_io::ErrorKind;
}

impl<C> Write for ChunkedBodyWriter<C>
where
    C: Write,
{
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        self.write_all(buf).await.map_err(|e| e.kind())?;
        Ok(buf.len())
    }

    async fn write_all(&mut self, buf: &[u8]) -> Result<(), Self::Error> {
        let len = buf.len();

        // Do not write an empty chunk as that will terminate the body
        // Use `ChunkedBodyWriter.write_empty_chunk` instead if this is intended
        if len == 0 {
            return Ok(());
        }

        // Write chunk header
        let mut header_buf = [0; 2 * size_of::<usize>() + 2];
        let header_len = write_chunked_header(&mut header_buf, len);
        self.0
            .write_all(&header_buf[..header_len])
            .await
            .map_err(|e| e.kind())?;

        // Write chunk
        self.0.write_all(buf).await.map_err(|e| e.kind())?;

        // Write newline footer
        self.0.write_all(b"\r\n").await.map_err(|e| e.kind())?;
        Ok(())
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        self.0.flush().await.map_err(|e| e.kind())
    }
}

pub struct BufferedChunkedBodyWriter<'a, C: Write> {
    conn: C,
    buf: &'a mut [u8],
    header_pos: usize,
    pos: usize,
    max_header_size: usize,
    max_footer_size: usize,
}

impl<'a, C> BufferedChunkedBodyWriter<'a, C>
where
    C: Write,
{
    pub fn new_with_data(conn: C, buf: &'a mut [u8], written: usize) -> Self {
        let max_hex_chars = hex_chars(buf.len());
        let max_header_size = max_hex_chars as usize + 2;
        let max_footer_size = 2;
        assert!(buf.len() > max_header_size + max_footer_size); // There must be space for the chunk header and footer
        Self {
            conn,
            buf,
            header_pos: written,
            pos: written + max_header_size,
            max_header_size,
            max_footer_size,
        }
    }

    pub async fn terminate(&mut self) -> Result<(), C::Error> {
        if self.pos > self.header_pos + self.max_header_size {
            self.finish_current_chunk();
        }
        const EMPTY: &[u8; 5] = b"0\r\n\r\n";
        if self.header_pos + EMPTY.len() > self.buf.len() {
            self.emit_finished_chunk().await?;
        }

        self.buf[self.header_pos..self.header_pos + EMPTY.len()].copy_from_slice(EMPTY);
        self.header_pos += EMPTY.len();
        self.pos = self.header_pos + self.max_header_size;
        self.emit_finished_chunk().await
    }

    fn append_current_chunk(&mut self, buf: &[u8]) -> usize {
        let buffered = usize::min(buf.len(), self.buf.len() - self.max_footer_size - self.pos);
        if buffered > 0 {
            self.buf[self.pos..self.pos + buffered].copy_from_slice(&buf[..buffered]);
            self.pos += buffered;
        }
        buffered
    }

    fn finish_current_chunk(&mut self) {
        // Write the header in the allocated position position
        let chunk_len = self.pos - self.header_pos - self.max_header_size;
        let header_buf = &mut self.buf[self.header_pos..self.header_pos + self.max_header_size];
        let header_len = write_chunked_header(header_buf, chunk_len);

        // Move the payload if the header length was not as large as it could possibly be
        let spacing = self.max_header_size - header_len;
        if spacing > 0 {
            self.buf.copy_within(
                self.header_pos + self.max_header_size..self.pos,
                self.header_pos + header_len,
            );
            self.pos -= spacing
        }

        // Write newline footer after chunk payload
        self.buf[self.pos..self.pos + 2].copy_from_slice(b"\r\n");
        self.pos += 2;

        self.header_pos = self.pos;
        self.pos = self.header_pos + self.max_header_size;
    }

    async fn emit_finished_chunk(&mut self) -> Result<(), C::Error> {
        self.conn.write_all(&self.buf[..self.header_pos]).await?;
        self.header_pos = 0;
        self.pos = self.max_header_size;
        Ok(())
    }
}

impl<C> ErrorType for BufferedChunkedBodyWriter<'_, C>
where
    C: Write,
{
    type Error = embedded_io::ErrorKind;
}

impl<C> Write for BufferedChunkedBodyWriter<'_, C>
where
    C: Write,
{
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        let written = self.append_current_chunk(buf);
        if written < buf.len() {
            self.finish_current_chunk();
            self.emit_finished_chunk().await.map_err(|e| e.kind())?;
        }
        Ok(written)
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        if self.header_pos > 0 {
            self.finish_current_chunk();
            self.emit_finished_chunk().await.map_err(|e| e.kind())?;
        }
        self.conn.flush().await.map_err(|e| e.kind())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_chars_values() {
        assert_eq!(1, hex_chars(0));
        assert_eq!(1, hex_chars(1));
        assert_eq!(1, hex_chars(0xF));
        assert_eq!(2, hex_chars(0x10));
        assert_eq!(2, hex_chars(0xFF));
        assert_eq!(3, hex_chars(0x100));
    }

    #[tokio::test]
    async fn basic_auth() {
        let mut buffer: Vec<u8> = Vec::new();
        Request::new(Method::GET, "/")
            .basic_auth("username", "password")
            .build()
            .write_header(&mut buffer)
            .await
            .unwrap();

        assert_eq!(
            b"GET / HTTP/1.1\r\nAuthorization: Basic dXNlcm5hbWU6cGFzc3dvcmQ=\r\n\r\n",
            buffer.as_slice()
        );
    }

    #[tokio::test]
    async fn with_empty_body() {
        let mut buffer = Vec::new();
        Request::new(Method::POST, "/")
            .body([].as_slice())
            .build()
            .write_header(&mut buffer)
            .await
            .unwrap();

        assert_eq!(b"POST / HTTP/1.1\r\nContent-Length: 0\r\n\r\n", buffer.as_slice());
    }

    #[tokio::test]
    async fn with_known_body_adds_content_length_header() {
        let mut buffer = Vec::new();
        Request::new(Method::POST, "/")
            .body(b"BODY".as_slice())
            .build()
            .write_header(&mut buffer)
            .await
            .unwrap();

        assert_eq!(b"POST / HTTP/1.1\r\nContent-Length: 4\r\n\r\n", buffer.as_slice());
    }

    struct ChunkedBody;

    impl RequestBody for ChunkedBody {
        fn len(&self) -> Option<usize> {
            None // Unknown length: triggers chunked body
        }

        async fn write<W: Write>(&self, _writer: &mut W) -> Result<(), W::Error> {
            unreachable!()
        }
    }

    #[tokio::test]
    async fn with_unknown_body_adds_transfer_encoding_header() {
        let mut buffer = Vec::new();

        Request::new(Method::POST, "/")
            .body(ChunkedBody)
            .build()
            .write_header(&mut buffer)
            .await
            .unwrap();

        assert_eq!(
            b"POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n",
            buffer.as_slice()
        );
    }
}
