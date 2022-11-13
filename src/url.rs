use crate::Error;

/// A parsed URL to extract different parts of the URL.
pub struct Url<'a> {
    host: &'a str,
    scheme: UrlScheme,
    port: u16,
    path: &'a str,
}

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum UrlScheme {
    /// HTTP scheme
    HTTP,
    /// HTTPS (HTTP + TLS) scheme
    HTTPS,
}

impl<'a> Url<'a> {
    /// Parse the provided url
    pub fn parse(url: &'a str) -> Result<Url<'a>, Error> {
        let mut parts = url.split("://");
        let scheme = if let Some(s) = parts.next() {
            if s.eq_ignore_ascii_case("http") {
                UrlScheme::HTTP
            } else if s.eq_ignore_ascii_case("https") {
                UrlScheme::HTTPS
            } else {
                return Err(Error::InvalidUrl);
            }
        } else {
            return Err(Error::InvalidUrl);
        };

        let default_port = match scheme {
            UrlScheme::HTTP => 80,
            UrlScheme::HTTPS => 443,
        };

        let (host, port, path) = if let Some(s) = parts.next() {
            // Port is defined
            if let Some(port_delim) = s.find(":") {
                let host = &s[..port_delim];
                let rest = &s[port_delim..];

                let (port, path) = if let Some(path_delim) = rest.find("/") {
                    let port: u16 = rest[1..path_delim].parse::<u16>().unwrap_or(default_port);
                    let path = &rest[path_delim..];
                    let path = if path.is_empty() { "/" } else { path };
                    (port, path)
                } else {
                    let port: u16 = rest[1..].parse::<u16>().unwrap_or(default_port);
                    (port, "/")
                };
                (host, port, path)
            } else {
                let (host, path) = if let Some(needle) = s.find("/") {
                    let host = &s[..needle];
                    let path = &s[needle..];
                    (host, if path.is_empty() { "/" } else { path })
                } else {
                    (s, "/")
                };
                (host, default_port, path)
            }
        } else {
            return Err(Error::InvalidUrl);
        };

        Ok(Self {
            scheme,
            host,
            path,
            port,
        })
    }

    pub fn scheme(&self) -> UrlScheme {
        self.scheme
    }

    pub fn host(&self) -> &'a str {
        self.host
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn path(&self) -> &'a str {
        self.path
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;

    #[test]
    fn test_parse_minimal() {
        let url = Url::parse("http://localhost").unwrap();
        assert_eq!(url.host(), "localhost");
        assert_eq!(url.port(), 80);
        assert_eq!(url.scheme(), UrlScheme::HTTP);
        assert_eq!(url.path(), "/");
    }

    #[test]
    fn test_parse_path() {
        let url = Url::parse("http://localhost/foo/bar").unwrap();
        assert_eq!(url.host(), "localhost");
        assert_eq!(url.port(), 80);
        assert_eq!(url.scheme(), UrlScheme::HTTP);
        assert_eq!(url.path(), "/foo/bar");
    }

    #[test]
    fn test_parse_port() {
        let url = Url::parse("http://localhost:8088").unwrap();
        assert_eq!(url.host(), "localhost");
        assert_eq!(url.port(), 8088);
        assert_eq!(url.scheme(), UrlScheme::HTTP);
        assert_eq!(url.path(), "/");
    }

    #[test]
    fn test_parse_port_path() {
        let url = Url::parse("http://localhost:8088/foo/bar").unwrap();
        assert_eq!(url.host(), "localhost");
        assert_eq!(url.port(), 8088);
        assert_eq!(url.scheme(), UrlScheme::HTTP);
        assert_eq!(url.path(), "/foo/bar");
    }

    #[test]
    fn test_parse_scheme() {
        let url = Url::parse("https://localhost/").unwrap();
        assert_eq!(url.host(), "localhost");
        assert_eq!(url.port(), 443);
        assert_eq!(url.scheme(), UrlScheme::HTTPS);
        assert_eq!(url.path(), "/");
    }
}
