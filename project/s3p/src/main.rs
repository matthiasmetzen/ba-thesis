#![allow(incomplete_features)]
#![feature(async_fn_in_trait)]
#![feature(return_position_impl_trait_in_trait)]
#![feature(trait_alias)]
#![feature(associated_type_defaults)]
#![feature(type_alias_impl_trait)]
#![feature(impl_trait_projections)]
#![feature(impl_trait_in_assoc_type)]
#![feature(fn_traits)]
#![feature(type_name_of_val)]
#![feature(once_cell_try)]
#![feature(result_option_inspect)]
#![feature(associated_type_bounds)]

mod cli;
mod client;
mod config;
mod middleware;
mod pipeline;
mod req;
mod server;
mod webhook;

use std::{fs::File, path::Path, time::Duration};

use clap::Parser;
use client::{s3::S3Client, ClientDelegate};
use middleware::{CacheLayer, Chain, DynChain, Identity};
use miette::{IntoDiagnostic, Result, WrapErr};
use pipeline::Pipeline;
use s3s::auth::SimpleAuth;
use server::{S3ServerBuilder, Server, ServerDelegate};
use tracing_subscriber::{filter::LevelFilter, fmt, prelude::*, util::TryInitError, EnvFilter};

#[tokio::main]
async fn main() -> Result<()> {
    let _ = try_init_tracing();

    dotenvy::dotenv()
        .into_diagnostic()
        .wrap_err("Loading environment failed")?;

    let args = cli::CliArgs::parse();

    let config = if let Some(file) = args.config.config_file {
        match (
            file.exists(),
            args.config.generate_if_missing,
            args.config.regenerate,
        ) {
            (_, _, true) => {
                if file.is_file() {
                    std::fs::remove_file(file.as_path())
                        .into_diagnostic()
                        .wrap_err_with(|| format!("Could not delete file {:?}", file))?;
                }
                config::generate(file.as_path())?;
                Some(config::load(file.as_path())?)
            }
            (true, _, _) => Some(config::load(file)?),
            (false, true, _) => {
                config::generate(file.as_path())?;
                Some(config::load(file.as_path())?)
            }
            _ => None,
        }
    } else {
        None
    }
    .unwrap_or_default();

    /*let host = std::env::var("S3_HOST").unwrap_or("localhost".to_string());
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

    let validate_credentials: bool = match std::env::var("S3_VALIDATE_CREDENTIALS") {
        Ok(f) => f
            .parse()
            .into_diagnostic()
            .wrap_err("Failed to parse S3_VALIDATE_CREDENTIALS")
            .with_context(|| format!("Expected bool, got {}", f))?,
        Err(_) => false,
    };*/

    let server = ServerDelegate::from(&config.server);

    /*let middleware = Chain::new(
        CacheLayer::new(4192, Duration::from_secs(10), None),
        Identity,
    );*/
    let middleware = DynChain::from(&config.middlewares);

    let client = ClientDelegate::from(&config.client);

    let p = Pipeline::new(server, middleware, client);
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
        .with(
            EnvFilter::builder()
                .with_default_directive(LevelFilter::INFO.into())
                .from_env_lossy(),
        )
        .try_init()
}
