use async_broadcast::broadcast;
use miette::Result;

use crate::{
    client::Client,
    middleware::{Layer, RequestProcessor},
    server::{Server, ServerBuilder},
};

pub struct Pipeline<S, M, C>
where
    S: ServerBuilder + Send + Sync,
    M: Layer + Send + Sync,
    C: Client + Send + Sync,
{
    server: S,
    middleware: M,
    client: C,
}

impl<S, M, C> Pipeline<S, M, C>
where
    S: ServerBuilder + Send + Sync,
    M: Layer + Send + Sync,
    C: Client + Send + Sync + 'static,
{
    pub fn new(server: S, middleware: M, client: C) -> Self {
        Self {
            server,
            middleware,
            client,
        }
    }

    #[allow(unused)]
    pub async fn run(mut self) -> Result<impl Server> {
        // TODO: make cap configurable
        let (mut tx, rx) = broadcast(256);
        let rx = rx.deactivate();

        tx.set_await_active(false);

        let handler = RequestProcessor::new(self.client, self.middleware)
            .subscribe(&tx)
            .into_handler();

        let server = self.server.broadcast(&tx).serve(handler)?;

        // drop rx late so the channel doesn't close
        drop(rx);

        Ok(server)
    }
}

#[cfg(test)]
mod tests {
    use std::pin::Pin;

    use crate::{
        client::s3::S3Client,
        middleware::{Chain, Identity},
        req::{Request, Response},
        server::{Handler, S3ServerBuilder, Server},
        webhook::BroadcastSend,
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

        fn broadcast(&mut self, _tx: &BroadcastSend) -> &mut Self {
            self
        }
    }

    #[tokio::test]
    async fn test_run_pipeline() -> Result<()> {
        let s3 = S3ServerBuilder::new("localhost".into(), 3000);
        let middleware = Chain::new(Identity, Identity);
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
