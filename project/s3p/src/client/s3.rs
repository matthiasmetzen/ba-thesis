use std::{any::Any, pin::Pin, sync::Arc, task::Poll};

use crate::{
    config::S3ClientConfig,
    req::{
        s3::{S3Operation, S3RequestExt},
        Request, Response, S3Extension, SendError,
    },
};
use aws_credential_types::{provider::SharedCredentialsProvider, Credentials};
use aws_sdk_s3::config::Region;
use futures::{Future, TryFutureExt};
use miette::{miette, Diagnostic, Report, Result};
use thiserror::Error as ThisError;
use tower::Service;

use s3s::{dto::SplitMetadata, ops::OperationType, S3};
use tracing::debug;

use super::Client;

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

#[derive(Default)]
pub struct S3ClientBuilder<'a> {
    endpoint_url: Option<&'a str>,
    credentials_provider: Option<SharedCredentialsProvider>,
    region: Option<Region>,
    force_path_style: bool,

    // Overwrites other settings
    conf: Option<aws_sdk_s3::Config>,
}

#[allow(unused)]
impl<'a> S3ClientBuilder<'a> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn endpoint_url(self, endpoint_url: &'a str) -> Self {
        let mut this = self;
        this.endpoint_url = Some(endpoint_url);
        this
    }

    pub fn credentials_provider(
        self,
        provider: impl Into<Option<SharedCredentialsProvider>>,
    ) -> Self {
        let mut this = self;
        this.credentials_provider = provider.into();
        this
    }

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

    pub fn region(self, region: impl Into<Region>) -> Self {
        let mut this = self;
        this.region = Some(region.into());
        this
    }

    pub fn force_path_style(self, force: bool) -> Self {
        let mut this = self;
        this.force_path_style = force;
        this
    }

    pub fn config(self, config: impl Into<Option<aws_sdk_s3::Config>>) -> Self {
        let mut this = self;
        this.conf = config.into();
        this
    }

    pub fn build(self) -> Result<S3Client> {
        let config = match self.conf {
            Some(c) => c,
            None => {
                let endpoint_url = self
                    .endpoint_url
                    .ok_or(miette!("Missing required config: {}", "endpoint_url"))?;

                let region = self.region.or(Some(Region::from_static("us-east-1")));

                let mut conf = aws_sdk_s3::Config::builder()
                    .endpoint_url(endpoint_url)
                    .region(region);

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

impl From<&S3ClientConfig> for S3Client {
    fn from(value: &S3ClientConfig) -> Self {
        let mut builder = Self::builder()
            .force_path_style(value.force_path_style)
            .endpoint_url(&value.endpoint_url);

        if let Some(creds) = &value.credentials {
            builder = builder.credentials_from_single(&creds.access_key_id, &creds.secret_key);
        }

        // Unwrap should be safe. build() only fail if endpoint_url was not set, which will alwas be set here.
        builder.build().unwrap()
    }
}

pub struct S3Client {
    inner: S3ClientInner,
}

#[derive(Clone)]
pub struct S3ClientInner(Arc<dyn s3s::S3>);

#[allow(unused)]
impl S3Client {
    pub fn new(client: impl s3s::S3) -> Self {
        Self {
            inner: S3ClientInner(Arc::new(client)),
        }
    }

    pub fn builder<'a>() -> S3ClientBuilder<'a> {
        S3ClientBuilder::new()
    }

    pub fn from_config(config: aws_sdk_s3::Config) -> Self {
        let client = aws_sdk_s3::Client::from_conf(config);
        Self::from_client(client)
    }

    pub fn from_client(client: aws_sdk_s3::Client) -> Self {
        let proxy = s3s_aws::Proxy::from(client);

        S3Client::new(proxy)
    }
}

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
            let s3_ext = req
                .extensions
                .get_mut::<S3Extension>()
                .ok_or_else(|| S3Error::MissingExt)?;

            let op = s3_ext.op.take().ok_or_else(|| S3Error::MissingOp)?;

            let mut req: s3s::http::Request = req.try_into().map_err(|e| S3Error::Other(e))?;
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
            /*let input = req
            .try_get_input::<Op>()
            .ok_or_else(|| miette!("Request missing input data"))?;*/

            let s3_ext = {
                let ext = req
                    .extensions
                    .get::<S3Extension>()
                    .ok_or_else(|| S3Error::MissingExt)?;

                S3Extension::new_from(ext)
            };

            let req = {
                let mut req: s3s::http::Request = req.try_into().map_err(|e| S3Error::Other(e))?;

                // Do not use input from S3Extension to ensure body data
                let input = Op::Input::try_from(&mut req).map_err(|e| S3Error::InputErr(e))?;

                s3s::ops::build_s3_request(input, &mut req)
            };

            let res = s3s::ops::TypedOperation::call(op, &s3, req)
                .await
                .map_err(|err| S3Error::ResponseErr(err))?;

            let (meta, data) = res.output.split_metadata();
            let output = Arc::new(meta.clone());
            s3_ext
                .data
                .set(output as Arc<dyn Any + Send + Sync + 'static>)
                .unwrap(); //Not shared, can not fail

            let mut output: Op::Output = meta.into();
            output.set_data(data);
            let mut resp: s3s::http::Response =
                output.try_into().map_err(|e| S3Error::OutputErr(e))?;

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

        if op.is_none() {
            return client.call(req);
        }

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
