use std::any::Any;





use std::ops::Deref;
use std::ops::DerefMut;


use std::sync::Arc;
use std::sync::OnceLock;

use http::Extensions;
use miette::miette;
use miette::Result;

use s3s::auth::Credentials;
use s3s::dto::SplitMetadata;
use s3s::http::{Multipart, OrderedQs};
use s3s::ops::Operation;
use s3s::ops::TypedOperation;
use s3s::path::S3Path;
use s3s::stream::ByteStream;
use s3s::stream::VecByteStream;
use s3s::Body;

use super::Response;
use super::{HttpExtension, Request};

#[derive(Default)]
pub struct S3Extension {
    pub s3_path: Option<S3Path>,
    pub qs: Option<OrderedQs>,

    pub multipart: Option<Multipart>,
    pub vec_stream: Option<VecByteStream>,

    pub credentials: Option<Credentials>,
    pub(crate) op: Option<s3s::ops::OperationType>, // TODO: can be non-optional
    //pub(crate) input: Mutex<Option<Pin<Arc<dyn Any + Send + Sync>>>>,
    // FIXME: Cow<'static, ..> might be better
    pub(crate) data: OnceLock<Arc<dyn Any + Send + Sync>>,
}

impl S3Extension {
    pub fn new_from(old: &Self) -> Self {
        Self {
            s3_path: old.s3_path.clone(),
            qs: old.qs.clone(),
            multipart: None,
            vec_stream: None,
            credentials: old.credentials.clone(),
            op: old.op.clone(),
            data: OnceLock::new(),
        }
    }
}

impl std::fmt::Debug for S3Extension {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("S3Extension")
            .field("s3_path", &self.s3_path)
            .field("op", &self.op.as_ref().map(|e| e.name()).unwrap_or("None"))
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
            data: OnceLock::new(),
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

    /*fn try_get_input<Op>(&self) -> Option<ArcRef<Op::Input>>
    where
        Op: s3s::ops::TypedOperation,
        Op::Input: for<'a> TryFrom<&'a mut s3s::http::Request> + Send + Sync + 'static;*/

    fn try_get_input<Op: S3Operation>(&self) -> Option<Arc<Op::InputMeta>>;
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

    fn try_get_input<Op: S3Operation>(&self) -> Option<Arc<Op::InputMeta>> {
        let s3_ext = self.extensions.get::<S3Extension>()?;

        let val = s3_ext
            .data
            .get_or_try_init(|| -> Result<Arc<dyn Any + Send + Sync + 'static>, ()> {
                let mut req = self.try_as_s3_request().map_err(|_| ())?;
                let inner = Op::Input::try_from(&mut req).map_err(|_| ())?;
                let (meta, _) = inner.split_metadata();
                Ok(Arc::new(meta))
            })
            .ok()?;

        val.clone().downcast::<Op::InputMeta>().ok()
    }
}

pub(crate) trait S3ResponseExt {
    fn as_s3s_response(&self) -> s3s::http::Response;

    fn try_get_output<Op: S3Operation>(&self) -> Option<Arc<Op::OutputMeta>>;
}

impl S3ResponseExt for Response {
    fn as_s3s_response(&self) -> s3s::http::Response {
        s3s::http::Response {
            status: self.status,
            headers: self.headers.clone(),
            body: Body::empty(),
            extensions: Extensions::new(),
        }
    }

    fn try_get_output<Op: S3Operation>(&self) -> Option<Arc<Op::OutputMeta>> {
        let s3_ext = self.extensions.get::<S3Extension>()?;

        let val = s3_ext.data.get()?;

        val.clone().downcast::<Op::OutputMeta>().ok()
    }
}

pub trait S3Operation:
    TypedOperation<
        Input: s3s::dto::SplitMetadata<Meta: Send + Sync>
                   + Send
                   + Sync
                   + From<<<Self as TypedOperation>::Input as s3s::dto::SplitMetadata>::Meta>
                   + for<'a> TryFrom<&'a mut s3s::http::Request>,
        Output: s3s::dto::SplitMetadata<Meta: Send + Sync>
                    + Send
                    + Sync
                    + From<<<Self as TypedOperation>::Output as s3s::dto::SplitMetadata>::Meta>
                    + for<'a> TryInto<s3s::http::Response>,
    > + Operation
{
    type InputMeta: Send + Sync + Clone + From<Self::Input> + Into<<Self as TypedOperation>::Input> =
        <Self::Input as s3s::dto::SplitMetadata>::Meta where Self::Input: From<<Self::Input as s3s::dto::SplitMetadata>::Meta>;
    type OutputMeta: Send + Sync + Clone + From<Self::Output> + Into<Self::Output> =
        <Self::Output as s3s::dto::SplitMetadata>::Meta where Self::Output: From<<Self::Output as s3s::dto::SplitMetadata>::Meta>;
}

impl<Op> S3Operation for Op where
    Op: TypedOperation<
            Input: s3s::dto::SplitMetadata<Meta: Send + Sync>
                       + Send
                       + Sync
                       + From<<<Self as TypedOperation>::Input as s3s::dto::SplitMetadata>::Meta>
                       + for<'a> TryFrom<&'a mut s3s::http::Request>,
            Output: s3s::dto::SplitMetadata<Meta: Send + Sync>
                        + Send
                        + Sync
                        + From<<<Self as TypedOperation>::Output as s3s::dto::SplitMetadata>::Meta>
                        + for<'a> TryInto<s3s::http::Response>,
        > + Operation
{
}

pub struct S3Response<'a, Op: S3Operation> {
    response: &'a mut Response,
    pub metadata: std::sync::Arc<Op::OutputMeta>,
    _op: std::marker::PhantomData<Op>,
}

impl<'a, Op: S3Operation> S3Response<'a, Op> {
    #[allow(unused)]
    pub fn into_inner(self) -> &'a mut Response {
        self.response
    }
}

impl<'a, Op: S3Operation> TryFrom<&'a mut Response> for S3Response<'a, Op> {
    type Error = miette::Error;

    fn try_from(resp: &'a mut Response) -> Result<Self, Self::Error> {
        let output = resp.try_get_output::<Op>().ok_or_else(|| {
            miette!(
                "No response data found for operation {}",
                std::any::type_name::<Op>()
            )
        })?;

        Ok(Self {
            response: resp,
            metadata: output,
            _op: std::marker::PhantomData,
        })
    }
}

impl<Op: S3Operation> Deref for S3Response<'_, Op> {
    type Target = Response;

    fn deref(&self) -> &Self::Target {
        self.response
    }
}

impl<Op: S3Operation> DerefMut for S3Response<'_, Op> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.response
    }
}

#[cfg(test)]
mod tests {

    use ctor::ctor;

    

    

    #[ctor]
    fn prepare() {
        let _ = crate::try_init_tracing();
    }
}
