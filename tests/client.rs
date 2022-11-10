#![feature(type_alias_impl_trait)]
use core::future::Future;
use embedded_io::adapters::FromTokio;
use embedded_nal_async::{AddrType, IpAddr, Ipv4Addr};
use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Server};
use reqwless::client::HttpClient;
use reqwless::request::{ContentType, Request};
use std::net::SocketAddr;
use std::sync::Once;
use tokio::net::TcpStream;
use tokio::sync::oneshot;

static INIT: Once = Once::new();

fn setup() {
    INIT.call_once(|| {
        env_logger::init();
    });
}

#[tokio::test]
async fn test_request_response() {
    setup();
    let addr = ([127, 0, 0, 1], 0).into();

    let service = make_service_fn(|_| async { Ok::<_, hyper::Error>(service_fn(echo)) });

    let server = Server::bind(&addr).serve(service);
    let addr = server.local_addr();

    let (tx, rx) = oneshot::channel();
    let t = tokio::spawn(async move {
        tokio::select! {
            _ = server => {}
            _ = rx => {}
        }
    });

    let request = Request::post()
        .payload(b"PING")
        .content_type(ContentType::TextPlain)
        .build();

    let mut client = HttpClient::new(TokioTcp, StaticDns);
    let mut connection = client.connect("localhost", addr.port()).await.unwrap();
    let mut rx_buf = [0; 4096];
    let response = connection.request(request, &mut rx_buf).await.unwrap();
    assert_eq!(response.payload.unwrap(), b"PING");

    tx.send(()).unwrap();
    t.await.unwrap();
}

struct StaticDns;
impl embedded_nal_async::Dns for StaticDns {
    type Error = TestError;
    type GetHostByNameFuture<'m> = impl Future<Output = Result<IpAddr, Self::Error>>
    where
        Self: 'm;

    fn get_host_by_name<'m>(&'m self, _: &'m str, _: AddrType) -> Self::GetHostByNameFuture<'m> {
        async move { Ok(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))) }
    }

    type GetHostByAddressFuture<'m>
    = impl Future<Output = Result<heapless::String<256>, Self::Error>>
    where
        Self: 'm;
    fn get_host_by_address<'m>(&'m self, _: IpAddr) -> Self::GetHostByAddressFuture<'m> {
        async move { todo!() }
    }
}

struct TokioTcp;
#[derive(Debug)]
struct TestError;

impl embedded_io::Error for TestError {
    fn kind(&self) -> embedded_io::ErrorKind {
        embedded_io::ErrorKind::Other
    }
}

impl embedded_nal_async::TcpConnect for TokioTcp {
    type Error = std::io::Error;
    type Connection<'m> = FromTokio<TcpStream>;
    type ConnectFuture<'m> = impl Future<Output = Result<Self::Connection<'m>, Self::Error>> + 'm where Self: 'm;
    fn connect<'m>(&'m self, remote: embedded_nal_async::SocketAddr) -> Self::ConnectFuture<'m> {
        async move {
            let remote = SocketAddr::new(
                std::net::IpAddr::V4(std::net::Ipv4Addr::new(127, 0, 0, 1)),
                remote.port(),
            );
            let stream = TcpStream::connect(remote).await?;
            let stream = FromTokio::new(stream);
            Ok(stream)
        }
    }
}

async fn echo(req: hyper::Request<Body>) -> Result<hyper::Response<Body>, hyper::Error> {
    match (req.method(), req.uri().path()) {
        _ => Ok(hyper::Response::new(req.into_body())),
    }
}
