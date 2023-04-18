use std::{sync::Arc, ops::{Deref, DerefMut}};

use futures::{channel::mpsc::UnboundedReceiver, StreamExt};
use tracing::error;

use crate::{middleware::{Layer, MiddlewareAction}, server::{Server}, client::Client, request::{Response, Request}};

pub struct PipelineInner<S: Server, M: Layer, C: Client> {
    server: S,
    middleware: M,
    client: C,

    rx: UnboundedReceiver<Request>
}

pub struct Pipeline<S, M, C>(Arc<PipelineInner<S, M, C>>)
where
    S: Server + Send + Sync,
    M: Layer + Send + Sync,
    C: Client + Send + Sync;

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

impl<S, M, C> DerefMut for Pipeline<S, M, C> 
where
    S: Server + Send + Sync,
    M: Layer + Send + Sync,
    C: Client + Send + Sync
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0.deref_mut()
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
    async fn run(mut self) {
        let mut pipe = self.clone();

        let srv_handle = self.server.serve();

        while let Some(req) = self.rx.next().await {
            tokio::task::spawn(async move {
                let res = pipe.middleware.process_request(req).await;
                let resp = match res {
                    Ok(MiddlewareAction::Forward(req)) => pipe.client.send(req).await,
                    Ok(MiddlewareAction::Reply(res)) => Ok(res),
                    Err(e) => {
                        let _ = req.reply(e).await
                            .map_err(|e| error!("Failed to reply with response {:?}", e));
                        return;
                    }
                };

                match resp {
                    Ok(r) => {
                        let _ = req.reply(r).await
                            .map_err(|e| error!("Failed to reply with response {:?}", e));
                        return;
                    },
                    Err(e) => {
                        let _ = req.reply(e).await
                            .map_err(|e| error!("Failed to reply with response {:?}", e));
                        return;
                    },
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{request::{Request, Response}, middleware::{Stack, Identity}};
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
    async fn accept(&self) -> Result<Request> {
        todo!()
    }

    async fn reply(&self, response: Response) -> Result<Response> {
        todo!()
    }
}

    #[tokio::test]
    async fn test_run() {
        let p = Pipeline {
            input: StubServer,
            middlewares: Stack::new(Identity, Identity),
            output: StubClient,
        };
        p.run().await;

        ()
    }
}