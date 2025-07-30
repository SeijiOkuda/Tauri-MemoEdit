// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
#[tauri::command]

fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

use tauri::{Manager, WindowEvent, Window, Emitter};

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
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .on_window_event(|window: &Window, event: &WindowEvent| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close(); // 自動で閉じるのをキャンセル
                let _ = window.emit("app-close-requested", ());
            }
        })
        .invoke_handler(tauri::generate_handler![greet, frontend_ready])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
