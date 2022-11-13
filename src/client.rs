use crate::request::*;
/// Client using embedded-nal-async traits to establish connections and perform HTTP requests.
///
use crate::url::{Url, UrlScheme};
use crate::{request, Error};
use core::future::Future;
use embedded_io::asynch::{Read, Write};
use embedded_io::Error as _;
use embedded_nal_async::{Dns, SocketAddr, TcpConnect};
use embedded_tls::{Aes128GcmSha256, TlsConnection};
use rand_core::{CryptoRng, RngCore};

/// An async HTTP client that can establish a TCP connection and perform
/// HTTP requests.
pub struct HttpClient<'a, T, D, TLS = NoTls>
where
    T: TcpConnect + 'a,
    D: Dns + 'a,
    TLS: Tls + 'a,
{
    client: &'a T,
    dns: &'a D,
    tls: TLS,
}

pub trait Tls {
    type RNG: RngCore + CryptoRng;
    fn state(&mut self) -> Option<(&mut Self::RNG, &mut [u8])>;
}

/// Type for signaling no TLS implementation
pub struct NoTls;

impl Tls for NoTls {
    type RNG = NoTls;
    fn state(&mut self) -> Option<(&mut Self::RNG, &mut [u8])> {
        None
    }
}

/// Type for TLS configuration of HTTP client.
pub struct TlsConfig<'a, RNG>
where
    RNG: RngCore + CryptoRng,
{
    rng: &'a mut RNG,
    buffer: &'a mut [u8],
}

impl<'a, RNG> TlsConfig<'a, RNG>
where
    RNG: RngCore + CryptoRng,
{
    pub fn new(rng: &'a mut RNG, buffer: &'a mut [u8]) -> Self {
        Self { rng, buffer }
    }
}

impl<'a, RNG> Tls for TlsConfig<'a, RNG>
where
    RNG: RngCore + CryptoRng,
{
    type RNG = RNG;
    fn state(&mut self) -> Option<(&mut Self::RNG, &mut [u8])> {
        Some((self.rng, self.buffer))
    }
}

impl RngCore for NoTls {
    fn next_u32(&mut self) -> u32 {
        todo!()
    }
    fn next_u64(&mut self) -> u64 {
        todo!()
    }
    fn fill_bytes(&mut self, _: &mut [u8]) {
        todo!()
    }
    fn try_fill_bytes(&mut self, _: &mut [u8]) -> Result<(), rand_core::Error> {
        todo!()
    }
}

impl CryptoRng for NoTls {}

impl<'a, T, D> HttpClient<'a, T, D, NoTls>
where
    T: TcpConnect + 'a,
    D: Dns + 'a,
{
    /// Create a new HTTP client for a given connection handle and a target host.
    pub fn new(client: &'a T, dns: &'a D) -> HttpClient<T, D, NoTls> {
        Self {
            client,
            dns,
            tls: NoTls,
        }
    }
}

impl<'a, T, D, TLS> HttpClient<'a, T, D, TLS>
where
    T: TcpConnect + 'a,
    D: Dns + 'a,
    TLS: Tls,
{
    /// Create a new HTTP client for a given connection handle and a target host.
    pub fn new_with_tls(client: &'a T, dns: &'a D, tls: TLS) -> Self {
        Self { client, dns, tls }
    }

    async fn connect<'m>(
        &'m mut self,
        url: &Url<'m>,
    ) -> Result<HttpConnection<T::Connection<'m>, TlsConnection<'m, T::Connection<'m>, Aes128GcmSha256>>, Error> {
        let host = url.host();
        let port = url.port();

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
            if let Some((rng, buffer)) = self.tls.state() {
                use embedded_tls::{TlsConfig, TlsContext};
                let config = TlsConfig::new().with_server_name(url.host());
                let mut conn: TlsConnection<'m, T::Connection<'m>, Aes128GcmSha256> = TlsConnection::new(conn, buffer);
                conn.open::<_, embedded_tls::NoClock, 0>(TlsContext::new(&config, rng))
                    .await
                    .map_err(|_| Error::Tls)?;
                Ok(HttpConnection::Tls(conn))
            } else {
                Ok(HttpConnection::Plain(conn))
            }
        } else {
            Ok(HttpConnection::Plain(conn))
        }
    }

    /// Build a http request and connect to a HTTP server. The returned request builder can be used to modify request parameters,
    /// before sending the request.
    pub async fn request<'m>(
        &'m mut self,
        method: Method,
        url: &'m str,
    ) -> Result<
        HttpRequestBuilder<HttpConnection<T::Connection<'m>, TlsConnection<'m, T::Connection<'m>, Aes128GcmSha256>>>,
        Error,
    > {
        let url = Url::parse(url)?;
        let builder: request::RequestBuilder<'m> = Request::new(method).path(url.path()).host(url.host());

        let conn = self.connect(&url).await?;
        Ok(HttpRequestBuilder::new(conn, builder))
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

impl<T, S> embedded_io::Io for HttpConnection<T, S>
where
    T: Read + Write,
    S: Read + Write,
{
    type Error = embedded_io::ErrorKind;
}

impl<T, S> embedded_io::asynch::Read for HttpConnection<T, S>
where
    T: Read + Write,
    S: Read + Write,
{
    type ReadFuture<'m> = impl Future<Output = Result<usize, Self::Error>> + 'm
    where
        Self: 'm;

    fn read<'m>(&'m mut self, buf: &'m mut [u8]) -> Self::ReadFuture<'m> {
        async move {
            match self {
                Self::Plain(conn) => conn.read(buf).await.map_err(|e| e.kind()),
                Self::Tls(conn) => conn.read(buf).await.map_err(|e| e.kind()),
            }
        }
    }
}

impl<T, S> embedded_io::asynch::Write for HttpConnection<T, S>
where
    T: Read + Write,
    S: Read + Write,
{
    type WriteFuture<'m> = impl Future<Output = Result<usize, Self::Error>> + 'm
    where
        Self: 'm;

    fn write<'m>(&'m mut self, buf: &'m [u8]) -> Self::WriteFuture<'m> {
        async move {
            match self {
                Self::Plain(conn) => conn.write(buf).await.map_err(|e| e.kind()),
                Self::Tls(conn) => conn.write(buf).await.map_err(|e| e.kind()),
            }
        }
    }

    type FlushFuture<'m> = impl Future<Output = Result<(), Self::Error>> + 'm
    where
        Self: 'm;

    fn flush<'a>(&'a mut self) -> Self::FlushFuture<'a> {
        async move {
            match self {
                Self::Plain(conn) => conn.flush().await.map_err(|e| e.kind()),
                Self::Tls(conn) => conn.flush().await.map_err(|e| e.kind()),
            }
        }
    }
}

/// An async HTTP connection for performing a HTTP request + response roundtrip.
///
/// The connection is closed when drop'ed.
pub struct HttpRequestBuilder<'a, T> {
    conn: T,
    request: RequestBuilder<'a>,
}

impl<'a, T> HttpRequestBuilder<'a, T>
where
    T: Write + Read,
{
    fn new(conn: T, request: RequestBuilder<'a>) -> Self {
        Self { conn, request }
    }

    /// Set optional headers on the request.
    pub fn headers(mut self, headers: &'a [(&'a str, &'a str)]) -> Self {
        self.request = self.request.headers(headers);
        self
    }

    /// Set the data to send in the HTTP request body.
    pub fn body(mut self, body: &'a [u8]) -> Self {
        self.request = self.request.body(body);
        self
    }

    /// Set the content type header for the request.
    pub fn content_type(mut self, content_type: ContentType) -> Self {
        self.request = self.request.content_type(content_type);
        self
    }

    /// Set the basic authentication header for the request.
    pub fn basic_auth(mut self, username: &'a str, password: &'a str) -> Self {
        self.request = self.request.basic_auth(username, password);
        self
    }

    /// Perform a HTTP request. A connection is created using the underlying client,
    /// and the request is written to the connection.
    ///
    /// The response is stored in the provided rx_buf, which should be sized to contain the entire response.
    ///
    /// The returned response references data in the provided `rx_buf` argument.
    pub async fn send<'m>(mut self, rx_buf: &'m mut [u8]) -> Result<Response<'m>, Error> {
        let request = self.request.build();
        request.write(&mut self.conn).await?;
        let response = Response::read(&mut self.conn, rx_buf).await?;
        Ok(response)
    }
}
