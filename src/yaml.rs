use crate::db::{self, Room, Yaml, YamlFile, YamlGame, YamlValidationStatus};
use crate::error::{Error, Result, WithContext};
use crate::extractor::YamlFeatures;
use crate::jobs::{YamlValidationParams, YamlValidationQueue};
use crate::session::LoggedInSession;

use crate::index_manager::IndexManager;
use anyhow::anyhow;
use apwm::{Manifest, World};
use chrono::Utc;
use counter::Counter;
use diesel_async::AsyncPgConnection;
use itertools::Itertools;
use once_cell::sync::Lazy;
use regex::Regex;

use rocket::http::CookieJar;
use rocket::State;
use semver::Version;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::BufReader;
use std::time::Duration;
use tracing_opentelemetry::OpenTelemetrySpanExt;
use wq::JobStatus;

pub fn parse_raw_yamls(yamls: &[&str]) -> Result<Vec<(String, YamlFile)>> {
    let yaml = yamls
        .iter()
        .map(|yaml| {
            yaml.trim()
                .trim_start_matches("---")
                .trim_end_matches("---")
        })
        .join("\n---\n");

    let reader = BufReader::new(yaml.as_bytes());
    let documents = yaml_split::DocumentIterator::new(reader);

    let documents = documents
        .into_iter()
        .map(|doc| {
            let Ok(doc) = doc else {
                anyhow::bail!("Invalid yaml file. Syntax error.")
            };

            let doc = doc.trim_start_matches('\u{feff}').to_string();
            let Ok(parsed) = serde_yaml::from_str(&doc) else {
                anyhow::bail!(
                    "This does not look like an archipelago YAML. Check that your YAML syntax is valid."
                )
            };
            Ok((doc, parsed))
        })
        .collect::<anyhow::Result<Vec<(String, YamlFile)>>>()?;

    Ok(documents)
}

pub struct YamlValidationResult<'a> {
    pub game_name: String,
    pub document: &'a String,
    pub parsed: &'a YamlFile,
    pub features: YamlFeatures,
    pub validation_status: YamlValidationStatus,
    pub apworlds: Vec<(String, Version)>,
    pub error: Option<String>,
}

#[tracing::instrument(skip_all)]
pub async fn parse_and_validate_yamls_for_room<'a>(
    room: &Room,
    documents: &'a [(String, YamlFile)],
    session: &mut LoggedInSession,
    cookies: &CookieJar<'_>,
    yaml_validation_queue: &YamlValidationQueue,
    index_manager: &IndexManager,
    conn: &mut AsyncPgConnection,
) -> Result<Vec<YamlValidationResult<'a>>> {
    let yamls_in_room = db::get_yamls_for_room(room.id, conn)
        .await
        .context("Couldn't get room yamls")?;

    let mut own_games_nb = yamls_in_room
        .iter()
        .filter(|yaml| Some(yaml.owner_id) == session.0.user_id)
        .count() as i32;

    let mut player_counter = Counter::new();

    let mut players_in_room = yamls_in_room
        .iter()
        .map(|yaml| get_ap_player_name(&yaml.player_name, &mut player_counter))
        .collect::<HashSet<String>>();

    let mut games = Vec::with_capacity(documents.len());

    for (document, parsed) in documents.iter() {
        if let Some(yaml_limit_per_user) = room.settings.yaml_limit_per_user {
            let allow_bypass = session.0.is_admin
                || room
                    .settings
                    .yaml_limit_bypass_list
                    .contains(&session.user_id());
            if own_games_nb >= yaml_limit_per_user && !allow_bypass {
                return Err(anyhow::anyhow!(format!(
                    "The room only allows {} game(s) per person. Cannot upload.",
                    yaml_limit_per_user
                ))
                .into());
            }
        }
        let player_name =
            validate_player_name(&parsed.name, &players_in_room, &mut player_counter)?;
        players_in_room.insert(player_name);

        let game_name = validate_game(&parsed.game)?;

        let (apworlds, validation_status, error) = if room.settings.yaml_validation {
            let validation_result = validate_yaml(
                document,
                parsed,
                &room.settings.manifest,
                index_manager,
                yaml_validation_queue,
            )
            .await?;

            match validation_result {
                YamlValidationJobResult::Success(apworlds) => {
                    (apworlds, YamlValidationStatus::Validated, None)
                }
                YamlValidationJobResult::Failure(apworlds, error) => {
                    if room.settings.allow_invalid_yamls {
                        session.0.warning_msg.push(format!(
                            "Invalid YAML:\n{}\n Uploading anyway since the room owner allowed it.",
                            error
                        ));
                        session.0.save(cookies)?;
                        (apworlds, YamlValidationStatus::Failed, Some(error))
                    } else {
                        Err(anyhow::anyhow!(error))?
                    }
                }
                YamlValidationJobResult::Unsupported(worlds) => {
                    let index = index_manager.index.read().await;
                    let error = format!("Error: {}",
                        worlds.iter().map(|game| {
                            let is_game_in_index = index.get_world_by_name(game).is_some();
                            if is_game_in_index {
                                format!("Uploaded a game for game {} which has been disabled for this room", game)
                            } else {
                                format!("Uploaded a game for game {} which is not supported on this lobby", game)
                            }
                        }).join("\n")
                    );

                    if room.settings.allow_unsupported {
                        session.0.warning_msg.push(format!(
                            "Uploaded a YAML with unsupported games: {}.\n Couldn't verify it.",
                            worlds.iter().join("; ")
                        ));
                        session.0.save(cookies)?;

                        (vec![], YamlValidationStatus::Unsupported, Some(error))
                    } else {
                        Err(anyhow::anyhow!(error))?
                    }
                }
            }
        } else {
            (vec![], YamlValidationStatus::Unknown, None)
        };

        let features = crate::extractor::extract_features(parsed, document)?;

        games.push(YamlValidationResult {
            game_name,
            document,
            parsed,
            features,
            validation_status,
            apworlds,
            error,
        });

        own_games_nb += 1;
    }

    Ok(games)
}

fn validate_player_name<'a>(
    original_player_name: &'a String,
    players_in_room: &HashSet<String>,
    player_counter: &'a mut Counter<String>,
) -> Result<String> {
    // AP 0.5.0 doesn't like non ASCII names while hosting.
    if !original_player_name.is_ascii() {
        return Err(Error(anyhow::anyhow!(format!(
            "Your YAML contains an invalid name: {}.",
            original_player_name
        ))));
    }

    static RE_NAME_CURLY_BRACES: Lazy<Regex> = Lazy::new(|| Regex::new(r"\{(.*?)\}").unwrap());
    let allowed_in_curly_brances: HashSet<&str> =
        HashSet::from(["{PLAYER}", "{player}", "{NUMBER}", "{number}"]);
    // AP 0.5.1 doesn't like having `{.*}` if the inner value isn't NUMBER/number/PLAYER/player
    let curly_matches = RE_NAME_CURLY_BRACES.find_iter(original_player_name);
    for m in curly_matches {
        if !allowed_in_curly_brances.contains(m.as_str()) {
            return Err(Error(anyhow::anyhow!(format!("Your YAML contains an invalid name: {}. Archipelago doesn't allow having anything in curly braces within a name other than player/PLAYER/number/NUMBER. Found {}", original_player_name, m.as_str()))));
        }
    }

    let player_name = get_ap_player_name(original_player_name, player_counter);

    if is_reserved_name(&player_name) {
        return Err(Error(anyhow::anyhow!(format!(
            "{} is a reserved name",
            player_name
        ))));
    }

    if players_in_room.contains(&player_name) {
        return Err(Error(anyhow::anyhow!(format!(
            "Adding this yaml would duplicate a player name: {}",
            player_name
        ))));
    }

    Ok(player_name)
}

fn validate_game(game: &YamlGame) -> Result<String> {
    match game {
        YamlGame::Name(name) => Ok(name.clone()),
        YamlGame::Map(map) => {
            let weighted_map: HashMap<&String, &f64> =
                map.iter().filter(|(_, &weight)| weight >= 1.0).collect();

            match weighted_map.len() {
                1 => Ok(weighted_map.keys().next().unwrap().to_string()),
                n if n > 1 => Ok(format!("Random ({})", n)),
                _ => Err(anyhow::anyhow!(
                    "Your YAML contains games but none of them has any chance of getting rolled"
                ))?,
            }
        }
    }
}

pub type ApworldsErrorsUnsupported = (Vec<(String, Version)>, Vec<String>, Vec<String>);

pub enum YamlValidationJobResult {
    Success(Vec<(String, Version)>),
    Failure(Vec<(String, Version)>, String),
    Unsupported(Vec<String>),
}

#[tracing::instrument(skip_all)]
async fn validate_yaml(
    yaml: &str,
    parsed: &YamlFile,
    manifest: &Manifest,
    index_manager: &IndexManager,
    yaml_validation_queue: &YamlValidationQueue,
) -> Result<YamlValidationJobResult> {
    let apworlds = match get_apworlds_for_games(index_manager, manifest, &parsed.game).await {
        Ok(apworlds) => apworlds,
        Err(unsupported) => return Ok(YamlValidationJobResult::Unsupported(unsupported)),
    };

    let mut params = YamlValidationParams {
        apworlds,
        yaml: yaml.to_string(),
        otlp_context: HashMap::new(),
        yaml_id: None,
    };

    let cx = tracing::Span::current().context();
    opentelemetry::global::get_text_map_propagator(|propagator| {
        propagator.inject_context(&cx, &mut params.otlp_context)
    });

    let job_id = yaml_validation_queue
        .enqueue_job(&params, wq::Priority::Normal, Duration::from_secs(30))
        .await?;

    let Some(status) = yaml_validation_queue
        .wait_for_job(&job_id, Some(Duration::from_secs(30)))
        .await?
    else {
        // TODO: alert, this is not normal
        yaml_validation_queue.cancel_job(job_id).await?;
        Err(anyhow!("Timed out while validating this YAML. Either generation is very slow or the service is overloaded. Try again a bit later."))?
    };

    if matches!(status, JobStatus::InternalError) {
        yaml_validation_queue.cancel_job(job_id).await?;
        // TODO: Alert, this is not normal either
        Err(anyhow!("Internal error while validating this YAML. This should not happen, please report the bug."))?
    }

    if matches!(status, JobStatus::Failure) {
        let result = yaml_validation_queue.get_job_result(job_id).await?;
        yaml_validation_queue.delete_job_result(job_id).await?;
        return Ok(YamlValidationJobResult::Failure(
            params.apworlds,
            result.error.unwrap_or_else(|| "Internal Error".to_string()),
        ));
    }

    yaml_validation_queue.delete_job_result(job_id).await?;
    assert_eq!(status, JobStatus::Success);

    // TODO: Maybe show warnings from validation?

    Ok(YamlValidationJobResult::Success(params.apworlds))
}

fn get_ap_player_name<'a>(
    original_name: &'a String,
    player_counter: &'a mut Counter<String>,
) -> String {
    player_counter[original_name] += 1;

    let number = player_counter[original_name];
    let player = player_counter.total::<usize>();

    #[allow(clippy::literal_string_with_formatting_args)]
    let new_name = original_name
        .replace("{number}", &format!("{}", number))
        .replace(
            "{NUMBER}",
            &(if number > 1 {
                format!("{}", number)
            } else {
                "".to_string()
            }),
        )
        .replace("{player}", &format!("{}", player))
        .replace(
            "{PLAYER}",
            &(if player > 1 {
                format!("{}", player)
            } else {
                "".to_string()
            }),
        );

    new_name.trim_start()[..std::cmp::min(new_name.len(), 16)]
        .trim_end()
        .to_string()
}

fn is_reserved_name(player_name: &str) -> bool {
    player_name.to_lowercase() == "meta" || player_name.to_lowercase() == "archipelago"
}

pub async fn get_apworlds_for_games(
    index_manager: &IndexManager,
    manifest: &Manifest,
    games: &YamlGame,
) -> std::result::Result<Vec<(String, Version)>, Vec<String>> {
    match games {
        YamlGame::Name(name) => {
            let Some(apworld_path) = index_manager
                .get_apworld_from_game_name(manifest, name)
                .await
            else {
                return Err(vec![name.to_string()]);
            };
            Ok(vec![apworld_path])
        }
        YamlGame::Map(map) => {
            let mut result = Vec::new();
            let mut errors = Vec::new();
            for (game, _) in map.iter().filter(|(_, probability)| **probability != 0.) {
                let resolved_game = index_manager
                    .get_apworld_from_game_name(manifest, game)
                    .await;

                match resolved_game {
                    Some(game) => result.push(game),
                    None => errors.push(game.clone()),
                }
            }

            if !errors.is_empty() {
                return Err(errors);
            }
            Ok(result)
        }
    }
}

pub async fn revalidate_yamls_if_necessary(
    room: &Room,
    index_manager: &State<IndexManager>,
    yaml_validation_queue: &State<YamlValidationQueue>,
    conn: &mut AsyncPgConnection,
) -> Result<()> {
    if !room.settings.yaml_validation {
        return Ok(());
    }

    let yamls = db::get_yamls_for_room(room.id, conn).await?;

    let (resolved_index, _) = {
        let index = index_manager.index.read().await.clone();
        room.settings.manifest.resolve_with(&index)
    };

    for yaml in &yamls {
        if should_revalidate_yaml(yaml, &resolved_index) {
            queue_yaml_validation(yaml, room, index_manager, yaml_validation_queue, conn).await?;
        }
    }

    Ok(())
}

fn should_revalidate_yaml(
    yaml: &Yaml,
    resolved_index: &BTreeMap<String, (World, Version)>,
) -> bool {
    for (apworld_name, apworld_version) in &yaml.apworlds {
        if let Some((_, new_version)) = resolved_index.get(apworld_name) {
            if new_version != apworld_version {
                return true;
            }
        }
    }

    false
}

pub async fn queue_yaml_validation(
    yaml: &Yaml,
    room: &Room,
    index_manager: &State<IndexManager>,
    yaml_validation_queue: &State<YamlValidationQueue>,
    conn: &mut AsyncPgConnection,
) -> Result<()> {
    let Ok(parsed) = serde_yaml::from_str::<YamlFile>(&yaml.content) else {
        Err(anyhow!(
            "Internal error, unable to reparse a YAML that was already parsed before"
        ))?
    };

    let apworlds =
        match get_apworlds_for_games(index_manager, &room.settings.manifest, &parsed.game).await {
            Ok(apworlds) => apworlds,
            Err(unsupported) => {
                let error = format!("Unsupported apworlds: {}", unsupported.join(", "));

                db::update_yaml_status(
                    yaml.id,
                    db::YamlValidationStatus::Unsupported,
                    Some(error),
                    vec![],
                    Utc::now(),
                    conn,
                )
                .await?;
                return Ok(());
            }
        };

    let mut params = YamlValidationParams {
        apworlds,
        yaml: yaml.content.clone(),
        otlp_context: HashMap::new(),
        yaml_id: Some(yaml.id),
    };

    let cx = tracing::Span::current().context();
    opentelemetry::global::get_text_map_propagator(|propagator| {
        propagator.inject_context(&cx, &mut params.otlp_context)
    });

    yaml_validation_queue
        .enqueue_job(&params, wq::Priority::Low, Duration::from_secs(600))
        .await?;

    Ok(())
}
