#[cfg(feature = "date-header-u8")]
#[cfg(feature = "date-header-chrono")]
compile_error!("Specify zero or one of features date-header-u8, date-header-chrono");
#[cfg(feature = "date-header-chrono")]
use chrono::NaiveDateTime;

/// HTTP content types

#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ContentType {
    TextHtml,
    TextPlain,
    ApplicationJson,
    ApplicationCbor,
    ApplicationOctetStream,
}

impl<'a> From<&'a [u8]> for ContentType {
    fn from(from: &'a [u8]) -> ContentType {
        match from {
            b"application/json" => ContentType::ApplicationJson,
            b"application/cbor" => ContentType::ApplicationCbor,
            b"text/html" => ContentType::TextHtml,
            b"text/plain" => ContentType::TextPlain,
            _ => ContentType::ApplicationOctetStream,
        }
    }
}

impl ContentType {
    pub fn as_str(&self) -> &str {
        match self {
            ContentType::TextHtml => "text/html",
            ContentType::TextPlain => "text/plain",
            ContentType::ApplicationJson => "application/json",
            ContentType::ApplicationCbor => "application/cbor",
            ContentType::ApplicationOctetStream => "application/octet-stream",
        }
    }
}

/// Transfer encoding
#[derive(Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum TransferEncoding {
    Chunked,
    Compress,
    Deflate,
    Gzip,
}

impl<'a> TryFrom<&'a [u8]> for TransferEncoding {
    type Error = ();

    fn try_from(value: &'a [u8]) -> Result<Self, Self::Error> {
        Ok(match value {
            b"chunked" => TransferEncoding::Chunked,
            b"compress" => TransferEncoding::Compress,
            b"deflate" => TransferEncoding::Deflate,
            b"gzip" => TransferEncoding::Gzip,
            _ => return Err(()),
        })
    }
}

impl TransferEncoding {
    pub fn as_str(&self) -> &str {
        match self {
            TransferEncoding::Deflate => "deflate",
            TransferEncoding::Chunked => "chunked",
            TransferEncoding::Compress => "compress",
            TransferEncoding::Gzip => "gzip",
        }
    }
}

/// Keep-alive header
#[derive(Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct KeepAlive {
    timeout: Option<u8>,
    max: Option<u8>,
}

impl<'a> TryFrom<&'a [u8]> for KeepAlive {
    type Error = core::str::Utf8Error;

    fn try_from(from: &'a [u8]) -> Result<Self, Self::Error> {
        let mut keep_alive = KeepAlive {
            timeout: None,
            max: None,
        };
        for part in core::str::from_utf8(from)?.split(',') {
            let mut splitted = part.split('=').map(|s| s.trim());
            if let (Some(key), Some(value)) = (splitted.next(), splitted.next()) {
                match key {
                    _ if key.eq_ignore_ascii_case("timeout") => keep_alive.timeout = value.parse().ok(),
                    _ if key.eq_ignore_ascii_case("max") => keep_alive.max = value.parse().ok(),
                    _ => (),
                }
            }
        }
        Ok(keep_alive)
    }
}

#[cfg(feature = "date-header-chrono")]
#[derive(Debug, Eq, PartialEq)]
pub struct NaiveDateTimeHeaderValue(pub NaiveDateTime);

#[cfg(all(feature = "defmt", feature = "date-header-chrono"))]
extern crate alloc;
#[cfg(all(feature = "defmt", feature = "date-header-chrono"))]
use alloc::string::ToString;
#[cfg(all(feature = "defmt", feature = "date-header-chrono"))]
impl defmt::Format for NaiveDateTimeHeaderValue {
    fn format(self: &Self, f: defmt::Formatter) {
        defmt::write!(
            f,
            "{:?}",
            self.0.format("%a, %d %b %Y %H:%M:%S GMT").to_string().as_str()
        );
    }
}

#[derive(Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[cfg(any(feature = "date-header-u8", feature = "date-header-chrono"))]
pub struct HeaderDate {
    #[cfg(feature = "date-header-u8")]
    pub date: Option<[u8; 29]>, // like "Mon, 03 Jul 2024 12:34:56 GMT"
    #[cfg(feature = "date-header-chrono")]
    pub date: Option<NaiveDateTimeHeaderValue>,
}

#[cfg(feature = "date-header-u8")]
impl<'a> TryFrom<&'a [u8]> for HeaderDate {
    type Error = ();

    fn try_from(from: &'a [u8]) -> Result<Self, Self::Error> {
        let mut buf: [u8; 29] = [b' '; 29];
        buf.copy_from_slice(&from[..29]);
        Ok(Self { date: Some(buf) })
    }
}

#[cfg(feature = "date-header-chrono")]
impl<'a> TryFrom<&'a [u8]> for HeaderDate {
    type Error = ();

    fn try_from(from: &'a [u8]) -> Result<Self, Self::Error> {
        use core::str;
        if let Ok(s) = str::from_utf8(&from[5..]) {
            if let Ok((dt, _rem)) = NaiveDateTime::parse_and_remainder(s, "%d %b %Y %H:%M:%S") {
                return Ok(Self {
                    date: Some(NaiveDateTimeHeaderValue(dt)),
                });
            }
        }
        Err(())
    }
}
