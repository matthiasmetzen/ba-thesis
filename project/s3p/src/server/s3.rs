use futures::{future::BoxFuture, FutureExt};
use http::StatusCode;
use hyper::service::{make_service_fn, service_fn};
use miette::{miette, Result};
use s3s::auth::S3Auth;
use tokio::sync::broadcast::Sender;

use super::{Handler, Server, ServerBuilder};
use crate::{
    middleware::Event,
    req::{Request, S3Extension},
};

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
    pub broadcast_tx: Option<Sender<Event>>,
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
    fn broadcast(self, tx: &Sender<Event>) -> Self {
        let mut this = self;
        this.broadcast_tx = Some(tx.clone());
        this
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
                s3_ext.op = Some(op.into());

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
            Some(tx) => Some(S3WebhookServer::new(
                self.host.as_str(),
                self.port + 1,
                tx.clone(),
            )?),
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
                if let Some(a) = broadcast.take() { drop(a) }
                Ok(())
            }),
            term_sig: tx,
        };

        Ok(srv)
    }
}

#[allow(dead_code)]
struct S3WebhookServer<'a> {
    tx: Sender<Event>,
    fut: BoxFuture<'a, Result<()>>,
    term_sig: tokio::sync::oneshot::Sender<()>,
}

impl S3WebhookServer<'_> {
    fn new(host: &str, port: u16, tx: Sender<Event>) -> Result<Self> {
        let make_svc = {
            let tx = tx.clone();
            make_service_fn(move |_| {
                let tx = tx.clone();
                std::future::ready(Ok::<_, std::convert::Infallible>(service_fn(move |req| {
                    let _ = tx.send(req.uri().to_string().trim_start_matches('/').to_string());

                    async move {
                        hyper::Response::builder()
                            .status(StatusCode::NOT_IMPLEMENTED)
                            .body(hyper::Body::default())
                    }
                })))
            })
        };

        let listener = TcpListener::bind((host, port)).map_err(|e| miette::miette!(e))?;
        let server = hyper::Server::from_tcp(listener)
            .map_err(|e| miette::miette!(e))?
            .serve(make_svc);

        let (term_sig_tx, term_sig_rx) = tokio::sync::oneshot::channel::<()>();
        let server = server.with_graceful_shutdown(async {
            term_sig_rx.await.ok();
        });

        let task = tokio::spawn(server);
        info!("Webhook is running at http://{}:{}/", host, port);

        Ok(Self {
            tx,
            term_sig: term_sig_tx,
            fut: Box::pin(async move {
                let _ = task.await.map_err(|e| miette::miette!(e))?;
                Ok(())
            }),
        })
    }

    async fn stop(self) -> Result<()> {
        self.term_sig
            .send(())
            .map_err(|_| miette!("Failed to send stop signal"))?;
        self.fut.await.ok();
        Ok(())
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
