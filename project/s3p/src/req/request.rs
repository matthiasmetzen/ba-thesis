use http::{Extensions, HeaderMap, HeaderValue, Method, Uri};
use s3s::Body;

use super::s3::S3Extension;

#[derive(Default)]
pub struct Request {
    pub extensions: Extensions,
}

impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut fmt = f.debug_struct("Request");

        self.extensions
            .get::<HttpExtension>()
            .map(|ext| fmt.field("http", ext));
        self.extensions
            .get::<S3Extension>()
            .map(|ext| fmt.field("s3", ext));

        fmt.field("extensions", &self.extensions.len());

        fmt.finish()
    }
}

#[derive(Debug)]
pub struct HttpExtension {
    pub method: Method,
    pub uri: Uri,
    pub headers: HeaderMap<HeaderValue>,
    pub body: Body, // FIXME: this could maybe be Option<Body> and more generic body type (eg. http::Body)
}
