use http::{uri::PathAndQuery, Extensions, HeaderMap, HeaderValue, Method, Uri};
use http_cache_semantics::RequestLike;
use s3s::Body;

use super::s3::S3Extension;

/// A basic representation of a request, flattened version of [http::Request]
#[derive(Default)]
pub struct Request {
    pub method: Method,
    pub uri: Uri,
    pub headers: HeaderMap<HeaderValue>,
    pub body: Body,
    pub extensions: Extensions,
}

impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut fmt = f.debug_struct("Request");

        self.extensions
            .get::<S3Extension>()
            .map(|ext| fmt.field("s3", ext));

        fmt.field("method", &self.method)
            .field("uri", &self.uri)
            .field("headers", &self.headers)
            .field("body", &self.body)
            .field("extensions", &self.extensions.len())
            .finish()
    }
}

/// impl of [Requestlike] for http_cache_policy
impl RequestLike for Request {
    fn uri(&self) -> Uri {
        self.uri.clone()
    }

    /// trim '/' from the end of a uri to ensure matches even with different styles
    fn is_same_uri(&self, other: &Uri) -> bool {
        //self.uri.eq(other)

        if self.uri.scheme() != other.scheme() {
            return false;
        }

        if self.uri.authority() != other.authority() {
            return false;
        }

        if self.uri.path().trim_end_matches('/') != other.path().trim_end_matches('/') {
            return false;
        }

        if self.uri.query() != other.query() {
            return false;
        }

        true
    }

    fn method(&self) -> &Method {
        &self.method
    }

    fn headers(&self) -> &HeaderMap {
        &self.headers
    }
}
