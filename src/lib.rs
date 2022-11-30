#![no_std]
#![feature(impl_trait_projections)]
#![feature(async_fn_in_trait)]
#![allow(incomplete_features)]
#![doc = include_str!("../README.md")]
use core::{num::ParseIntError, str::Utf8Error};

mod fmt;

pub mod client;
pub mod request;
mod url;

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
    InvalidUrl,
    /// Tls Error
    Tls,
}

impl From<embedded_io::ErrorKind> for Error {
    fn from(e: embedded_io::ErrorKind) -> Error {
        Error::Network(e)
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
