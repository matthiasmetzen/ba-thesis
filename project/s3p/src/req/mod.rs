pub mod request;
pub mod response;
pub mod s3;

pub use request::Request;
pub use response::Response;

pub use s3::S3Extension;

/// The default error for all things related to request & response errors
/// The [crate::Server] uses this to reply with propper error messages
#[derive(thiserror::Error, Debug)]
pub enum SendError {
    #[error("Internal error")]
    Internal(miette::Report),
    #[error("Error constructing request")]
    RequestErr(Response, miette::Report),
    #[error("Error response")]
    ResponseErr(Response, miette::Report),
}

impl Into<hyper::Response<hyper::Body>> for SendError {
    fn into(self) -> hyper::Response<hyper::Body> {
        match self {
            SendError::RequestErr(resp, rep) | SendError::ResponseErr(resp, rep) => resp.into(),
            _ => {
                let body = hyper::Body::from(self.to_string());
                hyper::Response::builder().status(500).body(body).unwrap()
            }
        }
    }
}
