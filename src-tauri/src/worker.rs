use std::{
    fs,
    process::Stdio,
    time::{Duration, Instant},
};

use anyhow::{anyhow, Context};
use serde::Deserialize;
use tauri::AppHandle;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::Command,
    time::timeout,
};

use crate::{
    app_state::{emit_state, update_state, AppStatus, SharedState, WorkerProcess},
    runtime_scripts, transcript_file, ui,
};

#[derive(Debug, Deserialize)]
struct WorkerEvent {
    #[serde(rename = "type")]
    event_type: String,
    text: Option<String>,
    message: Option<String>,
}

pub async fn start_recording(app: &AppHandle, state: &SharedState) -> anyhow::Result<()> {
    runtime_scripts::ensure_scripts(state).await?;

    let (
        venv_python,
        worker_script,
        selected_mic_device,
    ) = {
        let guard = state.0.lock().await;

        if guard.status == AppStatus::Recording {
            return Err(anyhow!("recording is already active"));
        }

        if !matches!(guard.status, AppStatus::Ready | AppStatus::Idle) {
            return Err(anyhow!(
                "dependencies are not ready yet. wait for installation to finish"
            ));
        }

        (
            guard.venv_python.clone(),
            guard.worker_script.clone(),
            guard.selected_mic_device.clone(),
        )
    };

    if !venv_python.exists() {
        return Err(anyhow!(
            "Python environment is missing. Retry dependency installation"
        ));
    }

    let (venv_python, model_path, language) = {
        let guard = state.0.lock().await;
        (
            guard.venv_python.clone(),
            guard.model_path.clone(),
            guard.language.clone(),
        )
    };

    if !model_ready(&model_path) {
        return Err(anyhow!(
            "Selected model is not installed. Click Install Model first."
        ));
    }

    let mut command = Command::new(&venv_python);
    command
        .arg(&worker_script)
        .arg("--language")
        .arg(language)
        .arg("--model-path")
        .arg(&model_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    if let Ok(exe_path) = std::env::current_exe() {
        command.arg("--sck-helper-path").arg(exe_path);
    }

    if let Ok(audio_device) = std::env::var("WHISPERBAR_AUDIO_DEVICE") {
        if !audio_device.trim().is_empty() {
            command.arg("--audio-device").arg(audio_device);
        }
    }
    if let Some(mic_device) = selected_mic_device {
        if !mic_device.trim().is_empty() {
            command.arg("--mic-device").arg(mic_device);
        }
    } else if let Ok(mic_device) = std::env::var("WHISPERBAR_MIC_DEVICE") {
        if !mic_device.trim().is_empty() {
            command.arg("--mic-device").arg(mic_device);
        }
    }
    let mut child = command
        .spawn()
        .with_context(|| format!("failed starting worker with {}", venv_python.display()))?;

    let stdout = child
        .stdout
        .take()
        .context("unable to capture worker stdout")?;
    let stderr = child
        .stderr
        .take()
        .context("unable to capture worker stderr")?;
    let stdin = child.stdin.take();

    let app_stdout = app.clone();
    let state_stdout = state.clone();
    let stdout_task = tauri::async_runtime::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();

        while let Ok(Some(line)) = lines.next_line().await {
            handle_worker_event(&app_stdout, &state_stdout, &line).await;
        }
    });

    let app_stderr = app.clone();
    let state_stderr = state.clone();
    let stderr_task = tauri::async_runtime::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();

        while let Ok(Some(line)) = lines.next_line().await {
            if !line.trim().is_empty() {
                update_state(&app_stderr, &state_stderr, |inner| {
                    if inner.status == AppStatus::Recording {
                        inner.status_message = format!("Recording ({line})");
                    }
                })
                .await;
            }
        }
    });

    update_state(app, state, |inner| {
        inner.worker = Some(WorkerProcess {
            child,
            stdin,
            stdout_task: Some(stdout_task),
            stderr_task: Some(stderr_task),
        });
        inner.status = AppStatus::Recording;
        inner.status_message = "Recording".to_string();
        inner.error_message = None;
        inner.last_saved_path = None;
        inner.transcript.clear();
    })
    .await;

    ui::hide_tray_window(app);
    ui::ensure_floating_window(app)?;

    emit_state(app, state).await;

    Ok(())
}

fn model_ready(model_path: &std::path::Path) -> bool {
    if !model_path.exists() || !model_path.join("config.json").exists() {
        return false;
    }

    fs::read_dir(model_path)
        .ok()
        .into_iter()
        .flat_map(|entries| entries.flatten())
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .any(|name| name.starts_with("weights.") || name.starts_with("model"))
}

async fn handle_worker_event(app: &AppHandle, state: &SharedState, line: &str) {
    let Ok(event) = serde_json::from_str::<WorkerEvent>(line) else {
        return;
    };

    match event.event_type.as_str() {
        "status" => {
            if let Some(message) = event.message {
                update_state(app, state, move |inner| {
                    if inner.status == AppStatus::Recording {
                        inner.status_message = message;
                    }
                })
                .await;
            }
        }
        "partial" => {
            if let Some(text) = event.text {
                if text.trim().is_empty() {
                    return;
                }

                update_state(app, state, move |inner| {
                    if !inner.transcript.is_empty() {
                        inner.transcript.push('\n');
                    }
                    inner.transcript.push_str(text.trim());
                    if inner.status == AppStatus::Recording {
                        inner.status_message = "Recording".to_string();
                    }
                })
                .await;
            }
        }
        "final" => {
            if let Some(text) = event.text {
                update_state(app, state, move |inner| {
                    if !text.trim().is_empty() && text.len() > inner.transcript.len() {
                        inner.transcript = text;
                    }
                })
                .await;
            }
        }
        "error" => {
            let message = event
                .message
                .unwrap_or_else(|| "Worker reported an unknown error".to_string());

            update_state(app, state, move |inner| {
                inner.status = AppStatus::Error;
                inner.status_message = "Recording error".to_string();
                inner.error_message = Some(message);
                inner.worker = None;
            })
            .await;

            ui::close_floating_window(app);
            ui::show_tray_window(app);
        }
        _ => {}
    }
}

pub async fn stop_recording(app: &AppHandle, state: &SharedState) -> anyhow::Result<String> {
    let mut worker = {
        let mut guard = state.0.lock().await;
        if guard.status != AppStatus::Recording {
            return Err(anyhow!("recording is not active"));
        }

        guard.status_message = "Stopping recording".to_string();
        guard
            .worker
            .take()
            .ok_or_else(|| anyhow!("missing worker process"))?
    };

    emit_state(app, state).await;

    if let Some(stdin) = worker.stdin.as_mut() {
        stdin
            .write_all(b"stop\n")
            .await
            .context("failed signaling worker to stop")?;
    }

    let start_wait = Instant::now();
    if timeout(Duration::from_secs(15), worker.child.wait())
        .await
        .is_err()
    {
        worker.child.kill().await.context("failed killing worker")?;
        let _ = worker.child.wait().await;

        update_state(app, state, move |inner| {
            inner.status_message = format!(
                "Worker forced to stop after {}s",
                start_wait.elapsed().as_secs()
            );
        })
        .await;
    }

    if let Some(stdout_task) = worker.stdout_task.take() {
        let _ = timeout(Duration::from_secs(3), stdout_task).await;
    }
    if let Some(stderr_task) = worker.stderr_task.take() {
        let _ = timeout(Duration::from_secs(3), stderr_task).await;
    }

    let transcript = {
        let guard = state.0.lock().await;
        guard.transcript.clone()
    };

    if transcript.trim().is_empty() {
        update_state(app, state, |inner| {
            inner.status = AppStatus::Error;
            inner.status_message = "No transcript captured".to_string();
            inner.error_message = Some(
                "No speech was captured. Check microphone permission and audio input device."
                    .to_string(),
            );
            inner.worker = None;
        })
        .await;
        ui::close_floating_window(app);
        ui::show_tray_window(app);
        return Err(anyhow!(
            "No speech was captured. Check microphone permission and audio input device."
        ));
    }

    let file_path = transcript_file::save_markdown(&transcript).await?;
    let file_path_str = file_path.display().to_string();

    update_state(app, state, move |inner| {
        inner.status = AppStatus::Ready;
        inner.status_message = "Ready".to_string();
        inner.last_saved_path = Some(file_path_str.clone());
        inner.error_message = None;
        inner.install_progress = Some(1.0);
        inner.worker = None;
    })
    .await;

    ui::close_floating_window(app);
    ui::show_tray_window(app);

    Ok(file_path.display().to_string())
}
