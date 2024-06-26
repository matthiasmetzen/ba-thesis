use super::Body;
use super::Multipart;
use super::OrderedQs;

use crate::auth::Credentials;
use crate::path::S3Path;
use crate::stream::DynByteStream;
use crate::stream::VecByteStream;

use hyper::http::Extensions;
use hyper::http::HeaderValue;
use hyper::HeaderMap;
use hyper::Method;
use hyper::Uri;

pub struct Request {
    pub method: Method,
    pub uri: Uri,
    pub headers: HeaderMap<HeaderValue>,
    pub extensions: Extensions,
    pub body: Body,
    pub s3ext: S3Extensions,
}

#[derive(Default)]
pub struct S3Extensions {
    pub s3_path: Option<S3Path>,
    pub qs: Option<OrderedQs>,

    pub multipart: Option<Multipart>,
    pub vec_stream: Option<VecByteStream>,

    pub credentials: Option<Credentials>,
}

impl S3Extensions {
    pub fn take_stream(&mut self) -> Option<DynByteStream> {
        self.vec_stream.take().map(|stream| Box::pin(stream) as DynByteStream)
    }
}

impl From<hyper::Request<Body>> for Request {
    fn from(req: hyper::Request<Body>) -> Self {
        let (parts, body) = req.into_parts();
        Self {
            method: parts.method,
            uri: parts.uri,
            headers: parts.headers,
            extensions: parts.extensions,
            body,
            s3ext: S3Extensions::default(),
        }
    }
}
