use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DriveFile {
    pub id: String,
    pub name: String,
}

// -----------------------------------------------------------------------
// ファイル作成 (multipart/related)
// -----------------------------------------------------------------------

#[tauri::command]
pub async fn drive_create_file(
    name: String,
    content: String,
    access_token: String,
) -> Result<DriveFile, String> {
    let client = reqwest::Client::new();
    let boundary = "MemoEditBoundary_XYZ_1234567890";
    let metadata = serde_json::json!({ "name": name, "mimeType": "text/plain" }).to_string();

    let body = format!(
        "--{b}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{m}\r\n\
         --{b}\r\nContent-Type: text/plain; charset=UTF-8\r\n\r\n{c}\r\n\
         --{b}--",
        b = boundary,
        m = metadata,
        c = content,
    );

    let resp = client
        .post("https://www.googleapis.com/upload/drive/v3/files?uploadType=multipart&fields=id,name")
        .header("Authorization", format!("Bearer {}", access_token))
        .header("Content-Type", format!("multipart/related; boundary={}", boundary))
        .body(body)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    let json: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;

    if let Some(e) = json.get("error") {
        return Err(format!("Drive create error: {}", e));
    }

    Ok(DriveFile {
        id: json["id"].as_str().ok_or("id missing")?.to_string(),
        name: json["name"].as_str().unwrap_or(&name).to_string(),
    })
}

// -----------------------------------------------------------------------
// ファイル内容更新 (simple media upload)
// -----------------------------------------------------------------------

#[tauri::command]
pub async fn drive_update_file(
    file_id: String,
    content: String,
    access_token: String,
) -> Result<(), String> {
    let client = reqwest::Client::new();

    let resp = client
        .patch(format!(
            "https://www.googleapis.com/upload/drive/v3/files/{}?uploadType=media",
            file_id
        ))
        .header("Authorization", format!("Bearer {}", access_token))
        .header("Content-Type", "text/plain; charset=UTF-8")
        .body(content)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if resp.status().is_success() {
        return Ok(());
    }

    let json: serde_json::Value = resp.json().await.unwrap_or_default();
    Err(format!(
        "Drive update error: {}",
        json.get("error").unwrap_or(&serde_json::Value::Null)
    ))
}

// -----------------------------------------------------------------------
// ファイル削除
// -----------------------------------------------------------------------

#[tauri::command]
pub async fn drive_delete_file(
    file_id: String,
    access_token: String,
) -> Result<(), String> {
    let client = reqwest::Client::new();

    let resp = client
        .delete(format!(
            "https://www.googleapis.com/drive/v3/files/{}",
            file_id
        ))
        .header("Authorization", format!("Bearer {}", access_token))
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if resp.status().as_u16() == 204 || resp.status().is_success() {
        return Ok(());
    }

    let json: serde_json::Value = resp.json().await.unwrap_or_default();
    Err(format!(
        "Drive delete error: {}",
        json.get("error").unwrap_or(&serde_json::Value::Null)
    ))
}
