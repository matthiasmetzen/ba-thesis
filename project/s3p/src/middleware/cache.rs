use std::{
    ops::Deref,
    time::{Duration, Instant},
};

use super::*;
use crate::req::{s3::S3RequestExt, *};

use http::{HeaderValue, StatusCode};
use hyper::body::Bytes;
use miette::{miette, IntoDiagnostic};
use moka::future::Cache;
use moka::Expiry;
use multi_index_map::MultiIndexMap;
use parking_lot::RwLock;
use s3s::ops;
use s3s::ops::Operation;
use s3s::Body;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::broadcast::Sender;
use tokio::task::AbortHandle;
use tracing::{debug, warn};

type Key = String;
type Data = CachedResponse;
type ETag = String;

#[derive(Debug, Clone, Default)]
struct CachedResponse {
    ttl: Option<Duration>,
    tti: Option<Duration>,
    status_code: StatusCode,
    etag: Option<String>,
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

        let etag = resp
            .headers
            .get("ETag")
            .and_then(|header| header.to_str().ok())
            .map(|s| s.to_string());

        Self {
            status_code: resp.status,
            body: bytes,
            etag,
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

#[derive(MultiIndexMap, Clone, Debug)]
pub struct ETagEntry {
    #[multi_index(hashed_unique)]
    key: Key,
    #[multi_index(hashed_unique)]
    etag: ETag,
}

pub struct CacheLayer {
    cache: Arc<Cache<Key, Data>>,
    lut: Arc<RwLock<MultiIndexETagEntryMap>>,
    config: CacheLayerConfig,
    rx_abort: Option<AbortHandle>,
}

impl CacheLayer {
    pub fn new(
        capacity: u64,
        ttl: impl Into<Option<Duration>>,
        tti: impl Into<Option<Duration>>,
    ) -> Self {
        let lut = Arc::new(RwLock::new(MultiIndexETagEntryMap::default()));

        let mut cache = Cache::builder()
            .max_capacity(capacity)
            .weigher(|_k: &Key, v: &CachedResponse| -> u32 {
                v.len().try_into().unwrap_or(u32::MAX)
            })
            .expire_after(PerItemExpiration);

        cache = {
            let lut = lut.clone();

            cache.eviction_listener_with_queued_delivery_mode(move |k, _v, _cause| {
                let mut lut_w = lut.write();
                lut_w.remove_by_key(k.deref());
            })
        };

        if let Some(ttl) = ttl.into() {
            cache = cache.time_to_live(ttl)
        }

        if let Some(tti) = tti.into() {
            cache = cache.time_to_live(tti)
        }

        Self {
            cache: Arc::new(cache.build()),
            lut,
            config: CacheLayerConfig,
            rx_abort: None,
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

        if let Some(etag) = data.etag {
            resp.headers.append(
                "ETag",
                HeaderValue::from_str(etag.as_str()).into_diagnostic()?,
            );
        }

        // TODO: proper headers

        resp.headers.append(
            "cache-control",
            HeaderValue::from_str("no-cache").into_diagnostic()?,
        );

        if let Some(bytes) = data.body {
            resp.body = Body::from(bytes);
        }

        Ok(resp)
    }

    fn event_handler(&self, mut rx: Receiver<Event>) -> impl Future<Output = ()> {
        let cache = self.cache.clone();

        async move {
            loop {
                let event = rx.recv().await;
                match event {
                    Ok(msg) => {
                        debug!("CacheLayer received message: {}", msg);

                        tokio::spawn({
                            let cache = cache.clone();
                            async move {
                                match msg.as_str() {
                                    "delete" => {
                                        let _ = cache.remove("foo").await;
                                    }
                                    _ => {}
                                };
                            }
                        });
                    }
                    Err(RecvError::Lagged(skipped)) => {
                        warn!("Lag while handling events: {} events were skipped", skipped);
                    }
                    Err(RecvError::Closed) => {
                        debug!("Event channel was closed");
                        break;
                    }
                }
            }
        }
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
                .time_to_live(intent.ttl.map(Duration::from_millis))
                .time_to_idle(intent.tti.map(Duration::from_millis));

            let ee = ETagEntry {
                key: key.clone(),
                etag: resp
                    .headers
                    .get("ETag")
                    .map_or_else(|| key.clone(), |h| h.to_str().unwrap().to_string()),
            };
            self.cache.insert(key, cr).await;
            self.lut.write().insert(ee);
        }

        Ok(resp)
    }

    fn subscribe(&mut self, tx: &Sender<Event>) {
        // Abort previously started tasks
        self.unsubscribe();

        let rx = tx.subscribe();

        let handle = tokio::spawn(self.event_handler(rx));

        self.rx_abort = Some(handle.abort_handle());
    }

    fn unsubscribe(&mut self) {
        if let Some(h) = self.rx_abort.as_mut() {
            h.abort()
        }
        self.rx_abort = None;
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
