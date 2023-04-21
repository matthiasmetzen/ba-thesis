use crate::request::{Response, Request};
use miette::Result;

pub trait Client {
    async fn send(&self, request: Request) -> Result<Response>;
}