# HTTP client for embedded devices

[![CI](https://github.com/drogue-iot/reqwless/actions/workflows/ci.yaml/badge.svg)](https://github.com/drogue-iot/reqwless/actions/workflows/ci.yaml)
[![crates.io](https://img.shields.io/crates/v/reqwless.svg)](https://crates.io/crates/reqwless)
[![docs.rs](https://docs.rs/reqwless/badge.svg)](https://docs.rs/reqwless)
[![Matrix](https://img.shields.io/matrix/drogue-iot:matrix.org)](https://matrix.to/#/#drogue-iot:matrix.org)

The `reqwless` crate implements an HTTP client that can be used in `no_std` environment, with any transport that implements the 
traits from the `embedded-io` crate. No alloc or std lib required!

It offers two sets of APIs:

* A low-level `request` API which allows you to construct HTTP requests and write them to a `embedded-io` transport.
* A higher level `client` API which uses the `embedded-nal-async` (+ optional `embedded-tls` / `mbedtls`) crates to establish TCP + TLS connections.

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

The client is still lacking many features, but can perform basic HTTP GET/PUT/POST/DELETE requests with payloads. However, not all content types and status codes are implemented, and are added on a need basis.  For TLS, it uses either `embedded-tls` or `mbedtls` as the transport.

NOTE: TLS verification is not supported in no_std environments for `embedded-tls`.

In addition to common headers like `.content_type()` on requests, broader `.headers()` functions on both request and response objects access arbitrary header values.

If you are missing a feature or would like an improvement, please raise an issue or a PR.

## TLS 1.2*, 1.3 and Supported Cipher Suites
`reqwless` uses `embedded-tls` or `mbedtls` to establish secure TLS connections for `https://..` urls.

*TLS 1.2 is only supported with `mbedtls`

:warning: Note that both features cannot be used together and will cause a compilation error.

### mbedtls
`mbedtls` supports TLS 1.2 and 1.3. It uses the [`mbedtls-rs`](https://crates.io/crates/mbedtls-rs) crate, a Rust wrapper over mbedtls. On esp32 boards it can take advantage of hardware acceleration.

Enable the `mbedtls` feature on `reqwless` and add `mbedtls-rs` as a direct dependency, enabling the backend feature for your target. For esp32 boards that is your SoC, currently one of:

 - `esp32`
 - `esp32c2`
 - `esp32c3`
 - `esp32c6`
 - `esp32h2`
 - `esp32s2`
 - `esp32s3`

Cargo.toml:

```toml
reqwless = { version = "0.14.0", default-features = false, features = ["mbedtls", "log"] }
mbedtls-rs = { version = "0.1", features = ["esp32s3"] }
```

#### Example
```rust,ignore
/// ... [initialization code. See esp-wifi]
let state = TcpClientState::<1, 4096, 4096>::new();
let mut tcp_client = TcpClient::new(stack, &state);
let dns_socket = DnsSocket::new(&stack);

// `Tls` holds the single active mbedtls instance; `rng` must be a `CryptoRng + Send`.
let tls = mbedtls_rs::Tls::new(rng).unwrap();
let ca = reqwless::Certificate::new(reqwless::X509::PEM(CERT)).unwrap();
let config = TlsConfig::new(
    tls.reference(),
    reqwless::ClientSessionConfig {
        ca_chain: Some(ca),
        min_version: reqwless::TlsVersion::Tls1_3,
        ..reqwless::ClientSessionConfig::new()
    },
);
let mut client = HttpClient::new_with_tls(&tcp_client, &dns_socket, config);

let mut request = client
    .request(reqwless::request::Method::GET, "https://www.google.com")
    .await
    .unwrap()
    .content_type(reqwless::headers::ContentType::TextPlain)
    .headers(&[("Host", "google.com")])
    .send(&mut buffer)
    .await
    .unwrap();
```

### embedded-tls
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

`reqwless` can compile on stable Rust 1.77 and up.
