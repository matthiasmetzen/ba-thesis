use miette::Report;

use crate::config::ServerType;
use crate::req::{Request, Response, SendError};
use crate::webhook::BroadcastSend;

use std::future::Future;
use std::ops::Deref;
use std::sync::Arc;

pub mod s3;
pub use s3::{S3Server, S3ServerBuilder};

/// Servers get started using the serve method on the builder
pub trait ServerBuilder {
    fn broadcast(&mut self, tx: &BroadcastSend) -> &mut Self;
    fn serve(&self, handler: impl Handler) -> Result<impl Server, Report>;
}

/// Represents a Server component
pub trait Server: Send {
    async fn stop(self) -> Result<(), Report>;
}

/// A Handler gets passed to the server upon creation. The [Server] forwards requests to it.
/// Its implemented for [Fn(Request) -> Future + Send + Sync]
pub trait Handler<Req = Request, Resp = Response>: Send + Sync {
    type Future: Future<Output = Result<Resp, SendError>> + Send;

    fn handle(&self, msg: Req) -> Self::Future;
}

// Accept async closures as handlers
impl<Fun, Fut, Req, Resp> Handler<Req, Resp> for Fun
where
    Req: Send + Sync + 'static,
    Fun: Fn(Req) -> Fut + Send + Sync,
    Fut: Future<Output = Result<Resp, SendError>> + Send,
{
    type Future = impl Future<Output = Result<Resp, SendError>> + Send;

    fn handle(&self, msg: Req) -> Self::Future {
        self(msg)
    }
}

impl<Req, Resp, H: Handler<Req, Resp>> Handler<Req, Resp> for Arc<H>
where
    Req: Send + Sync + 'static,
    Resp: Send + Sync,
{
    type Future = H::Future;

    fn handle(&self, msg: Req) -> Self::Future {
        self.deref().handle(msg)
    }
}

/// Enum to select the server type during creation from config
pub enum ServerDelegate {
    S3(S3ServerBuilder),
}

impl From<&ServerType> for ServerDelegate {
    fn from(config: &ServerType) -> Self {
        match config {
            ServerType::S3(c) => Self::S3(S3ServerBuilder::from(c)),
        }
    }
}

impl ServerBuilder for ServerDelegate {
    fn broadcast(&mut self, tx: &BroadcastSend) -> &mut Self {
        match self {
            Self::S3(s) => s.broadcast(tx),
        };

        self
    }

    fn serve(&self, handler: impl Handler + 'static) -> Result<impl Server, Report> {
        match self {
            Self::S3(s) => s.serve(handler),
        }
    }
}
