use crate::webhook::{BroadcastSend, WebhookEvent};
use futures::{future::BoxFuture, TryFutureExt};
use http::StatusCode;
use hyper::service::{make_service_fn, service_fn};
use miette::{miette, Result};
use serde_json::json;
use tower::timeout::Timeout;

use std::{net::TcpListener, time::Duration};

use tracing::{error, info};

use super::{WebhookServer, WebhookServerBuilder};

#[allow(dead_code)]
pub struct S3WebhookServer<'a> {
    tx: BroadcastSend,
    fut: BoxFuture<'a, Result<()>>,
    term_sig: tokio::sync::oneshot::Sender<()>,
}

#[async_trait::async_trait]
impl WebhookServer for S3WebhookServer<'_> {
    async fn stop(self) -> Result<()> {
        self.term_sig
            .send(())
            .map_err(|_| miette!("Failed to send stop signal"))?;
        self.fut.await.ok();
        Ok(())
    }
}

#[derive(Default)]
pub struct S3WebhookServerBuilder {
    pub host: String,
    pub port: u16,
}

#[allow(unused)]
impl S3WebhookServerBuilder {
    pub fn new(host: String, port: u16) -> Self {
        Self {
            host,
            port,
            ..Default::default()
        }
    }
}

impl WebhookServerBuilder for S3WebhookServerBuilder {
    fn serve(&self, tx: &BroadcastSend) -> Result<impl WebhookServer> {
        let make_svc = {
            let tx = tx.clone();
            make_service_fn(move |_| {
                let tx = tx.clone();
                std::future::ready(Ok::<_, std::convert::Infallible>(service_fn(move |req| {
                    let tx = tx.clone();
                    async move {
                        let event = match WebhookEvent::from_request(req).await {
                            Ok(event) => event,
                            Err(err) => {
                                error!("{}", err);
                                return hyper::Response::builder()
                                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                                    .body(
                                        json!({
                                            "message": "Failed to parse json",
                                            "error": err.to_string(),
                                        })
                                        .to_string()
                                        .into(),
                                    );
                            }
                        };

                        let res = tx
                            .broadcast(event)
                            .inspect_err(|e| {
                                error!("{:?}", e);
                            })
                            .await;

                        let status = match res {
                            Ok(_) => StatusCode::OK,
                            Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
                        };

                        hyper::Response::builder()
                            .status(status)
                            .body(hyper::Body::default())
                    }
                })))
            })
        };

        let make_svc = Timeout::new(make_svc, Duration::from_secs(1));

        let listener =
            TcpListener::bind((self.host.as_str(), self.port)).map_err(|e| miette::miette!(e))?;
        let server = hyper::Server::from_tcp(listener)
            .map_err(|e| miette::miette!(e))?
            .serve(make_svc);

        let (term_sig_tx, term_sig_rx) = tokio::sync::oneshot::channel::<()>();
        let server = server.with_graceful_shutdown(async {
            term_sig_rx.await.ok();
        });

        let task = tokio::spawn(server);
        info!("Webhook is running at http://{}:{}/", self.host, self.port);

        Ok(S3WebhookServer {
            tx: tx.clone(),
            term_sig: term_sig_tx,
            fut: Box::pin(async move {
                let _ = task.await.map_err(|e| miette::miette!(e))?;
                Ok(())
            }),
        })
    }
}
