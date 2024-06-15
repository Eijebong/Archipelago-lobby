use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::io::Write;
use std::path::Path;

use anyhow::Context;
use apwm::Index;
use apwm::World;
use askama::Template;
use rocket::fs::NamedFile;
use rocket::http::hyper::header::CONTENT_DISPOSITION;
use rocket::http::CookieJar;
use rocket::http::Header;
use rocket::response::Redirect;
use rocket::routes;
use rocket::Responder;
use rocket::State;
use walkdir::DirEntry;
use walkdir::WalkDir;

use crate::error::Result;
use crate::utils::ZipFile;
use crate::APWorldPath;
use crate::TplContext;

use super::auth::LoggedInSession;

#[derive(Template)]
#[template(path = "apworlds.html")]
struct WorldsListTpl<'a> {
    base: TplContext<'a>,
    supported_apworlds: BTreeMap<&'a String, &'a World>,
    unsupported_apworlds: BTreeMap<&'a String, &'a World>,
    index: &'a Index,
}

#[derive(Responder)]
enum APWorldResponse<'a> {
    NamedFile(NamedFile),
    ZipFile(ZipFile<'a>),
    Redirect(Redirect),
}

#[rocket::get("/worlds")]
fn list_worlds<'a>(
    index: &'a State<Index>,
    session: LoggedInSession,
    cookies: &CookieJar,
) -> Result<WorldsListTpl<'a>> {
    let (supported_apworlds, unsupported_apworlds): (BTreeMap<_, _>, BTreeMap<_, _>) = index
        .worlds
        .iter()
        .partition(|(_, world)| world.is_supported());

    Ok(WorldsListTpl {
        base: TplContext::from_session("apworlds", session.0, cookies),
        supported_apworlds,
        unsupported_apworlds,
        index,
    })
}

fn zip_dir(path: &Path, filter: impl Fn(&DirEntry) -> bool) -> Result<Vec<u8>> {
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(vec![]));
    let options =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let prefix = path.parent().map_or_else(|| "/", |p| p.to_str().unwrap());
    let mut buffer = Vec::new();

    for entry in WalkDir::new(path) {
        let entry = entry?;
        if !filter(&entry) {
            continue;
        }
        let path = entry.path().strip_prefix(prefix).unwrap();

        if entry.file_type().is_dir() {
            writer.add_directory(path.to_string_lossy(), options)?;
        }
        if entry.file_type().is_file() {
            writer.start_file(path.to_string_lossy(), options)?;
            File::open(entry.path())?.read_to_end(&mut buffer)?;
            writer.write_all(&buffer)?;
            buffer.clear();
        }
    }

    let res = writer.finish()?;
    Ok(res.into_inner())
}

#[rocket::get("/worlds/download_all")]
fn download_all<'a>(
    apworld_path: &'a State<APWorldPath>,
    index: &'a State<Index>,
    _session: LoggedInSession,
) -> Result<ZipFile<'a>> {
    let content = zip_dir(&apworld_path.0, |entry| {
        if entry.depth() != 1 {
            return true;
        }

        let Some(file_stem) = entry.path().file_stem() else {
            return false;
        };
        let file_stem = file_stem.to_string_lossy().to_string();
        // We're using file_stem here so both `pokemon_emerald/` and `pokemon_crystal.apworld`
        // would match the world name
        let is_world = index.worlds.contains_key(&file_stem);

        let Some(file_name) = entry.path().file_name() else {
            return false;
        };
        let file_name = file_name.to_string_lossy().to_string();
        let is_dependency = index
            .worlds
            .values()
            .any(|world| world.dependencies.contains(&file_name));
        let is_required_file = index
            .common
            .required_global_files
            .iter()
            .filter_map(|dep| Path::new(dep).file_name())
            .map(|s| s.to_string_lossy().to_string())
            .collect::<Vec<_>>()
            .contains(&file_name);

        is_world || is_dependency || is_required_file
    })?;
    let value = "attachment; filename=\"apworlds.zip\"";

    return Ok(ZipFile {
        content,
        headers: Header::new(CONTENT_DISPOSITION.as_str(), value),
    });
}

#[rocket::get("/worlds/download/<world_name>")]
async fn download_world<'a>(
    index: &'a State<Index>,
    apworld_path: &'a State<APWorldPath>,
    world_name: &str,
    _session: LoggedInSession,
) -> Result<APWorldResponse<'a>> {
    let world = index
        .worlds
        .get(world_name)
        .context("This APworld doesn't seem to exist")?;

    match (world.is_supported(), world.has_patches()) {
        // This is an unpatched stock apworld, people shouldn't be trying to download these.
        (true, false) => Err(anyhow::anyhow!(
            "This is a stock apworld, you should get it from your archipelago installation"
        )
        .into()),
        // This is a patched stock apworld, zip it up.
        (true, true) => {
            let world_path = apworld_path.0.join(world_name).to_owned();
            let content = zip_dir(&world_path, |_| true)?;

            let value = format!("attachment; filename=\"{}.apworld\"", world_name);
            return Ok(APWorldResponse::ZipFile(ZipFile {
                content,
                headers: Header::new(CONTENT_DISPOSITION.as_str(), value),
            }));
        }
        // This is a patched unsupported apworld, send it as is.
        (false, true) => {
            let apworld_path = apworld_path.0.join(format!("{}.apworld", world_name));
            if !apworld_path.exists() {
                return Err(anyhow::anyhow!(
                    "This apworld seems to be in the host's index but not in their apworld folder."
                )
                .into());
            }

            return Ok(APWorldResponse::NamedFile(
                NamedFile::open(&apworld_path).await?,
            ));
        }
        // This is an unpatched unsupported apworld, redirect to the original download link
        (false, false) => {
            if world.origin.is_local() {
                let apworld_path = apworld_path.0.join(format!("{}.apworld", world_name));
                if !apworld_path.exists() {
                    return Err(anyhow::anyhow!(
                        "This apworld seems to be in the host's index but not in their apworld folder."
                    )
                    .into());
                }

                return Ok(APWorldResponse::NamedFile(
                    NamedFile::open(&apworld_path).await?,
                ));
            }

            return Ok(APWorldResponse::Redirect(Redirect::to(
                world.url().to_string(),
            )));
        }
    }
}

pub fn routes() -> Vec<rocket::Route> {
    routes![list_worlds, download_all, download_world,]
}
