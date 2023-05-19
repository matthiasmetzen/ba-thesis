use super::*;
use crate::request::*;

use http::{HeaderName, HeaderValue, Method, StatusCode};
use hyper::body::{Bytes, HttpBody};
use miette::{miette, IntoDiagnostic};
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
    pub fn new(capacity: u64) -> Self {
        Self {
            cache: Cache::builder()
                .max_capacity(capacity)
                .weigher(|_k: &Key, v: &CachedResponse| -> u32 {
                    v.len().try_into().unwrap_or(u32::MAX)
                })
                .build(),
        }
    }

    fn make_key(&self, request: &Request) -> Option<Key> {
        let http_ext = request.extensions.get::<HttpExtension>()?;
        http_ext.uri.path().to_string().into()
    }

    fn is_cachable(&self, request: &Request) -> bool {
        let Some(http_ext) = request
            .extensions
            .get::<HttpExtension>() 
            else {
                return false;
            };
        http_ext.method == Method::GET
    }
}

#[async_trait::async_trait]
impl Layer for CacheLayer {
    async fn call(&self, req: Request, next: impl NextLayer) -> Result<Response> {
        if !self.is_cachable(&req) {
            return next.call(req).await;
        }

        let key = self.make_key(&req).ok_or_else(|| {
            miette!("Response marked as cacheable but key was None").context(format!("{:#?}", req))
        })?;

        if let Some(data) = self.cache.get(&key) {
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

            return Ok(resp);
        }

        let resp = next.call(req).await?;

        self.cache.insert(key, CachedResponse::new(&resp)).await;

        Ok(resp)
    }
}
