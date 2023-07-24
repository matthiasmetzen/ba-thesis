use crate::{
    config::MiddlewareType,
    req::{Request, Response, SendError},
    webhook::BroadcastSend,
};

pub mod cache;
pub use self::cache::CacheLayer;

use crate::{client::Client, server::Handler};

use std::{future::Future, sync::Arc};

/// Represents a middleware. unlike middlewares provided by tower, this implementation is object-safe.
#[async_trait::async_trait]
pub trait Layer: Send + Sync {
    /// Takes a [Request] and a handler that will resolve the request when called and resolves the request
    async fn call(&self, req: Request, next: &dyn NextLayer) -> Result<Response, SendError>;

    // Subscribe to broadcast events
    fn subscribe(&mut self, _tx: &BroadcastSend) {}

    // Unsubscribe from broadcast events
    fn unsubscribe(&mut self) {}
}

/// Represents a middleware stack that uses dynamic dispatch
/// Can be cained by nesting
pub struct DynChain {
    current: Box<dyn Layer>,
    next: Box<dyn Layer>,
}

/// Build a new middleware stack from the provided configuration
impl From<&Vec<MiddlewareType>> for DynChain {
    fn from(config: &Vec<MiddlewareType>) -> Self {
        let mut chain = DynChain::new(Box::new(Identity), Box::new(Identity));

        for t in config {
            let layer: Box<dyn Layer> = match t {
                MiddlewareType::Cache(c) => Box::new(CacheLayer::from(c)),
                MiddlewareType::Identity => Box::new(Identity),
            };

            chain = chain.then(layer);
        }

        chain
    }
}

impl DynChain {
    /// Create a new [DynChain]
    pub fn new(current: Box<dyn Layer>, next: Box<dyn Layer>) -> Self {
        Self { current, next }
    }

    /// Append a new [Layer] to the stack
    pub fn then(self, next: Box<dyn Layer>) -> DynChain {
        Self::new(Box::new(self), next)
    }
}

#[async_trait::async_trait]
impl Layer for DynChain {
    async fn call(&self, req: Request, next: &dyn NextLayer) -> Result<Response, SendError> {
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

/// Representation of a middeware stack using generics. Faster than [DynChain] but can not be used in every situation.
// based on https://github.com/tower-rs/tower/blob/master/tower-layer/src/stack.rs#L5
/// Can be cained by nesting
pub struct Chain<Current: Layer, Next: Layer> {
    current: Current,
    next: Next,
}

#[allow(unused)]
impl<C: Layer, N: Layer> Chain<C, N> {
    /// Create a new [Chain]
    pub fn new(current: C, next: N) -> Self {
        Self { current, next }
    }

    /// Append a new [Layer] to the stack
    pub fn then<L: Layer>(self, next: L) -> Chain<Self, L> {
        Chain::new(self, next)
    }
}

/// A handler that will resolve a request.
/// Implemented for `async Fn(Request) -> Result<Response, SendError>`
#[async_trait::async_trait]
pub trait NextLayer: Send + Sync {
    async fn call(&self, req: Request) -> Result<Response, SendError>;
}

#[async_trait::async_trait]
impl<Fun, Fut> NextLayer for Fun
where
    Fun: Fn(Request) -> Fut + Send + Sync,
    Fut: Future<Output = Result<Response, SendError>> + Send,
{
    async fn call(&self, req: Request) -> Result<Response, SendError> {
        self(req).await
    }
}

#[async_trait::async_trait]
impl<Fun> Layer for Fun
where
    Fun: Fn(
            Request,
            &dyn NextLayer,
        ) -> std::pin::Pin<Box<dyn Future<Output = Result<Response, SendError>> + Send>>
        + Send
        + Sync,
{
    async fn call(&self, req: Request, next: &dyn NextLayer) -> Result<Response, SendError> {
        self(req, next).await
    }
}

#[async_trait::async_trait]
impl<C: Layer, N: Layer> Layer for Chain<C, N> {
    async fn call(&self, req: Request, next: &dyn NextLayer) -> Result<Response, SendError> {
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

/// This [Layer] does nothing. It is used as a placeholder.
pub struct Identity;

#[async_trait::async_trait]
impl Layer for Identity {
    async fn call(&self, req: Request, next: &dyn NextLayer) -> Result<Response, SendError> {
        next.call(req).await
    }
}

/// Combines a [Layer] with a [Client] that will eventually resolve the request.
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

    pub async fn call(&self, req: Request) -> Result<Response, SendError> {
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
