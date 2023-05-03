use miette::Report;
use http::{Extensions, Method, HeaderMap, HeaderValue, Version, Uri};
use s3s::{Body, S3};
use s3s::auth::Credentials;
use s3s::path::S3Path;
use s3s::stream::{DynByteStream, ByteStream, RemainingLength};
use s3s::http::{OrderedQs, Multipart};

#[derive(Default)]
pub struct Request {
    pub extensions: Extensions,
}

impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut fmt = f.debug_struct("Request");

        self.extensions.get::<HttpExtension>().map(|ext| fmt.field("http", ext));
        self.extensions.get::<S3Extension>().map(|ext| fmt.field("s3", ext));

        fmt.field("extensions", &self.extensions.len());

        fmt.finish()
    }
}

#[derive(Default)]
pub struct S3Extension {
    pub s3_path: Option<S3Path>,
    pub qs: Option<OrderedQs>,

    pub multipart: Option<Multipart>,
    pub vec_stream: Option<DynByteStream>,

    pub credentials: Option<Credentials>,
    pub op: String, // TODO: actual op instead of name
}

impl std::fmt::Debug for S3Extension {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("S3Extension")
            .field("s3_path", &self.s3_path)
            .field("op", &self.op)
            .field("qs", &self.qs)
            .field("multipart", &self.multipart)
            .field("vec_stream", &self.vec_stream.as_ref().map(|s| s.remaining_length()))
            .field("credentials", &self.credentials)
            .finish()
    }
}

#[derive(Debug)]
pub struct HttpExtension {
    pub method: Method,
    pub uri: Uri,
    pub headers: HeaderMap<HeaderValue>,
    pub body: Body, // FIXME: this could maybe be Option<Body> and more generic body type (eg. http::Body)
}

impl Request {
}

#[derive(Debug, Clone, Default)]
pub struct Response;

impl From<Report> for Response {
    fn from(_value: Report) -> Self {
        todo!()
    }
}

impl From<Response> for hyper::Response<hyper::Body> {
    fn from(_value: Response) -> Self {
        // FIXME: temporary
        hyper::Response::default()
    }
}

impl From<hyper::Request<hyper::Body>> for Response {
    fn from(_value: hyper::Request<hyper::Body>) -> Self {
        todo!()
    }
}

impl From<s3s::http::S3Extensions> for S3Extension {
    fn from(value: s3s::http::S3Extensions) -> Self {
        let mut value = value;
        let stream = value.take_stream();
        
        Self {
            s3_path: value.s3_path,
            qs: value.qs,
            multipart: value.multipart,
            vec_stream: stream,
            credentials: value.credentials,
            op: "".into()
        }
    }
}

impl From<s3s::http::Request> for Request {
    fn from(value: s3s::http::Request) -> Self {
        let mut exts = value.extensions;
        let s3_ext = S3Extension::from(value.s3ext);
        exts.insert(s3_ext);

        let http_ext = HttpExtension {
            method: value.method,
            uri: value.uri,
            headers: value.headers,
            body: value.body,
        };

        exts.insert(http_ext);

        
        Request {
            extensions: exts
        }
    }
}