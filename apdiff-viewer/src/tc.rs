use anyhow::Result;
use semver::Version;
use taskcluster::{Index, Queue};

// Tilde-escape characters that are unsafe in TC index path components.
// We can't use percent-encoding because Express URL-decodes path params
// before the TC Index handler sees them (TC bug).
//
// TC allowed chars: [a-zA-Z0-9_!~*'()%-]  (and . as namespace separator)
// We use ~ as escape: ~XX where XX is the lowercase hex of the byte.
// ~ itself is escaped as ~7e.
fn encode_index_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b == b'_' || b == b'-' {
            out.push(b as char);
        } else {
            out.push_str(&format!("~{:02x}", b));
        }
    }
    out
}

fn decode_index_component(s: &str) -> String {
    let mut out = Vec::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'~' && i + 2 < bytes.len() {
            if let Ok(byte) =
                u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16)
            {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn index_path(namespace_prefix: &str, world_name: &str, version: &str) -> String {
    let safe_name = encode_index_component(world_name);
    let safe_version = encode_index_component(version);
    format!("{namespace_prefix}.{safe_name}.{safe_version}")
}

pub async fn get_task_artifacts(queue: &Queue, task_id: &str) -> Result<Vec<String>> {
    let mut continuation_token = None;
    let mut all_artifacts = Vec::new();

    loop {
        let artifacts_page = queue
            .listLatestArtifacts(task_id, continuation_token.as_deref(), None)
            .await?;

        continuation_token = artifacts_page
            .get("continuationToken")
            .and_then(|token| token.as_str().map(String::from));

        if let Some(artifacts) = artifacts_page.get("artifacts").and_then(|v| v.as_array()) {
            let page_artifacts: Vec<String> = artifacts
                .iter()
                .filter_map(|v| v.get("name")?.as_str().map(String::from))
                .collect();
            all_artifacts.extend(page_artifacts);
        }

        if continuation_token.is_none() {
            break;
        }
    }

    Ok(all_artifacts)
}

pub async fn fetch_artifact_bytes(
    queue: &Queue,
    task_id: &str,
    artifact_name: &str,
) -> Result<Vec<u8>> {
    let url = queue.getLatestArtifact_url(task_id, artifact_name)?;
    let bytes = reqwest::get(&url)
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    Ok(bytes.to_vec())
}

pub async fn fetch_artifact_text(
    queue: &Queue,
    task_id: &str,
    artifact_name: &str,
) -> Result<String> {
    let url = queue.getLatestArtifact_url(task_id, artifact_name)?;
    Ok(reqwest::get(&url).await?.error_for_status()?.text().await?)
}

pub async fn find_indexed_task(index: &Index, index_path: &str) -> Result<Option<String>> {
    match index.findTask(index_path).await {
        Ok(value) => Ok(value
            .get("taskId")
            .and_then(|v| v.as_str())
            .map(String::from)),
        Err(e) => {
            if taskcluster::err_status_code(&e) == Some(taskcluster::StatusCode::NOT_FOUND) {
                Ok(None)
            } else {
                Err(e.into())
            }
        }
    }
}

pub async fn list_indexed_versions(
    index: &Index,
    namespace_prefix: &str,
    world_name: &str,
) -> Result<Vec<(Version, String)>> {
    let namespace = format!(
        "{}.{}",
        namespace_prefix,
        encode_index_component(world_name)
    );
    let mut continuation_token = None;
    let mut versions = Vec::new();

    loop {
        let result = index
            .listTasks(&namespace, continuation_token.as_deref(), None)
            .await?;

        let prefix = format!("{namespace}.");
        if let Some(tasks) = result.get("tasks").and_then(|v| v.as_array()) {
            for task in tasks {
                let Some(ns) = task.get("namespace").and_then(|v| v.as_str()) else {
                    continue;
                };
                let Some(task_id) = task.get("taskId").and_then(|v| v.as_str()) else {
                    continue;
                };

                if let Some(encoded_version) = ns.strip_prefix(&prefix) {
                    let version_str = decode_index_component(encoded_version);
                    if let Ok(version) = Version::parse(&version_str) {
                        versions.push((version, task_id.to_string()));
                    }
                }
            }
        }

        continuation_token = result
            .get("continuationToken")
            .and_then(|token| token.as_str().map(String::from));
        if continuation_token.is_none() {
            break;
        }
    }

    versions.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(versions)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_roundtrip() {
        let cases = [
            "hello",
            "5.3.0",
            "1.0.80-exp+hotfix1",
            "Twilight Princess",
            "pokemon_crystal",
            "ape_escape_3",
            "a~b.c+d e",
            "",
            "café",
            "100%",
            "foo!bar",
            "a'b*c(d)e",
        ];
        for input in cases {
            let encoded = encode_index_component(input);
            let decoded = decode_index_component(&encoded);
            assert_eq!(
                decoded, input,
                "roundtrip failed for {input:?}: encoded as {encoded:?}"
            );
        }
    }

    #[test]
    fn test_encoded_is_tc_safe() {
        let inputs = ["5.3.0", "Twilight Princess", "a~b+c.d", "café", "100%"];
        let tc_component_re = regex::Regex::new(r"^[a-zA-Z0-9_!\~*'()%-]+$").unwrap();
        for input in inputs {
            let encoded = encode_index_component(input);
            assert!(
                tc_component_re.is_match(&encoded),
                "{input:?} encoded to {encoded:?} which is not TC-safe"
            );
        }
    }

    #[test]
    fn test_index_path() {
        assert_eq!(
            index_path("ap.index.world", "pokemon_crystal", "5.3.0"),
            "ap.index.world.pokemon_crystal.5~2e3~2e0"
        );
        assert_eq!(
            index_path("ap.index.world", "Twilight Princess", "1.0.0+build"),
            "ap.index.world.Twilight~20Princess.1~2e0~2e0~2bbuild"
        );
    }
}
