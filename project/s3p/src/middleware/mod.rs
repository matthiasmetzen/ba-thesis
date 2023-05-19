use crate::request::{Request, Response};
use miette::Result;

pub mod cache;
pub use self::cache::CacheLayer;

use crate::{client::Client, server::Handler};

use std::{future::Future, sync::Arc};

#[async_trait::async_trait]
pub trait Layer: Send + Sync {
    async fn call(&self, req: Request, next: impl NextLayer) -> Result<Response>;
}

// based on https://github.com/tower-rs/tower/blob/master/tower-layer/src/stack.rs#L5
pub struct Chain<Current: Layer, Next: Layer> {
    current: Current,
    next: Next,
}

impl<C: Layer, N: Layer> Chain<C, N> {
    #[allow(unused)]
    pub fn new(current: C, next: N) -> Self {
        Self { current, next }
    }

    #[allow(unused)]
    pub fn then<L: Layer>(self, next: L) -> Chain<Self, L> {
        Chain::new(self, next)
    }
}

#[async_trait::async_trait]
pub trait NextLayer: Send {
    async fn call(self, req: Request) -> Result<Response>;
}

#[async_trait::async_trait]
impl<Fun, Fut> NextLayer for Fun
where
    Fun: FnOnce(Request) -> Fut + Send,
    Fut: Future<Output = Result<Response>> + Send,
{
    async fn call(self, req: Request) -> Result<Response> {
        self(req).await
    }
}

#[async_trait::async_trait]
impl<C: Layer, N: Layer> Layer for Chain<C, N> {
    async fn call(&self, req: Request, next: impl NextLayer) -> Result<Response> {
        let then = |req| self.next.call(req, next);
        self.current.call(req, then).await
    }
}

pub struct Identity;

#[async_trait::async_trait]
impl Layer for Identity {
    async fn call(&self, req: Request, next: impl NextLayer) -> Result<Response> {
        next.call(req).await
    }
}

pub struct RequestProcessor<C: Client, L: Layer = Identity> {
    layer: L,
    client: C,
}

#[allow(unused)]
impl<C: Client> RequestProcessor<C, Identity> {
    pub fn from_client(client: C) -> RequestProcessor<C, Identity> {
        RequestProcessor {
            layer: Identity,
            client,
        }
    }
}

#[allow(unused)]
impl<C: Client, L: Layer> RequestProcessor<C, L> {
    pub fn new(client: C, layer: L) -> RequestProcessor<C, L> {
        RequestProcessor { layer, client }
    }

    pub fn set_client<NC: Client>(self, client: NC) -> RequestProcessor<NC, L> {
        RequestProcessor {
            layer: self.layer,
            client,
        }
    }

    pub fn set_layer<NL: Layer>(self, layer: NL) -> RequestProcessor<C, NL> {
        RequestProcessor {
            layer,
            client: self.client,
        }
    }

    pub fn layer<NL: Layer>(self, layer: NL) -> RequestProcessor<C, Chain<L, NL>> {
        RequestProcessor {
            layer: Chain::new(self.layer, layer),
            client: self.client,
        }
    }

    pub async fn call(&self, req: Request) -> Result<Response> {
        let send = &|req| self.client.send(req);
        self.layer.call(req, send).await
    }

    pub fn as_handler(self) -> impl Handler {
        let client = Arc::new(self.client);
        let layer = Arc::new(self.layer);

        move |req: Request| {
            let client = client.clone();
            let layer = layer.clone();

            let send = move |req| client.send(req);
            async move { layer.call(req, send).await }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn make_stack() {
        let _s: Chain<Identity, Identity> = Chain::new(Identity, Identity);
    }
}
