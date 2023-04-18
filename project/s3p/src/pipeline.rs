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
        }
    }
}
