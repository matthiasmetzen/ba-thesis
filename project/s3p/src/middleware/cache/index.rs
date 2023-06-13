use tantivy::{
    collector::TopDocs,
    query::QueryParser,
    schema::{Field, Schema, FAST, INDEXED, STORED, STRING},
    DateTime, Index, TantivyError,
};

pub use tantivy::{doc, Document};

#[allow(unused)]
struct CacheFields {
    key: Field,
    last_updated_at: Field,
    op: Field,
    etag: Field,
    bucket: Field,
    object_key: Field,
    version_id: Field,
    bucket_owner: Field,
}

pub struct CacheIndex {
    idx: Index,
    fields: CacheFields,
}

pub struct IndexEntry {
    key: String,
    last_updated_at: DateTime,
    op: String,

    values: IndexEnum,
}

pub enum IndexEnum {
    Object(IndexedObject),
    Bucket(IndexedBucket),
    Other,
}

pub struct IndexedObject {
    etag: String,
    bucket: String,
    object_key: String,
    version_id: Option<String>,
    last_updated_at: DateTime,
    bucket_owner: Option<String>,
}

pub struct IndexedBucket {
    bucket: String,
    bucket_owner: Option<String>,
}

impl CacheIndex {
    pub fn new() -> Self {
        let mut schema = Schema::builder();

        let fields = CacheFields {
            key: schema.add_text_field("key", STRING),
            last_updated_at: schema.add_date_field("last_updated_at", STORED),
            op: schema.add_text_field("op", STRING | STORED),

            etag: schema.add_text_field("etag", STRING | STORED | FAST),
            bucket: schema.add_text_field("bucket", STRING | STORED | FAST),
            object_key: schema.add_facet_field("object_key", INDEXED | STORED),
            version_id: schema.add_text_field("version_id", STRING | STORED | FAST),
            bucket_owner: schema.add_text_field("bucket_owner", STRING | STORED | FAST),
        };

        let schema = schema.build();

        let idx = Index::create_in_ram(schema);

        Self { idx, fields }
    }

    pub fn get_first(&self, query: &str) -> Option<Document> {
        let reader = self.idx.reader().unwrap();
        let searcher = reader.searcher();

        let query_parser = QueryParser::for_index(&self.idx, vec![]);

        let query = query_parser.parse_query(query).unwrap();

        let docs = searcher.search(&query, &TopDocs::with_limit(1)).ok()?;

        searcher.doc(docs.first()?.1).ok()
    }

    pub fn add(&self, key: &str, etag: &str) -> Result<(), TantivyError> {
        let key_field = self.fields.key;
        let etag_field = self.fields.etag;

        let doc = doc!(
            key_field => key,
            etag_field => etag
        );

        let mut writer = self.idx.writer(3_000_000)?;

        if let Some(key) = doc.get_first(key_field).map(|v| v.as_text()).flatten() {
            let query_parser = QueryParser::for_index(&self.idx, vec![key_field]);
            let query = query_parser.parse_query(key)?;
            writer.delete_query(query)?;
        }

        writer.add_document(doc)?;

        writer.commit()?;

        Ok(())
    }

    pub fn delete(&self, query: &str) -> Result<(), TantivyError> {
        let mut writer = self.idx.writer(3_000_000)?;

        let query_parser = QueryParser::for_index(&self.idx, vec![]);

        let query = query_parser.parse_query(query)?;

        writer.delete_query(query)?;

        writer.commit()?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use ctor::ctor;
    use miette::{IntoDiagnostic, Result};
    use tracing::debug;

    use super::*;

    #[ctor]
    fn prepare() {
        let _ = crate::try_init_tracing();
    }

    #[test]
    fn text_cache_index() -> Result<()> {
        let idx = CacheIndex::new();

        idx.add("foo", "bar").into_diagnostic()?;
        idx.add("foo", "baz").into_diagnostic()?;

        let doc = idx.get_first("key:foo");
        assert!(doc.is_some());
        let doc = doc.unwrap();
        debug!("{:#?}", doc);

        idx.delete("etag:bar").into_diagnostic()?;

        let doc = idx.get_first("key:foo");
        assert!(doc.is_some());
        let doc = doc.unwrap();
        debug!("{:#?}", doc);

        Ok(())
    }
}
