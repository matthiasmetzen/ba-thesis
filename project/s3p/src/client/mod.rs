use crate::request::{Request, Response};
use futures::Future;
use miette::{ErrReport, Result};
use tower::Service;

pub mod s3;

pub trait Client: Send + Sync {
    fn send(&self, request: Request) -> impl Future<Output = Result<Response>> + Send;
}

/* pub trait Client:
    Service<
    Request,
    Response = Response,
    Error = Error,
    Future = Pin<Box<dyn Future<Output = Result<Response, Error>> + Send>>,
>
{
} */

pub trait ToOwnedClient {
    fn to_owned_client(&self) -> Self;
}

impl<T> Client for T
where
    T: Service<Request, Response = Response, Error = ErrReport> + Send + Sync,
    T::Future: Future<Output = Result<T::Response, T::Error>> + Send,
    Self: ToOwnedClient,
{
    fn send(&self, request: Request) -> impl Future<Output = Result<Response>> {
        let mut client = self.to_owned_client();
        client.call(request)
    }
}
