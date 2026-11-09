use serde::{
    Deserialize, Deserializer, Serialize,
    de::{self, DeserializeOwned, MapAccess, SeqAccess, Visitor},
};
use serde_json::{Map, Number, Value};
use std::fmt;

/// serde_json::Value alone silently accepts duplicate map keys. Persisted data
/// must not contain ambiguous fields, including inside native Runtime payloads.
struct UniqueValue(Value);
impl<'de> Deserialize<'de> for UniqueValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct UniqueVisitor;
        impl<'de> Visitor<'de> for UniqueVisitor {
            type Value = UniqueValue;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("unambiguous JSON")
            }
            fn visit_bool<E: de::Error>(self, v: bool) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::Bool(v)))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::Number(v.into())))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::Number(v.into())))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<Self::Value, E> {
                Number::from_f64(v)
                    .map(|v| UniqueValue(Value::Number(v)))
                    .ok_or_else(|| E::custom("non-finite JSON number"))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::String(v.to_owned())))
            }
            fn visit_string<E: de::Error>(self, v: String) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::String(v)))
            }
            fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::Null))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::Null))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut out = Vec::new();
                while let Some(UniqueValue(v)) = seq.next_element()? {
                    out.push(v);
                }
                Ok(UniqueValue(Value::Array(out)))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut out = Map::new();
                while let Some(key) = map.next_key::<String>()? {
                    if out.contains_key(&key) {
                        return Err(de::Error::custom("duplicate JSON field"));
                    }
                    let UniqueValue(v) = map.next_value()?;
                    out.insert(key, v);
                }
                Ok(UniqueValue(Value::Object(out)))
            }
        }
        deserializer.deserialize_any(UniqueVisitor)
    }
}

pub(crate) fn value(bytes: &[u8]) -> Result<Value, String> {
    serde_json::from_slice::<UniqueValue>(bytes)
        .map(|v| v.0)
        .map_err(|e| e.to_string())
}

pub(crate) fn decode<T: DeserializeOwned + Serialize>(original: Value) -> Result<T, String> {
    let decoded: T = serde_json::from_value(original.clone()).map_err(|e| e.to_string())?;
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
