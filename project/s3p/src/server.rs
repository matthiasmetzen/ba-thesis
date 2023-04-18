use futures::{SinkExt, Sink};
use futures::channel::mpsc::{Sender, SendError};
use futures::future::BoxFuture;
use hyper::service::Service;
use miette::Result;
use tower::ServiceBuilder;
use tower::make::Shared;

use crate::request::{Request, Response};

use std::net::TcpListener;
use std::sync::Arc;
use std::future::Future;

use clap::Parser;
use tracing::info;

type ResponseChannel = futures::channel::oneshot::Receiver<Response>;

pub trait Server {
    type Request;
    type ChannelSender: SrvSender;

    async fn serve(&self) -> Result<impl Future>;
    async fn process(&self, request: Self::Request) -> Result<ResponseChannel>;
}

type ReqSender = futures::channel::mpsc::UnboundedSender<Request>;
type ReqReceiver = futures::channel::mpsc::UnboundedReceiver<Request>;

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
#[derive(Clone)]
struct S3Server<C: SrvSender> {
    tx: C
}

impl<C: SrvSender> S3Server<C> {
    fn new(tx: C) -> Self {
        Self {
            tx
        }
    }
}

impl<C: SrvSender> Service<hyper::Request<hyper::Body>> for S3Server<C> {
    type Response = hyper::Response<hyper::Body>;

    type Error = miette::Report;

    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut std::task::Context<'_>) -> std::task::Poll<std::result::Result<(), Self::Error>> {
        std::task::Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: hyper::Request<hyper::Body>) -> Self::Future {
        let req = req.map(s3s::Body::from);
        let request = s3s::http::Request::from(req);

        let fut = async move {
            let rx = self.process(request).await?;
            let res: Response = rx.await.map_err(|e| miette::miette!(e))?;
            Ok::<Self::Response, Self::Error>(res.into())
        };

        Box::pin(fut)
    }
}

#[derive(Clone)]
struct SharedServer<S: Server>(Arc<S>);

impl <T, S: Server + Service<T>> Service<T> for SharedServer<S> {
    type Response = <S as Service<T>>::Response;

    type Error = <S as Service<T>>::Error;

    type Future = <S as Service<T>>::Future;

    fn poll_ready(&mut self, cx: &mut std::task::Context<'_>) -> std::task::Poll<std::result::Result<(), Self::Error>> {
        self.0.poll_ready(cx)
    }

    fn call(&mut self, req: T) -> Self::Future {
        self.0.call(req)
    }
}

impl<C: SrvSender> Server for S3Server<C> {
    type Request = s3s::http::Request;

    type ChannelSender = C;

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

    async fn process(&self, request: Self::Request) -> Result<ResponseChannel> {
        info!("Called S3Server::process");

        let (tx, rx) = futures::channel::oneshot::channel();
        let mut req: Request = request.into(); // TODO: make a RequestBuilder
        req.tx = tx;
        self.tx.clone().send(req).await.map_err(|e| miette::miette!(e))?;

        Ok(rx)
    }
}