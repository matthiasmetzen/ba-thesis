use std::time::{Duration, Instant, SystemTime};

use super::*;
use crate::{
    client::s3::S3Error,
    config::CacheMiddlewareConfig,
    req::{s3::S3Response, *},
    webhook::{
        event_types::{
            LifecycleExpirationEvent, ObjectCreatedEvent, ObjectRemovedEvent, ObjectRestoreEvent,
            S3EventType,
        },
        BroadcastRecv, ReceiverExt, WebhookEvent,
    },
};

use http_cache_semantics::{BeforeRequest, CacheOptions, CachePolicy};
use miette::Report;

use async_broadcast::RecvError;
use futures::{StreamExt, TryStreamExt};

use hyper::body::Bytes;
use miette::{miette, Context, IntoDiagnostic};
use moka::future::Cache;
use moka::Expiry;
use s3s::{
    dto::{
        GetObjectOutput, GetObjectOutputMeta, HeadBucketOutput, HeadObjectOutput,
        ListBucketsOutput, ListObjectVersionsOutput, ListObjectsOutput, ListObjectsV2Output,
        SplitMetadata,
    },
    ops,
    ops::OperationType,
};
use tokio::task::AbortHandle;
use tracing::{debug, error, warn};

mod logic;
pub use logic::*;

/// The key type used by the cache
type Key = String;

/// The data type used by the cache
type Data = CachedResponse;

/// Representation for a cached response with cache data
#[derive(Debug, Clone)]
struct CachedResponse {
    /// Time-to-Live
    ttl: Option<Duration>,
    /// Time-to-Idle
    tti: Option<Duration>,
    /// Time last updated
    updated_at: SystemTime,
    /// Actual response data
    data: CacheData,
}

#[derive(Debug, Clone)]
enum Either<L, R> {
    Left(L),
    Right(R),
}

/// Types of cached data representation for supported operations
#[derive(Debug, Clone)]
enum CacheData {
    GetObject(GetObjectOutputMeta, Bytes),
    HeadObject(HeadObjectOutput),
    ListObjects(Either<ListObjectsOutput, ListObjectsV2Output>),
    ListObjectVersions(ListObjectVersionsOutput),
    Bucket(HeadBucketOutput),
    ListBuckets(ListBucketsOutput),
}

#[derive(Debug)]
pub enum CacheState<T> {
    Fresh(T),
    Stale(T),
    None,
}

/// Creates a new [Response] from a [CachedResponse]
impl TryFrom<CachedResponse> for Response {
    type Error = miette::Report;

    fn try_from(value: CachedResponse) -> Result<Self, Report> {
        // Each CacheData variant needs its own handling
        match value.data {
            CacheData::GetObject(meta, bytes) => {
                let resp = {
                    // build response from metadata + body bytes
                    let mut output: GetObjectOutput = meta.into();
                    let body = s3s::http::Body::from(bytes);
                    output.set_data(Some(body.into()));

                    let res: s3s::http::Response = output
                        .try_into()
                        .into_diagnostic()
                        .wrap_err("Failed to cast to response")?;
                    res
                };

                let resp: Response = resp.into();

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
                    // select between ListObjects and ListObjectsV2 response
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
            CacheData::ListObjectVersions(lst) => {
                let resp: s3s::http::Response = lst.try_into().into_diagnostic()?;

                Ok(resp.into())
            }
            CacheData::Bucket(meta) => {
                let resp: s3s::http::Response = meta
                    .try_into()
                    .into_diagnostic()
                    .wrap_err("Failed to cast to response")?;

                Ok(resp.into())
            }
            CacheData::ListBuckets(lst) => {
                let resp: s3s::http::Response = lst.try_into().into_diagnostic()?;

                Ok(resp.into())
            }
        }
    }
}

impl CachedResponse {
    // Set Time-to-Live
    fn time_to_live(self, ttl: impl Into<Option<Duration>>) -> Self {
        let mut this = self;
        this.ttl = ttl.into();
        this
    }

    // Set Time-to-Idle
    fn time_to_idle(self, tti: impl Into<Option<Duration>>) -> Self {
        let mut this = self;
        this.tti = tti.into();
        this
    }

    // Size of the response. This is used for the cache weighting
    fn size(&self) -> usize {
        // +8 for size of status code + padding
        match &self.data {
            CacheData::GetObject(_, bytes) => bytes.len(),
            _ => 1,
        }
    }
}

/// Async version of [std::convert::From]
trait AsyncFrom<T>: Sized {
    async fn async_from(value: T) -> Self;
}

impl<'a> AsyncFrom<&mut S3Response<'a, ops::GetObject>> for CachedResponse {
    async fn async_from(resp: &mut S3Response<'a, ops::GetObject>) -> Self {
        // TODO: limit cachable body size
        let bytes = {
            // take body from response
            let mut body = std::mem::take(&mut resp.body);
            // read all bytes for the response
            let bytes = body.store_all_unlimited().await.ok();
            // reattach body to response
            if let Some(ref b) = bytes {
                resp.body = s3s::Body::from(b.clone());
            } else {
                resp.body = body;
            }

            bytes
        };

        Self {
            ttl: Default::default(),
            tti: Default::default(),
            updated_at: SystemTime::now(),
            data: CacheData::GetObject(resp.metadata.as_ref().clone(), bytes.unwrap()),
        }
    }
}

impl<'a> AsyncFrom<&mut S3Response<'a, ops::HeadObject>> for CachedResponse {
    async fn async_from(resp: &mut S3Response<'a, ops::HeadObject>) -> Self {
        Self {
            ttl: Default::default(),
            tti: Default::default(),
            updated_at: SystemTime::now(),
            data: CacheData::HeadObject(resp.metadata.as_ref().clone()),
        }
    }
}

impl<'a> AsyncFrom<&mut S3Response<'a, ops::ListObjects>> for CachedResponse {
    async fn async_from(resp: &mut S3Response<'a, ops::ListObjects>) -> Self {
        Self {
            ttl: Default::default(),
            tti: Default::default(),
            updated_at: SystemTime::now(),
            data: CacheData::ListObjects(Either::Left(resp.metadata.as_ref().clone())),
        }
    }
}

impl<'a> AsyncFrom<&mut S3Response<'a, ops::ListObjectsV2>> for CachedResponse {
    async fn async_from(resp: &mut S3Response<'a, ops::ListObjectsV2>) -> Self {
        Self {
            ttl: Default::default(),
            tti: Default::default(),
            updated_at: SystemTime::now(),
            data: CacheData::ListObjects(Either::Right(resp.metadata.as_ref().clone())),
        }
    }
}

impl<'a> AsyncFrom<&mut S3Response<'a, ops::ListObjectVersions>> for CachedResponse {
    async fn async_from(resp: &mut S3Response<'a, ops::ListObjectVersions>) -> Self {
        Self {
            ttl: Default::default(),
            tti: Default::default(),
            updated_at: SystemTime::now(),
            data: CacheData::ListObjectVersions(resp.metadata.as_ref().clone()),
        }
    }
}

impl<'a> AsyncFrom<&mut S3Response<'a, ops::HeadBucket>> for CachedResponse {
    async fn async_from(resp: &mut S3Response<'a, ops::HeadBucket>) -> Self {
        Self {
            ttl: Default::default(),
            tti: Default::default(),
            updated_at: SystemTime::now(),
            data: CacheData::Bucket(resp.metadata.as_ref().clone()),
        }
    }
}

impl<'a> AsyncFrom<&mut S3Response<'a, ops::ListBuckets>> for CachedResponse {
    async fn async_from(resp: &mut S3Response<'a, ops::ListBuckets>) -> Self {
        Self {
            ttl: Default::default(),
            tti: Default::default(),
            updated_at: SystemTime::now(),
            data: CacheData::ListBuckets(resp.metadata.as_ref().clone()),
        }
    }
}

/// Per-item expiration policy for [CachedResponse] that uses the TTL and TTI on the object. TTL is reset on update
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

/// A [crate::middleware::Layer] that implements caching for multiple S3 operations.
/// Can process webhook events with [crate::webhook::event_types::S3WebhookEvent]
pub struct CacheLayer {
    cache: Arc<Cache<Key, Data>>,
    config: CacheMiddlewareConfig,
    rx_abort: Option<AbortHandle>,
}

impl From<CacheMiddlewareConfig> for CacheLayer {
    fn from(config: CacheMiddlewareConfig) -> Self {
        // Creates an asynchronous cache with weighting, global ttl, global tti and per-item-exiration
        let mut cache = Cache::builder()
            .max_capacity(config.cache_size)
            .weigher(|_k: &Key, v: &CachedResponse| -> u32 {
                v.size().try_into().unwrap_or(u32::MAX)
            })
            .expire_after(PerItemExpiration);

        if let Some(ttl) = config.ttl.map(Duration::from_millis) {
            cache = cache.time_to_live(ttl)
        }

        if let Some(tti) = config.tti.map(Duration::from_millis) {
            cache = cache.time_to_live(tti)
        }

        Self {
            cache: Arc::new(cache.build()),
            config,
            rx_abort: None,
        }
    }
}

impl From<&CacheMiddlewareConfig> for CacheLayer {
    fn from(config: &CacheMiddlewareConfig) -> Self {
        Self::from(config.clone())
    }
}

impl CacheLayer {
    #[allow(unused)]
    pub fn new(
        capacity: u64,
        ttl: impl Into<Option<Duration>>,
        tti: impl Into<Option<Duration>>,
    ) -> Self {
        let config = CacheMiddlewareConfig {
            cache_size: capacity,
            ttl: ttl.into().map(|d: Duration| d.as_millis() as u64),
            tti: tti.into().map(|d: Duration| d.as_millis() as u64),
            ..Default::default()
        };

        Self::from(config)
    }

    /// Gets a response from the cache. Does not check for HTTP cache policy.
    #[allow(unused)]
    pub fn get_cached_response(&self, key: &Key) -> Result<Response, SendError> {
        let data = self
            .cache
            .get(key)
            .ok_or_else(|| SendError::Internal(miette!("No cache entry found")))?;

        debug!("found cache entry for {}", key);
        data.try_into().map_err(SendError::Internal)
    }

    /// Gets a response from the cache. Responses are checked for staleness via HTTP cache policy.
    pub async fn get_matching_response(
        &self,
        key: &Key,
        req: &mut Request,
    ) -> CacheState<Response> {
        let Some(data) = self.cache.get(key) else {
            return CacheState::None;
        };

        debug!("found cache entry for {}", key);
        let resp_time = data.updated_at;
        let Ok(mut resp) = data.try_into() else {
            return CacheState::None;
        };

        // Cache policy for private (non-shared) cache
        let options = CacheOptions {
            shared: false,
            ..Default::default()
        };

        let now = SystemTime::now();

        let policy = CachePolicy::new_options(req, &resp, resp_time, options);
        debug!("is cacheable: {}", policy.is_storable());

        // Fixes: TTL was set to 0 since no caching headers were found on the response
        // this is an ugly escape hatch
        if policy.time_to_live(resp_time) == Duration::from_secs(0) {
            return CacheState::Fresh(resp);
        }

        // Check http cache policy
        match policy.before_request(req, now) {
            BeforeRequest::Fresh(parts) => {
                debug!("cache entry for {} was fresh", key);
                resp.headers.extend(parts.headers);
                CacheState::Fresh(resp)
            }
            BeforeRequest::Stale { request, matches } => {
                debug!("cache entry for {} was stale", key);

                if !matches {
                    // Response from cache did not match the request. remove cache entry and send request unconditionally
                    self.cache.remove(key).await;
                    return CacheState::None;
                }

                req.headers.extend(request.headers);
                CacheState::Stale(resp)
            }
        }
    }

    fn event_handler(&self, rx: BroadcastRecv) -> impl Future<Output = ()> {
        let cache = self.cache.clone();

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
                            /*
                                TODOs:
                                    - refetch when possible
                                    - delete ListObject caches matching updated prefixes
                            */
                            // New object was created
                            S3EventType::ObjectCreated(ev) => match ev {
                                ObjectCreatedEvent::Any
                                | ObjectCreatedEvent::CompleteMultipartUpload
                                | ObjectCreatedEvent::Copy
                                | ObjectCreatedEvent::Put => {
                                    let key_data = KeyData::GetObject {
                                        bucket: &record.s3.bucket.name,
                                        object: &record.s3.object.key,
                                        version_id: &record.s3.object.version_id,
                                    };

                                    cache.invalidate(&key_data.as_key()).await;

                                    let key_data = KeyData::HeadObject {
                                        bucket: &record.s3.bucket.name,
                                        object: &record.s3.object.key,
                                        version_id: &record.s3.object.version_id,
                                    };

                                    cache.invalidate(&key_data.as_key()).await;

                                    // TODO: Clear ListObject, ListObjectVersions
                                }
                                //Existing object was updated. Refetch possible
                                ObjectCreatedEvent::Post => {
                                    let key_data = KeyData::GetObject {
                                        bucket: &record.s3.bucket.name,
                                        object: &record.s3.object.key,
                                        version_id: &record.s3.object.version_id,
                                    };

                                    cache.invalidate(&key_data.as_key()).await;

                                    let key_data = KeyData::HeadObject {
                                        bucket: &record.s3.bucket.name,
                                        object: &record.s3.object.key,
                                        version_id: &record.s3.object.version_id,
                                    };

                                    cache.invalidate(&key_data.as_key()).await;

                                    //TODO: refetch updated
                                }
                                _ => unimplemented!(),
                            },
                            // Object was removed
                            S3EventType::ObjectRemoved(ev) => match ev {
                                ObjectRemovedEvent::Any | ObjectRemovedEvent::Delete => {
                                    let key_data = KeyData::GetObject {
                                        bucket: &record.s3.bucket.name,
                                        object: &record.s3.object.key,
                                        version_id: &record.s3.object.version_id,
                                    };

                                    cache.invalidate(&key_data.as_key()).await;

                                    let key_data = KeyData::HeadObject {
                                        bucket: &record.s3.bucket.name,
                                        object: &record.s3.object.key,
                                        version_id: &record.s3.object.version_id,
                                    };

                                    cache.invalidate(&key_data.as_key()).await;

                                    // TODO: Clear ListObject, ListObjectVersions
                                }
                                _ => unimplemented!(),
                            },
                            // Object expired
                            S3EventType::LifecycleExpiration(ev) => match ev {
                                LifecycleExpirationEvent::Delete => {
                                    let key_data = KeyData::GetObject {
                                        bucket: &record.s3.bucket.name,
                                        object: &record.s3.object.key,
                                        version_id: &record.s3.object.version_id,
                                    };

                                    cache.invalidate(&key_data.as_key()).await;

                                    let key_data = KeyData::HeadObject {
                                        bucket: &record.s3.bucket.name,
                                        object: &record.s3.object.key,
                                        version_id: &record.s3.object.version_id,
                                    };

                                    cache.invalidate(&key_data.as_key()).await;

                                    // TODO: Clear ListObject, ListObjectVersions
                                }
                                _ => unimplemented!(),
                            },
                            S3EventType::ObjectRestore(ev) => match ev {
                                ObjectRestoreEvent::Any
                                | ObjectRestoreEvent::Completed
                                | ObjectRestoreEvent::Delete
                                | ObjectRestoreEvent::Post => {
                                    let key_data = KeyData::GetObject {
                                        bucket: &record.s3.bucket.name,
                                        object: &record.s3.object.key,
                                        version_id: &record.s3.object.version_id,
                                    };

                                    cache.invalidate(&key_data.as_key()).await;

                                    let key_data = KeyData::HeadObject {
                                        bucket: &record.s3.bucket.name,
                                        object: &record.s3.object.key,
                                        version_id: &record.s3.object.version_id,
                                    };

                                    cache.invalidate(&key_data.as_key()).await;
                                }
                            },
                            _ => unimplemented!(),
                        }
                    }
                }
            })
    }
}

/// Provides unified key generation for operations
enum KeyData<'a> {
    GetObject {
        bucket: &'a str,
        object: &'a str,
        version_id: &'a str,
    },
    HeadObject {
        bucket: &'a str,
        object: &'a str,
        version_id: &'a str,
    },
    ObjectList {
        bucket: &'a str,
        prefix: Option<&'a str>,
        delim: Option<&'a str>,
    },
    ObjectVersionList {
        bucket: &'a str,
        prefix: Option<&'a str>,
        delim: Option<&'a str>,
    },
    Bucket {
        bucket: &'a str,
    },
    BucketList,
}

impl From<&KeyData<'_>> for Key {
    fn from(value: &KeyData<'_>) -> Self {
        match value {
            KeyData::GetObject {
                bucket,
                object,
                version_id,
            } => format!("GetObject {}, {}, {}", bucket, object, version_id),
            KeyData::HeadObject {
                bucket,
                object,
                version_id,
            } => format!("HeadObject {}, {}, {}", bucket, object, version_id),
            KeyData::ObjectList {
                bucket,
                prefix,
                delim,
            } => format!(
                "ObjectList {}, {}, {}",
                bucket,
                prefix.unwrap_or_default(),
                delim.unwrap_or_default()
            ),
            KeyData::ObjectVersionList {
                bucket,
                prefix,
                delim,
            } => format!(
                "ObjectVersionList {}, {}, {}",
                bucket,
                prefix.unwrap_or_default(),
                delim.unwrap_or_default()
            ),
            KeyData::Bucket { bucket } => format!("Bucket {}", bucket),
            KeyData::BucketList => "BucketList".into(),
        }
    }
}

impl KeyData<'_> {
    fn as_key(&self) -> Key {
        self.into()
    }
}

impl CacheLogic for CacheLayer {
    fn make_cache_intent(
        &self,
        request: &Request,
        config: &CacheMiddlewareConfig,
    ) -> Option<CacheIntent> {
        if let Some(s3ext) = request.extensions.get::<S3Extension>() {
            return s3ext.make_cache_intent(request, config);
        }

        None
    }
}

#[async_trait::async_trait]
impl Layer for CacheLayer {
    async fn call(&self, mut req: Request, next: &dyn NextLayer) -> Result<Response, SendError> {
        // Check cachability and get expiration settings from config
        let Some(intent) = self.make_cache_intent(&req, &self.config) else {
            // Request is not cacheable
            return next.call(req).await;
        };

        let key = intent.key;

        // Check if the etag header is set. We later check if a new etag was set for the upstream request
        let has_etag = req.headers.get(http::header::ETAG).is_some();

        // get response and staleness from cache
        let cached = self.get_matching_response(&key, &mut req).await;

        if let CacheState::Fresh(resp) = cached {
            // Fresh responses can be sent as-is
            return Ok(resp);
        }

        // Response not stored or stale
        let mut resp = match next.call(req).await {
            Ok(r) => r,
            Err(e) => match (e, cached) {
                // Responses with 304 Not Modified will be passed as SendError::ResponseErr
                (SendError::ResponseErr(err_resp, report), CacheState::Stale(c)) => {
                    match (err_resp.status, has_etag) {
                        // We added caching headers, must respond with cached data
                        (http::StatusCode::NOT_MODIFIED, false) => c,
                        // Client added caching headers, forward 304 response
                        (http::StatusCode::NOT_MODIFIED, true) => return Ok(err_resp),
                        _ => return Err(SendError::ResponseErr(err_resp, report)),
                    }
                }
                (e, _) => return Err(e),
            },
        };

        // Add or update cache entry
        if let Some(s3_ext) = resp.extensions.get::<S3Extension>() {
            let Some(op) = s3_ext.op.as_ref() else {
                return Err(S3Error::MissingOp.into());
            };

            // Create CachedResponse from response
            let cr = match op {
                OperationType::GetObject(_) => {
                    let mut resp: S3Response<ops::GetObject> = S3Response::try_from(&mut resp)?;
                    let cr = CachedResponse::async_from(&mut resp).await;
                    cr
                }
                OperationType::HeadObject(_) => {
                    let mut resp: S3Response<ops::HeadObject> = S3Response::try_from(&mut resp)?;
                    let cr = CachedResponse::async_from(&mut resp).await;
                    cr
                }
                OperationType::ListObjects(_) => {
                    let mut resp: S3Response<ops::ListObjects> = S3Response::try_from(&mut resp)?;
                    let cr = CachedResponse::async_from(&mut resp).await;
                    cr
                }
                OperationType::ListObjectsV2(_) => {
                    let mut resp: S3Response<ops::ListObjectsV2> = S3Response::try_from(&mut resp)?;
                    let cr = CachedResponse::async_from(&mut resp).await;
                    cr
                }
                OperationType::ListObjectVersions(_) => {
                    let mut resp: S3Response<ops::ListObjectVersions> =
                        S3Response::try_from(&mut resp)?;
                    let cr = CachedResponse::async_from(&mut resp).await;
                    cr
                }
                OperationType::HeadBucket(_) => {
                    let mut resp: S3Response<ops::HeadBucket> = S3Response::try_from(&mut resp)?;
                    let cr = CachedResponse::async_from(&mut resp).await;
                    cr
                }
                OperationType::ListBuckets(_) => {
                    let mut resp: S3Response<ops::ListBuckets> = S3Response::try_from(&mut resp)?;
                    let cr = CachedResponse::async_from(&mut resp).await;
                    cr
                }
                _ => {
                    error!("Unimplemented cached response for {}", op.name());
                    return Ok(resp);
                }
            };

            // set TTL & TTI
            let cr = cr
                .time_to_live(intent.ttl.map(Duration::from_millis))
                .time_to_idle(intent.tti.map(Duration::from_millis));

            debug!("{:#?}", cr);

            self.cache.insert(key, cr).await;
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

/// Expresses the intent to cache a response and provides per-item-expiration data
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
