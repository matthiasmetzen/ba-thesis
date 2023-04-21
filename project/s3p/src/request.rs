use miette::{Report, WrapErr, miette};

#[derive(Debug, Clone)]
pub struct Request {
}

impl Request {
}

#[derive(Debug, Clone)]
pub struct Response;

impl From<Report> for Response {
    fn from(value: Report) -> Self {
        todo!()
    }
}

impl From<Response> for hyper::Response<hyper::Body> {
    fn from(value: Response) -> Self {
        todo!()
    }
}

impl From<hyper::Request<hyper::Body>> for Response {
    fn from(value: hyper::Request<hyper::Body>) -> Self {
        todo!()
    }
}

// FIXME: Temporary
impl From<s3s::http::Request> for Request {
    fn from(value: s3s::http::Request) -> Self {
        todo!()
    }
}