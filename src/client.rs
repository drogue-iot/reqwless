use crate::Error;
/// Client using embedded-nal-async traits to establish connections and perform HTTP requests.
///
use crate::body_writer::{BufferingChunkedBodyWriter, ChunkedBodyWriter, FixedBodyWriter};
use crate::headers::ContentType;
use crate::request::*;
use crate::response::*;
use buffered_io::asynch::BufferedWrite;
use core::net::SocketAddr;
use embedded_io::Error as _;
use embedded_io::ErrorType;
use embedded_io_async::{Read, Write};
use embedded_nal_async::{Dns, TcpConnect};
#[cfg(feature = "embedded-tls")]
use embedded_tls::{
    Aes128GcmSha256, CryptoProvider, NoClock, SignatureScheme, TlsError, TlsVerifier, pki::CertVerifier,
};
use nourl::{Url, UrlScheme};
#[cfg(feature = "embedded-tls")]
use p256::ecdsa::{DerSignature, signature::SignerMut};
#[cfg(feature = "embedded-tls")]
use rand_core::CryptoRngCore;

/// An async HTTP client that can establish a TCP connection and perform
/// HTTP requests.
pub struct HttpClient<'a, T, D>
where
    T: TcpConnect + 'a,
    D: Dns + 'a,
{
    client: &'a T,
    dns: &'a D,
    #[cfg(any(feature = "embedded-tls", feature = "esp-mbedtls"))]
    tls: Option<TlsConfig<'a>>,
}

/// Type for TLS configuration of HTTP client.
#[cfg(feature = "esp-mbedtls")]
pub struct TlsConfig<'a, const RX_SIZE: usize = 4096, const TX_SIZE: usize = 4096> {
    /// Minimum TLS version for the connection
    version: crate::TlsVersion,

    /// Client certificates. See [esp_mbedtls::Certificates]
    certificates: crate::Certificates<'a>,

    /// A reference to instance of the MbedTLS library.
    tls_reference: esp_mbedtls::TlsReference<'a>,
}

/// Type for TLS configuration of HTTP client.
#[cfg(feature = "embedded-tls")]
pub struct TlsConfig<'a> {
    seed: u64,
    read_buffer: &'a mut [u8],
    write_buffer: &'a mut [u8],
    verify: TlsVerify<'a>,
}

#[cfg(feature = "embedded-tls")]
struct Provider {
    rng: rand_chacha::ChaCha8Rng,
    verifier: CertVerifier<Aes128GcmSha256, NoClock, 4096>,
}

#[cfg(feature = "embedded-tls")]
impl CryptoProvider for Provider {
    type CipherSuite = Aes128GcmSha256;
    type Signature = DerSignature;

    fn rng(&mut self) -> impl CryptoRngCore {
        &mut self.rng
    }

    fn verifier(&mut self) -> Result<&mut impl TlsVerifier<Self::CipherSuite>, TlsError> {
        Ok(&mut self.verifier)
    }

    fn signer(&mut self, key_der: &[u8]) -> Result<(impl SignerMut<Self::Signature>, SignatureScheme), TlsError> {
        use p256::{SecretKey, ecdsa::SigningKey};

        let secret_key = SecretKey::from_sec1_der(key_der).map_err(|_| TlsError::InvalidPrivateKey)?;

        Ok((SigningKey::from(&secret_key), SignatureScheme::EcdsaSecp256r1Sha256))
    }
}

/// Supported verification modes.
#[cfg(feature = "embedded-tls")]
pub enum TlsVerify<'a> {
    /// No verification of the remote host
    None,
    /// Use pre-shared keys for verifying
    Psk { identity: &'a [u8], psk: &'a [u8] },
    /// Use certificates for verifying
    /// ca: CA cert in DER format
    /// cert: Optional client cert in DER format (needed only for client verification)
    /// key: Optional client privkey in DER format (needed only for client verification)
    Certificate {
        ca: &'a [u8],
        cert: Option<&'a [u8]>,
        key: Option<&'a [u8]>,
    },
}

#[cfg(feature = "embedded-tls")]
impl<'a> TlsConfig<'a> {
    pub fn new(seed: u64, read_buffer: &'a mut [u8], write_buffer: &'a mut [u8], verify: TlsVerify<'a>) -> Self {
        Self {
            seed,
            write_buffer,
            read_buffer,
            verify,
        }
    }
}

#[cfg(feature = "esp-mbedtls")]
impl<'a, const RX_SIZE: usize, const TX_SIZE: usize> TlsConfig<'a, RX_SIZE, TX_SIZE> {
    pub fn new(
        version: crate::TlsVersion,
        certificates: crate::Certificates<'a>,
        tls_reference: crate::TlsReference<'a>,
    ) -> Self {
        Self {
            version,
            certificates,
            tls_reference,
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
        Self {
            client,
            dns,
            #[cfg(any(feature = "embedded-tls", feature = "esp-mbedtls"))]
            tls: None,
        }
    }

    /// Create a new HTTP client for a given connection handle and a target host.
    #[cfg(any(feature = "embedded-tls", feature = "esp-mbedtls"))]
    pub fn new_with_tls(client: &'a T, dns: &'a D, tls: TlsConfig<'a>) -> Self {
        Self {
            client,
            dns,
            tls: Some(tls),
        }
    }

    async fn connect<'conn>(
        &'conn mut self,
        url: &Url<'_>,
    ) -> Result<HttpConnection<'conn, T::Connection<'conn>>, Error> {
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
            .map_err(|e| e.kind())?;

        if url.scheme() == UrlScheme::HTTPS {
            #[cfg(feature = "esp-mbedtls")]
            if let Some(tls) = self.tls.as_mut() {
                let mut servername = host.as_bytes().to_vec();
                servername.push(0);
                let mut session = esp_mbedtls::asynch::Session::new(
                    conn,
                    esp_mbedtls::Mode::Client {
                        servername: unsafe { core::ffi::CStr::from_bytes_with_nul_unchecked(&servername) },
                    },
                    tls.version,
                    tls.certificates,
                    tls.tls_reference,
                )?;

                session.connect().await?;
                Ok(HttpConnection::Tls(session))
            } else {
                Ok(HttpConnection::Plain(conn))
            }

            #[cfg(feature = "embedded-tls")]
            if let Some(tls) = self.tls.as_mut() {
                use embedded_tls::{TlsConfig, TlsContext, UnsecureProvider};
                use rand_chacha::ChaCha8Rng;
                use rand_core::SeedableRng;
                let rng = ChaCha8Rng::seed_from_u64(tls.seed);
                let mut config = TlsConfig::new().with_server_name(url.host());

                let mut conn: embedded_tls::TlsConnection<'conn, T::Connection<'conn>, embedded_tls::Aes128GcmSha256> =
                    embedded_tls::TlsConnection::new(conn, tls.read_buffer, tls.write_buffer);

                match tls.verify {
                    TlsVerify::None => {
                        use embedded_tls::UnsecureProvider;
                        conn.open(TlsContext::new(&config, UnsecureProvider::new(rng))).await?;
                    }
                    TlsVerify::Psk { identity, psk } => {
                        use embedded_tls::UnsecureProvider;
                        config = config.with_psk(psk, &[identity]);
                        conn.open(TlsContext::new(&config, UnsecureProvider::new(rng))).await?;
                    }
                    TlsVerify::Certificate { ca, cert, key } => {
                        use embedded_tls::Certificate;

                        config = config.with_ca(Certificate::X509(ca));

                        if let Some(cert) = cert {
                            config = config.with_cert(Certificate::X509(cert));
                        }

                        if let Some(key) = key {
                            let k = pkcs8::PrivateKeyInfo::try_from(key).map_err(|_| TlsError::InvalidPrivateKey)?;
                            config = config.with_priv_key(k.private_key);
                        }

                        conn.open(TlsContext::new(
                            &config,
                            Provider {
                                rng: rng,
                                verifier: embedded_tls::pki::CertVerifier::new(),
                            },
                        ))
                        .await?;
                    }
                }

                Ok(HttpConnection::Tls(conn))
            } else {
                Ok(HttpConnection::Plain(conn))
            }
            #[cfg(all(not(feature = "embedded-tls"), not(feature = "esp-mbedtls")))]
            Err(Error::InvalidUrl(nourl::Error::UnsupportedScheme))
        } else {
            #[cfg(feature = "embedded-tls")]
            match self.tls.as_mut() {
                Some(tls) => Ok(HttpConnection::PlainBuffered(BufferedWrite::new(
                    conn,
                    tls.write_buffer,
                ))),
                None => Ok(HttpConnection::Plain(conn)),
            }
            #[cfg(not(feature = "embedded-tls"))]
            Ok(HttpConnection::Plain(conn))
        }
    }

    /// Create a single http request.
    pub async fn request<'conn>(
        &'conn mut self,
        method: Method,
        url: &'conn str,
    ) -> Result<HttpRequestHandle<'conn, T::Connection<'conn>, ()>, Error> {
        let url = Url::parse(url)?;
        let conn = self.connect(&url).await?;
        Ok(HttpRequestHandle {
            conn,
            request: Some(Request::new(method, url.path()).host(url.host())),
        })
    }

    /// Create a connection to a server with the provided `resource_url`.
    /// The path in the url is considered the base path for subsequent requests.
    pub async fn resource<'res>(
        &'res mut self,
        resource_url: &'res str,
    ) -> Result<HttpResource<'res, T::Connection<'res>>, Error> {
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
#[allow(clippy::large_enum_variant)]
pub enum HttpConnection<'conn, C>
where
    C: Read + Write,
{
    Plain(C),
    PlainBuffered(BufferedWrite<'conn, C>),
    #[cfg(feature = "esp-mbedtls")]
    Tls(esp_mbedtls::asynch::Session<'conn, C>),
    #[cfg(feature = "embedded-tls")]
    Tls(embedded_tls::TlsConnection<'conn, C, embedded_tls::Aes128GcmSha256>),
    #[cfg(all(not(feature = "embedded-tls"), not(feature = "esp-mbedtls")))]
    Tls((&'conn mut (), core::convert::Infallible)), // Variant is impossible to create, but we need it to avoid "unused lifetime" warning
}

#[cfg(feature = "defmt")]
impl<C> defmt::Format for HttpConnection<'_, C>
where
    C: Read + Write,
{
    fn format(&self, fmt: defmt::Formatter) {
        match self {
            HttpConnection::Plain(_) => defmt::write!(fmt, "Plain"),
            HttpConnection::PlainBuffered(_) => defmt::write!(fmt, "PlainBuffered"),
            HttpConnection::Tls(_) => defmt::write!(fmt, "Tls"),
        }
    }
}

impl<C> core::fmt::Debug for HttpConnection<'_, C>
where
    C: Read + Write,
{
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        match self {
            HttpConnection::Plain(_) => f.debug_tuple("Plain").finish(),
            HttpConnection::PlainBuffered(_) => f.debug_tuple("PlainBuffered").finish(),
            HttpConnection::Tls(_) => f.debug_tuple("Tls").finish(),
        }
    }
}

impl<'conn, T> HttpConnection<'conn, T>
where
    T: Read + Write,
{
    /// Turn the request into a buffered request.
    ///
    /// This is only relevant if no TLS is used, as `embedded-tls` buffers internally and we reuse
    /// its buffer for non-TLS connections.
    pub fn into_buffered<'buf>(self, tx_buf: &'buf mut [u8]) -> HttpConnection<'buf, T>
    where
        'conn: 'buf,
    {
        match self {
            HttpConnection::Plain(conn) => HttpConnection::PlainBuffered(BufferedWrite::new(conn, tx_buf)),
            HttpConnection::PlainBuffered(conn) => HttpConnection::PlainBuffered(conn),
            HttpConnection::Tls(tls) => HttpConnection::Tls(tls),
        }
    }

    /// Send a request on an established connection.
    ///
    /// The request is sent in its raw form without any base path from the resource.
    /// The response headers are stored in the provided rx_buf, which should be sized to contain at least the response headers.
    ///
    /// The response is returned.
    pub async fn send<'req, 'buf, B: RequestBody>(
        &'conn mut self,
        request: Request<'req, B>,
        rx_buf: &'buf mut [u8],
    ) -> Result<Response<'conn, 'buf, HttpConnection<'conn, T>>, Error> {
        self.write_request(&request).await?;
        self.flush().await?;
        Response::read(self, request.method, rx_buf).await
    }

    async fn write_request<'req, B: RequestBody>(&mut self, request: &Request<'req, B>) -> Result<(), Error> {
        request.write_header(self).await?;

        if let Some(body) = request.body.as_ref() {
            match body.len() {
                Some(0) => {
                    // Empty body
                }
                Some(len) => {
                    trace!("Writing not-chunked body");
                    let mut writer = FixedBodyWriter::new(self);
                    body.write(&mut writer).await.map_err(|e| e.kind())?;

                    if writer.written() != len {
                        return Err(Error::IncorrectBodyWritten);
                    }
                }
                None => {
                    trace!("Writing chunked body");
                    match self {
                        HttpConnection::Plain(c) => {
                            let mut writer = ChunkedBodyWriter::new(c);
                            body.write(&mut writer).await?;
                            writer.terminate().await.map_err(|e| e.kind())?;
                        }
                        HttpConnection::PlainBuffered(buffered) => {
                            let (conn, buf, unwritten) = buffered.split();
                            let mut writer = BufferingChunkedBodyWriter::new_with_data(conn, buf, unwritten);
                            body.write(&mut writer).await?;
                            writer.terminate().await.map_err(|e| e.kind())?;
                            buffered.clear();
                        }
                        #[cfg(any(feature = "embedded-tls", feature = "esp-mbedtls"))]
                        HttpConnection::Tls(c) => {
                            let mut writer = ChunkedBodyWriter::new(c);
                            body.write(&mut writer).await?;
                            writer.terminate().await.map_err(|e| e.kind())?;
                        }
                        #[cfg(all(not(feature = "embedded-tls"), not(feature = "esp-mbedtls")))]
                        HttpConnection::Tls(_) => unreachable!(),
                    };
                }
            }
        }
        Ok(())
    }
}

impl<T> ErrorType for HttpConnection<'_, T>
where
    T: Read + Write,
{
    type Error = embedded_io::ErrorKind;
}

impl<T> Read for HttpConnection<'_, T>
where
    T: Read + Write,
{
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        match self {
            Self::Plain(conn) => conn.read(buf).await.map_err(|e| e.kind()),
            Self::PlainBuffered(conn) => conn.read(buf).await.map_err(|e| e.kind()),
            #[cfg(any(feature = "embedded-tls", feature = "esp-mbedtls"))]
            Self::Tls(conn) => conn.read(buf).await.map_err(|e| e.kind()),
            #[cfg(not(any(feature = "embedded-tls", feature = "esp-mbedtls")))]
            _ => unreachable!(),
        }
    }
}

impl<T> Write for HttpConnection<'_, T>
where
    T: Read + Write,
{
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        match self {
            Self::Plain(conn) => conn.write(buf).await.map_err(|e| e.kind()),
            Self::PlainBuffered(conn) => conn.write(buf).await.map_err(|e| e.kind()),
            #[cfg(any(feature = "embedded-tls", feature = "esp-mbedtls"))]
            Self::Tls(conn) => conn.write(buf).await.map_err(|e| e.kind()),
            #[cfg(not(any(feature = "embedded-tls", feature = "esp-mbedtls")))]
            _ => unreachable!(),
        }
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        match self {
            Self::Plain(conn) => conn.flush().await.map_err(|e| e.kind()),
            Self::PlainBuffered(conn) => conn.flush().await.map_err(|e| e.kind()),
            #[cfg(any(feature = "embedded-tls", feature = "esp-mbedtls"))]
            Self::Tls(conn) => conn.flush().await.map_err(|e| e.kind()),
            #[cfg(not(any(feature = "embedded-tls", feature = "esp-mbedtls")))]
            _ => unreachable!(),
        }
    }
}

/// A HTTP request handle
///
/// The underlying connection is closed when drop'ed.
pub struct HttpRequestHandle<'conn, C, B>
where
    C: Read + Write,
    B: RequestBody,
{
    pub conn: HttpConnection<'conn, C>,
    request: Option<DefaultRequestBuilder<'conn, B>>,
}

impl<'conn, C, B> HttpRequestHandle<'conn, C, B>
where
    C: Read + Write,
    B: RequestBody,
{
    /// Turn the request into a buffered request.
    ///
    /// This is only relevant if no TLS is used, as `embedded-tls` buffers internally and we reuse
    /// its buffer for non-TLS connections.
    pub fn into_buffered<'buf>(self, tx_buf: &'buf mut [u8]) -> HttpRequestHandle<'buf, C, B>
    where
        'conn: 'buf,
    {
        HttpRequestHandle {
            conn: self.conn.into_buffered(tx_buf),
            request: self.request,
        }
    }

    /// Send the request.
    ///
    /// The response headers are stored in the provided rx_buf, which should be sized to contain at least the response headers.
    ///
    /// The response is returned.
    pub async fn send<'req, 'buf>(
        &'req mut self,
        rx_buf: &'buf mut [u8],
    ) -> Result<Response<'req, 'buf, HttpConnection<'conn, C>>, Error> {
        let request = self.request.take().ok_or(Error::AlreadySent)?.build();
        self.conn.write_request(&request).await?;
        self.conn.flush().await?;
        Response::read(&mut self.conn, request.method, rx_buf).await
    }
}

impl<'m, C, B> RequestBuilder<'m, B> for HttpRequestHandle<'m, C, B>
where
    C: Read + Write,
    B: RequestBody,
{
    type WithBody<T: RequestBody> = HttpRequestHandle<'m, C, T>;

    fn headers(mut self, headers: &'m [(&'m str, &'m str)]) -> Self {
        self.request = Some(self.request.unwrap().headers(headers));
        self
    }

    fn path(mut self, path: &'m str) -> Self {
        self.request = Some(self.request.unwrap().path(path));
        self
    }

    fn body<T: RequestBody>(self, body: T) -> Self::WithBody<T> {
        HttpRequestHandle {
            conn: self.conn,
            request: Some(self.request.unwrap().body(body)),
        }
    }

    fn host(mut self, host: &'m str) -> Self {
        self.request = Some(self.request.unwrap().host(host));
        self
    }

    fn content_type(mut self, content_type: ContentType) -> Self {
        self.request = Some(self.request.unwrap().content_type(content_type));
        self
    }

    fn accept(mut self, content_type: ContentType) -> Self {
        self.request = Some(self.request.unwrap().accept(content_type));
        self
    }

    fn basic_auth(mut self, username: &'m str, password: &'m str) -> Self {
        self.request = Some(self.request.unwrap().basic_auth(username, password));
        self
    }

    fn build(self) -> Request<'m, B> {
        self.request.unwrap().build()
    }
}

/// A HTTP resource describing a scoped endpoint
///
/// The underlying connection is closed when drop'ed.
pub struct HttpResource<'res, C>
where
    C: Read + Write,
{
    pub conn: HttpConnection<'res, C>,
    pub host: &'res str,
    pub base_path: &'res str,
}

impl<'res, C> HttpResource<'res, C>
where
    C: Read + Write,
{
    /// Turn the resource into a buffered resource
    ///
    /// This is only relevant if no TLS is used, as `embedded-tls` buffers internally and we reuse
    /// its buffer for non-TLS connections.
    pub fn into_buffered<'buf>(self, tx_buf: &'buf mut [u8]) -> HttpResource<'buf, C>
    where
        'res: 'buf,
    {
        HttpResource {
            conn: self.conn.into_buffered(tx_buf),
            host: self.host,
            base_path: self.base_path,
        }
    }

    pub fn request<'req>(
        &'req mut self,
        method: Method,
        path: &'req str,
    ) -> HttpResourceRequestBuilder<'req, 'res, C, ()> {
        HttpResourceRequestBuilder {
            conn: &mut self.conn,
            request: Request::new(method, path).host(self.host),
            base_path: self.base_path,
        }
    }

    /// Create a new scoped GET http request.
    pub fn get<'req>(&'req mut self, path: &'req str) -> HttpResourceRequestBuilder<'req, 'res, C, ()> {
        self.request(Method::GET, path)
    }

    /// Create a new scoped POST http request.
    pub fn post<'req>(&'req mut self, path: &'req str) -> HttpResourceRequestBuilder<'req, 'res, C, ()> {
        self.request(Method::POST, path)
    }

    /// Create a new scoped PUT http request.
    pub fn put<'req>(&'req mut self, path: &'req str) -> HttpResourceRequestBuilder<'req, 'res, C, ()> {
        self.request(Method::PUT, path)
    }

    /// Create a new scoped DELETE http request.
    pub fn delete<'req>(&'req mut self, path: &'req str) -> HttpResourceRequestBuilder<'req, 'res, C, ()> {
        self.request(Method::DELETE, path)
    }

    /// Create a new scoped HEAD http request.
    pub fn head<'req>(&'req mut self, path: &'req str) -> HttpResourceRequestBuilder<'req, 'res, C, ()> {
        self.request(Method::HEAD, path)
    }

    /// Send a request to a resource.
    ///
    /// The base path of the resource is prepended to the request path.
    /// The response headers are stored in the provided rx_buf, which should be sized to contain at least the response headers.
    ///
    /// The response is returned.
    pub async fn send<'req, 'buf, B: RequestBody>(
        &'req mut self,
        mut request: Request<'req, B>,
        rx_buf: &'buf mut [u8],
    ) -> Result<Response<'req, 'buf, HttpConnection<'res, C>>, Error> {
        request.base_path = Some(self.base_path);
        self.conn.write_request(&request).await?;
        self.conn.flush().await?;
        Response::read(&mut self.conn, request.method, rx_buf).await
    }
}

pub struct HttpResourceRequestBuilder<'req, 'conn, C, B>
where
    C: Read + Write,
    B: RequestBody,
{
    conn: &'req mut HttpConnection<'conn, C>,
    base_path: &'req str,
    request: DefaultRequestBuilder<'req, B>,
}

impl<'req, 'conn, C, B> HttpResourceRequestBuilder<'req, 'conn, C, B>
where
    C: Read + Write,
    B: RequestBody,
{
    /// Send the request.
    ///
    /// The base path of the resource is prepended to the request path.
    /// The response headers are stored in the provided rx_buf, which should be sized to contain at least the response headers.
    ///
    /// The response is returned.
    pub async fn send<'buf>(
        self,
        rx_buf: &'buf mut [u8],
    ) -> Result<Response<'req, 'buf, HttpConnection<'conn, C>>, Error> {
        let conn = self.conn;
        let mut request = self.request.build();
        request.base_path = Some(self.base_path);
        conn.write_request(&request).await?;
        conn.flush().await?;
        Response::read(conn, request.method, rx_buf).await
    }
}

impl<'req, 'conn, C, B> RequestBuilder<'req, B> for HttpResourceRequestBuilder<'req, 'conn, C, B>
where
    C: Read + Write,
    B: RequestBody,
{
    type WithBody<T: RequestBody> = HttpResourceRequestBuilder<'req, 'conn, C, T>;

    fn headers(mut self, headers: &'req [(&'req str, &'req str)]) -> Self {
        self.request = self.request.headers(headers);
        self
    }

    fn path(mut self, path: &'req str) -> Self {
        self.request = self.request.path(path);
        self
    }

    fn body<T: RequestBody>(self, body: T) -> Self::WithBody<T> {
        HttpResourceRequestBuilder {
            conn: self.conn,
            base_path: self.base_path,
            request: self.request.body(body),
        }
    }

    fn host(mut self, host: &'req str) -> Self {
        self.request = self.request.host(host);
        self
    }

    fn content_type(mut self, content_type: ContentType) -> Self {
        self.request = self.request.content_type(content_type);
        self
    }

    fn accept(mut self, content_type: ContentType) -> Self {
        self.request = self.request.accept(content_type);
        self
    }

    fn basic_auth(mut self, username: &'req str, password: &'req str) -> Self {
        self.request = self.request.basic_auth(username, password);
        self
    }

    fn build(self) -> Request<'req, B> {
        self.request.build()
    }
}

#[cfg(test)]
mod tests {
    use core::convert::Infallible;

    use super::*;

    #[derive(Default)]
    struct VecBuffer(Vec<u8>);

    impl ErrorType for VecBuffer {
        type Error = Infallible;
    }

    impl Read for VecBuffer {
        async fn read(&mut self, _buf: &mut [u8]) -> Result<usize, Self::Error> {
            unreachable!()
        }
    }

    impl Write for VecBuffer {
        async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
            self.0.extend_from_slice(buf);
            Ok(buf.len())
        }

        async fn flush(&mut self) -> Result<(), Self::Error> {
            self.0.flush().await
        }
    }

    #[tokio::test]
    async fn with_empty_body() {
        let mut buffer = VecBuffer::default();
        let mut conn = HttpConnection::Plain(&mut buffer);

        let request = Request::new(Method::POST, "/").body([].as_slice()).build();
        conn.write_request(&request).await.unwrap();

        assert_eq!(b"POST / HTTP/1.1\r\nContent-Length: 0\r\n\r\n", buffer.0.as_slice());
    }

    #[tokio::test]
    async fn with_known_body() {
        let mut buffer = VecBuffer::default();
        let mut conn = HttpConnection::Plain(&mut buffer);

        let request = Request::new(Method::POST, "/").body(b"BODY".as_slice()).build();
        conn.write_request(&request).await.unwrap();

        assert_eq!(b"POST / HTTP/1.1\r\nContent-Length: 4\r\n\r\nBODY", buffer.0.as_slice());
    }

    struct ChunkedBody(&'static [&'static [u8]]);

    impl RequestBody for ChunkedBody {
        fn len(&self) -> Option<usize> {
            None // Unknown length: triggers chunked body
        }

        async fn write<W: Write>(&self, writer: &mut W) -> Result<(), W::Error> {
            for chunk in self.0 {
                writer.write_all(chunk).await?;
            }
            Ok(())
        }
    }

    #[tokio::test]
    async fn with_unknown_body_unbuffered() {
        let mut buffer = VecBuffer::default();
        let mut conn = HttpConnection::Plain(&mut buffer);

        static CHUNKS: [&'static [u8]; 2] = [b"PART1", b"PART2"];
        let request = Request::new(Method::POST, "/").body(ChunkedBody(&CHUNKS)).build();
        conn.write_request(&request).await.unwrap();

        assert_eq!(
            b"POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nPART1\r\n5\r\nPART2\r\n0\r\n\r\n",
            buffer.0.as_slice()
        );
    }

    #[tokio::test]
    async fn with_unknown_body_buffered() {
        let mut buffer = VecBuffer::default();
        let mut tx_buf = [0; 1024];
        let mut conn = HttpConnection::Plain(&mut buffer).into_buffered(&mut tx_buf);

        static CHUNKS: [&'static [u8]; 2] = [b"PART1", b"PART2"];
        let request = Request::new(Method::POST, "/").body(ChunkedBody(&CHUNKS)).build();
        conn.write_request(&request).await.unwrap();

        assert_eq!(
            b"POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\na\r\nPART1PART2\r\n0\r\n\r\n",
            buffer.0.as_slice()
        );
    }
}
