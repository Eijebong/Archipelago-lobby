use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use anyhow::Result;
use semver::Version;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Debug, Default)]
pub struct IndexLock {
    pub path: PathBuf,
    // World name -> (Version -> Checksum)
    apworlds: BTreeMap<String, BTreeMap<Version, String>>,
}

impl IndexLock {
    pub fn new(path: &Path) -> Result<Self> {
        if !path.is_file() {
            return Ok(Self {
                path: path.into(),
                ..Default::default()
            });
        }

        let lock_content = std::fs::read_to_string(path)?;
        let deser = toml::Deserializer::new(&lock_content);
        let apworlds: BTreeMap<String, BTreeMap<Version, String>> =
            serde_path_to_error::deserialize(deser)?;

        Ok(Self {
            path: path.into(),
            apworlds,
        })
    }

    pub fn write(&self) -> Result<()> {
        let content = toml::to_string(&self.apworlds)?;
        std::fs::write(&self.path, content)?;

        Ok(())
    }

    pub fn contains(&self, world_name: &str, version: &Version) -> bool {
        let Some(world) = self.apworlds.get(world_name) else {
            return false;
        };
        world.get(version).is_some()
    }

    pub fn set_checksum(&mut self, world_name: &str, version: &Version, checksum: &str) {
        let world = self.apworlds.entry(world_name.to_string()).or_default();
        world.insert(version.clone(), checksum.to_string());
    }

    pub fn get_checksum(&self, world_name: &str, version: &Version) -> Option<String> {
        let world = self.apworlds.get(world_name)?;
        world.get(version).cloned()
    }
}
