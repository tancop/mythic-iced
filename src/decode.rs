use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use serde::Deserializer;

const B64_ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn b64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(B64_ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(B64_ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            B64_ALPHABET[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            B64_ALPHABET[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

fn b64_value(byte: u8) -> Option<u32> {
    match byte {
        b'A'..=b'Z' => Some((byte - b'A') as u32),
        b'a'..=b'z' => Some((byte - b'a' + 26) as u32),
        b'0'..=b'9' => Some((byte - b'0' + 52) as u32),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

fn b64_decode(s: &str) -> Option<Vec<u8>> {
    let bytes = s.as_bytes();
    if bytes.is_empty() || !bytes.len().is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for chunk in bytes.chunks(4) {
        let pad = chunk.iter().rev().take_while(|&&b| b == b'=').count();
        if pad > 2 {
            return None;
        }
        let mut n = 0u32;
        for (i, &b) in chunk.iter().enumerate() {
            if b == b'=' {
                if i < 4 - pad {
                    return None;
                }
            } else {
                if i >= 4 - pad {
                    return None;
                }
                n |= b64_value(b)? << (18 - 6 * i);
            }
        }
        out.push((n >> 16) as u8);
        if pad < 2 {
            out.push((n >> 8) as u8);
        }
        if pad < 1 {
            out.push(n as u8);
        }
    }
    Some(out)
}

/// Encode a library page cursor (`{"offset":N}` as standard base64) so pages
/// can be requested in parallel without waiting for the previous page's
/// `nextCursor`.
pub fn encode_offset_cursor(offset: u64) -> String {
    b64_encode(format!("{{\"offset\":{offset}}}").as_bytes())
}

/// Decode a library page cursor back to its offset; `None` when the cursor
/// has an unexpected shape (caller should fall back to sequential paging).
pub fn decode_offset_cursor(cursor: &str) -> Option<u64> {
    let bytes = b64_decode(cursor.trim())?;
    let text = str::from_utf8(&bytes).ok()?;
    let inner = text
        .trim()
        .strip_prefix("{\"offset\":")?
        .strip_suffix('}')?;
    inner.trim().parse().ok()
}

/// Fallback for required datetime fields when the value is missing, null,
/// or unparseable: the game still loads, it just sorts as the oldest item.
pub fn default_epoch() -> DateTime<Utc> {
    DateTime::default()
}

/// Parse the datetime shapes Epic has been observed to return. Plain
/// RFC 3339 first, then space-separated / offset-less / date-only variants.
pub fn parse_datetime_lenient(s: &str) -> Option<DateTime<Utc>> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if let Ok(dt) = s.parse::<DateTime<Utc>>() {
        return Some(dt);
    }
    for fmt in [
        "%Y-%m-%dT%H:%M:%S%.f%z",
        "%Y-%m-%dT%H:%M:%S%z",
        "%Y-%m-%d %H:%M:%S%.f%z",
        "%Y-%m-%d %H:%M:%S%z",
    ] {
        if let Ok(dt) = DateTime::parse_from_str(s, fmt) {
            return Some(dt.with_timezone(&Utc));
        }
    }
    for fmt in [
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S",
    ] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(s, fmt) {
            return Some(naive.and_utc());
        }
    }
    if let Ok(date) = NaiveDate::parse_from_str(s, "%Y-%m-%d")
        && let Some(naive) = date.and_hms_opt(0, 0, 0)
    {
        return Some(naive.and_utc());
    }
    None
}

/// Required datetime: null/missing/garbage falls back to the epoch (with a
/// warning) instead of failing the whole page of library records.
pub fn deserialize_datetime<'de, D>(de: D) -> Result<DateTime<Utc>, D::Error>
where
    D: Deserializer<'de>,
{
    let opt = Option::<String>::deserialize(de)?;
    match opt.as_deref().map(parse_datetime_lenient) {
        Some(Some(dt)) => Ok(dt),
        Some(None) => {
            log::warn!(
                "unparseable datetime {:?} falling back to epoch",
                opt.as_deref().unwrap_or("<missing>")
            );
            Ok(default_epoch())
        }
        None => Ok(default_epoch()),
    }
}

/// Optional datetime: null/missing/garbage becomes `None`.
pub fn deserialize_optional_datetime<'de, D>(de: D) -> Result<Option<DateTime<Utc>>, D::Error>
where
    D: Deserializer<'de>,
{
    let opt = Option::<String>::deserialize(de)?;
    match opt {
        None => Ok(None),
        Some(s) => match parse_datetime_lenient(&s) {
            Some(dt) => Ok(Some(dt)),
            None => {
                log::warn!("unparseable datetime {s:?}, treating as missing");
                Ok(None)
            }
        },
    }
}

/// Deserializes game category lists like [ {"path": "games"} ]; null becomes
/// an empty list instead of failing the enclosing record.
pub fn deserialize_path_list<'de, D>(de: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let opt = Option::<Vec<SingleValueWrapper<String>>>::deserialize(de)?;
    Ok(opt.unwrap_or_default().into_iter().map(|w| w.0).collect())
}

use serde::Deserialize;
use serde::de::{self, IgnoredAny, MapAccess, Visitor};
use std::fmt;
use std::marker::PhantomData;

/// Decodes a single value nested in a map (JSON object)
#[derive(Debug)]
pub struct SingleValueWrapper<T>(pub T);

impl<'de, T> Deserialize<'de> for SingleValueWrapper<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct WrapperVisitor<T>(PhantomData<T>);

        impl<'de, T> Visitor<'de> for WrapperVisitor<T>
        where
            T: Deserialize<'de>,
        {
            type Value = SingleValueWrapper<T>;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("an object with exactly one key-value pair")
            }

            fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                let value = match map.next_entry::<IgnoredAny, T>()? {
                    Some((_, val)) => val,
                    None => return Err(de::Error::custom("expected one key-value pair")),
                };

                if map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {
                    return Err(de::Error::custom("expected exactly one key-value pair"));
                }

                Ok(SingleValueWrapper(value))
            }
        }

        deserializer.deserialize_map(WrapperVisitor(PhantomData))
    }
}
