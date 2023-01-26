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
