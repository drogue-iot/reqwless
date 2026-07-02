#![cfg_attr(not(test), no_std)]
#![doc = include_str!("../README.md")]
#![allow(async_fn_in_trait)]
use core::{num::ParseIntError, str::Utf8Error};

use embedded_io_async::ReadExactError;

mod fmt;

mod body_writer;
pub mod client;
pub mod headers;
mod reader;
pub mod request;
pub mod response;

/// Errors that can be returned by this library.
#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Error {
    /// An error with DNS (it's always DNS)
    Dns,
    /// An error with the underlying network
    Network(embedded_io::ErrorKind),
    /// An error encoding or decoding data
    Codec,
    /// An error parsing the URL
    InvalidUrl(nourl::Error),
    /// Tls Error
    #[cfg(feature = "embedded-tls")]
    Tls(embedded_tls::TlsError),
    /// Tls Error
    #[cfg(feature = "mbedtls-rs")]
    Tls(mbedtls_rs::SessionError),
    /// The provided buffer is too small
    BufferTooSmall,
    /// The request is already sent
    AlreadySent,
    /// An invalid number of bytes were written to request body
    IncorrectBodyWritten,
    /// The underlying connection was closed while being used
    ConnectionAborted,
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::Debug::fmt(self, f)
    }
}

impl core::error::Error for Error {}

impl embedded_io::Error for Error {
    fn kind(&self) -> embedded_io::ErrorKind {
        match self {
            Error::Network(kind) => *kind,
            Error::ConnectionAborted => embedded_io::ErrorKind::ConnectionAborted,
            _ => embedded_io::ErrorKind::Other,
        }
    }
}

impl From<embedded_io::ErrorKind> for Error {
    fn from(e: embedded_io::ErrorKind) -> Error {
        Error::Network(e)
    }
}

impl<E: embedded_io::Error> From<ReadExactError<E>> for Error {
    fn from(value: ReadExactError<E>) -> Self {
        match value {
            ReadExactError::UnexpectedEof => Error::ConnectionAborted,
            ReadExactError::Other(e) => Error::Network(e.kind()),
        }
    }
}

#[cfg(feature = "embedded-tls")]
impl From<embedded_tls::TlsError> for Error {
    fn from(e: embedded_tls::TlsError) -> Error {
        Error::Tls(e)
    }
}

/// Re-export those members since they're used for [client::TlsConfig].
#[cfg(feature = "mbedtls-rs")]
pub use mbedtls_rs::{Certificate, Credentials, TlsReference, TlsVersion, X509};

#[cfg(feature = "mbedtls-rs")]
impl From<mbedtls_rs::SessionError> for Error {
    fn from(e: mbedtls_rs::SessionError) -> Error {
        Error::Tls(e)
    }
}

impl From<ParseIntError> for Error {
    fn from(_: ParseIntError) -> Error {
        Error::Codec
    }
}

impl From<Utf8Error> for Error {
    fn from(_: Utf8Error) -> Error {
        Error::Codec
    }
}

impl From<nourl::Error> for Error {
    fn from(e: nourl::Error) -> Self {
        Error::InvalidUrl(e)
    }
}

/// Trait for types that may optionally implement [`embedded_io_async::BufRead`]
pub trait TryBufRead: embedded_io_async::Read {
    async fn try_fill_buf(&mut self) -> Option<Result<&[u8], Self::Error>> {
        None
    }

    fn try_consume(&mut self, _amt: usize) {}
}

impl<C> TryBufRead for crate::client::HttpConnection<'_, C>
where
    C: embedded_io_async::Read + embedded_io_async::Write,
{
    async fn try_fill_buf(&mut self) -> Option<Result<&[u8], Self::Error>> {
        // embedded-tls has its own internal buffer, let's prefer that if we can
        #[cfg(feature = "embedded-tls")]
        if let Self::Tls(ref mut tls) = *self {
            use embedded_io_async::{BufRead, Error};
            return Some(tls.fill_buf().await.map_err(|e| e.kind()));
        }

        None
    }

    fn try_consume(&mut self, amt: usize) {
        #[cfg(feature = "embedded-tls")]
        if let Self::Tls(tls) = self {
            use embedded_io_async::BufRead;
            tls.consume(amt);
        }

        #[cfg(not(feature = "embedded-tls"))]
        {
            _ = amt;
        }
    }
}
