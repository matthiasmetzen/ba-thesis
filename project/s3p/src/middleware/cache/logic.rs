use super::*;
use crate::req::s3::S3RequestExt;

use s3s::ops::Operation;
use s3s::ops::{self, OperationType};

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
        let Some(op) = self.op.as_ref() else {
            return None;
        };

        match op {
            OperationType::GetObject(op) => op.make_cache_intent(request, config),
            OperationType::HeadBucket(op) => op.make_cache_intent(request, config),
            OperationType::ListBuckets(op) => op.make_cache_intent(request, config),
            OperationType::ListObjects(op) => op.make_cache_intent(request, config),
            OperationType::ListObjectsV2(op) => op.make_cache_intent(request, config),
            OperationType::ListObjectVersions(op) => op.make_cache_intent(request, config),
            _ => None,
        }
    }
}

impl CacheLogic for ops::GetObject {
    fn make_cache_intent(
        &self,
        request: &Request,
        _config: &CacheLayerConfig,
    ) -> Option<CacheIntent> {
        let des = request.try_get_input::<Self>()?;

        if des.range.is_some() || des.part_number.is_some() {
            return None;
        }

        let key = format!(
            "op={}, {}:{}:{}",
            self.name(),
            des.bucket,
            des.key,
            des.version_id.as_ref().unwrap_or(&"".to_string())
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
        let des = request.try_get_input::<Self>()?;

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
        let des = request.try_get_input::<Self>()?;

        if des.expected_bucket_owner.is_some() || des.range.is_some() {
            return None;
        }

        let key = format!(
            "op={}, {}, {}",
            self.name(),
            des.bucket,
            des.version_id.as_deref().unwrap_or_default()
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
        let des = request.try_get_input::<Self>()?;

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
        let des = request.try_get_input::<Self>()?;

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
            des.prefix.as_deref().unwrap_or_default()
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
        let des = request.try_get_input::<Self>()?;

        if des.expected_bucket_owner.is_some() || des.key_marker.is_some() || des.max_keys.is_some()
        {
            return None;
        }

        let key = format!(
            "op={}, {}, {}, {}",
            self.name(),
            des.bucket,
            des.prefix.as_deref().unwrap_or_default(),
            des.delimiter.as_deref().unwrap_or_default()
        );

        Some(CacheIntent::new(key))
    }
}
