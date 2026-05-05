mod llm;

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
