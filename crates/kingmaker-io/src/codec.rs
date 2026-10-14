//! Codec-v2 decoder. Non-JSON JS types stay explicit, never coerced to zero/null.
use anyhow::{bail, Context, Result};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::borrow::Cow;

pub const CODEC_TAG: &str = "__kingmaker_codec_v2__";
const ESCAPED_PREFIX: &str = "__kingmaker_codec_v2__~";

#[derive(Debug, Default)]
pub struct CodecContext {
    backing: Option<Arc<dyn kingmaker_core::source_store::SourceRead>>,
    dictionary: Arc<HashMap<u64, Value>>,
    shared: Arc<HashMap<(String, u64), Value>>,
}
impl CodecContext {
    pub fn new(dictionary: HashMap<u64, Value>, shared: HashMap<(String, u64), Value>) -> Self {
        Self {
            backing: None,
            dictionary: Arc::new(dictionary),
            shared: Arc::new(shared),
        }
    }
    pub fn from_shared(
        dictionary: Arc<HashMap<u64, Value>>,
        shared: Arc<HashMap<(String, u64), Value>>,
    ) -> Self {
        Self { backing:None,dictionary, shared }
    }
    pub fn dictionary(&self) -> Arc<HashMap<u64, Value>> {
        self.dictionary.clone()
    }
    pub fn shared(&self) -> Arc<HashMap<(String, u64), Value>> {
        self.shared.clone()
    }
    pub fn from_source(source:&kingmaker_core::SourceSnapshot)->Self {
        Self {backing:source.backing.clone(),dictionary:source.dictionary.clone(),shared:source.shared.clone()}
    }
    pub fn resolve(&self,n:u64)->Result<Cow<'_,Value>> {
        let value=if let Some(value)=self.dictionary.get(&n) {Cow::Borrowed(value)} else {
            let backing=self.backing.as_ref().with_context(||format!("Unknown dictionary ID {n}"))?;
            Cow::Owned((*backing.dictionary_value(n)?.with_context(||format!("Unknown dictionary ID {n}"))?).clone())
        };
        if !value.is_string() && !value.is_number() {bail!("Invalid external dictionary ID {n}: expected string or finite number");}
        Ok(value)
    }
    fn shared_value(&self,key:&(String,u64))->Result<Cow<'_,Value>> {
        if let Some(value)=self.shared.get(key) {return Ok(Cow::Borrowed(value));}
        let backing=self.backing.as_ref().with_context(||format!("Unknown shared entry {key:?}"))?;
        Ok(Cow::Owned((*backing.shared_value(&key.0,key.1)?.with_context(||format!("Unknown shared entry {key:?}"))?).clone()))
    }
    pub fn dictionary_len(&self) -> usize {
        if self.dictionary.is_empty() {self.backing.as_ref().map_or(0,|b|b.reference_stats().dictionary)} else {self.dictionary.len()}
    }
    pub fn shared_len(&self) -> usize {
        if self.shared.is_empty() {self.backing.as_ref().map_or(0,|b|b.reference_stats().shared)} else {self.shared.len()}
    }
    pub fn validate_references(&self, envelope: &Value) -> Result<()> {
        validate_structure(envelope)?;
        if let Some(n) = envelope.get("id") {
            let n = n.as_u64().context("Invalid envelope own ordinal")?;
            let external = self.resolve(n)?;
            if envelope
                .get("external_id")
                .is_some_and(|value| value != external.as_ref())
            {
                bail!("Envelope own ordinal/external_id mismatch");
            }
        }
        fn walk(
            ctx: &CodecContext,
            value: &Value,
            depth: usize,
            active: &mut HashSet<(String, u64)>,
        ) -> Result<()> {
            if depth > 128 {
                bail!("Reference nesting limit");
            }
            if let Some(obj) = value.as_object() {
                if obj
                    .keys()
                    .all(|k| [CODEC_TAG, "value", "table", "index"].contains(&k.as_str()))
                {
                    if let Some(kind) = obj.get(CODEC_TAG).and_then(Value::as_str) {
                        if kind == "id" {
                            ctx.resolve(
                                obj.get("value")
                                    .and_then(Value::as_u64)
                                    .context("Invalid dictionary ordinal")?,
                            )?;
                            return Ok(());
                        }
                        if kind == "shared" {
                            let key = (
                                obj.get("table")
                                    .and_then(Value::as_str)
                                    .context("Missing shared table")?
                                    .to_string(),
                                obj.get("index")
                                    .and_then(Value::as_u64)
                                    .context("Invalid shared index")?,
                            );
                            if !active.insert(key.clone()) {
                                bail!("Cyclic shared reference {key:?}");
                            }
                            let shared = ctx.shared_value(&key)?;
                            walk(ctx, &shared, depth + 1, active)?;
                            active.remove(&key);
                            return Ok(());
                        }
                        if !["undefined", "bigint", "number", "date", "set", "map"].contains(&kind)
                        {
                            bail!("Unsupported codec marker {kind}");
                        }
                        // Validate the representation even if this table has no native phase yet.
                        ctx.decode_value(value, "", &HashSet::new(), &mut HashSet::new(), depth)?;
                        return Ok(());
                    }
                }
                for child in obj.values() {
                    walk(ctx, child, depth + 1, active)?;
                }
            } else if let Some(array) = value.as_array() {
                for child in array {
                    walk(ctx, child, depth + 1, active)?;
                }
            }
            Ok(())
        }
        walk(self, envelope, 0, &mut HashSet::new())
    }
    pub fn decode_record(&self, envelope: &Value) -> Result<Value> {
        let bigint_fields = validate_structure(envelope)?;
        let table = envelope["table"].as_str().context("Missing record table")?;
        let mut active_shared = HashSet::new();
        let raw = envelope.get("value").context("Missing envelope value")?;
        let mut decoded = self
            .decode_value(raw, "", &bigint_fields, &mut active_shared, 0)
            .with_context(|| format!("Decode {table}"))?;
        if let (Some(out), Some(extensions)) = (
            decoded.as_object_mut(),
            envelope.get("extensions").and_then(Value::as_object),
        ) {
            for (encoded_key, value) in extensions {
                let key = decode_key(encoded_key);
                if !out.contains_key(key) {
                    out.insert(
                        key.to_string(),
                        self.decode_value(
                            value,
                            &escape_path(key),
                            &bigint_fields,
                            &mut active_shared,
                            0,
                        )?,
                    );
                }
            }
        }
        Ok(decoded)
    }
    fn decode_value(
        &self,
        value: &Value,
        path: &str,
        bigint_fields: &HashSet<String>,
        active: &mut HashSet<(String, u64)>,
        depth: usize,
    ) -> Result<Value> {
        if depth > 128 {
            bail!("Codec nesting limit at {path}");
        }
        if bigint_fields.contains(path) {
            if let Some(text) = value.as_str().filter(|s| decimal_integer(s)) {
                return Ok(json!({CODEC_TAG:"bigint","value":text}));
            }
        }
        if let Some(obj) = value.as_object() {
            let is_marker = obj
                .keys()
                .all(|k| [CODEC_TAG, "value", "table", "index"].contains(&k.as_str()));
            if is_marker {
                if let Some(kind) = obj.get(CODEC_TAG).and_then(Value::as_str) {
                    return match kind {
                        "id" => self
                            .resolve(
                                obj.get("value")
                                    .and_then(Value::as_u64)
                                    .context("Invalid dictionary ordinal")?,
                            )
                            .map(Cow::into_owned),
                        "shared" => {
                            let key = (
                                obj.get("table")
                                    .and_then(Value::as_str)
                                    .context("Missing shared kind")?
                                    .to_string(),
                                obj.get("index")
                                    .and_then(Value::as_u64)
                                    .context("Invalid shared ordinal")?,
                            );
                            if !active.insert(key.clone()) {
                                bail!("Cyclic shared reference {key:?}");
                            }
                            let shared = self.shared_value(&key)?;
                            let result =
                                self.decode_value(&shared, path, bigint_fields, active, depth + 1);
                            active.remove(&key);
                            result
                        }
                        "undefined" => Ok(json!({CODEC_TAG:"undefined"})),
                        "bigint" => {
                            let text = obj
                                .get("value")
                                .and_then(Value::as_str)
                                .filter(|s| decimal_integer(s))
                                .context("Invalid encoded bigint")?;
                            Ok(json!({CODEC_TAG:"bigint","value":text}))
                        }
                        "number" => {
                            let text = obj
                                .get("value")
                                .and_then(Value::as_str)
                                .context("Missing special number")?;
                            if !["-0", "NaN", "Infinity", "-Infinity"].contains(&text) {
                                bail!("Unsupported encoded number {text}");
                            }
                            Ok(json!({CODEC_TAG:"number","value":text}))
                        }
                        "date" => {
                            let text = obj
                                .get("value")
                                .and_then(Value::as_str)
                                .context("Missing encoded date")?;
                            Ok(json!({CODEC_TAG:"date","value":text}))
                        }
                        "set" | "map" => {
                            let items = obj
                                .get("value")
                                .and_then(Value::as_array)
                                .context("Invalid set/map array")?;
                            let mut out = Vec::with_capacity(items.len());
                            for (i, item) in items.iter().enumerate() {
                                let at = child_path(path, &i.to_string());
                                if kind == "map" {
                                    let pair = item
                                        .as_array()
                                        .filter(|p| p.len() == 2)
                                        .context("Invalid map pair")?;
                                    out.push(Value::Array(vec![
                                        self.decode_value(
                                            &pair[0],
                                            &child_path(&at, "0"),
                                            bigint_fields,
                                            active,
                                            depth + 1,
                                        )?,
                                        self.decode_value(
                                            &pair[1],
                                            &child_path(&at, "1"),
                                            bigint_fields,
                                            active,
                                            depth + 1,
                                        )?,
                                    ]));
                                } else {
                                    out.push(self.decode_value(
                                        item,
                                        &at,
                                        bigint_fields,
                                        active,
                                        depth + 1,
                                    )?);
                                }
                            }
                            Ok(json!({CODEC_TAG:kind,"value":out}))
                        }
                        _ => bail!("Unsupported codec marker {kind} at {path}"),
                    };
                }
            }
            let mut out = Map::new();
            for (encoded_key, child) in obj {
                let key = decode_key(encoded_key);
                out.insert(
                    key.to_string(),
                    self.decode_value(
                        child,
                        &child_path(path, key),
                        bigint_fields,
                        active,
                        depth + 1,
                    )?,
                );
            }
            return Ok(Value::Object(out));
        }
        if let Some(items) = value.as_array() {
            return items
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    self.decode_value(
                        v,
                        &child_path(path, &i.to_string()),
                        bigint_fields,
                        active,
                        depth + 1,
                    )
                })
                .collect();
        }
        Ok(value.clone())
    }
}
fn validate_structure(envelope: &Value) -> Result<HashSet<String>> {
    if envelope["storage_schema_version"].as_u64() != Some(2) {
        bail!("Unsupported state record schema");
    }
    envelope["table"].as_str().context("Missing record table")?;
    envelope.get("value").context("Missing envelope value")?;
    envelope
        .get("extensions")
        .and_then(Value::as_object)
        .context("Missing extensions object")?;
    let fields = envelope
        .get("bigint_fields")
        .and_then(Value::as_array)
        .context("Missing bigint_fields array")?;
    fields
        .iter()
        .map(|field| match field {
            Value::String(s) => Ok(s.clone()),
            Value::Array(parts) => parts
                .iter()
                .map(|p| match p {
                    Value::String(s) => Ok(escape_path(s)),
                    Value::Number(n) => Ok(escape_path(&n.to_string())),
                    _ => bail!("Invalid bigint path"),
                })
                .collect::<Result<Vec<_>>>()
                .map(|p| p.join("/")),
            _ => bail!("Invalid bigint path"),
        })
        .collect()
}
fn decimal_integer(text: &str) -> bool {
    let digits = text.strip_prefix('-').unwrap_or(text);
    !digits.is_empty()
        && (digits == "0" || !digits.starts_with('0'))
        && digits.bytes().all(|b| b.is_ascii_digit())
}
fn escape_path(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}
fn child_path(parent: &str, key: &str) -> String {
    let key = escape_path(key);
    if parent.is_empty() {
        key
    } else {
        format!("{parent}/{key}")
    }
}
fn decode_key(key: &str) -> &str {
    key.strip_prefix(ESCAPED_PREFIX).unwrap_or(key)
}
