use crate::{
    db::get_username,
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
use rocket::{form::Form, http::uri::Host, http::Header, FromForm, Route, State};
use semver::Version;
use std::{
    collections::{HashMap, HashSet},
    str::FromStr,
    time::Duration,
};
use tokio::sync::RwLock;
use tracing_opentelemetry::OpenTelemetrySpanExt;
use wq::JobStatus;

use crate::jobs::{OptionsGenParams, OptionsGenQueue};

const WEIGHTED_TYPES: &[&str] = &[
    "bool",
    "choice",
    "named_range",
    "text_choice",
    "range",
    "text",
];

async fn get_default_player_name(session: &Session, ctx: &Context) -> String {
    let Some(user_id) = session.user_id else {
        return "Player{NUMBER}".to_string();
    };
    let mut conn = ctx.db_pool.get().await.unwrap();
    let Ok(Some(username)) = get_username(user_id, &mut conn).await else {
        return "Player{NUMBER}".to_string();
    };
    let truncated = username.chars().take(14).collect::<String>();
    format!("{}{{NUMBER}}", truncated)
}

pub type OptionsCache = std::sync::Arc<RwLock<HashMap<(String, Version), OptionsDef>>>;

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

    warnings: Vec<String>,
    prefilled_values: Option<HashMap<String, serde_json::Value>>,
    prefilled_player_name: Option<String>,
    prefilled_description: Option<String>,
    yaml_option_names: HashSet<String>,
    // Options whose prefilled values are no longer valid for the current definitions
    outdated_values: HashSet<String>,
    default_player_name: String,
}

impl OptionsTpl<'_> {
    fn get_prefilled(&self, option_name: &str) -> Option<&serde_json::Value> {
        self.prefilled_values
            .as_ref()
            .and_then(|pv| pv.get(option_name))
    }

    // Helper functions for template value extraction
    fn prefilled_bool(&self, prefilled: &Option<&serde_json::Value>, default: bool) -> bool {
        prefilled
            .and_then(|v| {
                v.as_bool().or_else(|| match v.as_str() {
                    Some("true") => Some(true),
                    Some("false") => Some(false),
                    _ => None,
                })
            })
            .unwrap_or(default)
    }

    fn prefilled_num(&self, prefilled: &Option<&serde_json::Value>) -> Option<i64> {
        prefilled.and_then(|v| {
            v.as_i64()
                .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        })
    }

    fn prefilled_str<'a>(&self, prefilled: &'a Option<&'a serde_json::Value>) -> Option<&'a str> {
        prefilled.and_then(|v| v.as_str())
    }

    fn prefilled_array<'a>(
        &self,
        prefilled: &'a Option<&'a serde_json::Value>,
    ) -> Option<&'a Vec<serde_json::Value>> {
        prefilled.and_then(|v| v.as_array())
    }

    fn prefilled_object<'a>(
        &self,
        prefilled: &'a Option<&'a serde_json::Value>,
    ) -> Option<&'a serde_json::Map<String, serde_json::Value>> {
        prefilled.and_then(|v| v.as_object())
    }

    fn array_contains_str(&self, arr: &[serde_json::Value], s: &str) -> bool {
        arr.iter().any(|v| v.as_str() == Some(s))
    }

    fn suggestions_contain(&self, suggestions: &[String], s: &str) -> bool {
        suggestions.iter().any(|sug| sug == s)
    }

    fn prefilled_is_suggestion(
        &self,
        prefilled: &Option<&serde_json::Value>,
        option_def: &crate::jobs::OptionDef,
    ) -> bool {
        let Some(pstr) = self.prefilled_str(prefilled) else {
            return false;
        };
        let Some(suggestions) = &option_def.suggestions else {
            return false;
        };
        suggestions.iter().any(|s| s == pstr)
    }

    fn default_is_suggestion(&self, option_def: &crate::jobs::OptionDef) -> bool {
        let Some(default_str) = option_def.default.as_str() else {
            return false;
        };
        let Some(suggestions) = &option_def.suggestions else {
            return false;
        };
        suggestions.iter().any(|s| s == default_str)
    }

    fn default_str<'a>(&self, option_def: &'a crate::jobs::OptionDef) -> Option<&'a str> {
        option_def.default.as_str()
    }

    fn is_weighted(
        &self,
        option_def: &crate::jobs::OptionDef,
        prefilled: &Option<&serde_json::Value>,
    ) -> bool {
        WEIGHTED_TYPES.contains(&option_def.ty.as_str())
            && prefilled.map(|v| v.is_object()).unwrap_or(false)
    }

    fn weights_json(&self, prefilled: &Option<&serde_json::Value>) -> String {
        prefilled
            .and_then(|v| v.as_object())
            .map(|obj| serde_json::to_string(obj).unwrap_or_default())
            .unwrap_or_default()
    }

    fn prefilled_json(&self, prefilled: &Option<&serde_json::Value>) -> String {
        prefilled
            .map(|v| serde_json::to_string(v).unwrap_or_default())
            .unwrap_or_default()
    }

    fn is_new_option(&self, option_name: &str) -> bool {
        self.prefilled_values.is_some() && !self.yaml_option_names.contains(option_name)
    }

    fn is_outdated(&self, option_name: &str) -> bool {
        self.outdated_values.contains(option_name)
    }
}

/// Helper to fetch OptionsDef, using cache if available or queuing a job if not.
#[tracing::instrument(skip(options_gen_queue, options_cache))]
pub(crate) async fn get_options_def(
    apworld_name: &str,
    version: &Version,
    options_gen_queue: &State<OptionsGenQueue>,
    options_cache: &State<OptionsCache>,
) -> Result<OptionsDef> {
    let cache_key = (apworld_name.to_string(), version.clone());

    {
        let cache = options_cache.read().await;
        if let Some(cached_options) = cache.get(&cache_key) {
            return Ok(cached_options.clone());
        }
    }

    let mut params = OptionsGenParams {
        apworld: (apworld_name.to_string(), version.clone()),
        otlp_context: HashMap::new(),
    };

    let cx = tracing::Span::current().context();
    opentelemetry::global::get_text_map_propagator(|propagator| {
        propagator.inject_context(&cx, &mut params.otlp_context)
    });

    let job_id = options_gen_queue
        .enqueue_job(&params, wq::Priority::High, Duration::from_secs(10))
        .await?;

    let Some(status) = options_gen_queue
        .wait_for_job(&job_id, Some(Duration::from_secs(10)))
        .await?
    else {
        tracing::error!(%job_id, %apworld_name, %version, "Options gen job timed out");
        Err(anyhow!(
            "The option definitions could not get fetched, try again in a bit"
        ))?
    };
    if matches!(status, JobStatus::InternalError) {
        tracing::error!(%job_id, %apworld_name, %version, "Options gen job returned internal error");
        Err(anyhow!(
            "There was an unexpected error while generating option definitions, try again."
        ))?
    }
    if matches!(status, JobStatus::Failure) {
        tracing::error!(%job_id, %apworld_name, %version, "Options gen job failed");
        Err(anyhow!("Generating option definitions failed, try again."))?
    }

    // The queue callback handles caching on success
    let cache = options_cache.read().await;
    cache
        .get(&cache_key)
        .cloned()
        .ok_or_else(|| anyhow!("Options not found in cache after successful job").into())
}

pub struct OptionsPreloadFairing;

#[rocket::async_trait]
impl rocket::fairing::Fairing for OptionsPreloadFairing {
    fn info(&self) -> rocket::fairing::Info {
        rocket::fairing::Info {
            name: "Options Preload",
            kind: rocket::fairing::Kind::Liftoff,
        }
    }

    #[tracing::instrument(name = "options_preload", skip_all)]
    async fn on_liftoff(&self, rocket: &rocket::Rocket<rocket::Orbit>) {
        if std::env::var("PRELOAD_OPTIONS_DEFS").is_err() {
            return;
        }

        let index_manager = rocket.state::<IndexManager>().unwrap();
        let options_gen_queue = rocket.state::<OptionsGenQueue>().unwrap();

        let worlds: Vec<_> = {
            let index = index_manager.index.read().await;
            index
                .worlds
                .iter()
                .filter_map(|(apworld_name, world)| {
                    let latest_version = world.versions.keys().max()?;
                    Some((apworld_name.clone(), latest_version.clone()))
                })
                .collect()
        };

        tracing::info!("Enqueuing options preload for {} worlds", worlds.len());

        for (apworld_name, version) in worlds {
            let mut params = OptionsGenParams {
                apworld: (apworld_name.clone(), version.clone()),
                otlp_context: HashMap::new(),
            };

            let cx = tracing::Span::current().context();
            opentelemetry::global::get_text_map_propagator(|propagator| {
                propagator.inject_context(&cx, &mut params.otlp_context)
            });

            if let Err(e) = options_gen_queue
                .enqueue_job(&params, wq::Priority::Low, Duration::from_secs(120))
                .await
            {
                tracing::warn!(%apworld_name, %version, %e, "Failed to enqueue preload job");
            }
        }
    }
}

#[rocket::get("/options/<apworld_name>/<version>")]
#[tracing::instrument(skip(
    options_gen_queue,
    options_cache,
    index_manager,
    ctx,
    session,
    redirect_to
))]
async fn options_gen_api<'a>(
    apworld_name: &'a str,
    version: String,
    options_gen_queue: &State<OptionsGenQueue>,
    options_cache: &State<OptionsCache>,
    index_manager: &'a State<IndexManager>,
    ctx: &'a State<Context>,
    session: Session,
    redirect_to: &RedirectTo,
) -> Result<OptionsTpl<'a>> {
    redirect_to.set("/options");
    let index = index_manager.index.read().await;
    let Some(apworld) = index.worlds.get(apworld_name) else {
        Err(anyhow!("Unknown apworld"))?
    };
    let mut apworlds: Vec<(String, String)> = index
        .worlds
        .iter()
        .map(|(apworld_name, world)| (apworld_name.clone(), world.display_name.clone()))
        .collect();
    apworlds.sort_by_key(|(_, world_name)| world_name.to_lowercase());
    let versions: Vec<String> = apworld
        .versions
        .keys()
        .map(|v| v.to_string())
        .rev()
        .collect();
    if !versions.contains(&version.to_string()) {
        Err(anyhow!("Unknown version for this apworld"))?
    }
    drop(index);

    let parsed_version = Version::from_str(&version)?;
    let options = get_options_def(
        apworld_name,
        &parsed_version,
        options_gen_queue,
        options_cache,
    )
    .await?;

    let default_player_name = get_default_player_name(&session, ctx).await;

    Ok(OptionsTpl {
        base: TplContext::from_session("options", session, ctx).await,
        apworlds,
        versions,
        selected_apworld: Some(apworld_name.to_string()),
        selected_version: Some(version.to_string()),
        options: Some(options),
        warnings: vec![],
        prefilled_values: None,
        prefilled_player_name: None,
        prefilled_description: None,
        yaml_option_names: HashSet::new(),
        outdated_values: HashSet::new(),
        default_player_name,
    })
}

#[rocket::get("/options/<apworld_name>")]
#[tracing::instrument(skip(
    options_gen_queue,
    options_cache,
    index_manager,
    ctx,
    session,
    redirect_to
))]
async fn options_apworld_versions<'a>(
    apworld_name: &'a str,
    index_manager: &'a State<IndexManager>,
    options_gen_queue: &State<OptionsGenQueue>,
    options_cache: &State<OptionsCache>,
    ctx: &'a State<Context>,
    session: Session,
    redirect_to: &RedirectTo,
) -> Result<OptionsTpl<'a>> {
    redirect_to.set("/options");
    let index = index_manager.index.read().await;
    let Some(apworld) = index.worlds.get(apworld_name) else {
        Err(anyhow!("Unknown apworld"))?
    };
    let versions: Vec<String> = apworld
        .versions
        .keys()
        .map(|v| v.to_string())
        .rev()
        .collect();
    let last_version = versions.first().unwrap().to_string();
    drop(index);

    options_gen_api(
        apworld_name,
        last_version,
        options_gen_queue,
        options_cache,
        index_manager,
        ctx,
        session,
        redirect_to,
    )
    .await
}

#[rocket::get("/options")]
#[tracing::instrument(skip(index_manager, ctx, session))]
async fn options_gen<'a>(
    index_manager: &State<IndexManager>,
    ctx: &'a State<Context>,
    session: Session,
) -> Result<OptionsTpl<'a>> {
    let index = index_manager.index.read().await;
    let mut apworlds: Vec<(String, String)> = index
        .worlds
        .iter()
        .map(|(apworld_name, world)| (apworld_name.clone(), world.display_name.clone()))
        .collect();
    apworlds.sort_by_key(|(_, world_name)| world_name.to_lowercase());

    let default_player_name = get_default_player_name(&session, ctx).await;

    Ok(OptionsTpl {
        base: TplContext::from_session("options", session, ctx).await,
        apworlds,
        versions: vec![],
        selected_apworld: None,
        selected_version: None,
        options: None,
        warnings: vec![],
        prefilled_values: None,
        prefilled_player_name: None,
        prefilled_description: None,
        yaml_option_names: HashSet::new(),
        outdated_values: HashSet::new(),
        default_player_name,
    })
}

#[derive(FromForm)]
struct YamlUpload<'a> {
    yaml: &'a str,
}

fn validate_option_value(value: &serde_json::Value, option_def: &crate::jobs::OptionDef) -> bool {
    // For weighted options, only validate non-zero weighted values
    if let Some(obj) = value.as_object() {
        return obj
            .iter()
            .filter(|(_, v)| v.as_i64().map(|n| n != 0).unwrap_or(true))
            .all(|(k, _)| validate_single_value(k, option_def));
    }

    if let Some(s) = value.as_str() {
        return validate_single_value(s, option_def);
    }
    if let Some(n) = value.as_i64() {
        return validate_numeric_value(n, option_def);
    }
    if let Some(arr) = value.as_array() {
        if !option_def.has_valid_keys() {
            return true;
        }
        let valid_keys = option_def.valid_keys();
        return arr.iter().all(|v| {
            v.as_str()
                .map(|s| valid_keys.iter().any(|k| k == s))
                .unwrap_or(true)
        });
    }
    true
}

fn validate_single_value(value: &str, option_def: &crate::jobs::OptionDef) -> bool {
    // "random", "random-low", "random-high", etc. are always valid
    if value.starts_with("random") {
        return true;
    }

    match option_def.ty.as_str() {
        "choice" => option_def
            .choices
            .as_ref()
            .map(|c| c.iter().any(|choice| choice == value))
            .unwrap_or(true),
        "text_choice" => option_def
            .suggestions
            .as_ref()
            .map(|s| s.iter().any(|sug| sug == value))
            .unwrap_or(true),
        "named_range" => {
            if let Some(suggestions) = &option_def.suggestions {
                if suggestions.iter().any(|s| s == value) {
                    return true;
                }
            }
            if let Ok(n) = value.parse::<i64>() {
                return validate_numeric_value(n, option_def);
            }
            false
        }
        _ => true,
    }
}

fn validate_numeric_value(value: i64, option_def: &crate::jobs::OptionDef) -> bool {
    match option_def.ty.as_str() {
        "range" | "named_range" => {
            if let Some((min, max)) = option_def.range {
                value >= min && value <= max
            } else {
                true
            }
        }
        _ => true,
    }
}

#[rocket::post("/options/edit", data = "<form>")]
#[tracing::instrument(skip(options_gen_queue, options_cache, index_manager, ctx, session, form))]
async fn edit_yaml<'a>(
    form: Form<YamlUpload<'a>>,
    options_gen_queue: &State<OptionsGenQueue>,
    options_cache: &State<OptionsCache>,
    index_manager: &'a State<IndexManager>,
    ctx: &'a State<Context>,
    redirect_to: &RedirectTo,
    session: Session,
) -> Result<OptionsTpl<'a>> {
    redirect_to.set("/options");

    let yaml: serde_json::Value =
        serde_saphyr::from_str(form.yaml).map_err(|e| anyhow!("Failed to parse YAML: {}", e))?;

    let player_name = yaml
        .get("name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let description = yaml
        .get("description")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let game_field = yaml
        .get("game")
        .ok_or_else(|| anyhow!("YAML is missing 'game' field"))?;

    let game_name = if let Some(s) = game_field.as_str() {
        s.to_string()
    } else if game_field.is_object() {
        return Err(anyhow!(
            "Weighted game randomizers are not supported. Please use a YAML with a single game."
        )
        .into());
    } else {
        return Err(anyhow!("Invalid 'game' field in YAML").into());
    };

    let index = index_manager.index.read().await;
    let (apworld_name, latest_version) = index
        .worlds
        .iter()
        .find(|(_, world)| world.name == game_name)
        .map(|(name, world)| {
            let latest = world.versions.keys().max().unwrap().clone();
            (name.clone(), latest)
        })
        .ok_or_else(|| anyhow!("Game '{}' not found", game_name))?;

    let mut apworlds: Vec<(String, String)> = index
        .worlds
        .iter()
        .map(|(apworld_name, world)| (apworld_name.clone(), world.display_name.clone()))
        .collect();
    apworlds.sort_by_key(|(_, world_name)| world_name.to_lowercase());

    let versions: Vec<String> = index
        .worlds
        .get(&apworld_name)
        .unwrap()
        .versions
        .keys()
        .map(|v| v.to_string())
        .rev()
        .collect();
    drop(index);

    let options = get_options_def(
        &apworld_name,
        &latest_version,
        options_gen_queue,
        options_cache,
    )
    .await?;

    let option_defs: HashMap<&String, &crate::jobs::OptionDef> = options
        .iter()
        .flat_map(|(_, group_options)| group_options.iter())
        .collect();

    let game_options = yaml.get(&game_name).and_then(|v| v.as_object());

    let mut prefilled_values: HashMap<String, serde_json::Value> = HashMap::new();
    let mut warnings: Vec<String> = vec![];
    let mut yaml_option_names: HashSet<String> = HashSet::new();
    let mut outdated_values: HashSet<String> = HashSet::new();

    if let Some(game_opts) = game_options {
        for (key_str, value) in game_opts {
            yaml_option_names.insert(key_str.clone());

            let Some(option_def) = option_defs.get(key_str) else {
                warnings.push(key_str.clone());
                continue;
            };

            let is_weighted_type = WEIGHTED_TYPES.contains(&option_def.ty.as_str());
            if value.is_object()
                && option_def.ty != "counter"
                && option_def.ty != "dict"
                && !is_weighted_type
            {
                warnings.push(format!(
                    "{} (weighted options not supported for this type)",
                    key_str
                ));
                continue;
            }

            let coalesced = value.as_object().and_then(|mapping| {
                if !is_weighted_type {
                    return None;
                }
                let non_zero: Vec<_> = mapping
                    .iter()
                    .filter(|(_, v)| v.as_i64().map(|n| n != 0).unwrap_or(true))
                    .collect();
                if non_zero.len() == 1 {
                    Some(serde_json::Value::String(non_zero[0].0.clone()))
                } else {
                    None
                }
            });

            let json_value = coalesced.unwrap_or_else(|| value.clone());

            if option_def.is_editable() && !validate_option_value(&json_value, option_def) {
                outdated_values.insert(key_str.clone());
                // If invalid, don't insert it in prefill so it gets reset
                continue;
            }

            prefilled_values.insert(key_str.clone(), json_value);
        }
    }

    // If we have a prefilled player name, then the default doesn't matter since it won't be used
    let default_player_name = match &player_name {
        Some(_) => "Player{NUMBER}".to_string(),
        None => get_default_player_name(&session, ctx).await,
    };

    Ok(OptionsTpl {
        base: TplContext::from_session("options", session, ctx).await,
        apworlds,
        versions,
        selected_apworld: Some(apworld_name),
        selected_version: Some(latest_version.to_string()),
        options: Some(options),
        warnings,
        prefilled_values: Some(prefilled_values),
        prefilled_player_name: player_name,
        prefilled_description: description,
        yaml_option_names,
        outdated_values,
        default_player_name,
    })
}

#[rocket::post("/options/<apworld_name>/<version>/download", data = "<form>")]
#[tracing::instrument(skip(host, index_manager, form, options_gen_queue, options_cache))]
async fn download_yaml<'a>(
    apworld_name: &str,
    version: &str,
    host: &Host<'_>,
    form: Form<HashMap<String, String>>,
    index_manager: &State<IndexManager>,
    options_gen_queue: &State<OptionsGenQueue>,
    options_cache: &State<OptionsCache>,
) -> Result<YamlDownload<'a>> {
    let game_name = {
        let index = index_manager.index.read().await;
        let Some(apworld) = index.worlds.get(apworld_name) else {
            Err(anyhow!("Unknown apworld"))?
        };
        apworld.name.clone()
    };

    let parsed_version = Version::from_str(version)?;
    let options = get_options_def(
        apworld_name,
        &parsed_version,
        options_gen_queue,
        options_cache,
    )
    .await?;

    let player_name = form.get("player").map(|s| s.as_str()).unwrap_or("Player");
    let player_name = if player_name.is_empty() {
        "Player"
    } else {
        player_name
    };

    let description = form
        .get("description")
        .filter(|s| !s.is_empty())
        .cloned()
        .unwrap_or_else(|| format!("Generated on https://{}/options/{}", host, apworld_name));

    // Build game options in definition order
    let mut game_options: IndexMap<String, serde_json::Value> = IndexMap::new();
    for (_group_name, group_options) in options.iter() {
        for (option_name, _option_def) in group_options.iter() {
            if let Some(value) = form.get(option_name) {
                // Try to parse as JSON for complex types (lists, counters, bools, numbers)
                let parsed = serde_json::from_str::<serde_json::Value>(value)
                    .unwrap_or_else(|_| serde_json::Value::String(value.clone()));
                game_options.insert(option_name.clone(), parsed);
            }
        }
    }

    // Build the full YAML structure
    let mut root: IndexMap<String, serde_json::Value> = IndexMap::new();
    root.insert(
        "game".to_string(),
        serde_json::Value::String(game_name.clone()),
    );
    root.insert(
        "name".to_string(),
        serde_json::Value::String(player_name.to_string()),
    );
    root.insert(
        "description".to_string(),
        serde_json::Value::String(description),
    );
    root.insert(game_name.clone(), serde_json::to_value(&game_options)?);

    let yaml = serde_saphyr::to_string(&root)?;

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
        edit_yaml,
        download_yaml,
    ]
}
