use futures::channel::oneshot::Sender;
use miette::{Report};

#[derive(Debug)]
pub struct Request {
    pub tx: Sender<Response>
}

impl Request {
    pub async fn reply(self, res: impl Into<Response>) -> Result<(), Response> {
        self.tx.send(res.into())
    }
}

#[derive(Debug)]
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