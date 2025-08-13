use apwm::{Index, VersionReq, World, WorldOrigin};
use semver::Version;
use std::collections::BTreeMap;
use std::path::PathBuf;

pub fn create_test_world(
    name: &str,
    default_version: VersionReq,
    versions: Vec<(&str, WorldOrigin)>,
    supported: bool,
) -> World {
    let mut version_map = BTreeMap::new();
    for (version_str, origin) in versions {
        version_map.insert(Version::parse(version_str).unwrap(), origin);
    }

    World {
        path: PathBuf::from(format!("{}.toml", name)),
        name: name.to_string(),
        display_name: name.to_string(),
        default_url: None,
        default_version,
        home: None,
        versions: version_map,
        disabled: false,
        supported,
        tags: vec![],
    }
}

pub fn create_test_index(worlds: Vec<(&str, VersionReq, Vec<(&str, WorldOrigin)>, bool)>) -> Index {
    let mut world_map = BTreeMap::new();
    let archipelago_version = Version::parse(ARCHIPELAGO_VERSION).unwrap();

    for (name, default_version, versions, supported) in worlds {
        let mut world = create_test_world(name, default_version, versions, supported);

        if world.supported {
            world
                .versions
                .insert(archipelago_version.clone(), WorldOrigin::Supported);
        }

        world_map.insert(name.to_string(), world);
    }

    Index {
        path: PathBuf::from("test_index.toml"),
        archipelago_repo: "https://github.com/ArchipelagoMW/Archipelago"
            .parse()
            .unwrap(),
        archipelago_version,
        index_homepage: "https://archipelago.gg".to_string(),
        index_dir: PathBuf::from("index"),
        worlds: world_map,
    }
}

pub const ARCHIPELAGO_VERSION: &str = "0.5.0";
