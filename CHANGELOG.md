# Changelog

## Unreleased

### Fixes

* Fix regression introduced in v0.9.0 when reading chunked body where the final newline is not read (#58)

## [v0.9.0](https://github.com/drogue-iot/reqwless/compare/v0.8.0...v0.9.0) (2023-10-30)

### Fixes

* bugfixes and enhancements
* bump version
([c4efcb5](https://github.com/drogue-iot/reqwless/commit/c4efcb5cb3c5b78f179f8d9eb65afbb8959bed97))
* Implement `BufRead` for `BodyReader` ([#45](https://github.com/drogue-iot/reqwless/pull/45))
* Buffer writes automatically if `embedded-tls` is set up, regardless of the URL scheme ([#43](https://github.com/drogue-iot/reqwless/pull/43))

## v0.8.0 (2023-10-05)

### Features

* **headers:** Add keep-alive header parsing in response
([fa25d98](https://github.com/drogue-iot/reqwless/commit/fa25d98e36f985df3ea1dd97fef88cf1343b89fe))
* use nourl crate
([238c811](https://github.com/drogue-iot/reqwless/commit/238c811ff55d02d4b42115ee558102f083c29247))
* enable TLS PSK support and explicit verification
([982a381](https://github.com/drogue-iot/reqwless/commit/982a381db0e7c57790f983e056324fdc9fd8602d))
* use async fn in traits
([ed6e718](https://github.com/drogue-iot/reqwless/commit/ed6e718e3e3dd4fdca70220a715ffd76901d283d))
* mutation of rx payload
([c97ac9c](https://github.com/drogue-iot/reqwless/commit/c97ac9c17d5158aec9061b726ff1329cc5bac325))
* tls support
([12b1dd7](https://github.com/drogue-iot/reqwless/commit/12b1dd748ded5ae77a30a5db4bd12d38f0690a01))
* embedded-nal-async http client
([7d82b43](https://github.com/drogue-iot/reqwless/commit/7d82b43448ae38099964dead35ed63da27158cc1))

### Fixes

* **keep-alive:** Fix error for keep-alive header
([ed29d57](https://github.com/drogue-iot/reqwless/commit/ed29d57371ae08d5da3bae5ff631ae6ecc474073))
* add split read and write bufs
([6df94c9](https://github.com/drogue-iot/reqwless/commit/6df94c990da9410b8a4d919336401de670953fa4))
* pass the &mut slice back
([3269515](https://github.com/drogue-iot/reqwless/commit/32695155f28bc39deb139a94f2a048b2fd8a2fb1))
* use version with defmt support
([84f4cb6](https://github.com/drogue-iot/reqwless/commit/84f4cb6b29cad956c29d65ef6b1879916b4d53d3))
