use std::{collections::BTreeMap, path::Path};

use anyhow::{bail, Result};
use semver::Version;
use serde::{Deserialize, Serialize};

use crate::{Index, World};

#[derive(Debug, Serialize, Deserialize)]
pub struct ManifestCommon {
    pub archipelago_version: Version,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum WorldVersion {
    Latest,
    LatestSupported,
    Specific(Version),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub common: ManifestCommon,
    pub worlds: BTreeMap<String, WorldVersion>,
}

pub enum ResolveError<'a> {
    WorldNotFound(&'a str),
    VersionNotFound(String, WorldVersion),
    WorldNotInManifest(&'a str),
}

impl<'a> ResolveError<'a> {
    pub fn is_fatal(&self) -> bool {
        match self {
            Self::WorldNotFound(..) | Self::WorldNotInManifest(..) => true,
            Self::VersionNotFound(..) => false,
        }
    }
}

impl Manifest {
    pub fn new(archipelago_version: Version) -> Self {
        Self {
            common: ManifestCommon {
                archipelago_version,
            },
            worlds: BTreeMap::new(),
        }
    }

    pub fn from_index_with_latest_versions(index: &Index) -> Result<Self> {
        let mut result = Self::new(index.archipelago_version.clone());
        for (name, world) in index.worlds().iter() {
            if world.get_latest_release().is_none() {
                bail!(format!("World `{}` has no known release", name));
            };

            result.worlds.insert(name.to_string(), WorldVersion::Latest);
        }

        Ok(result)
    }

    pub fn parse_file(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;

        Manifest::from_str(&content)
    }

    pub fn from_str(content: &str) -> Result<Self> {
        let deser = toml::Deserializer::new(content);

        let manifest: Manifest = serde_path_to_error::deserialize(deser)?;
        Ok(manifest)
    }

    pub fn resolve_with(
        &self,
        index: &Index,
    ) -> (BTreeMap<String, (World, Version)>, Vec<ResolveError>) {
        let index_worlds = index.worlds();
        let mut resolve_errors = vec![];
        let mut ret = BTreeMap::new();

        for (apworld_name, version_requirement) in &self.worlds {
            let Some(world) = index_worlds.get(apworld_name) else {
                resolve_errors.push(ResolveError::WorldNotFound(apworld_name));
                continue;
            };

            let version = match self.resolve_world_version(world, version_requirement) {
                Ok((_, version)) => version,
                Err(e) => {
                    resolve_errors.push(e);
                    continue;
                }
            };

            ret.insert(apworld_name.clone(), (world.clone(), version));
        }

        (ret, resolve_errors)
    }

    pub fn resolve_from_game_name<'a>(
        &'a self,
        game_name: &'a str,
        index: &Index,
    ) -> Result<(World, Version), ResolveError<'a>> {
        let worlds = index.worlds();
        let Some((apworld_name, world)) = worlds.iter().find(|(_, game)| game.name == game_name)
        else {
            return Err(ResolveError::WorldNotFound(game_name));
        };

        let Some(version_requirement) = self.worlds.get(apworld_name) else {
            return Err(ResolveError::WorldNotInManifest(game_name));
        };

        self.resolve_world_version(world, version_requirement)
    }

    fn resolve_world_version(
        &self,
        world: &World,
        version_requirement: &WorldVersion,
    ) -> Result<(World, Version), ResolveError> {
        let resolved = match version_requirement {
            WorldVersion::LatestSupported => world.get_latest_supported_release(),
            WorldVersion::Latest => world.get_latest_release(),
            WorldVersion::Specific(version) => {
                world.get_version(&version).map(|origin| (version, origin))
            }
        };

        match resolved {
            None => {
                return Err(ResolveError::VersionNotFound(
                    world.name.clone(),
                    version_requirement.clone(),
                ))
            }
            Some((version, _)) => Ok((world.clone(), version.clone())),
        }
    }
}
