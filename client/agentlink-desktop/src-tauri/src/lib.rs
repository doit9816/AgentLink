use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, WindowEvent};
use tauri_plugin_updater::UpdaterExt;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[derive(Default)]
struct BridgeState {
    child: Mutex<Option<Child>>,
    qr_sessions: Mutex<HashMap<String, Arc<Mutex<QrSetupStatus>>>>,
}

#[derive(Default)]
struct DesktopUpdateState {
    pending: Mutex<Option<tauri_plugin_updater::Update>>,
}

const ACCOUNTS_FEISHU_BASE: &str = "https://accounts.feishu.cn";
const ACCOUNTS_LARK_BASE: &str = "https://accounts.larksuite.com";
const OPEN_FEISHU_BASE: &str = "https://open.feishu.cn";
const OPEN_LARK_BASE: &str = "https://open.larksuite.com";
const DEFAULT_WEIXIN_API_BASE: &str = "https://ilinkai.weixin.qq.com";
const DEFAULT_WEIXIN_BOT_TYPE: &str = "3";
const DESKTOP_UPDATER_ENDPOINT: &str =
    "https://github.com/doit9816/AgentLink/releases/latest/download/latest.json";

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ClientOptions {
    exe_path: String,
    config_path: String,
    project: String,
    work_dir: String,
    platform: String,
    extra: Option<String>,
    /// Legacy field from older desktop builds; Channel UI no longer exposes CLI mode.
    #[allow(dead_code)]
    operation_mode: Option<String>,
    agent_type: Option<String>,
    agent_backend: Option<String>,
    agent_command: Option<String>,
    agent_model: Option<String>,
    agent_mode: Option<String>,
    reasoning_effort: Option<String>,
    agent_args: Option<String>,
    channel_fields: Option<HashMap<String, String>>,
    /// Desktop connection id; used to load Channel fields from SQLite with priority.
    connection_id: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ClientStatus {
    exe_exists: bool,
    config_exists: bool,
    bridge_running: bool,
    bridge_pid: Option<u32>,
    project_configured: bool,
    channel_configured: bool,
    agent_configured: bool,
    binding_ready: bool,
    agent_installed: bool,
    connection_status: String,
    channel_status: String,
    agent_status: String,
    binding_status: String,
    agent_install_status: String,
    summary: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AvailableUpdatePayload {
    version: String,
    date: Option<String>,
    body: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentCheckOptions {
    agent_type: String,
    command: Option<String>,
}

#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PickPathOptions {
    #[allow(dead_code)]
    kind: String,
    #[allow(dead_code)]
    current: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BridgeLogs {
    stdout: String,
    stderr: String,
    combined: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentCheckStatus {
    agent_type: String,
    installed: bool,
    status: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SaveConfigResult {
    message: String,
    config_path: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConfigSnapshot {
    project: String,
    work_dir: String,
    selected_channel: String,
    selected_agent: String,
    channel_fields: HashMap<String, String>,
    agent_fields: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct QrSetupStatus {
    session_id: String,
    platform: String,
    state: String,
    message: String,
    user_code: Option<String>,
    qr_url: Option<String>,
    qr_svg: Option<String>,
    output: Vec<String>,
    done: bool,
    success: Option<bool>,
}

/// Unified binding/validation result for Channel save, guided setup, and status checks.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ChannelBindingStatus {
    platform: String,
    state: String,
    message: String,
    ready: bool,
    missing_fields: Vec<String>,
    setup_url: Option<String>,
    next_step: String,
}

#[derive(Debug, Deserialize)]
struct RegistrationInitResponse {
    #[serde(default)]
    supported_auth_methods: Vec<String>,
    #[serde(default)]
    error: String,
    #[serde(default)]
    error_description: String,
}

#[derive(Debug, Deserialize)]
struct RegistrationBeginResponse {
    #[serde(default)]
    device_code: String,
    #[serde(default)]
    user_code: String,
    #[serde(default)]
    verification_uri_complete: String,
    #[serde(default)]
    interval: u64,
    #[serde(default, alias = "expires_in")]
    expire_in: u64,
    #[serde(default)]
    error: String,
    #[serde(default)]
    error_description: String,
}

#[derive(Debug, Deserialize, Default)]
struct RegistrationUserInfo {
    #[serde(default)]
    open_id: String,
    #[serde(default)]
    tenant_brand: String,
}

#[derive(Debug, Deserialize)]
struct RegistrationPollResponse {
    #[serde(default, alias = "app_id")]
    client_id: String,
    #[serde(default, alias = "app_secret")]
    client_secret: String,
    #[serde(default)]
    user_info: RegistrationUserInfo,
    #[serde(default)]
    error: String,
    #[serde(default)]
    error_description: String,
}

#[derive(Debug, Deserialize)]
struct WeixinQrBeginResponse {
    #[serde(default)]
    qrcode: String,
    #[serde(default)]
    qrcode_img_content: String,
}

#[derive(Debug, Deserialize, Default)]
struct WeixinQrPollResponse {
    #[serde(default)]
    status: String,
    #[serde(default)]
    bot_token: String,
    #[serde(default)]
    ilink_bot_id: String,
    #[serde(default)]
    baseurl: String,
    #[serde(default)]
    ilink_user_id: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ClientConnectionState {
    id: String,
    name: String,
    exe_path: String,
    config_path: String,
    project: String,
    work_dir: String,
    operation_mode: Option<String>,
    selected_channel: String,
    selected_agent: String,
    channel_fields: HashMap<String, HashMap<String, String>>,
    agent_fields: HashMap<String, HashMap<String, String>>,
    #[serde(default)]
    channel_targets: HashMap<String, Vec<serde_json::Value>>,
    #[serde(default)]
    active_target_ids: HashMap<String, String>,
    #[serde(default)]
    test: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdatePreferences {
    #[serde(default = "default_auto_check_updates")]
    auto_check_updates: bool,
    #[serde(default = "default_auto_install_updates")]
    auto_install_updates: bool,
    #[serde(default)]
    last_update_check_at: Option<String>,
    #[serde(default)]
    last_update_error: Option<String>,
}

impl Default for UpdatePreferences {
    fn default() -> Self {
        Self {
            auto_check_updates: true,
            auto_install_updates: true,
            last_update_check_at: None,
            last_update_error: None,
        }
    }
}

fn default_auto_check_updates() -> bool {
    true
}

fn default_auto_install_updates() -> bool {
    true
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ClientDatabaseState {
    active_connection_id: String,
    connections: Vec<ClientConnectionState>,
    #[serde(default)]
    update_preferences: UpdatePreferences,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TestMessageRequest {
    webhook_url: String,
    bearer_token: Option<String>,
    payload: serde_json::Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChannelTargetRequest {
    receive_id_type: Option<String>,
    receive_id: Option<String>,
    session_key: Option<String>,
    user_id: Option<String>,
    user_name: Option<String>,
    session_webhook: Option<String>,
    webhook_url: Option<String>,
    message_id: Option<String>,
    reply_context: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChannelMessageRequest {
    options: ClientOptions,
    target: ChannelTargetRequest,
    content: String,
    message_type: Option<String>,
    payload: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TestMessageResponse {
    status: u16,
    body: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DiscoveredTarget {
    name: String,
    platform: String,
    session_key: String,
    user_id: String,
    user_name: Option<String>,
    message_id: Option<String>,
    reply_ctx: String,
    receive_id_type: String,
    receive_id: String,
    webhook_url: String,
    content_preview: String,
    updated_at: u64,
}

#[tauri::command]
fn inspect_status(app: AppHandle, options: ClientOptions) -> Result<ClientStatus, String> {
    let exe_exists = resolve_existing_path(&options.exe_path).is_some();
    let config_path = resolve_config_read_path(&app, &options.config_path).ok();
    let config_exists = config_path.is_some();
    let raw = config_path
        .as_ref()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .unwrap_or_default();
    let parsed = raw.parse::<toml::Value>().ok();
    let project = parsed
        .as_ref()
        .and_then(|root| find_project(root, &options.project));
    let agent_type = opt_or(&options.agent_type, "codex");
    let project_configured = project.is_some();
    let channel_configured = project
        .map(|project| project_has_platform(project, &options.platform))
        .unwrap_or(false);
    let agent_configured = project
        .and_then(|project| project.get("agent"))
        .and_then(toml::Value::as_table)
        .and_then(|agent| agent.get("type"))
        .and_then(toml::Value::as_str)
        == Some(agent_type);

    let fields = resolved_channel_fields(&app, &options)?;
    let field_ready = required_fields_ready(&options.platform, &fields);
    let binding_ready =
        field_ready || (channel_configured && platform_supports_auto_qr(&options.platform));

    let agent_fields = resolved_agent_fields(&app, &options)?;
    let agent_command = agent_fields.get("command").cloned();
    let agent_installed = agent_is_available(agent_type, &agent_command);

    Ok(ClientStatus {
        exe_exists,
        config_exists,
        bridge_running: false,
        bridge_pid: None,
        project_configured,
        channel_configured,
        agent_configured,
        binding_ready,
        agent_installed,
        connection_status: status_text(project_configured),
        channel_status: status_text(channel_configured),
        agent_status: status_text(agent_configured || agent_type == "mock"),
        binding_status: binding_status_text(&options.platform, binding_ready),
        agent_install_status: agent_install_status_text(agent_type, agent_installed),
        summary: status_summary(
            &options.exe_path,
            &options.config_path,
            exe_exists,
            config_exists,
            project_configured,
            channel_configured,
            agent_configured,
            binding_ready,
            agent_installed,
            &options.project,
            &options.platform,
            agent_type,
        ),
    })
}

#[tauri::command]
fn bridge_runtime_status(state: tauri::State<BridgeState>) -> Result<serde_json::Value, String> {
    let mut guard = state.child.lock().map_err(|err| err.to_string())?;
    if let Some(child) = guard.as_mut() {
        match child.try_wait().map_err(|err| err.to_string())? {
            Some(status) => {
                let code = status.code();
                *guard = None;
                return Ok(json!({
                    "running": false,
                    "pid": null,
                    "status": format!("exited {:?}", code)
                }));
            }
            None => {
                return Ok(json!({
                    "running": true,
                    "pid": child.id(),
                    "status": "running"
                }));
            }
        }
    }
    Ok(json!({
        "running": false,
        "pid": null,
        "status": "not running"
    }))
}

#[tauri::command]
fn hide_to_tray(app: AppHandle) -> Result<String, String> {
    hide_main_window(&app)?;
    Ok("已隐藏到系统托盘。".to_string())
}

#[tauri::command]
fn show_main_window_cmd(app: AppHandle) -> Result<String, String> {
    show_main_window(&app)?;
    Ok("窗口已显示。".to_string())
}

#[tauri::command]
fn read_bridge_logs(max_lines: Option<usize>) -> Result<BridgeLogs, String> {
    let max_lines = max_lines.unwrap_or(160).clamp(20, 1000);
    let log_dir = std::env::temp_dir().join("agentlink-desktop");
    let stdout_path = log_dir.join("agentlink.out.log");
    let stderr_path = log_dir.join("agentlink.err.log");
    let stdout = tail_text_file(&stdout_path, max_lines)?;
    let stderr = tail_text_file(&stderr_path, max_lines)?;
    let mut combined = String::new();
    if !stdout.trim().is_empty() {
        combined.push_str("[stdout]\n");
        combined.push_str(stdout.trim_end());
        combined.push('\n');
    }
    if !stderr.trim().is_empty() {
        combined.push_str("[stderr]\n");
        combined.push_str(stderr.trim_end());
        combined.push('\n');
    }
    if combined.is_empty() {
        combined.push_str(&format!("暂无 Bridge 日志：{}", log_dir.display()));
    }
    Ok(BridgeLogs {
        stdout,
        stderr,
        combined,
    })
}

#[tauri::command]
async fn check_for_updates_bust(
    app: AppHandle,
    state: tauri::State<'_, DesktopUpdateState>,
) -> Result<Option<AvailableUpdatePayload>, String> {
    let update = build_cache_busting_updater(&app)?
        .check()
        .await
        .map_err(|err| err.to_string())?;

    let mut pending = state.pending.lock().map_err(|err| err.to_string())?;
    if let Some(update) = update {
        let payload = available_update_payload(&update);
        *pending = Some(update);
        Ok(Some(payload))
    } else {
        *pending = None;
        Ok(None)
    }
}

#[tauri::command]
async fn install_available_update_bust(
    app: AppHandle,
    state: tauri::State<'_, DesktopUpdateState>,
) -> Result<String, String> {
    let update = state
        .pending
        .lock()
        .map_err(|err| err.to_string())?
        .clone()
        .ok_or_else(|| "没有可安装的更新，请先检查更新。".to_string())?;

    let _ = app.emit(
        "update-download-event",
        json!({
            "event": "Started",
            "data": {
                "contentLength": null
            }
        }),
    );

    let progress_app = app.clone();
    let finish_app = app.clone();
    let last_total = Arc::new(Mutex::new(None::<u64>));
    let last_total_progress = last_total.clone();
    let last_total_finish = last_total.clone();
    update
        .download_and_install(
            move |chunk_length, content_length| {
                if let Ok(mut guard) = last_total_progress.lock() {
                    *guard = content_length;
                }
                let _ = progress_app.emit(
                    "update-download-event",
                    json!({
                        "event": "Progress",
                        "data": {
                            "chunkLength": chunk_length,
                            "contentLength": content_length
                        }
                    }),
                );
            },
            move || {
                let content_length = last_total_finish.lock().ok().and_then(|guard| *guard);
                let _ = finish_app.emit(
                    "update-download-event",
                    json!({
                        "event": "Finished",
                        "data": {
                            "contentLength": content_length
                        }
                    }),
                );
            },
        )
        .await
        .map_err(|err| err.to_string())?;

    *state.pending.lock().map_err(|err| err.to_string())? = None;
    Ok("更新已下载并安装。".to_string())
}

#[tauri::command]
fn check_agent(options: AgentCheckOptions) -> Result<AgentCheckStatus, String> {
    let installed = agent_is_available(&options.agent_type, &options.command);
    Ok(AgentCheckStatus {
        agent_type: options.agent_type.clone(),
        installed,
        status: agent_install_status_text(&options.agent_type, installed),
    })
}

#[tauri::command]
fn pick_path(options: PickPathOptions) -> Result<Option<String>, String> {
    #[cfg(windows)]
    {
        pick_path_windows(options)
    }
    #[cfg(not(windows))]
    {
        pick_path_native(options)
    }
}

#[cfg(windows)]
fn pick_path_windows(options: PickPathOptions) -> Result<Option<String>, String> {
    let current_dir = options
        .current
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(resolve_output_path)
        .and_then(|path| {
            if path.is_dir() {
                Some(path)
            } else {
                path.parent().map(Path::to_path_buf)
            }
        })
        .filter(|path| path.exists())
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    let script = match options.kind.as_str() {
        "exe" => {
            r#"
Add-Type -AssemblyName System.Windows.Forms
$dialog = New-Object System.Windows.Forms.OpenFileDialog
$dialog.Title = '选择 AgentLink 可执行文件'
$dialog.Filter = 'Executable (*.exe)|*.exe|All files (*.*)|*.*'
$dialog.InitialDirectory = $env:AGENTLINK_PICKER_DIR
if ($dialog.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) { [Console]::Out.Write($dialog.FileName) }
"#
        }
        "config" => {
            r#"
Add-Type -AssemblyName System.Windows.Forms
$dialog = New-Object System.Windows.Forms.OpenFileDialog
$dialog.Title = '选择 AgentLink 配置文件'
$dialog.Filter = 'TOML (*.toml)|*.toml|All files (*.*)|*.*'
$dialog.InitialDirectory = $env:AGENTLINK_PICKER_DIR
if ($dialog.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) { [Console]::Out.Write($dialog.FileName) }
"#
        }
        "folder" => {
            r#"
Add-Type -AssemblyName System.Windows.Forms
$dialog = New-Object System.Windows.Forms.FolderBrowserDialog
$dialog.Description = '选择项目工作目录'
$dialog.SelectedPath = $env:AGENTLINK_PICKER_DIR
if ($dialog.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) { [Console]::Out.Write($dialog.SelectedPath) }
"#
        }
        other => return Err(format!("unsupported path picker kind: {other}")),
    };

    let mut command = Command::new("powershell");
    command
        .arg("-NoProfile")
        .arg("-STA")
        .arg("-ExecutionPolicy")
        .arg("Bypass")
        .arg("-Command")
        .arg(script)
        .env("AGENTLINK_PICKER_DIR", current_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.creation_flags(CREATE_NO_WINDOW);
    let output = command.output().map_err(|err| err.to_string())?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            format!("path picker exited with status {}", output.status)
        } else {
            stderr
        });
    }
    let picked = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok((!picked.is_empty()).then_some(picked))
}

#[cfg(not(windows))]
fn pick_path_native(options: PickPathOptions) -> Result<Option<String>, String> {
    let current_dir = options
        .current
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(resolve_output_path)
        .and_then(|path| {
            if path.is_dir() {
                Some(path)
            } else {
                path.parent().map(Path::to_path_buf)
            }
        })
        .filter(|path| path.exists())
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    let dialog = rfd::FileDialog::new().set_directory(&current_dir);
    let picked = match options.kind.as_str() {
        "exe" => dialog.set_title("选择 AgentLink 可执行文件").pick_file(),
        "config" => dialog
            .set_title("选择 AgentLink 配置文件")
            .add_filter("TOML", &["toml"])
            .pick_file(),
        "folder" => dialog.set_title("选择项目工作目录").pick_folder(),
        other => return Err(format!("unsupported path picker kind: {other}")),
    };

    Ok(picked.and_then(|path| path.into_os_string().into_string().ok()))
}

#[tauri::command]
fn load_config_snapshot(app: AppHandle, options: ClientOptions) -> Result<ConfigSnapshot, String> {
    let config_path = resolve_config_read_path(&app, &options.config_path)?;
    let raw = std::fs::read_to_string(&config_path).map_err(|err| err.to_string())?;
    let parsed = raw.parse::<toml::Value>().map_err(|err| err.to_string())?;
    let project = find_project(&parsed, &options.project)
        .ok_or_else(|| format!("project `{}` not found in config", options.project))?;

    let selected_channel = project
        .get("default_platforms")
        .and_then(toml::Value::as_array)
        .and_then(|items| items.first())
        .and_then(toml::Value::as_str)
        .or_else(|| {
            project
                .get("platforms")
                .and_then(toml::Value::as_array)
                .and_then(|items| items.first())
                .and_then(toml::Value::as_table)
                .and_then(|item| item.get("id").or_else(|| item.get("type")))
                .and_then(toml::Value::as_str)
        })
        .unwrap_or(&options.platform)
        .to_string();
    let mut options_for_fields = options.clone();
    options_for_fields.platform = selected_channel.clone();
    let channel_fields = resolved_channel_fields(&app, &options_for_fields)?;

    let agent = project
        .get("agent")
        .and_then(toml::Value::as_table)
        .ok_or_else(|| "project agent section not found".to_string())?;
    let selected_agent = agent
        .get("type")
        .and_then(toml::Value::as_str)
        .unwrap_or("codex")
        .to_string();
    let mut options_for_agent = options.clone();
    options_for_agent.agent_type = Some(selected_agent.clone());
    let agent_fields = resolved_agent_fields(&app, &options_for_agent)?;
    let work_dir = agent_fields
        .get("workDir")
        .or_else(|| agent_fields.get("work_dir"))
        .cloned()
        .unwrap_or_else(|| options.work_dir.clone());

    Ok(ConfigSnapshot {
        project: options.project,
        work_dir,
        selected_channel,
        selected_agent,
        channel_fields,
        agent_fields,
    })
}

#[tauri::command]
fn load_client_state(app: tauri::AppHandle) -> Result<Option<ClientDatabaseState>, String> {
    let db = open_client_db(&app)?;
    let active_connection_id: Option<String> = db
        .query_row(
            "select value from settings where key = 'active_connection_id'",
            [],
            |row| row.get(0),
        )
        .ok();
    let update_preferences = load_update_preferences(&db);

    let mut stmt = db
        .prepare("select data from connections order by updated_at desc, id asc")
        .map_err(|err| err.to_string())?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|err| err.to_string())?;

    let mut connections = Vec::new();
    for row in rows {
        let raw = row.map_err(|err| err.to_string())?;
        let value = serde_json::from_str::<ClientConnectionState>(&raw)
            .map_err(|err| format!("invalid client state json: {err}"))?;
        connections.push(value);
    }

    if connections.is_empty() {
        return Ok(None);
    }

    overlay_connections_from_sqlite(&db, &mut connections)?;

    let active_connection_id = active_connection_id
        .filter(|id| connections.iter().any(|item| item.id == *id))
        .unwrap_or_else(|| connections[0].id.clone());

    Ok(Some(ClientDatabaseState {
        active_connection_id,
        connections,
        update_preferences,
    }))
}

#[tauri::command]
fn save_client_state(app: tauri::AppHandle, state: ClientDatabaseState) -> Result<String, String> {
    let mut db = open_client_db(&app)?;
    let tx = db.transaction().map_err(|err| err.to_string())?;
    tx.execute(
        "insert into settings(key, value) values('active_connection_id', ?1)
         on conflict(key) do update set value = excluded.value",
        [&state.active_connection_id],
    )
    .map_err(|err| err.to_string())?;
    let update_preferences =
        serde_json::to_string(&state.update_preferences).map_err(|err| err.to_string())?;
    tx.execute(
        "insert into settings(key, value) values('update_preferences', ?1)
         on conflict(key) do update set value = excluded.value",
        [&update_preferences],
    )
    .map_err(|err| err.to_string())?;

    let ids: Vec<&str> = state
        .connections
        .iter()
        .map(|item| item.id.as_str())
        .collect();
    if ids.is_empty() {
        tx.execute("delete from connections", [])
            .map_err(|err| err.to_string())?;
    } else {
        let placeholders = vec!["?"; ids.len()].join(",");
        let sql = format!("delete from connections where id not in ({placeholders})");
        tx.execute(&sql, rusqlite::params_from_iter(ids))
            .map_err(|err| err.to_string())?;
    }

    sync_connection_settings_from_connections(&tx, &state.connections)?;
    sync_channel_configs_from_connections(&tx, &state.connections)?;
    sync_agent_configs_from_connections(&tx, &state.connections)?;
    for connection in &state.connections {
        let data = serde_json::to_string(connection).map_err(|err| err.to_string())?;
        tx.execute(
            "insert into connections(id, data, updated_at) values(?1, ?2, strftime('%s','now'))
             on conflict(id) do update set data = excluded.data, updated_at = excluded.updated_at",
            (&connection.id, &data),
        )
        .map_err(|err| err.to_string())?;
    }
    tx.commit().map_err(|err| err.to_string())?;
    Ok("client state saved to sqlite".to_string())
}

fn load_update_preferences(db: &rusqlite::Connection) -> UpdatePreferences {
    let raw: Option<String> = db
        .query_row(
            "select value from settings where key = 'update_preferences'",
            [],
            |row| row.get(0),
        )
        .ok();
    raw.and_then(|value| serde_json::from_str(&value).ok())
        .unwrap_or_default()
}

#[tauri::command]
fn validate_config(app: AppHandle, options: ClientOptions) -> Result<String, String> {
    validate_config_local(&app, &options)
}

#[tauri::command]
fn validate_channel_binding(
    app: AppHandle,
    options: ClientOptions,
) -> Result<ChannelBindingStatus, String> {
    Ok(channel_binding_status(&app, &options)?)
}

#[tauri::command]
fn prepare_channel_binding(
    app: AppHandle,
    options: ClientOptions,
    state: tauri::State<'_, BridgeState>,
) -> Result<QrSetupStatus, String> {
    start_guided_channel_binding(app, options, state)
}

#[tauri::command]
async fn send_test_message(request: TestMessageRequest) -> Result<TestMessageResponse, String> {
    let url = request.webhook_url.trim();
    if url.is_empty() {
        return Err("HTTP webhook URL is empty.".to_string());
    }

    let client = reqwest::Client::new();
    let mut builder = client.post(url).json(&request.payload);
    if let Some(token) = request
        .bearer_token
        .as_deref()
        .map(str::trim)
        .filter(|token| !token.is_empty())
    {
        builder = builder.bearer_auth(token);
    }
    let response = builder.send().await.map_err(|err| {
        format!(
            "Failed to connect to `{url}`: {err}. Make sure the HTTP platform is started and the URL matches its listen address."
        )
    })?;
    let status = response.status();
    let body = response.text().await.map_err(|err| err.to_string())?;
    if status.is_success() {
        Ok(TestMessageResponse {
            status: status.as_u16(),
            body,
        })
    } else {
        Err(format!("HTTP {} from `{url}`: {body}", status.as_u16()))
    }
}

#[tauri::command]
async fn send_channel_message(
    request: ChannelMessageRequest,
) -> Result<TestMessageResponse, String> {
    let platform = request.options.platform.trim().to_lowercase();
    match platform.as_str() {
        "http" => send_http_test_message(&request).await,
        "feishu" | "lark" => send_feishu_direct_message(&request).await,
        "telegram" => send_telegram_direct_message(&request).await,
        "dingtalk" => send_dingtalk_webhook_message(&request).await,
        "slack" => send_slack_direct_message(&request).await,
        "discord" => send_discord_direct_message(&request).await,
        "line" => send_line_direct_message(&request).await,
        "weixin" => send_weixin_direct_message(&request).await,
        other => Err(format!(
            "{other} 暂不支持客户端直发测试。请先让真实用户在该通道发一条消息，或使用 HTTP 测试通道注入消息。"
        )),
    }
}

#[tauri::command]
fn discover_channel_targets(
    app: AppHandle,
    options: ClientOptions,
) -> Result<Vec<DiscoveredTarget>, String> {
    let mut out = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for db_path in project_store_candidates(&app, &options)? {
        if !db_path.exists() {
            continue;
        }
        let conn = match rusqlite::Connection::open(&db_path) {
            Ok(conn) => conn,
            Err(_) => continue,
        };
        let mut stmt = match conn.prepare(
            r#"
SELECT platform, session_key, user_id, user_name, message_id, reply_ctx, content_preview, updated_at
FROM targets
WHERE project = ?1 AND platform = ?2
ORDER BY updated_at DESC
LIMIT 100
"#,
        ) {
            Ok(stmt) => stmt,
            Err(_) => continue,
        };
        let rows = stmt
            .query_map((&options.project, &options.platform), |row| {
                let platform: String = row.get(0)?;
                let session_key: String = row.get(1)?;
                let user_id: String = row.get(2)?;
                let user_name: Option<String> = row.get(3)?;
                let message_id: Option<String> = row.get(4)?;
                let reply_ctx: String = row.get(5)?;
                let content_preview: String = row.get(6)?;
                let updated_at: u64 = row.get(7)?;
                let (receive_id_type, receive_id, webhook_url) =
                    discovered_receive_fields(&platform, &reply_ctx, &user_id);
                let name = user_name
                    .clone()
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or_else(|| session_key.clone());
                Ok(DiscoveredTarget {
                    name,
                    platform,
                    session_key,
                    user_id,
                    user_name,
                    message_id,
                    reply_ctx,
                    receive_id_type,
                    receive_id,
                    webhook_url,
                    content_preview,
                    updated_at,
                })
            })
            .map_err(|err| err.to_string())?;
        for row in rows {
            let target = row.map_err(|err| err.to_string())?;
            if seen.insert(target.session_key.clone()) {
                out.push(target);
            }
        }
    }
    out.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(out)
}

async fn send_http_test_message(
    request: &ChannelMessageRequest,
) -> Result<TestMessageResponse, String> {
    let fields = request.options.channel_fields.as_ref();
    let url = opt_target(&request.target.webhook_url)
        .or_else(|| field(fields, "webhook_url"))
        .unwrap_or_else(|| {
            let listen = field(fields, "listen").unwrap_or_else(|| "127.0.0.1:18080".to_string());
            format!("http://{listen}/webhook")
        });
    let payload = request.payload.clone().unwrap_or_else(|| {
        json!({
            "session_key": opt_target(&request.target.session_key).unwrap_or_else(|| "client:test:user".to_string()),
            "user_id": opt_target(&request.target.user_id).unwrap_or_else(|| "client-user".to_string()),
            "user_name": opt_target(&request.target.user_name).unwrap_or_else(|| "Desktop Client".to_string()),
            "message_type": request.message_type.as_deref().unwrap_or("text"),
            "content": request.content,
            "reply_ctx": "client-test"
        })
    });
    send_json_post(
        &url,
        field(fields, "bearer_token"),
        payload,
        "HTTP Platform",
    )
    .await
}

async fn send_feishu_direct_message(
    request: &ChannelMessageRequest,
) -> Result<TestMessageResponse, String> {
    let fields = request.options.channel_fields.as_ref();
    let app_id = required_field(fields, "app_id")?;
    let app_secret = required_field(fields, "app_secret")?;
    let api_base = field(fields, "api_base").unwrap_or_else(|| {
        if request.options.platform == "lark" {
            OPEN_LARK_BASE.to_string()
        } else {
            OPEN_FEISHU_BASE.to_string()
        }
    });
    let receive_id_type =
        opt_target(&request.target.receive_id_type).unwrap_or_else(|| "chat_id".to_string());
    let receive_id = required_target_receive_id(&request.target)?;
    let client = reqwest::Client::new();
    let token_value: serde_json::Value = client
        .post(format!(
            "{api_base}/open-apis/auth/v3/tenant_access_token/internal"
        ))
        .json(&json!({ "app_id": app_id, "app_secret": app_secret }))
        .send()
        .await
        .map_err(|err| err.to_string())?
        .error_for_status()
        .map_err(|err| err.to_string())?
        .json()
        .await
        .map_err(|err| err.to_string())?;
    let token = token_value
        .get("tenant_access_token")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| format!("飞书 token 响应缺少 tenant_access_token: {token_value}"))?;
    let body = json!({
        "receive_id": receive_id,
        "msg_type": "text",
        "content": serde_json::to_string(&json!({ "text": request.content })).map_err(|err| err.to_string())?
    });
    send_json_post(
        &format!("{api_base}/open-apis/im/v1/messages?receive_id_type={receive_id_type}"),
        Some(token.to_string()),
        body,
        "Feishu",
    )
    .await
}

async fn send_telegram_direct_message(
    request: &ChannelMessageRequest,
) -> Result<TestMessageResponse, String> {
    let fields = request.options.channel_fields.as_ref();
    let token = required_field(fields, "token")?;
    let api_base =
        field(fields, "api_base").unwrap_or_else(|| "https://api.telegram.org".to_string());
    let chat_id = required_target_receive_id(&request.target)?;
    let mut body = json!({
        "chat_id": chat_id,
        "text": request.content,
        "disable_web_page_preview": true
    });
    if let Some(message_id) =
        opt_target(&request.target.message_id).and_then(|v| v.parse::<i64>().ok())
    {
        body["reply_to_message_id"] = json!(message_id);
    }
    send_json_post(
        &format!("{api_base}/bot{token}/sendMessage"),
        None,
        body,
        "Telegram",
    )
    .await
}

async fn send_dingtalk_webhook_message(
    request: &ChannelMessageRequest,
) -> Result<TestMessageResponse, String> {
    let url = opt_target(&request.target.session_webhook)
        .or_else(|| opt_target(&request.target.webhook_url))
        .or_else(|| opt_target(&request.target.receive_id))
        .ok_or_else(|| "钉钉测试需要填写 sessionWebhook 或机器人 webhook URL。".to_string())?;
    send_json_post(
        &url,
        None,
        json!({
            "msgtype": "markdown",
            "markdown": {
                "title": "AgentLink",
                "text": request.content
            }
        }),
        "DingTalk",
    )
    .await
}

async fn send_slack_direct_message(
    request: &ChannelMessageRequest,
) -> Result<TestMessageResponse, String> {
    let fields = request.options.channel_fields.as_ref();
    let token = required_field(fields, "bot_token")?;
    let channel = required_target_receive_id(&request.target)?;
    send_json_post(
        "https://slack.com/api/chat.postMessage",
        Some(token),
        json!({ "channel": channel, "text": request.content }),
        "Slack",
    )
    .await
}

async fn send_discord_direct_message(
    request: &ChannelMessageRequest,
) -> Result<TestMessageResponse, String> {
    let fields = request.options.channel_fields.as_ref();
    let token = required_field(fields, "token")?;
    let channel_id = required_target_receive_id(&request.target)?;
    send_json_post(
        &format!("https://discord.com/api/v10/channels/{channel_id}/messages"),
        Some(token),
        json!({ "content": request.content }),
        "Discord",
    )
    .await
}

async fn send_line_direct_message(
    request: &ChannelMessageRequest,
) -> Result<TestMessageResponse, String> {
    let fields = request.options.channel_fields.as_ref();
    let token = required_field(fields, "token")?;
    let to = required_target_receive_id(&request.target)?;
    send_json_post(
        "https://api.line.me/v2/bot/message/push",
        Some(token),
        json!({ "to": to, "messages": [{ "type": "text", "text": request.content }] }),
        "LINE",
    )
    .await
}

async fn send_weixin_direct_message(
    request: &ChannelMessageRequest,
) -> Result<TestMessageResponse, String> {
    let fields = request.options.channel_fields.as_ref();
    let token = required_field(fields, "token")?;
    let api_base = field(fields, "api_base").unwrap_or_else(|| DEFAULT_WEIXIN_API_BASE.to_string());
    let to_user_id = required_target_receive_id(&request.target)?;
    let reply_ctx = opt_target(&request.target.reply_context).ok_or_else(|| {
        "微信测试发送需要先从微信给机器人发一条消息，用发现到的目标缓存 context_token。".to_string()
    })?;
    let reply_value = serde_json::from_str::<serde_json::Value>(&reply_ctx)
        .map_err(|err| format!("微信 replyContext 解析失败：{err}"))?;
    let context_token = json_string(&reply_value, "context_token").unwrap_or_default();
    let client_id = json_string(&reply_value, "client_id").unwrap_or_default();
    if context_token.is_empty() {
        return Err(
            "微信目标缺少 context_token。请先从微信给机器人发一条新消息，再刷新发现目标。"
                .to_string(),
        );
    }
    let body = json!({
        "msg": {
            "to_user_id": to_user_id,
            "client_id": client_id,
            "message_type": 2,
            "message_state": 2,
            "context_token": context_token,
            "item_list": [{ "type": 1, "text_item": { "text": request.content }}]
        },
        "base_info": { "channel_version": "agentlink-desktop/1.0" }
    });
    let url = format!("{}/ilink/bot/sendmessage", api_base.trim_end_matches('/'));
    let client = reqwest::Client::new();
    let mut builder = client
        .post(&url)
        .bearer_auth(token)
        .header("AuthorizationType", "ilink_bot_token")
        .header("X-WECHAT-UIN", weixin_uin_header());
    if let Some(route_tag) = field(fields, "route_tag") {
        builder = builder.header("SKRouteTag", route_tag);
    }
    let response = builder
        .json(&body)
        .send()
        .await
        .map_err(|err| format!("Weixin 请求失败 `{url}`: {err}"))?;
    let status = response.status();
    let body = response.text().await.map_err(|err| err.to_string())?;
    if status.is_success() {
        Ok(TestMessageResponse {
            status: status.as_u16(),
            body,
        })
    } else {
        Err(format!(
            "Weixin HTTP {} from `{url}`: {body}",
            status.as_u16()
        ))
    }
}

async fn send_json_post(
    url: &str,
    bearer_token: Option<String>,
    payload: serde_json::Value,
    label: &str,
) -> Result<TestMessageResponse, String> {
    let client = reqwest::Client::new();
    let mut builder = client.post(url).json(&payload);
    if let Some(token) = bearer_token
        .as_deref()
        .map(str::trim)
        .filter(|token| !token.is_empty())
    {
        builder = builder.bearer_auth(token);
    }
    let response = builder
        .send()
        .await
        .map_err(|err| format!("{label} 请求失败 `{url}`: {err}"))?;
    let status = response.status();
    let body = response.text().await.map_err(|err| err.to_string())?;
    if status.is_success() {
        Ok(TestMessageResponse {
            status: status.as_u16(),
            body,
        })
    } else {
        Err(format!(
            "{label} HTTP {} from `{url}`: {body}",
            status.as_u16()
        ))
    }
}

fn field(fields: Option<&HashMap<String, String>>, key: &str) -> Option<String> {
    fields?
        .get(key)
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn required_field(fields: Option<&HashMap<String, String>>, key: &str) -> Result<String, String> {
    field(fields, key).ok_or_else(|| format!("缺少 Channel 配置字段 `{key}`。"))
}

fn opt_target(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn required_target_receive_id(target: &ChannelTargetRequest) -> Result<String, String> {
    opt_target(&target.receive_id)
        .or_else(|| opt_target(&target.webhook_url))
        .ok_or_else(|| "请先在测试目标里填写接收 ID。".to_string())
}

fn weixin_uin_header() -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(rand::random::<u32>().to_string())
}

fn config_data_dir(app: &AppHandle, options: &ClientOptions) -> Result<PathBuf, String> {
    let config_path = resolve_config_read_path(app, &options.config_path)?;
    Ok(config_path
        .parent()
        .map(|parent| parent.join("data"))
        .unwrap_or_else(|| PathBuf::from("data")))
}

fn project_store_candidates(
    app: &AppHandle,
    options: &ClientOptions,
) -> Result<Vec<PathBuf>, String> {
    let config_path = resolve_config_read_path(app, &options.config_path)?;
    let data_dir = std::fs::read_to_string(&config_path)
        .ok()
        .and_then(|raw| raw.parse::<toml::Value>().ok())
        .and_then(|root| {
            root.get("data_dir")
                .and_then(toml::Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| "./data".to_string());
    let data_path = PathBuf::from(&data_dir);
    let config_parent = config_path.parent().map(Path::to_path_buf);
    let current_dir = std::env::current_dir().map_err(|err| err.to_string())?;
    let mut dirs = Vec::new();
    if data_path.is_absolute() {
        dirs.push(data_path);
    } else {
        if let Some(parent) = config_parent {
            dirs.push(parent.join(&data_path));
        }
        dirs.push(current_dir.join(&data_path));
    }
    dirs.push(config_data_dir(app, options)?);
    dirs.push(std::env::temp_dir().join("agentlink-live-data"));

    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    for dir in dirs {
        let db = dir.join(format!("{}.sqlite3", options.project));
        let key = db.to_string_lossy().to_string();
        if seen.insert(key) {
            out.push(db);
        }
    }
    Ok(out)
}

fn discovered_receive_fields(
    platform: &str,
    reply_ctx: &str,
    user_id: &str,
) -> (String, String, String) {
    let value = serde_json::from_str::<serde_json::Value>(reply_ctx).unwrap_or_default();
    match platform {
        "feishu" | "lark" => (
            "chat_id".to_string(),
            json_string(&value, "chat_id").unwrap_or_default(),
            String::new(),
        ),
        "telegram" => (
            "chat_id".to_string(),
            json_string(&value, "chat_id").unwrap_or_default(),
            String::new(),
        ),
        "dingtalk" => (
            "session_webhook".to_string(),
            json_string(&value, "conversation_id").unwrap_or_default(),
            json_string(&value, "session_webhook").unwrap_or_default(),
        ),
        "qq" => {
            if let Some(group_id) = json_string(&value, "group_id") {
                ("group_id".to_string(), group_id, String::new())
            } else {
                (
                    "user_id".to_string(),
                    json_string(&value, "user_id").unwrap_or_else(|| user_id.to_string()),
                    String::new(),
                )
            }
        }
        "slack" | "discord" | "line" => (
            "channel_id".to_string(),
            json_string(&value, "channel_id")
                .or_else(|| json_string(&value, "chat_id"))
                .unwrap_or_default(),
            String::new(),
        ),
        _ => ("user_id".to_string(), user_id.to_string(), String::new()),
    }
}

fn json_string(value: &serde_json::Value, key: &str) -> Option<String> {
    value.get(key).and_then(|item| {
        item.as_str()
            .map(str::to_string)
            .or_else(|| item.as_i64().map(|value| value.to_string()))
            .or_else(|| item.as_u64().map(|value| value.to_string()))
    })
}

#[tauri::command]
fn setup_channel(options: ClientOptions) -> Result<String, String> {
    let exe = resolve_exe_path(&options.exe_path)?;
    let args = setup_args(&options)?;
    run_capture_owned(&exe, args)
}

#[tauri::command]
fn launch_qr_setup(options: ClientOptions) -> Result<String, String> {
    if !platform_supports_scan(&options.platform) {
        return Err(format!(
            "{} does not support QR setup. Please fill credentials manually.",
            options.platform
        ));
    }

    let exe = resolve_exe_path(&options.exe_path)?;
    let args = setup_args(&options)?;
    run_capture_owned(&exe, args)
}

#[tauri::command]
async fn start_qr_setup(
    app: AppHandle,
    options: ClientOptions,
    state: tauri::State<'_, BridgeState>,
) -> Result<QrSetupStatus, String> {
    if !platform_supports_auto_qr(&options.platform) {
        return Err(format!(
            "{} 不支持客户端内自动扫码，请使用「配置引导」填写并校验字段。",
            options.platform
        ));
    }

    if matches!(options.platform.as_str(), "feishu" | "lark") {
        return start_feishu_qr_setup(app, options, state).await;
    }
    if options.platform == "weixin" {
        return start_weixin_qr_setup(app, options, state).await;
    }

    Err(format!("{} 未实现客户端内扫码绑定。", options.platform))
}

#[tauri::command]
fn get_qr_setup_status(
    session_id: String,
    state: tauri::State<BridgeState>,
) -> Result<Option<QrSetupStatus>, String> {
    let sessions = state.qr_sessions.lock().map_err(|err| err.to_string())?;
    sessions
        .get(&session_id)
        .map(qr_setup_status_from_arc)
        .transpose()
}

#[tauri::command]
fn save_config(app: AppHandle, options: ClientOptions) -> Result<SaveConfigResult, String> {
    let config_path = write_config_file(&app, &options)?;
    Ok(SaveConfigResult {
        message: format!("config saved: {}", config_path.display()),
        config_path: normalize_display_path(&config_path),
    })
}

#[tauri::command]
fn save_channel_config(
    app: AppHandle,
    connection_id: String,
    platform: String,
    fields: HashMap<String, String>,
) -> Result<String, String> {
    let db = open_client_db(&app)?;
    upsert_channel_config(
        &db,
        &connection_id,
        &platform.trim().to_lowercase(),
        &fields,
    )?;
    Ok("channel config saved to sqlite".to_string())
}

#[tauri::command]
fn save_agent_config(
    app: AppHandle,
    connection_id: String,
    agent_type: String,
    fields: HashMap<String, String>,
) -> Result<String, String> {
    let db = open_client_db(&app)?;
    upsert_agent_config(
        &db,
        &connection_id,
        &agent_type.trim().to_lowercase(),
        &fields,
    )?;
    Ok("agent config saved to sqlite".to_string())
}

/// Create a writable config file when none exists yet (install / first-run friendly).
#[tauri::command]
fn ensure_config_file(app: AppHandle, options: ClientOptions) -> Result<SaveConfigResult, String> {
    if let Ok(path) = resolve_config_read_path(&app, &options.config_path) {
        return Ok(SaveConfigResult {
            message: format!("config ready: {}", path.display()),
            config_path: normalize_display_path(&path),
        });
    }
    let config_path = write_config_file(&app, &options)?;
    Ok(SaveConfigResult {
        message: format!("config created: {}", config_path.display()),
        config_path: normalize_display_path(&config_path),
    })
}

fn write_config_file(app: &AppHandle, options: &ClientOptions) -> Result<PathBuf, String> {
    let config_path = resolve_config_write_path(app, &options.config_path)?;
    let mut options_for_render = options.clone();
    options_for_render.config_path = normalize_display_path(&config_path);
    options_for_render.channel_fields = Some(resolved_channel_fields(app, options)?);
    let rendered = render_config(app, &options_for_render)?;
    if let Some(parent) = config_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
    }
    std::fs::write(&config_path, rendered).map_err(|err| err.to_string())?;
    persist_channel_fields(app, options)?;
    persist_agent_fields(app, options)?;
    Ok(config_path)
}

#[tauri::command]
fn start_bridge(
    app: AppHandle,
    options: ClientOptions,
    state: tauri::State<BridgeState>,
) -> Result<String, String> {
    let config = write_config_file(&app, &options)?;
    let exe = resolve_exe_path(&options.exe_path)?;
    let mut guard = state.child.lock().map_err(|err| err.to_string())?;
    if let Some(mut child) = guard.take() {
        match child.try_wait().map_err(|err| err.to_string())? {
            Some(status) => {
                if !status.success() {
                    return Err(format!("previous bridge exited with status {status}"));
                }
            }
            None => {
                *guard = Some(child);
                return Ok("bridge already running".to_string());
            }
        }
    }

    let log_dir = std::env::temp_dir().join("agentlink-desktop");
    std::fs::create_dir_all(&log_dir).map_err(|err| err.to_string())?;
    let stdout_path = log_dir.join("agentlink.out.log");
    let stderr_path = log_dir.join("agentlink.err.log");
    let stdout = std::fs::File::create(&stdout_path).map_err(|err| err.to_string())?;
    let stderr = std::fs::File::create(&stderr_path).map_err(|err| err.to_string())?;
    let mut command = Command::new(exe);
    command
        .arg("--config")
        .arg(&config)
        .arg("--platform")
        .arg(&options.platform)
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    let mut child = command.spawn().map_err(|err| err.to_string())?;
    std::thread::sleep(std::time::Duration::from_millis(800));
    if let Some(status) = child.try_wait().map_err(|err| err.to_string())? {
        let stdout = std::fs::read_to_string(&stdout_path).unwrap_or_default();
        let stderr = std::fs::read_to_string(&stderr_path).unwrap_or_default();
        let detail = [stdout.trim(), stderr.trim()]
            .into_iter()
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        return Err(format!(
            "bridge exited immediately with status {status}. config: {}{}{}",
            config.display(),
            if detail.is_empty() { "" } else { "\n" },
            detail
        ));
    }
    *guard = Some(child);
    Ok(format!(
        "bridge started with platform {}; config {}; logs {} / {}",
        options.platform,
        config.display(),
        stdout_path.display(),
        stderr_path.display()
    ))
}

#[tauri::command]
fn stop_bridge(state: tauri::State<BridgeState>) -> Result<String, String> {
    let mut guard = state.child.lock().map_err(|err| err.to_string())?;
    if let Some(mut child) = guard.take() {
        child.kill().map_err(|err| err.to_string())?;
        return Ok("bridge stopped".to_string());
    }
    Ok("bridge is not running".to_string())
}

fn setup_args(options: &ClientOptions) -> Result<Vec<String>, String> {
    setup_args_inner(options, true)
}

fn setup_args_inner(
    options: &ClientOptions,
    include_existing_credentials: bool,
) -> Result<Vec<String>, String> {
    let config = resolve_output_path(&options.config_path);
    let mut args = vec![
        "setup".to_string(),
        "--platform".to_string(),
        options.platform.clone(),
        "--config".to_string(),
        path_str(&config)?.to_string(),
        "--project".to_string(),
        options.project.clone(),
        "--work-dir".to_string(),
        options.work_dir.clone(),
    ];

    let fields = options.channel_fields.clone().unwrap_or_default();
    match options.platform.as_str() {
        "qq" => {
            args.push("--ws-url".to_string());
            args.push(
                field_or_extra(options, &fields, "ws_url")
                    .unwrap_or_else(|| "ws://127.0.0.1:3001".to_string()),
            );
            if let Some(token) = non_empty(fields.get("token")) {
                args.push("--token".to_string());
                args.push(token);
            }
        }
        "weixin" => {
            if let Some(token) = non_empty(fields.get("token")) {
                args.push("--token".to_string());
                args.push(token);
            }
        }
        "feishu" | "lark" if include_existing_credentials => {
            let app_id = non_empty(fields.get("app_id"));
            let app_secret = non_empty(fields.get("app_secret"));
            if let (Some(app_id), Some(app_secret)) = (app_id, app_secret) {
                args.push("--app".to_string());
                args.push(format!("{app_id}:{app_secret}"));
            }
        }
        _ => {}
    }

    Ok(args)
}

fn render_config(app: &AppHandle, options: &ClientOptions) -> Result<String, String> {
    let agent_type = opt_or(&options.agent_type, "codex");
    let agent_fields = resolved_agent_fields(app, options)?;
    let agent_mode = agent_fields
        .get("mode")
        .map(|value| value.as_str())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| opt_or(&options.agent_mode, "suggest"));
    let agent_command = agent_fields
        .get("command")
        .map(|value| value.as_str())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| options.agent_command.as_deref().unwrap_or_default());
    let fields = options
        .channel_fields
        .clone()
        .unwrap_or_else(|| resolved_channel_fields(app, options).unwrap_or_default());
    let platform_id = options.platform.as_str();
    let data_dir = config_data_dir(app, options)?;
    let mut out = String::new();

    out.push_str(&format!(
        "data_dir = {}\n\n",
        toml_string(&data_dir.to_string_lossy())
    ));
    out.push_str("[[projects]]\n");
    out.push_str(&format!("name = {}\n", toml_string(&options.project)));
    out.push_str(&format!(
        "default_platforms = [{}]\n\n",
        toml_string(platform_id)
    ));

    out.push_str("[projects.agent]\n");
    out.push_str(&format!("type = {}\n\n", toml_string(agent_type)));
    out.push_str("[projects.agent.options]\n");
    let work_dir = agent_fields
        .get("workDir")
        .or_else(|| agent_fields.get("work_dir"))
        .map(|value| value.as_str())
        .filter(|value| !value.is_empty())
        .unwrap_or(options.work_dir.as_str());
    out.push_str(&format!("work_dir = {}\n", toml_string(work_dir)));
    if agent_type == "codex" {
        let backend = agent_fields
            .get("backend")
            .map(|value| value.as_str())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| opt_or(&options.agent_backend, "exec"));
        out.push_str(&format!("backend = {}\n", toml_string(backend)));
        out.push_str(&format!("mode = {}\n", toml_string(agent_mode)));
        out.push_str(&format!(
            "codex_bin = {}\n",
            toml_string(if agent_command.is_empty() {
                "codex"
            } else {
                agent_command
            })
        ));
        push_optional(
            &mut out,
            "model",
            agent_fields
                .get("model")
                .map(|value| value.as_str())
                .or(options.agent_model.as_deref()),
        );
        push_optional(
            &mut out,
            "reasoning_effort",
            agent_fields
                .get("reasoningEffort")
                .or_else(|| agent_fields.get("reasoning_effort"))
                .map(|value| value.as_str())
                .or(options.reasoning_effort.as_deref()),
        );
    } else if agent_type != "mock" {
        push_optional(&mut out, "command", Some(agent_command));
        push_optional(
            &mut out,
            "model",
            agent_fields
                .get("model")
                .map(|value| value.as_str())
                .or(options.agent_model.as_deref()),
        );
        push_optional(&mut out, "mode", Some(agent_mode));
        let args = lines(
            agent_fields
                .get("args")
                .map(|value| value.as_str())
                .unwrap_or_else(|| options.agent_args.as_deref().unwrap_or_default()),
        );
        if !args.is_empty() {
            out.push_str("args = [");
            out.push_str(
                &args
                    .iter()
                    .map(|arg| toml_string(arg))
                    .collect::<Vec<_>>()
                    .join(", "),
            );
            out.push_str("]\n");
        }
    }
    out.push('\n');

    out.push_str("[[projects.platforms]]\n");
    out.push_str(&format!("id = {}\n", toml_string(platform_id)));
    out.push_str(&format!("type = {}\n\n", toml_string(platform_id)));
    out.push_str("[projects.platforms.options]\n");
    out.push_str(&format!("name = {}\n", toml_string(platform_id)));
    for (key, value) in normalized_platform_fields(platform_id, &fields) {
        if value.trim().is_empty() {
            continue;
        }
        if matches!(
            key.as_str(),
            "dry_run" | "insecure" | "share_session_in_channel"
        ) {
            out.push_str(&format!("{} = {}\n", key, parse_bool(&value)));
        } else {
            out.push_str(&format!("{} = {}\n", key, toml_string(&value)));
        }
    }

    Ok(out)
}

fn normalized_platform_fields(
    platform: &str,
    fields: &HashMap<String, String>,
) -> Vec<(String, String)> {
    let mut items = BTreeMap::new();
    for (key, value) in fields {
        let key = key.trim();
        if key.is_empty() || matches!(key, "name" | "id" | "type") {
            continue;
        }
        items.insert(key.to_string(), value.clone());
    }
    if matches!(platform, "feishu" | "lark") {
        items
            .entry("connection_mode".to_string())
            .or_insert_with(|| "websocket".to_string());
    }
    items.into_iter().collect()
}

fn find_project<'a>(root: &'a toml::Value, project_name: &str) -> Option<&'a toml::value::Table> {
    root.get("projects")?
        .as_array()?
        .iter()
        .filter_map(toml::Value::as_table)
        .find(|project| {
            project
                .get("name")
                .and_then(toml::Value::as_str)
                .is_some_and(|name| name == project_name)
        })
}

fn project_has_platform(project: &toml::value::Table, platform: &str) -> bool {
    project
        .get("default_platforms")
        .and_then(toml::Value::as_array)
        .is_some_and(|items| {
            items
                .iter()
                .any(|item| item.as_str().is_some_and(|name| name == platform))
        })
        || project
            .get("platforms")
            .and_then(toml::Value::as_array)
            .is_some_and(|items| {
                items.iter().filter_map(toml::Value::as_table).any(|item| {
                    item.get("id")
                        .or_else(|| item.get("type"))
                        .and_then(toml::Value::as_str)
                        .is_some_and(|name| name == platform)
                })
            })
}

fn platform_options(
    project: &toml::value::Table,
    platform: &str,
) -> Option<HashMap<String, String>> {
    let item = project
        .get("platforms")?
        .as_array()?
        .iter()
        .filter_map(toml::Value::as_table)
        .find(|item| {
            item.get("id")
                .or_else(|| item.get("type"))
                .and_then(toml::Value::as_str)
                .is_some_and(|name| name == platform)
        })?;
    let options = item.get("options")?.as_table()?;
    Some(
        options
            .iter()
            .map(|(key, value)| (key.clone(), toml_value_to_string(value)))
            .collect(),
    )
}

fn toml_value_to_string(value: &toml::Value) -> String {
    if let Some(value) = value.as_str() {
        value.to_string()
    } else if let Some(value) = value.as_bool() {
        value.to_string()
    } else if let Some(value) = value.as_integer() {
        value.to_string()
    } else if let Some(value) = value.as_float() {
        value.to_string()
    } else if let Some(values) = value.as_array() {
        values
            .iter()
            .map(toml_value_to_string)
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        String::new()
    }
}

fn agent_option_key(agent_type: &str, key: &str) -> String {
    match (agent_type, key) {
        ("codex", "codex_bin") => "command".to_string(),
        ("codex", "reasoning_effort") => "reasoningEffort".to_string(),
        (_, "work_dir") => "workDir".to_string(),
        _ => key.to_string(),
    }
}

fn qr_svg(content: &str) -> Result<String, String> {
    let code = qrcode::QrCode::new(content.as_bytes()).map_err(|err| err.to_string())?;
    Ok(code
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(240, 240)
        .quiet_zone(true)
        .build())
}

fn qr_setup_status_from_arc(status: &Arc<Mutex<QrSetupStatus>>) -> Result<QrSetupStatus, String> {
    status
        .lock()
        .map(|status| status.clone())
        .map_err(|err| err.to_string())
}

async fn start_feishu_qr_setup(
    app: AppHandle,
    options: ClientOptions,
    state: tauri::State<'_, BridgeState>,
) -> Result<QrSetupStatus, String> {
    let session_id = setup_session_id(&options.platform)?;
    let platform = options.platform.clone();
    let accounts_base = if platform == "lark" {
        ACCOUNTS_LARK_BASE.to_string()
    } else {
        ACCOUNTS_FEISHU_BASE.to_string()
    };
    let client = reqwest::Client::new();

    let init: RegistrationInitResponse =
        registration_call(&client, &accounts_base, "init", &[]).await?;
    if !init.error.is_empty() {
        return Err(format!("{}: {}", init.error, init.error_description));
    }
    if !init.supported_auth_methods.is_empty()
        && !init
            .supported_auth_methods
            .iter()
            .any(|value| value == "client_secret")
    {
        return Err("current onboarding endpoint does not support client_secret".to_string());
    }

    let begin: RegistrationBeginResponse = registration_call(
        &client,
        &accounts_base,
        "begin",
        &[
            ("archetype", "PersonalAgent"),
            ("auth_method", "client_secret"),
            ("request_user_info", "open_id"),
        ],
    )
    .await?;
    if !begin.error.is_empty() {
        return Err(format!("{}: {}", begin.error, begin.error_description));
    }
    if begin.device_code.is_empty() || begin.verification_uri_complete.is_empty() {
        return Err("incomplete onboarding response".to_string());
    }

    let qr_url = begin.verification_uri_complete.clone();
    let status = Arc::new(Mutex::new(QrSetupStatus {
        session_id: session_id.clone(),
        platform: platform.clone(),
        state: "waiting_scan".to_string(),
        message: "请在本窗口扫描二维码，并在飞书/Lark 中完成授权确认。".to_string(),
        user_code: Some(begin.user_code.clone()),
        qr_url: Some(qr_url.clone()),
        qr_svg: qr_svg(&qr_url).ok(),
        output: vec![
            "飞书/Lark 客户端内置扫码绑定已启动。".to_string(),
            format!("User code: {}", begin.user_code),
            format!("URL: {qr_url}"),
        ],
        done: false,
        success: None,
    }));

    state
        .qr_sessions
        .lock()
        .map_err(|err| err.to_string())?
        .insert(session_id, Arc::clone(&status));

    tauri::async_runtime::spawn(poll_feishu_qr_setup(
        app,
        client,
        accounts_base,
        begin.device_code,
        begin.interval.max(5),
        begin.expire_in.max(600),
        options,
        Arc::clone(&status),
    ));

    qr_setup_status_from_arc(&status)
}

fn start_guided_channel_binding(
    app: AppHandle,
    options: ClientOptions,
    state: tauri::State<'_, BridgeState>,
) -> Result<QrSetupStatus, String> {
    if !platform_supports_guided_setup(&options.platform) {
        return Err(format!(
            "{} 不支持配置引导，请直接填写字段并保存。",
            options.platform
        ));
    }

    let binding = channel_binding_status(&app, &options)?;
    let session_id = setup_session_id(&options.platform)?;
    if !binding.ready {
        let status = Arc::new(Mutex::new(QrSetupStatus {
            session_id: session_id.clone(),
            platform: options.platform.clone(),
            state: "missing_fields".to_string(),
            message: binding.message.clone(),
            user_code: None,
            qr_url: binding.setup_url.clone(),
            qr_svg: binding
                .setup_url
                .as_deref()
                .and_then(|url| qr_svg(url).ok()),
            output: vec![binding.message.clone(), binding.next_step.clone()],
            done: true,
            success: Some(false),
        }));
        state
            .qr_sessions
            .lock()
            .map_err(|err| err.to_string())?
            .insert(session_id, Arc::clone(&status));
        return qr_setup_status_from_arc(&status);
    }

    write_config_file(&app, &options)?;
    let message = guided_setup_message(&options.platform);
    let setup_url = platform_setup_url(&options.platform);
    let status = Arc::new(Mutex::new(QrSetupStatus {
        session_id: session_id.clone(),
        platform: options.platform.clone(),
        state: "external_platform".to_string(),
        message: message.to_string(),
        user_code: None,
        qr_url: setup_url.clone(),
        qr_svg: setup_url.as_deref().and_then(|url| qr_svg(url).ok()),
        output: vec![
            message.to_string(),
            binding.next_step.clone(),
            setup_url
                .as_deref()
                .map(|url| format!("平台入口: {url}"))
                .unwrap_or_else(|| "请在对应平台或网关页面完成剩余配置。".to_string()),
        ],
        done: true,
        success: Some(true),
    }));
    state
        .qr_sessions
        .lock()
        .map_err(|err| err.to_string())?
        .insert(session_id, Arc::clone(&status));
    qr_setup_status_from_arc(&status)
}

async fn start_weixin_qr_setup(
    app: AppHandle,
    options: ClientOptions,
    state: tauri::State<'_, BridgeState>,
) -> Result<QrSetupStatus, String> {
    write_config_file(&app, &options)?;
    let session_id = setup_session_id("weixin")?;
    let fields = options.channel_fields.clone().unwrap_or_default();
    let api_base =
        non_empty(fields.get("api_base")).unwrap_or_else(|| DEFAULT_WEIXIN_API_BASE.to_string());
    let bot_type =
        non_empty(fields.get("bot_type")).unwrap_or_else(|| DEFAULT_WEIXIN_BOT_TYPE.to_string());
    let route_tag = non_empty(fields.get("route_tag"));
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(40))
        .build()
        .map_err(|err| err.to_string())?;

    let begin = weixin_fetch_bot_qr(&client, &api_base, &bot_type, route_tag.as_deref()).await?;
    let qr_url = begin.qrcode_img_content.trim().to_string();
    if begin.qrcode.trim().is_empty() {
        return Err("weixin get_bot_qrcode returned empty qrcode".to_string());
    }
    if qr_url.is_empty() {
        return Err("weixin get_bot_qrcode returned empty qrcode_img_content".to_string());
    }

    let status = Arc::new(Mutex::new(QrSetupStatus {
        session_id: session_id.clone(),
        platform: "weixin".to_string(),
        state: "waiting_scan".to_string(),
        message: "请使用微信扫描客户端中的二维码，并在手机上确认登录。".to_string(),
        user_code: None,
        qr_url: Some(qr_url.clone()),
        qr_svg: qr_svg(&qr_url).ok(),
        output: vec![
            "微信 ilink 客户端内置扫码绑定已启动。".to_string(),
            format!("API: {api_base}"),
            format!("URL: {qr_url}"),
        ],
        done: false,
        success: None,
    }));

    state
        .qr_sessions
        .lock()
        .map_err(|err| err.to_string())?
        .insert(session_id, Arc::clone(&status));

    tauri::async_runtime::spawn(poll_weixin_qr_setup(
        app,
        client,
        api_base,
        bot_type,
        route_tag,
        begin.qrcode,
        options,
        Arc::clone(&status),
    ));

    qr_setup_status_from_arc(&status)
}

async fn poll_weixin_qr_setup(
    app: AppHandle,
    client: reqwest::Client,
    api_base: String,
    bot_type: String,
    route_tag: Option<String>,
    mut qr_key: String,
    options: ClientOptions,
    status: Arc<Mutex<QrSetupStatus>>,
) {
    let timeout_at = std::time::Instant::now() + std::time::Duration::from_secs(480);
    let mut refresh_count = 1usize;
    let max_refresh = 3usize;
    let mut poll_count = 0usize;

    while std::time::Instant::now() < timeout_at {
        poll_count += 1;
        let poll = weixin_poll_qr_status(&client, &api_base, &qr_key, route_tag.as_deref()).await;
        let poll = match poll {
            Ok(poll) => poll,
            Err(err) => {
                finish_qr_setup(&status, false, format!("Weixin QR poll failed: {err}"));
                return;
            }
        };
        record_weixin_poll(&status, poll_count, &poll);

        match poll.status.as_str() {
            "" | "wait" => {}
            "scaned" => {
                if let Ok(mut status) = status.lock() {
                    status.state = "waiting_confirm".to_string();
                    status.message = "已扫码，请在手机上确认登录。".to_string();
                }
            }
            "expired" => {
                refresh_count += 1;
                if refresh_count > max_refresh {
                    finish_qr_setup(
                        &status,
                        false,
                        "二维码已多次过期，请重新扫码绑定。".to_string(),
                    );
                    return;
                }
                match weixin_fetch_bot_qr(&client, &api_base, &bot_type, route_tag.as_deref()).await
                {
                    Ok(begin) if !begin.qrcode.trim().is_empty() => {
                        qr_key = begin.qrcode;
                        let qr_url = begin.qrcode_img_content.trim().to_string();
                        if let Ok(mut status) = status.lock() {
                            status.state = "waiting_scan".to_string();
                            status.message = format!(
                                "二维码已过期，已刷新第 {refresh_count}/{max_refresh} 次。"
                            );
                            if !qr_url.is_empty() {
                                status.qr_url = Some(qr_url.clone());
                                status.qr_svg = qr_svg(&qr_url).ok();
                                status.output.push(format!("Refreshed QR URL: {qr_url}"));
                            }
                        }
                    }
                    Ok(_) => {
                        finish_qr_setup(
                            &status,
                            false,
                            "二维码过期后刷新失败：响应缺少 qrcode。".to_string(),
                        );
                        return;
                    }
                    Err(err) => {
                        finish_qr_setup(&status, false, format!("刷新微信二维码失败：{err}"));
                        return;
                    }
                }
            }
            "confirmed" => {
                if poll.bot_token.trim().is_empty() {
                    finish_qr_setup(
                        &status,
                        false,
                        "微信已确认，但响应缺少 bot_token。".to_string(),
                    );
                    return;
                }
                if poll.ilink_bot_id.trim().is_empty() {
                    finish_qr_setup(
                        &status,
                        false,
                        "微信已确认，但响应缺少 ilink_bot_id。".to_string(),
                    );
                    return;
                }

                let mut next_options = options.clone();
                let mut fields = next_options.channel_fields.take().unwrap_or_default();
                fields.insert("token".to_string(), poll.bot_token.trim().to_string());
                fields.insert(
                    "account_id".to_string(),
                    poll.ilink_bot_id.trim().to_string(),
                );
                fields.insert("bot_type".to_string(), bot_type.clone());
                if !poll.baseurl.trim().is_empty() {
                    fields.insert("api_base".to_string(), poll.baseurl.trim().to_string());
                } else {
                    fields.insert("api_base".to_string(), api_base.clone());
                }
                if !poll.ilink_user_id.trim().is_empty()
                    && non_empty(fields.get("allow_from")).is_none()
                {
                    fields.insert(
                        "allow_from".to_string(),
                        poll.ilink_user_id.trim().to_string(),
                    );
                }
                next_options.channel_fields = Some(fields);
                match write_config_file(&app, &next_options) {
                    Ok(path) => finish_qr_setup(
                        &status,
                        true,
                        format!(
                            "微信扫码绑定完成，配置已保存：{}。下一步：从微信给机器人发一条消息，用于缓存 context_token。",
                            path.display()
                        ),
                    ),
                    Err(err) => finish_qr_setup(
                        &status,
                        false,
                        format!("微信扫码成功，但保存配置失败：{err}"),
                    ),
                }
                return;
            }
            other => {
                if let Ok(mut status) = status.lock() {
                    status.message = format!("Weixin QR poll returned `{other}`.");
                }
            }
        }

        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }

    finish_qr_setup(&status, false, "等待微信扫码超时，请重试。".to_string());
}

async fn weixin_fetch_bot_qr(
    client: &reqwest::Client,
    api_base: &str,
    bot_type: &str,
    route_tag: Option<&str>,
) -> Result<WeixinQrBeginResponse, String> {
    let mut url = weixin_url(api_base, &["ilink", "bot", "get_bot_qrcode"])?;
    url.query_pairs_mut().append_pair("bot_type", bot_type);
    let mut request = client.get(url);
    if let Some(route_tag) = route_tag.filter(|value| !value.trim().is_empty()) {
        request = request.header("SKRouteTag", route_tag);
    }
    let response = request.send().await.map_err(|err| err.to_string())?;
    let status = response.status();
    let text = response.text().await.map_err(|err| err.to_string())?;
    if !status.is_success() {
        return Err(format!("get_bot_qrcode HTTP {status}: {text}"));
    }
    serde_json::from_str(&text).map_err(|err| format!("get_bot_qrcode json: {err}; body={text}"))
}

async fn weixin_poll_qr_status(
    client: &reqwest::Client,
    api_base: &str,
    qr_key: &str,
    route_tag: Option<&str>,
) -> Result<WeixinQrPollResponse, String> {
    let mut url = weixin_url(api_base, &["ilink", "bot", "get_qrcode_status"])?;
    url.query_pairs_mut().append_pair("qrcode", qr_key);
    let mut request = client.get(url).header("iLink-App-ClientVersion", "1");
    if let Some(route_tag) = route_tag.filter(|value| !value.trim().is_empty()) {
        request = request.header("SKRouteTag", route_tag);
    }
    let response = match request.send().await {
        Ok(response) => response,
        Err(err) if err.is_timeout() => {
            return Ok(WeixinQrPollResponse {
                status: "wait".to_string(),
                ..Default::default()
            });
        }
        Err(err) => return Err(err.to_string()),
    };
    let status = response.status();
    let text = response.text().await.map_err(|err| err.to_string())?;
    if !status.is_success() {
        return Err(format!("get_qrcode_status HTTP {status}: {text}"));
    }
    serde_json::from_str(&text).map_err(|err| format!("get_qrcode_status json: {err}; body={text}"))
}

fn weixin_url(api_base: &str, path: &[&str]) -> Result<reqwest::Url, String> {
    let mut base = api_base.trim().trim_end_matches('/').to_string();
    if base.is_empty() {
        base = DEFAULT_WEIXIN_API_BASE.to_string();
    }
    base.push('/');
    let mut url = reqwest::Url::parse(&base).map_err(|err| err.to_string())?;
    {
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| format!("invalid weixin api base: {api_base}"))?;
        segments.pop_if_empty();
        for item in path {
            segments.push(item);
        }
    }
    Ok(url)
}

fn record_weixin_poll(
    status: &Arc<Mutex<QrSetupStatus>>,
    poll_count: usize,
    poll: &WeixinQrPollResponse,
) {
    if let Ok(mut status) = status.lock() {
        status.output.push(format!(
            "poll #{poll_count}: status={}",
            if poll.status.is_empty() {
                "wait"
            } else {
                poll.status.as_str()
            }
        ));
        if status.output.len() > 300 {
            status.output.drain(0..100);
        }
    }
}

async fn poll_feishu_qr_setup(
    app: AppHandle,
    client: reqwest::Client,
    mut accounts_base: String,
    device_code: String,
    mut interval: u64,
    expire_in: u64,
    options: ClientOptions,
    status: Arc<Mutex<QrSetupStatus>>,
) {
    let timeout_seconds = expire_in.min(600);
    let timeout_at = std::time::Instant::now() + std::time::Duration::from_secs(timeout_seconds);
    let mut platform_type = options.platform.clone();
    let mut poll_count = 0usize;

    while std::time::Instant::now() < timeout_at {
        poll_count += 1;
        let poll_value = registration_call_value(
            &client,
            &accounts_base,
            "poll",
            &[("device_code", device_code.as_str())],
        )
        .await;

        let poll_value = match poll_value {
            Ok(value) => value,
            Err(err) => {
                finish_qr_setup(&status, false, format!("飞书/Lark 轮询失败：{err}"));
                return;
            }
        };
        record_poll_value(&status, poll_count, &poll_value);

        let poll: RegistrationPollResponse = match serde_json::from_value(poll_value.clone()) {
            Ok(poll) => poll,
            Err(err) => {
                finish_qr_setup(
                    &status,
                    false,
                    format!("QR poll parse failed: {err}; body={poll_value}"),
                );
                return;
            }
        };

        let tenant_brand = poll.user_info.tenant_brand.trim().to_ascii_lowercase();
        if tenant_brand == "lark" && accounts_base != ACCOUNTS_LARK_BASE {
            platform_type = "lark".to_string();
            accounts_base = ACCOUNTS_LARK_BASE.to_string();
            continue;
        }

        if !poll.client_id.is_empty() && !poll.client_secret.is_empty() {
            let mut next_options = options.clone();
            next_options.platform = platform_type.clone();
            let mut fields = next_options.channel_fields.take().unwrap_or_default();
            fields.insert("app_id".to_string(), poll.client_id.clone());
            fields.insert("app_secret".to_string(), poll.client_secret.clone());
            fields.insert("connection_mode".to_string(), "websocket".to_string());
            fields.insert(
                "api_base".to_string(),
                if platform_type == "lark" {
                    OPEN_LARK_BASE
                } else {
                    OPEN_FEISHU_BASE
                }
                .to_string(),
            );
            if !poll.user_info.open_id.is_empty() {
                fields.insert("owner_open_id".to_string(), poll.user_info.open_id);
            }
            next_options.channel_fields = Some(fields);
            match write_config_file(&app, &next_options) {
                Ok(path) => {
                    finish_qr_setup(
                        &status,
                        true,
                        format!("飞书/Lark 扫码绑定完成，配置已保存：{}", path.display()),
                    );
                }
                Err(err) => {
                    finish_qr_setup(
                        &status,
                        false,
                        format!("飞书/Lark 授权成功，但保存配置失败：{err}"),
                    );
                }
            }
            return;
        }

        match poll.error.as_str() {
            "" | "authorization_pending" => {}
            "slow_down" => interval += 5,
            "access_denied" => {
                finish_qr_setup(&status, false, "用户拒绝了飞书/Lark 授权。".to_string());
                return;
            }
            "expired_token" => {
                finish_qr_setup(
                    &status,
                    false,
                    "飞书/Lark 扫码会话已过期，请重新扫码。".to_string(),
                );
                return;
            }
            other => {
                finish_qr_setup(
                    &status,
                    false,
                    format!("{}: {}", other, poll.error_description),
                );
                return;
            }
        }

        tokio::time::sleep(std::time::Duration::from_secs(interval)).await;
    }
    finish_qr_setup(
        &status,
        false,
        "等待飞书/Lark 扫码授权超时，请重试。".to_string(),
    );
}

async fn registration_call<T: for<'de> Deserialize<'de>>(
    client: &reqwest::Client,
    base: &str,
    action: &str,
    params: &[(&str, &str)],
) -> Result<T, String> {
    let value = registration_call_value(client, base, action, params).await?;
    serde_json::from_value(value).map_err(|err| err.to_string())
}

async fn registration_call_value(
    client: &reqwest::Client,
    base: &str,
    action: &str,
    params: &[(&str, &str)],
) -> Result<serde_json::Value, String> {
    let mut form = vec![("action", action)];
    form.extend(params.iter().copied());
    let response = client
        .post(format!("{base}/oauth/v1/app/registration"))
        .form(&form)
        .send()
        .await
        .map_err(|err| err.to_string())?;
    let status = response.status();
    let text = response.text().await.map_err(|err| err.to_string())?;
    serde_json::from_str(&text).map_err(|err| format!("HTTP {status}: {err}; body={text}"))
}

fn finish_qr_setup(status: &Arc<Mutex<QrSetupStatus>>, success: bool, message: String) {
    if let Ok(mut status) = status.lock() {
        status.state = if success { "completed" } else { "failed" }.to_string();
        status.message = message.clone();
        status.output.push(message);
        status.done = true;
        status.success = Some(success);
    }
}

fn record_poll_value(
    status: &Arc<Mutex<QrSetupStatus>>,
    poll_count: usize,
    value: &serde_json::Value,
) {
    if let Ok(mut status) = status.lock() {
        let error = value
            .get("error")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let label = if error.is_empty() { "ok" } else { error };
        status.state = match label {
            "authorization_pending" => "waiting_scan",
            "slow_down" => "waiting_scan",
            _ => status.state.as_str(),
        }
        .to_string();
        status.message = match label {
            "authorization_pending" => "等待在飞书/Lark 中扫码并确认授权。".to_string(),
            "slow_down" => "飞书/Lark 要求降低轮询频率，请稍候。".to_string(),
            "ok" => "飞书/Lark 授权成功，正在写入配置…".to_string(),
            other => format!("Feishu/Lark poll returned {other}."),
        };
        status
            .output
            .push(format!("poll #{poll_count}: {}", compact_json(value)));
        if status.output.len() > 300 {
            status.output.drain(0..100);
        }
    }
}

fn compact_json(value: &serde_json::Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| value.to_string())
}

fn setup_session_id(platform: &str) -> Result<String, String> {
    Ok(format!(
        "{}-{}",
        platform,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|err| err.to_string())?
            .as_millis()
    ))
}

fn platform_supports_auto_qr(platform: &str) -> bool {
    matches!(platform, "feishu" | "lark" | "weixin")
}

fn platform_supports_guided_setup(platform: &str) -> bool {
    matches!(platform, "qq" | "dingtalk" | "wecom")
}

fn platform_supports_scan(platform: &str) -> bool {
    platform_supports_auto_qr(platform) || platform_supports_guided_setup(platform)
}

fn platform_setup_url(platform: &str) -> Option<String> {
    match platform {
        "dingtalk" => Some("https://open-dev.dingtalk.com/".to_string()),
        "wecom" => Some("https://work.weixin.qq.com/wework_admin/frame#apps".to_string()),
        _ => None,
    }
}

fn required_field_names(platform: &str) -> &'static [&'static str] {
    match platform {
        "feishu" | "lark" => &["app_id", "app_secret"],
        "dingtalk" => &["client_id", "client_secret", "robot_code"],
        "telegram" | "discord" | "line" | "max" | "weixin" => &["token"],
        "slack" => &["bot_token", "app_token"],
        "qq" => &["ws_url"],
        "wecom" => &["corp_id", "corp_secret", "agent_id"],
        "qqbot" | "weibo" => &["app_id", "app_secret"],
        "http" | "bridge" => &["listen"],
        _ => &[],
    }
}

fn missing_required_fields(platform: &str, fields: &HashMap<String, String>) -> Vec<String> {
    if matches!(platform, "qqbot" | "weibo") && non_empty(fields.get("token")).is_some() {
        return Vec::new();
    }
    required_field_names(platform)
        .iter()
        .filter(|key| non_empty(fields.get(**key)).is_none())
        .map(|key| (*key).to_string())
        .collect()
}

fn required_fields_ready(platform: &str, fields: &HashMap<String, String>) -> bool {
    missing_required_fields(platform, fields).is_empty()
}

fn channel_binding_status(
    app: &AppHandle,
    options: &ClientOptions,
) -> Result<ChannelBindingStatus, String> {
    let platform = options.platform.trim().to_lowercase();
    let fields = resolved_channel_fields(app, options)?;
    let missing_fields = missing_required_fields(&platform, &fields);
    let ready = missing_fields.is_empty();
    let setup_url = platform_setup_url(&platform);
    let next_step = guided_next_step(&platform);
    let (state, message) = if ready {
        (
            "ready",
            "必填字段已齐全，可以保存 Channel 配置。".to_string(),
        )
    } else if platform_supports_auto_qr(&platform) {
        (
            "scan_available",
            format!(
                "缺少字段：{}。可先扫码绑定自动写入。",
                missing_fields.join(", ")
            ),
        )
    } else if platform_supports_guided_setup(&platform) {
        (
            "guided_setup",
            format!(
                "缺少字段：{}。请先在平台或网关完成配置，再回到这里填写。",
                missing_fields.join(", ")
            ),
        )
    } else {
        (
            "missing_credentials",
            format!("缺少字段：{}。", missing_fields.join(", ")),
        )
    };
    Ok(ChannelBindingStatus {
        platform: platform.clone(),
        state: state.to_string(),
        message,
        ready,
        missing_fields,
        setup_url,
        next_step,
    })
}

fn guided_next_step(platform: &str) -> String {
    match platform {
        "qq" => {
            "在 NapCat/LLOneBot 等 OneBot 网关中扫码登录 QQ；确认 ws_url 可连接后保存配置并启动 Bridge。"
                .to_string()
        }
        "dingtalk" => {
            "在钉钉开放平台创建应用并开通机器人，填写 Client ID、Client Secret 和 Robot Code；管理后台二维码不是自动授权码。"
                .to_string()
        }
        "wecom" => {
            "在企业微信管理后台创建应用或智能机器人，填写 corp_id、corp_secret、agent_id 和回调参数。"
                .to_string()
        }
        _ => "填写完整字段后保存 Channel 配置。".to_string(),
    }
}

fn guided_setup_message(platform: &str) -> &'static str {
    match platform {
        "qq" => "QQ 网关配置已保存。请在 NapCat/LLOneBot 等网关中扫码登录，不要在本客户端等待自动授权。",
        "dingtalk" => {
            "钉钉配置已保存。请在钉钉开放平台核对应用与机器人信息；平台管理页二维码需手动完成应用配置。"
        }
        "wecom" => "企业微信配置已保存。请在管理后台完成应用/机器人与回调配置，再启动 Bridge。",
        _ => "Channel 配置已保存，请按平台说明完成剩余步骤。",
    }
}

fn status_text(ready: bool) -> String {
    if ready { "configured" } else { "missing" }.to_string()
}

fn binding_status_text(platform: &str, ready: bool) -> String {
    if ready {
        "ready".to_string()
    } else if platform_supports_auto_qr(platform) {
        "scan available".to_string()
    } else if platform_supports_guided_setup(platform) {
        "guided setup".to_string()
    } else {
        "missing credentials".to_string()
    }
}

fn agent_install_status_text(agent_type: &str, installed: bool) -> String {
    if agent_type == "mock" {
        "built-in".to_string()
    } else if installed {
        "installed".to_string()
    } else {
        "not found".to_string()
    }
}

fn build_cache_busting_updater(app: &AppHandle) -> Result<tauri_plugin_updater::Updater, String> {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| err.to_string())?
        .as_millis();
    let nonce = rand::random::<u64>();
    let separator = if DESKTOP_UPDATER_ENDPOINT.contains('?') {
        '&'
    } else {
        '?'
    };
    let endpoint = format!("{DESKTOP_UPDATER_ENDPOINT}{separator}ts={ts}&nonce={nonce}");
    let endpoint = reqwest::Url::parse(&endpoint).map_err(|err| err.to_string())?;

    let builder = app
        .updater_builder()
        .endpoints(vec![endpoint])
        .map_err(|err| err.to_string())?
        .header("Cache-Control", "no-cache, no-store, max-age=0")
        .map_err(|err| err.to_string())?
        .header("Pragma", "no-cache")
        .map_err(|err| err.to_string())?
        .header("Expires", "0")
        .map_err(|err| err.to_string())?;

    builder.build().map_err(|err| err.to_string())
}

fn available_update_payload(update: &tauri_plugin_updater::Update) -> AvailableUpdatePayload {
    AvailableUpdatePayload {
        version: update.version.clone(),
        date: update.date.map(|date| date.to_string()),
        body: update.body.clone(),
    }
}

#[allow(clippy::too_many_arguments)]
fn status_summary(
    exe_path: &str,
    config_path: &str,
    exe_exists: bool,
    config_exists: bool,
    project_configured: bool,
    channel_configured: bool,
    agent_configured: bool,
    binding_ready: bool,
    agent_installed: bool,
    project: &str,
    platform: &str,
    agent_type: &str,
) -> String {
    if exe_path.trim().is_empty() {
        return "请先选择 AgentLink CLI 的本机路径。".to_string();
    }
    if config_path.trim().is_empty() {
        return "请先选择或生成配置文件。".to_string();
    }
    if !exe_exists {
        return "AgentLink 可执行文件不存在，请先确认路径。".to_string();
    }
    if !config_exists {
        return "配置文件还没有生成，请先保存配置。".to_string();
    }
    if !project_configured {
        return format!("配置文件里还没有 project `{project}`。");
    }
    if !channel_configured {
        return format!("project `{project}` 里还没有 channel `{platform}`。");
    }
    if !agent_configured {
        return format!("project `{project}` 里还没有 agent `{agent_type}`。");
    }
    if !agent_installed {
        return format!("没有找到 agent `{agent_type}` 的命令，请检查安装或命令路径。");
    }
    if !binding_ready {
        return format!("channel `{platform}` 还需要补充密钥或完成网关绑定。");
    }
    format!("`{platform}` 和 `{agent_type}` 已就绪，可以启动。")
}

fn agent_is_available(agent_type: &str, command: &Option<String>) -> bool {
    if agent_type == "mock" {
        return true;
    }
    let command = command
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| default_agent_command(agent_type));
    if command.is_empty() {
        return false;
    }

    let path = Path::new(command);
    if path.components().count() > 1 || command.contains('\\') || command.contains('/') {
        return resolve_existing_path(command).is_some();
    }

    command_exists(command)
}

fn default_agent_command(agent_type: &str) -> &str {
    match agent_type {
        "codex" => "codex",
        "claude-code" => "claude",
        "gemini" => "gemini",
        "cursor" => "agent",
        "kimi" => "kimi",
        "qoder" => "qodercli",
        "opencode" => "opencode",
        "iflow" => "iflow",
        "devin" => "devin",
        "acp" => "acp-agent",
        _ => "",
    }
}

fn command_exists(command: &str) -> bool {
    let probe = if cfg!(windows) {
        let mut command_builder = Command::new("where");
        command_builder.arg(command);
        #[cfg(windows)]
        command_builder.creation_flags(CREATE_NO_WINDOW);
        command_builder.output()
    } else {
        Command::new("which").arg(command).output()
    };
    probe.map(|output| output.status.success()).unwrap_or(false)
}

fn validate_config_local(app: &AppHandle, options: &ClientOptions) -> Result<String, String> {
    let config = resolve_config_read_path(app, &options.config_path)?;
    let raw = std::fs::read_to_string(&config)
        .map_err(|err| format!("failed to read config `{}`: {err}", config.display()))?;
    let parsed = raw
        .parse::<toml::Value>()
        .map_err(|err| format!("invalid TOML `{}`: {err}", config.display()))?;
    let project = find_project(&parsed, &options.project)
        .ok_or_else(|| format!("project `{}` not found in config", options.project))?;
    if !project_has_platform(project, &options.platform) {
        return Err(format!(
            "channel `{}` not found in project `{}`",
            options.platform, options.project
        ));
    }
    let agent_type = opt_or(&options.agent_type, "codex");
    let configured_agent = project
        .get("agent")
        .and_then(toml::Value::as_table)
        .and_then(|agent| agent.get("type"))
        .and_then(toml::Value::as_str)
        .unwrap_or("");
    if configured_agent != agent_type {
        return Err(format!(
            "agent `{agent_type}` is not configured in project `{}`",
            options.project
        ));
    }
    Ok(format!(
        "local config ok: project `{}`, channel `{}`, agent `{}`",
        options.project, options.platform, agent_type
    ))
}

fn field_or_extra(
    options: &ClientOptions,
    fields: &HashMap<String, String>,
    key: &str,
) -> Option<String> {
    non_empty(fields.get(key)).or_else(|| {
        options
            .extra
            .as_ref()
            .filter(|v| !v.trim().is_empty())
            .cloned()
    })
}

fn non_empty(value: Option<&String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn opt_or<'a>(value: &'a Option<String>, default: &'a str) -> &'a str {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(default)
}

fn push_optional(out: &mut String, key: &str, value: Option<&str>) {
    if let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) {
        out.push_str(&format!("{key} = {}\n", toml_string(value)));
    }
}

fn lines(value: &str) -> Vec<String> {
    value
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

fn parse_bool(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn toml_string(value: &str) -> String {
    let escaped = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t");
    format!("\"{escaped}\"")
}

fn run_capture_owned(exe: &str, args: Vec<String>) -> Result<String, String> {
    let mut command = Command::new(exe);
    command.args(args);
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    let output = command.output().map_err(|err| err.to_string())?;
    output_text(output)
}

fn output_text(output: std::process::Output) -> Result<String, String> {
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if output.status.success() {
        Ok(if stdout.is_empty() { stderr } else { stdout })
    } else {
        Err(if stderr.is_empty() { stdout } else { stderr })
    }
}

fn open_client_db(app: &tauri::AppHandle) -> Result<rusqlite::Connection, String> {
    let path = client_db_path(app)?;
    let db = rusqlite::Connection::open(path).map_err(|err| err.to_string())?;
    db.execute_batch(
        "create table if not exists settings(
            key text primary key,
            value text not null
        );
        create table if not exists connections(
            id text primary key,
            data text not null,
            updated_at integer not null
        );
        create table if not exists channel_configs(
            connection_id text not null,
            platform text not null,
            fields_json text not null,
            updated_at integer not null default (strftime('%s','now')),
            primary key (connection_id, platform)
        );
        create table if not exists agent_configs(
            connection_id text not null,
            agent_type text not null,
            fields_json text not null,
            updated_at integer not null default (strftime('%s','now')),
            primary key (connection_id, agent_type)
        );
        create table if not exists connection_settings(
            connection_id text primary key,
            name text not null,
            exe_path text not null,
            config_path text not null,
            project text not null,
            work_dir text not null,
            selected_channel text not null,
            selected_agent text not null,
            updated_at integer not null default (strftime('%s','now'))
        );",
    )
    .map_err(|err| err.to_string())?;
    Ok(db)
}

fn upsert_channel_config(
    db: &rusqlite::Connection,
    connection_id: &str,
    platform: &str,
    fields: &HashMap<String, String>,
) -> Result<(), String> {
    let fields_json = serde_json::to_string(fields).map_err(|err| err.to_string())?;
    db.execute(
        "insert into channel_configs(connection_id, platform, fields_json, updated_at)
         values(?1, ?2, ?3, strftime('%s','now'))
         on conflict(connection_id, platform) do update set
            fields_json = excluded.fields_json,
            updated_at = excluded.updated_at",
        (connection_id, platform, fields_json),
    )
    .map_err(|err| err.to_string())?;
    Ok(())
}

fn load_channel_config(
    db: &rusqlite::Connection,
    connection_id: &str,
    platform: &str,
) -> Result<HashMap<String, String>, String> {
    let raw: Option<String> = db
        .query_row(
            "select fields_json from channel_configs where connection_id = ?1 and platform = ?2",
            (connection_id, platform),
            |row| row.get(0),
        )
        .ok();
    Ok(raw
        .and_then(|value| serde_json::from_str(&value).ok())
        .unwrap_or_default())
}

fn load_all_channel_configs(
    db: &rusqlite::Connection,
    connection_id: &str,
) -> Result<HashMap<String, HashMap<String, String>>, String> {
    let mut stmt = db
        .prepare("select platform, fields_json from channel_configs where connection_id = ?1")
        .map_err(|err| err.to_string())?;
    let rows = stmt
        .query_map([connection_id], |row| {
            let platform: String = row.get(0)?;
            let fields_json: String = row.get(1)?;
            Ok((platform, fields_json))
        })
        .map_err(|err| err.to_string())?;
    let mut out = HashMap::new();
    for row in rows {
        let (platform, fields_json) = row.map_err(|err| err.to_string())?;
        if let Ok(fields) = serde_json::from_str::<HashMap<String, String>>(&fields_json) {
            out.insert(platform, fields);
        }
    }
    Ok(out)
}

fn sync_channel_configs_from_connections(
    db: &rusqlite::Connection,
    connections: &[ClientConnectionState],
) -> Result<(), String> {
    for connection in connections {
        for (platform, fields) in &connection.channel_fields {
            upsert_channel_config(db, &connection.id, platform, fields)?;
        }
    }
    Ok(())
}

fn merge_stored_fields(target: &mut HashMap<String, String>, stored: HashMap<String, String>) {
    for (key, value) in stored {
        if !value.trim().is_empty() {
            target.insert(key, value);
        }
    }
}

fn upsert_connection_settings(
    db: &rusqlite::Connection,
    connection: &ClientConnectionState,
) -> Result<(), String> {
    db.execute(
        "insert into connection_settings(
            connection_id, name, exe_path, config_path, project, work_dir,
            selected_channel, selected_agent, updated_at
         ) values(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, strftime('%s','now'))
         on conflict(connection_id) do update set
            name = excluded.name,
            exe_path = excluded.exe_path,
            config_path = excluded.config_path,
            project = excluded.project,
            work_dir = excluded.work_dir,
            selected_channel = excluded.selected_channel,
            selected_agent = excluded.selected_agent,
            updated_at = excluded.updated_at",
        (
            &connection.id,
            &connection.name,
            &connection.exe_path,
            &connection.config_path,
            &connection.project,
            &connection.work_dir,
            &connection.selected_channel,
            &connection.selected_agent,
        ),
    )
    .map_err(|err| err.to_string())?;
    Ok(())
}

fn load_connection_settings(
    db: &rusqlite::Connection,
    connection_id: &str,
) -> Result<Option<ClientConnectionState>, String> {
    let row = db.query_row(
        "select name, exe_path, config_path, project, work_dir, selected_channel, selected_agent
         from connection_settings where connection_id = ?1",
        [connection_id],
        |row| {
            Ok(ClientConnectionState {
                id: connection_id.to_string(),
                name: row.get(0)?,
                exe_path: row.get(1)?,
                config_path: row.get(2)?,
                project: row.get(3)?,
                work_dir: row.get(4)?,
                selected_channel: row.get(5)?,
                selected_agent: row.get(6)?,
                operation_mode: None,
                channel_fields: HashMap::new(),
                agent_fields: HashMap::new(),
                channel_targets: HashMap::new(),
                active_target_ids: HashMap::new(),
                test: serde_json::Value::Null,
            })
        },
    );
    match row {
        Ok(value) => Ok(Some(value)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(err) => Err(err.to_string()),
    }
}

fn sync_connection_settings_from_connections(
    db: &rusqlite::Connection,
    connections: &[ClientConnectionState],
) -> Result<(), String> {
    for connection in connections {
        upsert_connection_settings(db, connection)?;
    }
    Ok(())
}

fn upsert_agent_config(
    db: &rusqlite::Connection,
    connection_id: &str,
    agent_type: &str,
    fields: &HashMap<String, String>,
) -> Result<(), String> {
    let fields_json = serde_json::to_string(fields).map_err(|err| err.to_string())?;
    db.execute(
        "insert into agent_configs(connection_id, agent_type, fields_json, updated_at)
         values(?1, ?2, ?3, strftime('%s','now'))
         on conflict(connection_id, agent_type) do update set
            fields_json = excluded.fields_json,
            updated_at = excluded.updated_at",
        (connection_id, agent_type, fields_json),
    )
    .map_err(|err| err.to_string())?;
    Ok(())
}

fn load_agent_config(
    db: &rusqlite::Connection,
    connection_id: &str,
    agent_type: &str,
) -> Result<HashMap<String, String>, String> {
    let raw: Option<String> = db
        .query_row(
            "select fields_json from agent_configs where connection_id = ?1 and agent_type = ?2",
            (connection_id, agent_type),
            |row| row.get(0),
        )
        .ok();
    Ok(raw
        .and_then(|value| serde_json::from_str(&value).ok())
        .unwrap_or_default())
}

fn load_all_agent_configs(
    db: &rusqlite::Connection,
    connection_id: &str,
) -> Result<HashMap<String, HashMap<String, String>>, String> {
    let mut stmt = db
        .prepare("select agent_type, fields_json from agent_configs where connection_id = ?1")
        .map_err(|err| err.to_string())?;
    let rows = stmt
        .query_map([connection_id], |row| {
            let agent_type: String = row.get(0)?;
            let fields_json: String = row.get(1)?;
            Ok((agent_type, fields_json))
        })
        .map_err(|err| err.to_string())?;
    let mut out = HashMap::new();
    for row in rows {
        let (agent_type, fields_json) = row.map_err(|err| err.to_string())?;
        if let Ok(fields) = serde_json::from_str::<HashMap<String, String>>(&fields_json) {
            out.insert(agent_type, fields);
        }
    }
    Ok(out)
}

fn sync_agent_configs_from_connections(
    db: &rusqlite::Connection,
    connections: &[ClientConnectionState],
) -> Result<(), String> {
    for connection in connections {
        for (agent_type, fields) in &connection.agent_fields {
            upsert_agent_config(db, &connection.id, agent_type, fields)?;
        }
    }
    Ok(())
}

fn overlay_connections_from_sqlite(
    db: &rusqlite::Connection,
    connections: &mut [ClientConnectionState],
) -> Result<(), String> {
    for connection in connections.iter_mut() {
        if let Some(settings) = load_connection_settings(db, &connection.id)? {
            if !settings.name.trim().is_empty() {
                connection.name = settings.name;
            }
            if !settings.exe_path.trim().is_empty() {
                connection.exe_path = settings.exe_path;
            }
            if !settings.config_path.trim().is_empty() {
                connection.config_path = settings.config_path;
            }
            if !settings.project.trim().is_empty() {
                connection.project = settings.project;
            }
            if !settings.work_dir.trim().is_empty() {
                connection.work_dir = settings.work_dir;
            }
            if !settings.selected_channel.trim().is_empty() {
                connection.selected_channel = settings.selected_channel;
            }
            if !settings.selected_agent.trim().is_empty() {
                connection.selected_agent = settings.selected_agent;
            }
        }

        let stored_channels = load_all_channel_configs(db, &connection.id)?;
        for (platform, fields) in stored_channels {
            let entry = connection
                .channel_fields
                .entry(platform)
                .or_insert_with(HashMap::new);
            merge_stored_fields(entry, fields);
        }

        let stored_agents = load_all_agent_configs(db, &connection.id)?;
        for (agent_type, fields) in stored_agents {
            let entry = connection
                .agent_fields
                .entry(agent_type)
                .or_insert_with(HashMap::new);
            merge_stored_fields(entry, fields);
        }
    }
    Ok(())
}

fn channel_fields_from_toml(app: &AppHandle, options: &ClientOptions) -> HashMap<String, String> {
    let Ok(config_path) = resolve_config_read_path(app, &options.config_path) else {
        return HashMap::new();
    };
    let Ok(raw) = std::fs::read_to_string(config_path) else {
        return HashMap::new();
    };
    let Ok(parsed) = raw.parse::<toml::Value>() else {
        return HashMap::new();
    };
    let Some(project) = find_project(&parsed, &options.project) else {
        return HashMap::new();
    };
    platform_options(project, &options.platform).unwrap_or_default()
}

fn resolved_channel_fields(
    app: &AppHandle,
    options: &ClientOptions,
) -> Result<HashMap<String, String>, String> {
    let platform = options.platform.trim().to_lowercase();
    let mut fields = channel_fields_from_toml(app, options);
    fields.extend(options.channel_fields.clone().unwrap_or_default());
    if let Some(connection_id) = options
        .connection_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let db = open_client_db(app)?;
        let stored = load_channel_config(&db, connection_id, &platform)?;
        fields.extend(stored);
    }
    Ok(fields)
}

fn persist_channel_fields(app: &AppHandle, options: &ClientOptions) -> Result<(), String> {
    let Some(connection_id) = options
        .connection_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(());
    };
    let fields = resolved_channel_fields(app, options)?;
    let db = open_client_db(app)?;
    upsert_channel_config(
        &db,
        connection_id,
        &options.platform.trim().to_lowercase(),
        &fields,
    )
}

fn agent_fields_from_toml(
    app: &AppHandle,
    options: &ClientOptions,
    agent_type: &str,
) -> HashMap<String, String> {
    let Ok(config_path) = resolve_config_read_path(app, &options.config_path) else {
        return HashMap::new();
    };
    let Ok(raw) = std::fs::read_to_string(config_path) else {
        return HashMap::new();
    };
    let Ok(parsed) = raw.parse::<toml::Value>() else {
        return HashMap::new();
    };
    let Some(project) = find_project(&parsed, &options.project) else {
        return HashMap::new();
    };
    let Some(agent) = project.get("agent").and_then(toml::Value::as_table) else {
        return HashMap::new();
    };
    if agent
        .get("type")
        .and_then(toml::Value::as_str)
        .unwrap_or("codex")
        != agent_type
    {
        return HashMap::new();
    }
    let Some(agent_options) = agent.get("options").and_then(toml::Value::as_table) else {
        return HashMap::new();
    };
    let mut fields = HashMap::new();
    for (key, value) in agent_options {
        fields.insert(
            agent_option_key(agent_type, key),
            toml_value_to_string(value),
        );
    }
    fields
}

fn options_agent_fields(options: &ClientOptions) -> HashMap<String, String> {
    let mut fields = HashMap::new();
    insert_nonempty_field(&mut fields, "backend", options.agent_backend.as_deref());
    insert_nonempty_field(&mut fields, "command", options.agent_command.as_deref());
    insert_nonempty_field(&mut fields, "model", options.agent_model.as_deref());
    insert_nonempty_field(&mut fields, "mode", options.agent_mode.as_deref());
    insert_nonempty_field(
        &mut fields,
        "reasoningEffort",
        options.reasoning_effort.as_deref(),
    );
    insert_nonempty_field(&mut fields, "args", options.agent_args.as_deref());
    insert_nonempty_field(
        &mut fields,
        "workDir",
        Some(options.work_dir.as_str()).filter(|value| !value.trim().is_empty()),
    );
    fields
}

fn insert_nonempty_field(fields: &mut HashMap<String, String>, key: &str, value: Option<&str>) {
    if let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) {
        fields.insert(key.to_string(), value.to_string());
    }
}

fn resolved_agent_fields(
    app: &AppHandle,
    options: &ClientOptions,
) -> Result<HashMap<String, String>, String> {
    let agent_type = opt_or(&options.agent_type, "codex");
    let mut fields = agent_fields_from_toml(app, options, agent_type);
    fields.extend(options_agent_fields(options));
    if let Some(connection_id) = options
        .connection_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let db = open_client_db(app)?;
        let stored = load_agent_config(&db, connection_id, agent_type)?;
        fields.extend(stored);
    }
    Ok(fields)
}

fn persist_agent_fields(app: &AppHandle, options: &ClientOptions) -> Result<(), String> {
    let Some(connection_id) = options
        .connection_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(());
    };
    let agent_type = opt_or(&options.agent_type, "codex");
    let fields = resolved_agent_fields(app, options)?;
    let db = open_client_db(app)?;
    upsert_agent_config(&db, connection_id, agent_type, &fields)
}

fn client_db_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(|err| err.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|err| err.to_string())?;
    Ok(dir.join("client.sqlite3"))
}

fn tail_text_file(path: &Path, max_lines: usize) -> Result<String, String> {
    if !path.exists() {
        return Ok(String::new());
    }
    let raw = std::fs::read_to_string(path).map_err(|err| err.to_string())?;
    let clean = strip_ansi_sequences(&raw);
    let mut lines = clean.lines().rev().take(max_lines).collect::<Vec<_>>();
    lines.reverse();
    Ok(lines.join("\n"))
}

fn strip_ansi_sequences(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\u{1b}' {
            out.push(ch);
            continue;
        }
        if chars.peek() == Some(&'[') {
            chars.next();
            for next in chars.by_ref() {
                if ('@'..='~').contains(&next) {
                    break;
                }
            }
        }
    }
    out
}

fn resolve_exe_path(input: &str) -> Result<String, String> {
    resolve_existing_path(input)
        .and_then(|path| path.to_str().map(str::to_string))
        .ok_or_else(|| format!("executable not found: {input}"))
}

fn resolve_existing_path(input: &str) -> Option<PathBuf> {
    for variant in path_lookup_variants(input) {
        if let Some(found) = candidate_paths(&variant)
            .into_iter()
            .find(|path| path.exists())
        {
            return found.canonicalize().ok().or(Some(found));
        }
    }
    None
}

fn resolve_output_path(input: &str) -> PathBuf {
    if let Some(existing) = resolve_existing_path(input) {
        return existing;
    }
    candidate_paths(input)
        .into_iter()
        .find(|path| path.parent().is_some_and(Path::exists))
        .unwrap_or_else(|| PathBuf::from(input))
}

fn user_config_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(|err| err.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|err| err.to_string())?;
    Ok(dir.join("agentlink.toml"))
}

fn is_packaged_readonly_path(path: &Path) -> bool {
    let text = path.to_string_lossy();
    text.contains(".app/Contents/")
        || text.contains(".app\\Contents\\")
        || (text.contains("/Resources/") && text.contains("resources-generated"))
}

fn directory_is_writable(dir: &Path) -> bool {
    if dir.as_os_str().is_empty() {
        return false;
    }
    if !dir.exists() {
        return std::fs::create_dir_all(dir).is_ok();
    }
    let probe = dir.join(format!(".agentlink-write-{}", std::process::id()));
    match std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&probe)
    {
        Ok(_) => {
            let _ = std::fs::remove_file(probe);
            true
        }
        Err(_) => false,
    }
}

fn path_is_writable(path: &Path) -> bool {
    if is_packaged_readonly_path(path) {
        return false;
    }
    let dir = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent().unwrap_or(path).to_path_buf()
    };
    directory_is_writable(&dir)
}

fn seed_user_config_from(template: Option<&Path>, user_path: &Path) -> Result<PathBuf, String> {
    if let Some(parent) = user_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
    }
    if !user_path.exists() {
        if let Some(template) = template.filter(|path| path.is_file()) {
            std::fs::copy(template, user_path).map_err(|err| err.to_string())?;
        }
    }
    Ok(user_path.to_path_buf())
}

fn dev_repo_writable_config_path(path: &Path) -> bool {
    if is_packaged_readonly_path(path) {
        return false;
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let Some(repo_root) = manifest_repo_root(&manifest) else {
        return false;
    };
    path.starts_with(&repo_root) && path_is_writable(path)
}

fn resolve_config_write_path(app: &AppHandle, input: &str) -> Result<PathBuf, String> {
    let trimmed = input.trim();
    if !trimmed.is_empty() {
        let resolved =
            resolve_existing_path(trimmed).unwrap_or_else(|| resolve_output_path(trimmed));
        if dev_repo_writable_config_path(&resolved) {
            return Ok(resolved);
        }
    }

    let user_path = user_config_path(app)?;
    let template = if trimmed.is_empty() {
        None
    } else {
        let resolved =
            resolve_existing_path(trimmed).unwrap_or_else(|| resolve_output_path(trimmed));
        resolved.is_file().then_some(resolved)
    };
    seed_user_config_from(template.as_deref(), &user_path)
}

fn resolve_config_read_path(app: &AppHandle, input: &str) -> Result<PathBuf, String> {
    let user_path = user_config_path(app)?;
    if user_path.is_file() {
        return Ok(user_path);
    }

    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("请先选择或生成配置文件路径。".to_string());
    }

    let resolved = resolve_existing_path(trimmed)
        .filter(|path| path.is_file())
        .ok_or_else(|| format!("config file not found: {trimmed}"))?;
    Ok(resolved)
}

fn normalize_input_path(input: &str) -> String {
    let trimmed = input.trim();
    if cfg!(windows) {
        trimmed.to_string()
    } else {
        trimmed.replace('\\', "/")
    }
}

fn path_lookup_variants(input: &str) -> Vec<String> {
    let trimmed = normalize_input_path(input);
    if trimmed.is_empty() {
        return standard_agentlink_exe_rel_paths()
            .into_iter()
            .filter_map(|path| path.to_str().map(str::to_string))
            .collect();
    }

    let mut variants = vec![trimmed.to_string()];
    let path = Path::new(&trimmed);
    if is_agentlink_exe_name(path.file_name().and_then(|name| name.to_str())) {
        for name in agentlink_exe_file_names() {
            if let Some(candidate) = replace_path_file_name(path, name) {
                push_unique_string(&mut variants, candidate);
            }
        }
        for relative in standard_agentlink_exe_rel_paths() {
            if let Some(text) = relative.to_str() {
                push_unique_string(&mut variants, text.to_string());
            }
        }
    }

    variants
}

fn is_agentlink_exe_name(name: Option<&str>) -> bool {
    matches!(name, Some("agentlink") | Some("agentlink.exe"))
}

fn agentlink_exe_file_names() -> &'static [&'static str] {
    if cfg!(windows) {
        &["agentlink.exe", "agentlink"]
    } else {
        &["agentlink", "agentlink.exe"]
    }
}

fn standard_agentlink_exe_rel_paths() -> Vec<PathBuf> {
    ["release", "debug"]
        .into_iter()
        .flat_map(|profile| {
            agentlink_exe_file_names()
                .iter()
                .map(move |name| Path::new("target").join(profile).join(name))
        })
        .collect()
}

fn standard_dev_config_rel_path() -> &'static str {
    "examples/agentlink.all.toml"
}

fn replace_path_file_name(path: &Path, file_name: &str) -> Option<String> {
    let parent = path.parent()?;
    Some(parent.join(file_name).to_string_lossy().to_string())
}

fn push_unique_string(values: &mut Vec<String>, value: String) {
    if !values.iter().any(|item| item == &value) {
        values.push(value);
    }
}

fn candidate_paths(input: &str) -> Vec<PathBuf> {
    let input_path = PathBuf::from(normalize_input_path(input));
    if input_path.is_absolute() {
        return vec![input_path];
    }

    let mut paths = vec![input_path.clone()];
    let dist_suffix = suffix_from_component(&input_path, "dist");

    for root in candidate_roots() {
        let mut current = root;
        loop {
            push_candidate(&mut paths, current.join(&input_path));
            if let Some(suffix) = dist_suffix.as_ref() {
                push_candidate(&mut paths, current.join(suffix));
            }
            if !current.pop() {
                break;
            }
        }
    }

    paths
}

fn candidate_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();

    if let Ok(current) = std::env::current_dir() {
        push_candidate(&mut roots, current);
    }

    if let Ok(exe) = std::env::current_exe() {
        push_exe_related_roots(&mut roots, &exe);
    }

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    push_candidate(&mut roots, manifest_dir.clone());
    if let Some(repo_root) = manifest_repo_root(&manifest_dir) {
        push_candidate(&mut roots, repo_root);
    }

    roots
}

fn push_exe_related_roots(roots: &mut Vec<PathBuf>, exe: &Path) {
    let Some(parent) = exe.parent() else {
        return;
    };

    push_candidate(roots, parent.to_path_buf());
    push_candidate(roots, parent.join("resources"));
    push_candidate(roots, parent.join("Resources"));

    if let Some(bundle_contents) = parent.parent() {
        push_candidate(roots, bundle_contents.join("resources"));
        push_candidate(roots, bundle_contents.join("Resources"));
    }
}

fn manifest_repo_root(manifest_dir: &Path) -> Option<PathBuf> {
    manifest_dir
        .ancestors()
        .find(|dir| {
            dir.join("Cargo.toml").is_file()
                && (dir.join("client").join("agentlink-desktop").is_dir()
                    || dir.join("client/agentlink-desktop").is_dir())
        })
        .map(Path::to_path_buf)
}

fn prefer_repo_relative_path(path: &Path) -> String {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    if let Some(repo_root) = manifest_repo_root(&manifest) {
        if let Ok(relative) = path.strip_prefix(&repo_root) {
            return normalize_display_path(relative);
        }
    }
    normalize_display_path(path)
}

fn normalize_display_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DevDefaultPaths {
    exe_path: String,
    config_path: String,
    work_dir: String,
}

fn bundled_agentlink_exe_path(app: &AppHandle) -> Option<PathBuf> {
    let resource_dir = app.path().resource_dir().ok()?;
    let names = agentlink_exe_file_names();
    let profiles = ["release", "debug"];
    let mut candidates = Vec::new();
    for profile in profiles {
        for name in names {
            candidates.push(
                resource_dir
                    .join("resources-generated")
                    .join("target")
                    .join(profile)
                    .join(name),
            );
            candidates.push(resource_dir.join("target").join(profile).join(name));
        }
    }
    candidates.into_iter().find(|path| path.is_file())
}

#[tauri::command]
fn default_runtime_paths(app: AppHandle) -> DevDefaultPaths {
    let exe_path = bundled_agentlink_exe_path(&app)
        .or_else(|| resolve_existing_path(agentlink_exe_file_names()[0]))
        .map(|path| normalize_display_path(&path))
        .unwrap_or_else(|| {
            if cfg!(windows) {
                "agentlink.exe".to_string()
            } else {
                "agentlink".to_string()
            }
        });

    let config_path = user_config_path(&app)
        .map(|path| normalize_display_path(&path))
        .unwrap_or_default();

    let work_dir = app
        .path()
        .home_dir()
        .ok()
        .map(|path| normalize_display_path(&path))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| ".".to_string());

    DevDefaultPaths {
        exe_path,
        config_path,
        work_dir,
    }
}

#[tauri::command]
fn default_dev_paths() -> DevDefaultPaths {
    let exe_path = standard_agentlink_exe_rel_paths()
        .into_iter()
        .find_map(|relative| resolve_existing_path(relative.to_str().unwrap_or_default()))
        .map(|path| prefer_repo_relative_path(&path))
        .unwrap_or_else(|| {
            normalize_display_path(
                &Path::new("target")
                    .join("release")
                    .join(agentlink_exe_file_names()[0]),
            )
        });

    let config_path = resolve_existing_path(standard_dev_config_rel_path())
        .map(|path| prefer_repo_relative_path(&path))
        .unwrap_or_else(|| standard_dev_config_rel_path().to_string());

    DevDefaultPaths {
        exe_path,
        config_path,
        work_dir: ".".to_string(),
    }
}

#[cfg(test)]
mod channel_binding_tests {
    use super::*;

    #[test]
    fn feishu_required_fields_include_connection_mode_default_on_save_render() {
        let mut fields = HashMap::from([
            ("app_id".to_string(), "cli_test".to_string()),
            ("app_secret".to_string(), "sec_test".to_string()),
            ("owner_open_id".to_string(), "ou_test".to_string()),
        ]);
        fields.insert("connection_mode".to_string(), "websocket".to_string());
        fields.insert("api_base".to_string(), OPEN_FEISHU_BASE.to_string());
        assert!(required_fields_ready("feishu", &fields));
        let rendered = normalized_platform_fields("feishu", &fields);
        assert_eq!(
            rendered
                .iter()
                .find(|(key, _)| key == "connection_mode")
                .map(|(_, value)| value.as_str()),
            Some("websocket")
        );
    }

    #[test]
    fn weixin_poll_status_labels_are_handled() {
        for status in ["wait", "scaned", "expired", "confirmed"] {
            assert!(
                matches!(status, "wait" | "scaned" | "expired" | "confirmed"),
                "unexpected status {status}"
            );
        }
    }

    #[test]
    fn guided_platforms_do_not_use_auto_qr() {
        for platform in ["qq", "dingtalk", "wecom"] {
            assert!(platform_supports_guided_setup(platform));
            assert!(!platform_supports_auto_qr(platform));
        }
    }

    #[test]
    fn dingtalk_binding_requires_robot_code() {
        let fields = HashMap::from([
            ("client_id".to_string(), "id".to_string()),
            ("client_secret".to_string(), "secret".to_string()),
        ]);
        assert!(!required_fields_ready("dingtalk", &fields));
        let missing = missing_required_fields("dingtalk", &fields);
        assert!(missing.contains(&"robot_code".to_string()));
    }

    #[test]
    fn packaged_resource_config_is_readonly() {
        let path = PathBuf::from(
            "/Applications/AgentLink.app/Contents/Resources/resources-generated/examples/agentlink.all.toml",
        );
        assert!(is_packaged_readonly_path(&path));
    }

    #[test]
    fn qq_guided_binding_requires_ws_url() {
        let missing = missing_required_fields("qq", &HashMap::new());
        assert!(missing.contains(&"ws_url".to_string()));
    }
}

#[cfg(test)]
mod path_resolution_tests {
    use super::*;

    #[test]
    fn manifest_repo_root_points_at_agentlink() {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let repo_root = manifest_repo_root(&manifest).expect("repo root");
        assert!(repo_root.join("examples").is_dir());
        let release_exe = repo_root
            .join("target")
            .join("release")
            .join(agentlink_exe_file_names()[0]);
        assert!(release_exe.is_file(), "missing {}", release_exe.display());
    }

    #[test]
    fn resolves_dev_default_relative_paths_from_desktop_cwd() {
        let desktop_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(1)
            .expect("agentlink-desktop dir")
            .to_path_buf();
        let previous = std::env::current_dir().ok();
        std::env::set_current_dir(&desktop_dir).expect("set cwd");
        let exe = resolve_existing_path("target/release/agentlink");
        let config = resolve_existing_path("examples/agentlink.all.toml");
        if let Some(dir) = previous {
            let _ = std::env::set_current_dir(dir);
        }
        assert!(
            exe.is_some(),
            "exe should resolve from agentlink-desktop cwd"
        );
        assert!(
            config.is_some(),
            "config should resolve from agentlink-desktop cwd"
        );
    }

    #[test]
    fn windows_style_input_resolves_on_unix() {
        if cfg!(windows) {
            return;
        }
        let desktop_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(1)
            .expect("agentlink-desktop dir")
            .to_path_buf();
        let previous = std::env::current_dir().ok();
        std::env::set_current_dir(&desktop_dir).expect("set cwd");
        let exe = resolve_existing_path("target\\release\\agentlink.exe");
        if let Some(dir) = previous {
            let _ = std::env::set_current_dir(dir);
        }
        assert!(exe.is_some(), "windows-style relative path should resolve");
    }

    #[test]
    fn packaged_macos_bundle_adds_resources_root() {
        let mut roots = Vec::new();
        push_exe_related_roots(
            &mut roots,
            Path::new("/Applications/AgentLink.app/Contents/MacOS/agentlink-desktop"),
        );
        let normalized = roots
            .iter()
            .map(|path| normalize_display_path(path))
            .collect::<Vec<_>>();
        assert!(
            normalized
                .iter()
                .any(|path| path == "/Applications/AgentLink.app/Contents/MacOS"),
            "expected macOS executable dir in candidate roots"
        );
        assert!(
            normalized
                .iter()
                .any(|path| path == "/Applications/AgentLink.app/Contents/Resources"),
            "expected macOS resources dir in candidate roots"
        );
    }
}

fn push_candidate(paths: &mut Vec<PathBuf>, candidate: PathBuf) {
    if !paths.iter().any(|path| path == &candidate) {
        paths.push(candidate);
    }
}

fn suffix_from_component(path: &Path, component_name: &str) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    let mut found = false;
    for component in path.components() {
        let value = component.as_os_str();
        if found || value.to_string_lossy() == component_name {
            found = true;
            out.push(value);
        }
    }
    found.then_some(out)
}

fn path_str(path: &Path) -> Result<&str, String> {
    path.to_str()
        .ok_or_else(|| format!("path is not valid UTF-8: {}", path.display()))
}

fn show_main_window(app: &AppHandle) -> Result<(), String> {
    let Some(window) = app.get_webview_window("main") else {
        return Err("main window not found".to_string());
    };
    window.show().map_err(|err| err.to_string())?;
    window.unminimize().map_err(|err| err.to_string())?;
    window.set_focus().map_err(|err| err.to_string())?;
    Ok(())
}

fn hide_main_window(app: &AppHandle) -> Result<(), String> {
    let Some(window) = app.get_webview_window("main") else {
        return Err("main window not found".to_string());
    };
    window.hide().map_err(|err| err.to_string())
}

fn quit_app(app: &AppHandle) {
    if let Some(state) = app.try_state::<BridgeState>() {
        if let Ok(mut guard) = state.child.lock() {
            if let Some(mut child) = guard.take() {
                let _ = child.kill();
            }
        }
    }
    app.exit(0);
}

fn setup_tray(app: &mut tauri::App) -> tauri::Result<()> {
    let hide = MenuItem::with_id(app, "tray_hide", "隐藏到托盘", true, None::<&str>)?;
    let about = MenuItem::with_id(app, "tray_about", "关于 AgentLink", true, None::<&str>)?;
    let start = MenuItem::with_id(app, "tray_start_bridge", "启动 Bridge", true, None::<&str>)?;
    let stop = MenuItem::with_id(app, "tray_stop_bridge", "停止 Bridge", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "tray_quit", "退出 AgentLink", true, None::<&str>)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&hide, &about, &sep1, &start, &stop, &sep2, &quit])?;

    let mut builder = TrayIconBuilder::with_id("agentlink-tray")
        .menu(&menu)
        .tooltip("AgentLink")
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "tray_hide" => {
                let _ = hide_main_window(app);
            }
            "tray_about" => {
                let _ = show_main_window(app);
                let _ = app.emit("tray-show-about", ());
            }
            "tray_start_bridge" => {
                let _ = show_main_window(app);
                let _ = app.emit("tray-start-bridge", ());
            }
            "tray_stop_bridge" => {
                let _ = app.emit("tray-stop-bridge", ());
            }
            "tray_quit" => quit_app(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| match event {
            TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            }
            | TrayIconEvent::DoubleClick { .. } => {
                let _ = show_main_window(tray.app_handle());
            }
            _ => {}
        });

    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }
    builder.build(app)?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(BridgeState::default())
        .manage(DesktopUpdateState::default())
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            let _ = show_main_window(app);
        }))
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            setup_tray(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            load_client_state,
            save_client_state,
            load_config_snapshot,
            pick_path,
            check_agent,
            bridge_runtime_status,
            hide_to_tray,
            show_main_window_cmd,
            read_bridge_logs,
            check_for_updates_bust,
            install_available_update_bust,
            send_test_message,
            send_channel_message,
            discover_channel_targets,
            start_qr_setup,
            get_qr_setup_status,
            validate_channel_binding,
            prepare_channel_binding,
            save_config,
            ensure_config_file,
            save_channel_config,
            save_agent_config,
            inspect_status,
            default_dev_paths,
            default_runtime_paths,
            validate_config,
            setup_channel,
            launch_qr_setup,
            start_bridge,
            stop_bridge
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
