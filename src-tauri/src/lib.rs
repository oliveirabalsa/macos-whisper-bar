#![cfg(target_os = "macos")]

mod app_state;
mod audio;
mod bootstrap;
mod models;
mod runtime_scripts;
mod sck_audio_helper;
mod transcript_file;
mod tray;
mod ui;
mod worker;

use app_state::{save_settings, snapshot, update_state, AppSnapshot, AppStatus, SharedState};
use audio::AudioDeviceOption;
use models::ModelOption;
use tauri::{ActivationPolicy, AppHandle, Manager, State};

#[tauri::command]
async fn get_app_state(state: State<'_, SharedState>) -> Result<AppSnapshot, String> {
    Ok(snapshot(state.inner()).await)
}

#[tauri::command]
async fn set_language(
    app: AppHandle,
    state: State<'_, SharedState>,
    language: String,
) -> Result<(), String> {
    if language != "en" && language != "pt-BR" {
        return Err("unsupported language".to_string());
    }

    update_state(&app, state.inner(), |inner| {
        inner.language = language;
    })
    .await;

    {
        let guard = state.inner().0.lock().await;
        let _ = save_settings(&guard);
    }

    Ok(())
}

#[tauri::command]
async fn get_model_options() -> Result<Vec<ModelOption>, String> {
    Ok(models::model_options())
}

#[tauri::command]
async fn refresh_audio_devices(
    app: AppHandle,
    state: State<'_, SharedState>,
) -> Result<Vec<AudioDeviceOption>, String> {
    refresh_audio_devices_inner(&app, state.inner()).await
}

async fn refresh_audio_devices_inner(
    app: &AppHandle,
    state: &SharedState,
) -> Result<Vec<AudioDeviceOption>, String> {
    let devices = audio::list_audio_devices()
        .await
        .map_err(|error| error.to_string())?;

    update_state(app, state, |inner| {
        let mic_valid = inner
            .selected_mic_device
            .as_ref()
            .map(|id| devices.iter().any(|device| device.id == *id))
            .unwrap_or(false);

        if !mic_valid {
            inner.selected_mic_device = audio::choose_default_mic(&devices);
        }

    })
    .await;

    {
        let guard = state.0.lock().await;
        let _ = save_settings(&guard);
    }

    Ok(devices)
}

#[tauri::command]
async fn set_audio_inputs(
    app: AppHandle,
    state: State<'_, SharedState>,
    mic_device: Option<String>,
) -> Result<(), String> {
    {
        let guard = state.inner().0.lock().await;
        if guard.status == AppStatus::Recording {
            return Err("cannot change audio input while recording".to_string());
        }
    }

    update_state(&app, state.inner(), |inner| {
        inner.selected_mic_device = mic_device.clone().filter(|value| !value.trim().is_empty());
    })
    .await;

    {
        let guard = state.inner().0.lock().await;
        let _ = save_settings(&guard);
    }

    Ok(())
}

#[tauri::command]
async fn set_model(
    app: AppHandle,
    state: State<'_, SharedState>,
    model_id: String,
) -> Result<(), String> {
    if models::find_model(&model_id).is_none() {
        return Err(format!("unsupported model id: {model_id}"));
    }

    {
        let guard = state.inner().0.lock().await;
        if guard.status == AppStatus::Recording {
            return Err("cannot change model while recording".to_string());
        }
    }

    let model_path = models::model_path(
        &{
            let guard = state.inner().0.lock().await;
            guard.app_data_dir.clone()
        },
        &model_id,
    )
    .ok_or_else(|| format!("unsupported model id: {model_id}"))?;

    update_state(&app, state.inner(), |inner| {
        inner.selected_model_id = model_id.clone();
        inner.model_path = model_path.clone();
        inner.error_message = None;
        inner.status = AppStatus::Ready;
        inner.status_message = "Model selected. Click Install Model if missing.".to_string();
    })
    .await;

    {
        let guard = state.inner().0.lock().await;
        let _ = save_settings(&guard);
    }

    Ok(())
}

#[tauri::command]
async fn install_selected_model(
    app: AppHandle,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    let selected_model_id = {
        let guard = state.inner().0.lock().await;
        guard.selected_model_id.clone()
    };

    bootstrap::run_bootstrap_for_model(&app, state.inner(), &selected_model_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn start_recording(app: AppHandle, state: State<'_, SharedState>) -> Result<(), String> {
    if let Err(error) = worker::start_recording(&app, state.inner()).await {
        let message = error.to_string();
        set_error(&app, state.inner(), message.clone());
        return Err(message);
    }

    Ok(())
}

#[tauri::command]
async fn stop_recording(app: AppHandle, state: State<'_, SharedState>) -> Result<String, String> {
    match worker::stop_recording(&app, state.inner()).await {
        Ok(path) => Ok(path),
        Err(error) => {
            let message = error.to_string();
            set_error(&app, state.inner(), message.clone());
            Err(message)
        }
    }
}

#[tauri::command]
async fn retry_bootstrap(app: AppHandle, state: State<'_, SharedState>) -> Result<(), String> {
    bootstrap::run_bootstrap(&app, state.inner())
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn clear_error(app: AppHandle, state: State<'_, SharedState>) -> Result<(), String> {
    update_state(&app, state.inner(), |inner| {
        inner.error_message = None;
        if inner.status == AppStatus::Error {
            if inner.install_progress == Some(1.0) {
                inner.status = AppStatus::Ready;
                inner.status_message = "Ready".to_string();
            } else {
                inner.status = AppStatus::Ready;
                inner.status_message = "Ready".to_string();
            }
        }
    })
    .await;

    Ok(())
}

fn set_error(app: &AppHandle, state: &SharedState, message: String) {
    let app = app.clone();
    let state = state.clone();

    tauri::async_runtime::spawn(async move {
        update_state(&app, &state, move |inner| {
            inner.status = AppStatus::Error;
            inner.status_message = "Error".to_string();
            inner.error_message = Some(message);
            inner.install_progress = None;
        })
        .await;
    });
}

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let app_handle = app.handle().clone();
            app.set_activation_policy(ActivationPolicy::Accessory);

            let app_data_dir = app.path().app_data_dir()?;

            let state = SharedState::new(app_data_dir);
            app.manage(state.clone());

            ui::ensure_tray_window(&app_handle)?;
            tray::build_tray(&app_handle)?;

            let state_for_bootstrap = state.clone();
            tauri::async_runtime::spawn(async move {
                app_state::emit_state(&app_handle, &state_for_bootstrap).await;
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_app_state,
            set_language,
            get_model_options,
            refresh_audio_devices,
            set_audio_inputs,
            set_model,
            install_selected_model,
            start_recording,
            stop_recording,
            retry_bootstrap,
            clear_error
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

pub fn run_sck_audio_helper() -> Result<(), String> {
    sck_audio_helper::run().map_err(|error| error.to_string())
}
