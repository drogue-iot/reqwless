use embedded_io_async::{BufRead, Write};
use hyper::server::conn::Http;
use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Server};
use rand::rngs::OsRng;
use rand::RngCore;
use reqwless::client::HttpClient;
use reqwless::headers::ContentType;
use reqwless::request::{Method, RequestBody, RequestBuilder};
use reqwless::response::Status;
use std::net::SocketAddr;
use std::sync::Once;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio_rustls::rustls;
use tokio_rustls::TlsAcceptor;

mod connection;

use connection::*;

static INIT: Once = Once::new();

fn setup() {
    INIT.call_once(|| {
        env_logger::init();
    });
}

static TCP: TokioTcp = TokioTcp;
static LOOPBACK_DNS: LoopbackDns = LoopbackDns;
static PUBLIC_DNS: StdDns = StdDns;

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
    let mut client = HttpClient::new(&TCP, &LOOPBACK_DNS);
    let mut rx_buf = [0; 4096];
    for _ in 0..2 {
        let mut request = client
            .request(Method::POST, &url)
            .await
            .unwrap()
            .body(b"PING".as_slice())
            .content_type(ContentType::TextPlain);
        let response = request.send(&mut rx_buf).await.unwrap();
        let body = response.body().read_to_end().await;
        assert_eq!(body.unwrap(), b"PING");
    }

    tx.send(()).unwrap();
    t.await.unwrap();
}

#[tokio::test]
async fn test_resource_notls() {
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
    let mut client = HttpClient::new(&TCP, &LOOPBACK_DNS);
    let mut rx_buf = [0; 4096];
    let mut resource = client.resource(&url).await.unwrap();
    for _ in 0..2 {
        let response = resource
            .post("/")
            .body(b"PING".as_slice())
            .content_type(ContentType::TextPlain)
            .send(&mut rx_buf)
            .await
            .unwrap();
        let body = response.body().read_to_end().await;
        assert_eq!(body.unwrap(), b"PING");
    }

    tx.send(()).unwrap();
    t.await.unwrap();
}

#[tokio::test]
async fn test_resource_notls_bufread() {
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
    let mut client = HttpClient::new(&TCP, &LOOPBACK_DNS);
    let mut rx_buf = [0; 4096];
    let mut resource = client.resource(&url).await.unwrap();
    for _ in 0..2 {
        let response = resource
            .post("/")
            .body(b"PING".as_slice())
            .content_type(ContentType::TextPlain)
            .send(&mut rx_buf)
            .await
            .unwrap();
        let mut body_reader = response.body().reader();

        let mut body = vec![];
        loop {
            let buf = body_reader.fill_buf().await.unwrap();
            if buf.is_empty() {
                break;
            }
            body.extend_from_slice(buf);
            let buf_len = buf.len();
            body_reader.consume(buf_len);
        }

        assert_eq!(body, b"PING");
    }

    tx.send(()).unwrap();
    t.await.unwrap();
}

#[tokio::test]
#[cfg(feature = "embedded-tls")]
async fn test_resource_rustls() {
    use reqwless::client::{TlsConfig, TlsVerify};

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

    let mut tls_read_buf: [u8; 16384] = [0; 16384];
    let mut tls_write_buf: [u8; 16384] = [0; 16384];
    let url = format!("https://localhost:{}", addr.port());
    let mut client = HttpClient::new_with_tls(
        &TCP,
        &LOOPBACK_DNS,
        TlsConfig::new(OsRng.next_u64(), &mut tls_read_buf, &mut tls_write_buf, TlsVerify::None),
    );
    let mut rx_buf = [0; 4096];
    let mut resource = client.resource(&url).await.unwrap();
    for _ in 0..2 {
        let response = resource
            .post("/")
            .body(b"PING".as_slice())
            .content_type(ContentType::TextPlain)
            .send(&mut rx_buf)
            .await
            .unwrap();
        let body = response.body().read_to_end().await;
        assert_eq!(body.unwrap(), b"PING");
    }

    tx.send(()).unwrap();
    t.await.unwrap();
}

#[ignore]
#[tokio::test]
#[cfg(feature = "embedded-tls")]
async fn test_resource_drogue_cloud_sandbox() {
    use reqwless::client::{TlsConfig, TlsVerify};

    setup();
    let mut tls_read_buf: [u8; 16384] = [0; 16384];
    let mut tls_write_buf: [u8; 16384] = [0; 16384];
    let mut client = HttpClient::new_with_tls(
        &TCP,
        &PUBLIC_DNS,
        TlsConfig::new(OsRng.next_u64(), &mut tls_read_buf, &mut tls_write_buf, TlsVerify::None),
    );
    let mut rx_buf = [0; 4096];

    // The server must support TLS1.3
    // Also, if requests on embedded platforms fail with Error::Dns, then try to
    // enable the "alloc" feature on embedded-tls to enable RSA ciphers.
    let mut resource = client.resource("https://http.sandbox.drogue.cloud/v1").await.unwrap();

    for _ in 0..2 {
        let response = resource
            .post("/testing")
            .content_type(ContentType::TextPlain)
            .body(b"PING".as_slice())
            .send(&mut rx_buf)
            .await
            .unwrap();
        assert_eq!(Status::Forbidden, response.status);
        assert_eq!(76, response.body().discard().await.unwrap());
    }
}

#[tokio::test]
async fn test_request_response_notls_buffered() {
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
    let mut client = HttpClient::new(&TCP, &LOOPBACK_DNS);
    let mut tx_buf = [0; 4096];
    let mut rx_buf = [0; 4096];
    let mut request = client
        .request(Method::POST, &url)
        .await
        .unwrap()
        .into_buffered(&mut tx_buf)
        .body(b"PING".as_slice())
        .content_type(ContentType::TextPlain);
    let response = request.send(&mut rx_buf).await.unwrap();
    let body = response.body().read_to_end().await;
    assert_eq!(body.unwrap(), b"PING");

    tx.send(()).unwrap();
    t.await.unwrap();
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
async fn test_request_response_notls_buffered_chunked() {
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
    let mut client = HttpClient::new(&TCP, &LOOPBACK_DNS);
    let mut tx_buf = [0; 4096];
    let mut rx_buf = [0; 4096];
    static CHUNKS: [&'static [u8]; 2] = [b"PART1", b"PART2"];
    let mut request = client
        .request(Method::POST, &url)
        .await
        .unwrap()
        .into_buffered(&mut tx_buf)
        .body(ChunkedBody(&CHUNKS))
        .content_type(ContentType::TextPlain);
    let response = request.send(&mut rx_buf).await.unwrap();
    let body = response.body().read_to_end().await;
    assert_eq!(body.unwrap(), b"PART1PART2");

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

async fn echo(req: hyper::Request<Body>) -> Result<hyper::Response<Body>, hyper::Error> {
    match (req.method(), req.uri().path()) {
        _ => Ok(hyper::Response::new(req.into_body())),
    }
}

#[test]
fn compile_tests() {
    #[allow(dead_code)]
    async fn rx_buffer_lifetime_is_propagated_to_output<'buf>(port: u16, rx_buf: &'buf mut [u8]) -> &'buf mut [u8] {
        let mut http = HttpClient::new(&TCP, &LOOPBACK_DNS);
        let url = format!("http://127.0.0.1:{}", port);
        let mut request = http.request(Method::GET, &url).await.unwrap();
        let response = request.send(rx_buf).await.unwrap();
        response.body().read_to_end().await.unwrap()
    }
}
