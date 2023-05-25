use crate::dto::RustTypes;
use crate::gen::Codegen;
use crate::rust::default_value_literal;
use crate::xml::{is_xml_output, is_xml_payload};
use crate::{default, f, headers, o};
use crate::{dto, rust, smithy};

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ops::Not;

use heck::ToSnakeCase;

#[derive(Debug)]
pub struct Operation {
    pub name: String,

    pub input: String,
    pub output: String,

    pub smithy_input: String,
    pub smithy_output: String,

    pub doc: Option<String>,

    pub http_method: String,
    pub http_uri: String,
    pub http_code: u16,
}

pub type Operations = BTreeMap<String, Operation>;

pub fn collect_operations(model: &smithy::Model) -> Operations {
    let mut operations: Operations = default();
    let mut insert = |name, op| assert!(operations.insert(name, op).is_none());

    for (shape_name, shape) in &model.shapes {
        let smithy::Shape::Operation(sh) = shape else { continue };

        let op_name = dto::to_type_name(shape_name).to_owned();

        let cvt = |n| {
            if n == "smithy.api#Unit" {
                o("Unit")
            } else {
                o(dto::to_type_name(n))
            }
        };

        let smithy_input = cvt(sh.input.target.as_str());
        let smithy_output = cvt(sh.output.target.as_str());

        let input = {
            if smithy_input != "Unit" {
                assert_eq!(smithy_input.strip_suffix("Request").unwrap(), op_name);
            }
            f!("{op_name}Input")
        };

        let output = {
            if smithy_output != "Unit" && smithy_output != "NotificationConfiguration" {
                assert_eq!(smithy_output.strip_suffix("Output").unwrap(), op_name);
            }
            f!("{op_name}Output")
        };

        // See https://github.com/awslabs/smithy-rs/discussions/2308
        let smithy_http_code = sh.traits.http_code().unwrap();
        let http_code = if op_name == "PutBucketPolicy" {
            assert_eq!(smithy_output, "Unit");
            assert_eq!(smithy_http_code, 200);
            204
        } else {
            smithy_http_code
        };

        let op = Operation {
            name: op_name.clone(),

            input,
            output,

            smithy_input,
            smithy_output,

            doc: sh.traits.doc().map(o),

            http_method: sh.traits.http_method().unwrap().to_owned(),
            http_uri: sh.traits.http_uri().unwrap().to_owned(),
            http_code,
        };
        insert(op_name, op);
    }

    operations
}

pub fn codegen(ops: &Operations, rust_types: &RustTypes, g: &mut Codegen) {
    g.lines([
        "//! Auto generated by codegen/src/ops.rs",
        "",
        "#![allow(clippy::declare_interior_mutable_const)]",
        "#![allow(clippy::borrow_interior_mutable_const)]",
        "",
        "use crate::dto::*;", //
        "use crate::header::*;",
        "use crate::http;",
        "use crate::error::*;",
        "use crate::path::S3Path;",
        "use crate::s3_trait::S3;",
        "",
        "use std::borrow::Cow;",
        "use std::sync::Arc;",
        "",
    ]);

    codegen_http(ops, rust_types, g);
    codegen_router(ops, rust_types, g);
}

fn status_code_name(code: u16) -> &'static str {
    match code {
        200 => "OK",
        204 => "NO_CONTENT",
        _ => unimplemented!(),
    }
}

fn codegen_http(ops: &Operations, rust_types: &RustTypes, g: &mut Codegen) {
    codegen_header_value(ops, rust_types, g);

    for op in ops.values() {
        g.ln(f!("pub struct {};", op.name));
        g.lf();

        g.ln(f!("impl {} {{", op.name));

        codegen_op_http_de(op, rust_types, g);
        codegen_op_http_ser(op, rust_types, g);

        g.ln("}");
        g.lf();

        codegen_op_http_call(op, g);
        g.lf();
    }
}

fn codegen_header_value(ops: &Operations, rust_types: &RustTypes, g: &mut Codegen) {
    let mut str_enum_names: BTreeSet<&str> = default();

    for op in ops.values() {
        for ty_name in [op.input.as_str(), op.output.as_str()] {
            let rust_type = &rust_types[ty_name];
            match rust_type {
                rust::Type::Provided(_) => {}
                rust::Type::Struct(ty) => {
                    for field in ty.fields.iter().filter(|field| field.position == "header") {
                        let field_type = &rust_types[field.type_.as_str()];
                        match field_type {
                            rust::Type::List(list_ty) => {
                                let member_type = &rust_types[list_ty.member.type_.as_str()];
                                if let rust::Type::StrEnum(ty) = member_type {
                                    str_enum_names.insert(ty.name.as_str());
                                }
                            }
                            rust::Type::StrEnum(ty) => {
                                str_enum_names.insert(ty.name.as_str());
                            }
                            rust::Type::Alias(_) => {}
                            rust::Type::Provided(_) => {}
                            rust::Type::Timestamp(_) => {}
                            _ => unimplemented!("{field_type:#?}"),
                        }
                    }
                }
                _ => unimplemented!(),
            }
        }
    }

    for rust_type in str_enum_names.iter().map(|&x| &rust_types[x]) {
        let rust::Type::StrEnum(ty) = rust_type else { panic!() };

        g.ln(f!("impl http::TryIntoHeaderValue for {} {{", ty.name));
        g.ln("type Error = http::InvalidHeaderValue;");
        g.ln("fn try_into_header_value(self) -> Result<http::HeaderValue, Self::Error> {");
        g.ln("    match Cow::from(self) {");
        g.ln("        Cow::Borrowed(s) => http::HeaderValue::try_from(s),");
        g.ln("        Cow::Owned(s) => http::HeaderValue::try_from(s),");
        g.ln("    }");
        g.ln("}");
        g.ln("}");
        g.lf();
    }

    for rust_type in str_enum_names.iter().map(|&x| &rust_types[x]) {
        let rust::Type::StrEnum(ty) = rust_type else { panic!() };

        g.ln(f!("impl http::TryFromHeaderValue for {} {{", ty.name));
        g.ln("type Error = http::ParseHeaderError;");
        g.ln("fn try_from_header_value(val: &http::HeaderValue) -> Result<Self, Self::Error> {");
        g.ln("    let val = val.to_str().map_err(|_|http::ParseHeaderError::Enum)?;");
        g.ln("    Ok(Self::from(val.to_owned()))");
        g.ln("}");
        g.ln("}");
        g.lf();
    }
}

fn codegen_op_http_ser(op: &Operation, rust_types: &RustTypes, g: &mut Codegen) {
    let output = op.output.as_str();
    let rust_type = &rust_types[output];
    match rust_type {
        rust::Type::Provided(ty) => {
            assert_eq!(ty.name, "Unit");
            g.ln("pub fn serialize_http() -> http::Response {");

            if op.http_code == 200 {
                g.ln("http::Response::default()");
            } else {
                g.ln("let mut res = http::Response::default();");

                let code_name = status_code_name(op.http_code);
                g.ln(f!("res.status = http::StatusCode::{code_name};"));

                g.ln("res");
            }

            g.ln("}");
        }
        rust::Type::Struct(ty) => {
            if ty.fields.is_empty() {
                g.ln(f!("pub fn serialize_http(_: {output}) -> S3Result<http::Response> {{"));
                {
                    let code_name = status_code_name(op.http_code);
                    g.ln(f!("Ok(http::Response::with_status(http::StatusCode::{code_name}))"));
                }
                g.ln("}");
            } else {
                g.ln(f!("pub fn serialize_http(x: {output}) -> S3Result<http::Response> {{"));

                assert!(ty.fields.is_empty().not());
                for field in &ty.fields {
                    assert!(["header", "metadata", "xml", "payload"].contains(&field.position.as_str()),);
                }

                {
                    let code_name = status_code_name(op.http_code);
                    g.ln(f!("let mut res = http::Response::with_status(http::StatusCode::{code_name});"));
                }

                if is_xml_output(ty) {
                    g.ln("http::set_xml_body(&mut res, &x)?;");
                } else if let Some(field) = ty.fields.iter().find(|x| x.position == "payload") {
                    match field.type_.as_str() {
                        "Policy" => {
                            assert!(field.option_type);
                            g.ln(f!("if let Some(val) = x.{} {{", field.name));
                            g.ln("res.body = http::Body::from(val);");
                            g.ln("}");
                        }
                        "StreamingBlob" => {
                            if field.option_type {
                                g.ln(f!("if let Some(val) = x.{} {{", field.name));
                                g.ln("http::set_stream_body(&mut res, val);");
                                g.ln("}");
                            } else {
                                g.ln(f!("http::set_stream_body(&mut res, x.{});", field.name));
                            }
                        }
                        "SelectObjectContentEventStream" => {
                            assert!(field.option_type);
                            g.ln(f!("if let Some(val) = x.{} {{", field.name));
                            g.ln("http::set_event_stream_body(&mut res, val);");
                            g.ln("}");
                        }
                        _ => {
                            if field.option_type {
                                g.ln(f!("if let Some(ref val) = x.{} {{", field.name));
                                g.ln("    http::set_xml_body(&mut res, val)?;");
                                g.ln("}");
                            } else {
                                g.ln(f!("http::set_xml_body(&mut res, &x.{})?;", field.name));
                            }
                        }
                    }
                }

                for field in &ty.fields {
                    if field.position == "header" {
                        let field_name = field.name.as_str();
                        let header_name = crate::headers::to_constant_name(field.http_header.as_deref().unwrap());

                        let field_type = &rust_types[field.type_.as_str()];
                        if let rust::Type::Timestamp(ts_ty) = field_type {
                            assert!(field.option_type);
                            let fmt = ts_ty.format.as_deref().unwrap_or("HttpDate");
                            g.ln(f!(
                            "http::add_opt_header_timestamp(&mut res, {header_name}, x.{field_name}, TimestampFormat::{fmt})?;"
                        ));
                        } else if field.option_type {
                            g.ln(f!("http::add_opt_header(&mut res, {header_name}, x.{field_name})?;"));
                        } else {
                            g.ln(f!("http::add_header(&mut res, {header_name}, x.{field_name})?;"));
                        }
                    }
                    if field.position == "metadata" {
                        assert!(field.option_type);
                        g.ln(f!("http::add_opt_metadata(&mut res, x.{})?;", field.name));
                    }
                }

                g.ln("Ok(res)");

                g.ln("}");
            }
        }
        _ => unimplemented!(),
    }
    g.lf();
}

fn codegen_op_http_de(op: &Operation, rust_types: &RustTypes, g: &mut Codegen) {
    let input = op.input.as_str();
    let rust_type = &rust_types[input];
    match rust_type {
        rust::Type::Provided(ty) => {
            assert_eq!(ty.name, "Unit");
        }
        rust::Type::Struct(ty) => {
            if ty.fields.is_empty() {
                g.ln(f!("pub fn deserialize_http(_: &mut http::Request) -> S3Result<{input}> {{"));
                g.ln(f!("Ok({input} {{}})"));
                g.ln("}");
            } else {
                g.ln(f!("pub fn deserialize_http(req: &mut http::Request) -> S3Result<{input}> {{"));

                if op.name == "PutObject" {
                    // POST object
                    g.ln("if let Some(m) = req.s3ext.multipart.take() {");
                    g.ln("    return Self::deserialize_http_multipart(req, m);");
                    g.ln("}");
                    g.lf();
                }

                let path_pattern = PathPattern::parse(op.http_uri.as_str());
                match path_pattern {
                    PathPattern::Root => {}
                    PathPattern::Bucket => {
                        if op.name != "WriteGetObjectResponse" {
                            g.ln("let bucket = http::unwrap_bucket(req);");
                            g.lf();
                        }
                    }
                    PathPattern::Object => {
                        g.ln("let (bucket, key) = http::unwrap_object(req);");
                        g.lf();
                    }
                }

                for field in &ty.fields {
                    match field.position.as_str() {
                        "bucket" => {
                            assert_eq!(field.name, "bucket");
                        }
                        "key" => {
                            assert_eq!(field.name, "key");
                        }
                        "query" => {
                            let query = field.http_query.as_deref().unwrap();

                            let field_type = &rust_types[&field.type_];

                            if let rust::Type::List(_) = field_type {
                                panic!()
                            } else if let rust::Type::Timestamp(ts_ty) = field_type {
                                assert!(field.option_type);
                                let fmt = ts_ty.format.as_deref().unwrap_or("DateTime");
                                g.ln(f!(
                                    "let {}: Option<{}> = http::parse_opt_query_timestamp(req, \"{}\", TimestampFormat::{})?;",
                                    field.name,
                                    field.type_,
                                    query,
                                    fmt
                                ));
                            } else if field.option_type {
                                g.ln(f!(
                                    "let {}: Option<{}> = http::parse_opt_query(req, \"{}\")?;",
                                    field.name,
                                    field.type_,
                                    query
                                ));
                            } else if let Some(ref default_value) = field.default_value {
                                let literal = default_value_literal(default_value);
                                g.ln(f!(
                                    "let {}: {} = http::parse_opt_query(req, \"{}\")?.unwrap_or({});",
                                    field.name,
                                    field.type_,
                                    query,
                                    literal,
                                ));
                            } else {
                                g.ln(f!("let {}: {} = http::parse_query(req, \"{}\")?;", field.name, field.type_, query,));
                            }
                        }
                        "header" => {
                            let header = headers::to_constant_name(field.http_header.as_deref().unwrap());
                            let field_type = &rust_types[&field.type_];

                            if let rust::Type::List(_) = field_type {
                                assert!(field.option_type.not());
                                g.ln(f!("let {}: {} = http::parse_list_header(req, &{})?;", field.name, field.type_, header));
                            } else if let rust::Type::Timestamp(ts_ty) = field_type {
                                assert!(field.option_type);
                                let fmt = ts_ty.format.as_deref().unwrap_or("HttpDate");
                                g.ln(f!(
                                    "let {}: Option<{}> = http::parse_opt_header_timestamp(req, &{}, TimestampFormat::{})?;",
                                    field.name,
                                    field.type_,
                                    header,
                                    fmt
                                ));
                            } else if field.option_type {
                                g.ln(f!(
                                    "let {}: Option<{}> = http::parse_opt_header(req, &{})?;",
                                    field.name,
                                    field.type_,
                                    header
                                ));
                            } else if let Some(ref default_value) = field.default_value {
                                // ASK: content length
                                // In S3 smithy model, content-length has a default value (0).
                                // Why? Is it correct???

                                let literal = default_value_literal(default_value);
                                g.ln(f!(
                                    "let {}: {} = http::parse_opt_header(req, &{})?.unwrap_or({});",
                                    field.name,
                                    field.type_,
                                    header,
                                    literal,
                                ));
                            } else {
                                g.ln(f!("let {}: {} = http::parse_header(req, &{})?;", field.name, field.type_, header));
                            }
                        }
                        "metadata" => {
                            assert!(field.option_type);
                            g.ln(f!("let {}: Option<{}> = http::parse_opt_metadata(req)?;", field.name, field.type_));
                        }
                        "payload" => match field.type_.as_str() {
                            "Policy" => {
                                assert!(field.option_type.not());
                                g.ln(f!("let {}: {} = http::take_string_body(req)?;", field.name, field.type_));
                            }
                            "StreamingBlob" => {
                                assert!(field.option_type);
                                g.ln(f!("let {}: Option<{}> = Some(http::take_stream_body(req));", field.name, field.type_));
                            }
                            _ => {
                                if field.option_type {
                                    g.ln(f!("let {}: Option<{}> = http::take_opt_xml_body(req)?;", field.name, field.type_));
                                } else {
                                    g.ln(f!("let {}: {} = http::take_xml_body(req)?;", field.name, field.type_));
                                }
                            }
                        },

                        _ => unimplemented!(),
                    }
                    g.lf();
                }

                g.ln(f!("Ok({input} {{"));
                for field in &ty.fields {
                    match field.position.as_str() {
                        "bucket" | "key" | "query" | "header" | "metadata" | "payload" => {
                            g.ln(f!("{},", field.name));
                        }
                        _ => unimplemented!(),
                    }
                }
                g.ln("})");

                g.ln("}");
                g.lf();

                if op.name == "PutObject" {
                    codegen_op_http_de_multipart(op, rust_types, g);
                }
            }
        }
        _ => unimplemented!(),
    }
    g.lf();
}

fn codegen_op_http_de_multipart(op: &Operation, rust_types: &RustTypes, g: &mut Codegen) {
    assert_eq!(op.name, "PutObject");

    g.ln(f!(
        "pub fn deserialize_http_multipart(req: &mut http::Request, m: http::Multipart) -> S3Result<{}> {{",
        op.input
    ));

    g.lines([
        "let bucket = http::unwrap_bucket(req);",
        "let key = http::parse_field_value(&m, \"key\")?.ok_or_else(|| invalid_request!(\"missing key\"))?;",
        "",
        "let vec_stream = req.s3ext.vec_stream.take().expect(\"missing vec stream\");",
        "",
        "let content_length = i64::try_from(vec_stream.exact_remaining_length()).map_err(|e|s3_error!(e, InvalidArgument, \"content-length overflow\"))?;",
        "let content_length = (content_length != 0).then_some(content_length);",
        "",
        "let body: Option<StreamingBlob> = Some(StreamingBlob::new(vec_stream));",
        "",
    ]);

    let rust::Type::Struct(ty) = &rust_types[op.input.as_str()] else { panic!() };

    for field in &ty.fields {
        match field.position.as_str() {
            "bucket" | "key" | "payload" => {}
            "header" => {
                let header = field.http_header.as_deref().unwrap();
                assert!(header.as_bytes().iter().all(|&x| x == b'-' || x.is_ascii_alphanumeric()));
                let header = header.to_ascii_lowercase();

                if header == "content-length" {
                    continue;
                }

                let field_type = &rust_types[field.type_.as_str()];

                if let rust::Type::Timestamp(ts_ty) = field_type {
                    assert!(field.option_type);
                    let fmt = ts_ty.format.as_deref().unwrap_or("HttpDate");
                    g.ln(f!(
                        "let {}: Option<{}> = http::parse_field_value_timestamp(&m, \"{}\", TimestampFormat::{})?;",
                        field.name,
                        field.type_,
                        header,
                        fmt
                    ));
                } else if field.option_type {
                    g.ln(f!(
                        "let {}: Option<{}> = http::parse_field_value(&m, \"{}\")?;",
                        field.name,
                        field.type_,
                        header
                    ));
                } else if let Some(ref default_value) = field.default_value {
                    g.ln(f!(
                        "let {}: {} = http::parse_field_value(&m, \"{}\")?.unwrap_or({});",
                        field.name,
                        field.type_,
                        header,
                        default_value_literal(default_value)
                    ));
                } else {
                    unimplemented!()
                }
            }
            "metadata" => {
                assert!(field.option_type);
                g.ln(f!("let {}: Option<{}> = {{", field.name, field.type_));
                g.ln(f!("    let mut metadata: {} = Default::default();", field.type_));
                g.ln("    for (name, value) in m.fields() {");
                g.ln("        if let Some(key) = name.strip_prefix(\"x-amz-meta-\") {");
                g.ln("            if key.is_empty() { continue; }");
                g.ln("            metadata.insert(key.to_owned(), value.to_owned());");
                g.ln("        }");
                g.ln("    }");
                g.ln("    if metadata.is_empty() { None } else { Some(metadata) }");
                g.ln("};");
            }
            _ => unimplemented!(),
        }
        g.lf();
    }

    g.ln(f!("Ok({} {{", op.input));
    for field in &ty.fields {
        g.ln(f!("{},", field.name));
    }
    g.ln("})");
    g.ln("}");
}

fn codegen_op_http_call(op: &Operation, g: &mut Codegen) {
    g.ln("#[async_trait::async_trait]");
    g.ln(f!("impl super::Operation for {} {{", op.name));

    g.ln("fn name(&self) -> &'static str {");
    g.ln(f!("\"{}\"", op.name));
    g.ln("}");
    g.lf();

    g.ln("async fn call(&self, s3: &Arc<dyn S3>, req: &mut http::Request) -> S3Result<http::Response> {");

    let method = op.name.to_snake_case();

    g.ln("let input = Self::deserialize_http(req)?;");
    g.ln("let req = super::build_s3_request(input, req);");
    g.ln(f!("let result = s3.{method}(req).await;"));

    g.ln("let res = match result {");
    g.ln("Ok(output) => Self::serialize_http(output)?,");

    g.ln("Err(err) => super::serialize_error(err)?,");
    g.ln("};");

    g.ln("Ok(res)");

    g.ln("}");

    g.lf();
    g.ln(f!("fn as_any(&self) -> &dyn std::any::Any {{"));
    g.ln(f!("self"));
    g.ln(f!("}}"));

    g.ln("}");
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum PathPattern {
    Root,
    Bucket,
    Object,
}

impl PathPattern {
    fn parse(part: &str) -> Self {
        let path = match part.split_once('?') {
            None => part,
            Some((p, _)) => p,
        };

        assert!(path.starts_with('/'));

        if path == "/" {
            return Self::Root;
        }

        match path[1..].split_once('/') {
            None => Self::Bucket,
            Some(_) => Self::Object,
        }
    }

    fn query_tag(part: &str) -> Option<String> {
        let (_, q) = part.split_once('?')?;
        let qs: Vec<(String, String)> = serde_urlencoded::from_str(q).unwrap();
        assert!(qs.iter().filter(|(_, v)| v.is_empty()).count() <= 1);
        qs.into_iter().find(|(_, v)| v.is_empty()).map(|(n, _)| n)
    }

    fn query_patterns(part: &str) -> Vec<(String, String)> {
        let Some((_, q)) = part.split_once('?') else{ return Vec::new() };
        let mut qs: Vec<(String, String)> = serde_urlencoded::from_str(q).unwrap();
        qs.retain(|(n, v)| n != "x-id" && v.is_empty().not());
        qs
    }
}

struct Route<'a> {
    op: &'a Operation,
    query_tag: Option<String>,
    query_patterns: Vec<(String, String)>,
    required_headers: Vec<&'a str>,
    required_query_strings: Vec<&'a str>,
    needs_full_body: bool,
}

fn collect_routes<'a>(ops: &'a Operations, rust_types: &'a RustTypes) -> HashMap<String, HashMap<PathPattern, Vec<Route<'a>>>> {
    let mut ans: HashMap<String, HashMap<PathPattern, Vec<Route<'_>>>> = default();
    for op in ops.values() {
        let pat = PathPattern::parse(&op.http_uri);
        let map = ans.entry(op.http_method.clone()).or_default();
        let vec = map.entry(pat).or_default();

        vec.push(Route {
            op,
            query_tag: PathPattern::query_tag(&op.http_uri),
            query_patterns: PathPattern::query_patterns(&op.http_uri),

            required_headers: required_headers(op, rust_types),
            required_query_strings: required_query_strings(op, rust_types),

            needs_full_body: needs_full_body(op, rust_types),
        });
    }
    for map in ans.values_mut() {
        for vec in map.values_mut() {
            vec.sort_by_key(|r| {
                let has_qt = r.query_tag.is_some();
                let has_qp = r.query_patterns.is_empty().not();

                let priority = match (has_qt, has_qp) {
                    (true, true) => 1,
                    (true, false) => 2,
                    (false, true) => 3,
                    (false, false) => 4,
                };

                (
                    priority,
                    Reverse(r.query_patterns.len()),
                    Reverse(r.required_query_strings.len()),
                    Reverse(r.required_headers.len()),
                    r.op.name.as_str(),
                )
            });
        }
    }
    ans
}

fn required_headers<'a>(op: &Operation, rust_types: &'a RustTypes) -> Vec<&'a str> {
    let input_type = &rust_types[op.input.as_str()];
    let rust::Type::Struct(ty) = input_type else { panic!() };

    let mut ans: Vec<&'a str> = default();
    for field in &ty.fields {
        let is_required = field.option_type.not() && field.default_value.is_none();
        if is_required && field.position == "header" {
            let header = field.http_header.as_deref().unwrap();
            ans.push(header);
        }
    }
    ans
}

fn required_query_strings<'a>(op: &Operation, rust_types: &'a RustTypes) -> Vec<&'a str> {
    let input_type = &rust_types[op.input.as_str()];
    let rust::Type::Struct(ty) = input_type else { panic!() };

    let mut ans: Vec<&'a str> = default();
    for field in &ty.fields {
        let is_required = field.option_type.not() && field.default_value.is_none();
        if is_required && field.position == "query" {
            let header = field.http_query.as_deref().unwrap();
            ans.push(header);
        }
    }
    ans
}

fn needs_full_body(op: &Operation, rust_types: &RustTypes) -> bool {
    if op.http_method == "GET" {
        return false;
    }

    let rust::Type::Struct(ty) = &rust_types[op.input.as_str()] else { panic!() };
    assert!(ty.xml_name.is_none());

    let has_xml_payload = ty.fields.iter().any(is_xml_payload);
    let has_string_payload = ty.fields.iter().any(|field| field.type_ == "Policy");
    has_xml_payload || has_string_payload
}

fn codegen_router(ops: &Operations, rust_types: &RustTypes, g: &mut Codegen) {
    let routes = collect_routes(ops, rust_types);

    let methods = ["HEAD", "GET", "POST", "PUT", "DELETE"];
    assert_eq!(methods.len(), routes.keys().count());
    for method in routes.keys() {
        assert!(methods.contains(&method.as_str()));
    }

    g.ln("pub fn resolve_route(req: &http::Request, s3_path: &S3Path, qs: Option<&http::OrderedQs>) -> S3Result<(&'static dyn super::Operation, bool)> {");

    let succ = |route: &Route, g: &mut Codegen, return_: bool| {
        if return_ {
            g.ln(f!(
                "return Ok((&{} as &'static dyn super::Operation, {}));",
                route.op.name,
                route.needs_full_body
            ));
        } else {
            g.ln(f!("Ok((&{} as &'static dyn super::Operation, {}))", route.op.name, route.needs_full_body));
        }
    };

    g.ln("match req.method {");
    for &method in &methods {
        g.ln(f!("hyper::Method::{method} => match s3_path {{"));

        for pattern in [PathPattern::Root, PathPattern::Bucket, PathPattern::Object] {
            let s3_path_pattern = match pattern {
                PathPattern::Root => "S3Path::Root",
                PathPattern::Bucket => "S3Path::Bucket{ .. }",
                PathPattern::Object => "S3Path::Object{ .. }",
            };

            g.ln(f!("{s3_path_pattern} => {{"));
            match routes[method].get(&pattern) {
                None => g.ln("Err(super::unknown_operation())"),
                Some(group) => {
                    // NOTE: To debug the routing order, uncomment the lines below.
                    // {
                    //     println!("{} {:?}", method, pattern);
                    //     println!();
                    //     for route in group {
                    //         println!(
                    //             "{:<80} qt={:<30} qp={:<30}, qs={:?}, hs={:?}",
                    //             route.op.name,
                    //             f!("{:?}", route.query_tag.as_deref()),
                    //             f!("{:?}", route.query_patterns),
                    //             route.required_query_strings,
                    //             route.required_headers
                    //         );
                    //         println!("{}", route.op.http_uri);
                    //         println!();
                    //     }
                    //     println!("\n\n\n");
                    // }

                    assert!(group.is_empty().not());
                    if group.len() == 1 {
                        let route = &group[0];
                        assert!(route.query_tag.is_none());
                        assert!(route.required_headers.is_empty());
                        assert!(route.required_query_strings.is_empty());
                        assert!(route.needs_full_body.not());
                        succ(route, g, false);
                    } else {
                        let is_final_op = |route: &Route| {
                            route.required_headers.is_empty()
                                && route.required_query_strings.is_empty()
                                && route.query_patterns.is_empty()
                                && route.query_tag.is_none()
                        };
                        let final_count = group.iter().filter(|r| is_final_op(r)).count();
                        assert!(final_count <= 1);
                        if final_count == 1 {}

                        g.ln("if let Some(qs) = qs {");
                        for route in group {
                            let has_qt = route.query_tag.is_some();
                            let has_qp = route.query_patterns.is_empty().not();

                            let qp = route.query_patterns.as_slice();

                            if has_qt {
                                let tag = route.query_tag.as_deref().unwrap();
                                assert!(tag.as_bytes().iter().all(|&x| x == b'-' || x.is_ascii_alphabetic()), "{tag}");
                            }
                            if has_qp {
                                assert!(qp.len() <= 1);
                            }

                            match (has_qt, has_qp) {
                                (true, true) => {
                                    assert_eq!(route.op.name, "SelectObjectContent");

                                    let tag = route.query_tag.as_deref().unwrap();
                                    let (n, v) = qp.first().unwrap();

                                    g.ln(f!("if qs.has(\"{tag}\") && super::check_query_pattern(qs, \"{n}\",\"{v}\") {{"));
                                    succ(route, g, true);
                                    g.ln("}");
                                }
                                (true, false) => {
                                    let tag = route.query_tag.as_deref().unwrap();

                                    g.ln(f!("if qs.has(\"{tag}\") {{"));
                                    succ(route, g, true);
                                    g.ln("}");
                                }
                                (false, true) => {
                                    let (n, v) = qp.first().unwrap();
                                    g.ln(f!("if super::check_query_pattern(qs, \"{n}\",\"{v}\") {{"));
                                    succ(route, g, true);
                                    g.ln("}");
                                }
                                (false, false) => {}
                            }
                        }
                        g.ln("}");

                        for route in group {
                            let has_qt = route.query_tag.is_some();
                            let has_qp = route.query_patterns.is_empty().not();

                            if has_qt || has_qp {
                                continue;
                            }

                            let qs = route.required_query_strings.as_slice();
                            let hs = route.required_headers.as_slice();
                            assert!(qs.len() <= 1);
                            assert!(hs.len() <= 2);

                            if qs.is_empty() && hs.is_empty() {
                                continue;
                            }

                            let mut cond: String = default();
                            for q in qs {
                                cond.push_str(&f!("qs.has(\"{q}\")"));
                            }
                            for h in hs {
                                if cond.is_empty().not() {
                                    cond.push_str(" && ");
                                }
                                cond.push_str(&f!("req.headers.contains_key(\"{h}\")"));
                            }

                            if qs.is_empty().not() {
                                g.ln("if let Some(qs) = qs {");
                                g.ln(f!("if {cond} {{"));
                                succ(route, g, true);
                                g.ln("}");
                                g.ln("}");
                            } else {
                                g.ln(f!("if {cond} {{"));
                                succ(route, g, true);
                                g.ln("}");
                            }
                        }

                        if final_count == 1 {
                            let route = group.last().unwrap();
                            assert!(is_final_op(route));
                            succ(route, g, false);
                        } else {
                            g.ln("Err(super::unknown_operation())");
                        }
                    }
                }
            }
            g.ln("}");
        }

        g.ln("}");
    }
    g.ln("_ => Err(super::unknown_operation())");
    g.ln("}");

    g.ln("}");
}
