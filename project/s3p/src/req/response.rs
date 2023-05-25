use http::{HeaderMap, HeaderValue, StatusCode};
use miette::Report;
use s3s::Body;

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
