use crate::request::{Response, Request};
use futures::Future;
use miette::Result;

pub trait Client: Send + Sync {
    fn send(&self, request: Request) -> impl Future<Output = Result<Response>> + Send;
}