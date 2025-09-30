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
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection};
use itertools::Itertools;
use once_cell::sync::Lazy;
use regex::Regex;

use rocket::State;
use semver::Version;
use serde_yaml::Value;
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
                .trim_start_matches('\u{feff}')
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

            let raw_parsed_yaml = match serde_yaml::from_str::<Value>(&doc) {
                Ok(doc) => doc,
                Err(e) => {
                    let error_str = e.to_string();
                    if error_str.contains("duplicate entry") && (error_str.contains("name") || error_str.contains("game")) {
                        anyhow::bail!("Your YAML contains duplicate keys. This usually means you forgot to add '---' to separate multiple YAML documents.");
                    }
                    anyhow::bail!("Your YAML syntax is invalid: {}", e)
                },
            };

            let Ok(parsed) = serde_yaml::from_value(raw_parsed_yaml) else {
                anyhow::bail!(
                    "This does not look like an archipelago YAML. Check that it has both a `name` and a `game` field."
                )
            };
            Ok((doc.trim().trim_start_matches("---").trim().to_string(), parsed))
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
    pub unsupported_games: Vec<String>,
    pub disabled_games: Vec<String>,
}

#[tracing::instrument(skip_all)]
pub async fn parse_and_validate_yamls_for_room<'a>(
    room: &Room,
    documents: &'a [(String, YamlFile)],
    session: &mut LoggedInSession,
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
                || room.settings.author_id == session.user_id()
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

        let (apworlds, validation_status, error, unsupported_games) =
            if room.settings.yaml_validation {
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
                        (apworlds, YamlValidationStatus::Validated, None, vec![])
                    }
                    YamlValidationJobResult::Failure(apworlds, error) => {
                        if room.settings.allow_invalid_yamls {
                            (apworlds, YamlValidationStatus::Failed, Some(error), vec![])
                        } else {
                            Err(anyhow::anyhow!(error))?
                        }
                    }
                    YamlValidationJobResult::Unsupported(unsupported_games) => (
                        vec![],
                        YamlValidationStatus::Unsupported,
                        None,
                        unsupported_games,
                    ),
                }
            } else {
                (vec![], YamlValidationStatus::Unknown, None, vec![])
            };

        let index = index_manager.index.read().await;
        let features = crate::extractor::extract_features(&index, parsed, document)?;
        let (disabled_games, unsupported_games): (Vec<_>, Vec<_>) = unsupported_games
            .into_iter()
            .partition(|game| index.get_world_by_name(game).is_some());

        games.push(YamlValidationResult {
            game_name,
            document,
            parsed,
            features,
            validation_status,
            apworlds,
            error,
            unsupported_games,
            disabled_games,
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

    if original_player_name.contains('{') || original_player_name.contains('}') {
        static RE_NAME_CURLY_BRACES: Lazy<Regex> = Lazy::new(|| Regex::new(r"\{(.*?)\}").unwrap());
        let allowed_in_curly_braces: HashSet<&str> =
            HashSet::from(["{PLAYER}", "{player}", "{NUMBER}", "{number}"]);

        let curly_matches: Vec<_> = RE_NAME_CURLY_BRACES
            .find_iter(original_player_name)
            .collect();

        let mut temp_name = original_player_name.clone();
        for m in &curly_matches {
            if !allowed_in_curly_braces.contains(m.as_str()) {
                return Err(Error(anyhow::anyhow!(format!("Your YAML contains an invalid name: {}. Archipelago doesn't allow having anything in curly braces within a name other than player/PLAYER/number/NUMBER. Found {}", original_player_name, m.as_str()))));
            }
            temp_name = temp_name.replace(m.as_str(), "");
        }

        // If there are still { or } characters left, they're unmatched
        if temp_name.contains('{') || temp_name.contains('}') {
            return Err(Error(anyhow::anyhow!(format!(
                "Your YAML contains an invalid name: {}. Names cannot contain unmatched curly braces.",
                original_player_name
            ))));
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
                n if n > 1 => Ok(format!("Random ({n})")),
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

pub fn get_ap_player_name<'a>(
    original_name: &'a str,
    player_counter: &'a mut Counter<String>,
) -> String {
    let lowercase_name = original_name.to_lowercase();
    player_counter[&lowercase_name] += 1;

    let number = player_counter[&lowercase_name];
    let player = player_counter.total::<usize>();

    #[allow(clippy::literal_string_with_formatting_args)]
    let new_name = original_name
        .replace("{number}", &format!("{number}"))
        .replace(
            "{NUMBER}",
            &(if number > 1 {
                format!("{number}")
            } else {
                "".to_string()
            }),
        )
        .replace("{player}", &format!("{player}"))
        .replace(
            "{PLAYER}",
            &(if player > 1 {
                format!("{player}")
            } else {
                "".to_string()
            }),
        );

    let new_name = new_name.trim_start();

    new_name[..std::cmp::min(new_name.len(), 16)]
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
        let index = index_manager.index.read().await;
        room.settings.manifest.resolve_with(&index)
    };

    conn.transaction::<(), Error, _>(|conn| {
        async move {
            for yaml in &yamls {
                if should_revalidate_yaml(yaml, &resolved_index) {
                    queue_yaml_validation(yaml, room, index_manager, yaml_validation_queue, conn)
                        .await?;
                }
            }

            Ok(())
        }
        .scope_boxed()
    })
    .await?;

    Ok(())
}

fn should_revalidate_yaml(
    yaml: &Yaml,
    resolved_index: &BTreeMap<String, (World, Version)>,
) -> bool {
    // That YAML either never got validated or it was unsupported at the time.
    if yaml.apworlds.is_empty() {
        return true;
    }

    for (apworld_name, apworld_version) in &yaml.apworlds {
        let new_apworld = resolved_index.get(apworld_name);

        match new_apworld {
            // An apworld that was used to validate the YAML is gone. Revalidate the YAML
            // to get it marked as unsupported.
            None => return true,

            // An apworld used for validation is still present in the manifest, if the resolved
            // version isn't the same as the one it was originally validated with then a
            // revalidation is necessaey
            Some((_, new_version)) => {
                if new_version != apworld_version {
                    return true;
                }
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

    db::reset_yaml_validation_status(yaml.id, conn).await?;

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

#[cfg(test)]
mod tests {
    use counter::Counter;

    use crate::yaml::get_ap_player_name;

    #[test]
    fn test_simple_ap_name() {
        let mut counter = Counter::new();
        assert_eq!(get_ap_player_name("foo", &mut counter), "foo");
        assert_eq!(get_ap_player_name("foo", &mut counter), "foo");
        assert_eq!(counter.get("foo"), Some(&2));
        assert_eq!(get_ap_player_name("bar", &mut counter), "bar");
        assert_eq!(counter.get("bar"), Some(&1));
    }

    #[test]
    #[allow(non_snake_case)]
    fn test_ap_name_with_NUMBER() {
        let mut counter = Counter::new();
        assert_eq!(get_ap_player_name("foo{NUMBER}", &mut counter), "foo");
        assert_eq!(get_ap_player_name("foo{NUMBER}", &mut counter), "foo2");
        assert_eq!(get_ap_player_name("foo{NUMBER}", &mut counter), "foo3");
        assert_eq!(get_ap_player_name("foo{NUMBER}", &mut counter), "foo4");
        assert_eq!(get_ap_player_name("bar{NUMBER}", &mut counter), "bar");
        assert_eq!(get_ap_player_name("bar{NUMBER}", &mut counter), "bar2");
        assert_eq!(get_ap_player_name("baz", &mut counter), "baz");
    }

    #[test]
    fn test_ap_name_with_number() {
        let mut counter = Counter::new();
        assert_eq!(get_ap_player_name("foo{number}", &mut counter), "foo1");
        assert_eq!(get_ap_player_name("foo{number}", &mut counter), "foo2");
        assert_eq!(get_ap_player_name("foo{number}", &mut counter), "foo3");
        assert_eq!(get_ap_player_name("foo{number}", &mut counter), "foo4");
        assert_eq!(get_ap_player_name("bar{number}", &mut counter), "bar1");
        assert_eq!(get_ap_player_name("bar{number}", &mut counter), "bar2");
        assert_eq!(get_ap_player_name("baz", &mut counter), "baz");
    }

    #[test]
    #[allow(non_snake_case)]
    fn test_ap_name_with_PLAYER() {
        let mut counter = Counter::new();
        assert_eq!(get_ap_player_name("foo{PLAYER}", &mut counter), "foo");
        assert_eq!(get_ap_player_name("foo{PLAYER}", &mut counter), "foo2");
        assert_eq!(get_ap_player_name("foo{PLAYER}", &mut counter), "foo3");
        assert_eq!(get_ap_player_name("foo{PLAYER}", &mut counter), "foo4");
        assert_eq!(get_ap_player_name("bar{PLAYER}", &mut counter), "bar5");
        assert_eq!(get_ap_player_name("bar{PLAYER}", &mut counter), "bar6");
        assert_eq!(get_ap_player_name("baz", &mut counter), "baz");
    }

    #[test]
    fn test_ap_name_with_player() {
        let mut counter = Counter::new();
        assert_eq!(get_ap_player_name("foo{player}", &mut counter), "foo1");
        assert_eq!(get_ap_player_name("foo{player}", &mut counter), "foo2");
        assert_eq!(get_ap_player_name("foo{player}", &mut counter), "foo3");
        assert_eq!(get_ap_player_name("foo{player}", &mut counter), "foo4");
        assert_eq!(get_ap_player_name("bar{player}", &mut counter), "bar5");
        assert_eq!(get_ap_player_name("bar{player}", &mut counter), "bar6");
        assert_eq!(get_ap_player_name("baz", &mut counter), "baz");
    }

    #[test]
    fn test_ap_name_with_mix_number_player() {
        let mut counter = Counter::new();
        assert_eq!(get_ap_player_name("foo{number}", &mut counter), "foo1");
        assert_eq!(get_ap_player_name("foo{player}", &mut counter), "foo2");
        assert_eq!(get_ap_player_name("foo{number}", &mut counter), "foo2");
        assert_eq!(get_ap_player_name("foo{player}", &mut counter), "foo4");
    }

    #[test]
    fn test_ap_name_case_insensitive() {
        let mut counter = Counter::new();
        assert_eq!(get_ap_player_name("fOO", &mut counter), "fOO");
        assert_eq!(get_ap_player_name("FOO{number}", &mut counter), "FOO1");
        assert_eq!(get_ap_player_name("foo{number}", &mut counter), "foo2");
        assert_eq!(get_ap_player_name("foo{NUMBER}", &mut counter), "foo3");
        assert_eq!(get_ap_player_name("FOO{NUMBER}", &mut counter), "FOO4");
        assert_eq!(get_ap_player_name("foo{player}", &mut counter), "foo6");
        assert_eq!(get_ap_player_name("FoO{NUMBER}", &mut counter), "FoO5");
        assert_eq!(get_ap_player_name("foo{player}", &mut counter), "foo8");
        assert_eq!(get_ap_player_name("foo{NUMBER}", &mut counter), "foo6");
    }

    #[test]
    fn test_ap_name_trim_length() {
        let mut counter = Counter::new();
        assert_eq!(
            get_ap_player_name(&format!("{ :<13}", "abc"), &mut counter),
            "abc"
        );
        assert_eq!(
            get_ap_player_name(&format!("{ :>13}", "abc"), &mut counter),
            "abc"
        );

        // Yes that is AP's behavior
        assert_eq!(
            get_ap_player_name(&format!("{ :>32}", "abc{NUMBER}"), &mut counter),
            "abc"
        );
        assert_eq!(
            get_ap_player_name(&format!("{ :<32}", "abc{NUMBER}"), &mut counter),
            "abc"
        );
        assert_eq!(
            get_ap_player_name(&format!("{ :<33}", "abc{number}"), &mut counter),
            "abc1"
        );
        assert_eq!(
            get_ap_player_name(&format!("{ :>33}", "abc{number}"), &mut counter),
            "abc1"
        );
        assert_eq!(
            get_ap_player_name(&format!("{ :>32}", "abc{NUMBER}"), &mut counter),
            "abc2"
        );
        assert_eq!(
            get_ap_player_name(&format!("{ :<32}", "abc{NUMBER}"), &mut counter),
            "abc2"
        );
        assert_eq!(
            get_ap_player_name(&format!("{ :<33}", "abc{number}"), &mut counter),
            "abc2"
        );
        assert_eq!(
            get_ap_player_name(&format!("{ :>33}", "abc{number}"), &mut counter),
            "abc2"
        );
    }

    #[test]
    fn test_ap_name_overflow_with_number() {
        let mut counter = Counter::new();
        assert_eq!(
            get_ap_player_name(&format!("{ :<16}{{NUMBER}}", "abc"), &mut counter),
            "abc"
        );
        assert_eq!(
            get_ap_player_name(&format!("{ :<16}{{NUMBER}}", "abc"), &mut counter),
            "abc"
        );
        assert_eq!(
            get_ap_player_name(&format!("{ :>16}{{NUMBER}}", "abc"), &mut counter),
            "abc"
        );
        assert_eq!(
            get_ap_player_name(&format!("{ :>16}{{NUMBER}}", "abc"), &mut counter),
            "abc2"
        );
        assert_eq!(
            get_ap_player_name(&format!("{:f>16}{{NUMBER}}", "abc"), &mut counter),
            "fffffffffffffabc"
        );
        assert_eq!(
            get_ap_player_name(&format!("{:f>16}{{NUMBER}}", "abc"), &mut counter),
            "fffffffffffffabc"
        );
        assert_eq!(
            get_ap_player_name(&format!("{:f>17}{{NUMBER}}", "abc"), &mut counter),
            "ffffffffffffffab"
        );
        assert_eq!(
            get_ap_player_name(&format!("{:f>17}{{NUMBER}}", "abc"), &mut counter),
            "ffffffffffffffab"
        );
        assert_eq!(
            get_ap_player_name(&format!("{:f>15}{{NUMBER}}", "abc"), &mut counter),
            "ffffffffffffabc"
        );
        assert_eq!(
            get_ap_player_name(&format!("{:f>15}{{NUMBER}}", "abc"), &mut counter),
            "ffffffffffffabc2"
        );
    }

    #[test]
    fn test_validate_player_name_valid() {
        use crate::yaml::validate_player_name;
        use counter::Counter;
        use std::collections::HashSet;

        let players_in_room = HashSet::new();
        let mut player_counter = Counter::new();

        assert!(
            validate_player_name(&"player".to_string(), &players_in_room, &mut player_counter)
                .is_ok()
        );
        assert!(validate_player_name(
            &"player{NUMBER}".to_string(),
            &players_in_room,
            &mut player_counter
        )
        .is_ok());
        assert!(validate_player_name(
            &"player{number}".to_string(),
            &players_in_room,
            &mut player_counter
        )
        .is_ok());
        assert!(validate_player_name(
            &"player{PLAYER}".to_string(),
            &players_in_room,
            &mut player_counter
        )
        .is_ok());
        assert!(validate_player_name(
            &"player{player}".to_string(),
            &players_in_room,
            &mut player_counter
        )
        .is_ok());
    }

    #[test]
    fn test_validate_player_name_unmatched_braces() {
        use crate::yaml::validate_player_name;
        use counter::Counter;
        use std::collections::HashSet;

        let players_in_room = HashSet::new();
        let mut player_counter = Counter::new();

        assert!(validate_player_name(
            &"player{NUMBER".to_string(),
            &players_in_room,
            &mut player_counter
        )
        .is_err());
        assert!(validate_player_name(
            &"playerNUMBER}".to_string(),
            &players_in_room,
            &mut player_counter
        )
        .is_err());
        assert!(validate_player_name(
            &"player{".to_string(),
            &players_in_room,
            &mut player_counter
        )
        .is_err());
        assert!(validate_player_name(
            &"player}".to_string(),
            &players_in_room,
            &mut player_counter
        )
        .is_err());
        assert!(validate_player_name(
            &"player{NUMBER)".to_string(),
            &players_in_room,
            &mut player_counter
        )
        .is_err());
        assert!(validate_player_name(
            &"player(NUMBER}".to_string(),
            &players_in_room,
            &mut player_counter
        )
        .is_err());
    }

    #[test]
    fn test_validate_player_name_invalid_braces_content() {
        use crate::yaml::validate_player_name;
        use counter::Counter;
        use std::collections::HashSet;

        let players_in_room = HashSet::new();
        let mut player_counter = Counter::new();

        assert!(validate_player_name(
            &"player{INVALID}".to_string(),
            &players_in_room,
            &mut player_counter
        )
        .is_err());
        assert!(validate_player_name(
            &"player{123}".to_string(),
            &players_in_room,
            &mut player_counter
        )
        .is_err());
        assert!(validate_player_name(
            &"player{Player}".to_string(),
            &players_in_room,
            &mut player_counter
        )
        .is_err());
        assert!(validate_player_name(
            &"player{Number}".to_string(),
            &players_in_room,
            &mut player_counter
        )
        .is_err());
    }

    #[test]
    fn test_parse_raw_yamls_valid() {
        use crate::yaml::parse_raw_yamls;

        let valid_single = r#"
name: Player1
game: A Link to the Past
"#;
        assert!(parse_raw_yamls(&[valid_single]).is_ok());

        let valid_multiple = r#"
name: Player1
game: A Link to the Past
---
name: Player2
game: Super Metroid
"#;
        assert!(parse_raw_yamls(&[valid_multiple]).is_ok());
    }

    #[test]
    fn test_parse_raw_yamls_duplicate_name_keys() {
        use crate::yaml::parse_raw_yamls;

        let duplicate_name = r#"
name: Player1
game: A Link to the Past
name: Player2
game: Super Metroid
"#;
        let result = parse_raw_yamls(&[duplicate_name]);
        assert!(result.is_err());
        let error_msg = result.unwrap_err().0.to_string();
        assert!(error_msg.contains("forgot to add '---'"));
    }

    #[test]
    fn test_parse_raw_yamls_duplicate_game_keys() {
        use crate::yaml::parse_raw_yamls;

        let duplicate_game = r#"
name: Player1
game: A Link to the Past
game: Super Metroid
"#;
        let result = parse_raw_yamls(&[duplicate_game]);
        assert!(result.is_err());
        let error_msg = result.unwrap_err().0.to_string();
        assert!(error_msg.contains("forgot to add '---'"));
    }

    #[test]
    fn test_parse_raw_yamls_both_duplicate_keys() {
        use crate::yaml::parse_raw_yamls;

        let both_duplicate = r#"
name: Player1
game: A Link to the Past
name: Player2
game: Super Metroid
"#;
        let result = parse_raw_yamls(&[both_duplicate]);
        assert!(result.is_err());
        let error_msg = result.unwrap_err().0.to_string();
        assert!(error_msg.contains("forgot to add '---'"));
    }
}
