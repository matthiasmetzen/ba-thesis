use miette::Report;

#[derive(Debug, Clone)]
pub struct Request {
}

impl Request {
}

#[derive(Debug, Clone)]
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

// FIXME: Temporary
impl From<s3s::http::Request> for Request {
    fn from(_value: s3s::http::Request) -> Self {
        todo!()
    }
}