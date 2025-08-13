use apwm::{Manifest, ResolveError, VersionReq, WorldOrigin};
use semver::Version;

mod common;
use common::{create_test_index, ARCHIPELAGO_VERSION};

#[test]
fn test_latest_respects_default_version_when_specific() {
    let index = create_test_index(vec![(
        "test_world",
        VersionReq::Specific(Version::parse("2.0.0").unwrap()),
        vec![
            ("1.0.0", WorldOrigin::Default),
            ("2.0.0", WorldOrigin::Default),
            ("3.0.0", WorldOrigin::Default),
        ],
        false,
    )]);

    let mut manifest = Manifest::new();
    manifest.add_version_req("test_world", VersionReq::Latest);

    let (resolved_worlds, errors) = manifest.resolve_with(&index);

    assert!(errors.is_empty(), "Should not have resolution errors");
    assert_eq!(resolved_worlds.len(), 1, "Should resolve one world");

    let (_world, resolved_version) = resolved_worlds.get("test_world").unwrap();
    assert_eq!(
        resolved_version,
        &Version::parse("2.0.0").unwrap(),
        "Latest should resolve to default_version (2.0.0), not actual latest (3.0.0)"
    );
}

#[test]
fn test_latest_respects_default_version_when_latest_supported() {
    let index = create_test_index(vec![(
        "test_world",
        VersionReq::LatestSupported,
        vec![("2.0.0", WorldOrigin::Default)],
        true,
    )]);

    let mut manifest = Manifest::new();
    manifest.add_version_req("test_world", VersionReq::Latest);

    let (resolved_worlds, errors) = manifest.resolve_with(&index);

    assert!(errors.is_empty(), "Should not have resolution errors");
    let (_world, resolved_version) = resolved_worlds.get("test_world").unwrap();
    assert_eq!(resolved_version, &Version::parse(ARCHIPELAGO_VERSION).unwrap(),
               "Latest should resolve to latest supported ({} - archipelago_version) when default_version is LatestSupported", ARCHIPELAGO_VERSION);
}

#[test]
fn test_latest_falls_back_to_latest_when_default_version_is_latest() {
    let index = create_test_index(vec![(
        "test_world",
        VersionReq::Latest,
        vec![
            ("1.0.0", WorldOrigin::Default),
            ("2.0.0", WorldOrigin::Default),
        ],
        false,
    )]);

    let mut manifest = Manifest::new();
    manifest.add_version_req("test_world", VersionReq::Latest);

    let (resolved_worlds, errors) = manifest.resolve_with(&index);

    assert!(errors.is_empty(), "Should not have resolution errors");
    let (_world, resolved_version) = resolved_worlds.get("test_world").unwrap();
    assert_eq!(
        resolved_version,
        &Version::parse("2.0.0").unwrap(),
        "Latest should resolve to actual latest when default_version is Latest"
    );
}

#[test]
fn test_latest_returns_error_when_default_version_not_found() {
    let index = create_test_index(vec![(
        "test_world",
        VersionReq::Specific(Version::parse("5.0.0").unwrap()),
        vec![
            ("1.0.0", WorldOrigin::Default),
            ("2.0.0", WorldOrigin::Default),
        ],
        false,
    )]);

    let mut manifest = Manifest::new();
    manifest.add_version_req("test_world", VersionReq::Latest);

    let (resolved_worlds, errors) = manifest.resolve_with(&index);

    assert_eq!(errors.len(), 1, "Should have one resolution error");
    assert!(resolved_worlds.is_empty(), "Should not resolve any worlds");

    match &errors[0] {
        ResolveError::VersionNotFound(world_name, version_req) => {
            assert_eq!(world_name, "test_world");
            assert_eq!(
                version_req,
                &VersionReq::Specific(Version::parse("5.0.0").unwrap())
            );
        }
        _ => panic!("Expected VersionNotFound error"),
    }
}

#[test]
fn test_latest_handles_disabled_default_version() {
    let index = create_test_index(vec![(
        "test_world",
        VersionReq::Disabled,
        vec![
            ("1.0.0", WorldOrigin::Default),
            ("2.0.0", WorldOrigin::Default),
        ],
        false,
    )]);

    let mut manifest = Manifest::new();
    manifest.add_version_req("test_world", VersionReq::Latest);

    let (resolved_worlds, errors) = manifest.resolve_with(&index);

    assert_eq!(errors.len(), 1, "Should have one resolution error");
    assert!(resolved_worlds.is_empty(), "Should not resolve any worlds");

    match &errors[0] {
        ResolveError::VersionNotFound(world_name, version_req) => {
            assert_eq!(world_name, "test_world");
            assert_eq!(version_req, &VersionReq::Disabled);
        }
        _ => panic!("Expected VersionNotFound error for disabled version"),
    }
}

#[test]
fn test_specific_version_still_falls_back_to_latest() {
    let index = create_test_index(vec![(
        "test_world",
        VersionReq::Latest,
        vec![
            ("1.0.0", WorldOrigin::Default),
            ("2.0.0", WorldOrigin::Default),
        ],
        false,
    )]);

    let mut manifest = Manifest::new();
    manifest.add_version_req(
        "test_world",
        VersionReq::Specific(Version::parse("5.0.0").unwrap()),
    );

    let (resolved_worlds, errors) = manifest.resolve_with(&index);

    assert!(errors.is_empty(), "Should not have resolution errors");
    let (_world, resolved_version) = resolved_worlds.get("test_world").unwrap();
    assert_eq!(
        resolved_version,
        &Version::parse("2.0.0").unwrap(),
        "Specific version should fall back to latest when not found"
    );
}

#[test]
fn test_latest_supported_unchanged() {
    let index = create_test_index(vec![(
        "test_world",
        VersionReq::Latest,
        vec![("2.0.0", WorldOrigin::Default)],
        true,
    )]);

    let mut manifest = Manifest::new();
    manifest.add_version_req("test_world", VersionReq::LatestSupported);

    let (resolved_worlds, errors) = manifest.resolve_with(&index);

    assert!(errors.is_empty(), "Should not have resolution errors");
    let (_world, resolved_version) = resolved_worlds.get("test_world").unwrap();
    assert_eq!(resolved_version, &Version::parse(ARCHIPELAGO_VERSION).unwrap(),
               "LatestSupported should always resolve to latest supported version ({} - archipelago_version)", ARCHIPELAGO_VERSION);
}
