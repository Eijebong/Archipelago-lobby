use std::fs::File;
use std::io::Read;
use std::io::Write;

use anyhow::Context as _;
use apwm::Index;
use askama::Template;
use http::header::CONTENT_DISPOSITION;
use rocket::fs::NamedFile;
use rocket::http::CookieJar;
use rocket::http::Header;
use rocket::response::Redirect;
use rocket::routes;
use rocket::Responder;
use rocket::State;

use crate::error::Result;
use crate::utils::RenamedFile;
use crate::utils::ZipFile;
use crate::Context;
use crate::IndexManager;
use crate::TplContext;

use super::auth::AdminSession;
use super::auth::LoggedInSession;

#[derive(Template)]
#[template(path = "apworlds.html")]
struct WorldsListTpl<'a> {
    base: TplContext<'a>,
    index: Index,
}

#[derive(Responder)]
#[allow(clippy::large_enum_variant)]
enum APWorldResponse<'a> {
    NamedFile(RenamedFile<'a>),
    Redirect(Redirect),
}

#[rocket::get("/worlds")]
#[tracing::instrument(skip_all)]
async fn list_worlds<'a>(
    index_manager: &'a State<IndexManager>,
    session: LoggedInSession,
    cookies: &CookieJar<'a>,
) -> Result<WorldsListTpl<'a>> {
    let index = index_manager.index.read().await.clone();

    Ok(WorldsListTpl {
        base: TplContext::from_session("apworlds", session.0, cookies),
        index,
    })
}

#[rocket::get("/worlds/download_all")]
#[tracing::instrument(skip_all)]
async fn download_all(
    index_manager: &State<IndexManager>,
    _session: LoggedInSession,
) -> Result<ZipFile> {
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(vec![]));
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let apworlds_path = &index_manager.apworlds_path;
    let prefix = "custom_worlds";
    writer.add_directory(prefix, options)?;

    let index = index_manager.index.read().await;
    let mut buffer = Vec::new();
    for (world_name, world) in &index.worlds() {
        let Some((version, _)) = world.get_latest_release() else {
            continue;
        };
        let file_path = index.get_world_local_path(apworlds_path, world_name, version);
        writer.start_file(format!("{}/{}.apworld", prefix, world_name), options)?;
        File::open(&file_path)?.read_to_end(&mut buffer)?;
        writer.write_all(&buffer)?;
        buffer.clear();
    }

    let value = "attachment; filename=\"apworlds.zip\"";
    let content = writer.finish()?.into_inner();

    return Ok(ZipFile {
        content,
        headers: Header::new(CONTENT_DISPOSITION.as_str(), value),
    });
}

#[rocket::get("/worlds/download/<world_name>/<version>")]
#[tracing::instrument(skip(index_manager, _session))]
async fn download_world<'a>(
    index_manager: &State<IndexManager>,
    version: &str,
    world_name: &str,
    _session: LoggedInSession,
) -> Result<APWorldResponse<'a>> {
    let index = index_manager.index.read().await;

    let worlds = index.worlds();
    let world = worlds
        .get(world_name)
        .context("This APworld doesn't seem to exist")?;

    let version = semver::Version::parse(version).context("The passed version isn't valid")?;

    let origin = world
        .get_version(&version)
        .context("The specified version doesn't exist for this apworld")?;

    if origin.is_local() || origin.has_patches() {
        let apworld_path = world.get_path_for_origin(origin)?;
        if !apworld_path.exists() {
            return Err(anyhow::anyhow!(
                "This apworld seems to be in the host's index but not in their apworld folder."
            )
            .into());
        }

        let value = format!("attachment; filename=\"{}.apworld\"", world_name);
        return Ok(APWorldResponse::NamedFile(RenamedFile {
            inner: NamedFile::open(&apworld_path).await?,
            headers: Header::new(CONTENT_DISPOSITION.as_str(), value),
        }));
    }

    return Ok(APWorldResponse::Redirect(Redirect::to(
        world.get_url_for_version(&version)?,
    )));
}

#[rocket::get("/worlds/refresh")]
#[tracing::instrument(skip_all)]
async fn refresh_worlds(
    index_manager: &State<IndexManager>,
    _session: AdminSession,
) -> Result<()> {
    index_manager.update().await?;

    Ok(())
}

pub fn routes() -> Vec<rocket::Route> {
    routes![list_worlds, download_all, download_world, refresh_worlds]
}
