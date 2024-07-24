pub(crate) mod de {
    use std::collections::BTreeMap;
    use std::marker::PhantomData;

    use serde::de::{MapAccess, Visitor};
    use serde::{Deserialize, Deserializer};

    pub fn empty_string_as_none<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<Option<String>, D::Error> {
        let o: Option<String> = Option::deserialize(d)?;
        Ok(o.filter(|s| !s.is_empty()))
    }

    struct DefaultMapVisitor<K, V> {
        marker: PhantomData<fn() -> BTreeMap<K, V>>,
    }

    impl<K, V> DefaultMapVisitor<K, V> {
        fn new() -> Self {
            DefaultMapVisitor {
                marker: PhantomData,
            }
        }
    }

    impl<'de, K, V> Visitor<'de> for DefaultMapVisitor<K, V>
    where
        K: Deserialize<'de> + Ord,
        V: Deserialize<'de> + Default,
    {
        type Value = BTreeMap<K, V>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a map")
        }

        fn visit_map<M>(self, mut access: M) -> Result<Self::Value, M::Error>
        where
            M: MapAccess<'de>,
        {
            let mut map = BTreeMap::new();

            while let Some(key) = access.next_key()? {
                let value = access.next_value::<V>().unwrap_or_default();
                map.insert(key, value);
            }

            Ok(map)
        }
    }

    pub fn map_with_default_value<
        'de,
        K: Deserialize<'de> + Ord,
        V: Deserialize<'de> + Default,
        D: Deserializer<'de>,
    >(
        d: D,
    ) -> Result<BTreeMap<K, V>, D::Error> {
        d.deserialize_map(DefaultMapVisitor::new())
    }
}
