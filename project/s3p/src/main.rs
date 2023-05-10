#![feature(async_fn_in_trait)]
#![feature(return_position_impl_trait_in_trait)]
#![feature(trait_alias)]
#![feature(associated_type_defaults)]
#![feature(type_alias_impl_trait)]
#![feature(impl_trait_projections)]
#![feature(impl_trait_in_assoc_type)]
#![feature(fn_traits)]

use client::S3Client;
use middleware::{Identity, Stack};
use miette::{IntoDiagnostic, Result, WrapErr};
use pipeline::Pipeline;
use s3s::auth::SimpleAuth;
use server::{S3ServerBuilder, Server};
use tracing_subscriber::{fmt, prelude::*, util::TryInitError, EnvFilter};

mod client;
mod middleware;
mod pipeline;
mod request;
mod server;

#[tokio::main]
async fn main() -> Result<()> {
    let _ = try_init_tracing();

    dotenvy::dotenv()
        .into_diagnostic()
        .wrap_err("Loading environment failed")?;

    let host = std::env::var("S3_HOST").unwrap_or("localhost".to_string());
    let port: u16 = match std::env::var("S3_PORT") {
        Ok(p) => p.parse().into_diagnostic().wrap_err("invalid port")?,
        Err(_) => 4356,
    };

    let endpoint_url = std::env::var("S3_ENDPOINT_URL")
        .into_diagnostic()
        .wrap_err("Missing S3 endpoint url")?;

    let base_domain = std::env::var("S3_BASE_DOMAIN").ok();

    let access_key_id = std::env::var("S3_ACCESS_KEY")
        .into_diagnostic()
        .wrap_err("missing access key")?;

    let secret_access_key = std::env::var("S3_SECRET_KEY")
        .into_diagnostic()
        .wrap_err("missing secret key")?;

    let force_path_style: bool = match std::env::var("S3_FORCE_PATH_STYLE") {
        Ok(f) => f
            .parse()
            .into_diagnostic()
            .wrap_err("Failed to parse S3_FORCE_PATH_STYLE")
            .with_context(|| format!("Expected bool, got {}", f))?,
        Err(_) => false,
    };

    let s3 = S3ServerBuilder::new(host, port)
        .auth(Some(SimpleAuth::from_single(
            access_key_id.as_str(),
            secret_access_key.as_str(),
        )))
        .base_domain(base_domain);

    let middleware = Stack::new(Identity, Identity);
    let client = S3Client::builder()
        .endpoint_url(endpoint_url.as_str())
        .credentials_from_single(access_key_id.as_str(), secret_access_key.as_str())
        .force_path_style(force_path_style)
        .build()?;
    let p = Pipeline::new(s3, middleware, client);
    let server = p.run().await?;

    tokio::signal::ctrl_c()
        .await
        .map_err(|e| miette::miette!(e))?; // FIXME: Temporary
    server.stop().await?;

    Ok(())
}

fn try_init_tracing() -> Result<(), TryInitError> {
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env())
        .try_init()
}
