use super::*;
use crate::req::{s3::S3RequestExt, *};

use http::{HeaderValue, StatusCode};
use hyper::body::Bytes;
use miette::{miette, IntoDiagnostic};
use moka::future::Cache;
use s3s::ops;
use s3s::ops::Operation;
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
            None => Body::default(),
        };

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

    pub fn get_cached_response(&self, key: &Key) -> Result<Response> {
        let data = self
            .cache
            .get(key)
            .ok_or_else(|| miette!("No cache entry found"))?;

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

impl CacheLogic for CacheLayer {
    fn make_cache_intent(&self, request: &Request) -> Option<CacheIntent> {
        if let Some(s3ext) = request.extensions.get::<S3Extension>() {
            return s3ext.make_cache_intent(request);
        }

        None
    }
}

#[async_trait::async_trait]
impl Layer for CacheLayer {
    async fn call(&self, req: Request, next: impl NextLayer) -> Result<Response> {
        let Some(intent) = self.make_cache_intent(&req) else {
            return next.call(req).await;
        };

        let key = intent.key;

        if let Ok(resp) = self.get_cached_response(&key) {
            return Ok(resp);
        }

        let mut resp = next.call(req).await?;

        // TODO: move and expand this check
        if resp.status == StatusCode::OK {
            self.cache
                .insert(key, CachedResponse::new(&mut resp).await)
                .await;
        }

        Ok(resp)
    }
}

pub struct CacheIntent {
    pub key: Key,
}

pub trait CacheLogic {
    fn make_cache_intent(&self, request: &Request) -> Option<CacheIntent>;
}

impl CacheLogic for ops::GetObject {
    fn make_cache_intent(&self, request: &Request) -> Option<CacheIntent> {
        let mut req: s3s::http::Request = request.try_as_s3_request().ok()?;
        let des = Self::deserialize_http(&mut req).ok()?;

        if des.range.is_some() || des.part_number.is_some() {
            return None;
        }

        let key = format!(
            "op={}, {}:{}:{}",
            self.name(),
            des.bucket,
            des.key,
            des.version_id.unwrap_or("".to_string())
        );

        Some(CacheIntent { key })
    }
}

impl CacheLogic for ops::HeadBucket {
    fn make_cache_intent(&self, request: &Request) -> Option<CacheIntent> {
        let mut req: s3s::http::Request = request.try_as_s3_request().ok()?;
        let des = Self::deserialize_http(&mut req).ok()?;

        if des.expected_bucket_owner.is_some() {
            return None;
        }

        let key = format!("op={}, {}", self.name(), des.bucket);

        Some(CacheIntent { key })
    }
}

impl CacheLogic for S3Extension {
    fn make_cache_intent(&self, request: &Request) -> Option<CacheIntent> {
        let op = self.op.as_ref()?;

        if let Ok(op) = op.try_as_ref::<ops::GetObject>() {
            return op.make_cache_intent(request);
        }

        if let Ok(op) = op.try_as_ref::<ops::HeadBucket>() {
            return op.make_cache_intent(request);
        }

        None
    }
}
