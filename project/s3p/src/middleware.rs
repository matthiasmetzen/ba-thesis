use crate::request::{Request, Response};
use miette::Result;

#[allow(unused)]
pub enum MiddlewareAction {
    Forward(Request),
    Reply(Response),
}

#[async_trait::async_trait]
pub trait Layer: Send + Sync {
    async fn process_request(&self, request: Request) -> Result<MiddlewareAction> {
        Ok(MiddlewareAction::Forward(request))
    }

    async fn process_response(&self, response: Response) -> Result<Response> {
        Ok(response)
    }
}

// based on https://github.com/tower-rs/tower/blob/master/tower-layer/src/stack.rs#L5
pub struct Stack<Inner: Layer, Outer: Layer> {
    inner: Inner,
    outer: Outer,
}

impl<I: Layer, O: Layer> Stack<I, O> {
    #[allow(unused)]
    pub fn new(inner: I, outer: O) -> Self {
        Self {
            inner,
            outer
        }
    }
}

#[async_trait::async_trait]
impl<I: Layer, O: Layer> Layer for Stack<I, O> {
    async fn process_request(&self, request: Request) -> Result<MiddlewareAction> {
        let r = self.outer.process_request(request).await?;
        match r {
            MiddlewareAction::Forward(req) => return self.inner.process_request(req).await,
            MiddlewareAction::Reply(_) => return Ok(r)
        }
    }

    async fn process_response(&self, response: Response) -> Result<Response> {
        let r = self.inner.process_response(response).await?;
        self.outer.process_response(r).await
    }
}

pub struct Identity;

impl Layer for Identity {}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn make_stack() {
        let _s = Stack::new(Identity, Identity);
    }
}