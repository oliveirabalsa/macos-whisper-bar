use std::process::Stdio;

use anyhow::{anyhow, Context};
use serde::Deserialize;
use tauri::AppHandle;
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
};

use crate::{
    app_state::{emit_state, save_settings, update_state, AppStatus, SharedState},
    models, runtime_scripts,
};

#[derive(Debug, Deserialize)]
struct BootstrapEvent {
    #[serde(rename = "type")]
    event_type: String,
    message: Option<String>,
    progress: Option<f32>,
    venv_python: Option<String>,
    model_path: Option<String>,
    model_id: Option<String>,
}

pub async fn run_bootstrap(app: &AppHandle, state: &SharedState) -> anyhow::Result<()> {
    let selected_model_id = {
        let guard = state.0.lock().await;
        guard.selected_model_id.clone()
    };

    run_bootstrap_for_model(app, state, &selected_model_id).await
}

pub async fn run_bootstrap_for_model(
    app: &AppHandle,
    state: &SharedState,
    model_id: &str,
) -> anyhow::Result<()> {
    runtime_scripts::ensure_scripts(state).await?;

    let model =
        models::find_model(model_id).ok_or_else(|| anyhow!("unsupported model id: {model_id}"))?;

    update_state(app, state, |inner| {
        inner.status = AppStatus::Installing;
        inner.status_message = format!("Preparing model: {}", model.name);
        inner.install_progress = Some(0.05);
        inner.error_message = None;
        inner.selected_model_id = model.id.to_string();
    })
    .await;

    let (script_path, app_data_dir) = {
        let guard = state.0.lock().await;
        (guard.bootstrap_script.clone(), guard.app_data_dir.clone())
    };

    let mut child = Command::new("python3")
        .arg(script_path)
        .arg("--app-data-dir")
        .arg(&app_data_dir)
        .arg("--model-id")
        .arg(model.id)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to start bootstrap script")?;

    let stdout = child
        .stdout
        .take()
        .context("missing stdout for bootstrap script")?;
    let stderr = child
        .stderr
        .take()
        .context("missing stderr for bootstrap script")?;

    let stderr_task = tauri::async_runtime::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        let mut output = String::new();

        while let Ok(Some(line)) = lines.next_line().await {
            if !output.is_empty() {
                output.push('\n');
            }
            output.push_str(&line);
        }

        output
    });

    let mut lines = BufReader::new(stdout).lines();

    while let Some(line) = lines
        .next_line()
        .await
        .context("error reading bootstrap output")?
    {
        if let Ok(event) = serde_json::from_str::<BootstrapEvent>(&line) {
            match event.event_type.as_str() {
                "progress" => {
                    let message = event
                        .message
                        .unwrap_or_else(|| format!("Preparing model: {}", model.name));
                    update_state(app, state, move |inner| {
                        inner.status = AppStatus::Installing;
                        inner.status_message = message;
                        inner.install_progress = event.progress;
                        inner.error_message = None;
                    })
                    .await;
                }
                "ready" => {
                    let model_id_from_event = event.model_id.clone();
                    update_state(app, state, |inner| {
                        if let Some(path) = event.venv_python {
                            inner.venv_python = path.into();
                        }
                        if let Some(path) = event.model_path {
                            inner.model_path = path.into();
                        }
                        if let Some(id) = model_id_from_event {
                            inner.selected_model_id = id;
                        } else {
                            inner.selected_model_id = model.id.to_string();
                        }
                        inner.status = AppStatus::Ready;
                        inner.status_message = "Ready".to_string();
                        inner.install_progress = Some(1.0);
                        inner.error_message = None;
                    })
                    .await;

                    {
                        let guard = state.0.lock().await;
                        let _ = save_settings(&guard);
                    }
                }
                "error" => {
                    let message = event
                        .message
                        .unwrap_or_else(|| "Dependency/model installation failed".to_string());
                    let message_for_state = message.clone();
                    update_state(app, state, move |inner| {
                        inner.status = AppStatus::Error;
                        inner.status_message = "Dependency/model installation failed".to_string();
                        inner.error_message = Some(message_for_state.clone());
                        inner.install_progress = None;
                    })
                    .await;
                    let _ = child.kill().await;
                    let _ = child.wait().await;
                    return Err(anyhow!(message));
                }
                _ => {}
            }
        }
    }

    let status = child
        .wait()
        .await
        .context("failed waiting for bootstrap script")?;

    let stderr_output = stderr_task.await.unwrap_or_default();

    if !status.success() {
        let message = if stderr_output.trim().is_empty() {
            "Dependency/model installation failed with a non-zero exit code".to_string()
        } else {
            format!("Dependency/model installation failed: {stderr_output}")
        };
        let message_for_state = message.clone();

        update_state(app, state, move |inner| {
            inner.status = AppStatus::Error;
            inner.status_message = "Dependency/model installation failed".to_string();
            inner.error_message = Some(message_for_state.clone());
            inner.install_progress = None;
        })
        .await;

        return Err(anyhow!(message));
    }

    {
        let guard = state.0.lock().await;
        if guard.status != AppStatus::Ready {
            drop(guard);
            update_state(app, state, |inner| {
                inner.status = AppStatus::Ready;
                inner.status_message = "Ready".to_string();
                inner.install_progress = Some(1.0);
                inner.error_message = None;
            })
            .await;
        }
    }

    emit_state(app, state).await;
    Ok(())
}
