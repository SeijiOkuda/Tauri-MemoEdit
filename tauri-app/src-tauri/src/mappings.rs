use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

// -----------------------------------------------------------------------
// 型定義
// -----------------------------------------------------------------------

/// ローカルパス → Drive ファイルID のマッピング全体
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct FileMappings {
    /// key: ローカルファイルのフルパス, value: Drive ファイルID
    pub local: HashMap<String, String>,
    /// クラウド専用タブ（ローカルパスなし）のエントリ
    pub cloud_only: Vec<CloudOnlyEntry>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CloudOnlyEntry {
    pub drive_file_id: String,
    pub name: String,
}

// -----------------------------------------------------------------------
// ファイルパス
// -----------------------------------------------------------------------

fn mappings_path(app: &AppHandle) -> Option<std::path::PathBuf> {
    app.path()
        .app_data_dir()
        .ok()
        .map(|p| p.join("file_mappings.json"))
}

// -----------------------------------------------------------------------
// 読み書き
// -----------------------------------------------------------------------

pub fn load_mappings(app: &AppHandle) -> FileMappings {
    let path = match mappings_path(app) {
        Some(p) => p,
        None => return FileMappings::default(),
    };
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return FileMappings::default(),
    };
    serde_json::from_str(&content).unwrap_or_default()
}

fn save_mappings(app: &AppHandle, mappings: &FileMappings) -> Result<(), String> {
    let path = mappings_path(app).ok_or("パスが取得できません")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(
        path,
        serde_json::to_string_pretty(mappings).unwrap(),
    )
    .map_err(|e| e.to_string())
}

// -----------------------------------------------------------------------
// コマンド
// -----------------------------------------------------------------------

/// ローカルパスと Drive ファイルID を紐づけ保存
#[tauri::command]
pub async fn mapping_set_local(
    app: AppHandle,
    local_path: String,
    drive_file_id: String,
) -> Result<(), String> {
    let mut m = load_mappings(&app);
    m.local.insert(local_path, drive_file_id);
    save_mappings(&app, &m)
}

/// ローカルパスの紐づけを削除
#[tauri::command]
pub async fn mapping_remove_local(
    app: AppHandle,
    local_path: String,
) -> Result<(), String> {
    let mut m = load_mappings(&app);
    m.local.remove(&local_path);
    save_mappings(&app, &m)
}

/// クラウド専用エントリを追加
#[tauri::command]
pub async fn mapping_add_cloud_only(
    app: AppHandle,
    drive_file_id: String,
    name: String,
) -> Result<(), String> {
    let mut m = load_mappings(&app);
    if !m.cloud_only.iter().any(|e| e.drive_file_id == drive_file_id) {
        m.cloud_only.push(CloudOnlyEntry { drive_file_id, name });
    }
    save_mappings(&app, &m)
}

/// クラウド専用エントリを削除
#[tauri::command]
pub async fn mapping_remove_cloud_only(
    app: AppHandle,
    drive_file_id: String,
) -> Result<(), String> {
    let mut m = load_mappings(&app);
    m.cloud_only.retain(|e| e.drive_file_id != drive_file_id);
    save_mappings(&app, &m)
}

/// マッピング全体を取得
#[tauri::command]
pub async fn mapping_get_all(app: AppHandle) -> Result<FileMappings, String> {
    Ok(load_mappings(&app))
}
