use std::collections::HashMap;

use semver::Version;
use serde::{Deserialize, Serialize};
use wq::WorkQueue;

#[derive(Serialize, Deserialize)]
pub struct YamlValidationParams {
    pub apworlds: Vec<(String, Version)>,
    pub yaml: String,
    pub otlp_context: HashMap<String, String>,
}
#[derive(Serialize, Deserialize, Clone)]
pub struct YamlValidationResponse {
    pub error: Option<String>,
}

pub type YamlValidationQueue = WorkQueue<YamlValidationParams, YamlValidationResponse>;
