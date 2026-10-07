//! Byte-compatible with the retained Node outbound encoder and Hagency's
//! `native/hagency-core/src/canonical.rs`: UTF-16 key ordering followed by
//! JavaScript array-index ordering. Authority DTOs reject inexact numbers.
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{Error, MAX_EXACT_JSON_INTEGER};

pub fn encode(value: &Value) -> Result<String, Error> {
    let mut out = String::new();
    write(value, &mut out, 0, false)?;
    Ok(out)
}
pub fn encode_transport(value: &Value) -> Result<String, Error> {
    let mut out = String::new();
    write(value, &mut out, 0, true)?;
    Ok(out)
}
pub fn digest(value: &Value) -> Result<String, Error> {
    Ok(hash(encode(value)?.as_bytes()))
}
pub fn transport_digest(value: &Value) -> Result<String, Error> {
    Ok(hash(encode_transport(value)?.as_bytes()))
}
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn index(key: &str) -> Option<u32> {
    let n = key.parse::<u32>().ok()?;
    (n < u32::MAX && n.to_string() == key).then_some(n)
}
fn string(s: &str) -> Result<String, Error> {
    serde_json::to_string(s).map_err(|_| Error::InvalidEncoding)
}
fn write(value: &Value, out: &mut String, depth: usize, transport: bool) -> Result<(), Error> {
    if depth > 64 {
        return Err(Error::InvalidEncoding);
    }
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::String(s) => out.push_str(&string(s)?),
        Value::Number(n) => {
            if transport {
                let number = n
                    .as_f64()
                    .filter(|n| n.is_finite())
                    .ok_or(Error::InvalidNumber)?;
                out.push_str(ryu_js::Buffer::new().format_finite(number));
            } else if let Some(n) = n
                .as_i64()
                .filter(|n| n.unsigned_abs() <= MAX_EXACT_JSON_INTEGER)
            {
                out.push_str(&n.to_string());
            } else if let Some(n) = n
                .as_f64()
                .filter(|n| n.fract() == 0.0 && n.abs() <= MAX_EXACT_JSON_INTEGER as f64)
            {
                out.push_str(&(n as i64).to_string());
            } else {
                return Err(Error::InvalidNumber);
            }
        }
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write(item, out, depth + 1, transport)?;
            }
            out.push(']');
        }
        Value::Object(items) => {
            if !transport && items.contains_key("__proto__") {
                return Err(Error::InvalidEncoding);
            }
            let mut keys = items.keys().collect::<Vec<_>>();
            keys.sort_by(|a, b| match (index(a), index(b)) {
                (Some(a), Some(b)) => a.cmp(&b),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => a.encode_utf16().cmp(b.encode_utf16()),
            });
            out.push('{');
            for (i, key) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&string(key)?);
                out.push(':');
                write(&items[key], out, depth + 1, transport)?;
            }
            out.push('}');
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn javascript_order_and_finite_transport_numbers_are_preserved() {
        let v: Value =
            serde_json::from_str(r#"{"10":1,"2":2,"01":3,"":4,"😀":5,"n":1e-7,"z":-0.0}"#)
                .unwrap();
        assert_eq!(
            encode_transport(&v).unwrap(),
            "{\"2\":2,\"10\":1,\"01\":3,\"n\":1e-7,\"z\":0,\"😀\":5,\"\":4}"
        );
        assert!(encode(&v).is_err());
        for raw in [
            "9007199254740992",
            "-9007199254740992",
            r#"{"__proto__":{}}"#,
        ] {
            assert!(encode(&serde_json::from_str(raw).unwrap()).is_err());
        }
        assert_eq!(encode(&serde_json::from_str("-0.0").unwrap()).unwrap(), "0");
    }
}
