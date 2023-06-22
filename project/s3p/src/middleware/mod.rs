use crate::{
    config::MiddlewareType,
    req::{Request, Response},
    webhook::BroadcastSend,
};
use miette::Result;

pub mod cache;
pub use self::cache::CacheLayer;

use crate::{client::Client, server::Handler};

use std::{future::Future, sync::Arc};

#[async_trait::async_trait]
pub trait Layer: Send + Sync {
    async fn call(&self, req: Request, next: &dyn NextLayer) -> Result<Response>;

    fn subscribe(&mut self, _tx: &BroadcastSend) {}

    fn unsubscribe(&mut self) {}
}

pub struct MiddlewareUtil;

impl MiddlewareUtil {
    pub fn from_config(config: &Vec<MiddlewareType>) -> impl Layer {
        let mut chain = DynChain::new(Box::new(Identity), Box::new(Identity));

        for t in config {
            let layer: Box<dyn Layer> = match t {
                MiddlewareType::Cache(c) => Box::new(CacheLayer::from_config(c.clone())),
                MiddlewareType::Identity => Box::new(Identity),
            };

            chain = chain.then(layer);
        }

        chain
    }
}

pub struct DynChain {
    current: Box<dyn Layer>,
    next: Box<dyn Layer>,
}

impl DynChain {
    pub fn new(current: Box<dyn Layer>, next: Box<dyn Layer>) -> Self {
        Self { current, next }
    }

    pub fn then(self, next: Box<dyn Layer>) -> DynChain {
        Self::new(Box::new(self), next)
    }
}

#[async_trait::async_trait]
impl Layer for DynChain {
    async fn call(&self, req: Request, next: &dyn NextLayer) -> Result<Response> {
        let then = |req| self.next.call(req, next);
        self.current.call(req, &then).await
    }

    fn subscribe(&mut self, tx: &BroadcastSend) {
        self.current.subscribe(tx);
        self.next.subscribe(tx);
    }

    fn unsubscribe(&mut self) {
        self.current.unsubscribe();
        self.next.unsubscribe();
    }
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
pub trait NextLayer: Send + Sync {
    async fn call(&self, req: Request) -> Result<Response>;
}

#[async_trait::async_trait]
impl<Fun, Fut> NextLayer for Fun
where
    Fun: Fn(Request) -> Fut + Send + Sync,
    Fut: Future<Output = Result<Response>> + Send,
{
    async fn call(&self, req: Request) -> Result<Response> {
        self(req).await
    }
}

#[async_trait::async_trait]
impl<Fun> Layer for Fun
where
    Fun: Fn(
            Request,
            &dyn NextLayer,
        ) -> std::pin::Pin<Box<dyn Future<Output = Result<Response>> + Send>>
        + Send
        + Sync,
{
    async fn call(&self, req: Request, next: &dyn NextLayer) -> Result<Response> {
        self(req, next).await
    }
}

#[async_trait::async_trait]
impl<C: Layer, N: Layer> Layer for Chain<C, N> {
    async fn call(&self, req: Request, next: &dyn NextLayer) -> Result<Response> {
        let then = |req| self.next.call(req, next);
        self.current.call(req, &then).await
    }

    fn subscribe(&mut self, tx: &BroadcastSend) {
        self.current.subscribe(tx);
        self.next.subscribe(tx);
    }

    fn unsubscribe(&mut self) {
        self.current.unsubscribe();
        self.next.unsubscribe();
    }
}

pub struct Identity;

#[async_trait::async_trait]
impl Layer for Identity {
    async fn call(&self, req: Request, next: &dyn NextLayer) -> Result<Response> {
        next.call(req).await
    }
}

pub struct RequestProcessor<C: Client, L: Layer = Identity> {
    layer: L,
    client: Arc<C>,
}

#[allow(unused)]
impl<C: Client> RequestProcessor<C, Identity> {
    pub fn from_client(client: C) -> RequestProcessor<C, Identity> {
        RequestProcessor {
            layer: Identity,
            client: Arc::new(client),
        }
    }
}

#[allow(unused)]
impl<C: Client + 'static, L: Layer> RequestProcessor<C, L> {
    pub fn new(client: C, layer: L) -> RequestProcessor<C, L> {
        RequestProcessor {
            layer,
            client: Arc::new(client),
        }
    }

    pub fn set_client<NC: Client>(self, client: NC) -> RequestProcessor<NC, L> {
        RequestProcessor {
            layer: self.layer,
            client: Arc::new(client),
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
        let client = self.client.clone();
        let send = move |req| client.send(req);
        self.layer.call(req, &send).await
    }

    pub fn subscribe(self, tx: &BroadcastSend) -> Self {
        let mut this = self;
        this.layer.subscribe(tx);
        this
    }

    pub fn unsubscribe(&mut self) {
        self.layer.unsubscribe();
    }

    pub fn into_handler(self) -> impl Handler {
        //let client = Arc::new(self.client);
        //let layer = Arc::new(self.layer);
        let this = Arc::new(self);

        move |req: Request| {
            let this = this.clone();
            let client = this.client.clone();

            let send = move |req| client.send(req);
            async move { this.layer.call(req, &send).await }
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
