use crate::Network;
use core::fmt::Write as _;
use core::{num::ParseIntError, str::Utf8Error};
use embedded_io::Error as _;
use heapless::String;

use crate::request::*;

pub struct HttpClient<'a, N>
where
    N: Network + 'a,
{
    connection: &'a mut N,
    host: &'a str,
}

impl<'a, N> HttpClient<'a, N>
where
    N: Network + 'a,
{
    pub fn new(connection: &'a mut N, host: &'a str) -> Self {
        Self { connection, host }
    }

    async fn write_data(&mut self, data: &[u8]) -> Result<(), Error> {
        self.connection.write(data).await.map_err(|e| e.kind())?;
        Ok(())
    }

    async fn write_str(&mut self, data: &str) -> Result<(), Error> {
        self.write_data(data.as_bytes()).await
    }

    async fn write_header(&mut self, key: &str, value: &str) -> Result<(), Error> {
        self.write_str(key).await?;
        self.write_str(": ").await?;
        self.write_str(value).await?;
        self.write_str("\r\n").await?;
        Ok(())
    }

    pub async fn request<'m>(&'m mut self, request: Request<'m>, rx_buf: &'m mut [u8]) -> Result<Response<'m>, Error> {
        self.write_str(request.method.as_str()).await?;
        self.write_str(" ").await?;
        self.write_str(request.path.unwrap_or("/")).await?;
        self.write_str(" HTTP/1.1\r\n").await?;

        self.write_header("Host", self.host).await?;

        if let Some(auth) = request.auth {
            match auth {
                Auth::Basic { username, password } => {
                    let mut combined: String<128> = String::new();
                    write!(combined, "{}:{}", username, password).unwrap();
                    let mut authz = [0; 256];
                    let authz_len = base64::encode_config_slice(combined.as_bytes(), base64::STANDARD, &mut authz);
                    self.write_str("Authorization: Basic ").await?;
                    self.write_str(unsafe { core::str::from_utf8_unchecked(&authz[..authz_len]) })
                        .await?;
                    self.write_str("\r\n").await?;
                }
            }
        }
        if let Some(content_type) = request.content_type {
            self.write_header("Content-Type", content_type.as_str()).await?;
        }
        if let Some(payload) = request.payload {
            let mut s: String<32> = String::new();
            write!(s, "{}", payload.len()).map_err(|_| Error::Parse)?;
            self.write_header("Content-Length", s.as_str()).await?;
        }
        if let Some(extra_headers) = request.extra_headers {
            for (header, value) in extra_headers.iter() {
                self.write_header(header, value).await?;
            }
        }
        self.write_str("\r\n").await?;
        trace!("Header written");
        match request.payload {
            None => Self::read_response(self.connection, rx_buf).await,
            Some(payload) => {
                trace!("Writing data");
                let result = self.connection.write(payload).await;
                match result {
                    Ok(_) => Self::read_response(self.connection, rx_buf).await,
                    Err(e) => {
                        warn!("Error sending data: {:?}", e.kind());
                        Err(Error::Network(e.kind()))
                    }
                }
            }
        }
    }

    async fn read_response<'m>(connection: &'m mut N, rx_buf: &'m mut [u8]) -> Result<Response<'m>, Error> {
        let mut pos = 0;
        let mut header_end = 0;
        while pos < rx_buf.len() {
            let n = connection.read(&mut rx_buf[pos..]).await.map_err(|e| {
                /*warn!(
                    "error {:?}, but read data from socket:  {:?}",
                    defmt::Debug2Format(&e),
                    defmt::Debug2Format(&core::str::from_utf8(&buf[..pos])),
                );*/
                e.kind()
            })?;

            pos += n;

            // Look for header end
            if let Some(n) = find_sequence(&rx_buf[..pos], b"\r\n\r\n") {
                header_end = n + 4;
                break;
            }
        }

        // Parse header
        let mut status = Status::BadRequest;
        let mut content_type = None;
        let mut content_length = 0;

        let header = core::str::from_utf8(&rx_buf[..header_end])?;
        trace!("Received header: {}", header);

        let lines = header.split("\r\n");
        for line in lines {
            if line.starts_with("HTTP") {
                let pos = b"HTTP/N.N ".len();
                status = line[pos..pos + 3].parse::<u32>()?.into();
            } else if match_header(line, "content-type") {
                content_type.replace(line["content-type:".len()..].trim_start().into());
            } else if match_header(line, "content-length") {
                content_length = line["content-length:".len()..].trim_start().parse::<usize>()?;
            }
        }

        // Copy to start of slice to save space
        for i in 0..(pos - header_end) {
            rx_buf[i] = rx_buf[header_end + i];
        }
        pos -= header_end;

        let payload = if content_length > 0 {
            // We might have data fetched already, keep that
            let content_length = content_length - pos;
            trace!("READING {} bytes of content", content_length);

            let mut to_read = core::cmp::min(rx_buf.len() - pos, content_length);
            //let to_copy = core::cmp::min(to_read, pos - header_end);
            /*
            trace!(
                "to_read({}), to_copy({}), header_end({}), pos({})",
                to_read,
                to_copy,
                header_end,
                pos
            );
            */
            //rx_buf[..to_copy].copy_from_slice(&buf[header_end..header_end + to_copy]);

            // Fetch the remaining data
            while to_read > 0 {
                trace!("Fetching {} bytes", to_read);
                let n = connection
                    .read(&mut rx_buf[pos..pos + to_read])
                    .await
                    .map_err(|e| e.kind())?;
                pos += n;
                to_read -= n;
            }
            trace!("http response has {} bytes in payload", pos);
            Some(&rx_buf[..pos])
        } else {
            trace!("0 bytes in payload");
            None
        };

        let response = Response {
            status,
            content_type,
            payload,
        };
        //trace!("HTTP response: {:?}", response);
        Ok(response)
    }
}

#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Error {
    Network(embedded_io::ErrorKind),
    DnsLookupFailed,
    Parse,
}

impl From<embedded_io::ErrorKind> for Error {
    fn from(e: embedded_io::ErrorKind) -> Error {
        Error::Network(e)
    }
}

impl From<ParseIntError> for Error {
    fn from(_: ParseIntError) -> Error {
        Error::Parse
    }
}

impl From<Utf8Error> for Error {
    fn from(_: Utf8Error) -> Error {
        Error::Parse
    }
}

// Find the needle sequence in the haystack. If found, return the hackstack position
// where the sequence was found.
fn find_sequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if haystack.len() < needle.len() {
        None
    } else {
        let mut p = 0;
        let mut windows = haystack.windows(needle.len());
        loop {
            if let Some(w) = windows.next() {
                if w == needle {
                    return Some(p);
                }
                p += 1;
            } else {
                return None;
            }
        }
    }
}

fn match_header(line: &str, hdr: &str) -> bool {
    if line.len() >= hdr.len() {
        line[0..hdr.len()].eq_ignore_ascii_case(hdr)
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sequence() {
        assert_eq!(Some(0), find_sequence(b"\r\n\r\n", b"\r\n\r\n"));
        assert_eq!(Some(3), find_sequence(b"foo\r\n\r\n", b"\r\n\r\n"));
        assert_eq!(Some(0), find_sequence(b"\r\n\r\nfoo", b"\r\n\r\n"));
        assert_eq!(Some(3), find_sequence(b"foo\r\n\r\nbar", b"\r\n\r\n"));
        assert_eq!(None, find_sequence(b"foobar\r\n\rother", b"\r\n\r\n"));
        assert_eq!(None, find_sequence(b"foo", b"\r\n\r\n"));
    }

    #[test]
    fn test_match_header() {
        assert!(match_header("Content-Length: 4", "Content-Length"));
        assert!(match_header("content-length: 4", "Content-Length"));
        assert!(match_header("Content-length: 4", "Content-Length"));
        assert!(!match_header("Content-type: application/json", "Content-Length"));
    }
}
