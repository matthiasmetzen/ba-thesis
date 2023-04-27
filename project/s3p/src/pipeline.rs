use std::{sync::Arc, ops::Deref};


use miette::Result;

use crate::{middleware::{Layer, MiddlewareAction}, server::{ServerBuilder, Server}, client::Client, request::Request};

pub struct PipelineInner<S: ServerBuilder, M: Layer, C: Client> {
    server: S,
    middleware: M,
    client: C,
}

pub struct Pipeline<S, M, C>(Arc<PipelineInner<S, M, C>>)
where
    S: ServerBuilder + Send + Sync,
    M: Layer + Send + Sync,
    C: Client + Send + Sync;

impl<S, M, C> Pipeline<S, M, C> 
where
    S: ServerBuilder + Send + Sync,
    M: Layer + Send + Sync,
    C: Client + Send + Sync
{
    #[allow(unused)]
    pub fn new(server: S, middleware: M, client: C) -> Self {
        Self(Arc::new(
                PipelineInner {
                    server,
                    middleware,
                    client,
                }
            ))
    }
}

impl<S, M, C> Clone for Pipeline<S, M, C> 
where
    S: ServerBuilder + Send + Sync,
    M: Layer + Send + Sync,
    C: Client + Send + Sync
{
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<S, M, C> Deref for Pipeline<S, M, C> 
where
    S: ServerBuilder + Send + Sync,
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
    S: ServerBuilder + Send + Sync,
    M: Layer + Send + Sync,
    C: Client + Send + Sync
{
    fn from(value: PipelineInner<S, M, C>) -> Self {
        Self(Arc::new(value))
    }
}

impl<S, M, C> Pipeline<S, M, C> 
where
    S: ServerBuilder + Send + Sync,
    M: Layer + Send + Sync,
    C: Client + Send + Sync
{
    #[allow(unused)]
    async fn run(&self) -> Result<impl Server + '_> {
        let handler = move |req: Request| async {
            let res = self.middleware.process_request(req).await;
            match res {
                Ok(MiddlewareAction::Forward(req)) => self.client.send(req).await,
                Ok(MiddlewareAction::Reply(res)) => Ok(res),
                Err(e) => Err(e),
            }
        };

        //let handler = Arc::new(handler);
        //self.server.register_handler(handler);

        let server = self.server.serve(handler)?;

        Ok(server)
    }
}

#[cfg(test)]
mod tests {
    use crate::{request::{Request, Response}, middleware::{Stack, Identity}, server::{Handler, Server, S3ServerBuilder}};
    use futures::Future;
    use miette::Result;

    use super::*;

    pub struct StubClient;
    impl Client for StubClient {
        fn send(&self, _request: Request) -> impl Future<Output = Result<Response>> {
            async {
                Ok(Response)
            }
        }
    }

    pub struct StubServer;
    impl Server for StubServer {
        async fn stop(self) -> Result<()> {
            todo!()
        }
    }
    pub struct StubServerBuilder;


    impl ServerBuilder for StubServerBuilder {
        type Request = Request;

        fn serve(&self, _handler: impl Handler) -> Result<impl Server> {
            Ok(StubServer)
        }
    }

    #[tokio::test]
    async fn test_run() -> Result<()> {
        let s3 = S3ServerBuilder::new("localhost".into(), 3000);
        let p = Pipeline::new(s3, Stack::new(Identity, Identity), StubClient);
        let server = p.run().await?;

        server.stop().await?;

        Ok(())
    }
}