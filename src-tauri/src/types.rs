use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::atomic::AtomicBool;

/// One explicit user action. Settings retain the original Launcher's key names.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct JobRequest {
    pub action: String,
    pub map_name: String,
    pub mod_name: String,
    pub settings: BTreeMap<String, String>,
    pub files: Vec<String>,
    pub tool_id: String,
    pub companions: bool,
}

#[derive(Default)]
pub struct JobState {
    pub busy: AtomicBool,
    pub cancel: AtomicBool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tool {
    pub id: String,
    pub label: String,
    pub stock_exe: String,
    pub companion_exe: String,
    pub available: bool,
    pub stock_available: bool,
    pub companion_available: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Context {
    pub valid_root: bool,
    pub root: String,
    pub message: String,
    pub development: bool,
    pub version: String,
    pub maps: Vec<String>,
    pub mods: Vec<String>,
    pub templates: Vec<String>,
    pub zone_sources: Vec<String>,
    pub tools: Vec<Tool>,
}
