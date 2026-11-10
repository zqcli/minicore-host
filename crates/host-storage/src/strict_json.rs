use serde::{
    Deserialize, Deserializer, Serialize,
    de::{self, DeserializeOwned, MapAccess, Visitor},
};
use serde_json::{Map, Value, value::RawValue};
use std::fmt;

// Parse each raw subtree without first converting numeric tokens to f64. This
// also avoids interpreting an actual object key as serde_json's private numeric
// transport marker. Duplicate fields are rejected before any map can overwrite.
// Raw subtrees borrow the input; only the resulting Value owns decoded content.
struct ObjectFields<'a>(Vec<(String, &'a RawValue)>);
impl<'de> Deserialize<'de> for ObjectFields<'de> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct FieldsVisitor;
        impl<'de> Visitor<'de> for FieldsVisitor {
            type Value = ObjectFields<'de>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an object without duplicate fields")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut names = std::collections::BTreeSet::new();
                let mut fields = Vec::new();
                while let Some(name) = map.next_key::<String>()? {
                    if !names.insert(name.clone()) {
                        return Err(de::Error::custom("duplicate JSON field"));
                    }
                    fields.push((name, map.next_value::<&RawValue>()?));
                }
                Ok(ObjectFields(fields))
            }
        }
        deserializer.deserialize_map(FieldsVisitor)
    }
}
pub(crate) fn value(bytes: &[u8]) -> Result<Value, String> {
    let raw: &RawValue = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    parse_raw(raw)
}
fn parse_raw(raw: &RawValue) -> Result<Value, String> {
    let source = raw.get().trim();
    match source.as_bytes().first() {
        Some(b'{') => {
            let ObjectFields(fields) = serde_json::from_str(source).map_err(|e| e.to_string())?;
            let mut object = Map::new();
            for (name, value) in fields {
                object.insert(name, parse_raw(value)?);
            }
            Ok(Value::Object(object))
        }
        Some(b'[') => {
            let items: Vec<&RawValue> = serde_json::from_str(source).map_err(|e| e.to_string())?;
            items
                .into_iter()
                .map(parse_raw)
                .collect::<Result<Vec<_>, _>>()
                .map(Value::Array)
        }
        _ => serde_json::from_str(source).map_err(|e| e.to_string()),
    }
}

pub(crate) fn decode<T: DeserializeOwned + Serialize>(original: Value) -> Result<T, String> {
    // Retain the original for preservation checks without an intermediate deep
    // clone. Native owned fields still get their own decoded content.
    let decoded = T::deserialize(&original).map_err(|e| e.to_string())?;
    let encoded = serde_json::to_value(&decoded).map_err(|e| e.to_string())?;
    preserve(&original, &encoded)?;
    Ok(decoded)
}

// A native type may legitimately add defaulted fields, but must not discard
// unknown fields or normalize the saved content during history loading.
fn preserve(original: &Value, encoded: &Value) -> Result<(), String> {
    match (original, encoded) {
        (Value::Object(a), Value::Object(b)) => {
            for (key, value) in a {
                preserve(
                    value,
                    b.get(key)
                        .ok_or_else(|| format!("unsupported or discarded field: {key}"))?,
                )?;
            }
        }
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => {
            for (a, b) in a.iter().zip(b) {
                preserve(a, b)?;
            }
        }
        (a, b) if a == b => {}
        _ => return Err("decoding would normalize or discard stored content".to_owned()),
    }
    Ok(())
}
