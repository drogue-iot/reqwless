#![no_std]
#![doc = include_str!("../README.md")]
use core::{num::ParseIntError, str::Utf8Error};

mod fmt;

pub mod client;
pub mod request;

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
