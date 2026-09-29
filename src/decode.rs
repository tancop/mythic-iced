use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use serde::Deserializer;

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
