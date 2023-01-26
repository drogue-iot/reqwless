use embedded_io::blocking::ReadExactError;
use embedded_io::ErrorKind;
use embedded_io::{asynch::Read, Error as _, Io};

use crate::headers::ContentType;
use crate::Error;

/// Type representing a parsed HTTP response.
#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Response<'a> {
    /// The HTTP response status code.
    pub status: Status,
    /// The HTTP response content type.
    pub content_type: Option<ContentType>,
    /// The content length.
    pub content_length: Option<usize>,
    header_buf: &'a mut [u8],
    header_len: usize,
    body_pos: usize,
}

impl<'a> Response<'a> {
    // Read at least the headers from the connection.
    pub async fn read<C: Read>(conn: &mut C, header_buf: &'a mut [u8]) -> Result<Response<'a>, Error> {
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
                header_len = parse_status.unwrap().into();
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
            status,
            content_type,
            content_length,
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
    pub fn body<'conn, C: Read>(self, conn: &'conn mut C) -> ResponseBody<'a, 'conn, C> {
        // Move the body part of the bytes in the header buffer to the beginning of the buffer
        let header_buf = self.header_buf;
        for i in 0..self.body_pos {
            header_buf[i] = header_buf[self.header_len + i];
        }

        // The header buffer is now the body buffer
        let body_buf = header_buf;
        let reader = BodyReader {
            conn,
            remaining: self.content_length.map(|cl| cl - self.body_pos),
        };

        ResponseBody {
            body_buf,
            body_pos: self.body_pos,
            reader,
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

/// Response body reader
pub struct BodyReader<'a, C: Read> {
    conn: &'a mut C,
    remaining: Option<usize>,
}

impl<C: Read> BodyReader<'_, C> {
    /// Read until the end of the body
    pub async fn read_to_end(&mut self, buf: &mut [u8]) -> Result<usize, Error> {
        let to_read = self.remaining.ok_or(Error::Codec)?;
        if buf.len() < to_read {
            // The buffer is not sufficiently large to contain the entire body
            return Err(Error::BufferTooSmall);
        }

        self.read_exact(&mut buf[..to_read]).await.map_err(|e| match e {
            ReadExactError::UnexpectedEof => Error::Network(ErrorKind::Other),
            ReadExactError::Other(e) => e,
        })?;

        assert_eq!(0, self.remaining.unwrap_or_default());

        Ok(to_read)
    }
}

impl<C: Read> Io for BodyReader<'_, C> {
    type Error = Error;
}

impl<C: Read> Read for BodyReader<'_, C> {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Error> {
        let remaining = self.remaining.ok_or(Error::Codec)?;
        let to_read = usize::min(remaining, buf.len());
        let len = self.conn.read(&mut buf[..to_read]).await.map_err(|e| e.kind())?;
        self.remaining.replace(remaining - len);
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
