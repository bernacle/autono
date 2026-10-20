pub mod bridge;

#[tauri::command]
async fn daemon(request: bridge::Request) -> Result<serde_json::Value, bridge::BridgeError> {
    bridge::Bridge::discover()?.dispatch(request).await
}

#[tauri::command]
async fn inspect_artifact(
    run_id: uuid::Uuid,
    path: String,
) -> Result<serde_json::Value, bridge::BridgeError> {
    bridge::Bridge::discover()?
        .inspect_artifact(run_id, path)
        .await
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![daemon, inspect_artifact])
        .run(tauri::generate_context!())
        .expect("run Autono desktop");
}
