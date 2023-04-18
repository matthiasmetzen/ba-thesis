use miette::{Report};


#[derive(Debug)]
pub struct Request;

impl Request {
    pub async fn reply(self, res: impl Into<Response>) -> Result<(), Response> {
        todo!()
    }
}

#[derive(Debug)]
pub struct Response;

impl From<Report> for Response {
    fn from(value: Report) -> Self {
        todo!()
    }
}
