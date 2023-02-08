use crate::headers::ContentType;
/// Low level API for encoding requests and decoding responses.
use crate::Error;
use core::fmt::Write as _;
use embedded_io::asynch::Write;
use embedded_io::Error as _;
use heapless::String;

/// A read only HTTP request type
pub struct Request<'a> {
    pub(crate) method: Method,
    pub(crate) base_path: Option<&'a str>,
    pub(crate) path: &'a str,
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
pub trait RequestBuilder<'a> {
    /// Set optional headers on the request.
    fn headers(self, headers: &'a [(&'a str, &'a str)]) -> Self;
    /// Set the path of the HTTP request.
    fn path(self, path: &'a str) -> Self;
    /// Set the data to send in the HTTP request body.
    fn body(self, body: &'a [u8]) -> Self;
    /// Set the host header.
    fn host(self, host: &'a str) -> Self;
    /// Set the content type header for the request.
    fn content_type(self, content_type: ContentType) -> Self;
    /// Set the basic authentication header for the request.
    fn basic_auth(self, username: &'a str, password: &'a str) -> Self;
    /// Return an immutable request.
    fn build(self) -> Request<'a>;
}

/// Request authentication scheme.
pub enum Auth<'a> {
    Basic { username: &'a str, password: &'a str },
}

impl<'a> Request<'a> {
    /// Create a new http request.
    #[allow(clippy::new_ret_no_self)]
    pub fn new(method: Method, path: &'a str) -> DefaultRequestBuilder<'a> {
        DefaultRequestBuilder(Request {
            method,
            path,
            ..Default::default()
        })
    }

    /// Create a new GET http request.
    pub fn get(path: &'a str) -> DefaultRequestBuilder<'a> {
        Self::new(Method::GET, path)
    }

    /// Create a new POST http request.
    pub fn post(path: &'a str) -> DefaultRequestBuilder<'a> {
        Self::new(Method::POST, path)
    }

    /// Create a new PUT http request.
    pub fn put(path: &'a str) -> DefaultRequestBuilder<'a> {
        Self::new(Method::PUT, path)
    }

    /// Create a new DELETE http request.
    pub fn delete(path: &'a str) -> DefaultRequestBuilder<'a> {
        Self::new(Method::DELETE, path)
    }

    /// Create a new HEAD http request.
    pub fn head(path: &'a str) -> DefaultRequestBuilder<'a> {
        Self::new(Method::HEAD, path)
    }

    /// Write request to the I/O stream
    pub async fn write<C>(&self, c: &mut C) -> Result<(), Error>
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

pub struct DefaultRequestBuilder<'a>(Request<'a>);

impl<'a> RequestBuilder<'a> for DefaultRequestBuilder<'a> {
    fn headers(mut self, headers: &'a [(&'a str, &'a str)]) -> Self {
        self.0.extra_headers.replace(headers);
        self
    }

    fn path(mut self, path: &'a str) -> Self {
        self.0.path = path;
        self
    }

    fn body(mut self, body: &'a [u8]) -> Self {
        self.0.body.replace(body);
        self
    }

    fn host(mut self, host: &'a str) -> Self {
        self.0.host.replace(host);
        self
    }

    fn content_type(mut self, content_type: ContentType) -> Self {
        self.0.content_type.replace(content_type);
        self
    }

    fn basic_auth(mut self, username: &'a str, password: &'a str) -> Self {
        self.0.auth.replace(Auth::Basic { username, password });
        self
    }

    fn build(self) -> Request<'a> {
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
