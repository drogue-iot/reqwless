use embedded_io::adapters::FromTokio;
use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Server};
use reqwless::request::Method;
use reqwless::{headers::ContentType, request::Request, response::Response};
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
    let mut stream = FromTokio::new(stream);

    let request = Request::post("/")
        .body(b"PING")
        .content_type(ContentType::TextPlain)
        .build();

    request.write(&mut stream).await.unwrap();
    let mut rx_buf = [0; 4096];
    let response = Response::read_headers(&mut stream, Method::POST, &mut rx_buf)
        .await
        .unwrap();
    let body = response.body(&mut stream).read_to_end().await;

    assert_eq!(body.unwrap(), b"PING");

    tx.send(()).unwrap();
    t.await.unwrap();
}

async fn echo(req: hyper::Request<Body>) -> Result<hyper::Response<Body>, hyper::Error> {
    match (req.method(), req.uri().path()) {
        _ => Ok(hyper::Response::new(req.into_body())),
    }
}
