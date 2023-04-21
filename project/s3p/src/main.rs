#![feature(async_fn_in_trait)]
#![feature(return_position_impl_trait_in_trait)]
#![feature(trait_alias)]
#![feature(associated_type_defaults)]

use tracing_subscriber::{EnvFilter, fmt, prelude::*};
use tracing::info;


mod server;
mod client;
mod middleware;
mod request;
mod pipeline;

#[tokio::main]
async fn main() {
    init_tracing();
}

fn init_tracing() {
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env())
        .init();
}