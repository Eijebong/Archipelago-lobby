use crate::db::{self, Room, YamlFile, YamlGame};
use crate::error::{Error, Result, WithContext};
use crate::extractor::YamlFeatures;
use crate::jobs::{YamlValidationParams, YamlValidationQueue};
use crate::session::LoggedInSession;

use crate::index_manager::IndexManager;
use anyhow::anyhow;
use apwm::Manifest;
use counter::Counter;
use diesel_async::AsyncPgConnection;
use itertools::Itertools;
use rocket::http::CookieJar;
use semver::Version;
use std::collections::{HashMap, HashSet};
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

#[tracing::instrument(skip_all)]
pub async fn parse_and_validate_yamls_for_room<'a>(
    room: &Room,
    documents: &'a [(String, YamlFile)],
    session: &mut LoggedInSession,
    cookies: &CookieJar<'_>,
    yaml_validation_queue: &YamlValidationQueue,
    index_manager: &IndexManager,
    conn: &mut AsyncPgConnection,
) -> Result<Vec<(String, &'a String, &'a YamlFile, YamlFeatures)>> {
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

        if room.settings.yaml_validation {
            let unsupported_games = validate_yaml(
                document,
                parsed,
                &room.settings.manifest,
                index_manager,
                yaml_validation_queue,
            )
            .await?;
            if !unsupported_games.is_empty() {
                if room.settings.allow_unsupported {
                    session.0.warning_msg.push(format!(
                        "Uploaded a YAML with unsupported games: {}. Couldn't verify it.",
                        unsupported_games.iter().join("; ")
                    ));
                    session.0.save(cookies)?;
                } else {
                    let index = index_manager.index.read().await;
                    let err = format!("Error: {}",
                        unsupported_games.iter().map(|game| {
                            let is_game_in_index = index.get_world_by_name(game).is_some();
                            if is_game_in_index {
                                format!("Uploaded a game for game {} which has been disabled for this room", game)
                            } else {
                                format!("Uploaded a game for game {} which is not supported on this lobby", game)
                            }
                        }).join("\n")
                    );

                    Err(anyhow::anyhow!(err))?
                }
            }
        }

        let features = crate::extractor::extract_features(parsed, document)?;

        games.push((game_name, document, parsed, features));
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

#[tracing::instrument(skip_all)]
async fn validate_yaml(
    yaml: &str,
    parsed: &YamlFile,
    manifest: &Manifest,
    index_manager: &IndexManager,
    yaml_validation_queue: &YamlValidationQueue,
) -> Result<Vec<String>> {
    let apworlds = match get_apworlds_for_games(index_manager, manifest, &parsed.game).await {
        Ok(apworlds) => apworlds,
        Err(unsupported) => return Ok(unsupported),
    };

    let mut params = YamlValidationParams {
        apworlds,
        yaml: yaml.to_string(),
        otlp_context: HashMap::new(),
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
        Err(anyhow!(
            "Error: {}",
            result.error.unwrap_or_else(|| "Internal error".to_string())
        ))?
    }

    assert_eq!(status, JobStatus::Success);

    // TODO: Maybe show warnings from validation?

    return Ok(vec![]);
}

fn get_ap_player_name<'a>(
    original_name: &'a String,
    player_counter: &'a mut Counter<String>,
) -> String {
    player_counter[original_name] += 1;

    let number = player_counter[original_name];
    let player = player_counter.total::<usize>();

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
    player_name == "meta" || player_name == "Archipelago"
}

async fn get_apworlds_for_games(
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
