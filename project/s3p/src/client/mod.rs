use crate::{
    config::ClientType,
    req::{Request, Response, SendError},
};
use futures::{Future, FutureExt};
use miette::{ErrReport, Result};
use thiserror::Error;
use tower::Service;

pub mod s3;
pub use self::s3::S3Client;

/// This trait defines a component used to resolve [Request]s asynchronously
pub trait Client: Send + Sync {
    /// Asynchrounously resolves a [Request] into a [Response]
    fn send(&self, request: Request) -> impl Future<Output = Result<Response, SendError>> + Send;
}

/// Provides enum dispatch over all included [Client]s
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
