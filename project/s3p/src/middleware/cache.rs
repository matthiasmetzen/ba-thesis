use super::*;
use crate::request::*;

use http::{HeaderValue, Method, StatusCode};
use hyper::body::Bytes;
use miette::{miette, IntoDiagnostic};
use moka::future::Cache;
use s3s::Body;
use tracing::debug;

type Key = String;
type Data = CachedResponse;

#[derive(Debug, Clone)]
struct CachedResponse {
    status_code: StatusCode,
    body: Option<Bytes>,
}

impl CachedResponse {
    async fn new(resp: &mut Response) -> Self {
        let mut body = std::mem::take(&mut resp.body);
        let bytes = body.store_all_unlimited().await.ok();

        resp.body = match bytes.clone() {
            Some(b) => Body::from(b),
            None => Body::default()
        };

        debug!("resp body: {:#?}", bytes);
        Self {
            status_code: resp.status,
            body: bytes,
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

    pub fn get_cached_response(&self, key: &Key) -> Result<Response> {
        let data = self.cache.get(key).ok_or_else(|| miette!("No cache entry found"))?;

        debug!("found cache entry for {}", key);
        let mut resp = Response::default();
        resp.status = data.status_code;

        // TODO: proper headers
        resp.headers.append(
            "Etag",
            HeaderValue::from_str(key.as_str()).into_diagnostic()?,
        );

        resp.headers.append(
            "cache-control",
            HeaderValue::from_str("no-cache").into_diagnostic()?,
        );

        
        if let Some(bytes) = data.body {
            resp.body = Body::from(bytes);
        }


        Ok(resp)
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

        if let Ok(resp) = self.get_cached_response(&key) {
            return Ok(resp);
        }

        let mut resp = next.call(req).await?;

        // TODO: move and expand this check
        if resp.status == StatusCode::OK {
            self.cache.insert(key, CachedResponse::new(&mut resp).await).await;
        }


        Ok(resp)
    }
}
