use std::{
    ops::Deref,
    time::{Duration, Instant},
};

use super::*;
use crate::{
    req::{s3::S3RequestExt, s3::S3ResponseExt, *},
    webhook::{
        event_types::{LifecycleExpirationEvent, ObjectCreatedEvent, S3EventType},
        BroadcastRecv, ReceiverExt, WebhookEvent,
    },
};

use async_broadcast::RecvError;
use futures::{StreamExt, TryStreamExt};
use http::{HeaderMap, HeaderValue, StatusCode};
use hyper::body::Bytes;
use miette::{miette, Context, IntoDiagnostic};
use moka::future::Cache;
use moka::Expiry;
use multi_index_map::MultiIndexMap;
use parking_lot::RwLock;
use s3s::{
    dto::{
        GetObjectOutput, GetObjectOutputMeta, HeadBucketOutput, HeadObjectOutput,
        ListBucketsOutput, ListObjectsOutput, ListObjectsV2Output, SplitMetadata, StreamingBlob,
    },
    ops,
    ops::OperationType,
    Body,
};
use tokio::task::AbortHandle;
use tracing::{debug, error, warn};

mod logic;
pub use logic::*;

type Key = String;
type Data = CachedResponse;
type ETag = String;

#[derive(Debug, Clone)]
struct CachedResponse {
    ttl: Option<Duration>,
    tti: Option<Duration>,
    status_code: StatusCode,
    updated_at: Instant,
    data: CacheData,
}

#[derive(Debug, Clone)]
enum Either<L, R> {
    Left(L),
    Right(R),
}

type GetObjectOutputW = (HeadObjectOutput, Bytes);

#[derive(Debug, Clone)]
enum CacheData {
    GetObject(GetObjectOutputMeta, Bytes),
    HeadObject(HeadObjectOutput),
    ListObjects(Either<ListObjectsOutput, ListObjectsV2Output>),
    Bucket(HeadBucketOutput),
    ListBuckets(ListBucketsOutput),
}

impl TryFrom<CachedResponse> for Response {
    type Error = miette::Report;

    fn try_from(value: CachedResponse) -> Result<Self> {
        match value.data {
            CacheData::GetObject(meta, bytes) => {
                let resp = {
                    let mut output: GetObjectOutput = meta.into();
                    let body = s3s::http::Body::from(bytes);
                    output.set_data(Some(body.into()));

                    debug!("{:#?}", output);

                    let res: s3s::http::Response = output
                        .try_into()
                        .into_diagnostic()
                        .wrap_err("Failed to cast to response")?;

                    debug!("Constructed s3s::http::Response");
                    res
                };

                // TODO: proper headers
                let mut resp: Response = resp.into();

                resp.headers.append(
                    "cache-control",
                    HeaderValue::from_str("no-cache").into_diagnostic()?,
                );

                Ok(resp)
            }
            CacheData::HeadObject(meta) => {
                let resp: s3s::http::Response = meta
                    .try_into()
                    .into_diagnostic()
                    .wrap_err("Failed to cast to response")?;
                Ok(resp.into())
            }
            CacheData::ListObjects(lst) => {
                let resp: s3s::http::Response = match lst {
                    Either::Left(l) => l
                        .try_into()
                        .into_diagnostic()
                        .wrap_err("Failed to cast to response")?,
                    Either::Right(l) => l
                        .try_into()
                        .into_diagnostic()
                        .wrap_err("Failed to cast to response")?,
                };

                Ok(resp.into())
            }
            CacheData::Bucket(bckt) => {
                let resp: s3s::http::Response = bckt.try_into().into_diagnostic()?;

                Ok(resp.into())
            }
            CacheData::ListBuckets(lst) => {
                let resp: s3s::http::Response = lst.try_into().into_diagnostic()?;

                Ok(resp.into())
            }
            _ => unimplemented!(),
        }
    }
}

impl CachedResponse {
    async fn new(resp: &mut Response) -> Result<Self> {
        let s3_ext = resp
            .extensions
            .get::<S3Extension>()
            .ok_or_else(|| miette!("Result is missing S3 Extension"))?;
        let op = s3_ext
            .op
            .as_ref()
            .ok_or_else(|| miette!("Operation type not set"))?;

        return match op {
            OperationType::GetObject(_op) => {
                let output = resp
                    .try_get_output::<ops::GetObject>()
                    .ok_or_else(|| miette!("Missing response metadata"))?;
                let mut body = std::mem::take(&mut resp.body);
                let bytes = body.store_all_unlimited().await.ok();

                Ok(Self {
                    status_code: resp.status,
                    ttl: Default::default(),
                    tti: Default::default(),
                    updated_at: Instant::now(),
                    data: CacheData::GetObject(output.deref().clone(), bytes.unwrap()),
                })
            }
            _ => unimplemented!(),
        };

        /*resp.body = match bytes.clone() {
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
            ttl: Default::default(),
            tti: Default::default(),
            updated_at: Instant::now(),
            data: CacheData::Object(Object {
                etag,
                last_modified: Instant::now(),
                bucket_owner: Owner {
                    id: "".into(),
                    display_name: "".into(),
                },
                bytes,
            }),
        }*/
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
        if let CacheData::GetObject(obj, bytes) = &self.data {
            return bytes.len();
        }
        return 1;
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
        data.try_into()
    }

    fn event_handler(&self, rx: BroadcastRecv) -> impl Future<Output = ()> {
        let cache = self.cache.clone();
        let lut = self.lut.clone();

        rx.recv_stream()
            .inspect_err(|e| match e {
                // Log on lag, no error handling
                RecvError::Overflowed(skipped) => {
                    warn!("Lag while handling events: {} events were skipped", skipped)
                }
                _ => unreachable!(),
            })
            // discard errors
            .filter_map(|e| futures::future::ready(e.ok()))
            // TODO: Investigate: Concurrent handling could become a problem here if events are processed out of order
            .filter_map(|event| {
                futures::future::ready(match event {
                    WebhookEvent::S3(event) => Some(event),
                    _ => None,
                })
            })
            .for_each_concurrent(None, move |event| {
                debug!("CacheLayer received message: {:?}", event);
                let cache = cache.clone();

                async move {
                    for record in event.records {
                        debug!("{:?}", record);
                        match record.event_type {
                            S3EventType::ObjectCreated(ev) => match ev {
                                ObjectCreatedEvent::Post => {
                                    // Existing object was updated
                                    let key = format!(
                                        "op=GetObject, {}, {}, {}",
                                        record.s3.bucket.name, record.s3.object.key, ""
                                    );
                                    cache.invalidate(key.as_str());
                                }
                                _ => continue,
                            },
                            S3EventType::ObjectRemoved(_) => {}
                            S3EventType::LifecycleExpiration(ev) => match ev {
                                LifecycleExpirationEvent::Delete => {}
                                _ => continue,
                            },
                            _ => continue,
                        }
                    }
                }
            })
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
                .wrap_err("Failed to construct cached response")?
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
            let mut lut = self.lut.write();
            lut.remove_by_key(&ee.key);
            lut.insert(ee);
        }

        Ok(resp)
    }

    fn subscribe(&mut self, tx: &BroadcastSend) {
        // Abort previously started tasks
        self.unsubscribe();

        let rx = tx.new_receiver();

        let handle = tokio::spawn(self.event_handler(rx));

        self.rx_abort = Some(handle.abort_handle());
    }

    fn unsubscribe(&mut self) {
        if let Some(h) = self.rx_abort.take() {
            h.abort()
        }
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
