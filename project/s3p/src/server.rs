use miette::Result;
use crate::request::{Request, Response};
use std::future::Future;
pub trait Server {
    type Request;

    async fn serve(&self) -> Result<impl Future>;
    async fn process(&self, request: Self::Request) -> Result<ResponseChannel>;
}
