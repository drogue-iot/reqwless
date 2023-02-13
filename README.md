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

NOTE: TLS verification is not supported in no_std environments for `embedded-tls`.

If you are missing a feature or would like an improvement, please raise an issue or a PR.

## TLS 1.3 and Supported Cipher Suites
`reqwless` uses `embedded-tls` to establish secure TLS connections for `https://..` urls.
`embedded-tls` only supports TLS 1.3, so to establish a connection the server must have this ssl protocol enabled.

An addition to the tls version requirement, there is also a negotiation of supported algorithms during the establishing phase of the secure communication between the client and server.
By default, the set of supported algorithms in `embedded-tls` is limited to algorithms that can run entirely on the stack.
To test whether the server supports this limited set of algorithm, try and test the server using the following `openssl` command:

```bash
openssl s_client -tls1_3 -ciphersuites TLS_AES_128_GCM_SHA256 -sigalgs "ECDSA+SHA256:ECDSA+SHA384:ed25519" -connect hostname:443
```

If the server successfully replies to the client hello then the enabled tls version and algorithms on the server should be ok.
If the command fails, then try and run without the limited set of signature algorithms

```bash
openssl s_client -tls1_3 -ciphersuites TLS_AES_128_GCM_SHA256 -connect hostname:443
```

If this works, then there are two options. Either enable the signature algorithms on the server by changing the private key from RSA to ECDSA or ed25519, or enable RSA keys on the client by specifying the `alloc` feature.
This enables `alloc` on `embedded-tls` which in turn enables RSA signature algorithms.


# Minimum supported Rust version (MSRV)

`reqwless` requires a feature from `nightly` to compile `embedded-io` with async support:

* `async_fn_in_trait`
* `impl_trait_projections`

This feature is complete, but is not yet merged to `stable`.
