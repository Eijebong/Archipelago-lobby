use crate::db;
use crate::db::OpenState;
use crate::db::RoomFilter;
use crate::jobs::YamlValidationQueue;
use crate::yaml::revalidate_yamls_if_necessary;
use anyhow::Context as _;
use apwm::Index;
use apwm::Manifest;
use apwm::World;
use apwm::WorldOrigin;
use askama::Template;
use http::header::CONTENT_DISPOSITION;
use rocket::fs::NamedFile;
use rocket::http::Header;
use rocket::routes;
use rocket::State;
use semver::Version;

use crate::error::Result;
use crate::index_manager::IndexManager;
use crate::session::{AdminSession, LoggedInSession};
use crate::utils::{RenamedFile, ZipFile};
use crate::Context;
use crate::TplContext;

#[derive(Template)]
#[template(path = "apworlds.html")]
struct WorldsListTpl<'a> {
    base: TplContext<'a>,
    index: Index,
    supported_apworlds: Vec<(String, (World, Version))>,
    unsupported_apworlds: Vec<(String, (World, Version))>,
}

#[rocket::get("/worlds")]
#[tracing::instrument(skip_all)]
async fn list_worlds<'a>(
    index_manager: &'a State<IndexManager>,
    session: LoggedInSession,
    ctx: &State<Context>,
) -> Result<WorldsListTpl<'a>> {
    let index = index_manager.index.read().await.clone();
    let manifest = Manifest::from_index_with_default_versions(&index)?;
    let (worlds, _) = manifest.resolve_with(&index);

    let (mut supported_apworlds, mut unsupported_apworlds): (Vec<_>, Vec<_>) =
        worlds.into_iter().partition(|(_, (world, version))| {
            world.supported && matches!(world.get_version(version).unwrap(), WorldOrigin::Supported)
        });

    supported_apworlds.sort_by_cached_key(|(_, (world, _))| world.display_name.clone());
    unsupported_apworlds.sort_by_cached_key(|(_, (world, _))| world.display_name.clone());

    Ok(WorldsListTpl {
        base: TplContext::from_session("apworlds", session.0, ctx).await,
        index,
        supported_apworlds,
        unsupported_apworlds,
    })
}

#[rocket::get("/worlds/download_all")]
#[tracing::instrument(skip_all)]
async fn download_all(
    index_manager: &State<IndexManager>,
    _session: LoggedInSession,
) -> Result<ZipFile> {
    let index = index_manager.index.read().await.clone();
    let manifest = Manifest::from_index_with_default_versions(&index)?;
    Ok(index_manager.download_apworlds(&manifest).await?)
}

#[rocket::get("/worlds/download/<world_name>/<version>")]
#[tracing::instrument(skip(index_manager, _session))]
async fn download_world<'a>(
    index_manager: &State<IndexManager>,
    version: &str,
    world_name: &str,
    _session: LoggedInSession,
) -> Result<RenamedFile<'a>> {
    let index = index_manager.index.read().await;

    let world = index
        .worlds
        .get(world_name)
        .context("This APworld doesn't seem to exist")?;

    let version = semver::Version::parse(version).context("The passed version isn't valid")?;

    let _origin = world
        .get_version(&version)
        .context("The specified version doesn't exist for this apworld")?;

    let apworld_path = index_manager
        .apworlds_path
        .join(format!("{}-{}.apworld", world_name, version));

    if !apworld_path.exists() {
        return Err(anyhow::anyhow!(
            "This apworld seems to be in the host's index but not in their apworld folder."
        )
        .into());
    }

    let value = format!("attachment; filename=\"{}.apworld\"", world_name);
    return Ok(RenamedFile {
        inner: NamedFile::open(&apworld_path).await?,
        headers: Header::new(CONTENT_DISPOSITION.as_str(), value),
    });
}

#[rocket::get("/worlds/refresh")]
#[tracing::instrument(skip_all)]
async fn refresh_worlds(
    index_manager: &State<IndexManager>,
    yaml_validation_queue: &State<YamlValidationQueue>,
    ctx: &State<Context>,
    _session: AdminSession,
) -> Result<()> {
    index_manager.update().await?;

    let mut conn = ctx.db_pool.get().await?;

    let (open_rooms, _) = db::list_rooms(
        RoomFilter::default().with_open_state(OpenState::Open),
        None,
        &mut conn,
    )
    .await?;

    for room in &open_rooms {
        revalidate_yamls_if_necessary(room, index_manager, yaml_validation_queue, &mut conn)
            .await?;
    }

    Ok(())
}

pub fn routes() -> Vec<rocket::Route> {
    routes![list_worlds, download_all, download_world, refresh_worlds]
}
