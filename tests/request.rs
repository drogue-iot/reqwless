use embedded_io_adapters::tokio_1::FromTokio;
use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Server};
use reqwless::client::HttpConnection;
use reqwless::request::{Method, RequestBuilder};
use reqwless::{headers::ContentType, request::Request, response::Response};
use std::str::from_utf8;
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

    let stream = TcpStream::connect(addr).await.unwrap();
    let mut stream = HttpConnection::Plain(FromTokio::new(stream));

    let request = Request::post("/")
        .body(b"PING".as_slice())
        .content_type(ContentType::TextPlain)
        .build();

    request.write(&mut stream).await.unwrap();
    let mut rx_buf = [0; 4096];
    let response = Response::read(&mut stream, Method::POST, &mut rx_buf).await.unwrap();
    let body = response.body().read_to_end().await;

    assert_eq!(body.unwrap(), b"PING");

    tx.send(()).unwrap();
    t.await.unwrap();
}

async fn echo(req: hyper::Request<Body>) -> Result<hyper::Response<Body>, hyper::Error> {
    match (req.method(), req.uri().path()) {
        _ => Ok(hyper::Response::new(req.into_body())),
    }
}

#[tokio::test]
async fn write_without_base_path() {
    let request = Request::get("/hello").build();

    let mut buf = Vec::new();
    request.write(&mut buf).await.unwrap();

    assert!(from_utf8(&buf).unwrap().starts_with("GET /hello HTTP/1.1"));
}

#[tokio::test]
async fn google_panic() {
    use std::net::SocketAddr;
    let google_ip = [142, 250, 74, 110];
    let addr = SocketAddr::from((google_ip, 80));

    let conn = tokio::net::TcpStream::connect(addr).await.unwrap();
    let mut conn = embedded_io_adapters::tokio_1::FromTokio::new(conn);

    let request = Request::get("/")
        .host("www.google.com")
        .content_type(ContentType::TextPlain)
        .build();
    request.write(&mut conn).await.unwrap();

    let mut rx_buf = [0; 1024];
    let resp = Response::read(&mut conn, Method::GET, &mut rx_buf).await.unwrap();
    let body = resp.body().read_to_end().await.unwrap();
    println!("{} -> {}", body.len(), core::str::from_utf8(&body).unwrap());
}
