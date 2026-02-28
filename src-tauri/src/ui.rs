use tauri::{AppHandle, Manager, PhysicalSize, WebviewUrl, WebviewWindowBuilder};

pub fn ensure_tray_window(app: &AppHandle) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window("tray") {
        let _ = window.set_decorations(true);
        let _ = window.set_always_on_top(false);
        let _ = window.set_shadow(true);
        return Ok(());
    }

    WebviewWindowBuilder::new(app, "tray", WebviewUrl::App("index.html".into()))
        .title("WhisperBar")
        .inner_size(560.0, 840.0)
        .min_inner_size(540.0, 780.0)
        .resizable(true)
        .decorations(true)
        .transparent(false)
        .shadow(true)
        .always_on_top(false)
        .visible(false)
        .skip_taskbar(true)
        .build()?;

    Ok(())
}

pub fn toggle_tray_window(app: &AppHandle) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window("tray") {
        let visible = window.is_visible().unwrap_or(false);
        if visible {
            let _ = window.hide();
        } else {
            let _ = window.show();
            let _ = window.set_focus();
        }
    }

    Ok(())
}

pub fn show_tray_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("tray") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

pub fn hide_tray_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("tray") {
        let _ = window.hide();
    }
}

pub fn ensure_floating_window(app: &AppHandle) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window("floating") {
        let _ = window.set_decorations(true);
        let _ = window.set_always_on_top(true);
        let _ = window.set_shadow(true);
        let _ = window.show();
        let _ = window.set_focus();
        return Ok(());
    }

    WebviewWindowBuilder::new(app, "floating", WebviewUrl::App("index.html".into()))
        .title("WhisperBar Live")
        .inner_size(1080.0, 760.0)
        .min_inner_size(860.0, 560.0)
        .resizable(true)
        .decorations(true)
        .transparent(false)
        .shadow(true)
        .always_on_top(true)
        .visible(true)
        .skip_taskbar(false)
        .build()?;

    if let Some(window) = app.get_webview_window("floating") {
        let _ = window.set_size(PhysicalSize::new(1080, 760));
    }

    Ok(())
}

pub fn close_floating_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("floating") {
        let _ = window.close();
    }
}
