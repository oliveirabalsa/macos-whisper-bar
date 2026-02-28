use std::{fs, path::PathBuf, sync::Arc};

use crate::models;
use serde::{Deserialize, Serialize};
use tauri::async_runtime::JoinHandle;
use tauri::{AppHandle, Emitter};
use tokio::{process::Child, process::ChildStdin, sync::Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AppStatus {
    Idle,
    Installing,
    Ready,
    Recording,
    Error,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSnapshot {
    pub status: AppStatus,
    pub status_message: String,
    pub language: String,
    pub selected_model_id: String,
    pub selected_model_installed: bool,
    pub selected_mic_device: Option<String>,
    pub transcript: String,
    pub last_saved_path: Option<String>,
    pub install_progress: Option<f32>,
    pub error_message: Option<String>,
}

pub struct WorkerProcess {
    pub child: Child,
    pub stdin: Option<ChildStdin>,
    pub stdout_task: Option<JoinHandle<()>>,
    pub stderr_task: Option<JoinHandle<()>>,
}

pub struct StateInner {
    pub status: AppStatus,
    pub status_message: String,
    pub language: String,
    pub selected_model_id: String,
    pub selected_mic_device: Option<String>,
    pub transcript: String,
    pub last_saved_path: Option<String>,
    pub install_progress: Option<f32>,
    pub error_message: Option<String>,
    pub app_data_dir: PathBuf,
    pub scripts_dir: PathBuf,
    pub bootstrap_script: PathBuf,
    pub worker_script: PathBuf,
    pub venv_python: PathBuf,
    pub model_path: PathBuf,
    pub worker: Option<WorkerProcess>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PersistedSettings {
    pub language: Option<String>,
    pub selected_model_id: Option<String>,
    pub selected_mic_device: Option<String>,
}

impl StateInner {
    pub fn new(app_data_dir: PathBuf) -> Self {
        let scripts_dir = app_data_dir.join("python");
        let venv_python = app_data_dir.join("python-env").join("bin").join("python");
        let selected_model_id = models::default_model_id().to_string();
        let model_path = models::model_path(&app_data_dir, &selected_model_id)
            .unwrap_or_else(|| app_data_dir.join("models").join("whisper-large-v3-turbo"));

        let mut state = Self {
            status: AppStatus::Ready,
            status_message: "Ready".to_string(),
            language: "en".to_string(),
            selected_model_id,
            selected_mic_device: None,
            transcript: String::new(),
            last_saved_path: None,
            install_progress: None,
            error_message: None,
            bootstrap_script: scripts_dir.join("bootstrap.py"),
            worker_script: scripts_dir.join("worker.py"),
            app_data_dir,
            scripts_dir,
            venv_python,
            model_path,
            worker: None,
        };

        if let Ok(settings) = load_settings(&state.app_data_dir) {
            if let Some(language) = settings.language {
                if language == "en" || language == "pt-BR" {
                    state.language = language;
                }
            }

            if let Some(model_id) = settings.selected_model_id {
                if models::find_model(&model_id).is_some() {
                    state.selected_model_id = model_id.clone();
                    state.model_path = models::model_path(&state.app_data_dir, &model_id)
                        .unwrap_or_else(|| {
                            state.app_data_dir.join("models").join("whisper-large-v3-turbo")
                        });
                }
            }

            state.selected_mic_device = settings.selected_mic_device;
        }

        if !is_model_installed(&state.model_path) {
            state.status_message = "Model not installed. Select a model and click Install Model.".to_string();
        }

        state
    }

    pub fn snapshot(&self) -> AppSnapshot {
        AppSnapshot {
            status: self.status,
            status_message: self.status_message.clone(),
            language: self.language.clone(),
            selected_model_id: self.selected_model_id.clone(),
            selected_model_installed: is_model_installed(&self.model_path),
            selected_mic_device: self.selected_mic_device.clone(),
            transcript: self.transcript.clone(),
            last_saved_path: self.last_saved_path.clone(),
            install_progress: self.install_progress,
            error_message: self.error_message.clone(),
        }
    }
}

fn is_model_installed(model_path: &std::path::Path) -> bool {
    if !model_path.exists() {
        return false;
    }

    let config_exists = model_path.join("config.json").exists();
    let has_weights = std::fs::read_dir(model_path)
        .ok()
        .into_iter()
        .flat_map(|entries| entries.flatten())
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .any(|name| name.starts_with("weights.") || name.starts_with("model"));

    config_exists && has_weights
}

pub fn save_settings(inner: &StateInner) -> anyhow::Result<()> {
    let settings = PersistedSettings {
        language: Some(inner.language.clone()),
        selected_model_id: Some(inner.selected_model_id.clone()),
        selected_mic_device: inner.selected_mic_device.clone(),
    };
    save_settings_to_dir(&inner.app_data_dir, &settings)
}

fn settings_path(app_data_dir: &std::path::Path) -> PathBuf {
    app_data_dir.join("settings.json")
}

fn load_settings(app_data_dir: &std::path::Path) -> anyhow::Result<PersistedSettings> {
    let path = settings_path(app_data_dir);
    let raw = fs::read_to_string(&path)?;
    let parsed = serde_json::from_str::<PersistedSettings>(&raw)?;
    Ok(parsed)
}

fn save_settings_to_dir(
    app_data_dir: &std::path::Path,
    settings: &PersistedSettings,
) -> anyhow::Result<()> {
    fs::create_dir_all(app_data_dir)?;
    let path = settings_path(app_data_dir);
    let json = serde_json::to_string_pretty(settings)?;
    fs::write(path, json)?;
    Ok(())
}

#[derive(Clone)]
pub struct SharedState(pub Arc<Mutex<StateInner>>);

impl SharedState {
    pub fn new(app_data_dir: PathBuf) -> Self {
        Self(Arc::new(Mutex::new(StateInner::new(app_data_dir))))
    }
}

pub async fn snapshot(state: &SharedState) -> AppSnapshot {
    let guard = state.0.lock().await;
    guard.snapshot()
}

pub async fn emit_state(app: &AppHandle, state: &SharedState) {
    let snapshot = snapshot(state).await;
    let _ = app.emit("whisperbar://state", snapshot);
}

pub async fn update_state<F>(app: &AppHandle, state: &SharedState, updater: F)
where
    F: FnOnce(&mut StateInner),
{
    let snapshot = {
        let mut guard = state.0.lock().await;
        updater(&mut guard);
        guard.snapshot()
    };
    let _ = app.emit("whisperbar://state", snapshot);
}
