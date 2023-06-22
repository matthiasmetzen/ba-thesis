use super::{Handler, Server, ServerBuilder};
use crate::{
    config::S3ServerConfig,
    req::{Request, S3Extension},
    webhook::{s3::S3WebhookServerBuilder, BroadcastSend, WebhookServer, WebhookServerBuilder},
};
use futures::{future::BoxFuture, FutureExt};
use hyper::service::{make_service_fn, service_fn};
use miette::{miette, Result};
use s3s::auth::{S3Auth, SimpleAuth};

use std::net::TcpListener;
use std::sync::Arc;

use tracing::{debug, info};

struct S3Server<'a> {
    fut: BoxFuture<'a, Result<()>>,
    term_sig: tokio::sync::oneshot::Sender<()>,
}

impl<'a> Server for S3Server<'a> {
    async fn stop(self) -> Result<()> {
        self.term_sig
            .send(())
            .map_err(|_| miette!("Failed to send stop signal"))?;
        self.fut.await.ok();
        Ok(())
    }
}

#[derive(Default)]
pub struct S3ServerBuilder {
    pub host: String,
    pub port: u16,
    pub auth: Option<Arc<Box<dyn S3Auth>>>,
    pub base_domain: Option<String>,
    pub broadcast_tx: Option<BroadcastSend>,
}

#[allow(unused)]
impl S3ServerBuilder {
    pub fn new(host: String, port: u16) -> Self {
        Self {
            host,
            port,
            ..Default::default()
        }
    }

    pub fn auth(mut self, auth: Option<impl S3Auth>) -> Self {
        self.auth = auth.map(|a| Arc::new(Box::new(a) as Box<dyn S3Auth>));
        self
    }

    pub fn base_domain(mut self, base_domain: impl Into<Option<String>>) -> Self {
        self.base_domain = base_domain.into();
        self
    }
}

impl ServerBuilder for S3ServerBuilder {
    fn broadcast(&mut self, tx: &BroadcastSend) -> &mut Self {
        self.broadcast_tx = Some(tx.clone());
        self
    }

    fn serve(&self, handler: impl Handler + 'static) -> Result<impl Server> {
        let h = Arc::new(handler);
        let auth = self.auth.clone();
        let base_domain = Arc::new(self.base_domain.clone());

        let mut broadcast = self.broadcast_tx.clone();

        let svc_fn = move |req: hyper::Request<hyper::Body>| {
            let h = h.clone();
            let auth = auth.clone();
            let base_domain = base_domain.clone();

            async move {
                let req = req.map(s3s::Body::from);

                let mut req = s3s::http::Request::from(req);

                let auth = auth.as_deref().map(|a| a.as_ref());
                let base_domain = base_domain.as_deref();

                let op = s3s::ops::prepare(&mut req, auth, base_domain)
                    .await
                    .map_err(|e| miette!(e))?;

                let mut req = Request::from(req);
                let s3_ext = req
                    .extensions
                    .get_mut::<S3Extension>()
                    .ok_or_else(|| miette!("Could not find S3Extension"))?;
                s3_ext.op = Some(op);

                debug!("{:#?}", req);

                let resp = h.handle(req).await;
                match resp {
                    Ok(r) => Ok(r.into()),
                    Err(e) => Err(e),
                }
            }
        };

        let svc_fn = Arc::new(svc_fn);
        let make_svc = make_service_fn(move |_| {
            let svc_fn = svc_fn.clone();
            std::future::ready(Ok::<_, std::convert::Infallible>(service_fn(move |req| {
                svc_fn.call((req,)).map(|res| match res {
                    Ok(_) => res,
                    Err(err) => {
                        let body = hyper::Body::from(err.to_string());
                        hyper::Response::builder()
                            .status(500)
                            .body(body)
                            .map_err(|e| miette!(e))
                    }
                })
            })))
        });

        // Run server
        let listener =
            TcpListener::bind((self.host.as_str(), self.port)).map_err(|e| miette::miette!(e))?;
        let server = hyper::Server::from_tcp(listener)
            .map_err(|e| miette::miette!(e))?
            .serve(make_svc);

        let webhook = match broadcast.as_ref() {
            Some(tx) => {
                Some(S3WebhookServerBuilder::new(self.host.clone(), self.port + 1).serve(&tx)?)
            }
            None => None,
        };

        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let server = server.with_graceful_shutdown(async {
            rx.await.ok();
            if let Some(hook) = webhook {
                let _ = hook.stop().await;
            }
        });

        let task = tokio::spawn(server);
        info!("server is running at http://{}:{}/", self.host, self.port);

        let srv = S3Server {
            fut: Box::pin(async move {
                let _ = task.await.map_err(|e| miette::miette!(e))?;
                // Ensure broadcast channel lives until the server stops
                // TODO: Send Shutdown message?
                if let Some(a) = broadcast.take() {
                    drop(a)
                }
                Ok(())
            }),
            term_sig: tx,
        };

        Ok(srv)
    }
}

impl From<&S3ServerConfig> for S3ServerBuilder {
    fn from(config: &S3ServerConfig) -> Self {
        let mut builder = S3ServerBuilder::new(config.host.clone(), config.port);

        if config.validate_credentials && config.credentials.is_some() {
            let creds = config.credentials.as_ref().unwrap();

            builder = builder.auth(Some(SimpleAuth::from_single(
                creds.access_key_id.as_str(),
                creds.secret_key.as_str(),
            )));
        }

        builder = builder.base_domain(config.base_domain.clone());

        builder
    }
}

#[cfg(test)]
mod tests {

    use ctor::ctor;
    use tokio::signal::ctrl_c;

    use crate::req::Response;

    use super::*;

    #[ctor]
    fn prepare() {
        let _ = crate::try_init_tracing();
    }

    // Runs until Ctrl+C is received
    #[tokio::test]
    async fn run_s3_server() -> Result<()> {
        let server = S3ServerBuilder::new("localhost".into(), 3000);

        let handler = |_req| async { Ok(Response::default()) };

        let server = server.serve(handler)?;

        ctrl_c().await.map_err(|e| miette::miette!(e))?; // FIXME: Temporary
        server.stop().await?;

        Ok(())
    }
}
