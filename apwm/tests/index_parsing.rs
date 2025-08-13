use apwm::{VersionReq, WorldOrigin};
use semver::Version;

mod common;
use common::{create_test_index, ARCHIPELAGO_VERSION};

#[test]
fn test_supported_world_gets_archipelago_version() {
    let index = create_test_index(vec![(
        "supported_world",
        VersionReq::Latest,
        vec![
            ("1.0.0", WorldOrigin::Default),
            ("2.0.0", WorldOrigin::Default),
        ],
        true,
    )]);

    let world = index.worlds.get("supported_world").unwrap();

    assert_eq!(
        world.versions.len(),
        3,
        "Supported world should have archipelago_version added"
    );

    let archipelago_version = Version::parse(ARCHIPELAGO_VERSION).unwrap();
    let origin = world.versions.get(&archipelago_version).unwrap();
    assert_eq!(
        *origin,
        WorldOrigin::Supported,
        "Archipelago version should be marked as Supported"
    );

    let supported_count = world
        .versions
        .values()
        .filter(|origin| matches!(origin, WorldOrigin::Supported))
        .count();
    assert_eq!(
        supported_count, 1,
        "There should be exactly one Supported version"
    );
}

#[test]
fn test_non_supported_world_has_no_supported_versions() {
    let index = create_test_index(vec![(
        "non_supported_world",
        VersionReq::Latest,
        vec![
            ("1.0.0", WorldOrigin::Default),
            ("2.0.0", WorldOrigin::Default),
        ],
        false,
    )]);

    let world = index.worlds.get("non_supported_world").unwrap();

    assert_eq!(
        world.versions.len(),
        2,
        "Non-supported world should not have archipelago_version added"
    );

    let supported_count = world
        .versions
        .values()
        .filter(|origin| matches!(origin, WorldOrigin::Supported))
        .count();
    assert_eq!(
        supported_count, 0,
        "Non-supported world should have no Supported versions"
    );
}

#[test]
fn test_latest_supported_resolves_to_archipelago_version() {
    let index = create_test_index(vec![(
        "supported_world",
        VersionReq::Latest,
        vec![
            ("1.0.0", WorldOrigin::Default),
            ("2.0.0", WorldOrigin::Default),
        ],
        true,
    )]);

    let world = index.worlds.get("supported_world").unwrap();
    let latest_supported = world.get_latest_supported_release();

    assert!(
        latest_supported.is_some(),
        "Should find a supported release"
    );
    let (version, origin) = latest_supported.unwrap();
    assert_eq!(
        *version,
        Version::parse(ARCHIPELAGO_VERSION).unwrap(),
        "Latest supported should be archipelago_version"
    );
    assert_eq!(
        *origin,
        WorldOrigin::Supported,
        "Should be marked as Supported"
    );
}

#[test]
fn test_non_supported_world_has_no_latest_supported() {
    let index = create_test_index(vec![(
        "non_supported_world",
        VersionReq::Latest,
        vec![
            ("1.0.0", WorldOrigin::Default),
            ("2.0.0", WorldOrigin::Default),
        ],
        false,
    )]);

    let world = index.worlds.get("non_supported_world").unwrap();
    let latest_supported = world.get_latest_supported_release();

    assert!(
        latest_supported.is_none(),
        "Non-supported world should have no supported releases"
    );
}

#[test]
fn test_archipelago_version_is_consistent_across_worlds() {
    let index = create_test_index(vec![
        (
            "world1",
            VersionReq::Latest,
            vec![("1.0.0", WorldOrigin::Default)],
            true,
        ),
        (
            "world2",
            VersionReq::Latest,
            vec![("2.0.0", WorldOrigin::Default)],
            true,
        ),
        (
            "world3",
            VersionReq::Latest,
            vec![("3.0.0", WorldOrigin::Default)],
            false,
        ),
    ]);

    let archipelago_version = Version::parse(ARCHIPELAGO_VERSION).unwrap();

    let world1 = index.worlds.get("world1").unwrap();
    assert!(
        world1.versions.contains_key(&archipelago_version),
        "World1 should have archipelago_version"
    );

    let world2 = index.worlds.get("world2").unwrap();
    assert!(
        world2.versions.contains_key(&archipelago_version),
        "World2 should have archipelago_version"
    );

    let world3 = index.worlds.get("world3").unwrap();
    assert!(
        !world3.versions.contains_key(&archipelago_version),
        "World3 should not have archipelago_version"
    );
}
