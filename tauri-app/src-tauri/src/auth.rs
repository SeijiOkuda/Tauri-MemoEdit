use std::sync::Mutex;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand::{distributions::Alphanumeric, Rng};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager};

// -----------------------------------------------------------------------
// OAuth クライアント設定 (UIから入力してファイルに保存)
// -----------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct OAuthConfig {
    pub client_id: String,
    pub client_secret: String,
}

pub type OAuthConfigMutex = Mutex<OAuthConfig>;

fn oauth_config_path(app: &AppHandle) -> Option<std::path::PathBuf> {
    app.path().app_data_dir().ok().map(|p| p.join("google_oauth.json"))
}

pub fn load_oauth_config(app: &AppHandle) -> Option<OAuthConfig> {
    let path = oauth_config_path(app)?;
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

#[tauri::command]
pub async fn save_oauth_config(
    app: AppHandle,
    config_state: tauri::State<'_, OAuthConfigMutex>,
    client_id: String,
    client_secret: String,
) -> Result<(), String> {
    let config = OAuthConfig { client_id, client_secret };
    let path = oauth_config_path(&app).ok_or("設定ファイルのパスが取得できません")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, serde_json::to_string(&config).unwrap())
        .map_err(|e| e.to_string())?;
    let mut cfg = config_state.lock().map_err(|e| e.to_string())?;
    *cfg = config;
    Ok(())
}

#[tauri::command]
pub async fn get_oauth_config(
    config_state: tauri::State<'_, OAuthConfigMutex>,
) -> Result<OAuthConfig, String> {
    let cfg = config_state.lock().map_err(|e| e.to_string())?;
    Ok(cfg.clone())
}

// -----------------------------------------------------------------------
// 認証トークン / ユーザー情報
// -----------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct UserInfo {
    pub name: String,
    pub email: String,
    pub picture: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TokenData {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: u64,
}

#[derive(Default)]
pub struct AuthState {
    pub token: Option<TokenData>,
    pub user_info: Option<UserInfo>,
}

pub type AuthStateMutex = Mutex<AuthState>;

// -----------------------------------------------------------------------
// 認証状態の永続化
// -----------------------------------------------------------------------

fn auth_data_path(app: &AppHandle) -> Option<std::path::PathBuf> {
    app.path().app_data_dir().ok().map(|p| p.join("auth.json"))
}

pub fn load_auth_state(app: &AppHandle) -> Option<(TokenData, UserInfo)> {
    let path = auth_data_path(app)?;
    let content = std::fs::read_to_string(path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&content).ok()?;
    let token: TokenData = serde_json::from_value(json["token"].clone()).ok()?;
    let user_info: UserInfo = serde_json::from_value(json["user_info"].clone()).ok()?;
    Some((token, user_info))
}

fn save_auth_state(app: &AppHandle, token: &TokenData, user_info: &UserInfo) {
    if let Some(path) = auth_data_path(app) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let json = serde_json::json!({ "token": token, "user_info": user_info });
        let _ = std::fs::write(path, json.to_string());
    }
}

fn clear_auth_state(app: &AppHandle) {
    if let Some(path) = auth_data_path(app) {
        let _ = std::fs::remove_file(path);
    }
}

// -----------------------------------------------------------------------
// PKCE / 空きポート
// -----------------------------------------------------------------------

fn generate_pkce() -> (String, String) {
    let verifier: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(64)
        .map(char::from)
        .collect();
    let hash = Sha256::digest(verifier.as_bytes());
    let challenge = URL_SAFE_NO_PAD.encode(hash);
    (verifier, challenge)
}

fn find_free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

// -----------------------------------------------------------------------
// コマンド: ログイン開始
// -----------------------------------------------------------------------

#[tauri::command]
pub async fn start_google_auth(
    app: AppHandle,
    config_state: tauri::State<'_, OAuthConfigMutex>,
) -> Result<(), String> {
    let (client_id, client_secret) = {
        let cfg = config_state.lock().map_err(|e| e.to_string())?;
        if cfg.client_id.is_empty() {
            return Err("OAuth認証情報が設定されていません".to_string());
        }
        (cfg.client_id.clone(), cfg.client_secret.clone())
    };

    let (verifier, challenge) = generate_pkce();
    let port = find_free_port();
    let redirect_uri = format!("http://localhost:{}/callback", port);

    let state_param: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(16)
        .map(char::from)
        .collect();

    let auth_url = format!(
        "https://accounts.google.com/o/oauth2/v2/auth\
        ?client_id={}\
        &redirect_uri={}\
        &response_type=code\
        &scope={}\
        &code_challenge={}\
        &code_challenge_method=S256\
        &state={}\
        &access_type=offline\
        &prompt=consent",
        client_id,
        urlencoding::encode(&redirect_uri),
        urlencoding::encode("https://www.googleapis.com/auth/drive.file openid email profile"),
        challenge,
        state_param,
    );

    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_url(&auth_url, None::<&str>)
        .map_err(|e| e.to_string())?;

    let app_clone = app.clone();
    tauri::async_runtime::spawn(async move {
        let result =
            tauri::async_runtime::spawn_blocking(move || wait_for_callback(port)).await;

        match result {
            Ok(Ok(code)) => {
                match exchange_code(&code, &verifier, &redirect_uri, &client_id, &client_secret)
                    .await
                {
                    Ok(token_data) => match fetch_user_info(&token_data.access_token).await {
                        Ok(user_info) => {
                            save_auth_state(&app_clone, &token_data, &user_info);
                            if let Some(state) = app_clone.try_state::<AuthStateMutex>() {
                                let mut auth = state.lock().unwrap();
                                auth.token = Some(token_data);
                                auth.user_info = Some(user_info.clone());
                            }
                            let _ = app_clone.emit("auth-complete", user_info);
                        }
                        Err(e) => {
                            let _ = app_clone.emit("auth-error", e);
                        }
                    },
                    Err(e) => {
                        let _ = app_clone.emit("auth-error", e);
                    }
                }
            }
            Ok(Err(e)) => {
                let _ = app_clone.emit("auth-error", e);
            }
            Err(e) => {
                let _ = app_clone.emit("auth-error", e.to_string());
            }
        }
    });

    Ok(())
}

// -----------------------------------------------------------------------
// コールバック待受 (blocking)
// -----------------------------------------------------------------------

fn wait_for_callback(port: u16) -> Result<String, String> {
    let server = tiny_http::Server::http(format!("127.0.0.1:{}", port))
        .map_err(|e| e.to_string())?;

    let request = server
        .recv_timeout(std::time::Duration::from_secs(120))
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "認証がタイムアウトしました".to_string())?;

    let url = request.url().to_string();

    let html = "<!DOCTYPE html><html><head><meta charset='utf-8'></head>\
        <body style='font-family:sans-serif;text-align:center;padding:3rem;\
        background:#1a1a1a;color:#f6f6f6'>\
        <h2>認証完了</h2><p>このタブを閉じてアプリに戻ってください。</p>\
        </body></html>";

    let response = tiny_http::Response::from_string(html).with_header(
        "Content-Type: text/html; charset=utf-8"
            .parse::<tiny_http::Header>()
            .unwrap(),
    );
    let _ = request.respond(response);

    let code = url
        .split('?')
        .nth(1)
        .and_then(|q| q.split('&').find(|p| p.starts_with("code=")))
        .and_then(|p| p.strip_prefix("code="))
        .map(|s| {
            urlencoding::decode(s)
                .map(|c| c.into_owned())
                .unwrap_or_else(|_| s.to_string())
        })
        .ok_or_else(|| "認証コードが取得できませんでした".to_string())?;

    Ok(code)
}

// -----------------------------------------------------------------------
// トークン交換
// -----------------------------------------------------------------------

async fn exchange_code(
    code: &str,
    verifier: &str,
    redirect_uri: &str,
    client_id: &str,
    client_secret: &str,
) -> Result<TokenData, String> {
    let client = reqwest::Client::new();
    let params = [
        ("client_id", client_id),
        ("client_secret", client_secret),
        ("code", code),
        ("code_verifier", verifier),
        ("grant_type", "authorization_code"),
        ("redirect_uri", redirect_uri),
    ];

    let json: serde_json::Value = client
        .post("https://oauth2.googleapis.com/token")
        .form(&params)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;

    if let Some(e) = json.get("error") {
        return Err(format!("Token exchange error: {} - {}", e, json.get("error_description").unwrap_or(&serde_json::Value::Null)));
    }

    Ok(TokenData {
        access_token: json["access_token"].as_str().ok_or("access_token missing")?.to_string(),
        refresh_token: json["refresh_token"].as_str().map(String::from),
        expires_at: unix_now() + json["expires_in"].as_u64().unwrap_or(3600),
    })
}

// -----------------------------------------------------------------------
// ユーザー情報取得
// -----------------------------------------------------------------------

async fn fetch_user_info(access_token: &str) -> Result<UserInfo, String> {
    let client = reqwest::Client::new();
    let json: serde_json::Value = client
        .get("https://www.googleapis.com/oauth2/v2/userinfo")
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;

    Ok(UserInfo {
        name: json["name"].as_str().unwrap_or("").to_string(),
        email: json["email"].as_str().unwrap_or("").to_string(),
        picture: json["picture"].as_str().map(String::from),
    })
}

// -----------------------------------------------------------------------
// コマンド: 現在のユーザー情報
// -----------------------------------------------------------------------

#[tauri::command]
pub async fn get_auth_user(
    state: tauri::State<'_, AuthStateMutex>,
) -> Result<Option<UserInfo>, String> {
    let auth = state.lock().map_err(|e| e.to_string())?;
    Ok(auth.user_info.clone())
}

// -----------------------------------------------------------------------
// コマンド: アクセストークン取得 (期限切れなら自動リフレッシュ)
// -----------------------------------------------------------------------

#[tauri::command]
pub async fn get_access_token(
    app: AppHandle,
    auth_state: tauri::State<'_, AuthStateMutex>,
    config_state: tauri::State<'_, OAuthConfigMutex>,
) -> Result<Option<String>, String> {
    let refresh_token = {
        let auth = auth_state.lock().map_err(|e| e.to_string())?;
        match &auth.token {
            None => return Ok(None),
            Some(t) if t.expires_at > unix_now() + 60 => return Ok(Some(t.access_token.clone())),
            Some(t) => t.refresh_token.clone(),
        }
    };

    let (client_id, client_secret) = {
        let cfg = config_state.lock().map_err(|e| e.to_string())?;
        (cfg.client_id.clone(), cfg.client_secret.clone())
    };

    if let Some(rt) = refresh_token {
        match refresh_token_internal(&rt, &client_id, &client_secret).await {
            Ok(new_token) => {
                let access = new_token.access_token.clone();
                let mut auth = auth_state.lock().map_err(|e| e.to_string())?;
                if let Some(ref ui) = auth.user_info.clone() {
                    save_auth_state(&app, &new_token, ui);
                }
                auth.token = Some(new_token);
                Ok(Some(access))
            }
            Err(_) => {
                let mut auth = auth_state.lock().map_err(|e| e.to_string())?;
                auth.token = None;
                auth.user_info = None;
                drop(auth);
                clear_auth_state(&app);
                let _ = app.emit("auth-expired", ());
                Ok(None)
            }
        }
    } else {
        Ok(None)
    }
}

// -----------------------------------------------------------------------
// トークンリフレッシュ (内部)
// -----------------------------------------------------------------------

async fn refresh_token_internal(
    refresh_token: &str,
    client_id: &str,
    client_secret: &str,
) -> Result<TokenData, String> {
    let client = reqwest::Client::new();
    let params = [
        ("client_id", client_id),
        ("client_secret", client_secret),
        ("refresh_token", refresh_token),
        ("grant_type", "refresh_token"),
    ];

    let json: serde_json::Value = client
        .post("https://oauth2.googleapis.com/token")
        .form(&params)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;

    if let Some(e) = json.get("error") {
        return Err(format!("Refresh error: {}", e));
    }

    Ok(TokenData {
        access_token: json["access_token"].as_str().ok_or("access_token missing")?.to_string(),
        refresh_token: Some(refresh_token.to_string()),
        expires_at: unix_now() + json["expires_in"].as_u64().unwrap_or(3600),
    })
}

// -----------------------------------------------------------------------
// コマンド: ログアウト
// -----------------------------------------------------------------------

#[tauri::command]
pub async fn sign_out(
    app: AppHandle,
    state: tauri::State<'_, AuthStateMutex>,
) -> Result<(), String> {
    let mut auth = state.lock().map_err(|e| e.to_string())?;
    auth.token = None;
    auth.user_info = None;
    drop(auth);
    clear_auth_state(&app);
    Ok(())
}

// -----------------------------------------------------------------------
// ユーティリティ
// -----------------------------------------------------------------------

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}
