use embedded_io::adapters::FromTokio;
use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Response, Server};
use reqwless::client::HttpClient;
use reqwless::request::{ContentType, Request};
use tokio::net::TcpStream;
use tokio::sync::oneshot;

#[tokio::test]
async fn test_client() {
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
    let mut client = HttpClient::new(&mut stream, "localhost");

    let mut rx_buf = [0; 4096];
    let request = Request::post()
        .payload(b"PING")
        .content_type(ContentType::TextPlain)
        .build();

    let response = client.request(request, &mut rx_buf).await.unwrap();

    assert_eq!(response.payload.unwrap(), b"PING");

    tx.send(()).unwrap();
    t.await.unwrap();
}

async fn echo(req: hyper::Request<Body>) -> Result<Response<Body>, hyper::Error> {
    match (req.method(), req.uri().path()) {
        _ => Ok(Response::new(req.into_body())),
    }
}
