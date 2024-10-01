use embedded_io_adapters::tokio_1::FromTokio;
use embedded_io_async::{ErrorType, Read, Write};
use embedded_nal_async::AddrType;
use reqwless::TryBufRead;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, ToSocketAddrs};
use tokio::net::TcpStream;

#[derive(Debug)]
pub struct TestError;

impl embedded_io::Error for TestError {
    fn kind(&self) -> embedded_io::ErrorKind {
        embedded_io::ErrorKind::Other
    }
}

pub struct LoopbackDns;
impl embedded_nal_async::Dns for LoopbackDns {
    type Error = TestError;

    async fn get_host_by_name(&self, _: &str, _: AddrType) -> Result<IpAddr, Self::Error> {
        Ok(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)))
    }

    async fn get_host_by_address(&self, _: IpAddr, _: &mut [u8]) -> Result<usize, Self::Error> {
        Err(TestError)
    }
}

pub struct StdDns;

impl embedded_nal_async::Dns for StdDns {
    type Error = std::io::Error;

    async fn get_host_by_name(&self, host: &str, addr_type: AddrType) -> Result<IpAddr, Self::Error> {
        for address in (host, 0).to_socket_addrs()? {
            match address {
                SocketAddr::V4(a) if addr_type == AddrType::IPv4 || addr_type == AddrType::Either => {
                    return Ok(IpAddr::V4(a.ip().octets().into()))
                }
                SocketAddr::V6(a) if addr_type == AddrType::IPv6 || addr_type == AddrType::Either => {
                    return Ok(IpAddr::V6(a.ip().octets().into()))
                }
                _ => {}
            }
        }
        Err(std::io::ErrorKind::AddrNotAvailable.into())
    }

    async fn get_host_by_address(&self, _: IpAddr, _: &mut [u8]) -> Result<usize, Self::Error> {
        todo!()
    }
}

pub struct TokioTcp;
pub struct TokioStream(pub(crate) FromTokio<TcpStream>);

impl TryBufRead for TokioStream {}

impl ErrorType for TokioStream {
    type Error = <FromTokio<TcpStream> as ErrorType>::Error;
}

impl Read for TokioStream {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        self.0.read(buf).await
    }
}

impl Write for TokioStream {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        self.0.write(buf).await
    }
}

impl embedded_nal_async::TcpConnect for TokioTcp {
    type Error = std::io::Error;
    type Connection<'m> = TokioStream;

    async fn connect<'m>(&'m self, remote: SocketAddr) -> Result<Self::Connection<'m>, Self::Error> {
        let ip = match remote {
            SocketAddr::V4(a) => a.ip().octets().into(),
            SocketAddr::V6(a) => a.ip().octets().into(),
        };
        let remote = SocketAddr::new(ip, remote.port());
        let stream = TcpStream::connect(remote).await?;
        let stream = FromTokio::new(stream);
        Ok(TokioStream(stream))
    }
}
