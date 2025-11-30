use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use semver::Version;
use serde::{Deserialize, Serialize};

use crate::diff::{CombinedDiff, Diff};

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
        let deser = toml::Deserializer::parse(&lock_content)?;
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

    pub fn remove_version(&mut self, world_name: &str, version: &Version) {
        let Some(world) = self.apworlds.get_mut(world_name) else {
            return;
        };
        world.remove(version);
    }

    pub fn apply_diff(&mut self, diff: &CombinedDiff) -> Result<()> {
        for (version_range, diff_type) in &diff.diffs {
            match diff_type {
                Diff::VersionAdded { checksum, .. } => {
                    // Version range is from..to, we want to add "to"
                    let version = version_range
                        .1
                        .as_ref()
                        .context("VersionAdded must have a target version")?;
                    self.set_checksum(&diff.apworld_name, version, checksum);
                }
                Diff::VersionRemoved => {
                    // Version range is from..to, we want to remove "from"
                    let version = version_range
                        .0
                        .as_ref()
                        .context("VersionRemoved must have a source version")?;
                    self.remove_version(&diff.apworld_name, version);
                }
            }
        }
        Ok(())
    }

    pub fn apply_diffs_from_dir(&mut self, diffs_dir: &Path) -> Result<()> {
        let entries = std::fs::read_dir(diffs_dir)
            .with_context(|| format!("Failed to read diffs directory: {}", diffs_dir.display()))?;

        for entry in entries {
            let entry = entry?;
            let path = entry.path();

            if path.extension().and_then(|s| s.to_str()) != Some("apdiff") {
                continue;
            }

            let content = std::fs::read_to_string(&path)?;
            let diff: CombinedDiff = serde_json::from_str(&content)?;
            self.apply_diff(&diff)
                .with_context(|| format!("Failed to apply diff from: {}", path.display()))?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::{CombinedDiff, Diff, VersionRange};
    use std::collections::BTreeMap;
    use std::str::FromStr;
    use tempfile::tempdir;

    #[test]
    fn test_apply_diff_version_added() -> Result<()> {
        let tmpdir = tempdir()?;
        let lock_path = tmpdir.path().join("index.lock");

        let mut lock = IndexLock::new(&lock_path)?;

        let diff = CombinedDiff {
            world_name: "Test World".to_string(),
            apworld_name: "test_world".to_string(),
            diffs: BTreeMap::from([(
                VersionRange(None, Some(Version::from_str("1.0.0")?)),
                Diff::VersionAdded {
                    content: "test diff".to_string(),
                    checksum: "abc123".to_string(),
                },
            )]),
        };

        lock.apply_diff(&diff)?;

        assert_eq!(
            lock.get_checksum("test_world", &Version::from_str("1.0.0")?),
            Some("abc123".to_string())
        );

        Ok(())
    }

    #[test]
    fn test_apply_diff_version_removed() -> Result<()> {
        let tmpdir = tempdir()?;
        let lock_path = tmpdir.path().join("index.lock");

        let mut lock = IndexLock::new(&lock_path)?;

        lock.set_checksum("test_world", &Version::from_str("1.0.0")?, "abc123");

        let diff = CombinedDiff {
            world_name: "Test World".to_string(),
            apworld_name: "test_world".to_string(),
            diffs: BTreeMap::from([(
                VersionRange(Some(Version::from_str("1.0.0")?), None),
                Diff::VersionRemoved,
            )]),
        };

        lock.apply_diff(&diff)?;

        assert_eq!(
            lock.get_checksum("test_world", &Version::from_str("1.0.0")?),
            None
        );

        Ok(())
    }

    #[test]
    fn test_apply_diff_multiple_versions() -> Result<()> {
        let tmpdir = tempdir()?;
        let lock_path = tmpdir.path().join("index.lock");

        let mut lock = IndexLock::new(&lock_path)?;

        let diff = CombinedDiff {
            world_name: "Test World".to_string(),
            apworld_name: "test_world".to_string(),
            diffs: BTreeMap::from([
                (
                    VersionRange(None, Some(Version::from_str("1.0.0")?)),
                    Diff::VersionAdded {
                        content: "diff 1".to_string(),
                        checksum: "abc123".to_string(),
                    },
                ),
                (
                    VersionRange(
                        Some(Version::from_str("1.0.0")?),
                        Some(Version::from_str("2.0.0")?),
                    ),
                    Diff::VersionAdded {
                        content: "diff 2".to_string(),
                        checksum: "def456".to_string(),
                    },
                ),
            ]),
        };

        lock.apply_diff(&diff)?;

        assert_eq!(
            lock.get_checksum("test_world", &Version::from_str("1.0.0")?),
            Some("abc123".to_string())
        );
        assert_eq!(
            lock.get_checksum("test_world", &Version::from_str("2.0.0")?),
            Some("def456".to_string())
        );

        Ok(())
    }

    #[test]
    fn test_apply_diffs_from_dir() -> Result<()> {
        let tmpdir = tempdir()?;
        let lock_path = tmpdir.path().join("index.lock");
        let diffs_dir = tmpdir.path().join("diffs");
        std::fs::create_dir(&diffs_dir)?;

        let mut lock = IndexLock::new(&lock_path)?;

        let diff1 = CombinedDiff {
            world_name: "World 1".to_string(),
            apworld_name: "world1".to_string(),
            diffs: BTreeMap::from([(
                VersionRange(None, Some(Version::from_str("1.0.0")?)),
                Diff::VersionAdded {
                    content: "diff 1".to_string(),
                    checksum: "checksum1".to_string(),
                },
            )]),
        };

        let diff2 = CombinedDiff {
            world_name: "World 2".to_string(),
            apworld_name: "world2".to_string(),
            diffs: BTreeMap::from([(
                VersionRange(None, Some(Version::from_str("2.0.0")?)),
                Diff::VersionAdded {
                    content: "diff 2".to_string(),
                    checksum: "checksum2".to_string(),
                },
            )]),
        };

        std::fs::write(
            diffs_dir.join("world1.apdiff"),
            serde_json::to_string(&diff1)?,
        )?;
        std::fs::write(
            diffs_dir.join("world2.apdiff"),
            serde_json::to_string(&diff2)?,
        )?;

        std::fs::write(diffs_dir.join("readme.txt"), "ignore me")?;

        lock.apply_diffs_from_dir(&diffs_dir)?;

        assert_eq!(
            lock.get_checksum("world1", &Version::from_str("1.0.0")?),
            Some("checksum1".to_string())
        );
        assert_eq!(
            lock.get_checksum("world2", &Version::from_str("2.0.0")?),
            Some("checksum2".to_string())
        );

        Ok(())
    }

    #[test]
    fn test_apply_diffs_write_and_read() -> Result<()> {
        let tmpdir = tempdir()?;
        let lock_path = tmpdir.path().join("index.lock");
        let diffs_dir = tmpdir.path().join("diffs");
        std::fs::create_dir(&diffs_dir)?;

        let mut lock = IndexLock::new(&lock_path)?;

        let diff = CombinedDiff {
            world_name: "Test".to_string(),
            apworld_name: "test".to_string(),
            diffs: BTreeMap::from([(
                VersionRange(None, Some(Version::from_str("1.0.0")?)),
                Diff::VersionAdded {
                    content: "test".to_string(),
                    checksum: "abc123".to_string(),
                },
            )]),
        };

        std::fs::write(
            diffs_dir.join("test.apdiff"),
            serde_json::to_string(&diff)?,
        )?;

        lock.apply_diffs_from_dir(&diffs_dir)?;
        lock.write()?;

        let lock2 = IndexLock::new(&lock_path)?;
        assert_eq!(
            lock2.get_checksum("test", &Version::from_str("1.0.0")?),
            Some("abc123".to_string())
        );

        Ok(())
    }
}
