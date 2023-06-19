use std::{
    any::Any,
    ops::Deref,
    pin::Pin,
    sync::{Arc, OnceLock},
    task::Poll,
};

use crate::req::{s3::S3RequestExt, Request, Response, S3Extension};
use aws_credential_types::{provider::SharedCredentialsProvider, Credentials};
use aws_sdk_s3::config::Region;
use futures::Future;
use miette::{miette, Error, IntoDiagnostic, Result};
use tower::Service;

use s3s::{
    dto::SplitMetadata,
    ops::{self, OperationType},
    ops::{Operation, TypedOperation},
    S3,
};

use super::Client;

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

    pub fn endpoint_url(self, endpoint_url: impl Into<&'a str>) -> Self {
        let mut this = self;
        this.endpoint_url = Some(endpoint_url.into());
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

pub struct S3Client(Arc<s3s_aws::Proxy>);

#[allow(unused)]
impl S3Client {
    pub fn builder<'a>() -> S3ClientBuilder<'a> {
        S3ClientBuilder::new()
    }

    pub fn from_config(config: aws_sdk_s3::Config) -> Self {
        let client = aws_sdk_s3::Client::from_conf(config);
        Self::from_client(client)
    }

    pub fn from_client(client: aws_sdk_s3::Client) -> Self {
        let proxy = s3s_aws::Proxy::from(client);

        S3Client(Arc::new(proxy))
    }
}

impl Service<Request> for S3Client {
    type Response = Response;

    type Error = Error;

    type Future = Pin<Box<dyn Future<Output = Result<Response, Error>> + Send>>;

    fn poll_ready(
        &mut self,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::result::Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, mut req: Request) -> Self::Future {
        let client = self.0.clone() as Arc<dyn S3>;

        let fut = async move {
            let s3_ext = req
                .extensions
                .get_mut::<S3Extension>()
                .ok_or_else(|| miette!("Could not get S3 Extension from Request"))?;

            let op = s3_ext.op.take().ok_or_else(|| {
                miette!("S3 Extension has no operation").context(format!("{:#?}", s3_ext))
            })?;

            let mut req: s3s::http::Request = req.try_into()?;
            let res = op
                .call(&client, &mut req)
                .await
                .map_err(|err| miette!(err))?;

            Ok(res.into())
        };

        Box::pin(fut)
    }
}

impl<Op: s3s::ops::TypedOperation + s3s::ops::Operation> Service<(Request, &'static Op)>
    for S3Client
where
    Op::Input: for<'a> TryFrom<&'a mut s3s::http::Request>
        + Send
        + Sync
        + s3s::dto::SplitMetadata
        + 'static
        + From<<<Op as TypedOperation>::Input as s3s::dto::SplitMetadata>::Meta>,
    <<Op as TypedOperation>::Input as s3s::dto::SplitMetadata>::Meta: Send + Sync,
    Op::Output: for<'a> TryInto<s3s::http::Response>
        + Send
        + Sync
        + s3s::dto::SplitMetadata
        + 'static
        + From<<<Op as TypedOperation>::Output as s3s::dto::SplitMetadata>::Meta>,
    <<Op as TypedOperation>::Output as s3s::dto::SplitMetadata>::Meta: Send + Sync,
{
    type Response = Response;

    type Error = Error;

    type Future = Pin<Box<dyn Future<Output = Result<Response, Error>> + Send>>;

    fn poll_ready(
        &mut self,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::result::Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, (req, op): (Request, &'static Op)) -> Self::Future {
        let client = self.0.clone() as Arc<dyn S3>;

        let fut = async move {
            let input = req
                .try_get_input::<Op>()
                .ok_or_else(|| miette!("Request missing input data"))?;

            let s3_ext = {
                let ext = req
                    .extensions
                    .get::<S3Extension>()
                    .ok_or_else(|| miette!("Could not get S3 Extension from Request"))?;

                S3Extension::new_from(ext)
            };

            let req = {
                let mut req: s3s::http::Request = req.try_into()?;

                let input: Op::Input = input.deref().clone().into();

                s3s::ops::build_s3_request(input, &mut req)
            };

            let res = s3s::ops::TypedOperation::call(op, &client, req)
                .await
                .map_err(|err| miette!(err))?;

            let (meta, data) = res.output.split_metadata();
            let output = Arc::new(meta.clone());
            s3_ext
                .data
                .set(output as Arc<dyn Any + Send + Sync + 'static>)
                .map_err(|_| miette!("Could not set output data on response extension"))?;

            let mut output: Op::Output = meta.into();
            output.set_data(data);
            let mut resp: s3s::http::Response = output
                .try_into()
                .map_err(|_| miette!("Failed to construct response"))?;

            resp.extensions.insert(s3_ext);

            Ok(resp.into())
        };

        Box::pin(fut)
    }
}

impl Client for S3Client {
    fn send(&self, req: Request) -> impl Future<Output = Result<Response>> + Send {
        let mut client = S3Client(self.0.clone());

        let op = req
            .extensions
            .get::<S3Extension>()
            .map(|ext| ext.op.clone())
            .flatten();

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

impl ToOwned for S3Client {
    type Owned = Self;

    fn to_owned(&self) -> Self::Owned {
        Self(self.0.clone())
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
