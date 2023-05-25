use std::{pin::Pin, sync::Arc, task::Poll};

use crate::req::{Request, Response, S3Extension};
use aws_credential_types::{provider::SharedCredentialsProvider, Credentials};
use aws_sdk_s3::config::Region;
use futures::Future;
use miette::{miette, Error, Result};
use tower::Service;

use s3s::S3;

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
            let res =
                op.0.call(&client, &mut req)
                    .await
                    .map_err(|err| miette!(err))?;

            Ok(res.into())
        };

        Box::pin(fut)
    }
}

impl Client for S3Client {
    fn send(&self, req: Request) -> impl Future<Output = Result<Response>> + Send {
        let mut client = S3Client(self.0.clone());

        client.call(req)
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
