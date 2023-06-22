use std::ops::{Deref, DerefMut};

use http::{Extensions, HeaderMap, HeaderValue, StatusCode};
use hyper::body::Bytes;
use miette::{miette, Report};
use s3s::{
    ops::{Operation, TypedOperation},
    Body,
};

use crate::req::S3Extension;

use super::s3::S3ResponseExt;

#[derive(Debug, Default)]
pub struct Response {
    pub status: StatusCode,
    pub headers: HeaderMap<HeaderValue>,
    pub body: Body,
    pub extensions: Extensions,
}

#[derive(Debug, Default, Clone)]
pub struct RawResponse {
    pub status: StatusCode,
    pub headers: HeaderMap<HeaderValue>,
    pub bytes: Option<Bytes>,
}

impl From<Report> for Response {
    fn from(_value: Report) -> Self {
        todo!()
    }
}

impl From<Response> for hyper::Response<hyper::Body> {
    fn from(value: Response) -> Self {
        // FIXME: temporary
        let mut res_builder = hyper::Response::builder().status(value.status);

        res_builder.headers_mut().unwrap().extend(value.headers);

        res_builder.body(value.body.into()).unwrap()
    }
}

impl From<s3s::http::Response> for Response {
    fn from(value: s3s::http::Response) -> Self {
        Response {
            status: value.status,
            headers: value.headers,
            body: value.body,
            extensions: value.extensions,
        }
    }
}

impl From<hyper::Request<hyper::Body>> for Response {
    fn from(_value: hyper::Request<hyper::Body>) -> Self {
        todo!()
    }
}

impl From<Response> for RawResponse {
    fn from(value: Response) -> Self {
        Self {
            status: value.status,
            headers: value.headers,
            bytes: value.body.bytes(),
        }
    }
}

impl From<RawResponse> for Response {
    fn from(value: RawResponse) -> Self {
        Self {
            status: value.status,
            headers: value.headers,
            body: match value.bytes {
                Some(b) => Body::from(b),
                None => Body::default(),
            },
            extensions: Extensions::new(),
        }
    }
}
