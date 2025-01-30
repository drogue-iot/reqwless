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
