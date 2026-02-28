use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{
    image::Image,
    menu::{Menu, MenuItem},
    AppHandle, Manager,
};

use crate::{app_state::SharedState, ui, worker};

pub fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Show WhisperBar", true, None::<&str>)?;
    let stop_recording =
        MenuItem::with_id(app, "stop_recording", "Stop Recording", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit WhisperBar", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &stop_recording, &quit])?;

    let builder = TrayIconBuilder::with_id("whisperbar")
        .icon(tray_template_icon())
        .icon_as_template(true)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => {
                ui::show_tray_window(app);
            }
            "stop_recording" => {
                let app_handle = app.clone();
                let state = app_handle.state::<SharedState>().inner().clone();
                tauri::async_runtime::spawn(async move {
                    let _ = worker::stop_recording(&app_handle, &state).await;
                });
            }
            "quit" => {
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                let _ = ui::toggle_tray_window(&app);
            }
        });

    builder.build(app)?;

    Ok(())
}

fn tray_template_icon() -> Image<'static> {
    Image::from_bytes(include_bytes!("../icons/tray-template.png"))
        .expect("invalid tray-template icon bytes")
}
