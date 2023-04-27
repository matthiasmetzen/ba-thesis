use futures::future::BoxFuture;
use hyper::service::{make_service_fn, service_fn};
use miette::{Result, miette};

use crate::request::{Request, Response};

use std::net::TcpListener;
use std::ops::Deref;
use std::sync::Arc;
use std::future::Future;

use tracing::info;

pub trait ServerBuilder {
    type Request;

    fn serve(&self, handler: impl Handler) -> Result<impl Server>;
}

pub trait Server: Send {
    async fn stop(self) -> Result<()>;
}

pub trait Handler<Req: Clone = Request, Resp = Response>: Send + Sync {
    type Future: Future<Output = Result<Resp>> + Send;

    fn handle(&self, msg: Req) -> Self::Future;
}

// Accept async closures as handlers
impl<Fun, Fut, Req, Resp> Handler<Req, Resp> for Fun
where
    Req: Clone + Send + Sync + 'static,
    Fun: Fn(Req) -> Fut + Send + Sync,
    Fut: Future<Output = Result<Resp>> + Send
{
    type Future = impl Future<Output = Result<Resp>> + Send;

    fn handle(&self, msg: Req) -> Self::Future {
        self(msg)
    }
}

impl<Req, Resp, H: Handler<Req, Resp>> Handler<Req, Resp> for Arc<H> 
where
    Req: Clone + Send + Sync + 'static,
    Resp: Send + Sync,
{
    type Future = H::Future;

    fn handle(&self, msg: Req) -> Self::Future {
        self.deref().handle(msg)
    }
}

struct S3Server<'a> {
    fut: BoxFuture<'a, Result<()>>,
    term_sig: tokio::sync::oneshot::Sender<()>,
}

#[derive(Clone)]
pub struct S3ServerBuilder {
    pub host: String,
    pub port: u16,
}

impl S3ServerBuilder {
    #[allow(unused)]
    pub fn new(host: String, port: u16) -> Self {
        Self {
            host,
            port,
        }
    }
}

impl<'a> Server for S3Server<'a> {
    async fn stop(self) -> Result<()> {
        self.term_sig.send(()).map_err(|_| miette!("Failed to send stop signal"))?;
        self.fut.await.ok();
        Ok(())
    }
}

impl ServerBuilder for S3ServerBuilder {
    type Request = s3s::http::Request;

    fn serve(&self, handler: impl Handler + 'static) -> Result<impl Server> {
        let h = Arc::new(handler);
        let svc_fn = move |req: hyper::Request<hyper::Body>| {
            let h = h.clone();
            async move {
                let req = req.map(s3s::Body::from);
                let req = s3s::http::Request::from(req);

                let resp = h.handle(req.into()).await;
                match resp {
                    Ok(r) => Ok(r.into()),
                    Err(e) => Err(e),
                }
            }
        };

        let svc_fn = Arc::new(svc_fn);
        let make_svc = make_service_fn(move |_| {
            let svc_fn = svc_fn.clone();
            std::future::ready(Ok::<_, std::convert::Infallible>(service_fn(move |req| svc_fn.call((req,)))))
        });

        // Run server
        let listener = TcpListener::bind((self.host.as_str(), self.port))
            .map_err(|e| miette::miette!(e))?;
        let server = hyper::Server::from_tcp(listener)
            .map_err(|e| miette::miette!(e))?
            .serve(make_svc);

        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let server = server
            .with_graceful_shutdown(async {
                rx.await.ok();
            });

        let task = tokio::spawn(server);
        info!("server is running at http://{}:{}/", self.host, self.port);

        let srv = S3Server {
            fut: Box::pin(Box::pin(async move { 
                let _ = task.await.map_err(|e| miette::miette!(e))?; 
                Ok(())
            })),
            term_sig: tx,
        };

        Ok(srv)
    }
}