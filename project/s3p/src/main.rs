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

use clap::Parser;
use client::ClientDelegate;
use middleware::DynChain;
use miette::{IntoDiagnostic, Result, WrapErr};
use pipeline::Pipeline;

use server::{Server, ServerDelegate};
use tracing_subscriber::{filter::LevelFilter, fmt, prelude::*, util::TryInitError, EnvFilter};

#[tokio::main]
async fn main() -> Result<()> {
    let _ = try_init_tracing();

    let _ = dotenvy::dotenv();

    let args = cli::CliArgs::parse();

    // read config from file if present
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

    // Construct a Server from config
    let server = ServerDelegate::from(&config.server);

    // Construct a Middleware Stack from config
    let middleware = DynChain::from(&config.middlewares);

    // Construct a Client from config
    let client = ClientDelegate::from(&config.client);

    //Construct the pipeline
    let p = Pipeline::new(server, middleware, client);
    let server = p.run().await?;

    // Wait for Ctrl+C for graceful shutdown
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
