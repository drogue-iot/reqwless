/// Low level API for encoding requests and decoding responses.
use crate::headers::ContentType;
use crate::Error;
use core::fmt::Write as _;
use embedded_io::Error as _;
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
    pub(crate) accept: Option<ContentType>,
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
            accept: None,
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
    /// Set the accept header for the request.
    fn accept(self, content_type: ContentType) -> Self;
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
        if let Some(accept) = &self.accept {
            write_header(c, "Accept", accept.as_str()).await?;
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
            accept: self.0.accept,
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

    fn accept(mut self, content_type: ContentType) -> Self {
        self.0.accept.replace(content_type);
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

#[derive(Clone, Copy, Debug, PartialEq)]
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

#[cfg(test)]
mod tests {
    use super::*;

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

    #[tokio::test]
    async fn with_accept_header() {
        let mut buffer: Vec<u8> = Vec::new();

        Request::new(Method::GET, "/")
            .accept(ContentType::ApplicationJson)
            .build()
            .write_header(&mut buffer)
            .await
            .unwrap();

        assert_eq!(b"GET / HTTP/1.1\r\nAccept: application/json\r\n\r\n", buffer.as_slice());
    }
}
