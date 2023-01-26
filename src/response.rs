use embedded_io::asynch::Read;

use embedded_io::Error as _;

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
    /// The HTTP response body.
    pub body: Option<&'a mut [u8]>,
}

impl<'a> Response<'a> {
    pub async fn read<C>(conn: &mut C, rx_buf: &'a mut [u8]) -> Result<Response<'a>, Error>
    where
        C: Read,
    {
        let mut pos = 0;
        while pos < rx_buf.len() {
            let n = conn.read(&mut rx_buf[pos..]).await.map_err(|e| {
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
            if response.parse(&rx_buf[..pos]).map_err(|_| Error::Codec)?.is_complete() {
                break;
            } else {
            }
        }

        // Parse header
        let mut headers = [httparse::EMPTY_HEADER; 64];
        let mut response = httparse::Response::new(&mut headers);
        let result = response.parse(&rx_buf[..pos]).map_err(|_| Error::Codec)?;
        if result.is_partial() {
            return Err(Error::Codec);
        }
        let header_end = result.unwrap();
        let status = response.code.unwrap_or(400);
        let mut content_type = None;
        let mut content_length = 0;

        for header in response.headers {
            if header.name.eq_ignore_ascii_case("content-type") {
                content_type.replace(header.value.into());
            } else if header.name.eq_ignore_ascii_case("content-length") {
                content_length = core::str::from_utf8(header.value)
                    .map(|value| value.parse::<usize>().unwrap_or(0))
                    .unwrap_or(0);
            }
        }

        // Overwrite header and copy the rest of data to the start of the slice to save space.
        if header_end < pos {
            for i in 0..(pos - header_end) {
                rx_buf[i] = rx_buf[header_end + i];
            }
            pos = pos - header_end;
        } else {
            pos = 0;
        }

        let body = if content_length > 0 {
            // We might have data fetched already, keep that

            let mut to_read = core::cmp::min(rx_buf.len() - pos, content_length - pos);
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
                let n = conn.read(&mut rx_buf[pos..pos + to_read]).await.map_err(|e| e.kind())?;
                pos += n;
                to_read -= n;
            }
            trace!("http response has {} bytes in body", pos);
            Some(&mut rx_buf[..pos])
        } else {
            trace!("0 bytes in body");
            None
        };

        let response = Response {
            status: status.into(),
            content_type,
            body,
        };
        //trace!("HTTP response: {:?}", response);
        Ok(response)
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
