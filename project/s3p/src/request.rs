use http::{Extensions, HeaderMap, HeaderValue, Method, StatusCode, Uri};
use miette::{miette, Report};
use s3s::auth::Credentials;
use s3s::http::{Multipart, OrderedQs};
use s3s::path::S3Path;
use s3s::stream::ByteStream;
use s3s::stream::VecByteStream;
use s3s::Body;

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

pub(crate) struct Operation(pub &'static dyn s3s::ops::Operation);

impl std::fmt::Debug for Operation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct(self.0.name()).finish()
    }
}

impl From<&'static dyn s3s::ops::Operation> for Operation {
    fn from(value: &'static dyn s3s::ops::Operation) -> Self {
        Self(value)
    }
}

#[derive(Default)]
pub struct S3Extension {
    pub s3_path: Option<S3Path>,
    pub qs: Option<OrderedQs>,

    pub multipart: Option<Multipart>,
    pub vec_stream: Option<VecByteStream>,

    pub credentials: Option<Credentials>,
    pub(crate) op: Option<Operation>, // TODO: actual op instead of name
}

impl std::fmt::Debug for S3Extension {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("S3Extension")
            .field("s3_path", &self.s3_path)
            .field("op", &self.op)
            .field("qs", &self.qs)
            .field("multipart", &self.multipart)
            .field(
                "vec_stream",
                &self.vec_stream.as_ref().map(|s| s.remaining_length()),
            )
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

impl Request {}

#[derive(Debug, Default)]
pub struct Response {
    pub status: StatusCode,
    pub headers: HeaderMap<HeaderValue>,
    pub body: Body,
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

        res_builder.headers_mut().unwrap().extend(value.headers);

        res_builder.body(value.body.into()).unwrap()
    }
}

impl From<s3s::http::Response> for Response {
    fn from(value: s3s::http::Response) -> Self {
        Response {
            status: value.status,
            headers: value.headers,
            body: value.body,
        }
    }
}

impl From<hyper::Request<hyper::Body>> for Response {
    fn from(_value: hyper::Request<hyper::Body>) -> Self {
        todo!()
    }
}

impl From<s3s::http::S3Extensions> for S3Extension {
    fn from(value: s3s::http::S3Extensions) -> Self {
        Self {
            s3_path: value.s3_path,
            qs: value.qs,
            multipart: value.multipart,
            vec_stream: value.vec_stream,
            credentials: value.credentials,
            op: None,
        }
    }
}

impl From<S3Extension> for s3s::http::S3Extensions {
    fn from(s3_ext: S3Extension) -> Self {
        Self {
            s3_path: s3_ext.s3_path,
            qs: s3_ext.qs,
            multipart: s3_ext.multipart,
            vec_stream: s3_ext.vec_stream,
            credentials: s3_ext.credentials,
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

        Request { extensions: exts }
    }
}

impl TryFrom<Request> for s3s::http::Request {
    type Error = miette::Report;

    fn try_from(req: Request) -> Result<Self, Self::Error> {
        let mut req = req;

        let s3_ext = req
            .extensions
            .remove::<S3Extension>()
            .ok_or_else(|| miette!("Could not get S3 Extension from Request"))?;

        let http_ext = req
            .extensions
            .remove::<HttpExtension>()
            .ok_or_else(|| miette!("Could not get S3 Extension from Request"))?;

        Ok(Self {
            method: http_ext.method,
            uri: http_ext.uri,
            headers: http_ext.headers,
            extensions: req.extensions,
            body: http_ext.body,
            s3ext: s3_ext.into(),
        })
    }
}
