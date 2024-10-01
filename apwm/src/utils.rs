use anyhow::Result;
use git2::{FetchOptions, Repository, ResetType};
use std::path::Path;

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

pub fn git_clone_shallow(url: &str, git_ref: &str, path: &Path) -> Result<()> {
    let repo = Repository::init(path)?;

    let mut remote = repo.remote("origin", url)?;
    let mut fetch_options = FetchOptions::new();
    fetch_options.depth(1);

    remote.fetch(&[git_ref], Some(&mut fetch_options), None)?;

    let fetch_head = repo.find_reference("FETCH_HEAD")?;
    repo.reset(
        &fetch_head.peel(git2::ObjectType::Commit)?,
        ResetType::Hard,
        None,
    )?;

    Ok(())
}

/// Copy the content of a directory `src` into `dst`. `dst` must be a directory.
pub(crate) fn copy_dir_all(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &dst.join(entry.file_name()))?;
        } else {
            std::fs::copy(entry.path(), dst.join(entry.file_name()))?;
        }
    }
    Ok(())
}
