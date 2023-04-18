use tracing::error;

use crate::{middleware::{Layer, MiddlewareAction}, server::{Server}, client::Client, request::{Response, Request}};

    server: S,
    middleware: M,
    client: C,
}
impl<S, M, C> Pipeline<S, M, C> 
where
    S: Server,
    M: Layer,
    C: Client
{
    async fn run(&self) {
        while let Some(req) = self.server.accept().await {
            let res = self.middleware.process_request(req).await;
            let resp = match res {
                Ok(MiddlewareAction::Forward(req)) => self.client.send(req).await,
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