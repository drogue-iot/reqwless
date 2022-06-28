#![no_std]
#![doc = include_str!("../README.md")]
use embedded_io::asynch::{Read, Write};

mod fmt;

pub mod client;
pub mod request;

/// A Convenience trait for an underlying transport implemented on embedded-io.
pub trait Network: Read + Write {}
impl<N: Read + Write> Network for N {}
