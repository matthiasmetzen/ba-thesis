#![cfg_attr(docsrs, feature(doc_cfg))]
#![forbid(unsafe_code)]
#![deny(
    clippy::all, //
    clippy::cargo, //
    clippy::must_use_candidate, //
    clippy::self_named_module_files, //
)]
#![warn(
    clippy::dbg_macro, //
)]
#![allow(
    clippy::bool_assert_comparison,  // I don't like `assert!(!expression)`. It's very misleading.
    clippy::multiple_crate_versions, // Sometimes not fixable
)]

#[macro_use]
mod utils;

#[macro_use]
mod error;

mod header;
pub mod http;
pub mod ops;
mod request;
mod s3_trait;
mod sig_v2;
mod sig_v4;
mod xml;

pub mod auth;
pub mod dto;
pub mod path;
pub mod service;
pub mod stream;

pub use self::error::*;
pub use self::http::Body;
pub use self::request::S3Request;
pub use self::s3_trait::S3;
