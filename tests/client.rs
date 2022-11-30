#![feature(async_fn_in_trait)]
#![feature(impl_trait_projections)]
#![allow(incomplete_features)]
use embedded_io::adapters::FromTokio;
use embedded_nal_async::{AddrType, IpAddr, Ipv4Addr};
use hyper::server::conn::Http;
use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Server};
use rand::rngs::OsRng;
use rand::RngCore;
use reqwless::client::{HttpClient, TlsConfig};
use reqwless::request::{ContentType, Method};
use std::net::SocketAddr;
use std::sync::Once;
use tokio::net::TcpListener;
use tokio::net::TcpStream;
use tokio::sync::oneshot;
use tokio_rustls::rustls;
use tokio_rustls::TlsAcceptor;

static INIT: Once = Once::new();

fn setup() {
    INIT.call_once(|| {
        env_logger::init();
    });
}

static TCP: TokioTcp = TokioTcp;
static DNS: StaticDns = StaticDns;

#[tokio::test]
async fn test_request_response_notls() {
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

    let url = format!("http://127.0.0.1:{}", addr.port());
    let mut client = HttpClient::new(&TCP, &DNS);
    let mut rx_buf = [0; 4096];
    let response = client
        .request(Method::POST, &url)
        .await
        .unwrap()
        .body(b"PING")
        .content_type(ContentType::TextPlain)
        .send(&mut rx_buf)
        .await
        .unwrap();
    assert_eq!(response.body.unwrap(), b"PING");

    tx.send(()).unwrap();
    t.await.unwrap();
}

#[tokio::test]
async fn test_request_response_rustls() {
    setup();
    let addr: SocketAddr = ([127, 0, 0, 1], 0).into();

    let test_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests");
    let certs = load_certs(&test_dir.join("certs").join("cert.pem"));
    let privkey = load_private_key(&test_dir.join("certs").join("key.pem"));

    let versions = &[&rustls::version::TLS13];
    let config = rustls::ServerConfig::builder()
        .with_cipher_suites(rustls::ALL_CIPHER_SUITES)
        .with_kx_groups(&rustls::ALL_KX_GROUPS)
        .with_protocol_versions(versions)
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(certs, privkey)
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidInput, err))
        .unwrap();
    let acceptor = TlsAcceptor::from(std::sync::Arc::new(config));

    let listener = TcpListener::bind(&addr).await.unwrap();
    let addr = listener.local_addr().unwrap();

    let (tx, rx) = oneshot::channel();
    let t = tokio::spawn(async move {
        tokio::select! {
            _ = async move {
                let (stream, _) = listener.accept().await.unwrap();
                let stream = acceptor.accept(stream).await.unwrap();
                Http::new()
                    .http1_only(true)
                    .http1_keep_alive(true)
                    .serve_connection(stream, service_fn(echo))
                    .await.unwrap();
        } => {}
            _ = rx => {}
        }
    });

    let mut tls_buf: [u8; 16384] = [0; 16384];
    let url = format!("https://localhost:{}", addr.port());
    let mut client = HttpClient::new_with_tls(&TCP, &DNS, TlsConfig::new(OsRng.next_u64(), &mut tls_buf));
    let mut rx_buf = [0; 4096];
    let response = client
        .request(Method::POST, &url)
        .await
        .unwrap()
        .body(b"PING")
        .content_type(ContentType::TextPlain)
        .send(&mut rx_buf)
        .await
        .unwrap();
    assert_eq!(response.body.unwrap(), b"PING");

    tx.send(()).unwrap();
    t.await.unwrap();
}

fn load_certs(filename: &std::path::PathBuf) -> Vec<rustls::Certificate> {
    let certfile = std::fs::File::open(filename).expect("cannot open certificate file");
    let mut reader = std::io::BufReader::new(certfile);
    rustls_pemfile::certs(&mut reader)
        .unwrap()
        .iter()
        .map(|v| rustls::Certificate(v.clone()))
        .collect()
}

fn load_private_key(filename: &std::path::PathBuf) -> rustls::PrivateKey {
    let keyfile = std::fs::File::open(filename).expect("cannot open private key file");
    let mut reader = std::io::BufReader::new(keyfile);

    loop {
        match rustls_pemfile::read_one(&mut reader).expect("cannot parse private key .pem file") {
            Some(rustls_pemfile::Item::RSAKey(key)) => return rustls::PrivateKey(key),
            Some(rustls_pemfile::Item::PKCS8Key(key)) => return rustls::PrivateKey(key),
            None => break,
            _ => {}
        }
    }

    panic!("no keys found in {:?} (encrypted keys not supported)", filename);
}

struct StaticDns;
impl embedded_nal_async::Dns for StaticDns {
    type Error = TestError;

    async fn get_host_by_name<'m>(&self, _: &str, _: AddrType) -> Result<IpAddr, Self::Error> {
        Ok(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)))
    }

    async fn get_host_by_address(&self, _: IpAddr) -> Result<heapless::String<256>, Self::Error> {
        Err(TestError)
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

    async fn connect<'m>(&self, remote: embedded_nal_async::SocketAddr) -> Result<Self::Connection<'m>, Self::Error> {
        let remote = SocketAddr::new(
            std::net::IpAddr::V4(std::net::Ipv4Addr::new(127, 0, 0, 1)),
            remote.port(),
        );
        let stream = TcpStream::connect(remote).await?;
        let stream = FromTokio::new(stream);
        Ok(stream)
    }
}

async fn echo(req: hyper::Request<Body>) -> Result<hyper::Response<Body>, hyper::Error> {
    match (req.method(), req.uri().path()) {
        _ => Ok(hyper::Response::new(req.into_body())),
    }
}
