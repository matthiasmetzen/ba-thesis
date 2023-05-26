use std::time::{Duration, Instant};

use super::*;
use crate::req::{s3::S3RequestExt, *};

use http::{HeaderValue, StatusCode};
use hyper::body::Bytes;
use miette::{miette, IntoDiagnostic};
use moka::future::Cache;
use moka::Expiry;
use s3s::ops;
use s3s::ops::Operation;
use s3s::Body;
use tracing::debug;

type Key = String;
type Data = CachedResponse;

#[derive(Debug, Clone, Default)]
struct CachedResponse {
    ttl: Option<Duration>,
    tti: Option<Duration>,
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
            ..Default::default()
        }
    }

    fn time_to_live(self, ttl: impl Into<Option<Duration>>) -> Self {
        let mut this = self;
        this.ttl = ttl.into();
        this
    }

    fn time_to_idle(self, tti: impl Into<Option<Duration>>) -> Self {
        let mut this = self;
        this.tti = tti.into();
        this
    }

    fn len(&self) -> usize {
        // +8 for size of status code + padding
        self.body.as_ref().map_or(0, |b| b.len()) + 8
    }
}

pub struct PerItemExpiration;
impl Expiry<Key, Data> for PerItemExpiration {
    fn expire_after_create(
        &self,
        _key: &Key,
        value: &Data,
        _current_time: Instant,
    ) -> Option<Duration> {
        value.ttl
    }

    fn expire_after_read(
        &self,
        _key: &Key,
        value: &Data,
        _current_time: Instant,
        // The duration until this entry expires.
        current_duration: Option<Duration>,
        // The time when this entry was modified (inserted or replaced).
        _last_modified_at: Instant,
    ) -> Option<Duration> {
        value.tti.or(current_duration)
    }

    fn expire_after_update(
        &self,
        _key: &Key,
        value: &Data,
        _current_time: Instant,
        // The duration until this entry expires.
        current_duration: Option<Duration>,
    ) -> Option<Duration> {
        value.ttl.or(current_duration)
    }
}

pub struct CacheLayerConfig;

pub struct CacheLayer {
    cache: Cache<Key, Data>,
    config: CacheLayerConfig,
}

impl CacheLayer {
    pub fn new(
        capacity: u64,
        ttl: impl Into<Option<Duration>>,
        tti: impl Into<Option<Duration>>,
    ) -> Self {
        let mut cache = Cache::builder()
            .max_capacity(capacity)
            .weigher(|_k: &Key, v: &CachedResponse| -> u32 {
                v.len().try_into().unwrap_or(u32::MAX)
            })
            .expire_after(PerItemExpiration);

        if let Some(ttl) = ttl.into() {
            cache = cache.time_to_live(ttl)
        }

        if let Some(tti) = tti.into() {
            cache = cache.time_to_live(tti)
        }

        Self {
            cache: cache.build(),
            config: CacheLayerConfig,
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
    fn make_cache_intent(
        &self,
        request: &Request,
        config: &CacheLayerConfig,
    ) -> Option<CacheIntent> {
        if let Some(s3ext) = request.extensions.get::<S3Extension>() {
            return s3ext.make_cache_intent(request, config);
        }

        None
    }
}

#[async_trait::async_trait]
impl Layer for CacheLayer {
    async fn call(&self, req: Request, next: impl NextLayer) -> Result<Response> {
        let Some(intent) = self.make_cache_intent(&req, &self.config) else {
            return next.call(req).await;
        };

        let key = intent.key;

        if let Ok(resp) = self.get_cached_response(&key) {
            return Ok(resp);
        }

        let mut resp = next.call(req).await?;

        // TODO: move and expand this check
        if resp.status == StatusCode::OK {
            let cr = CachedResponse::new(&mut resp)
                .await
                .time_to_live(intent.ttl.map(|t| Duration::from_millis(t)))
                .time_to_idle(intent.tti.map(|t| Duration::from_millis(t)));

            self.cache.insert(key, cr).await;
        }

        Ok(resp)
    }
}

#[derive(Default)]
pub struct CacheIntent {
    pub key: Key,
    pub ttl: Option<u64>,
    pub tti: Option<u64>,
}

#[allow(unused)]
impl CacheIntent {
    pub fn new(key: Key) -> Self {
        Self {
            key,
            ..Default::default()
        }
    }

    pub fn time_to_live(self, ttl: impl Into<Option<u64>>) -> Self {
        let mut this = self;
        this.ttl = ttl.into();
        this
    }

    pub fn time_to_idle(self, tti: impl Into<Option<u64>>) -> Self {
        let mut this = self;
        this.tti = tti.into();
        this
    }
}

pub trait CacheLogic {
    fn make_cache_intent(
        &self,
        request: &Request,
        config: &CacheLayerConfig,
    ) -> Option<CacheIntent>;
}

impl CacheLogic for S3Extension {
    fn make_cache_intent(
        &self,
        request: &Request,
        config: &CacheLayerConfig,
    ) -> Option<CacheIntent> {
        let op = self.op.as_ref()?;

        if let Ok(op) = op.try_as_ref::<ops::GetObject>() {
            return op.make_cache_intent(request, config);
        }

        if let Ok(op) = op.try_as_ref::<ops::HeadBucket>() {
            return op.make_cache_intent(request, config);
        }

        if let Ok(op) = op.try_as_ref::<ops::ListBuckets>() {
            return op.make_cache_intent(request, config);
        }

        if let Ok(op) = op.try_as_ref::<ops::ListObjects>() {
            return op.make_cache_intent(request, config);
        }

        if let Ok(op) = op.try_as_ref::<ops::ListObjectsV2>() {
            return op.make_cache_intent(request, config);
        }

        if let Ok(op) = op.try_as_ref::<ops::ListObjectVersions>() {
            return op.make_cache_intent(request, config);
        }

        None
    }
}

impl CacheLogic for ops::GetObject {
    fn make_cache_intent(
        &self,
        request: &Request,
        _config: &CacheLayerConfig,
    ) -> Option<CacheIntent> {
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

        Some(CacheIntent::new(key))
    }
}

impl CacheLogic for ops::HeadBucket {
    fn make_cache_intent(
        &self,
        request: &Request,
        _config: &CacheLayerConfig,
    ) -> Option<CacheIntent> {
        let mut req: s3s::http::Request = request.try_as_s3_request().ok()?;
        let des = Self::deserialize_http(&mut req).ok()?;

        if des.expected_bucket_owner.is_some() {
            return None;
        }

        let key = format!("op={}, {}", self.name(), des.bucket);

        Some(CacheIntent::new(key))
    }
}

impl CacheLogic for ops::HeadObject {
    fn make_cache_intent(
        &self,
        request: &Request,
        _config: &CacheLayerConfig,
    ) -> Option<CacheIntent> {
        let mut req: s3s::http::Request = request.try_as_s3_request().ok()?;
        let des = Self::deserialize_http(&mut req).ok()?;

        if des.expected_bucket_owner.is_some() || des.range.is_some() {
            return None;
        }

        let key = format!(
            "op={}, {}, {}",
            self.name(),
            des.bucket,
            des.version_id.unwrap_or_default()
        );

        Some(CacheIntent::new(key))
    }
}

impl CacheLogic for ops::ListBuckets {
    fn make_cache_intent(
        &self,
        _request: &Request,
        _config: &CacheLayerConfig,
    ) -> Option<CacheIntent> {
        let key = format!("op={}", self.name());

        Some(CacheIntent::new(key))
    }
}

impl CacheLogic for ops::ListObjects {
    fn make_cache_intent(
        &self,
        request: &Request,
        _config: &CacheLayerConfig,
    ) -> Option<CacheIntent> {
        let mut req: s3s::http::Request = request.try_as_s3_request().ok()?;
        let des = Self::deserialize_http(&mut req).ok()?;

        if des.expected_bucket_owner.is_some() {
            return None;
        }

        let key = format!("op={}, {}", self.name(), des.bucket);

        Some(CacheIntent::new(key))
    }
}

impl CacheLogic for ops::ListObjectsV2 {
    fn make_cache_intent(
        &self,
        request: &Request,
        _config: &CacheLayerConfig,
    ) -> Option<CacheIntent> {
        let mut req: s3s::http::Request = request.try_as_s3_request().ok()?;
        let des = Self::deserialize_http(&mut req).ok()?;

        if des.expected_bucket_owner.is_some()
            || des.max_keys.is_some()
            || des.start_after.is_some()
        {
            return None;
        }

        let key = format!(
            "op={}, {}, {}",
            self.name(),
            des.bucket,
            des.prefix.unwrap_or_default()
        );

        Some(CacheIntent::new(key))
    }
}

impl CacheLogic for ops::ListObjectVersions {
    fn make_cache_intent(
        &self,
        request: &Request,
        _config: &CacheLayerConfig,
    ) -> Option<CacheIntent> {
        let mut req: s3s::http::Request = request.try_as_s3_request().ok()?;
        let des = Self::deserialize_http(&mut req).ok()?;

        if des.expected_bucket_owner.is_some() || des.key_marker.is_some() || des.max_keys.is_some()
        {
            return None;
        }

        let key = format!(
            "op={}, {}, {}, {}",
            self.name(),
            des.bucket,
            des.prefix.unwrap_or_default(),
            des.delimiter.unwrap_or_default()
        );

        Some(CacheIntent::new(key))
    }
}
