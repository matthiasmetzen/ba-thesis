use futures::{Sink};
use futures::channel::mpsc::{SendError};
use futures::future::{BoxFuture, select_all};
use hyper::service::Service;
use miette::Result;
use parking_lot::{Mutex, RwLock};
use tower::ServiceBuilder;
use tower::make::Shared;

use crate::request::{Request, Response};

use std::collections::BTreeMap;
use std::net::TcpListener;
use std::sync::Arc;
use std::future::Future;

use clap::Parser;
use tracing::info;

pub trait Server {
    type Request;
    type HandlerId = usize;

    async fn serve(&self) -> Result<impl Future>;
    async fn process(&self, request: Self::Request) -> Result<Response>;

    // Uses internal mutability
    fn register_handler(&self, handler: Box<dyn Handler<Request>>) -> Self::HandlerId;
    // Uses internal mutability
    fn unregister_handler(&self, idx: Self::HandlerId);
}

pub(crate) trait ShareableServer<Req: Send> : Send + Sync {
    type Response: Send;

    type Error: Send;

    fn call<'slf>(&'slf self, req: Req) -> impl Future<Output = Result<Self::Response, Self::Error>> + Send + 'slf;

    fn call_shared(self: Arc<Self>, req: Req) -> impl Future<Output = Result<Self::Response, Self::Error>> + Send
    where 
        Arc<Self>: 'static
    {
        let svc = self.clone();
        Box::pin(async move { svc.call(req).await })
    }
}

#[async_trait::async_trait]
pub trait Handler<Req: Clone = Request, Resp = Response>: Send + Sync {
    async fn handle(&self, msg: Req) -> Result<Resp>;
}

// Accept async closures as handlers
#[async_trait::async_trait]
impl<Fun, Fut, Req, Resp> Handler<Req, Resp> for Fun
where
    for <'a> Req: Clone + Send + Sync + 'a,
    Fun: Fn(Req) -> Fut + Send + Sync,
    Fut: Future<Output = Result<Resp>> + Send + Sync
{
    async fn handle(&self, msg: Req) -> Result<Resp> {
        self(msg).await
    }
}

impl Clone for Box<dyn Handler> {
    fn clone(&self) -> Self {
        self.to_owned()
    }
}

//from: https://github.com/Nugine/s3s/blob/main/crates/s3s-proxy/src/main.rs#L16
#[derive(Debug, Parser)]
struct Opt {
    #[clap(long, default_value = "localhost")]
    host: String,

    #[clap(long, default_value = "8014")]
    port: u16,

    #[clap(long)]
    domain_name: Option<String>,

    #[clap(long)]
    endpoint_url: String,
}

pub trait SrvSender = Sink<Request, Error = SendError> + Send + Sync + Clone + 'static + std::marker::Unpin;
#[derive(Default)]
struct S3Server {
    cb: RwLock<BTreeMap<usize, Box<dyn Handler>>>,
    next_id: Mutex<usize>,
}

impl Clone for S3Server {
    fn clone(&self) -> Self {
        Self { 
            ..Default::default()
        }
    }
}

impl S3Server {
    #[allow(unused)]
    fn new() -> Self {
        Self {
            ..Default::default()
        }
    }
}

impl ShareableServer<hyper::Request<hyper::Body>> for S3Server {
    type Response = hyper::Response<hyper::Body>;

    type Error = miette::Report;

    fn call<'slf>(&'slf self, req: hyper::Request<hyper::Body>) -> impl Future<Output = Result<Self::Response, Self::Error>> + Send + 'slf{
        let req = req.map(s3s::Body::from);
        let request = s3s::http::Request::from(req);

        async move {
            let res = self.process(request).await?;
            Ok(res.into())
        }
    }
}

/* impl Service<hyper::Request<hyper::Body>> for S3Server {
    type Response = hyper::Response<hyper::Body>;

    type Error = miette::Report;

    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut std::task::Context<'_>) -> std::task::Poll<std::result::Result<(), Self::Error>> {
        std::task::Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: hyper::Request<hyper::Body>) -> Self::Future {
        let req = req.map(s3s::Body::from);
        let request = s3s::http::Request::from(req);

        let fut = async {
            let res = self.process(request).await?;
            Ok(res.into())
        };

        Box::pin(fut)
    }
} */

#[derive(Clone)]
struct SharedServer<S: Server>(Arc<S>);

/* impl <T: Send, S: Server + ShareableServer<T>> Service<T> for SharedServer<S> 
where
    Result<<S as ShareableServer<T>>::Response, <S as ShareableServer<T>>::Error>: Send,
{
    type Response = <S as ShareableServer<T>>::Response;

    type Error = <S as ShareableServer<T>>::Error;

    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut std::task::Context<'_>) -> std::task::Poll<std::result::Result<(), Self::Error>> {
        std::task::Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: T) -> Self::Future {
        let svc = self.0.clone();
        Box::pin(svc.call_shared(req))
    }
} */

impl Service<hyper::Request<hyper::Body>> for SharedServer<S3Server> {
    type Response = hyper::Response<hyper::Body>;

    type Error = miette::Report;

    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut std::task::Context<'_>) -> std::task::Poll<std::result::Result<(), Self::Error>> {
        std::task::Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: hyper::Request<hyper::Body>) -> Self::Future {
        let req = req.map(s3s::Body::from);
        let request = s3s::http::Request::from(req);

        let svc = self.0.clone();

        let fut = async move {
            let res = svc.process(request).await?;
            Ok(res.into())
        };

        Box::pin(fut)
    }
}

impl Server for S3Server {
    type Request = s3s::http::Request;

    async fn serve(&self) -> Result<impl Future> {
        let opt = Opt::parse();

        // Setup S3 service
        let service = SharedServer(Arc::new(self.clone()));
        let service = ServiceBuilder::new()
            .service(service);
        let shared = Shared::new(service);

        // Run server
        let listener = TcpListener::bind((opt.host.as_str(), opt.port))
            .map_err(|e| miette::miette!(e))?;
        let server = hyper::Server::from_tcp(listener)
            .map_err(|e| miette::miette!(e))?
            .serve(shared);

        let task = tokio::spawn(server);
        info!("server is running at http://{}:{}/", opt.host, opt.port);

        Ok(task)
    }

    async fn process(&self, request: Self::Request) -> Result<Response> {
        info!("Called S3Server::process");

        let req: Request = request.into(); // TODO: make a RequestBuilder

        let handlers = self.cb.read();

        // Send Request to registered handlers
        let resp = match handlers.len() {
            0 => unimplemented!(), // TODO: better error
            1 => {
                // Optimization if cb.len == 1: send by move
                let fut = handlers.first_key_value().unwrap().1.as_ref().handle(req);
                fut.await?
            }
            _ => {
                // Send Request to all available handlers
                let futs = handlers.iter().map(|(_,c)| c.handle(req.clone()));

                // Resolve handlers in parallel and get the first available response
                // TODO: continue until first successful response
                select_all(futs).await.0.map_err(|e| miette::miette!(e))?
            }
        };

        Ok(resp)
    }

    fn register_handler(&self, handler: Box<dyn Handler<Request>>) -> usize {
        let mut id_lock = self.next_id.lock();
        let id = *id_lock;
        self.cb.write().insert(id, handler);
        *id_lock += 1;
        id
    }

    fn unregister_handler(&self, idx: usize) {
        self.cb.write().remove(&idx);
    }
}