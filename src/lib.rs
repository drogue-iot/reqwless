#![no_std]
use embedded_io::asynch::{Read, Write};

mod fmt;

pub mod client;
pub mod request;

pub trait Network: Read + Write {}
impl<N: Read + Write> Network for N {}
