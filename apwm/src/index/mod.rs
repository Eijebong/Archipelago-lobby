pub mod lock;
pub mod world;

use anyhow::{bail, Context, Result};
use http::Uri;
use lock::IndexLock;
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
use std::{
    fs::{self, OpenOptions},
    io::Read,
};

use world::{World, WorldOrigin};

#[derive(Deserialize, Debug, Clone)]
pub struct Index {
    #[serde(skip)]
    pub path: PathBuf,
    #[serde(with = "http_serde::uri")]
    pub archipelago_repo: Uri,
    pub archipelago_version: Version,
    pub index_homepage: String,
    pub index_dir: PathBuf,
    #[serde(default)]
    pub worlds: BTreeMap<String, World>,
}

impl Index {
    pub fn new(index_path: &Path) -> Result<Self> {
        let index_content = std::fs::read_to_string(index_path).context("Reading index.toml")?;
        let deser = toml::Deserializer::new(&index_content);
        let mut index: Index = serde_path_to_error::deserialize(deser)?;
        let index_dir_resolved = index_path
            .parent()
            .context("index_path doesn't have a parent")?
            .join(&index.index_dir);

        if !index_dir_resolved.is_dir() {
            bail!("The specified index directory isn't a directory or doesn't exist");
        }

        let world_tomls = fs::read_dir(index_dir_resolved)?;
        for world_toml in world_tomls {
            let world_toml = world_toml?;
            let world_path = world_toml.path();
            let world_name = world_path
                .file_stem()
                .with_context(|| format!("World path {:?} is invalid", world_path))?
                .to_string_lossy();
            let mut world = World::new(&world_toml.path())?;
            if world.disabled {
                continue;
            }

            if world.supported {
                world
                    .versions
                    .insert(index.archipelago_version.clone(), WorldOrigin::Supported);
            }

            index.worlds.insert(world_name.to_string(), world);
        }

        index.path = index_path.into();

        Ok(index)
    }

    pub async fn refresh_into(
        &self,
        destination: &Path,
        only_new: bool,
        precise: Option<(String, Version)>,
    ) -> Result<IndexLock> {
        log::info!("Refreshing index into {:?}", destination);

        let parent = self.path.parent().context("Invalid index path")?;
        let lock_toml = parent.join("index.lock");
        let old_lock = IndexLock::new(&lock_toml)?;
        let mut new_lock = IndexLock::default();
        new_lock.path = old_lock.path.clone();
        if destination.is_file() {
            bail!("Error downloading, destination exists and is a file");
        }

        std::fs::create_dir_all(destination)?;

        for (world_name, world) in &self.worlds {
            for (version, origin) in &world.versions {
                log::debug!("Refreshing world: {}, version: {}", world_name, version);
                if let Some((ref target_world, ref target_version)) = precise {
                    if world_name != target_world {
                        log::debug!("Ignoring world because of precise requirement");
                        continue;
                    }
                    if version != target_version {
                        log::debug!("Ignoring version because of precise requirement");
                        continue;
                    }
                }

                if world.disabled {
                    log::debug!("World is disabled, ignoring");
                    continue;
                }

                if origin.is_supported() {
                    log::debug!("World is supported, skipping");
                    continue;
                }

                let apworld_destination_path =
                    self.get_world_local_path(destination, world_name, version);
                let expected_checksum = old_lock.get_checksum(world_name, version);

                if expected_checksum.is_some() && only_new {
                    log::debug!(
                        "World exists in lockfile and we only want to refresh new ones, ignoring."
                    );
                    new_lock.set_checksum(world_name, version, &expected_checksum.unwrap());
                    continue;
                }

                if apworld_destination_path.is_file() {
                    let mut apworld_destination = OpenOptions::new()
                        .read(true)
                        .open(&apworld_destination_path)?;
                    let mut buf = Vec::new();
                    apworld_destination.read_to_end(&mut buf)?;
                    let current_checksum = format!("{:x}", Sha256::digest(&buf));
                    if expected_checksum == Some(current_checksum.clone()) {
                        log::debug!("World exists in lockfile and on disk, checksums are matching, ignoring");
                        new_lock.set_checksum(world_name, version, &current_checksum);
                        continue;
                    }

                    if expected_checksum.is_none() {
                        log::debug!("World exists in index but not in lockfile, continuing.");
                    }
                }

                log::debug!("Copying world into worlds folder.");
                let apworld_destination = OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .open(&apworld_destination_path)?;
                let checksum = world.copy_to(version, &apworld_destination).await?;
                new_lock.set_checksum(world_name, version, &checksum);
            }
        }

        Ok(new_lock)
    }

    pub fn get_world_local_path(
        &self,
        apworld_root: &Path,
        world_name: &str,
        version: &Version,
    ) -> PathBuf {
        apworld_root.join(format!("{}-{}.apworld", world_name, version))
    }
}
