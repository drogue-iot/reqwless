/// Low level API for encoding requests and decoding responses.
use crate::Error;
use core::fmt::Write as _;
use embedded_io::asynch::{Read, Write};
use embedded_io::Error as _;
use heapless::String;

/// A read only HTTP request type
pub struct Request<'a> {
    pub(crate) method: Method,
    pub(crate) path: Option<&'a str>,
    pub(crate) auth: Option<Auth<'a>>,
    pub(crate) host: Option<&'a str>,
    pub(crate) body: Option<&'a [u8]>,
    pub(crate) content_type: Option<ContentType>,
    pub(crate) extra_headers: Option<&'a [(&'a str, &'a str)]>,
}

impl<'a> Default for Request<'a> {
    fn default() -> Self {
        Self {
            method: Method::GET,
            path: None,
            auth: None,
            host: None,
            body: None,
            content_type: None,
            extra_headers: None,
        }
    }
}

/// A HTTP request builder.
pub struct RequestBuilder<'a> {
    request: Request<'a>,
}

/// Request authentication scheme.
pub enum Auth<'a> {
    Basic { username: &'a str, password: &'a str },
}

impl<'a> Request<'a> {
    /// Create a new GET http request.
    pub fn new(method: Method) -> RequestBuilder<'a> {
        RequestBuilder {
            request: Request {
                method,
                ..Default::default()
            },
        }
    }

    /// Create a new GET http request.
    pub fn get() -> RequestBuilder<'a> {
        RequestBuilder {
            request: Request {
                method: Method::GET,
                ..Default::default()
            },
        }
    }

    /// Create a new POST http request.
    pub fn post() -> RequestBuilder<'a> {
        RequestBuilder {
            request: Request {
                method: Method::POST,
                ..Default::default()
            },
        }
    }

    /// Create a new PUT http request.
    pub fn put() -> RequestBuilder<'a> {
        RequestBuilder {
            request: Request {
                method: Method::PUT,
                ..Default::default()
            },
        }
    }

    /// Create a new DELETE http request.
    pub fn delete() -> RequestBuilder<'a> {
        RequestBuilder {
            request: Request {
                method: Method::DELETE,
                ..Default::default()
            },
        }
    }

    /// Write request to the I/O stream
    pub async fn write<C>(&self, c: &mut C) -> Result<(), Error>
    where
        C: Write,
    {
        write_str(c, self.method.as_str()).await?;
        write_str(c, " ").await?;
        write_str(c, self.path.unwrap_or("/")).await?;
        write_str(c, " HTTP/1.1\r\n").await?;

        //        write_header(c, "Host", self.host).await?;

        if let Some(auth) = &self.auth {
            match auth {
                Auth::Basic { username, password } => {
                    let mut combined: String<128> = String::new();
                    write!(combined, "{}:{}", username, password).map_err(|_| Error::Codec)?;
                    let mut authz = [0; 256];
                    let authz_len = base64::encode_config_slice(combined.as_bytes(), base64::STANDARD, &mut authz);
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
        if let Some(body) = self.body {
            let mut s: String<32> = String::new();
            write!(s, "{}", body.len()).map_err(|_| Error::Codec)?;
            write_header(c, "Content-Length", s.as_str()).await?;
        }
        if let Some(extra_headers) = self.extra_headers {
            for (header, value) in extra_headers.iter() {
                write_header(c, header, value).await?;
            }
        }
        write_str(c, "\r\n").await?;
        trace!("Header written");
        match self.body {
            None => c.flush().await.map_err(|e| Error::Network(e.kind())),
            Some(body) => {
                trace!("Writing data");
                let result = c.write(body).await;
                match result {
                    Ok(_) => c.flush().await.map_err(|e| Error::Network(e.kind())),
                    Err(e) => {
                        warn!("Error sending data: {:?}", e.kind());
                        Err(Error::Network(e.kind()))
                    }
                }
            }
        }
    }
}

impl<'a> RequestBuilder<'a> {
    /// Set optional headers on the request.
    pub fn headers(mut self, headers: &'a [(&'a str, &'a str)]) -> Self {
        self.request.extra_headers.replace(headers);
        self
    }

    /// Set the path of the HTTP request.
    pub fn path(mut self, path: &'a str) -> Self {
        self.request.path.replace(path);
        self
    }

    /// Set the data to send in the HTTP request body.
    pub fn body(mut self, body: &'a [u8]) -> Self {
        self.request.body.replace(body);
        self
    }

    /// Set the host header.
    pub fn host(mut self, host: &'a str) -> Self {
        self.request.host.replace(host);
        self
    }

    /// Set the content type header for the request.
    pub fn content_type(mut self, content_type: ContentType) -> Self {
        self.request.content_type.replace(content_type);
        self
    }

    /// Set the basic authentication header for the request.
    pub fn basic_auth(mut self, username: &'a str, password: &'a str) -> Self {
        self.request.auth.replace(Auth::Basic { username, password });
        self
    }

    /// Return an immutable request.
    pub fn build(self) -> Request<'a> {
        self.request
    }
}

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
}

impl Method {
    /// str representation of method
    pub fn as_str(&self) -> &str {
        match self {
            Method::POST => "POST",
            Method::PUT => "PUT",
            Method::GET => "GET",
            Method::DELETE => "DELETE",
        }
    }
}

/// Type representing a parsed HTTP response.
#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Response<'a> {
    /// The HTTP response status code.
    pub status: Status,
    /// The HTTP response content type.
    pub content_type: Option<ContentType>,
    /// The HTTP response body.
    pub body: Option<&'a [u8]>,
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
            Some(&rx_buf[..pos])
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

/// HTTP content types
#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ContentType {
    TextPlain,
    ApplicationJson,
    ApplicationCbor,
    ApplicationOctetStream,
}

impl<'a> From<&'a [u8]> for ContentType {
    fn from(from: &'a [u8]) -> ContentType {
        match from {
            b"application/json" => ContentType::ApplicationJson,
            b"application/cbor" => ContentType::ApplicationCbor,
            b"text/plain" => ContentType::TextPlain,
            _ => ContentType::ApplicationOctetStream,
        }
    }
}

impl ContentType {
    pub fn as_str(&self) -> &str {
        match self {
            ContentType::TextPlain => "text/plain",
            ContentType::ApplicationJson => "application/json",
            ContentType::ApplicationCbor => "application/cbor",
            ContentType::ApplicationOctetStream => "application/octet-stream",
        }
    }
}

async fn write_data<C: Write>(c: &mut C, data: &[u8]) -> Result<(), Error> {
    c.write(data).await.map_err(|e| e.kind())?;
    Ok(())
}

async fn write_str<C: Write>(c: &mut C, data: &str) -> Result<(), Error> {
    write_data(c, data.as_bytes()).await
}

async fn write_header<C: Write>(c: &mut C, key: &str, value: &str) -> Result<(), Error> {
    write_str(c, key).await?;
    write_str(c, ": ").await?;
    write_str(c, value).await?;
    write_str(c, "\r\n").await?;
    Ok(())
}
