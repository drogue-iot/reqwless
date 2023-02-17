use crate::headers::ContentType;
use crate::request::*;
use crate::response::*;
use crate::Error;
use embedded_io::asynch::{Read, Write};
use embedded_io::Error as _;
use embedded_nal_async::{Dns, SocketAddr, TcpConnect};
use embedded_tls::{Aes128GcmSha256, TlsConnection};
/// Client using embedded-nal-async traits to establish connections and perform HTTP requests.
///
use nourl::{Url, UrlScheme};
use rand_chacha::ChaCha8Rng;
use rand_core::{RngCore, SeedableRng};

/// An async HTTP client that can establish a TCP connection and perform
/// HTTP requests.
pub struct HttpClient<'a, T, D>
where
    T: TcpConnect + 'a,
    D: Dns + 'a,
{
    client: &'a T,
    dns: &'a D,
    tls: Option<TlsConfig<'a>>,
}

/// Type for TLS configuration of HTTP client.
pub struct TlsConfig<'a> {
    seed: u64,
    read_buffer: &'a mut [u8],
    write_buffer: &'a mut [u8],
    verify: TlsVerify<'a>,
}

/// Supported verification modes.
pub enum TlsVerify<'a> {
    /// No verification of the remote host
    None,
    /// Use pre-shared keys for verifying
    Psk { identity: &'a [u8], psk: &'a [u8] },
}

impl<'a> TlsConfig<'a> {
    /// Create a new TLS configuration
    /// The buffers are forwarded to [`embedded_tls::asynch::TlsConnection::new()`]. Consult its documentation to see
    /// the details about the required buffer sizes.
    pub fn new(seed: u64, read_buffer: &'a mut [u8], write_buffer: &'a mut [u8], verify: TlsVerify<'a>) -> Self {
        Self {
            seed,
            read_buffer,
            write_buffer,
            verify,
        }
    }
}

impl<'a, T, D> HttpClient<'a, T, D>
where
    T: TcpConnect + 'a,
    D: Dns + 'a,
{
    /// Create a new HTTP client for a given connection handle and a target host.
    pub fn new(client: &'a T, dns: &'a D) -> Self {
        Self { client, dns, tls: None }
    }

    /// Create a new HTTP client for a given connection handle and a target host.
    pub fn new_with_tls(client: &'a T, dns: &'a D, tls: TlsConfig<'a>) -> Self {
        Self {
            client,
            dns,
            tls: Some(tls),
        }
    }

    async fn connect<'m>(
        &'m mut self,
        url: &Url<'m>,
    ) -> Result<HttpConnection<T::Connection<'m>, TlsConnection<'m, T::Connection<'m>, Aes128GcmSha256>>, Error> {
        let host = url.host();
        let port = url.port_or_default();

        let remote = self
            .dns
            .get_host_by_name(host, embedded_nal_async::AddrType::Either)
            .await
            .map_err(|_| Error::Dns)?;

        let conn = self
            .client
            .connect(SocketAddr::new(remote, port))
            .await
            .map_err(|e| Error::Network(e.kind()))?;

        if url.scheme() == UrlScheme::HTTPS {
            if let Some(tls) = self.tls.as_mut() {
                use embedded_tls::{TlsConfig, TlsContext};
                let mut rng = ChaCha8Rng::seed_from_u64(tls.seed);
                tls.seed = rng.next_u64();
                let mut config = TlsConfig::new().with_server_name(url.host());
                if let TlsVerify::Psk { identity, psk } = tls.verify {
                    config = config.with_psk(psk, &[identity]);
                }
                let mut conn: TlsConnection<'m, T::Connection<'m>, Aes128GcmSha256> =
                    TlsConnection::new(conn, tls.read_buffer, tls.write_buffer);
                conn.open::<_, embedded_tls::NoVerify>(TlsContext::new(&config, &mut rng))
                    .await
                    .map_err(Error::Tls)?;
                Ok(HttpConnection::Tls(conn))
            } else {
                Ok(HttpConnection::Plain(conn))
            }
        } else {
            Ok(HttpConnection::Plain(conn))
        }
    }

    /// Create a single http request.
    pub async fn request<'m>(
        &'m mut self,
        method: Method,
        url: &'m str,
    ) -> Result<
        HttpRequestHandle<'m, HttpConnection<T::Connection<'m>, TlsConnection<'m, T::Connection<'m>, Aes128GcmSha256>>>,
        Error,
    > {
        let url = Url::parse(url)?;
        let conn = self.connect(&url).await?;
        Ok(HttpRequestHandle {
            conn,
            request: Some(Request::new(method, url.path()).host(url.host())),
        })
    }

    /// Create a connection to a server with the provided `resource_url`.
    /// The path in the url is considered the base path for subsequent requests.
    pub async fn resource<'m>(
        &'m mut self,
        resource_url: &'m str,
    ) -> Result<
        HttpResource<'m, HttpConnection<T::Connection<'m>, TlsConnection<'m, T::Connection<'m>, Aes128GcmSha256>>>,
        Error,
    > {
        let resource_url = Url::parse(resource_url)?;
        let conn = self.connect(&resource_url).await?;
        Ok(HttpResource {
            conn,
            host: resource_url.host(),
            base_path: resource_url.path(),
        })
    }
}

/// Represents a HTTP connection that may be encrypted or unencrypted.
pub enum HttpConnection<T, S>
where
    T: Read + Write,
    S: Read + Write,
{
    Plain(T),
    Tls(S),
}

impl<T, S> HttpConnection<T, S>
where
    T: Read + Write,
    S: Read + Write,
{
    /// Send a request on an established connection.
    ///
    /// The request is sent in its raw form without any base path from the resource.
    /// The response headers are stored in the provided rx_buf, which should be sized to contain at least the response headers.
    ///
    /// The response is returned.
    pub async fn send<'buf, 'conn>(
        &'conn mut self,
        request: Request<'conn>,
        rx_buf: &'buf mut [u8],
    ) -> Result<Response<'buf, 'conn, HttpConnection<T, S>>, Error> {
        request.write(self).await?;
        Response::read(self, request.method, rx_buf).await
    }
}

impl<T, S> embedded_io::Io for HttpConnection<T, S>
where
    T: Read + Write,
    S: Read + Write,
{
    type Error = embedded_io::ErrorKind;
}

impl<T, S> Read for HttpConnection<T, S>
where
    T: Read + Write,
    S: Read + Write,
{
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        match self {
            Self::Plain(conn) => conn.read(buf).await.map_err(|e| e.kind()),
            Self::Tls(conn) => conn.read(buf).await.map_err(|e| e.kind()),
        }
    }
}

impl<T, S> Write for HttpConnection<T, S>
where
    T: Read + Write,
    S: Read + Write,
{
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        match self {
            Self::Plain(conn) => conn.write(buf).await.map_err(|e| e.kind()),
            Self::Tls(conn) => conn.write(buf).await.map_err(|e| e.kind()),
        }
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        match self {
            Self::Plain(conn) => conn.flush().await.map_err(|e| e.kind()),
            Self::Tls(conn) => conn.flush().await.map_err(|e| e.kind()),
        }
    }
}

/// A HTTP request handle
///
/// The underlying connection is closed when drop'ed.
pub struct HttpRequestHandle<'a, C>
where
    C: Read + Write,
{
    pub conn: C,
    request: Option<DefaultRequestBuilder<'a>>,
}

impl<C> HttpRequestHandle<'_, C>
where
    C: Read + Write,
{
    /// Send the request.
    ///
    /// The response headers are stored in the provided rx_buf, which should be sized to contain at least the response headers.
    ///
    /// The response is returned.
    pub async fn send<'buf, 'conn>(&'conn mut self, rx_buf: &'buf mut [u8]) -> Result<Response<'buf, 'conn, C>, Error> {
        let request = self.request.take().ok_or(Error::AlreadySent)?.build();
        request.write(&mut self.conn).await?;
        Response::read(&mut self.conn, request.method, rx_buf).await
    }
}

impl<'a, C> RequestBuilder<'a> for HttpRequestHandle<'a, C>
where
    C: Read + Write,
{
    fn headers(mut self, headers: &'a [(&'a str, &'a str)]) -> Self {
        self.request = Some(self.request.unwrap().headers(headers));
        self
    }

    fn path(mut self, path: &'a str) -> Self {
        self.request = Some(self.request.unwrap().path(path));
        self
    }

    fn body(mut self, body: &'a [u8]) -> Self {
        self.request = Some(self.request.unwrap().body(body));
        self
    }

    fn host(mut self, host: &'a str) -> Self {
        self.request = Some(self.request.unwrap().host(host));
        self
    }

    fn content_type(mut self, content_type: ContentType) -> Self {
        self.request = Some(self.request.unwrap().content_type(content_type));
        self
    }

    fn basic_auth(mut self, username: &'a str, password: &'a str) -> Self {
        self.request = Some(self.request.unwrap().basic_auth(username, password));
        self
    }

    fn build(self) -> Request<'a> {
        self.request.unwrap().build()
    }
}

/// A HTTP resource describing a scoped endpoint
///
/// The underlying connection is closed when drop'ed.
pub struct HttpResource<'a, C>
where
    C: Read + Write,
{
    pub conn: C,
    pub host: &'a str,
    pub base_path: &'a str,
}

impl<'a, C> HttpResource<'a, C>
where
    C: Read + Write,
{
    pub fn request<'conn>(&'conn mut self, method: Method, path: &'a str) -> HttpResourceRequestBuilder<'a, 'conn, C> {
        HttpResourceRequestBuilder {
            conn: &mut self.conn,
            request: Request::new(method, path).host(self.host),
            base_path: self.base_path,
        }
    }

    /// Create a new scoped GET http request.
    pub fn get<'conn>(&'conn mut self, path: &'a str) -> HttpResourceRequestBuilder<'a, 'conn, C> {
        self.request(Method::GET, path)
    }

    /// Create a new scoped POST http request.
    pub fn post<'conn>(&'conn mut self, path: &'a str) -> HttpResourceRequestBuilder<'a, 'conn, C> {
        self.request(Method::POST, path)
    }

    /// Create a new scoped PUT http request.
    pub fn put<'conn>(&'conn mut self, path: &'a str) -> HttpResourceRequestBuilder<'a, 'conn, C> {
        self.request(Method::PUT, path)
    }

    /// Create a new scoped DELETE http request.
    pub fn delete<'conn>(&'conn mut self, path: &'a str) -> HttpResourceRequestBuilder<'a, 'conn, C> {
        self.request(Method::DELETE, path)
    }

    /// Create a new scoped HEAD http request.
    pub fn head<'conn>(&'conn mut self, path: &'a str) -> HttpResourceRequestBuilder<'a, 'conn, C> {
        self.request(Method::HEAD, path)
    }

    /// Send a request to a resource.
    ///
    /// The base path of the resource is prepended to the request path.
    /// The response headers are stored in the provided rx_buf, which should be sized to contain at least the response headers.
    ///
    /// The response is returned.
    pub async fn send<'buf, 'conn>(
        &'conn mut self,
        mut request: Request<'a>,
        rx_buf: &'buf mut [u8],
    ) -> Result<Response<'buf, 'conn, C>, Error> {
        request.base_path = Some(self.base_path);
        request.write(&mut self.conn).await?;
        Response::read(&mut self.conn, request.method, rx_buf).await
    }
}

pub struct HttpResourceRequestBuilder<'a, 'conn, C>
where
    C: Read + Write,
{
    conn: &'conn mut C,
    request: DefaultRequestBuilder<'a>,
    base_path: &'a str,
}

impl<'a, 'conn, C> HttpResourceRequestBuilder<'a, 'conn, C>
where
    C: Read + Write,
{
    /// Send the request.
    ///
    /// The base path of the resource is prepended to the request path.
    /// The response headers are stored in the provided rx_buf, which should be sized to contain at least the response headers.
    ///
    /// The response is returned.
    pub async fn send<'buf>(self, rx_buf: &'buf mut [u8]) -> Result<Response<'buf, 'conn, C>, Error> {
        let conn = self.conn;
        let mut request = self.request.build();
        request.base_path = Some(self.base_path);
        request.write(conn).await?;
        Response::read(conn, request.method, rx_buf).await
    }
}

impl<'a, 'conn, C> RequestBuilder<'a> for HttpResourceRequestBuilder<'a, 'conn, C>
where
    C: Read + Write,
{
    fn headers(mut self, headers: &'a [(&'a str, &'a str)]) -> Self {
        self.request = self.request.headers(headers);
        self
    }

    fn path(mut self, path: &'a str) -> Self {
        self.request = self.request.path(path);
        self
    }

    fn body(mut self, body: &'a [u8]) -> Self {
        self.request = self.request.body(body);
        self
    }

    fn host(mut self, host: &'a str) -> Self {
        self.request = self.request.host(host);
        self
    }

    fn content_type(mut self, content_type: ContentType) -> Self {
        self.request = self.request.content_type(content_type);
        self
    }

    fn basic_auth(mut self, username: &'a str, password: &'a str) -> Self {
        self.request = self.request.basic_auth(username, password);
        self
    }

    fn build(self) -> Request<'a> {
        self.request.build()
    }
}
