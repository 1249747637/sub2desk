use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, CONTENT_TYPE, LOCATION};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::BTreeMap, fs, path::PathBuf};
use uuid::Uuid;

const APP_DIR: &str = "Sub2Desk";
const STATE_FILE: &str = "state.json";
const KEYRING_SERVICE: &str = "Sub2Desk";
const KEYRING_USER: &str = "admin_api_key";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppState {
    profiles: Vec<Profile>,
    active_profile_id: String,
    settings: Settings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedAppState {
    profiles: Vec<Profile>,
    active_profile_id: String,
    settings: PersistedSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Settings {
    backend_base_url: String,
    admin_api_key: String,
    theme: Theme,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedSettings {
    backend_base_url: String,
    theme: Theme,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Profile {
    id: String,
    name: String,
    #[serde(default)]
    account_name: String,
    platform: Platform,
    pool_mode: bool,
    pool_mode_retry_count: u32,
    priority: i32,
    group_id: Option<u64>,
    base_url: String,
    api_key: String,
    models: Vec<ModelItem>,
    last_fetched_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModelItem {
    id: String,
    enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GroupOption {
    id: u64,
    name: String,
    platform: Option<String>,
    status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TestResult {
    model_id: String,
    message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubmitResult {
    account_id: Option<Value>,
    message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Platform {
    Openai,
    Anthropic,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Theme {
    Light,
    Dark,
}

#[tauri::command]
fn load_state() -> Result<AppState, String> {
    let path = state_path()?;
    let mut state = if path.exists() {
        let raw = fs::read_to_string(&path)
            .map_err(|err| format!("读取本地状态失败: {err}"))?;
        let persisted: PersistedAppState = serde_json::from_str(&raw)
            .map_err(|err| format!("解析本地状态失败: {err}"))?;
        AppState {
            profiles: persisted.profiles,
            active_profile_id: persisted.active_profile_id,
            settings: Settings {
                backend_base_url: persisted.settings.backend_base_url,
                admin_api_key: load_admin_api_key(),
                theme: persisted.settings.theme,
            },
        }
    } else {
        default_state()
    };

    normalize_state(&mut state);
    Ok(state)
}

#[tauri::command]
fn save_state(mut state: AppState) -> Result<AppState, String> {
    normalize_state(&mut state);
    save_admin_api_key(&state.settings.admin_api_key)?;

    let persisted = PersistedAppState {
        profiles: state.profiles.clone(),
        active_profile_id: state.active_profile_id.clone(),
        settings: PersistedSettings {
            backend_base_url: state.settings.backend_base_url.clone(),
            theme: state.settings.theme.clone(),
        },
    };

    let path = state_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| format!("创建配置目录失败: {err}"))?;
    }
    let raw = serde_json::to_string_pretty(&persisted)
        .map_err(|err| format!("序列化本地状态失败: {err}"))?;
    fs::write(path, raw).map_err(|err| format!("保存本地状态失败: {err}"))?;

    Ok(state)
}

#[tauri::command]
async fn fetch_groups(settings: Settings, platform: Platform) -> Result<Vec<GroupOption>, String> {
    let origin = normalize_origin(&settings.backend_base_url)?;
    if settings.admin_api_key.trim().is_empty() {
        return Err("请先在设置里填写管理员 API Key".into());
    }

    let platform_value = platform_name(&platform);
    let url = format!("{origin}/api/v1/admin/groups/all?platform={platform_value}");
    let client = reqwest::Client::new();
    let response = client
        .get(url)
        .header("x-api-key", settings.admin_api_key.trim())
        .send()
        .await
        .map_err(|err| format!("获取分组失败: {err}"))?;
    let status = response.status();
    let body: Value = response
        .json()
        .await
        .map_err(|err| format!("解析分组响应失败: {err}"))?;
    ensure_success(status.as_u16(), &body)?;

    let data = body
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| "分组响应缺少 data 数组".to_string())?;

    let groups = data
        .iter()
        .filter_map(|item| {
            let id = item.get("id").and_then(Value::as_u64)?;
            let name = item
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("未命名分组")
                .to_string();
            Some(GroupOption {
                id,
                name,
                platform: item
                    .get("platform")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
                status: item
                    .get("status")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
            })
        })
        .collect();

    Ok(groups)
}

#[tauri::command]
async fn fetch_models(profile: Profile) -> Result<Vec<ModelItem>, String> {
    validate_provider_profile(&profile)?;
    let url = build_models_url(&profile.base_url);
    let client = reqwest::Client::new();
    let response = client
        .get(url)
        .headers(provider_headers(&profile)?)
        .send()
        .await
        .map_err(|err| format!("获取模型列表失败: {err}"))?;
    let status = response.status();
    let body: Value = response
        .json()
        .await
        .map_err(|err| format!("解析模型列表失败: {err}"))?;
    if !status.is_success() {
        return Err(extract_error_message(status.as_u16(), &body));
    }

    let mut models = Vec::new();
    if let Some(data) = body.get("data").and_then(Value::as_array) {
        for item in data {
            if let Some(id) = item.get("id").and_then(Value::as_str) {
                models.push(ModelItem {
                    id: id.to_string(),
                    enabled: true,
                });
            }
        }
    }

    if models.is_empty() {
        return Err("模型列表为空，或响应中没有 data[].id".into());
    }

    models.sort_by(|a, b| a.id.cmp(&b.id));
    models.dedup_by(|a, b| a.id == b.id);
    Ok(models)
}

#[tauri::command]
async fn test_model(profile: Profile, model_id: String) -> Result<TestResult, String> {
    validate_provider_profile(&profile)?;
    if model_id.trim().is_empty() {
        return Err("模型 ID 不能为空".into());
    }

    let client = reqwest::Client::new();
    let response = match profile.platform {
        Platform::Openai => {
            client
                .post(build_v1_endpoint(&profile.base_url, "chat/completions"))
                .headers(provider_headers(&profile)?)
                .json(&json!({
                    "model": model_id,
                    "messages": [{ "role": "user", "content": "你好" }],
                    "max_tokens": 256
                }))
                .send()
                .await
        }
        Platform::Anthropic => {
            client
                .post(build_v1_endpoint(&profile.base_url, "messages"))
                .headers(provider_headers(&profile)?)
                .json(&json!({
                    "model": model_id,
                    "max_tokens": 256,
                    "messages": [{ "role": "user", "content": "你好" }]
                }))
                .send()
                .await
        }
    }
    .map_err(|err| format!("测试请求失败: {err}"))?;

    let status = response.status();
    let body: Value = response
        .json()
        .await
        .map_err(|err| format!("解析测试响应失败: {err}"))?;
    if !status.is_success() {
        return Err(extract_error_message(status.as_u16(), &body));
    }

    let message = extract_test_message(&profile.platform, &body)
        .unwrap_or_else(|| "测试成功，但响应里没有可展示文本".to_string());
    Ok(TestResult { model_id, message })
}

#[tauri::command]
async fn submit_profile(settings: Settings, profile: Profile) -> Result<SubmitResult, String> {
    let origin = normalize_origin(&settings.backend_base_url)?;
    if settings.admin_api_key.trim().is_empty() {
        return Err("请先在设置里填写管理员 API Key".into());
    }
    validate_provider_profile(&profile)?;
    let group_id = profile.group_id.ok_or_else(|| "请选择分组".to_string())?;
    let enabled_models: Vec<&ModelItem> = profile.models.iter().filter(|model| model.enabled).collect();
    if enabled_models.is_empty() {
        return Err("至少选择一个模型".into());
    }
    let account_name = normalized_account_name(&profile);

    let model_mapping: BTreeMap<String, String> = enabled_models
        .into_iter()
        .map(|model| (model.id.clone(), model.id.clone()))
        .collect();

    let body = json!({
        "name": account_name,
        "platform": platform_name(&profile.platform),
        "type": "apikey",
        "credentials": {
            "api_key": profile.api_key.trim(),
            "base_url": profile.base_url.trim(),
            "pool_mode": profile.pool_mode,
            "pool_mode_retry_count": profile.pool_mode_retry_count,
            "model_mapping": model_mapping
        },
        "extra": {},
        "group_ids": [group_id],
        "concurrency": 1,
        "priority": profile.priority.max(1),
        "confirm_mixed_channel_risk": true
    });

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|err| format!("创建 HTTP 客户端失败: {err}"))?;
    let create_url = format!("{origin}/api/v1/admin/accounts");
    let idempotency_key = Uuid::new_v4().to_string();
    let response = post_admin_json(
        &client,
        &create_url,
        settings.admin_api_key.trim(),
        &idempotency_key,
        &body,
    )
    .await?;
    let status = response.status();
    let response_body: Value = response
        .json()
        .await
        .map_err(|err| format!("解析 sub2api 响应失败: {err}"))?;
    ensure_standard_success(status.as_u16(), &response_body)?;

    let account_id = if let Some(account_id) = extract_account_id(&response_body, &account_name) {
        account_id
    } else if response_looks_like_account_list(&response_body) {
        match lookup_account_id(&client, &origin, settings.admin_api_key.trim(), &account_name).await? {
            Some(account_id) => account_id,
            None => {
                return Err(format!(
                    "创建请求没有返回账号对象，且回查不到账号。请求地址: {create_url}；后端返回像账号列表响应，可能被反代或后端入口错误处理。响应: {}",
                    compact_json(&response_body)
                ));
            }
        }
    } else {
        return Err(format!(
            "sub2api 未返回账号 ID。请求地址: {create_url}；响应: {}",
            compact_json(&response_body)
        ));
    };

    Ok(SubmitResult {
        account_id: Some(account_id.clone()),
        message: format!("账号已提交到 sub2api，ID: {}", display_json_value(&account_id)),
    })
}

async fn post_admin_json(
    client: &reqwest::Client,
    url: &str,
    admin_api_key: &str,
    idempotency_key: &str,
    body: &Value,
) -> Result<reqwest::Response, String> {
    let response = client
        .post(url)
        .header("x-api-key", admin_api_key)
        .header("Idempotency-Key", idempotency_key)
        .header(CONTENT_TYPE, "application/json")
        .json(body)
        .send()
        .await
        .map_err(|err| format!("提交到 sub2api 失败: {err}"))?;

    if !response.status().is_redirection() {
        return Ok(response);
    }

    let status = response.status();
    let redirected_url = response
        .headers()
        .get(LOCATION)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| format!("提交到 sub2api 被重定向 HTTP {status}，但响应缺少 Location"))?;
    let redirected_url = resolve_redirect_url(url, redirected_url)?;

    client
        .post(&redirected_url)
        .header("x-api-key", admin_api_key)
        .header("Idempotency-Key", idempotency_key)
        .header(CONTENT_TYPE, "application/json")
        .json(body)
        .send()
        .await
        .map_err(|err| format!("提交到重定向后的 sub2api 地址失败: {err}"))
}

async fn lookup_account_id(
    client: &reqwest::Client,
    origin: &str,
    admin_api_key: &str,
    account_name: &str,
) -> Result<Option<Value>, String> {
    let response = client
        .get(format!("{origin}/api/v1/admin/accounts"))
        .query(&[
            ("search", account_name),
            ("page", "1"),
            ("page_size", "20"),
            ("sort_by", "name"),
            ("sort_order", "asc"),
        ])
        .header("x-api-key", admin_api_key)
        .send()
        .await
        .map_err(|err| format!("提交后回查账号失败: {err}"))?;
    let status = response.status();
    let body: Value = response
        .json()
        .await
        .map_err(|err| format!("解析账号回查响应失败: {err}"))?;
    ensure_standard_success(status.as_u16(), &body)?;
    Ok(extract_account_id(&body, account_name))
}

fn default_state() -> AppState {
    let profiles: Vec<Profile> = (1..=3)
        .map(|idx| default_profile(format!("Profile {idx}")))
        .collect();
    let active_profile_id = profiles
        .first()
        .map(|profile| profile.id.clone())
        .unwrap_or_default();

    AppState {
        profiles,
        active_profile_id,
        settings: Settings {
            backend_base_url: "http://localhost:8080".into(),
            admin_api_key: load_admin_api_key(),
            theme: Theme::Light,
        },
    }
}

fn default_profile(name: String) -> Profile {
    Profile {
        id: Uuid::new_v4().to_string(),
        name,
        account_name: String::new(),
        platform: Platform::Openai,
        pool_mode: false,
        pool_mode_retry_count: 3,
        priority: 1,
        group_id: None,
        base_url: "https://api.openai.com".into(),
        api_key: String::new(),
        models: Vec::new(),
        last_fetched_at: None,
    }
}

fn normalize_state(state: &mut AppState) {
    if state.profiles.is_empty() {
        state.profiles = (1..=3)
            .map(|idx| default_profile(format!("Profile {idx}")))
            .collect();
    }

    for (idx, profile) in state.profiles.iter_mut().enumerate() {
        if profile.id.trim().is_empty() {
            profile.id = Uuid::new_v4().to_string();
        }
        if profile.name.trim().is_empty() {
            profile.name = format!("Profile {}", idx + 1);
        }
        profile.account_name = profile.account_name.trim().to_string();
        if profile.priority < 1 {
            profile.priority = 1;
        }
        if profile.pool_mode_retry_count == 0 {
            profile.pool_mode_retry_count = 3;
        }
    }

    if !state
        .profiles
        .iter()
        .any(|profile| profile.id == state.active_profile_id)
    {
        state.active_profile_id = state
            .profiles
            .first()
            .map(|profile| profile.id.clone())
            .unwrap_or_default();
    }
}

fn state_path() -> Result<PathBuf, String> {
    let base = dirs::config_dir().ok_or_else(|| "无法定位系统配置目录".to_string())?;
    Ok(base.join(APP_DIR).join(STATE_FILE))
}

fn load_admin_api_key() -> String {
    keyring_entry()
        .and_then(|entry| entry.get_password())
        .unwrap_or_default()
}

fn save_admin_api_key(value: &str) -> Result<(), String> {
    let entry = keyring_entry().map_err(|err| format!("打开系统凭据管理失败: {err}"))?;
    if value.trim().is_empty() {
        let _ = entry.delete_credential();
        return Ok(());
    }
    entry
        .set_password(value.trim())
        .map_err(|err| format!("保存管理员 API Key 到系统凭据失败: {err}"))
}

fn keyring_entry() -> Result<keyring::Entry, keyring::Error> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
}

fn validate_provider_profile(profile: &Profile) -> Result<(), String> {
    if profile.base_url.trim().is_empty() {
        return Err("请填写 Base URL".into());
    }
    if profile.api_key.trim().is_empty() {
        return Err("请填写上游 API Key".into());
    }
    Ok(())
}

fn provider_headers(profile: &Profile) -> Result<HeaderMap, String> {
    let mut headers = HeaderMap::new();
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

    match profile.platform {
        Platform::Openai => {
            let value = HeaderValue::from_str(&format!("Bearer {}", profile.api_key.trim()))
                .map_err(|err| format!("API Key 不是合法请求头: {err}"))?;
            headers.insert(AUTHORIZATION, value);
        }
        Platform::Anthropic => {
            let value = HeaderValue::from_str(profile.api_key.trim())
                .map_err(|err| format!("API Key 不是合法请求头: {err}"))?;
            headers.insert("x-api-key", value);
            headers.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
        }
    }

    Ok(headers)
}

fn build_models_url(base_url: &str) -> String {
    let base = base_url.trim().trim_end_matches('/');
    if base.ends_with("/v1/models") {
        base.to_string()
    } else if base.ends_with("/v1") {
        format!("{base}/models")
    } else {
        format!("{base}/v1/models")
    }
}

fn build_v1_endpoint(base_url: &str, endpoint: &str) -> String {
    let base = base_url.trim().trim_end_matches('/');
    let suffix = format!("/v1/{endpoint}");
    if base.ends_with(&suffix) {
        base.to_string()
    } else if base.ends_with("/v1") {
        format!("{base}/{endpoint}")
    } else {
        format!("{base}/v1/{endpoint}")
    }
}

fn normalize_origin(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err("请填写后端管理地址".into());
    }

    let without_api = trimmed
        .strip_suffix("/api/v1")
        .or_else(|| trimmed.strip_suffix("/api"))
        .unwrap_or(trimmed);
    let parsed = reqwest::Url::parse(without_api)
        .map_err(|err| format!("后端管理地址不是合法 URL: {err}"))?;
    let host = parsed
        .host_str()
        .ok_or_else(|| "后端管理地址缺少 host".to_string())?;
    let mut origin = format!("{}://{}", parsed.scheme(), host);
    if let Some(port) = parsed.port() {
        origin.push_str(&format!(":{port}"));
    }
    Ok(origin)
}

fn resolve_redirect_url(current_url: &str, location: &str) -> Result<String, String> {
    let current = reqwest::Url::parse(current_url)
        .map_err(|err| format!("当前请求 URL 不是合法 URL: {err}"))?;
    current
        .join(location)
        .map(|url| url.to_string())
        .map_err(|err| format!("重定向 Location 不是合法 URL: {err}"))
}

fn platform_name(platform: &Platform) -> &'static str {
    match platform {
        Platform::Openai => "openai",
        Platform::Anthropic => "anthropic",
    }
}

fn normalized_account_name(profile: &Profile) -> String {
    let configured = profile.account_name.trim();
    if !configured.is_empty() {
        return configured.to_string();
    }
    let timestamp = time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| Uuid::new_v4().to_string())
        .replace(':', "-");
    format!("sub2desk-{timestamp}")
}

fn ensure_success(status: u16, body: &Value) -> Result<(), String> {
    if !(200..300).contains(&status) {
        return Err(extract_error_message(status, body));
    }
    if body.get("code").and_then(Value::as_i64).unwrap_or(0) != 0 {
        return Err(extract_error_message(status, body));
    }
    Ok(())
}

fn ensure_standard_success(status: u16, body: &Value) -> Result<(), String> {
    if !(200..300).contains(&status) {
        return Err(extract_error_message(status, body));
    }

    match body.get("code").and_then(Value::as_i64) {
        Some(0) => Ok(()),
        Some(_) => Err(extract_error_message(status, body)),
        None => Err(format!(
            "sub2api 返回的不是标准响应格式，响应: {}",
            compact_json(body)
        )),
    }
}

fn extract_error_message(status: u16, body: &Value) -> String {
    let message = body
        .get("message")
        .or_else(|| body.get("error").and_then(|error| error.get("message")))
        .and_then(Value::as_str)
        .unwrap_or("请求失败");
    format!("HTTP {status}: {message}")
}

fn extract_account_id(body: &Value, account_name: &str) -> Option<Value> {
    let data = body.get("data")?;
    if let Some(id) = data.get("id").cloned().or_else(|| data.get("account_id").cloned()) {
        return Some(id);
    }

    data.get("items")
        .and_then(Value::as_array)?
        .iter()
        .find(|item| item.get("name").and_then(Value::as_str) == Some(account_name))
        .and_then(|item| item.get("id").cloned().or_else(|| item.get("account_id").cloned()))
}

fn response_looks_like_account_list(body: &Value) -> bool {
    body.get("data")
        .and_then(|data| data.get("items"))
        .and_then(Value::as_array)
        .is_some()
}

fn extract_test_message(platform: &Platform, body: &Value) -> Option<String> {
    match platform {
        Platform::Openai => {
            let choice = body.get("choices")?.as_array()?.first()?;
            let message = choice.get("message")?;
            first_non_empty_text(&[
                message.get("content"),
                message.get("reasoning_content"),
                choice.get("text"),
            ])
        }
        Platform::Anthropic => body.get("content")?.as_array()?.iter().find_map(|item| {
            first_non_empty_text(&[item.get("text"), item.get("thinking")])
        }),
    }
}

fn first_non_empty_text(values: &[Option<&Value>]) -> Option<String> {
    values
        .iter()
        .filter_map(|value| value.and_then(Value::as_str))
        .map(str::trim)
        .find(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn compact_json(value: &Value) -> String {
    let raw = serde_json::to_string(value).unwrap_or_else(|_| "<unserializable>".to_string());
    if raw.len() > 600 {
        format!("{}...", &raw[..600])
    } else {
        raw
    }
}

fn display_json_value(value: &Value) -> String {
    value
        .as_i64()
        .map(|id| id.to_string())
        .or_else(|| value.as_u64().map(|id| id.to_string()))
        .or_else(|| value.as_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| compact_json(value))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .on_window_event(|_window, event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                std::process::exit(0);
            }
        })
        .invoke_handler(tauri::generate_handler![
            load_state,
            save_state,
            fetch_groups,
            fetch_models,
            test_model,
            submit_profile
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
