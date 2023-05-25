use std::any::TypeId;

use http::Extensions;
use miette::miette;
use miette::Result;
use s3s::auth::Credentials;
use s3s::http::{Multipart, OrderedQs};
use s3s::path::S3Path;
use s3s::stream::ByteStream;
use s3s::stream::VecByteStream;
use s3s::Body;

use super::{HttpExtension, Request};

pub(crate) struct Operation(pub &'static dyn s3s::ops::Operation);

#[allow(unused)]
impl Operation {
    pub fn try_as_ref<T: s3s::ops::Operation>(&self) -> Result<&T> {
        let op = self.0.as_any().downcast_ref::<T>();

        op.ok_or_else(|| {
            miette!("Could not downcast ref").context(format!(
                "Expected T to be type {}, but got {}",
                std::any::type_name::<T>(),
                self.0.name()
            ))
        })
    }

    pub fn is_type<T: s3s::ops::Operation>(&self) -> bool {
        self.0.as_any().type_id() == std::any::TypeId::of::<T>()
    }

    pub fn inner_type_id(&self) -> TypeId {
        self.0.as_any().type_id()
    }
}

impl std::fmt::Debug for Operation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct(self.0.name()).finish()
    }
}

impl From<&'static dyn s3s::ops::Operation> for Operation {
    fn from(value: &'static dyn s3s::ops::Operation) -> Self {
        Self(value)
    }
}

#[derive(Default)]
pub struct S3Extension {
    pub s3_path: Option<S3Path>,
    pub qs: Option<OrderedQs>,

    pub multipart: Option<Multipart>,
    pub vec_stream: Option<VecByteStream>,

    pub credentials: Option<Credentials>,
    pub(crate) op: Option<Operation>, // TODO: actual op instead of name
}

impl std::fmt::Debug for S3Extension {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("S3Extension")
            .field("s3_path", &self.s3_path)
            .field("op", &self.op)
            .field("qs", &self.qs)
            .field("multipart", &self.multipart)
            .field(
                "vec_stream",
                &self.vec_stream.as_ref().map(|s| s.remaining_length()),
            )
            .field("credentials", &self.credentials)
            .finish()
    }
}

impl From<s3s::http::S3Extensions> for S3Extension {
    fn from(value: s3s::http::S3Extensions) -> Self {
        Self {
            s3_path: value.s3_path,
            qs: value.qs,
            multipart: value.multipart,
            vec_stream: value.vec_stream,
            credentials: value.credentials,
            op: None,
        }
    }
}

impl From<S3Extension> for s3s::http::S3Extensions {
    fn from(s3_ext: S3Extension) -> Self {
        Self {
            s3_path: s3_ext.s3_path,
            qs: s3_ext.qs,
            multipart: s3_ext.multipart,
            vec_stream: s3_ext.vec_stream,
            credentials: s3_ext.credentials,
        }
    }
}

impl TryFrom<Request> for s3s::http::Request {
    type Error = miette::Report;

    fn try_from(req: Request) -> Result<Self, Self::Error> {
        let mut req = req;

        let s3_ext = req
            .extensions
            .remove::<S3Extension>()
            .ok_or_else(|| miette!("Could not get S3 Extension from Request"))?;

        let http_ext = req
            .extensions
            .remove::<HttpExtension>()
            .ok_or_else(|| miette!("Could not get S3 Extension from Request"))?;

        Ok(Self {
            method: http_ext.method,
            uri: http_ext.uri,
            headers: http_ext.headers,
            extensions: req.extensions,
            body: http_ext.body,
            s3ext: s3_ext.into(),
        })
    }
}

impl From<s3s::http::Request> for Request {
    fn from(value: s3s::http::Request) -> Self {
        let mut exts = value.extensions;
        let s3_ext = S3Extension::from(value.s3ext);
        exts.insert(s3_ext);

        let http_ext = HttpExtension {
            method: value.method,
            uri: value.uri,
            headers: value.headers,
            body: value.body,
        };

        exts.insert(http_ext);

        Request { extensions: exts }
    }
}

pub(crate) trait S3RequestExt {
    fn try_as_s3_request(&self) -> Result<s3s::http::Request>;
}

impl S3RequestExt for Request {
    fn try_as_s3_request(&self) -> Result<s3s::http::Request> {
        let http_ext = self
            .extensions
            .get::<HttpExtension>()
            .ok_or_else(|| miette!("Missing S3 extension"))?;

        let s3_ext = self
            .extensions
            .get::<S3Extension>()
            .ok_or_else(|| miette!("Missing S3 extension"))?;

        Ok(s3s::http::Request {
            extensions: Extensions::new(),
            body: Body::empty(),
            headers: http_ext.headers.clone(),
            method: http_ext.method.clone(),
            uri: http_ext.uri.clone(),
            s3ext: s3s::http::S3Extensions {
                s3_path: s3_ext.s3_path.clone(),
                qs: None,
                multipart: None,
                vec_stream: None,
                credentials: s3_ext.credentials.clone(),
            },
        })
    }
}

#[cfg(test)]
mod tests {

    use ctor::ctor;

    use miette::Result;

    use super::*;

    #[ctor]
    fn prepare() {
        let _ = crate::try_init_tracing();
    }

    #[test]
    fn try_cast_op_succ() -> Result<()> {
        let op = Box::new(s3s::ops::GetObject);
        // lazy way to make 'static ref
        let op_ref: &dyn s3s::ops::Operation = Box::leak(op);

        let wrapped = Operation::from(op_ref);

        let _r: &s3s::ops::GetObject = wrapped.try_as_ref()?;

        Ok(())
    }

    #[test]
    fn try_cast_op_fail() -> Result<()> {
        let op = Box::new(s3s::ops::GetObject);
        // lazy way to make 'static ref
        let op_ref: &dyn s3s::ops::Operation = Box::leak(op);

        let wrapped = Operation::from(op_ref);

        let r: Result<&s3s::ops::PutObject, _> = wrapped.try_as_ref();

        assert!(r.is_err());

        Ok(())
    }
}
