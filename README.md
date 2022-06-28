# HTTP client for embedded devices

[![CI](https://github.com/drogue-iot/reqwless/actions/workflows/ci.yaml/badge.svg)](https://github.com/drogue-iot/reqwless/actions/workflows/ci.yaml)
[![crates.io](https://img.shields.io/crates/v/reqwless.svg)](https://crates.io/crates/reqwless)
[![docs.rs](https://docs.rs/reqwless/badge.svg)](https://docs.rs/reqwless)
[![Matrix](https://img.shields.io/matrix/drogue-iot:matrix.org)](https://matrix.to/#/#drogue-iot:matrix.org)

The `reqwless` crate implements an HTTP client that can be used in `no_std` environment, with any transport that implements the 
traits from the `embedded-io` create.

The client is still lacking many features, but can perform basic HTTP GET/PUT/POST/DELETE requests with payloads. However, not all content types and status codes are implemented, and are added on a need basis. For TLS, you can use `embedded-tls` as the transport.

If you are missing a feature or would like an improvement, please raise an issue or a PR.

# Minimum supported Rust version (MSRV)

`reqwless` requires two features from `nightly` to compile `embedded-io` with async support:

* `generic_associated_types`
* `type_alias_impl_trait`

These features are complete, but are not yet merged to `stable`.
