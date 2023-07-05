use crate::{
    config::ClientType,
    req::{Request, Response, SendError},
};
use futures::{Future, FutureExt};
use miette::{ErrReport, Result};
use thiserror::Error;
use tower::Service;

use self::s3::S3Client;

pub mod s3;

pub trait Client: Send + Sync {
    fn send(&self, request: Request) -> impl Future<Output = Result<Response, SendError>> + Send;
}

/* pub trait Client:
    Service<
    Request,
    Response = Response,
    Error = Error,
    Future = Pin<Box<dyn Future<Output = Result<Response, Error>> + Send>>,
>
{
} */

pub enum ClientDelegate {
    S3(S3Client),
}

impl From<&ClientType> for ClientDelegate {
    fn from(config: &ClientType) -> Self {
        match config {
            ClientType::S3(c) => ClientDelegate::S3(S3Client::from(c)),
        }
    }
}

impl Client for ClientDelegate {
    fn send(&self, request: Request) -> impl Future<Output = Result<Response, SendError>> + Send {
        match &self {
            Self::S3(c) => c.send(request).boxed(),
        }
    }
}
