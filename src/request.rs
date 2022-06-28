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

pub struct RequestBuilder<'a> {
    request: Request<'a>,
}

pub enum Auth<'a> {
    Basic {
        username: &'a str,
        password: &'a str,
    },
}

impl<'a> Request<'a> {
    pub fn get() -> RequestBuilder<'a> {
        RequestBuilder {
            request: Request {
                method: Method::GET,
                ..Default::default()
            },
        }
    }
    pub fn post() -> RequestBuilder<'a> {
        RequestBuilder {
            request: Request {
                method: Method::POST,
                ..Default::default()
            },
        }
    }
    pub fn put() -> RequestBuilder<'a> {
        RequestBuilder {
            request: Request {
                method: Method::PUT,
                ..Default::default()
            },
        }
    }

    pub fn delete() -> RequestBuilder<'a> {
        RequestBuilder {
            request: Request {
                method: Method::DELETE,
                ..Default::default()
            },
        }
    }

    pub fn payload(&self) -> Option<&[u8]> {
        self.payload
    }
}

impl<'a> RequestBuilder<'a> {
    pub fn headers(mut self, headers: &'a [(&'a str, &'a str)]) -> Self {
        self.request.extra_headers.replace(headers);
        self
    }

    pub fn path(mut self, path: &'a str) -> Self {
        self.request.path.replace(path);
        self
    }

    pub fn payload(mut self, payload: &'a [u8]) -> Self {
        self.request.payload.replace(payload);
        self
    }

    pub fn content_type(mut self, content_type: ContentType) -> Self {
        self.request.content_type.replace(content_type);
        self
    }

    pub fn basic_auth(mut self, username: &'a str, password: &'a str) -> Self {
        self.request
            .auth
            .replace(Auth::Basic { username, password });
        self
    }

    pub fn build(self) -> Request<'a> {
        self.request
    }
}

pub enum Method {
    GET,
    PUT,
    POST,
    DELETE,
}

impl Method {
    pub fn as_str(&self) -> &str {
        match self {
            Method::POST => "POST",
            Method::PUT => "PUT",
            Method::GET => "GET",
            Method::DELETE => "DELETE",
        }
    }
}

#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Response<'a> {
    pub status: Status,
    pub content_type: Option<ContentType>,
    pub payload: Option<&'a [u8]>,
}

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
