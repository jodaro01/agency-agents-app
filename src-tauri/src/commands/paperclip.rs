//! Native, read-only Paperclip roster bridge.
//!
//! Authentication uses Paperclip's browser-approved CLI challenge. The board
//! token is held only in the native Keychain; pending challenge secrets stay
//! in AppState memory. API responses are decoded into narrow DTOs so adapter
//! configuration and environment secrets are discarded during deserialization.

use std::time::Duration;

use keyring::Entry;
use reqwest::{redirect::Policy, Client, RequestBuilder};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::json;
use tauri::State;
use url::Url;
use uuid::Uuid;

use crate::{
    error::AppError,
    state::{AppState, PendingPaperclipAuth},
    util::fs::{atomic_write, read_capped},
};

const KEYCHAIN_SERVICE: &str = "com.zerologic.agency-agents-app";
const KEYCHAIN_ACCOUNT: &str = "paperclip_board_api_key";
const CONFIG_NAME: &str = "paperclip-connection.json";
const MAX_CONFIG_BYTES: u64 = 16 * 1024;
const MAX_RESPONSE_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PaperclipConfig {
    api_base_url: String,
    company_id: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaperclipStatus {
    configured: bool,
    api_base_url: Option<String>,
    company_id: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaperclipLoginStart {
    approval_url: String,
    expires_at: String,
    poll_interval_ms: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaperclipAuthPoll {
    status: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaperclipAgentSummary {
    id: String,
    name: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    adapter_type: Option<String>,
    #[serde(default)]
    status: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaperclipProjectSummary {
    id: String,
    name: String,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    url_key: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CliAuthChallenge {
    id: String,
    token: String,
    board_api_token: String,
    approval_path: String,
    expires_at: String,
    suggested_poll_interval_ms: Option<u64>,
}

#[derive(Deserialize)]
struct CliAuthChallengeStatus {
    status: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CliAuthMe {
    #[serde(default)]
    company_ids: Vec<String>,
}

fn invalid(message: &str) -> AppError {
    AppError::InvalidArgument {
        message: message.to_string(),
    }
}

fn keychain_entry() -> Result<Entry, AppError> {
    Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT).map_err(|_| {
        AppError::KeychainUnavailable {
            message: "Paperclip Keychain entry is unavailable.".into(),
        }
    })
}

fn read_keychain_token() -> Result<Option<String>, AppError> {
    match keychain_entry()?.get_password() {
        Ok(token) => Ok(Some(token)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(_) => Err(AppError::KeychainUnavailable {
            message: "Paperclip credential could not be read from Keychain.".into(),
        }),
    }
}

fn write_keychain_token(token: &str) -> Result<(), AppError> {
    keychain_entry()?.set_password(token).map_err(|_| {
        AppError::KeychainUnavailable {
            message: "Paperclip credential could not be saved to Keychain.".into(),
        }
    })
}

fn delete_keychain_token() -> Result<(), AppError> {
    match keychain_entry()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(_) => Err(AppError::KeychainUnavailable {
            message: "Paperclip credential could not be removed from Keychain.".into(),
        }),
    }
}

fn normalize_api_base(input: &str) -> Result<String, AppError> {
    let mut url = Url::parse(input.trim()).map_err(|_| {
        invalid("Enter a valid HTTPS Paperclip server URL.")
    })?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || (url.path() != "/" && !url.path().is_empty())
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid(
            "Paperclip must use an HTTPS origin without credentials, path, query, or fragment.",
        ));
    }
    url.set_path("");
    Ok(url.as_str().trim_end_matches('/').to_string())
}

fn normalize_company_id(input: &str) -> Result<String, AppError> {
    Uuid::parse_str(input.trim())
        .map(|id| id.hyphenated().to_string())
        .map_err(|_| invalid("Enter the Paperclip company UUID."))
}

fn build_client() -> Result<Client, AppError> {
    Client::builder()
        .https_only(true)
        .redirect(Policy::none())
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|_| AppError::Internal {
            message: "Paperclip HTTP client could not be initialized.".into(),
        })
}

fn endpoint(base: &str, path: &str) -> Result<Url, AppError> {
    Url::parse(base)
        .and_then(|url| url.join(path))
        .map_err(|_| invalid("Paperclip endpoint URL is invalid."))
}

async fn decode_json<T: DeserializeOwned>(
    request: RequestBuilder,
    label: &'static str,
) -> Result<T, AppError> {
    let response = request
        .header(reqwest::header::ACCEPT, "application/json")
        .header(reqwest::header::ACCEPT_ENCODING, "identity")
        .send()
        .await
        .map_err(|_| AppError::Network {
            url: label.into(),
            message: "request failed".into(),
        })?;
    let status = response.status();
    if !status.is_success() {
        return Err(AppError::HttpStatus {
            url: label.into(),
            status: status.as_u16(),
        });
    }
    let content_length = response.content_length().ok_or_else(|| {
        invalid("Paperclip response did not include a bounded content length.")
    })?;
    if content_length > MAX_RESPONSE_BYTES {
        return Err(invalid("Paperclip response exceeded the 2 MiB safety limit."));
    }
    let body = response.bytes().await.map_err(|_| AppError::Network {
        url: label.into(),
        message: "response body could not be read".into(),
    })?;
    if body.len() as u64 > MAX_RESPONSE_BYTES {
        return Err(invalid("Paperclip response exceeded the 2 MiB safety limit."));
    }
    serde_json::from_slice(&body).map_err(|_| AppError::Internal {
        message: "Paperclip returned an invalid JSON response.".into(),
    })
}

async fn send_empty(request: RequestBuilder, label: &'static str) -> Result<(), AppError> {
    let response = request
        .header(reqwest::header::ACCEPT_ENCODING, "identity")
        .send()
        .await
        .map_err(|_| AppError::Network {
            url: label.into(),
            message: "request failed".into(),
        })?;
    let status = response.status();
    if !status.is_success() && status != reqwest::StatusCode::UNAUTHORIZED && status != reqwest::StatusCode::NOT_FOUND {
        return Err(AppError::HttpStatus {
            url: label.into(),
            status: status.as_u16(),
        });
    }
    Ok(())
}

async fn revoke_board_token(client: &Client, base: &str, token: &str) -> Result<(), AppError> {
    let revoke_url = endpoint(base, "/api/cli-auth/revoke-current")?;
    send_empty(
        client.post(revoke_url).bearer_auth(token),
        "Paperclip credential revocation",
    )
    .await
}

async fn read_config(app_data_dir: &std::path::Path) -> Result<Option<PaperclipConfig>, AppError> {
    let path = app_data_dir.join(CONFIG_NAME);
    let exists = tokio::fs::try_exists(&path).await.map_err(|_| AppError::Io {
        message: "could not inspect Paperclip connection settings".into(),
    })?;
    if !exists {
        return Ok(None);
    }
    let bytes = read_capped(&path, MAX_CONFIG_BYTES).await?;
    let config: PaperclipConfig = serde_json::from_slice(&bytes).map_err(|_| AppError::Internal {
        message: "Saved Paperclip connection is invalid; reconnect to repair it.".into(),
    })?;
    let normalized = PaperclipConfig {
        api_base_url: normalize_api_base(&config.api_base_url)?,
        company_id: normalize_company_id(&config.company_id)?,
    };
    Ok(Some(normalized))
}

async fn write_config(
    app_data_dir: &std::path::Path,
    config: &PaperclipConfig,
) -> Result<(), AppError> {
    let bytes = serde_json::to_vec(config).map_err(|_| AppError::Internal {
        message: "Paperclip connection settings could not be encoded.".into(),
    })?;
    if bytes.len() as u64 > MAX_CONFIG_BYTES {
        return Err(invalid("Paperclip connection settings exceed the size limit."));
    }
    atomic_write(&app_data_dir.join(CONFIG_NAME), &bytes).await
}

async fn restore_config(
    app_data_dir: &std::path::Path,
    old: Option<&PaperclipConfig>,
) {
    match old {
        Some(config) => {
            let _ = write_config(app_data_dir, config).await;
        }
        None => {
            let _ = tokio::fs::remove_file(app_data_dir.join(CONFIG_NAME)).await;
        }
    }
}

async fn remove_config(app_data_dir: &std::path::Path) -> Result<(), AppError> {
    match tokio::fs::remove_file(app_data_dir.join(CONFIG_NAME)).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(AppError::Io {
            message: "Paperclip connection settings could not be removed.".into(),
        }),
    }
}

async fn stored_connection(
    state: &AppState,
    feature: &'static str,
) -> Result<(PaperclipConfig, String), AppError> {
    state.require_network(feature).await?;
    let config = read_config(&state.app_data_dir)
        .await?
        .ok_or_else(|| invalid("Connect to Paperclip first."))?;
    let token = read_keychain_token()?.ok_or_else(|| invalid("Paperclip is not connected."))?;
    Ok((config, token))
}

#[tauri::command]
pub async fn paperclip_status(state: State<'_, AppState>) -> Result<PaperclipStatus, AppError> {
    let config = read_config(&state.app_data_dir).await?;
    let has_token = read_keychain_token()?.is_some();
    Ok(PaperclipStatus {
        configured: config.is_some() && has_token,
        api_base_url: config.as_ref().map(|value| value.api_base_url.clone()),
        company_id: config.map(|value| value.company_id),
    })
}

#[tauri::command]
pub async fn paperclip_login_start(
    api_base_url: String,
    company_id: String,
    state: State<'_, AppState>,
) -> Result<PaperclipLoginStart, AppError> {
    state.require_network("paperclip_login").await?;
    let api_base_url = normalize_api_base(&api_base_url)?;
    let company_id = normalize_company_id(&company_id)?;

    let mut pending = state.paperclip_pending_auth.lock().await;
    if pending.is_some() {
        return Err(invalid("A Paperclip sign-in is already in progress."));
    }

    let client = build_client()?;
    let challenge_url = endpoint(&api_base_url, "/api/cli-auth/challenges")?;
    let challenge: CliAuthChallenge = decode_json(
        client
            .post(challenge_url)
            .json(&json!({
                "command": "Agency Agents App",
                "clientName": "Agency Agents App",
                "requestedAccess": "board",
                "requestedCompanyId": company_id,
            })),
        "Paperclip sign-in",
    )
    .await?;

    if challenge.id.is_empty()
        || challenge.id.len() > 128
        || !challenge
            .id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
        || !challenge.board_api_token.starts_with("pcp_board_")
        || challenge.token.is_empty()
        || challenge.token.len() > 512
    {
        return Err(invalid("Paperclip returned an invalid sign-in challenge."));
    }

    let expected_path = format!("/cli-auth/{}", challenge.id);
    let approval = Url::parse(&format!("https://paperclip.invalid{}", challenge.approval_path))
        .map_err(|_| invalid("Paperclip returned an invalid approval link."))?;
    let approval_token_matches = approval
        .query_pairs()
        .any(|(key, value)| key == "token" && value == challenge.token);
    if approval.path() != expected_path || !approval_token_matches {
        return Err(invalid("Paperclip returned an invalid approval link."));
    }
    let approval_url = endpoint(&api_base_url, &challenge.approval_path)?.to_string();
    let poll_interval_ms = challenge
        .suggested_poll_interval_ms
        .unwrap_or(1000)
        .clamp(500, 5000);

    *pending = Some(PendingPaperclipAuth {
        api_base_url,
        company_id,
        challenge_id: challenge.id,
        challenge_token: challenge.token,
        board_api_token: challenge.board_api_token,
        });

    Ok(PaperclipLoginStart {
        approval_url,
        expires_at: challenge.expires_at,
        poll_interval_ms,
    })
}

#[tauri::command]
pub async fn paperclip_login_poll(
    state: State<'_, AppState>,
) -> Result<PaperclipAuthPoll, AppError> {
    if let Err(error) = state.require_network("paperclip_login").await {
        *state.paperclip_pending_auth.lock().await = None;
        return Err(error);
    }
    let mut pending_guard = state.paperclip_pending_auth.lock().await;
    let pending = pending_guard
        .as_ref()
        .ok_or_else(|| invalid("No Paperclip sign-in is in progress."))?;
    let client = build_client()?;
    let poll_url = endpoint(
        &pending.api_base_url,
        &format!("/api/cli-auth/challenges/{}", pending.challenge_id),
    )?;
    let result: CliAuthChallengeStatus = decode_json(
        client
            .get(poll_url)
            .query(&[("token", pending.challenge_token.as_str())]),
        "Paperclip sign-in status",
    )
    .await?;

    match result.status.as_str() {
        "pending" => Ok(PaperclipAuthPoll {
            status: "pending".into(),
        }),
        "cancelled" | "expired" => {
            let status = result.status;
            *pending_guard = None;
            Ok(PaperclipAuthPoll { status })
        }
        "approved" => {
            let me_url = endpoint(&pending.api_base_url, "/api/cli-auth/me")?;
            let me: CliAuthMe = decode_json(
                client.get(me_url).bearer_auth(&pending.board_api_token),
                "Paperclip identity check",
            )
            .await?;
            if !me.company_ids.iter().any(|id| id == &pending.company_id) {
                let _ = revoke_board_token(&client, &pending.api_base_url, &pending.board_api_token).await;
                *pending_guard = None;
                return Err(invalid(
                    "The approved Paperclip account cannot access the selected company.",
                ));
            }

            let config = PaperclipConfig {
                api_base_url: pending.api_base_url.clone(),
                company_id: pending.company_id.clone(),
            };
            let previous = match read_config(&state.app_data_dir).await {
                Ok(previous) => previous,
                Err(error) => {
                    let _ = revoke_board_token(&client, &pending.api_base_url, &pending.board_api_token).await;
                    *pending_guard = None;
                    return Err(error);
                }
            };
            if let Err(error) = write_config(&state.app_data_dir, &config).await {
                let revoked = revoke_board_token(&client, &pending.api_base_url, &pending.board_api_token).await.is_ok();
                *pending_guard = None;
                if revoked {
                    return Err(error);
                }
                return Err(AppError::Internal {
                    message: "Could not save the Paperclip connection or confirm token revocation; revoke the pending Paperclip key in Paperclip.".into(),
                });
            }
            if let Err(error) = write_keychain_token(&pending.board_api_token) {
                restore_config(&state.app_data_dir, previous.as_ref()).await;
                let revoked = revoke_board_token(&client, &pending.api_base_url, &pending.board_api_token).await.is_ok();
                *pending_guard = None;
                if revoked {
                    return Err(error);
                }
                return Err(AppError::Internal {
                    message: "Could not store the Paperclip credential or confirm token revocation; revoke the pending Paperclip key in Paperclip.".into(),
                });
            }
            *pending_guard = None;
            Ok(PaperclipAuthPoll {
                status: "approved".into(),
            })
        }
        _ => Err(invalid("Paperclip returned an unknown sign-in status.")),
    }
}

#[tauri::command]
pub async fn paperclip_login_cancel(state: State<'_, AppState>) -> Result<(), AppError> {
    state.require_network("paperclip_login").await?;
    let mut pending_guard = state.paperclip_pending_auth.lock().await;
    let Some(pending) = pending_guard.as_ref() else {
        return Ok(());
    };
    let client = build_client()?;
    let cancel_url = endpoint(
        &pending.api_base_url,
        &format!(
            "/api/cli-auth/challenges/{}/cancel",
            pending.challenge_id
        ),
    )?;
    send_empty(
        client
            .post(cancel_url)
            .json(&json!({ "token": pending.challenge_token })),
        "Paperclip sign-in cancel",
    )
    .await?;
    *pending_guard = None;
    Ok(())
}

#[tauri::command]
pub async fn paperclip_agents_list(
    state: State<'_, AppState>,
) -> Result<Vec<PaperclipAgentSummary>, AppError> {
    let (config, token) = stored_connection(&state, "paperclip_agents").await?;
    let client = build_client()?;
    let url = endpoint(
        &config.api_base_url,
        &format!("/api/companies/{}/agents", config.company_id),
    )?;
    let mut agents: Vec<PaperclipAgentSummary> = decode_json(
        client.get(url).bearer_auth(token),
        "Paperclip agents",
    )
    .await?;
    agents.truncate(500);
    Ok(agents)
}

#[tauri::command]
pub async fn paperclip_projects_list(
    state: State<'_, AppState>,
) -> Result<Vec<PaperclipProjectSummary>, AppError> {
    let (config, token) = stored_connection(&state, "paperclip_projects").await?;
    let client = build_client()?;
    let url = endpoint(
        &config.api_base_url,
        &format!("/api/companies/{}/projects", config.company_id),
    )?;
    let mut projects: Vec<PaperclipProjectSummary> = decode_json(
        client.get(url).bearer_auth(token),
        "Paperclip projects",
    )
    .await?;
    projects.truncate(500);
    Ok(projects)
}

#[tauri::command]
pub async fn paperclip_disconnect(state: State<'_, AppState>) -> Result<(), AppError> {
    let config = read_config(&state.app_data_dir).await?;
    let token = read_keychain_token()?;
    if let (Some(config), Some(token)) = (config, token) {
        state.require_network("paperclip_disconnect").await?;
        let client = build_client()?;
        revoke_board_token(&client, &config.api_base_url, &token).await?;
    }

    delete_keychain_token()?;
    remove_config(&state.app_data_dir).await?;
    *state.paperclip_pending_auth.lock().await = None;
    Ok(())
}
