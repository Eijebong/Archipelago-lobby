use std::path::Path;

use crate::error::Result;
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
