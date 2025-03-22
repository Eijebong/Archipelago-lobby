use std::{
    collections::{HashMap, HashSet},
    path::Path,
    time::Duration,
};

use crate::{
    db::{
        self, Generation, GenerationStatus, Room, RoomId, Yaml, YamlFile, YamlGame,
        YamlValidationStatus,
    },
    error::{ApiResult, RedirectTo},
    index_manager::IndexManager,
    jobs::{GenerationOutDir, GenerationParams, GenerationQueue},
    session::LoggedInSession,
};
use apwm::Index;
use askama::Template;
use diesel_async::AsyncPgConnection;
use http::header::CONTENT_DISPOSITION;
use itertools::Itertools;
use rocket::tokio::fs::File;
use rocket::{fs::NamedFile, http::Header, State};
use rocket::{
    futures::stream::Stream,
    response::{stream::ByteStream, Redirect},
};
use tokio::io::AsyncReadExt;
use tracing_opentelemetry::OpenTelemetrySpanExt;
use wq::{JobId, JobStatus};

use crate::error::Result;
use crate::utils::RenamedFile;
use crate::{Context, TplContext};

#[derive(Template)]
#[template(path = "room_gen.html")]
struct GenRoomTpl<'a> {
    base: TplContext<'a>,
    room: Room,
    generation_checklist: HashMap<&'a str, bool>,
    current_gen: Option<Generation>,
}

#[derive(Default)]
struct GenerationInfo {
    pub log_file: Option<String>,
    pub output_file: Option<String>,
}

#[rocket::get("/room/<room_id>/generation")]
async fn gen_room(
    room_id: RoomId,
    session: LoggedInSession,
    ctx: &State<Context>,
) -> Result<GenRoomTpl> {
    let mut conn = ctx.db_pool.get().await?;
    let room = db::get_room(room_id, &mut conn).await?;
    let is_my_room = session.0.is_admin || session.user_id() == room.settings.author_id;

    if !is_my_room {
        Err(anyhow::anyhow!(
            "Cannot access generation for a room that isn't yours"
        ))?
    }

    let (generation_checklist, _) = get_info_for_gen(&room, &mut conn).await?;
    let current_gen = db::get_generation_for_room(room_id, &mut conn).await?;

    Ok(GenRoomTpl {
        base: TplContext::from_session("room", session.0, ctx).await,
        generation_checklist,
        room,
        current_gen,
    })
}

#[rocket::get("/room/<room_id>/generation/status")]
async fn gen_room_status<'a>(
    _ws: rocket_ws::WebSocket,
    room_id: RoomId,
    session: LoggedInSession,
    gen_queue: &'a State<GenerationQueue>,
    ctx: &'a State<Context>,
) -> ApiResult<rocket_ws::Stream!['a]> {
    let mut conn = ctx.db_pool.get().await?;

    let room = db::get_room(room_id, &mut conn).await?;
    let is_my_room = session.0.is_admin || session.user_id() == room.settings.author_id;

    if !is_my_room {
        Err(anyhow::anyhow!(
            "Cannot cancel generation for a room that isn't yours"
        ))?
    }

    let Some(current_gen) = db::get_generation_for_room(room_id, &mut conn).await? else {
        Err(anyhow::anyhow!(
            "Cannot get generatio info, there's none in progress"
        ))?
    };

    fn job_status_to_client_str(job_status: &JobStatus) -> &str {
        match job_status {
            JobStatus::Pending => "pending",
            JobStatus::Running => "running",
            JobStatus::Success => "success",
            JobStatus::Failure => "failure",
            JobStatus::InternalError => "error",
        }
    }

    Ok({
        rocket_ws::Stream! { _ws =>
            loop {
                if let Some(job_status) = gen_queue.get_job_status(&current_gen.job_id).await.unwrap() {
                    yield rocket_ws::Message::Text(job_status_to_client_str(&job_status).to_string());
                    if job_status.is_resolved() {
                        break;
                    }
                } else {
                    let Ok(Some(current_gen)) = db::get_generation_for_room(room_id, &mut conn).await else {
                        yield rocket_ws::Message::Text(job_status_to_client_str(&JobStatus::InternalError).to_string());
                        break;
                    };

                    let job_status = match current_gen.status {GenerationStatus::Done=>JobStatus::Success,
                    GenerationStatus::Pending => JobStatus::Pending,
                    GenerationStatus::Running => JobStatus::Running,
                    GenerationStatus::Failed => JobStatus::Failure, };
                    yield rocket_ws::Message::Text(job_status_to_client_str(&job_status).to_string());
                    break;
                }

                tokio::time::sleep(Duration::from_millis(500)).await
            }
        }
    })
}

#[rocket::get("/room/<room_id>/generation/start")]
async fn gen_room_start(
    room_id: RoomId,
    session: LoggedInSession,
    redirect_to: &RedirectTo,
    gen_queue: &State<GenerationQueue>,
    index_manager: &State<IndexManager>,
    ctx: &State<Context>,
) -> Result<Redirect> {
    redirect_to.set(&format!("/room/{}/generation", room_id));

    let mut conn = ctx.db_pool.get().await?;

    let room = db::get_room(room_id, &mut conn).await?;
    let is_my_room = session.0.is_admin || session.user_id() == room.settings.author_id;

    if !is_my_room {
        Err(anyhow::anyhow!(
            "Cannot access generation for a room that isn't yours"
        ))?
    }

    let (checklist, yamls) = get_info_for_gen(&room, &mut conn).await?;
    for (label, ok) in checklist {
        if !ok {
            Err(anyhow::anyhow!(
                "Cannot start generation because the following condition isn't met: {}",
                label
            ))?
        }
    }

    let index = index_manager.index.read().await.clone();
    enqueue_gen_job(&room, yamls, gen_queue, &index, &mut conn).await?;

    Ok(Redirect::to(rocket::uri!(gen_room(room_id))))
}

#[rocket::get("/room/<room_id>/generation/cancel")]
async fn gen_room_cancel(
    room_id: RoomId,
    session: LoggedInSession,
    redirect_to: &RedirectTo,
    gen_queue: &State<GenerationQueue>,
    ctx: &State<Context>,
) -> Result<Redirect> {
    redirect_to.set(&format!("/room/{}/generation", room_id));

    let mut conn = ctx.db_pool.get().await?;

    let room = db::get_room(room_id, &mut conn).await?;
    let is_my_room = session.0.is_admin || session.user_id() == room.settings.author_id;

    if !is_my_room {
        Err(anyhow::anyhow!(
            "Cannot cancel generation for a room that isn't yours"
        ))?
    }

    let Some(current_gen) = db::get_generation_for_room(room_id, &mut conn).await? else {
        Err(anyhow::anyhow!(
            "Cannot cancel generation, there's none in progress"
        ))?
    };

    gen_queue.cancel_job(current_gen.job_id).await?;
    db::delete_generation_for_room(room_id, &mut conn).await?;

    Ok(Redirect::to(rocket::uri!(gen_room(room_id))))
}

#[rocket::get("/room/<room_id>/generation/logs")]
async fn gen_room_logs<'a>(
    room_id: RoomId,
    session: LoggedInSession,
    generation_out_dir: &State<GenerationOutDir>,
    ctx: &'a State<Context>,
) -> Result<RenamedFile<'a>> {
    let mut conn = ctx.db_pool.get().await?;
    let room = db::get_room(room_id, &mut conn).await?;
    let is_my_room = session.0.is_admin || session.user_id() == room.settings.author_id;

    if !is_my_room {
        Err(anyhow::anyhow!(
            "Cannot view logs for a generation that's not yours"
        ))?
    }

    let Some(gen) = db::get_generation_for_room(room_id, &mut conn).await? else {
        Err(anyhow::anyhow!(
            "There's no generation running for this room"
        ))?
    };

    let generation_info = get_generation_info(gen.job_id, &generation_out_dir.inner().0)?;
    let Some(output_path) = generation_info.log_file else {
        Err(anyhow::anyhow!("There's no output for this room"))?
    };

    let complete_out_path = generation_out_dir
        .inner()
        .0
        .join(gen.job_id.to_string())
        .join(&output_path);

    return Ok(RenamedFile {
        inner: NamedFile::open(&complete_out_path).await?,
        headers: Header::new(CONTENT_DISPOSITION.as_str(), "inline"),
    });
}

#[rocket::get("/room/<room_id>/generation/logs/stream")]
async fn gen_room_logs_stream<'a>(
    room_id: RoomId,
    session: LoggedInSession,
    generation_out_dir: &'a State<GenerationOutDir>,
    generation_queue: &'a State<GenerationQueue>,
    ctx: &'a State<Context>,
) -> ApiResult<ByteStream<impl Stream<Item = Vec<u8>> + use<'a>>> {
    let mut conn = ctx.db_pool.get().await?;

    let room = db::get_room(room_id, &mut conn).await?;
    let is_my_room = session.0.is_admin || session.user_id() == room.settings.author_id;

    if !is_my_room {
        Err(anyhow::anyhow!(
            "Cannot view logs for a generation that's not yours"
        ))?
    }

    let Some(gen) = db::get_generation_for_room(room_id, &mut conn).await? else {
        Err(anyhow::anyhow!(
            "There's no generation running for this room"
        ))?
    };

    Ok(ByteStream! {
        let mut file = loop {
            let Ok(generation_info) = get_generation_info(gen.job_id, &generation_out_dir.inner().0) else {
                tokio::time::sleep(Duration::from_millis(500)).await;
                continue;
            };
            let Some(output_path) = generation_info.log_file else {
                tokio::time::sleep(Duration::from_millis(500)).await;
                continue;
            };

            let complete_out_path = generation_out_dir
                .inner()
                .0
                .join(gen.job_id.to_string())
                .join(&output_path);


            let Ok(file) = File::open(&complete_out_path).await else {
                return;
            };

            break file;
        };

        loop {

            let mut buf = Vec::with_capacity(8192);
            let Ok(n) = file.read_buf(&mut buf).await else {
                break;
            };

            if n != 0 {
                yield buf
            } else {
                let Ok(Some(status)) = generation_queue.get_job_status(&gen.job_id).await else {
                    break;
                };

                if status.is_resolved() {
                    break
                }

                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }
    })
}

#[rocket::get("/room/<room_id>/generation/output")]
async fn gen_room_output<'a>(
    room_id: RoomId,
    session: LoggedInSession,
    generation_out_dir: &State<GenerationOutDir>,
    ctx: &'a State<Context>,
) -> ApiResult<RenamedFile<'a>> {
    let mut conn = ctx.db_pool.get().await?;
    let room = db::get_room(room_id, &mut conn).await?;
    let is_my_room = session.0.is_admin || session.user_id() == room.settings.author_id;

    if !is_my_room {
        Err(anyhow::anyhow!(
            "Cannot get output for a generation that's not yours"
        ))?
    }

    let Some(gen) = db::get_generation_for_room(room_id, &mut conn).await? else {
        Err(anyhow::anyhow!(
            "There's no generation running for this room"
        ))?
    };

    let generation_info = get_generation_info(gen.job_id, &generation_out_dir.inner().0)?;
    let Some(output_path) = generation_info.output_file else {
        Err(anyhow::anyhow!("There's no output for this room"))?
    };

    let complete_out_path = generation_out_dir
        .inner()
        .0
        .join(gen.job_id.to_string())
        .join(&output_path);

    let value = format!("attachment; filename=\"output_{}.zip\"", room.id);
    return Ok(RenamedFile {
        inner: NamedFile::open(&complete_out_path).await?,
        headers: Header::new(CONTENT_DISPOSITION.as_str(), value),
    });
}

async fn enqueue_gen_job(
    room: &Room,
    yamls: Vec<Yaml>,
    gen_queue: &State<GenerationQueue>,
    index: &Index,
    conn: &mut AsyncPgConnection,
) -> Result<JobId> {
    let required_worlds = yamls
        .iter()
        .map(|yaml| {
            let Ok(parsed) = serde_yaml::from_str::<YamlFile>(&yaml.content) else {
                Err(anyhow::anyhow!(
                    "Internal error, unable to reparse a YAML that was already parsed before"
                ))?
            };

            Ok(match parsed.game {
                YamlGame::Name(name) => vec![name],
                YamlGame::Map(names) => names.keys().cloned().collect(),
            })
        })
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect::<HashSet<_>>();

    // Freeze the room manifest to the status it is when we gen
    let mut current_manifest = room.settings.manifest.0.clone();
    current_manifest.freeze(index)?;
    db::update_room_manifest(room.id, &current_manifest, conn).await?;

    let apworlds = current_manifest
        .resolve_with(index)
        .0
        .into_iter()
        .filter_map(|(_, (world, version))| {
            if !required_worlds.contains(&world.name) {
                return None;
            }

            Some((world.get_ap_name().unwrap(), version))
        })
        .collect::<Vec<_>>();

    let mut params = GenerationParams {
        apworlds,
        meta_file: room.settings.meta_file.clone(),
        room_id: room.id,
        otlp_context: HashMap::new(),
    };

    let cx = tracing::Span::current().context();
    opentelemetry::global::get_text_map_propagator(|propagator| {
        propagator.inject_context(&cx, &mut params.otlp_context)
    });

    let job_id = gen_queue
        .enqueue_job(
            &params,
            wq::Priority::Normal,
            Duration::from_secs(60 * 60 * 6),
        )
        .await?;
    db::insert_generation_for_room(room.id, job_id, conn).await?;

    Ok(job_id)
}

async fn get_info_for_gen(
    room: &Room,
    conn: &mut AsyncPgConnection,
) -> Result<(HashMap<&'static str, bool>, Vec<Yaml>)> {
    let mut generation_checklist = HashMap::new();
    generation_checklist.insert("The room must be closed", room.is_closed());

    let yamls = db::get_yamls_for_room(room.id, conn).await?;
    let current_generation = db::get_generation_for_room(room.id, conn).await?;

    generation_checklist.insert(
        "All YAMLs files must have been validated",
        yamls
            .iter()
            .all(|yaml| yaml.validation_status == YamlValidationStatus::Validated),
    );
    let yaml_count = yamls.len();
    generation_checklist.insert(
        "The room must contain between 1 and 50 YAMLs",
        yaml_count > 0 && yaml_count <= 50,
    );
    generation_checklist.insert(
        "There must be no generation in progress",
        current_generation.is_none(),
    );

    Ok((generation_checklist, yamls))
}

fn get_generation_info(job_id: JobId, output_dir: &Path) -> Result<GenerationInfo> {
    let mut log_file = None;
    let mut output_file = None;

    let gen_out_path = output_dir.join(job_id.to_string());

    let Ok(entries) = gen_out_path.read_dir() else {
        return Ok(GenerationInfo::default());
    };

    for entry in entries {
        let entry = entry?;
        let file_name = entry
            .file_name()
            .into_string()
            .expect("Failed to read dir entry");
        if file_name.ends_with(".zip") {
            output_file = Some(file_name.clone());
        }
        if file_name.ends_with(".log") {
            log_file = Some(file_name.clone());
        }
    }

    Ok(GenerationInfo {
        log_file,
        output_file,
    })
}

pub fn routes() -> Vec<rocket::Route> {
    rocket::routes![
        gen_room,
        gen_room_start,
        gen_room_cancel,
        gen_room_logs,
        gen_room_logs_stream,
        gen_room_output,
        gen_room_status,
    ]
}
