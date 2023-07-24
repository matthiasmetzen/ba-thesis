use async_broadcast::{Receiver, RecvError};
use futures::Stream;
use http::Request;
use miette::{Context, IntoDiagnostic, Result};

pub mod event_types;
pub mod s3;

use self::event_types::S3WebhookEvent;
pub use self::s3::S3WebhookServer;

pub type Event = WebhookEvent;
pub type BroadcastRecv = async_broadcast::Receiver<Event>;
pub type BroadcastSend = async_broadcast::Sender<Event>;

/// Extension trait for [Receiver]
pub(crate) trait ReceiverExt<T: Clone> {
    /// Generate a stream of received messages. Does not skip error messages
    fn recv_stream(self) -> impl Stream<Item = Result<T, RecvError>>;
}

impl<T: Clone> ReceiverExt<T> for Receiver<T> {
    fn recv_stream(self) -> impl Stream<Item = Result<T, RecvError>> {
        futures::stream::unfold(self, |mut this| async move {
            let res = this.recv().await;

            match res {
                Ok(_) => Some((res, this)),
                Err(RecvError::Overflowed(_)) => Some((res, this)),
                Err(RecvError::Closed) => None,
            }
        })
    }
}

/// A builder for webhooks
pub trait WebhookServerBuilder {
    fn serve(&self, tx: &BroadcastSend) -> Result<impl WebhookServer>;
}

/// Representation of a Webhook component
#[async_trait::async_trait]
pub trait WebhookServer: Send {
    /// stop a running webhook gracefully
    async fn stop(self) -> Result<()>;
}

/// Data reveived by the webhook gets sent as one of these types
// TODO: this is not very extensible
#[derive(Debug)]
#[allow(dead_code)]
pub enum WebhookEvent {
    S3(S3WebhookEvent),
    Http(Request<hyper::body::Bytes>),
    Other(String),
}

impl Clone for WebhookEvent {
    fn clone(&self) -> Self {
        self.to_owned()
    }
}

impl WebhookEvent {
    pub async fn from_request(req: hyper::Request<hyper::Body>) -> Result<Self> {
        let mut req = req;
        // Take bytes from body
        let body = hyper::body::to_bytes(req.body_mut())
            .await
            .into_diagnostic()
            .wrap_err("Error while parsing webhook request")
            .context(format!("{:?}", req))?;

        // Parse as S3 event
        let val = serde_json::from_slice::<S3WebhookEvent>(&body);

        Ok(match val {
            Ok(event) => Self::S3(event),
            Err(_) => Self::Http(req.map(|_| body)),
        })
    }
}
