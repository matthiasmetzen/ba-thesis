#![feature(async_fn_in_trait)]
#![feature(return_position_impl_trait_in_trait)]
#![feature(trait_alias)]
#![feature(associated_type_defaults)]
#![feature(type_alias_impl_trait)]
#![feature(impl_trait_projections)]
#![feature(impl_trait_in_assoc_type)]
#![feature(fn_traits)]

use tracing_subscriber::{EnvFilter, fmt, prelude::*};



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