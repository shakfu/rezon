mod llm;

use tauri::menu::{
    AboutMetadataBuilder, MenuBuilder, MenuItemBuilder, PredefinedMenuItem, SubmenuBuilder,
};
use tauri::{Emitter, Manager};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(llm::LlmState::default())
        .invoke_handler(tauri::generate_handler![
            llm::load_model,
            llm::model_status,
            llm::chat,
            llm::cancel_chat,
            llm::cloud_providers
        ])
        .setup(|app| {
            // ---- Native menu (macOS app menu + Edit) ----
            let settings_item = MenuItemBuilder::with_id("settings", "Settings...")
                .accelerator("CmdOrCtrl+,")
                .build(app)?;

            let about_metadata = AboutMetadataBuilder::new().name(Some("Rezo")).build();

            let app_menu = SubmenuBuilder::new(app, "Rezo")
                .item(&PredefinedMenuItem::about(
                    app,
                    Some("About Rezo"),
                    Some(about_metadata),
                )?)
                .separator()
                .item(&settings_item)
                .separator()
                .item(&PredefinedMenuItem::services(app, None)?)
                .separator()
                .item(&PredefinedMenuItem::hide(app, None)?)
                .item(&PredefinedMenuItem::hide_others(app, None)?)
                .item(&PredefinedMenuItem::show_all(app, None)?)
                .separator()
                .item(&PredefinedMenuItem::quit(app, None)?)
                .build()?;

            // Edit menu — required on macOS for Cut/Copy/Paste shortcuts to
            // work in text inputs once a custom menu is installed.
            let edit_menu = SubmenuBuilder::new(app, "Edit")
                .item(&PredefinedMenuItem::undo(app, None)?)
                .item(&PredefinedMenuItem::redo(app, None)?)
                .separator()
                .item(&PredefinedMenuItem::cut(app, None)?)
                .item(&PredefinedMenuItem::copy(app, None)?)
                .item(&PredefinedMenuItem::paste(app, None)?)
                .item(&PredefinedMenuItem::select_all(app, None)?)
                .build()?;

            let menu = MenuBuilder::new(app)
                .item(&app_menu)
                .item(&edit_menu)
                .build()?;
            app.set_menu(menu)?;

            app.on_menu_event(|app, event| {
                if event.id() == "settings" {
                    let _ = app.emit("open-settings", ());
                }
            });

            // ---- Auto-load last model ----
            let handle = app.handle().clone();
            if let Some(path) = llm::read_last_model(&handle) {
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = llm::do_load(&handle, path).await {
                        eprintln!("auto-load failed: {e}");
                        let _ = handle.emit("model-load-error", e);
                    }
                });
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|handle, event| {
        if let tauri::RunEvent::Exit = event {
            handle.state::<llm::LlmState>().shutdown();
        }
    });
}
