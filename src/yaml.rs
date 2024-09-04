use crate::db::{self, Room, YamlFile, YamlGame};
use crate::error::{Error, Result, WithContext};
use crate::extractor::YamlFeatures;
use crate::session::LoggedInSession;

use crate::index_manager::IndexManager;
use diesel_async::AsyncPgConnection;
use itertools::Itertools;
use opentelemetry_http::HeaderInjector;
use reqwest::Url;
use rocket::http::CookieJar;
use semver::Version;
use std::collections::{HashMap, HashSet};
use std::io::BufReader;
use tracing_opentelemetry::OpenTelemetrySpanExt;

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
    yaml_validator_url: &Option<Url>,
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

    let mut players_in_room = yamls_in_room
        .iter()
        .map(|yaml| get_ap_player_name(&yaml.player_name))
        .collect::<HashSet<&str>>();

    let mut games = Vec::with_capacity(documents.len());

    for (document, parsed) in documents.iter() {
        if let Some(yaml_limit_per_user) = room.yaml_limit_per_user {
            let allow_bypass =
                session.0.is_admin || room.yaml_limit_bypass_list.contains(&session.user_id());
            if own_games_nb >= yaml_limit_per_user && !allow_bypass {
                return Err(anyhow::anyhow!(format!(
                    "The room only allows {} game(s) per person. Cannot upload.",
                    yaml_limit_per_user
                ))
                .into());
            }
        }
        let player_name = validate_player_name(&parsed.name, &players_in_room)?;
        players_in_room.insert(player_name);

        let game_name = validate_game(&parsed.game)?;

        if room.yaml_validation {
            if let Some(yaml_validator_url) = yaml_validator_url {
                let unsupported_games =
                    validate_yaml(document, parsed, index_manager, yaml_validator_url).await?;
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
        }

        let features = crate::extractor::extract_features(parsed, document)?;

        games.push((game_name, document, parsed, features));
        own_games_nb += 1;
    }

    Ok(games)
}

fn validate_player_name<'a>(
    original_player_name: &'a str,
    players_in_room: &HashSet<&str>,
) -> Result<&'a str> {
    // AP 0.5.0 doesn't like non ASCII names while hosting.
    if !original_player_name.is_ascii() {
        return Err(Error(anyhow::anyhow!(format!(
            "Your YAML contains an invalid name: {}.",
            original_player_name
        ))));
    }

    let player_name = get_ap_player_name(original_player_name);

    if is_reserved_name(player_name) {
        return Err(Error(anyhow::anyhow!(format!(
            "{} is a reserved name",
            player_name
        ))));
    }

    let ignore_dupe = should_ignore_dupes(original_player_name);
    if !ignore_dupe && players_in_room.contains(&player_name) {
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
                    "Your YAML contains games but none of the has any chance of getting rolled"
                ))?,
            }
        }
    }
}

#[tracing::instrument(skip_all)]
async fn validate_yaml(
    yaml: &str,
    parsed: &YamlFile,
    index_manager: &IndexManager,
    yaml_validator_url: &Url,
) -> Result<Vec<String>> {
    #[derive(serde::Deserialize)]
    struct ValidationResponse {
        error: Option<String>,
        unsupported: Option<Vec<String>>,
    }

    let client = reqwest::Client::new();
    let apworlds = match get_apworlds_for_games(index_manager, &parsed.game).await {
        Ok(apworlds) => apworlds,
        Err(unsupported) => return Ok(unsupported),
    };

    let form = reqwest::multipart::Form::new()
        .text("data", yaml.to_string())
        .text("apworlds", serde_json::to_string(&apworlds)?);

    let cx = tracing::Span::current().context();

    let mut req = client
        .post(yaml_validator_url.join("/check_yaml")?)
        .multipart(form)
        .build()?;

    opentelemetry::global::get_text_map_propagator(|propagator| {
        propagator.inject_context(&cx, &mut HeaderInjector(req.headers_mut()))
    });

    tracing::event!(tracing::Level::INFO, "Request to yaml-checker started");
    let response = client
        .execute(req)
        .await
        .map_err(|_| anyhow::anyhow!("Error while communicating with the YAML validator."))?
        .json::<ValidationResponse>()
        .await?;

    if let Some(error) = response.error {
        return Err(anyhow::anyhow!(error).into());
    }

    Ok(response.unsupported.unwrap_or(vec![]))
}

fn should_ignore_dupes(player_name: &str) -> bool {
    player_name.contains("{NUMBER}")
        || player_name.contains("{number}")
        || player_name.contains("{PLAYER}")
        || player_name.contains("{player}")
}

fn get_ap_player_name(original_name: &str) -> &str {
    original_name.trim_start()[..std::cmp::min(original_name.len(), 16)].trim_end()
}

fn is_reserved_name(player_name: &str) -> bool {
    player_name == "meta" || player_name == "Archipelago"
}

async fn get_apworlds_for_games(
    index_manager: &IndexManager,
    games: &YamlGame,
) -> std::result::Result<Vec<(String, Version)>, Vec<String>> {
    match games {
        YamlGame::Name(name) => {
            let Some(apworld_path) = index_manager.get_apworld_from_game_name(name).await else {
                return Err(vec![name.to_string()]);
            };
            Ok(vec![apworld_path])
        }
        YamlGame::Map(map) => {
            let mut result = Vec::new();
            for (game, _) in map.iter().filter(|(_, probability)| **probability != 0.) {
                result.push(
                    index_manager
                        .get_apworld_from_game_name(game)
                        .await
                        .unwrap(),
                )
            }

            Ok(result)
        }
    }
}
