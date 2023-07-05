use http::{header, Extensions, HeaderMap, HeaderValue, StatusCode};
use http_cache_semantics::ResponseLike;
use hyper::body::Bytes;
use miette::Report;
use s3s::{
    stream::{ByteStream, RemainingLength},
    Body,
};

#[derive(Debug, Default)]
pub struct Response {
    pub status: StatusCode,
    pub headers: HeaderMap<HeaderValue>,
    pub body: Body,
    pub extensions: Extensions,
}

impl Response {
    pub fn with_status(status: StatusCode) -> Self {
        Self {
            status,
            ..Default::default()
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct RawResponse {
    pub status: StatusCode,
    pub headers: HeaderMap<HeaderValue>,
    pub bytes: Option<Bytes>,
}

impl From<Report> for Response {
    fn from(_value: Report) -> Self {
        todo!()
    }
}

impl From<Response> for hyper::Response<hyper::Body> {
    fn from(value: Response) -> Self {
        // FIXME: temporary
        let mut res_builder = hyper::Response::builder().status(value.status);

        let headers = res_builder.headers_mut().unwrap();
        headers.extend(value.headers);

        /*let content_len = value.body.remaining_length();
        if let Some(len) = content_len.exact() {
            if len > 0 {
                headers.insert(header::CONTENT_LENGTH, len.into());
            } else {
                headers.remove(header::CONTENT_LENGTH);
            }
        }*/

        res_builder.body(value.body.into()).unwrap()
    }
}

impl From<s3s::http::Response> for Response {
    fn from(value: s3s::http::Response) -> Self {
        Response {
            status: value.status,
            headers: value.headers,
            body: value.body,
            extensions: value.extensions,
        }
    }
}

impl From<hyper::Request<hyper::Body>> for Response {
    fn from(_value: hyper::Request<hyper::Body>) -> Self {
        todo!()
    }
}

impl From<Response> for RawResponse {
    fn from(value: Response) -> Self {
        Self {
            status: value.status,
            headers: value.headers,
            bytes: value.body.bytes(),
        }
    }
}

impl From<RawResponse> for Response {
    fn from(value: RawResponse) -> Self {
        Self {
            status: value.status,
            headers: value.headers,
            body: match value.bytes {
                Some(b) => Body::from(b),
                None => Body::default(),
            },
            extensions: Extensions::new(),
        }
    }
}

impl ResponseLike for Response {
    fn status(&self) -> StatusCode {
        self.status
    }

    fn headers(&self) -> &HeaderMap {
        &self.headers
    }
}
