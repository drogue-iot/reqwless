/// HTTP content types
#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ContentType {
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
            b"text/plain" => ContentType::TextPlain,
            _ => ContentType::ApplicationOctetStream,
        }
    }
}

impl ContentType {
    pub fn as_str(&self) -> &str {
        match self {
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
