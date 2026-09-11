use serde::{
    Deserializer,
    de::{self, SeqAccess},
};

/// Deserializes game category lists like [ {"path": "games"} ]
pub fn deserialize_path_list<'de, D>(de: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    struct Visitor;

    impl<'de> de::Visitor<'de> for Visitor {
        type Value = Vec<String>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("an array of { path: String } maps")
        }

        fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            let mut res: Vec<String> = Vec::new();

            while let Some(SingleValueWrapper(val)) =
                seq.next_element::<SingleValueWrapper<String>>()?
            {
                res.push(val);
            }

            Ok(res)
        }
    }

    de.deserialize_seq(Visitor)
}

use serde::Deserialize;
use serde::de::{IgnoredAny, MapAccess, Visitor};
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
