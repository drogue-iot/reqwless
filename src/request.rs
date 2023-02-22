use crate::headers::ContentType;
/// Low level API for encoding requests and decoding responses.
use crate::Error;
use core::fmt::Write as _;
use core::mem::size_of;
use embedded_io::asynch::Write;
use embedded_io::{Error as _, Io};
use heapless::String;

/// A read only HTTP request type
pub struct Request<'a, B>
where
    B: RequestBody,
{
    pub(crate) method: Method,
    pub(crate) base_path: Option<&'a str>,
    pub(crate) path: &'a str,
    pub(crate) auth: Option<Auth<'a>>,
    pub(crate) host: Option<&'a str>,
    pub(crate) body: Option<B>,
    pub(crate) content_type: Option<ContentType>,
    pub(crate) extra_headers: Option<&'a [(&'a str, &'a str)]>,
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
pub trait RequestBuilder<'a, B>
where
    B: RequestBody,
{
    type WithBody<T: RequestBody>: RequestBuilder<'a, T>;

    /// Set optional headers on the request.
    fn headers(self, headers: &'a [(&'a str, &'a str)]) -> Self;
    /// Set the path of the HTTP request.
    fn path(self, path: &'a str) -> Self;
    /// Set the data to send in the HTTP request body.
    fn body<T: RequestBody>(self, body: T) -> Self::WithBody<T>;
    /// Set the host header.
    fn host(self, host: &'a str) -> Self;
    /// Set the content type header for the request.
    fn content_type(self, content_type: ContentType) -> Self;
    /// Set the basic authentication header for the request.
    fn basic_auth(self, username: &'a str, password: &'a str) -> Self;
    /// Return an immutable request.
    fn build(self) -> Request<'a, B>;
}

/// Request authentication scheme.
pub enum Auth<'a> {
    Basic { username: &'a str, password: &'a str },
}

impl<'a> Request<'a, ()> {
    /// Create a new http request.
    #[allow(clippy::new_ret_no_self)]
    pub fn new(method: Method, path: &'a str) -> DefaultRequestBuilder<'a, ()> {
        DefaultRequestBuilder(Request {
            method,
            path,
            ..Default::default()
        })
    }

    /// Create a new GET http request.
    pub fn get(path: &'a str) -> DefaultRequestBuilder<'a, ()> {
        Self::new(Method::GET, path)
    }

    /// Create a new POST http request.
    pub fn post(path: &'a str) -> DefaultRequestBuilder<'a, ()> {
        Self::new(Method::POST, path)
    }

    /// Create a new PUT http request.
    pub fn put(path: &'a str) -> DefaultRequestBuilder<'a, ()> {
        Self::new(Method::PUT, path)
    }

    /// Create a new DELETE http request.
    pub fn delete(path: &'a str) -> DefaultRequestBuilder<'a, ()> {
        Self::new(Method::DELETE, path)
    }

    /// Create a new HEAD http request.
    pub fn head(path: &'a str) -> DefaultRequestBuilder<'a, ()> {
        Self::new(Method::HEAD, path)
    }
}

impl<'a, B> Request<'a, B>
where
    B: RequestBody,
{
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
        if let Some(body) = self.body.as_ref() {
            match body.len() {
                Some(0) => {
                    // Empty body
                }
                Some(len) => {
                    trace!("Writing not-chunked body");
                    let mut writer = FixedBodyWriter(c, 0);
                    body.write(&mut writer).await.map_err(|e| Error::Network(e.kind()))?;

                    if writer.1 != len {
                        return Err(Error::IncorrectBodyWritten);
                    }
                }
                None => {
                    trace!("Writing chunked body");
                    let mut writer = ChunkedBodyWriter(c, 0);
                    body.write(&mut writer).await.map_err(|e| Error::Network(e.kind()))?;

                    write_str(c, "0\r\n\r\n").await?;
                }
            }
        }

        c.flush().await.map_err(|e| Error::Network(e.kind()))
    }
}

pub struct DefaultRequestBuilder<'a, B>(Request<'a, B>)
where
    B: RequestBody;

impl<'a, B> RequestBuilder<'a, B> for DefaultRequestBuilder<'a, B>
where
    B: RequestBody,
{
    type WithBody<T: RequestBody> = DefaultRequestBuilder<'a, T>;

    fn headers(mut self, headers: &'a [(&'a str, &'a str)]) -> Self {
        self.0.extra_headers.replace(headers);
        self
    }

    fn path(mut self, path: &'a str) -> Self {
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

    fn build(self) -> Request<'a, B> {
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
    c.write_all(data.as_bytes()).await.map_err(|e| Error::Network(e.kind()))
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

pub struct FixedBodyWriter<'a, C: Write>(&'a mut C, usize);

impl<C> Io for FixedBodyWriter<'_, C>
where
    C: Write,
{
    type Error = C::Error;
}

impl<C> Write for FixedBodyWriter<'_, C>
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

pub struct ChunkedBodyWriter<'a, C: Write>(&'a mut C, usize);

impl<C> Io for ChunkedBodyWriter<'_, C>
where
    C: Write,
{
    type Error = C::Error;
}

impl<C> Write for ChunkedBodyWriter<'_, C>
where
    C: Write,
{
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        self.write_all(buf).await?;
        Ok(buf.len())
    }

    async fn write_all(&mut self, buf: &[u8]) -> Result<(), Self::Error> {
        // Write chunk header
        let len = buf.len();
        let mut hex = [0; 2 * size_of::<usize>()];
        hex::encode_to_slice(len.to_be_bytes(), &mut hex).unwrap();
        let leading_zeros = hex.iter().position(|x| *x != b'0').unwrap_or_default();
        let (_, hex) = hex.split_at(leading_zeros);
        self.0.write_all(hex).await?;
        self.0.write_all(b"\r\n").await?;

        // Write chunk
        self.0.write_all(buf).await?;
        self.1 += len;

        // Write newline
        self.0.write_all(b"\r\n").await?;
        Ok(())
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        self.0.flush().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn basic_auth() {
        let mut buffer = Vec::new();
        Request::new(Method::GET, "/")
            .basic_auth("username", "password")
            .build()
            .write(&mut buffer)
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
            .write(&mut buffer)
            .await
            .unwrap();

        assert_eq!(b"POST / HTTP/1.1\r\nContent-Length: 0\r\n\r\n", buffer.as_slice());
    }

    #[tokio::test]
    async fn with_known_body() {
        let mut buffer = Vec::new();
        Request::new(Method::POST, "/")
            .body(b"BODY".as_slice())
            .build()
            .write(&mut buffer)
            .await
            .unwrap();

        assert_eq!(b"POST / HTTP/1.1\r\nContent-Length: 4\r\n\r\nBODY", buffer.as_slice());
    }

    struct ChunkedBody<'a>(&'a [u8]);

    impl RequestBody for ChunkedBody<'_> {
        fn len(&self) -> Option<usize> {
            None // Unknown length: triggers chunked body
        }

        async fn write<W: Write>(&self, writer: &mut W) -> Result<(), W::Error> {
            writer.write_all(self.0).await
        }
    }

    #[tokio::test]
    async fn with_unknown_body() {
        let mut buffer = Vec::new();

        Request::new(Method::POST, "/")
            .body(ChunkedBody(b"BODY".as_slice()))
            .build()
            .write(&mut buffer)
            .await
            .unwrap();

        assert_eq!(
            b"POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nBODY\r\n0\r\n\r\n",
            buffer.as_slice()
        );
    }
}
