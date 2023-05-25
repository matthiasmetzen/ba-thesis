use miette::Result;

use crate::req::{Request, Response};

use std::future::Future;
use std::ops::Deref;
use std::sync::Arc;

pub mod s3;
pub use s3::S3ServerBuilder;

pub trait ServerBuilder {
    fn serve(&self, handler: impl Handler) -> Result<impl Server>;
}

pub trait Server: Send {
    async fn stop(self) -> Result<()>;
}

pub trait Handler<Req = Request, Resp = Response>: Send + Sync {
    type Future: Future<Output = Result<Resp>> + Send;

    fn handle(&self, msg: Req) -> Self::Future;
}

// Accept async closures as handlers
impl<Fun, Fut, Req, Resp> Handler<Req, Resp> for Fun
where
    Req: Send + Sync + 'static,
    Fun: Fn(Req) -> Fut + Send + Sync,
    Fut: Future<Output = Result<Resp>> + Send,
{
    type Future = impl Future<Output = Result<Resp>> + Send;

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
