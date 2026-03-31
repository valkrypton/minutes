use crate::capture::RecordingIntent;
use crate::config::Config;
use crate::pid::CaptureMode;
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

pub fn control_dir() -> PathBuf {
    Config::minutes_dir().join("desktop-control")
}

pub fn requests_dir() -> PathBuf {
    control_dir().join("requests")
}

pub fn responses_dir() -> PathBuf {
    control_dir().join("responses")
}

pub fn desktop_app_status_path() -> PathBuf {
    control_dir().join("desktop-app.json")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesktopAppStatus {
    pub pid: u32,
    pub updated_at: DateTime<Local>,
    pub platform: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartRecordingRequest {
    pub mode: CaptureMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intent: Option<RecordingIntent>,
    #[serde(default)]
    pub allow_degraded: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum DesktopControlAction {
    StartRecording(StartRecordingRequest),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesktopControlRequest {
    pub id: String,
    pub created_at: DateTime<Local>,
    pub action: DesktopControlAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesktopControlResponse {
    pub id: String,
    pub handled_at: DateTime<Local>,
    pub accepted: bool,
    pub detail: String,
}

fn ensure_dirs() -> std::io::Result<()> {
    fs::create_dir_all(requests_dir())?;
    fs::create_dir_all(responses_dir())?;
    Ok(())
}

pub fn write_desktop_app_status(status: &DesktopAppStatus) -> std::io::Result<()> {
    ensure_dirs()?;
    let path = desktop_app_status_path();
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(status)?)?;
    fs::rename(tmp, path)?;
    Ok(())
}

pub fn clear_desktop_app_status() -> std::io::Result<()> {
    let path = desktop_app_status_path();
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

pub fn read_desktop_app_status() -> Option<DesktopAppStatus> {
    let path = desktop_app_status_path();
    fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
}

pub fn request_path(id: &str) -> PathBuf {
    requests_dir().join(format!("{}.json", id))
}

pub fn response_path(id: &str) -> PathBuf {
    responses_dir().join(format!("{}.json", id))
}

pub fn write_request(request: &DesktopControlRequest) -> std::io::Result<()> {
    ensure_dirs()?;
    let path = request_path(&request.id);
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(request)?)?;
    fs::rename(tmp, path)?;
    Ok(())
}

pub fn write_response(response: &DesktopControlResponse) -> std::io::Result<()> {
    ensure_dirs()?;
    let path = response_path(&response.id);
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(response)?)?;
    fs::rename(tmp, path)?;
    Ok(())
}

pub fn remove_request(id: &str) -> std::io::Result<()> {
    let path = request_path(id);
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

pub fn remove_response(id: &str) -> std::io::Result<()> {
    let path = response_path(id);
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

pub fn pending_requests() -> Vec<DesktopControlRequest> {
    let mut requests = Vec::new();
    let dir = requests_dir();
    if !dir.exists() {
        return requests;
    }

    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        if let Ok(text) = fs::read_to_string(&path) {
            if let Ok(request) = serde_json::from_str::<DesktopControlRequest>(&text) {
                requests.push(request);
            }
        }
    }

    requests.sort_by_key(|request| request.created_at);
    requests
}
