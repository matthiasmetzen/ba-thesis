use std::{sync::Arc, ops::Deref};

use futures::{StreamExt, SinkExt};
use tracing::error;

use miette::WrapErr;

use crate::{middleware::{Layer, MiddlewareAction}, server::{Server}, client::Client, request::{Response, Request}};

pub struct PipelineInner<S: Server, M: Layer, C: Client> {
    server: S,
    middleware: M,
    client: C,
}

pub struct Pipeline<S, M, C>(Arc<PipelineInner<S, M, C>>)
where
    S: Server + Send + Sync,
    M: Layer + Send + Sync,
    C: Client + Send + Sync;

impl<S, M, C> Pipeline<S, M, C> 
where
    S: Server + Send + Sync,
    M: Layer + Send + Sync,
    C: Client + Send + Sync
{
    pub fn new(server: S, middleware: M, client: C) -> Self {
        Self {
            0: Arc::new(
                PipelineInner {
                    server,
                    middleware,
                    client,
                }
            )
        }
    }
}

impl<S, M, C> Clone for Pipeline<S, M, C> 
where
    S: Server + Send + Sync,
    M: Layer + Send + Sync,
    C: Client + Send + Sync
{
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<S, M, C> Deref for Pipeline<S, M, C> 
where
    S: Server + Send + Sync,
    M: Layer + Send + Sync,
    C: Client + Send + Sync
{
    type Target = PipelineInner<S, M, C>;

    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}

impl<S, M, C> From<PipelineInner<S, M, C>> for Pipeline<S, M, C>
where
    S: Server + Send + Sync,
    M: Layer + Send + Sync,
    C: Client + Send + Sync
{
    fn from(value: PipelineInner<S, M, C>) -> Self {
        Self(Arc::new(value))
    }
}

impl<S, M, C> Pipeline<S, M, C> 
where
    S: Server + Send + Sync,
    M: Layer + Send + Sync,
    C: Client + Send + Sync
{
    async fn run(&self) {
        let (tx, mut rx) = futures::channel::mpsc::unbounded();

        self.server.register_handler(Box::new(move |req: Request| {
            let (otx, orx) = futures::channel::oneshot::channel::<Response>();
            let mut tx = tx.clone();

            async move {

                tx.send((req, otx)).await.map_err(|e| miette::miette!(e))?;
                let res = orx.await
                    .map_err(|e| miette::miette!(e))
                    .wrap_err("Failed to receive response from channel");

                res
            }
        }));

        let srv_handle = self.server.serve();

        while let Some((req, otx)) = rx.next().await {
            let res = self.middleware.process_request(req).await;
            let resp = match res {
                Ok(MiddlewareAction::Forward(req)) => self.client.send(req).await,
                Ok(MiddlewareAction::Reply(res)) => Ok(res),
                Err(e) => {
                    let _ = otx.send(e.into())
                        .map_err(|e| error!("Failed to reply with response {:?}", e));
                    return;
                }
            };

            match resp {
                Ok(r) => {
                    let _ = otx.send(r)
                        .map_err(|e| error!("Failed to reply with response {:?}", e));
                    return;
                },
                Err(e) => {
                    let _ = otx.send(e.into())
                        .map_err(|e| error!("Failed to reply with response {:?}", e));
                    return;
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{request::{Request, Response}, middleware::{Stack, Identity}, server::Handler};
    use miette::Result;

    use super::*;

    pub struct StubClient;
    impl Client for StubClient {
        async fn send(&self, request: Request) -> Result<Response> {
            todo!()
        }
    }

    pub struct StubServer;
    impl Server for StubServer {
        type Request = Request;

        type HandlerId = usize;

        async fn serve(&self) -> Result<impl futures::Future> {
            Ok(futures::future::pending::<()>())
        }

        async fn process(&self, request: Self::Request) -> Result<Response> {
            todo!()
        }

        fn register_handler(&self, handler: Box<dyn Handler<Request>>) -> Self::HandlerId {
            todo!()
        }

        fn unregister_handler(&self, idx: Self::HandlerId) {
            todo!()
        }
    }

    #[tokio::test]
    async fn test_run() {
        let p = Pipeline::new(StubServer, Stack::new(Identity, Identity), StubClient);
        p.run().await;

        ()
    }
}