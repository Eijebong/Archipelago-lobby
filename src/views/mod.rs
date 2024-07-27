use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::io::{BufReader, Cursor, Write};
use std::path::PathBuf;

use crate::db::{RoomFilter, RoomStatus, Yaml, YamlFile, YamlGame};
use crate::utils::ZipFile;
use crate::{Context, TplContext};
use askama::Template;
use auth::{LoggedInSession, Session};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::AsyncConnection;
use itertools::Itertools;
use rocket::form::Form;
use rocket::http::hyper::header::CONTENT_DISPOSITION;
use rocket::http::{ContentType, CookieJar, Header};
use rocket::response::Redirect;
use rocket::routes;
use rocket::{get, post, uri, State};
use uuid::Uuid;

use crate::db::{self, Room};
use crate::error::{Error, RedirectTo, Result, WithContext};

pub mod apworlds;
pub mod auth;
pub mod room_manager;

#[derive(Template)]
#[template(path = "room.html")]
struct RoomTpl<'a> {
    base: TplContext<'a>,
    room: Room,
    author_name: String,
    yamls: Vec<(Yaml, String)>,
    player_count: usize,
    unique_player_count: usize,
    unique_game_count: usize,
    is_closed: bool,
    has_room_url: bool,
    is_my_room: bool,
}

#[derive(Template)]
#[template(path = "index.html")]
struct IndexTpl<'a> {
    base: TplContext<'a>,
    open_rooms: Vec<Room>,
    your_rooms: Vec<Room>,
}

#[get("/")]
async fn root<'a>(
    cookies: &CookieJar<'a>,
    session: Session,
    ctx: &State<Context>,
) -> Result<IndexTpl<'a>> {
    let open_rooms_filter = RoomFilter::new().with_status(RoomStatus::Open).with_max(10);
    let open_rooms_filter = if let Some(player_id) = session.user_id {
        open_rooms_filter
            .with_yamls_from(db::WithYaml::AndFor(player_id))
            .with_author(db::Author::IncludeUser(player_id))
    } else {
        open_rooms_filter
    };
    let open_rooms = db::list_rooms(open_rooms_filter, ctx).await?;

    let your_rooms = if let Some(player_id) = session.user_id {
        let your_rooms_filter = RoomFilter::new()
            .with_status(RoomStatus::Closed)
            .with_max(10)
            .with_yamls_from(db::WithYaml::OnlyFor(player_id))
            .with_private(true);
        db::list_rooms(your_rooms_filter, ctx).await?
    } else {
        vec![]
    };

    Ok(IndexTpl {
        base: TplContext::from_session("index", session, cookies),
        open_rooms,
        your_rooms,
    })
}

#[get("/room/<uuid>")]
async fn room<'a>(
    uuid: Uuid,
    ctx: &State<Context>,
    session: Session,
    cookies: &CookieJar<'a>,
) -> Result<RoomTpl<'a>> {
    let (room, author_name) = db::get_room_and_author(uuid, ctx).await?;
    let mut yamls = db::get_yamls_for_room_with_author_names(uuid, ctx).await?;
    yamls.sort_by(|a, b| a.0.game.cmp(&b.0.game));
    let unique_player_count = yamls.iter().unique_by(|yaml| yaml.0.owner_id).count();
    let unique_game_count = yamls
        .iter()
        .filter(|yaml| !&yaml.0.game.starts_with("Random ("))
        .unique_by(|yaml| &yaml.0.game)
        .count();

    let is_my_room = session.is_admin || session.user_id == Some(room.author_id);
    let current_user_has_yaml_in_room = yamls
        .iter()
        .any(|yaml| Some(yaml.0.owner_id) == session.user_id)
        || is_my_room;

    Ok(RoomTpl {
        base: TplContext::from_session("room", session, cookies),
        player_count: yamls.len(),
        unique_player_count,
        unique_game_count,
        is_closed: room.is_closed(),
        has_room_url: !room.room_url.is_empty() && current_user_has_yaml_in_room,
        author_name,
        room,
        yamls,
        is_my_room,
    })
}

#[post("/room/<uuid>/upload", data = "<yaml>")]
async fn upload_yaml(
    redirect_to: &RedirectTo,
    uuid: Uuid,
    yaml: Form<&[u8]>,
    mut session: LoggedInSession,
    cookies: &CookieJar<'_>,
    ctx: &State<Context>,
) -> Result<Redirect> {
    redirect_to.set(&format!("/room/{}", uuid));

    let room = db::get_room(uuid, ctx).await.context("Unknown room")?;
    if room.is_closed() {
        return Err(anyhow::anyhow!("This room is closed, you're late").into());
    }
    let (yaml, _encoding, has_errors) = encoding_rs::UTF_8.decode(&yaml);

    if has_errors {
        return Err(Error(anyhow::anyhow!("Error while decoding the yaml")));
    }

    let reader = BufReader::new(yaml.as_bytes());
    let documents = yaml_split::DocumentIterator::new(reader);
    let documents = documents
        .into_iter()
        .map(|doc| {
            let Ok(doc) = doc else {
                anyhow::bail!("Invalid yaml file. Syntax error.")
            };
            let Ok(parsed) = serde_yaml::from_str(&doc) else {
                anyhow::bail!(
                    "Invalid yaml file. This does not look like an archipelago game YAML."
                )
            };
            Ok((doc, parsed))
        })
        .collect::<anyhow::Result<Vec<(String, YamlFile)>>>()?;

    let yamls_in_room = db::get_yamls_for_room(uuid, ctx)
        .await
        .context("Couldn't get room yamls")?;
    let mut players_in_room = yamls_in_room
        .iter()
        .map(|yaml| {
            let mut player_name = yaml.player_name.clone();
            player_name.truncate(16);
            player_name
        })
        .collect::<HashSet<String>>();

    let mut games = Vec::with_capacity(documents.len());
    for (document, parsed) in documents.iter() {
        let mut player_name = parsed.name.clone();

        let ignore_dupe = player_name.contains("{NUMBER}")
            || player_name.contains("{number}")
            || player_name.contains("{PLAYER}")
            || player_name.contains("{player}");
        player_name.truncate(16);

        if player_name == "meta" || player_name == "Archipelago" {
            return Err(Error(anyhow::anyhow!(format!(
                "{} is a reserved name",
                player_name
            ))));
        }

        if !ignore_dupe && players_in_room.contains(&player_name) {
            return Err(Error(anyhow::anyhow!(
                "Adding this yaml would duplicate a player name"
            )));
        }

        let game_name = match &parsed.game {
            YamlGame::Name(name) => name.clone(),
            YamlGame::Map(map) => {
                let weighted_map: HashMap<&String, &f64> =
                    map.iter().filter(|(_, &weight)| weight >= 1.0).collect();

                match weighted_map.len() {
                    1 => weighted_map.keys().next().unwrap().to_string(),
                    n if n > 1 => format!("Random ({})", n),
                    _ => Err(anyhow::anyhow!(
                        "Your YAML contains games but none of the has any chance of getting rolled"
                    ))?,
                }
            }
        };

        if room.yaml_validation {
            let unsupported_games = validate_yaml(document, ctx).await?;
            if !unsupported_games.is_empty() {
                if room.allow_unsupported {
                    session.0.warning_msg.push(format!(
                        "Uploaded a YAML with unsupported games: {}. Couldn't verify it.",
                        unsupported_games.iter().join("; ")
                    ));
                    session.0.save(cookies)?;
                } else {
                    return Err(anyhow::anyhow!(format!(
                        "Your YAML contains the following unsupported games: {}. Can't upload.",
                        unsupported_games.iter().join("; ")
                    ))
                    .into());
                }
            }
        }

        players_in_room.insert(player_name);
        games.push((game_name, document, parsed));
    }

    let mut conn = ctx.db_pool.get().await?;
    conn.transaction::<(), Error, _>(|conn| {
        async move {
            for (game_name, document, parsed) in games {
                db::add_yaml_to_room(
                    uuid,
                    session.0.user_id.unwrap(),
                    &game_name,
                    document,
                    parsed,
                    conn,
                )
                .await?;
            }
            Ok(())
        }
        .scope_boxed()
    })
    .await?;

    Ok(Redirect::to(uri!(room(uuid))))
}

async fn validate_yaml(yaml: &str, ctx: &State<Context>) -> Result<Vec<String>> {
    if ctx.yaml_validator_url.is_none() {
        return Ok(vec![]);
    }

    #[derive(serde::Deserialize)]
    struct ValidationResponse {
        error: Option<String>,
        unsupported: Vec<String>,
    }

    let client = reqwest::Client::new();
    let form = reqwest::multipart::Form::new().text("data", yaml.to_string());

    let response = client
        .post(
            ctx.yaml_validator_url
                .as_ref()
                .unwrap()
                .join("/check_yaml")?,
        )
        .multipart(form)
        .send()
        .await
        .map_err(|_| anyhow::anyhow!("Error while communicating with the YAML validator."))?
        .json::<ValidationResponse>()
        .await?;

    if let Some(error) = response.error {
        return Err(anyhow::anyhow!(error).into());
    }

    Ok(response.unsupported)
}

#[get("/room/<room_id>/delete/<yaml_id>")]
async fn delete_yaml(
    redirect_to: &RedirectTo,
    room_id: Uuid,
    yaml_id: Uuid,
    session: LoggedInSession,
    ctx: &State<Context>,
) -> Result<Redirect> {
    redirect_to.set(&format!("/room/{}", room_id));

    let room = db::get_room(room_id, ctx).await.context("Unknown room")?;
    if room.is_closed() {
        return Err(anyhow::anyhow!("This room is closed, you're late").into());
    }

    let yaml = db::get_yaml_by_id(yaml_id, ctx).await?;

    let is_my_room = session.0.is_admin || session.0.user_id == Some(room.author_id);
    if yaml.owner_id != session.user_id() && !is_my_room {
        Err(anyhow::anyhow!("Can't delete a yaml file that isn't yours"))?
    }

    db::remove_yaml(yaml_id, ctx).await?;

    Ok(Redirect::to(format!("/room/{}", room_id)))
}

#[get("/room/<room_id>/yamls")]
async fn download_yamls<'a>(
    redirect_to: &RedirectTo,
    room_id: Uuid,
    ctx: &State<Context>,
    _session: LoggedInSession,
) -> Result<ZipFile<'a>> {
    redirect_to.set(&format!("/room/{}", room_id));

    let room = db::get_room(room_id, ctx).await?;
    let yamls = db::get_yamls_for_room(room_id, ctx).await?;
    let mut writer = zip::ZipWriter::new(Cursor::new(vec![]));

    let options =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let mut emitted_names = HashSet::new();

    for yaml in yamls {
        let player_name = yaml.sanitized_name();
        let mut original_file_name = format!("{}.yaml", player_name);

        let mut suffix = 0u64;
        if emitted_names.contains(&original_file_name) {
            loop {
                let new_file_name = format!("{}_{}.yaml", player_name, suffix);
                if !emitted_names.contains(&new_file_name) {
                    original_file_name = new_file_name;
                    break;
                }
                suffix += 1;
            }
        }
        writer.start_file(original_file_name.clone(), options)?;
        emitted_names.insert(original_file_name);
        writer.write_all(yaml.content.as_bytes())?;
    }

    let res = writer.finish()?;
    let value = format!(
        "attachment; filename=\"yamls-{}.zip\"",
        room.close_date.format("%Y-%m-%d_%H_%M_%S")
    );

    Ok(ZipFile {
        content: res.into_inner(),
        headers: Header::new(CONTENT_DISPOSITION.as_str(), value),
    })
}

#[derive(rocket::Responder)]
#[response(status = 200, content_type = "application/yaml")]
struct YamlContent<'a> {
    content: String,
    headers: Header<'a>,
}

#[get("/room/<room_id>/download/<yaml_id>")]
async fn download_yaml<'a>(
    redirect_to: &RedirectTo,
    room_id: Uuid,
    yaml_id: Uuid,
    ctx: &State<Context>,
) -> Result<YamlContent<'a>> {
    redirect_to.set("/");
    let _room = db::get_room(room_id, ctx)
        .await
        .context("Couldn't find the room")?;
    let yaml = db::get_yaml_by_id(yaml_id, ctx).await?;

    let value = format!("attachment; filename=\"{}.yaml\"", yaml.sanitized_name());

    Ok(YamlContent {
        content: yaml.content,
        headers: Header::new(CONTENT_DISPOSITION.as_str(), value),
    })
}

#[get("/static/<file..>")]
fn dist(file: PathBuf) -> Option<(ContentType, Cow<'static, [u8]>)> {
    let filename = file.display().to_string();
    let asset = Asset::get(&filename)?;
    let content_type = file
        .extension()
        .and_then(OsStr::to_str)
        .and_then(ContentType::from_extension)
        .unwrap_or(ContentType::Bytes);

    Some((content_type, asset.data))
}

#[derive(rust_embed::RustEmbed)]
#[folder = "./static/"]
struct Asset;

pub fn routes() -> Vec<rocket::Route> {
    routes![
        root,
        room,
        upload_yaml,
        delete_yaml,
        download_yamls,
        download_yaml,
        dist
    ]
}
