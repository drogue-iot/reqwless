use embedded_io::{asynch::Read, Error as _, Io};
use heapless::Vec;

use crate::headers::{ContentType, TransferEncoding};
use crate::request::Method;
use crate::Error;

/// Type representing a parsed HTTP response.
#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Response<'buf, 'conn, C>
where
    C: Read,
{
    conn: &'conn mut C,
    /// The method used to create the response.
    method: Method,
    /// The HTTP response status code.
    pub status: Status,
    /// The HTTP response content type.
    pub content_type: Option<ContentType>,
    /// The content length.
    pub content_length: Option<usize>,
    pub transfer_encoding: heapless::Vec<TransferEncoding, 4>,
    header_buf: &'buf mut [u8],
    header_len: usize,
    body_pos: usize,
}

impl<'buf, 'conn, C> Response<'buf, 'conn, C>
where
    C: Read,
{
    // Read at least the headers from the connection.
    pub async fn read(
        conn: &'conn mut C,
        method: Method,
        header_buf: &'buf mut [u8],
    ) -> Result<Response<'buf, 'conn, C>, Error> {
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

            pos += n;

            // Look for header end
            let mut headers = [httparse::EMPTY_HEADER; 64];
            let mut response = httparse::Response::new(&mut headers);
            let parse_status = response.parse(&header_buf[..pos]).map_err(|_| Error::Codec)?;
            if parse_status.is_complete() {
                header_len = parse_status.unwrap();
                break;
            } else {
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
            }
        }

        // The number of bytes that we have read into the body part of the response
        let body_pos = pos - header_len;

        if let Some(content_length) = content_length {
            if content_length < body_pos {
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
            header_buf,
            header_len,
            body_pos,
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
    pub fn body(self) -> Result<ResponseBody<'buf, 'conn, C>, Error> {
        if self.method == Method::HEAD {
            // Head requests does not have a body so we return an empty reader
            Ok(ResponseBody {
                body_buf: self.header_buf,
                body_offset: 0,
                body_pos: 0,
                reader: BodyReader::FixedLength(FixedLengthBodyReader {
                    conn: self.conn,
                    remaining: 0,
                }),
            })
        } else {
            // Move the body part of the bytes in the header buffer to the beginning of the buffer
            let header_buf = self.header_buf;
            for i in 0..self.body_pos {
                header_buf[i] = header_buf[self.header_len + i];
            }

            // From now on, the header buffer is now the body buffer as all header bytes have been overwritten
            let body_buf = header_buf;
            let reader = if let Some(content_length) = self.content_length {
                BodyReader::FixedLength(FixedLengthBodyReader {
                    conn: self.conn,
                    remaining: content_length - self.body_pos,
                })
            } else if self.transfer_encoding.contains(&TransferEncoding::Chunked) {
                BodyReader::Chunked(ChunkedBodyReader {
                    conn: self.conn,
                    chunk_remaining: 0,
                    empty_chunk_received: false,
                })
            } else {
                return Err(Error::Codec);
            };

            Ok(ResponseBody {
                body_buf,
                body_pos: self.body_pos,
                reader,
            })
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
pub struct ResponseBody<'buf, 'conn, C: Read> {
    /// The buffer initially provided to read the header.
    pub body_buf: &'buf mut [u8],
    /// The number bytes raed from the body and available in `body_buf`.
    pub body_pos: usize,
    /// The reader to be used for reading the remaining body.
    pub reader: BodyReader<'conn, C>,
}

impl<'buf, 'conn, C: Read> ResponseBody<'buf, 'conn, C> {
    /// Read the entire body
    pub async fn read_to_end(mut self) -> Result<&'buf [u8], Error> {
        // Read into the buffer after the portion that was already received when parsing the header
        let len = self.reader.read_to_end(&mut self.body_buf[self.body_pos..]).await?;
        Ok(&self.body_buf[..self.body_pos + len])
    }
}

/// A body reader
pub enum BodyReader<'conn, C>
where
    C: Read,
{
    FixedLength(FixedLengthBodyReader<'conn, C>),
    Chunked(ChunkedBodyReader<'conn, C>),
}

impl<C> BodyReader<'_, C>
where
    C: Read,
{
    /// Read the entire body
    pub async fn read_to_end(&mut self, buf: &mut [u8]) -> Result<usize, Error> {
        let mut len = 0;
        while len < buf.len() {
            match self.read(&mut buf[len..]).await {
                Ok(0) => break,
                Ok(n) => len += n,
                Err(e) => return Err(e.into()),
            }
        }

        let is_done = match self {
            BodyReader::FixedLength(reader) => reader.remaining == 0,
            BodyReader::Chunked(reader) => reader.empty_chunk_received,
        };

        if is_done {
            Ok(len)
        } else {
            Err(Error::BufferTooSmall)
        }
    }
}

impl<C> embedded_io::Io for BodyReader<'_, C>
where
    C: Read,
{
    type Error = Error;
}

impl<C> Read for BodyReader<'_, C>
where
    C: Read,
{
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        match self {
            BodyReader::FixedLength(reader) => reader.read(buf).await,
            BodyReader::Chunked(reader) => reader.read(buf).await,
        }
    }
}

/// Fixed length response body reader
pub struct FixedLengthBodyReader<'conn, C: Read> {
    conn: &'conn mut C,
    remaining: usize,
}

impl<C: Read> Io for FixedLengthBodyReader<'_, C> {
    type Error = Error;
}

impl<C: Read> Read for FixedLengthBodyReader<'_, C> {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Error> {
        if self.remaining == 0 {
            return Ok(0);
        }
        let to_read = usize::min(self.remaining, buf.len());
        let len = self.conn.read(&mut buf[..to_read]).await.map_err(|e| e.kind())?;
        self.remaining -= len;
        Ok(len)
    }
}

/// Chunked response body reader
pub struct ChunkedBodyReader<'conn, C> {
    conn: &'conn mut C,
    chunk_remaining: u32,
    empty_chunk_received: bool,
}

impl<'conn, C: Read> ChunkedBodyReader<'conn, C> {
    async fn read_chunk_end(&mut self) -> Result<(), Error> {
        // All chunks are terminated with a \r\n
        let mut newline_buf = [0; 2];
        self.conn.read_exact(&mut newline_buf).await.map_err(|_| Error::Codec)?;
        if newline_buf != ['\r' as u8, '\n' as u8] {
            return Err(Error::Codec);
        }
        Ok(())
    }
}

impl<C: Read> Io for ChunkedBodyReader<'_, C> {
    type Error = Error;
}

impl<C: Read> Read for ChunkedBodyReader<'_, C> {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Error> {
        if self.chunk_remaining == 0 {
            let mut header_buf = [0; 8 + 2]; // 32 bit hex + \r + \n
            let mut total_read = 0;

            // For now, limit the number of bytes that we can read to avoid reading into a header after the current
            let mut max_read = 3; // Single hex digit + \r + \n
            loop {
                let read = self
                    .conn
                    .read(&mut header_buf[total_read..max_read])
                    .await
                    .map_err(|e| e.kind())?;
                if read == 0 {
                    return Err(Error::Codec);
                }
                total_read += read;

                // Decode the chunked header
                let header_and_body = &header_buf[..total_read];
                if let Some(nl) = header_and_body.iter().position(|x| *x == '\n' as u8) {
                    let header = &header_and_body[..nl + 1];
                    if nl == 0 || header[nl - 1] != '\r' as u8 {
                        return Err(Error::Codec);
                    }
                    let hex_digits = nl - 1;
                    // Prepend hex with zeros
                    let mut hex = ['0' as u8; 8];
                    hex[8 - hex_digits..].copy_from_slice(&header[..hex_digits]);
                    let mut bytes = [0; 4];
                    hex::decode_to_slice(&hex, &mut bytes).map_err(|_| Error::Codec)?;
                    self.chunk_remaining = u32::from_be_bytes(bytes);

                    // Return the excess body bytes read during the header, if any
                    let excess_body_read = header_and_body.len() - header.len();
                    if excess_body_read > 0 {
                        if excess_body_read > self.chunk_remaining as usize {
                            // We have read chunk bytes that exceed the size of the chunk
                            return Err(Error::Codec);
                        }

                        buf[..excess_body_read].copy_from_slice(&header_and_body[header.len()..]);

                        return Ok(excess_body_read);
                    }

                    break;
                }

                if total_read >= 3 {
                    // At least three bytes were read and a \n was not found
                    // This means that the chunk length is at least double-digit hex
                    // which in turn means that it is impossible for another header to
                    // be present within the 10 bytes header buffer
                    max_read = 10;
                }
            }
        }

        let len = usize::min(self.chunk_remaining as usize, buf.len());
        self.conn.read(&mut buf[..len]).await.map_err(|e| e.kind())?;
        self.chunk_remaining -= len as u32;

        if self.chunk_remaining == 0 {
            self.read_chunk_end().await?;
        }

        Ok(len)
    }
}

/// HTTP status types
#[derive(Debug, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Status {
    Ok = 200,
    Created = 201,
    Accepted = 202,
    BadRequest = 400,
    Unauthorized = 401,
    Forbidden = 403,
    NotFound = 404,
    Unknown = 0,
}

impl From<u16> for Status {
    fn from(from: u16) -> Status {
        match from {
            200 => Status::Ok,
            201 => Status::Created,
            202 => Status::Accepted,
            400 => Status::BadRequest,
            401 => Status::Unauthorized,
            403 => Status::Forbidden,
            404 => Status::NotFound,
            n => {
                warn!("Unknown status code: {:?}", n);
                Status::Unknown
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn chunked_body_reader() {
        let mut source = "1\r\nX\r\n1\r\nY\r\n0\r\n\r\n".as_bytes();
        let mut reader = ChunkedBodyReader {
            conn: &mut source,
            chunk_remaining: 0,
            empty_chunk_received: false,
        };

        let mut buf = [0; 2];
        reader.read_exact(&mut buf).await.unwrap();

        assert_eq!(0, reader.read(&mut buf).await.unwrap());
    }
}
