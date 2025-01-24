use std::{collections::BTreeMap, fmt::Display, path::Path};

use anyhow::{bail, Result};
use semver::Version;
use serde::{de::Error, Deserialize, Serialize};

use crate::{Index, World};

#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
pub enum VersionReq {
    Disabled,
    #[default]
    Latest,
    LatestSupported,
    Specific(Version),
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum NewApworldPolicy {
    #[default]
    Enable,
    Disable,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Manifest {
    #[serde(default)]
    worlds: BTreeMap<String, VersionReq>,
    #[serde(default)]
    pub new_apworld_policy: NewApworldPolicy,
}

#[derive(Debug)]
pub enum ResolveError<'a> {
    WorldNotFound(&'a str),
    VersionNotFound(String, VersionReq),
    WorldDisabled(String),
}

impl Display for ResolveError<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResolveError::WorldNotFound(world) => {
                f.write_fmt(format_args!("Couldn't find world {}", world))
            }
            ResolveError::VersionNotFound(world, version_req) => f.write_fmt(format_args!(
                "Couldn't resolve version {} for world {}",
                version_req, world
            )),
            ResolveError::WorldDisabled(world) => {
                f.write_fmt(format_args!("The world {} is disabled", world))
            }
        }
    }
}

impl VersionReq {
    pub fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "Latest" => Self::Latest,
            "Latest Supported" => Self::LatestSupported,
            "Disabled" => Self::Disabled,
            s => Self::Specific(Version::parse(s)?),
        })
    }

    pub fn to_string_pretty(&self, world: &World) -> String {
        match self {
            Self::Latest => {
                let Some((version, _)) = world.get_latest_release() else {
                    return self.to_string();
                };
                format!("Latest ({})", version)
            }
            Self::LatestSupported => {
                let Some((version, _)) = world.get_latest_supported_release() else {
                    return self.to_string();
                };
                format!("Supported ({})", version)
            }
            _ => self.to_string(),
        }
    }
}

impl Display for VersionReq {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Latest => f.write_str("Latest"),
            Self::LatestSupported => f.write_str("Latest Supported"),
            Self::Specific(v) => v.fmt(f),
            Self::Disabled => f.write_str("Disabled"),
        }
    }
}

impl<'de> Deserialize<'de> for VersionReq {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let o: Option<String> = Option::deserialize(deserializer)?;

        o.map(|v| {
            let version = v.as_str();
            Ok(match version {
                "latest" => VersionReq::Latest,
                "latest_supported" => VersionReq::LatestSupported,
                "disabled" => VersionReq::Disabled,
                _ => VersionReq::Specific(Version::parse(version).map_err(D::Error::custom)?),
            })
        })
        .unwrap_or(Ok(VersionReq::Latest))
    }
}

impl Manifest {
    pub fn new() -> Self {
        Self {
            worlds: BTreeMap::new(),
            new_apworld_policy: NewApworldPolicy::Enable,
        }
    }

    pub fn from_index_with_latest_versions(index: &Index) -> Result<Self> {
        let mut result = Self::new();

        for (name, world) in &index.worlds {
            if world.get_latest_release().is_none() {
                bail!(format!("World `{}` has no known release", name));
            };

            result.worlds.insert(name.to_string(), VersionReq::Latest);
        }

        Ok(result)
    }

    pub fn from_index_with_default_versions(index: &Index) -> Result<Self> {
        let mut result = Self::new();

        for (name, world) in &index.worlds {
            if world.get_latest_release().is_none() {
                bail!(format!("World `{}` has no known release", name));
            };

            result
                .worlds
                .insert(name.to_string(), world.default_version.clone());
        }

        Ok(result)
    }

    pub fn parse_file(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;

        Manifest::parse(&content)
    }

    pub fn parse(content: &str) -> Result<Self> {
        let deser = toml::Deserializer::new(content);

        let manifest: Manifest = serde_path_to_error::deserialize(deser)?;
        Ok(manifest)
    }

    pub fn updated_with_index(&self, index: &Index) -> Result<Self> {
        let mut result = Self::new();
        result.new_apworld_policy = self.new_apworld_policy;

        for world_name in index.worlds.keys() {
            result
                .worlds
                .insert(world_name.to_string(), self.get_version_req(world_name));
        }

        Ok(result)
    }

    pub fn resolve_with(
        &self,
        index: &Index,
    ) -> (BTreeMap<String, (World, Version)>, Vec<ResolveError>) {
        let mut resolve_errors = vec![];
        let mut ret = BTreeMap::new();

        for (world_name, world) in &index.worlds {
            let version_requirement = self.get_version_req(world_name);
            if version_requirement == VersionReq::Disabled {
                continue;
            }

            let version = match self.resolve_world_version(world, &version_requirement) {
                Ok((_, version)) => version,
                Err(e) => {
                    resolve_errors.push(e);
                    continue;
                }
            };

            ret.insert(world_name.clone(), (world.clone(), version));
        }

        (ret, resolve_errors)
    }

    pub fn resolve_from_game_name<'a>(
        &'a self,
        game_name: &'a str,
        index: &Index,
    ) -> Result<(World, Version), ResolveError<'a>> {
        let Some((apworld_name, world)) =
            index.worlds.iter().find(|(_, game)| game.name == game_name)
        else {
            return Err(ResolveError::WorldNotFound(game_name));
        };

        let version_requirement = self.get_version_req(apworld_name);
        self.resolve_world_version(world, &version_requirement)
    }

    fn resolve_world_version(
        &self,
        world: &World,
        version_requirement: &VersionReq,
    ) -> Result<(World, Version), ResolveError> {
        let resolved = match version_requirement {
            VersionReq::LatestSupported => world.get_latest_supported_release(),
            VersionReq::Latest => world.get_latest_release(),
            VersionReq::Specific(version) => world
                .get_version(version)
                .map(|origin| (version, origin))
                .or_else(|| world.get_latest_release()),
            VersionReq::Disabled => {
                return Err(ResolveError::WorldDisabled(world.display_name.clone()))
            }
        };

        match resolved {
            None => Err(ResolveError::VersionNotFound(
                world.name.clone(),
                version_requirement.clone(),
            )),
            Some((version, _)) => Ok((world.clone(), version.clone())),
        }
    }

    pub fn is_enabled(&self, apworld_name: &str) -> bool {
        match self.new_apworld_policy {
            NewApworldPolicy::Enable => self
                .worlds
                .get(apworld_name)
                .map(|version_req| version_req != &VersionReq::Disabled)
                .unwrap_or(true),
            NewApworldPolicy::Disable => self
                .worlds
                .get(apworld_name)
                .map(|version_req| version_req != &VersionReq::Disabled)
                .unwrap_or(false),
        }
    }

    pub fn get_version_req(&self, apworld_name: &str) -> VersionReq {
        self.worlds
            .get(apworld_name)
            .cloned()
            .unwrap_or_else(|| match self.new_apworld_policy {
                NewApworldPolicy::Enable => VersionReq::Latest,
                NewApworldPolicy::Disable => VersionReq::Disabled,
            })
    }

    pub fn add_version_req(&mut self, apworld_name: &str, version_req: VersionReq) {
        self.worlds.insert(apworld_name.to_string(), version_req);
    }
}
