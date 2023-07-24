use std::{any::Any, pin::Pin, sync::Arc, task::Poll, time::Duration};

use crate::{
    config::S3ClientConfig,
    req::{
        s3::{S3Operation, S3RequestExt},
        Request, Response, S3Extension, SendError,
    },
};
use aws_credential_types::{provider::SharedCredentialsProvider, Credentials};
use aws_sdk_s3::config::{retry::RetryConfig, timeout::TimeoutConfig, Region};
use aws_smithy_async::rt::sleep::default_async_sleep;
use aws_smithy_client::http_connector::ConnectorSettings;
use aws_smithy_client::hyper_ext;
use futures::{Future, TryFutureExt};
use miette::{miette, Diagnostic, Report, Result};
use thiserror::Error as ThisError;
use tower::Service;

use s3s::{dto::SplitMetadata, ops::OperationType, S3};
use tracing::debug;

use super::Client;

/// Error type for [S3Client]
#[derive(ThisError, Diagnostic, Debug)]
#[diagnostic()]
pub enum S3Error {
    #[error("Could not get S3 Extension from Request")]
    MissingExt,
    #[error("S3 Extension has no operation")]
    MissingOp,
    #[error("Could not construct input")]
    InputErr(s3s::S3Error),
    #[error("Could not construct output")]
    OutputErr(s3s::S3Error),
    #[error("Could not construct request")]
    RequestErr(s3s::S3Error),
    #[error(transparent)]
    ResponseErr(#[from] s3s::S3Error),
    #[error("Other")]
    Other(miette::Report),
}

impl From<S3Error> for SendError {
    fn from(err: S3Error) -> SendError {
        match err {
            S3Error::MissingExt | S3Error::MissingOp => SendError::Internal(err.into()),
            S3Error::Other(e) => SendError::Internal(e),
            S3Error::InputErr(ref e) | S3Error::RequestErr(ref e) => {
                SendError::RequestErr(Response::from(e), err.into())
            }
            S3Error::OutputErr(ref e) | S3Error::ResponseErr(ref e) => {
                SendError::ResponseErr(Response::from(e), err.into())
            }
        }
    }
}

/// A builder type to create new [S3Client]s
#[derive(Default)]
pub struct S3ClientBuilder<'a> {
    endpoint_url: Option<&'a str>,
    credentials_provider: Option<SharedCredentialsProvider>,
    region: Option<Region>,
    force_path_style: bool,
    insecure: bool,
    enable_http2: bool,
    connect_timeout: Option<Duration>,
    read_timeout: Option<Duration>,
    operation_timeout: Option<Duration>,
    operation_attempt_timeout: Option<Duration>,
    retry_attempts: u32,

    // Overwrites other settings
    conf: Option<aws_sdk_s3::Config>,
}

#[allow(unused)]
impl<'a> S3ClientBuilder<'a> {
    /// Creates a new [S3ClientBuilder]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the endpoint url for the target S3 server
    pub fn endpoint_url(self, endpoint_url: &'a str) -> Self {
        let mut this = self;
        this.endpoint_url = Some(endpoint_url);
        this
    }

    /// Sets the [SharedCredentialsProvider]
    pub fn credentials_provider(
        self,
        provider: impl Into<Option<SharedCredentialsProvider>>,
    ) -> Self {
        let mut this = self;
        this.credentials_provider = provider.into();
        this
    }

    /// Sets the credentials provider from an access key id and a secret key
    pub fn credentials_from_single(
        self,
        access_key_id: &'a str,
        secret_access_key: &'a str,
    ) -> Self {
        let mut this = self;
        let creds = Credentials::from_keys(access_key_id, secret_access_key, None);

        this.credentials_provider = Some(SharedCredentialsProvider::new(creds));
        this
    }

    /// Sets the S3 server region
    pub fn region(self, region: impl Into<Region>) -> Self {
        let mut this = self;
        this.region = Some(region.into());
        this
    }

    /// Sets the path style used to query the S3 server
    /// disabled (default): https?://<bucket>.<server>/
    /// enabled: https?://<server>/bucket
    pub fn force_path_style(self, force: bool) -> Self {
        let mut this = self;
        this.force_path_style = force;
        this
    }

    /// Allow insecure requests
    pub fn insecure(self, allow: bool) -> Self {
        let mut this = self;
        this.insecure = allow;
        this
    }

    /// Send requests over HTTP/2. This is not supported by most S3 servers
    pub fn http2(self, enable: bool) -> Self {
        let mut this = self;
        this.enable_http2 = enable;
        this
    }

    /// Set a timeout connecting to the S3 server
    pub fn connect_timeout(self, duration: impl Into<Option<Duration>>) -> Self {
        let mut this = self;
        this.connect_timeout = duration.into();
        this
    }

    /// Set a timeout for reading responses
    pub fn read_timeout(self, duration: impl Into<Option<Duration>>) -> Self {
        let mut this = self;
        this.read_timeout = duration.into();
        this
    }

    /// Set a timeout for the entire operation. This includes request, response and retries.
    pub fn operation_timeout(self, duration: impl Into<Option<Duration>>) -> Self {
        let mut this = self;
        this.operation_timeout = duration.into();
        this
    }

    /// Set a timeout for a single operation attempt. This includes request and response, but not retries.
    pub fn operation_attempt_timeout(self, duration: impl Into<Option<Duration>>) -> Self {
        let mut this = self;
        this.operation_attempt_timeout = duration.into();
        this
    }

    /// Sets the maximum amount of retries that can beb performed per operation
    pub fn max_retry_attempts(self, max_retries: u32) -> Self {
        let mut this = self;
        this.retry_attempts = max_retries;
        this
    }

    /// Overwrite the [aws_sdk_s3::Config] generated by this builder
    pub fn config(self, config: impl Into<Option<aws_sdk_s3::Config>>) -> Self {
        let mut this = self;
        this.conf = config.into();
        this
    }

    /// Consume the builder to generate a new [S3Client]
    pub fn build(self) -> Result<S3Client> {
        let config = match self.conf {
            Some(c) => c,
            None => {
                let endpoint_url = self
                    .endpoint_url
                    .ok_or(miette!("Missing required config: {}", "endpoint_url"))?;

                let region = self.region.or(Some(Region::from_static("us-east-1")));

                // Timeout configuration
                let timeout_config = {
                    let mut tbuilder = TimeoutConfig::builder();
                    tbuilder.set_connect_timeout(self.connect_timeout);
                    tbuilder.set_read_timeout(self.read_timeout);
                    tbuilder.set_operation_timeout(self.operation_timeout);
                    tbuilder.set_operation_attempt_timeout(self.operation_attempt_timeout);

                    tbuilder.build()
                };

                let retry_config = RetryConfig::standard().with_max_attempts(self.retry_attempts);

                // Create a new HTTPConnector from settings
                let smithy_connector = {
                    let connector_builder =
                        hyper_rustls::HttpsConnectorBuilder::new().with_native_roots();

                    // HTTPS/insecure
                    let connector_builder = match self.insecure {
                        true => connector_builder.https_or_http(),
                        false => connector_builder.https_only(),
                    };

                    // HTTP protocol version
                    let connector = match self.enable_http2 {
                        true => connector_builder.enable_http1().enable_http2().build(),
                        false => connector_builder.enable_http1().build(),
                    };

                    hyper_ext::Adapter::builder().build(connector)
                };

                let mut conf = aws_sdk_s3::Config::builder()
                    .endpoint_url(endpoint_url)
                    .region(region)
                    .http_connector(smithy_connector);

                // sleep_impl should not be needed according to aws_sdk_s3, but without it we get runtime errors
                conf.set_sleep_impl(default_async_sleep());
                conf.set_timeout_config(Some(timeout_config));
                conf.set_retry_config(Some(retry_config));

                if let Some(provider) = self.credentials_provider {
                    conf = conf.credentials_provider(provider)
                }

                conf = conf.force_path_style(self.force_path_style);

                conf.build()
            }
        };

        Ok(S3Client::from_config(config))
    }
}

pub struct S3Client {
    inner: S3ClientInner,
}

impl From<&S3ClientConfig> for S3Client {
    fn from(value: &S3ClientConfig) -> Self {
        // Fill the builder from ClientConfig
        let mut builder = Self::builder()
            .force_path_style(value.force_path_style)
            .endpoint_url(&value.endpoint_url)
            .http2(value.enable_http2)
            .insecure(value.insecure)
            .connect_timeout(value.connect_timeout.map(Duration::from_millis))
            .read_timeout(value.read_timeout.map(Duration::from_millis))
            .operation_timeout(value.operation_timeout.map(Duration::from_millis))
            .operation_attempt_timeout(value.operation_attempt_timeout.map(Duration::from_millis))
            .max_retry_attempts(value.max_retry_attempts);

        if let Some(creds) = &value.credentials {
            builder = builder.credentials_from_single(&creds.access_key_id, &creds.secret_key);
        }

        // Unwrap should be safe. build() only fail if endpoint_url was not set, which will alwas be set here.
        builder.build().unwrap()
    }
}

#[derive(Clone)]
struct S3ClientInner(Arc<dyn s3s::S3>);

#[allow(unused)]
impl S3Client {
    /// Create a new [S3Client] from a [s3s::S3] implementation
    pub fn new(client: impl s3s::S3) -> Self {
        Self {
            inner: S3ClientInner(Arc::new(client)),
        }
    }

    pub fn builder<'a>() -> S3ClientBuilder<'a> {
        S3ClientBuilder::new()
    }

    /// Creates a new [S3Client] from an [aws_sdk_s3::Config]
    pub fn from_config(config: aws_sdk_s3::Config) -> Self {
        let client = aws_sdk_s3::Client::from_conf(config);
        Self::from_client(client)
    }

    /// Creates a new [S3Client] from an [aws_sdk_s3::Client]
    pub fn from_client(client: aws_sdk_s3::Client) -> Self {
        let proxy = s3s_aws::Proxy::from(client);

        S3Client::new(proxy)
    }
}

/// This service takes a [Request] and forwards it as a ([Request], [OperationType]) pair
impl Service<Request> for S3ClientInner {
    type Response = Response;

    type Error = S3Error;

    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(
        &mut self,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::result::Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, mut req: Request) -> Self::Future {
        let s3 = self.0.clone();

        let fut = async move {
            // get the S3Extension from the Request
            let s3_ext = req
                .extensions
                .get_mut::<S3Extension>()
                .ok_or_else(|| S3Error::MissingExt)?;

            // the Request will be consumed, so we can just take op
            let op = s3_ext.op.take().ok_or_else(|| S3Error::MissingOp)?;

            let mut req: s3s::http::Request = req.try_into().map_err(|e| S3Error::Other(e))?;

            // send the request with a typed op attached
            let res = op
                .call(&s3, &mut req)
                .await
                .map_err(|err| S3Error::ResponseErr(err))?;

            Ok(res.into())
        };

        Box::pin(fut)
    }
}

impl<Op: S3Operation> Service<(Request, &'static Op)> for S3ClientInner {
    type Response = Response;

    type Error = S3Error;

    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(
        &mut self,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::result::Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, (req, op): (Request, &'static Op)) -> Self::Future {
        let s3 = self.0.clone();

        let fut = async move {
            // Create a new S3Extension for the response
            let s3_ext = {
                let ext = req
                    .extensions
                    .get::<S3Extension>()
                    .ok_or_else(|| S3Error::MissingExt)?;

                S3Extension::new_from(ext)
            };

            // Convert into a typed request object
            let req = {
                let mut req: s3s::http::Request = req.try_into().map_err(|e| S3Error::Other(e))?;

                // Do not use input from S3Extension to ensure body data
                let input = Op::Input::try_from(&mut req).map_err(|e| S3Error::InputErr(e))?;

                s3s::ops::build_s3_request(input, &mut req)
            };

            // Send the request
            let res = s3s::ops::TypedOperation::call(op, &s3, req)
                .await
                .map_err(|err| S3Error::ResponseErr(err))?;

            // split into clonable metadata and non-clonable streams
            let (meta, data) = res.output.split_metadata();

            // attach output to the S3Extension for easier access
            let output = Arc::new(meta.clone());
            s3_ext
                .data
                .set(output as Arc<dyn Any + Send + Sync + 'static>)
                .unwrap(); //Not shared, can not fail

            // rebuild response from metadata + streams
            let mut output: Op::Output = meta.into();
            output.set_data(data);
            let mut resp: s3s::http::Response =
                output.try_into().map_err(|e| S3Error::OutputErr(e))?;

            // attach S3Extension to response
            resp.extensions.insert(s3_ext);

            let resp = Response::from(resp);
            debug!("{:#?}", resp);
            Ok(resp)
        };

        Box::pin(fut)
    }
}

impl Service<Request> for S3Client {
    type Response = Response;

    type Error = S3Error;

    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(
        &mut self,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::result::Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request) -> Self::Future {
        let client = &mut self.inner;

        let op = req
            .extensions
            .get::<S3Extension>()
            .and_then(|ext| ext.op.clone());

        // Attempt to call client when no operation is associated
        if op.is_none() {
            return client.call(req);
        }

        // Use enum dispatch to call the inner client with the correct operation
        match op.unwrap() {
            OperationType::AbortMultipartUpload(op) => client.call((req, op)),
            OperationType::CompleteMultipartUpload(op) => client.call((req, op)),
            OperationType::CopyObject(op) => client.call((req, op)),
            OperationType::CreateBucket(op) => client.call((req, op)),
            OperationType::CreateMultipartUpload(op) => client.call((req, op)),
            OperationType::DeleteBucket(op) => client.call((req, op)),
            OperationType::DeleteBucketAnalyticsConfiguration(op) => client.call((req, op)),
            OperationType::DeleteBucketCors(op) => client.call((req, op)),
            OperationType::DeleteBucketEncryption(op) => client.call((req, op)),
            OperationType::DeleteBucketIntelligentTieringConfiguration(op) => {
                client.call((req, op))
            }
            OperationType::DeleteBucketInventoryConfiguration(op) => client.call((req, op)),
            OperationType::DeleteBucketLifecycle(op) => client.call((req, op)),
            OperationType::DeleteBucketMetricsConfiguration(op) => client.call((req, op)),
            OperationType::DeleteBucketOwnershipControls(op) => client.call((req, op)),
            OperationType::DeleteBucketPolicy(op) => client.call((req, op)),
            OperationType::DeleteBucketReplication(op) => client.call((req, op)),
            OperationType::DeleteBucketTagging(op) => client.call((req, op)),
            OperationType::DeleteBucketWebsite(op) => client.call((req, op)),
            OperationType::DeleteObject(op) => client.call((req, op)),
            OperationType::DeleteObjectTagging(op) => client.call((req, op)),
            OperationType::DeleteObjects(op) => client.call((req, op)),
            OperationType::DeletePublicAccessBlock(op) => client.call((req, op)),
            OperationType::GetBucketAccelerateConfiguration(op) => client.call((req, op)),
            OperationType::GetBucketAcl(op) => client.call((req, op)),
            OperationType::GetBucketAnalyticsConfiguration(op) => client.call((req, op)),
            OperationType::GetBucketCors(op) => client.call((req, op)),
            OperationType::GetBucketEncryption(op) => client.call((req, op)),
            OperationType::GetBucketIntelligentTieringConfiguration(op) => client.call((req, op)),
            OperationType::GetBucketInventoryConfiguration(op) => client.call((req, op)),
            OperationType::GetBucketLifecycleConfiguration(op) => client.call((req, op)),
            OperationType::GetBucketLocation(op) => client.call((req, op)),
            OperationType::GetBucketLogging(op) => client.call((req, op)),
            OperationType::GetBucketMetricsConfiguration(op) => client.call((req, op)),
            OperationType::GetBucketNotificationConfiguration(op) => client.call((req, op)),
            OperationType::GetBucketOwnershipControls(op) => client.call((req, op)),
            OperationType::GetBucketPolicy(op) => client.call((req, op)),
            OperationType::GetBucketPolicyStatus(op) => client.call((req, op)),
            OperationType::GetBucketReplication(op) => client.call((req, op)),
            OperationType::GetBucketRequestPayment(op) => client.call((req, op)),
            OperationType::GetBucketTagging(op) => client.call((req, op)),
            OperationType::GetBucketVersioning(op) => client.call((req, op)),
            OperationType::GetBucketWebsite(op) => client.call((req, op)),
            OperationType::GetObject(op) => client.call((req, op)),
            OperationType::GetObjectAcl(op) => client.call((req, op)),
            OperationType::GetObjectAttributes(op) => client.call((req, op)),
            OperationType::GetObjectLegalHold(op) => client.call((req, op)),
            OperationType::GetObjectLockConfiguration(op) => client.call((req, op)),
            OperationType::GetObjectRetention(op) => client.call((req, op)),
            OperationType::GetObjectTagging(op) => client.call((req, op)),
            OperationType::GetObjectTorrent(op) => client.call((req, op)),
            OperationType::GetPublicAccessBlock(op) => client.call((req, op)),
            OperationType::HeadBucket(op) => client.call((req, op)),
            OperationType::HeadObject(op) => client.call((req, op)),
            OperationType::ListBucketAnalyticsConfigurations(op) => client.call((req, op)),
            OperationType::ListBucketIntelligentTieringConfigurations(op) => client.call((req, op)),
            OperationType::ListBucketInventoryConfigurations(op) => client.call((req, op)),
            OperationType::ListBucketMetricsConfigurations(op) => client.call((req, op)),
            OperationType::ListBuckets(op) => client.call((req, op)),
            OperationType::ListMultipartUploads(op) => client.call((req, op)),
            OperationType::ListObjectVersions(op) => client.call((req, op)),
            OperationType::ListObjects(op) => client.call((req, op)),
            OperationType::ListObjectsV2(op) => client.call((req, op)),
            OperationType::ListParts(op) => client.call((req, op)),
            OperationType::PutBucketAccelerateConfiguration(op) => client.call((req, op)),
            OperationType::PutBucketAcl(op) => client.call((req, op)),
            OperationType::PutBucketAnalyticsConfiguration(op) => client.call((req, op)),
            OperationType::PutBucketCors(op) => client.call((req, op)),
            OperationType::PutBucketEncryption(op) => client.call((req, op)),
            OperationType::PutBucketIntelligentTieringConfiguration(op) => client.call((req, op)),
            OperationType::PutBucketInventoryConfiguration(op) => client.call((req, op)),
            OperationType::PutBucketLifecycleConfiguration(op) => client.call((req, op)),
            OperationType::PutBucketLogging(op) => client.call((req, op)),
            OperationType::PutBucketMetricsConfiguration(op) => client.call((req, op)),
            OperationType::PutBucketNotificationConfiguration(op) => client.call((req, op)),
            OperationType::PutBucketOwnershipControls(op) => client.call((req, op)),
            OperationType::PutBucketPolicy(op) => client.call((req, op)),
            OperationType::PutBucketReplication(op) => client.call((req, op)),
            OperationType::PutBucketRequestPayment(op) => client.call((req, op)),
            OperationType::PutBucketTagging(op) => client.call((req, op)),
            OperationType::PutBucketVersioning(op) => client.call((req, op)),
            OperationType::PutBucketWebsite(op) => client.call((req, op)),
            OperationType::PutObject(op) => client.call((req, op)),
            OperationType::PutObjectAcl(op) => client.call((req, op)),
            OperationType::PutObjectLegalHold(op) => client.call((req, op)),
            OperationType::PutObjectLockConfiguration(op) => client.call((req, op)),
            OperationType::PutObjectRetention(op) => client.call((req, op)),
            OperationType::PutObjectTagging(op) => client.call((req, op)),
            OperationType::PutPublicAccessBlock(op) => client.call((req, op)),
            OperationType::RestoreObject(op) => client.call((req, op)),
            OperationType::SelectObjectContent(op) => client.call((req, op)),
            OperationType::UploadPart(op) => client.call((req, op)),
            OperationType::UploadPartCopy(op) => client.call((req, op)),
            OperationType::WriteGetObjectResponse(op) => client.call((req, op)),
            _ => client.call(req),
        }
    }
}

impl Client for S3Client {
    fn send(&self, req: Request) -> impl Future<Output = Result<Response, SendError>> + Send {
        let mut this = S3Client {
            inner: self.inner.clone(),
        };

        this.call(req).map_err(S3Error::into)
    }
}

#[cfg(test)]
mod tests {
    use ctor::ctor;
    use miette::Result;

    use super::*;

    #[ctor]
    fn prepare() {
        let _ = crate::try_init_tracing();
    }

    #[tokio::test]
    async fn build_s3_client() -> Result<()> {
        let _client = S3Client::builder().endpoint_url("localhost:9000").build()?;

        Ok(())
    }
}
