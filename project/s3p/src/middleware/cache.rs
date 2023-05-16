use super::*;
use crate::request::*;

use http::{HeaderName, HeaderValue, StatusCode};
use hyper::body::{Bytes, HttpBody};
use miette::IntoDiagnostic;
use moka::future::Cache;
use s3s::Body;

type Key = String;
type Data = CachedResponse;

#[derive(Debug, Clone)]
struct CachedResponse {
    status_code: StatusCode,
    body: Option<Bytes>,
}

impl CachedResponse {
    fn new(resp: &Response) -> Self {
        Self {
            status_code: resp.status,
            body: resp.body.bytes(),
        }
    }

    fn len(&self) -> usize {
        // +8 for size of status code + padding
        self.body.as_ref().map_or(0, |b| b.len()) + 8
    }
}

pub struct CacheLayer {
    cache: Cache<Key, Data>,
}

impl CacheLayer {
    fn new(capacity: u64) -> Self {
        Self {
            cache: Cache::builder()
                .max_capacity(capacity)
                .weigher(|_k: &Key, v: &CachedResponse| -> u32 {
                    v.len().try_into().unwrap_or(u32::MAX)
                })
                .build(),
        }
    }

    fn make_key(&self, request: &Request) -> Key {
        todo!()
    }

    fn is_cachable(&self, request: &Request) -> bool {
        todo!()
    }
}

#[async_trait::async_trait]
impl Layer for CacheLayer {
    async fn process_request(&self, request: Request) -> Result<MiddlewareAction> {
        let key = self.make_key(&request);

        match self.cache.get(&key) {
            Some(data) => {
                let mut resp = Response::default();
                resp.status = data.status_code;

                // TODO: proper headers
                resp.headers.append(
                    "cache-control-key",
                    HeaderValue::from_str(key.as_str()).into_diagnostic()?,
                );

                if let Some(bytes) = data.body {
                    resp.body = Body::from(bytes);
                }

                Ok(MiddlewareAction::Reply(resp))
            }
            _ => Ok(MiddlewareAction::Forward(request)),
        }
    }

    async fn process_response(&self, response: Response) -> Result<Response> {
        // TODO: how to get key here?

        let key: Key = todo!();
        let is_cachable: bool = todo!();

        if is_cachable {
            self.cache.insert(key, CachedResponse::new(&response));
        }

        Ok(response)
    }
}
