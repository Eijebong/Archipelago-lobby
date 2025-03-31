use std::{collections::HashSet, path::Path};

use crate::{
    db::{YamlId, YamlWithoutContent},
    error::Result,
};
use wq::JobId;

#[derive(Default)]
pub struct GenerationInfo {
    pub log_file: Option<String>,
    pub output_file: Option<String>,
}

pub fn get_generation_info(job_id: JobId, output_dir: &Path) -> Result<GenerationInfo> {
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

pub fn get_slots(room_yamls: &[YamlWithoutContent]) -> Vec<(String, YamlId)> {
    let mut room_yamls_with_resolved_names = Vec::with_capacity(room_yamls.len());

    // This is the same logic as in the room YAML download
    let mut emitted_names = HashSet::new();
    for yaml in room_yamls {
        let player_name = yaml.sanitized_name();
        let mut original_file_name = format!("{}.yaml", player_name);

        let mut suffix = 0u64;
        if emitted_names.contains(&original_file_name.to_lowercase()) {
            loop {
                let new_file_name = format!("{}_{}.yaml", player_name, suffix);
                if !emitted_names.contains(&new_file_name.to_lowercase()) {
                    original_file_name = new_file_name;
                    break;
                }
                suffix += 1;
            }
        }
        emitted_names.insert(original_file_name.to_lowercase());
        room_yamls_with_resolved_names.push((original_file_name, yaml.id))
    }

    room_yamls_with_resolved_names.sort_by_cached_key(|(r, _)| r.clone());

    room_yamls_with_resolved_names
}
