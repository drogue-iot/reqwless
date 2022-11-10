/// Client using embedded-nal-async traits to establish connections and perform HTTP requests.
use crate::Error;
use embedded_io::asynch::{Read, Write};
use embedded_io::Error as _;
use embedded_nal_async::{Dns, SocketAddr, TcpConnect};

use crate::request::*;

/// An async HTTP client that can establish a TCP connection and perform
/// HTTP requests.
pub struct HttpClient<T, D>
where
    T: TcpConnect,
    D: Dns,
{
    client: T,
    dns: D,
}

impl<T, D> HttpClient<T, D>
where
    T: TcpConnect,
    D: Dns,
{
    /// Create a new HTTP client for a given connection handle and a target host.
    pub fn new(client: T, dns: D) -> Self {
        Self { client, dns }
    }

    /// Connect to a HTTP server
    pub async fn connect<'m>(
        &'m mut self,
        host: &'m str,
        port: u16,
    ) -> Result<HttpConnection<T::Connection<'m>>, Error> {
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
        Ok(HttpConnection::new(conn, host))
    }
}

/// An async HTTP connection for performing a HTTP request + response roundtrip.
///
/// The connection is closed when drop'ed.
pub struct HttpConnection<'a, T> {
    conn: T,
    host: &'a str,
}

impl<'a, T> HttpConnection<'a, T>
where
    T: Write + Read,
{
    fn new(conn: T, host: &'a str) -> Self {
        Self { conn, host }
    }

    /// Perform a HTTP request. A connection is created using the underlying client,
    /// and the request is written to the connection.
    ///
    /// The response is stored in the provided rx_buf, which should be sized to contain the entire response.
    ///
    /// The returned response references data in the provided `rx_buf` argument.
    pub async fn request<'m>(
        &'m mut self,
        mut request: Request<'m>,
        rx_buf: &'m mut [u8],
    ) -> Result<Response<'m>, Error> {
        if request.host.is_none() {
            request.host.replace(self.host);
        }
        request.write(&mut self.conn).await?;
        let response = Response::read(&mut self.conn, rx_buf).await?;
        Ok(response)
    }
}
