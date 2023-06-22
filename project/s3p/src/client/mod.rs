use crate::{
    config::{ClientConfig, ClientType},
    req::{Request, Response},
};
use futures::{Future, FutureExt};
use miette::{ErrReport, Result};
use tower::Service;

use self::s3::S3Client;

pub mod s3;

pub trait Client: Send + Sync {
    fn send(&self, request: Request) -> impl Future<Output = Result<Response>> + Send;
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

pub trait ToOwnedClient {
    fn to_owned_client(&self) -> Self;
}

impl<T> Client for T
where
    T: Service<Request, Response = Response, Error = ErrReport> + Send + Sync,
    T::Future: Future<Output = Result<T::Response, T::Error>> + Send,
    Self: ToOwnedClient,
{
    fn send(&self, request: Request) -> impl Future<Output = Result<Response>> {
        let mut client = self.to_owned_client();
        client.call(request)
    }
}

pub struct ClientUtil;

impl ClientUtil {}

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
    fn send(&self, request: Request) -> impl Future<Output = Result<Response>> + Send {
        match &self {
            Self::S3(c) => c.send(request).boxed(),
        }
    }
}
