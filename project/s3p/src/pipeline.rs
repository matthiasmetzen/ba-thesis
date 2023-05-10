use std::{ops::Deref, sync::Arc};

use miette::Result;

use crate::{
    client::Client,
    middleware::{Layer, MiddlewareAction},
    request::Request,
    server::{Server, ServerBuilder},
};

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
    C: Client + Send + Sync,
{
    #[allow(unused)]
    pub fn new(server: S, middleware: M, client: C) -> Self {
        Self(Arc::new(PipelineInner {
            server,
            middleware,
            client,
        }))
    }
}

impl<S, M, C> Clone for Pipeline<S, M, C>
where
    S: ServerBuilder + Send + Sync,
    M: Layer + Send + Sync,
    C: Client + Send + Sync,
{
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<S, M, C> Deref for Pipeline<S, M, C>
where
    S: ServerBuilder + Send + Sync,
    M: Layer + Send + Sync,
    C: Client + Send + Sync,
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
    C: Client + Send + Sync,
{
    fn from(value: PipelineInner<S, M, C>) -> Self {
        Self(Arc::new(value))
    }
}

impl<S, M, C> Pipeline<S, M, C>
where
    S: ServerBuilder + Send + Sync,
    M: Layer + Send + Sync,
    C: Client + Send + Sync,
{
    #[allow(unused)]
    pub async fn run(&self) -> Result<impl Server + '_> {
        let handler = move |req: Request| async {
            let res = self.middleware.process_request(req).await;
            match res {
                Ok(MiddlewareAction::Forward(req)) => self.client.send(req).await,
                Ok(MiddlewareAction::Reply(res)) => Ok(res),
                Err(e) => Err(e),
            }
        };

        let server = self.server.serve(handler)?;

        Ok(server)
    }
}

#[cfg(test)]
mod tests {
    use std::pin::Pin;

    use crate::{
        client::S3Client,
        middleware::{Identity, Stack},
        request::{Request, Response},
        server::{Handler, S3ServerBuilder, Server},
    };
    use ctor::ctor;
    use futures::Future;
    use miette::Result;
    use tokio::signal::ctrl_c;
    use tower::Service;

    use super::*;

    #[ctor]
    fn prepare() {
        let _ = crate::try_init_tracing();
    }

    pub struct StubClient;
    impl Service<Request> for StubClient {
        type Response = Response;

        type Error = miette::Error;

        type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>>>>;

        fn poll_ready(
            &mut self,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::result::Result<(), Self::Error>> {
            todo!()
        }

        fn call(&mut self, _req: Request) -> Self::Future {
            todo!()
        }
    }
    impl Client for StubClient {
        fn send(&self, _request: Request) -> impl Future<Output = Result<Response>> + Send {
            async { todo!() }
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
        fn serve(&self, _handler: impl Handler) -> Result<impl Server> {
            Ok(StubServer)
        }
    }

    #[tokio::test]
    async fn test_run_pipeline() -> Result<()> {
        let s3 = S3ServerBuilder::new("localhost".into(), 3000);
        let middleware = Stack::new(Identity, Identity);
        let client = S3Client::builder()
            .endpoint_url("http://localhost:9000")
            .credentials_from_single("user", "password")
            .build()?;
        let p = Pipeline::new(s3, middleware, client);
        let server = p.run().await?;

        ctrl_c().await.map_err(|e| miette::miette!(e))?; // FIXME: Temporary
        server.stop().await?;

        Ok(())
    }
}
