/// A read only HTTP request type
pub struct Request<'a> {
    pub(crate) method: Method,
    pub(crate) path: Option<&'a str>,
    pub(crate) auth: Option<Auth<'a>>,
    pub(crate) payload: Option<&'a [u8]>,
    pub(crate) content_type: Option<ContentType>,
    pub(crate) extra_headers: Option<&'a [(&'a str, &'a str)]>,
}

impl<'a> Default for Request<'a> {
    fn default() -> Self {
        Self {
            method: Method::GET,
            path: None,
            auth: None,
            payload: None,
            content_type: None,
            extra_headers: None,
        }
    }
}

/// A HTTP request builder.
pub struct RequestBuilder<'a> {
    request: Request<'a>,
}

/// Request authentication scheme.
pub enum Auth<'a> {
    Basic { username: &'a str, password: &'a str },
}

impl<'a> Request<'a> {
    /// Create a new GET http request.
    pub fn get() -> RequestBuilder<'a> {
        RequestBuilder {
            request: Request {
                method: Method::GET,
                ..Default::default()
            },
        }
    }

    /// Create a new POST http request.
    pub fn post() -> RequestBuilder<'a> {
        RequestBuilder {
            request: Request {
                method: Method::POST,
                ..Default::default()
            },
        }
    }

    /// Create a new PUT http request.
    pub fn put() -> RequestBuilder<'a> {
        RequestBuilder {
            request: Request {
                method: Method::PUT,
                ..Default::default()
            },
        }
    }

    /// Create a new DELETE http request.
    pub fn delete() -> RequestBuilder<'a> {
        RequestBuilder {
            request: Request {
                method: Method::DELETE,
                ..Default::default()
            },
        }
    }
}

impl<'a> RequestBuilder<'a> {
    /// Set optional headers on the request.
    pub fn headers(mut self, headers: &'a [(&'a str, &'a str)]) -> Self {
        self.request.extra_headers.replace(headers);
        self
    }

    /// Set the path of the HTTP request.
    pub fn path(mut self, path: &'a str) -> Self {
        self.request.path.replace(path);
        self
    }

    /// Set the payload to send in the HTTP request body.
    pub fn payload(mut self, payload: &'a [u8]) -> Self {
        self.request.payload.replace(payload);
        self
    }

    /// Set the content type header for the request.
    pub fn content_type(mut self, content_type: ContentType) -> Self {
        self.request.content_type.replace(content_type);
        self
    }

    /// Set the basic authentication header for the request.
    pub fn basic_auth(mut self, username: &'a str, password: &'a str) -> Self {
        self.request.auth.replace(Auth::Basic { username, password });
        self
    }

    /// Return an immutable request.
    pub fn build(self) -> Request<'a> {
        self.request
    }
}

/// HTTP request methods
pub enum Method {
    /// GET
    GET,
    /// PUT
    PUT,
    /// POST
    POST,
    /// DELETE
    DELETE,
}

impl Method {
    /// str representation of method
    pub fn as_str(&self) -> &str {
        match self {
            Method::POST => "POST",
            Method::PUT => "PUT",
            Method::GET => "GET",
            Method::DELETE => "DELETE",
        }
    }
}

/// Type representing a parsed HTTP response.
#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Response<'a> {
    /// The HTTP response status code.
    pub status: Status,
    /// The HTTP response content type.
    pub content_type: Option<ContentType>,
    /// The HTTP response body.
    pub payload: Option<&'a [u8]>,
}

/// HTTP status types
#[derive(Debug, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Status {
    Ok = 200,
    Created = 201,
    Accepted = 202,
    BadRequest = 400,
    Unauthorized = 401,
    Forbidden = 403,
    NotFound = 404,
    Unknown = 0,
}

impl From<u32> for Status {
    fn from(from: u32) -> Status {
        match from {
            200 => Status::Ok,
            201 => Status::Created,
            202 => Status::Accepted,
            400 => Status::BadRequest,
            401 => Status::Unauthorized,
            403 => Status::Forbidden,
            404 => Status::NotFound,
            n => {
                warn!("Unknown status code: {:?}", n);
                Status::Unknown
            }
        }
    }
}

/// HTTP content types
#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ContentType {
    ApplicationJson,
    ApplicationCbor,
    ApplicationOctetStream,
}

impl<'a> From<&'a str> for ContentType {
    fn from(from: &'a str) -> ContentType {
        match from {
            "application/json" => ContentType::ApplicationJson,
            "application/cbor" => ContentType::ApplicationCbor,
            _ => ContentType::ApplicationOctetStream,
        }
    }
}

impl ContentType {
    pub fn as_str(&self) -> &str {
        match self {
            ContentType::ApplicationJson => "application/json",
            ContentType::ApplicationCbor => "application/cbor",
            ContentType::ApplicationOctetStream => "application/octet-stream",
        }
    }
}
