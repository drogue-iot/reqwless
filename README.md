# HTTP client for embedded devices

[![CI](https://github.com/drogue-iot/reqwless/actions/workflows/ci.yaml/badge.svg)](https://github.com/drogue-iot/reqwless/actions/workflows/ci.yaml)
[![crates.io](https://img.shields.io/crates/v/reqwless.svg)](https://crates.io/crates/reqwless)
[![docs.rs](https://docs.rs/reqwless/badge.svg)](https://docs.rs/reqwless)
[![Matrix](https://img.shields.io/matrix/drogue-iot:matrix.org)](https://matrix.to/#/#drogue-iot:matrix.org)

The `reqwless` crate implements an HTTP client that can be used in `no_std` environment, with any transport that implements the 
traits from the `embedded-io` crate. No alloc or std lib required!

It offers two sets of APIs:

* A low-level `request` API which allows you to contruct HTTP requests and write them to a `embedded-io` transport.
* A higher level `client` API which uses the `embedded-nal-async` (+ optional `embedded-tls`) crates to establish TCP + TLS connections.

## example

```rust,ignore
let url = format!("http://localhost", addr.port());
let mut client = HttpClient::new(TokioTcp, StaticDns); // Types implementing embedded-nal-async
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
```

The client is still lacking many features, but can perform basic HTTP GET/PUT/POST/DELETE requests with payloads. However, not all content types and status codes are implemented, and are added on a need basis.  For TLS, it uses `embedded-tls` as the transport.

If you are missing a feature or would like an improvement, please raise an issue or a PR.

# Minimum supported Rust version (MSRV)

`reqwless` requires a feature from `nightly` to compile `embedded-io` with async support:

* `async_fn_in_trait`
* `impl_trait_projections`

This feature is complete, but is not yet merged to `stable`.
