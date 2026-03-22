use std::{collections::BTreeMap, path::Path};

use anyhow::{Context, Result};
use semver::Version;
use serde::{Deserialize, Serialize};

use crate::Index;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Checksum {
    Supported,
    Hash(String),
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Changes {
    pub worlds: BTreeMap<String, WorldChanges>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct WorldChanges {
    pub world_name: String,
    pub added_versions: Vec<Version>,
    pub removed_versions: Vec<Version>,
    pub checksums: BTreeMap<Version, Checksum>,
}

pub fn compute_changes(old_index: &Index, new_index: &Index) -> Changes {
    let mut worlds = BTreeMap::new();

    for (name, new_world) in &new_index.worlds {
        match old_index.worlds.get(name) {
            None => {
                let added_versions: Vec<Version> = new_world.versions.keys().cloned().collect();
                worlds.insert(
                    name.clone(),
                    WorldChanges {
                        world_name: new_world.name.clone(),
                        added_versions,
                        removed_versions: vec![],
                        checksums: BTreeMap::new(),
                    },
                );
            }
            Some(old_world) => {
                let added: Vec<Version> = new_world
                    .versions
                    .keys()
                    .filter(|v| !old_world.versions.contains_key(*v))
                    .cloned()
                    .collect();
                let removed: Vec<Version> = old_world
                    .versions
                    .keys()
                    .filter(|v| !new_world.versions.contains_key(*v))
                    .cloned()
                    .collect();

                if added.is_empty() && removed.is_empty() {
                    continue;
                }

                worlds.insert(
                    name.clone(),
                    WorldChanges {
                        world_name: new_world.name.clone(),
                        added_versions: added,
                        removed_versions: removed,
                        checksums: BTreeMap::new(),
                    },
                );
            }
        }
    }

    for (name, old_world) in &old_index.worlds {
        if !new_index.worlds.contains_key(name) {
            let removed_versions: Vec<Version> = old_world.versions.keys().cloned().collect();
            worlds.insert(
                name.clone(),
                WorldChanges {
                    world_name: old_world.name.clone(),
                    added_versions: vec![],
                    removed_versions,
                    checksums: BTreeMap::new(),
                },
            );
        }
    }

    Changes { worlds }
}

pub async fn download_changed_apworlds(
    changes: &Changes,
    index: &Index,
    output_dir: &Path,
) -> Result<BTreeMap<String, BTreeMap<Version, Checksum>>> {
    let apworlds_dir = output_dir.join("apworlds");
    std::fs::create_dir_all(&apworlds_dir)?;

    let mut checksums: BTreeMap<String, BTreeMap<Version, Checksum>> = BTreeMap::new();

    for (apworld_name, world_changes) in &changes.worlds {
        let world = index.worlds.get(apworld_name).with_context(|| {
            format!("World {apworld_name} listed in changes but not found in index")
        })?;

        for version in &world_changes.added_versions {
            let origin = world.versions.get(version).with_context(|| {
                format!(
                    "Version {} not found in world {} index definition",
                    version, apworld_name
                )
            })?;

            if origin.is_supported() {
                checksums
                    .entry(apworld_name.clone())
                    .or_default()
                    .insert(version.clone(), Checksum::Supported);
                continue;
            }

            let dest_path = apworlds_dir.join(format!("{apworld_name}-{version}.apworld"));
            let dest_file = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&dest_path)?;

            let hash = world.copy_to(version, &dest_file, None).await?;
            checksums
                .entry(apworld_name.clone())
                .or_default()
                .insert(version.clone(), Checksum::Hash(hash));
        }
    }

    Ok(checksums)
}

pub fn apply_checksums(
    changes: &mut Changes,
    checksums: BTreeMap<String, BTreeMap<Version, Checksum>>,
) {
    for (apworld_name, version_checksums) in checksums {
        if let Some(wc) = changes.worlds.get_mut(&apworld_name) {
            wc.checksums.extend(version_checksums);
        }
    }
}

pub async fn download_from_changes(
    changes: &Changes,
    index: &Index,
    destination: &Path,
) -> Result<()> {
    std::fs::create_dir_all(destination)?;

    for (apworld_name, world_changes) in &changes.worlds {
        let world = index.worlds.get(apworld_name).with_context(|| {
            format!("World {apworld_name} listed in changes but not found in index")
        })?;

        for version in &world_changes.added_versions {
            let checksum = world_changes.checksums.get(version).with_context(|| {
                format!("No checksum in changes.json for {apworld_name} version {version}")
            })?;

            let expected_hash = match checksum {
                Checksum::Supported => continue,
                Checksum::Hash(h) => h,
            };

            let origin = world.versions.get(version).with_context(|| {
                format!("Version {version} not found in world {apworld_name} index definition")
            })?;
            if origin.is_supported() {
                continue;
            }

            let dest_path = destination.join(format!("{apworld_name}-{version}.apworld"));
            let dest_file = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&dest_path)?;

            // copy_to validates the checksum internally and returns an error on mismatch
            world
                .copy_to(version, &dest_file, Some(expected_hash.clone()))
                .await?;
        }
    }

    Ok(())
}

pub fn write_changes(changes: &Changes, output_dir: &Path) -> Result<()> {
    let path = output_dir.join("changes.json");
    let content = serde_json::to_string_pretty(changes)?;
    std::fs::write(path, content)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{VersionReq, World, WorldOrigin};
    use semver::Version;
    use std::collections::BTreeMap;
    use std::fs::OpenOptions;
    use std::io::Write;
    use std::str::FromStr;
    use tempfile::{tempdir, TempDir};
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    fn mock_world_versions(
        versions: &[&str],
    ) -> Result<(TempDir, BTreeMap<Version, WorldOrigin>)> {
        let mut result = BTreeMap::new();
        let tmpdir = tempdir()?;

        for version in versions {
            let path = tmpdir.path().join(version);
            let apworld_file = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&path)?;
            let mut archive = ZipWriter::new(&apworld_file);
            archive.start_file("VERSION", SimpleFileOptions::default())?;
            archive.write_all(version.as_bytes())?;
            archive.finish()?;
            result.insert(Version::from_str(version)?, WorldOrigin::Local(path));
        }
        Ok((tmpdir, result))
    }

    fn mock_world(name: &str, versions: BTreeMap<Version, WorldOrigin>) -> World {
        World {
            path: "/tmp/mock.toml".into(),
            name: name.into(),
            display_name: name.into(),
            default_url: None,
            default_version: VersionReq::Latest,
            versions,
            home: None,
            disabled: false,
            supported: false,
            tags: vec![],
        }
    }

    fn empty_index() -> Index {
        Index {
            path: "/tmp".into(),
            archipelago_repo: "https://example.com".parse().unwrap(),
            archipelago_version: Version::from_str("0.6.0").unwrap(),
            index_homepage: String::new(),
            index_dir: "index".into(),
            worlds: BTreeMap::new(),
        }
    }

    #[test]
    fn test_compute_changes_world_added() -> Result<()> {
        let old = empty_index();
        let mut new = empty_index();

        let (_tmp, versions) = mock_world_versions(&["0.1.0", "0.2.0"])?;
        new.worlds
            .insert("test".into(), mock_world("Test World", versions));

        let changes = compute_changes(&old, &new);

        assert_eq!(changes.worlds.len(), 1);
        let wc = &changes.worlds["test"];
        assert_eq!(wc.world_name, "Test World");
        assert_eq!(wc.added_versions.len(), 2);
        assert!(wc.removed_versions.is_empty());
        Ok(())
    }

    #[test]
    fn test_compute_changes_world_removed() -> Result<()> {
        let mut old = empty_index();
        let new = empty_index();

        let (_tmp, versions) = mock_world_versions(&["0.1.0"])?;
        old.worlds
            .insert("test".into(), mock_world("Test World", versions));

        let changes = compute_changes(&old, &new);

        assert_eq!(changes.worlds.len(), 1);
        let wc = &changes.worlds["test"];
        assert!(wc.added_versions.is_empty());
        assert_eq!(wc.removed_versions.len(), 1);
        assert_eq!(wc.removed_versions[0], Version::from_str("0.1.0")?);
        Ok(())
    }

    #[test]
    fn test_compute_changes_version_added() -> Result<()> {
        let mut old = empty_index();
        let mut new = empty_index();

        let (_tmp1, old_versions) = mock_world_versions(&["0.1.0"])?;
        let (_tmp2, new_versions) = mock_world_versions(&["0.1.0", "0.2.0"])?;

        old.worlds
            .insert("test".into(), mock_world("Test", old_versions));
        new.worlds
            .insert("test".into(), mock_world("Test", new_versions));

        let changes = compute_changes(&old, &new);

        assert_eq!(changes.worlds.len(), 1);
        let wc = &changes.worlds["test"];
        assert_eq!(wc.added_versions, vec![Version::from_str("0.2.0")?]);
        assert!(wc.removed_versions.is_empty());
        Ok(())
    }

    #[test]
    fn test_compute_changes_version_removed() -> Result<()> {
        let mut old = empty_index();
        let mut new = empty_index();

        let (_tmp1, old_versions) = mock_world_versions(&["0.1.0", "0.2.0"])?;
        let (_tmp2, new_versions) = mock_world_versions(&["0.1.0"])?;

        old.worlds
            .insert("test".into(), mock_world("Test", old_versions));
        new.worlds
            .insert("test".into(), mock_world("Test", new_versions));

        let changes = compute_changes(&old, &new);

        assert_eq!(changes.worlds.len(), 1);
        let wc = &changes.worlds["test"];
        assert!(wc.added_versions.is_empty());
        assert_eq!(wc.removed_versions, vec![Version::from_str("0.2.0")?]);
        Ok(())
    }

    #[test]
    fn test_compute_changes_no_changes() -> Result<()> {
        let mut old = empty_index();
        let mut new = empty_index();

        let (_tmp1, old_versions) = mock_world_versions(&["0.1.0"])?;
        let (_tmp2, new_versions) = mock_world_versions(&["0.1.0"])?;

        old.worlds
            .insert("test".into(), mock_world("Test", old_versions));
        new.worlds
            .insert("test".into(), mock_world("Test", new_versions));

        let changes = compute_changes(&old, &new);
        assert!(changes.worlds.is_empty());
        Ok(())
    }

    #[test]
    fn test_compute_changes_mixed() -> Result<()> {
        let mut old = empty_index();
        let mut new = empty_index();

        let (_tmp1, old_versions) = mock_world_versions(&["0.1.0", "0.2.0"])?;
        let (_tmp2, new_versions) = mock_world_versions(&["0.1.0", "0.3.0"])?;

        old.worlds
            .insert("test".into(), mock_world("Test", old_versions));
        new.worlds
            .insert("test".into(), mock_world("Test", new_versions));

        let changes = compute_changes(&old, &new);

        let wc = &changes.worlds["test"];
        assert_eq!(wc.added_versions, vec![Version::from_str("0.3.0")?]);
        assert_eq!(wc.removed_versions, vec![Version::from_str("0.2.0")?]);
        Ok(())
    }

    #[test]
    fn test_write_and_read_changes() -> Result<()> {
        let tmpdir = tempdir()?;
        let changes = Changes {
            worlds: BTreeMap::from([(
                "test".into(),
                WorldChanges {
                    world_name: "Test World".into(),
                    added_versions: vec![Version::from_str("0.1.0")?],
                    removed_versions: vec![],
                    checksums: BTreeMap::from([(
                        Version::from_str("0.1.0")?,
                        Checksum::Hash("abc123".into()),
                    )]),
                },
            )]),
        };

        write_changes(&changes, tmpdir.path())?;

        let content = std::fs::read_to_string(tmpdir.path().join("changes.json"))?;
        let read_back: Changes = serde_json::from_str(&content)?;

        assert_eq!(read_back.worlds.len(), 1);
        let wc = &read_back.worlds["test"];
        assert_eq!(wc.world_name, "Test World");
        assert_eq!(
            wc.checksums[&Version::from_str("0.1.0")?],
            Checksum::Hash("abc123".into())
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_download_changed_apworlds() -> Result<()> {
        let mut new = empty_index();
        let output_dir = tempdir()?;

        let (_tmp, versions) = mock_world_versions(&["0.1.0"])?;
        new.worlds
            .insert("test".into(), mock_world("Test", versions));

        let changes = compute_changes(&empty_index(), &new);
        let checksums = download_changed_apworlds(&changes, &new, output_dir.path()).await?;

        let version = Version::from_str("0.1.0")?;
        assert!(matches!(
            checksums["test"][&version],
            Checksum::Hash(_)
        ));

        let apworld_path = output_dir.path().join("apworlds/test-0.1.0.apworld");
        assert!(apworld_path.exists());
        Ok(())
    }

    #[tokio::test]
    async fn test_download_from_changes_validates_checksum() -> Result<()> {
        let mut new = empty_index();
        let output_dir = tempdir()?;
        let download_dir = tempdir()?;

        let (_tmp, versions) = mock_world_versions(&["0.1.0"])?;
        new.worlds
            .insert("test".into(), mock_world("Test", versions));

        let mut changes = compute_changes(&empty_index(), &new);
        let checksums =
            download_changed_apworlds(&changes, &new, output_dir.path()).await?;
        apply_checksums(&mut changes, checksums);

        // Correct checksum should succeed
        download_from_changes(&changes, &new, download_dir.path()).await?;
        assert!(download_dir.path().join("test-0.1.0.apworld").exists());

        // Wrong checksum should fail
        changes
            .worlds
            .get_mut("test")
            .unwrap()
            .checksums
            .insert(
                Version::from_str("0.1.0")?,
                Checksum::Hash("wrong".into()),
            );

        let result = download_from_changes(&changes, &new, download_dir.path()).await;
        assert!(result.is_err());
        Ok(())
    }
}
