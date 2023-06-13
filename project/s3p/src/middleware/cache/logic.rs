use super::*;
use crate::req::s3::S3RequestExt;

use s3s::ops;
use s3s::ops::Operation;

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
