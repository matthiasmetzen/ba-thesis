use crate::request::{Response, Request};
use miette::Result;

#[async_trait::async_trait]
pub trait Client {
    async fn send(&self, request: Request) -> Result<Response>;
}