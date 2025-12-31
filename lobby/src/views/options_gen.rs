use crate::{
    error::{RedirectTo, Result},
    index_manager::IndexManager,
    jobs::OptionsDef,
    session::Session,
    Context, TplContext,
};
use anyhow::anyhow;
use askama::Template;
use askama_web::WebTemplate;
use http::header::CONTENT_DISPOSITION;
use indexmap::IndexMap;
use rocket::{form::Form, http::uri::Host, http::Header, Route, State};
use semver::Version;
use std::{collections::HashMap, str::FromStr, time::Duration};
use tracing_opentelemetry::OpenTelemetrySpanExt;
use wq::JobStatus;

use crate::jobs::{OptionsGenParams, OptionsGenQueue};

#[derive(rocket::Responder)]
#[response(status = 200, content_type = "application/yaml")]
pub struct YamlDownload<'a> {
    pub content: String,
    pub headers: Header<'a>,
}

#[derive(Template, WebTemplate)]
#[template(path = "options.html")]
struct OptionsTpl<'a> {
    base: TplContext<'a>,
    // apworld_name, world_name
    apworlds: Vec<(String, String)>,
    versions: Vec<String>,
    selected_apworld: Option<String>,
    selected_version: Option<String>,
    options: Option<OptionsDef>,
}

#[rocket::get("/options/<apworld_name>/<version>")]
async fn options_gen_api<'a>(
    apworld_name: &'a str,
    version: String,
    options_gen_queue: &State<OptionsGenQueue>,
    index_manager: &'a State<IndexManager>,
    ctx: &'a State<Context>,
    session: Session,
    redirect_to: &RedirectTo,
) -> Result<OptionsTpl<'a>> {
    redirect_to.set("/options");
    let index = index_manager.index.read().await;
    let Some(apworld) = index.worlds.get(apworld_name) else {
        Err(anyhow!("Unkown apworld"))?
    };
    let mut apworlds: Vec<(String, String)> = index
        .worlds
        .iter()
        .map(|(apworld_name, world)| (apworld_name.clone(), world.name.clone()))
        .collect();
    apworlds.sort_by_key(|(_, world_name)| world_name.to_lowercase());
    let versions: Vec<String> = apworld
        .versions
        .keys()
        .map(|v| v.to_string())
        .rev()
        .collect();
    if !versions.contains(&version.to_string()) {
        Err(anyhow!("Unkown version for this apworld"))?
    }

    // TODO: Cache these per apworld/version, maybe on start even? Maybe on refresh?
    let mut params = OptionsGenParams {
        apworld: (apworld_name.to_string(), Version::from_str(&version)?),
        otlp_context: HashMap::new(),
    };

    let cx = tracing::Span::current().context();
    opentelemetry::global::get_text_map_propagator(|propagator| {
        propagator.inject_context(&cx, &mut params.otlp_context)
    });

    let job_id = options_gen_queue
        .enqueue_job(&params, wq::Priority::High, Duration::from_secs(60))
        .await?;

    let Some(status) = options_gen_queue
        .wait_for_job(&job_id, Some(Duration::from_secs(60)))
        .await?
    else {
        Err(anyhow!(
            "The option definitions could not get fetched, try again in a bit"
        ))?
    };
    if matches!(status, JobStatus::InternalError) {
        options_gen_queue.cancel_job(job_id).await?;
        Err(anyhow!(
            "There was an unexpected error while generating option definitions, try again."
        ))?
    }
    if matches!(status, JobStatus::Failure) {
        dbg!(options_gen_queue.get_job_result(job_id).await?.error);
        options_gen_queue.delete_job_result(job_id).await?;
        Err(anyhow!("Generating option definitions failed, try again."))?
    }

    assert_eq!(status, JobStatus::Success);
    let options = options_gen_queue.get_job_result(job_id).await?;
    options_gen_queue.delete_job_result(job_id).await?;

    Ok(OptionsTpl {
        base: TplContext::from_session("options", session, ctx).await,
        apworlds,
        versions,
        selected_apworld: Some(apworld_name.to_string()),
        selected_version: Some(version.to_string()),
        options: Some(options.options),
    })
}

#[rocket::get("/options/<apworld_name>")]
async fn options_apworld_versions<'a>(
    apworld_name: &'a str,
    index_manager: &'a State<IndexManager>,
    options_gen_queue: &State<OptionsGenQueue>,
    ctx: &'a State<Context>,
    session: Session,
    redirect_to: &RedirectTo,
) -> Result<OptionsTpl<'a>> {
    redirect_to.set("/options");
    let index = index_manager.index.read().await;
    let Some(apworld) = index.worlds.get(apworld_name) else {
        Err(anyhow!("Unkown apworld"))?
    };
    let mut apworlds: Vec<(String, String)> = index
        .worlds
        .iter()
        .map(|(apworld_name, world)| (apworld_name.clone(), world.name.clone()))
        .collect();
    apworlds.sort_by_key(|(_, world_name)| world_name.to_lowercase());
    let versions: Vec<String> = apworld
        .versions
        .keys()
        .map(|v| v.to_string())
        .rev()
        .collect();
    let last_version = versions.first().unwrap().to_string();

    options_gen_api(
        apworld_name,
        last_version,
        options_gen_queue,
        index_manager,
        ctx,
        session,
        redirect_to,
    )
    .await
}

#[rocket::get("/options")]
async fn options_gen<'a>(
    index_manager: &State<IndexManager>,
    ctx: &'a State<Context>,
    session: Session,
) -> Result<OptionsTpl<'a>> {
    let index = index_manager.index.read().await;
    let mut apworlds: Vec<(String, String)> = index
        .worlds
        .iter()
        .map(|(apworld_name, world)| (apworld_name.clone(), world.name.clone()))
        .collect();
    apworlds.sort_by_key(|(_, world_name)| world_name.to_lowercase());

    Ok(OptionsTpl {
        base: TplContext::from_session("options", session, ctx).await,
        apworlds,
        versions: vec![],
        selected_apworld: None,
        selected_version: None,
        options: None,
    })
}

#[rocket::post("/options/<apworld_name>/<_version>/download", data = "<form>")]
async fn download_yaml<'a>(
    apworld_name: &str,
    _version: &str,
    host: &Host<'_>,
    form: Form<HashMap<String, String>>,
    index_manager: &State<IndexManager>,
) -> Result<YamlDownload<'a>> {
    let index = index_manager.index.read().await;
    let Some(apworld) = index.worlds.get(apworld_name) else {
        Err(anyhow!("Unknown apworld"))?
    };
    let game_name = &apworld.name;

    let player_name = form.get("player").map(|s| s.as_str()).unwrap_or("Player");
    let player_name = if player_name.is_empty() {
        "Player"
    } else {
        player_name
    };

    // Build game options
    let mut game_options: IndexMap<String, serde_json::Value> = IndexMap::new();
    for (key, value) in form.iter() {
        if key == "player" {
            continue;
        }
        // Try to parse as JSON for complex types (lists, counters, bools, numbers)
        let parsed = serde_json::from_str::<serde_json::Value>(value)
            .unwrap_or_else(|_| serde_json::Value::String(value.clone()));
        game_options.insert(key.clone(), parsed);
    }

    // Build the full YAML structure
    let mut root: IndexMap<String, serde_yaml::Value> = IndexMap::new();
    root.insert(
        "game".to_string(),
        serde_yaml::Value::String(game_name.clone()),
    );
    root.insert(
        "name".to_string(),
        serde_yaml::Value::String(player_name.to_string()),
    );
    root.insert(
        "description".to_string(),
        serde_yaml::Value::String(format!(
            "Generated on https://{}/options/{}",
            host, apworld_name
        )),
    );
    root.insert(game_name.clone(), serde_yaml::to_value(&game_options)?);

    let yaml = serde_yaml::to_string(&root)?;

    let value = format!("attachment; filename=\"{}.yaml\"", player_name);
    Ok(YamlDownload {
        content: yaml,
        headers: Header::new(CONTENT_DISPOSITION.as_str(), value),
    })
}

pub fn routes() -> Vec<Route> {
    rocket::routes![
        options_apworld_versions,
        options_gen_api,
        options_gen,
        download_yaml,
    ]
}
