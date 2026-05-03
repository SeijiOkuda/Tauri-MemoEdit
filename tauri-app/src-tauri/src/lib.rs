mod auth;
mod drive;

use auth::{AuthState, AuthStateMutex, OAuthConfig, OAuthConfigMutex};
use tauri::{Emitter, Manager, Window, WindowEvent};

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[tauri::command]
async fn frontend_ready(app: tauri::AppHandle) {
    let args: Vec<String> = std::env::args().collect();
    println!("📂 フロントから準備完了通知 → args: {:?}", args);
    if args.len() > 1 {
        let file_path = &args[1];
        println!("📂 フロントから準備完了通知 → ファイル送信: {}", file_path);
        if let Some(window) = app.get_webview_window("main") {
            window.emit("open-file", file_path.clone()).unwrap();
        }
    }
}

pub fn run() {
    tauri::Builder::default()
        .manage(AuthStateMutex::new(AuthState::default()))
        .manage(OAuthConfigMutex::new(OAuthConfig::default()))
        .setup(|app| {
            // 保存済み OAuth 設定を復元
            if let Some(config) = auth::load_oauth_config(app.handle()) {
                let state = app.state::<OAuthConfigMutex>();
                let mut cfg = state.lock().unwrap();
                *cfg = config;
            }
            // 保存済み認証トークンを復元
            if let Some((token, user_info)) = auth::load_auth_state(app.handle()) {
                let state = app.state::<AuthStateMutex>();
                let mut auth = state.lock().unwrap();
                auth.token = Some(token);
                auth.user_info = Some(user_info);
            }
            Ok(())
        })
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .on_window_event(|window: &Window, event: &WindowEvent| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.emit("app-close-requested", ());
            }
        })
        .invoke_handler(tauri::generate_handler![
            greet,
            frontend_ready,
            auth::start_google_auth,
            auth::get_auth_user,
            auth::get_access_token,
            auth::sign_out,
            auth::save_oauth_config,
            auth::get_oauth_config,
            drive::drive_create_file,
            drive::drive_update_file,
            drive::drive_delete_file,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
