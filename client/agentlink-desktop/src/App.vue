<script setup>
import { computed, reactive, ref, watch } from "vue";
import { getVersion } from "@tauri-apps/api/app";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { relaunch } from "@tauri-apps/plugin-process";
import appLogo from "../src-tauri/icons/icon.png";

const CHANNELS = [
  {
    id: "feishu",
    label: "飞书",
    kind: "扫码/密钥",
    scan: true,
    summary: "推荐使用 WebSocket 长连接，不依赖公网回调地址。",
    required: ["app_id", "app_secret"],
    fields: [
      ["app_id", "App ID", ""],
      ["app_secret", "App Secret", "", "password"],
      ["connection_mode", "连接模式", "websocket", "select", ["websocket", "webhook"]],
      ["api_base", "API Base", "https://open.feishu.cn"],
      ["listen", "Webhook Listen", "127.0.0.1:18200"],
      ["callback_path", "Callback Path", "/feishu/webhook"],
      ["verification_token", "Verification Token", ""]
    ]
  },
  {
    id: "lark",
    label: "Lark",
    kind: "扫码/密钥",
    scan: true,
    summary: "飞书国际版，字段和飞书基本一致。",
    required: ["app_id", "app_secret"],
    fields: [
      ["app_id", "App ID", ""],
      ["app_secret", "App Secret", "", "password"],
      ["connection_mode", "连接模式", "websocket", "select", ["websocket", "webhook"]],
      ["api_base", "API Base", "https://open.larksuite.com"],
      ["listen", "Webhook Listen", "127.0.0.1:18200"],
      ["callback_path", "Callback Path", "/feishu/webhook"],
      ["verification_token", "Verification Token", ""]
    ]
  },
  {
    id: "dingtalk",
    label: "钉钉",
    kind: "平台绑定",
    scan: true,
    summary: "打开钉钉开放平台创建应用，填写 Client ID、Client Secret 和机器人编码。",
    required: ["client_id", "client_secret"],
    fields: [
      ["client_id", "Client ID", ""],
      ["client_secret", "Client Secret", "", "password"],
      ["robot_code", "Robot Code", ""],
      ["listen", "Listen", "127.0.0.1:18300"],
      ["callback_path", "Callback Path", "/dingtalk/webhook"]
    ]
  },
  {
    id: "telegram",
    label: "Telegram",
    kind: "Bot Token",
    summary: "BotFather 创建机器人后填写 token。",
    required: ["token"],
    fields: [
      ["token", "Bot Token", "", "password"],
      ["api_base", "API Base", "https://api.telegram.org"]
    ]
  },
  {
    id: "slack",
    label: "Slack",
    kind: "Socket Mode",
    summary: "需要 bot token 和 app-level token。",
    required: ["bot_token", "app_token"],
    fields: [
      ["bot_token", "Bot Token", "", "password"],
      ["app_token", "App Token", "", "password"]
    ]
  },
  {
    id: "discord",
    label: "Discord",
    kind: "Bot Token",
    summary: "Gateway 模式，填写 Bot Token。",
    required: ["token"],
    fields: [["token", "Bot Token", "", "password"]]
  },
  {
    id: "qq",
    label: "QQ 个人号",
    kind: "扫码网关",
    scan: true,
    summary: "通过 NapCat/LLOneBot 等 OneBot v11 网关扫码登录。",
    required: ["ws_url"],
    fields: [
      ["ws_url", "OneBot WebSocket", "ws://127.0.0.1:3001"],
      ["token", "Access Token", "", "password"]
    ]
  },
  {
    id: "weixin",
    label: "微信个人号",
    kind: "ilink 扫码",
    scan: true,
    summary: "通过 ilink 兼容网关扫码获取 token；成功后先从微信发一条消息缓存 context_token。",
    required: ["token"],
    fields: [
      ["token", "Bot Token", "", "password"],
      ["account_id", "Account ID / ilink_bot_id", ""],
      ["api_base", "API Base", "https://ilinkai.weixin.qq.com"],
      ["bot_type", "Bot Type", "3"],
      ["route_tag", "SKRouteTag", ""],
      ["allow_from", "Allow From", ""]
    ]
  },
  {
    id: "wecom",
    label: "企业微信",
    kind: "管理后台绑定",
    scan: true,
    summary: "需要在企业微信管理后台创建应用或机器人，并配置回调参数。",
    required: ["corp_id", "corp_secret", "agent_id"],
    fields: [
      ["corp_id", "Corp ID", ""],
      ["corp_secret", "Corp Secret", "", "password"],
      ["agent_id", "Agent ID", ""],
      ["callback_token", "Callback Token", ""],
      ["callback_aes_key", "Callback AES Key", "", "password"],
      ["listen", "Listen", "127.0.0.1:18500"],
      ["callback_path", "Callback Path", "/wecom/callback"]
    ]
  },
  {
    id: "line",
    label: "LINE",
    kind: "Webhook",
    summary: "Messaging API webhook，需要 channel secret 和 access token。",
    required: ["secret", "token"],
    fields: [
      ["secret", "Channel Secret", "", "password"],
      ["token", "Channel Access Token", "", "password"],
      ["listen", "Listen", "127.0.0.1:18400"],
      ["callback_path", "Webhook Path", "/line/webhook"]
    ]
  },
  {
    id: "max",
    label: "MAX",
    kind: "Bot Token",
    summary: "Bot API，支持 long polling 或 webhook。",
    required: ["token"],
    fields: [
      ["token", "Bot Token", "", "password"],
      ["api_base", "API Base", "https://platform-api.max.ru"],
      ["webhook_url", "Webhook URL", ""]
    ]
  },
  {
    id: "qqbot",
    label: "QQBot",
    kind: "Gateway",
    summary: "官方 Bot Gateway，可使用 app_id/app_secret 或 token。",
    required: ["app_id", "app_secret"],
    fields: [
      ["app_id", "App ID", ""],
      ["app_secret", "App Secret", "", "password"],
      ["token", "Access Token", "", "password"],
      ["gateway_url", "Gateway URL", ""]
    ]
  },
  {
    id: "weibo",
    label: "Weibo",
    kind: "WebSocket",
    summary: "开放平台 IM WebSocket，填写 app_id/app_secret 或 token。",
    required: ["app_id", "app_secret"],
    fields: [
      ["app_id", "App ID", ""],
      ["app_secret", "App Secret", "", "password"],
      ["token", "WS Token", "", "password"],
      ["ws_endpoint", "WS Endpoint", ""]
    ]
  },
  {
    id: "http",
    label: "HTTP",
    kind: "本地测试",
    summary: "本地 webhook 参考通道，适合自测或外部系统推消息。",
    required: ["listen"],
    fields: [
      ["listen", "Listen", "127.0.0.1:18080"],
      ["bearer_token", "Bearer Token", "", "password"]
    ]
  },
  {
    id: "bridge",
    label: "AgentLink WS",
    kind: "适配器",
    summary: "外部聊天适配器通用 WebSocket 协议。",
    required: ["listen"],
    fields: [
      ["listen", "Listen", "127.0.0.1:9810"],
      ["token", "AgentLink Token", "", "password"],
      ["insecure", "本地无 token", "false"]
    ]
  }
];

const AGENTS = [
  { id: "codex", label: "Codex", kind: "内置适配", command: "codex", summary: "支持 exec 和 app-server 后端。" },
  { id: "claude-code", label: "Claude Code", kind: "CLI", command: "claude" },
  { id: "gemini", label: "Gemini CLI", kind: "CLI", command: "gemini" },
  { id: "cursor", label: "Cursor Agent", kind: "CLI", command: "agent" },
  { id: "kimi", label: "Kimi CLI", kind: "CLI", command: "kimi" },
  { id: "qoder", label: "Qoder CLI", kind: "CLI", command: "qodercli" },
  { id: "opencode", label: "OpenCode", kind: "CLI", command: "opencode" },
  { id: "iflow", label: "iFlow CLI", kind: "CLI", command: "iflow" },
  { id: "devin", label: "Devin", kind: "CLI/ACP", command: "devin" },
  { id: "acp", label: "ACP", kind: "协议入口", command: "acp-agent" },
  { id: "mock", label: "Mock", kind: "测试", command: "" }
];

const STATUS_LABELS = {
  configured: "已配置",
  missing: "未配置",
  ready: "可用",
  "scan available": "可扫码",
  "missing credentials": "缺少密钥",
  installed: "已安装",
  "not found": "未安装",
  "built-in": "内置"
};

const FIELD_HELP = {
  connection_mode: "推荐 websocket。websocket 不需要公网回调地址；webhook 需要平台能访问本机或公网地址。",
  api_base: "平台开放 API 地址。国内飞书通常是 open.feishu.cn，Lark 是 open.larksuite.com。",
  listen: "Webhook 模式下本地监听地址。websocket 模式通常不会使用。",
  callback_path: "Webhook 回调路径，需要和平台后台配置保持一致。",
  app_id: "平台应用 ID。扫码绑定成功后通常会自动写入。",
  app_secret: "平台应用密钥。只保存在本地配置里，用于调用开放 API。",
  verification_token: "Webhook 验证 token，用于校验回调来源。",
  client_id: "钉钉应用 Client ID。",
  client_secret: "钉钉应用 Client Secret。",
  robot_code: "钉钉机器人编码，用于机器人发送消息。",
  token: "平台访问 token 或机器人 token。",
  ws_url: "OneBot/NapCat 等网关的 WebSocket 地址。",
  corp_id: "企业微信企业 ID。",
  corp_secret: "企业微信应用 Secret。",
  agent_id: "企业微信应用 Agent ID。",
  callback_token: "企业微信回调校验 token。",
  callback_aes_key: "企业微信回调加解密 AES Key。",
  secret: "平台 Channel Secret，用于签名校验。",
  webhook_url: "外部 Webhook 地址，部分平台可用于主动发送或绑定回调。"
};

const SELECT_HELP = {
  operationMode: "接口优先会优先使用客户端内置 Rust 接口，减少黑窗；命令行兼容用于直接调用 AgentLink 可执行文件。",
  channelTarget: "接收目标代表一个用户、群或 Webhook。一个接收目标同一时刻只能绑定一个 Agent。",
  receiveIdType: "告诉平台按哪种 ID 发送消息。飞书群通常用 chat_id，私聊可用 open_id；Telegram 用 chat_id。",
  messageType: "测试消息类型。日常调试先用 text，其它类型主要用于验证媒体或原始事件适配。",
  codexBackend: "exec 是调用 Codex CLI；app-server 预留给 Codex app-server 协议。",
  codexMode: "控制 Codex 的默认执行策略。suggest 最保守，full-auto/yolo 更自动化。",
  agentModel: "留空时使用该 Agent 自己的默认模型。",
  agentArgs: "额外命令行参数。每行一个，便于后续传入模型、模式或自定义开关。"
};

const CURRENT_OS = (() => {
  const userAgent = navigator.userAgent.toLowerCase();
  if (userAgent.includes("windows")) return "windows";
  if (userAgent.includes("mac os") || userAgent.includes("macintosh")) return "macos";
  return "unix";
})();

const IS_DEV_LAYOUT = /^https?:$/.test(window.location.protocol);
const DEV_DEFAULT_WORK_DIR = ".";
const DEV_DEFAULT_EXE_PATH = CURRENT_OS === "windows" ? "target/release/agentlink.exe" : "target/release/agentlink";
const DEV_DEFAULT_CONFIG_PATH = "examples/agentlink.all.toml";

function runtimeDefaults() {
  if (IS_DEV_LAYOUT) {
    return {
      workDir: DEV_DEFAULT_WORK_DIR,
      exePath: DEV_DEFAULT_EXE_PATH,
      configPath: DEV_DEFAULT_CONFIG_PATH
    };
  }
  return {
    workDir: "",
    exePath: "",
    configPath: ""
  };
}

const DEFAULTS = runtimeDefaults();
const DEFAULT_WORK_DIR = DEFAULTS.workDir;
const DEFAULT_EXE_PATH = DEFAULTS.exePath;
const DEFAULT_CONFIG_PATH = DEFAULTS.configPath;
const defaultConfigPath = () => DEFAULT_CONFIG_PATH;
const DIRECT_SEND_CHANNELS = new Set(["http", "feishu", "lark", "telegram", "dingtalk", "slack", "discord", "line", "weixin"]);

function defaultsFor(fields) {
  return Object.fromEntries(fields.map(([name, _label, value]) => [name, value ?? ""]));
}

function initialChannelFields() {
  return Object.fromEntries(CHANNELS.map((channel) => [channel.id, defaultsFor(channel.fields)]));
}

function defaultTarget(channelId = "feishu", index = 1) {
  const base = {
    id: `target-${Date.now()}-${channelId}-${index}`,
    name: `目标 ${index}`,
    channel: channelId,
    receiveIdType: "chat_id",
    receiveId: "",
    sessionKey: `${channelId}:client:test:${index}`,
    userId: "client-user",
    userName: "Desktop Client",
    sessionWebhook: "",
    webhookUrl: "",
    messageId: "",
    replyContext: ""
  };
  if (channelId === "telegram") base.receiveIdType = "chat_id";
  if (channelId === "qq") base.receiveIdType = "user_id";
  if (channelId === "weixin") base.receiveIdType = "user_id";
  if (channelId === "dingtalk") base.receiveIdType = "session_webhook";
  if (channelId === "http") {
    base.webhookUrl = "http://127.0.0.1:18080/webhook";
    base.receiveIdType = "webhook";
  }
  return base;
}

function initialChannelTargets() {
  return Object.fromEntries(CHANNELS.map((channel) => [channel.id, [defaultTarget(channel.id, 1)]]));
}

function initialActiveTargetIds(targets) {
  return Object.fromEntries(Object.entries(targets).map(([channelId, items]) => [channelId, items[0]?.id ?? ""]));
}

function initialAgentFields() {
  return Object.fromEntries(
    AGENTS.map((agent) => [
      agent.id,
      agent.id === "codex"
        ? { backend: "exec", mode: "suggest", command: "codex", model: "", reasoningEffort: "", args: "" }
        : { command: agent.command ?? agent.id, model: "", mode: "", args: "" }
    ])
  );
}

function createConnection(index = 1) {
  const channelTargets = initialChannelTargets();
  return {
    id: `conn-${Date.now()}-${index}`,
    name: index === 1 ? "默认连接" : `连接 ${index}`,
    exePath: DEFAULT_EXE_PATH,
    configPath: defaultConfigPath(),
    project: index === 1 ? "demo" : `demo-${index}`,
    workDir: DEFAULT_WORK_DIR,
    operationMode: "api",
    selectedChannel: "feishu",
    selectedAgent: "codex",
    channelFields: initialChannelFields(),
    agentFields: initialAgentFields(),
    channelTargets,
    activeTargetIds: initialActiveTargetIds(channelTargets),
    test: {
      messageType: "text",
      content: "帮我看一下这条测试消息"
    }
  };
}

const activeTab = ref("connect");
const busy = ref("idle");
const busyLabel = computed(() => ({ running: "执行中", ok: "完成", error: "失败" }[busy.value] ?? ""));
const showBusy = computed(() => busy.value !== "idle");
const operationRunning = computed(() => busy.value === "running");
const logText = ref("");
const toasts = reactive([]);
const activeConnectionId = ref("conn-default");
const clientStateLoaded = ref(false);
const targetAdvancedOpen = ref(false);
const agentChecks = reactive({});
const qrSetup = reactive({
  visible: false,
  sessionId: "",
  platform: "",
  state: "idle",
  message: "",
  userCode: "",
  qrUrl: "",
  qrSvg: "",
  output: [],
  done: false,
  success: null
});
let saveTimer = 0;
let qrPollTimer = 0;
let busyTimer = 0;
let toastSeq = 0;

const state = reactive({
  connections: [{ ...createConnection(1), id: "conn-default" }],
  test: {
    webhookUrl: "http://127.0.0.1:18080/webhook",
    sessionKey: "client:test:user",
    userId: "client-user",
    messageType: "text",
    content: "帮我看一下这条测试消息"
  }
});

const status = reactive({
  exeExists: false,
  configExists: false,
  projectConfigured: false,
  channelConfigured: false,
  agentConfigured: false,
  bindingReady: false,
  agentInstalled: false,
  connectionStatus: "missing",
  channelStatus: "missing",
  agentStatus: "missing",
  bindingStatus: "missing credentials",
  agentInstallStatus: "not found",
  summary: "等待检查"
});
const bridgeRuntime = reactive({
  running: false,
  pid: null,
  status: "not running"
});
const updatePreferences = reactive({
  autoCheckUpdates: true,
  autoInstallUpdates: true,
  lastUpdateCheckAt: "",
  lastUpdateError: ""
});
const updateStatus = reactive({
  appVersion: "",
  checked: false,
  checking: false,
  installing: false,
  installed: false,
  downloaded: 0,
  total: 0,
  available: null,
  error: ""
});
const updateDialog = reactive({
  visible: false,
  version: "",
  date: "",
  body: ""
});

const connection = computed(() => {
  return state.connections.find((item) => item.id === activeConnectionId.value) ?? state.connections[0];
});
const currentChannel = computed(() => CHANNELS.find((item) => item.id === connection.value.selectedChannel) ?? CHANNELS[0]);
const currentAgent = computed(() => AGENTS.find((item) => item.id === connection.value.selectedAgent) ?? AGENTS[0]);
const currentChannelFields = computed(() => connection.value.channelFields[connection.value.selectedChannel]);
const currentAgentFields = computed(() => connection.value.agentFields[connection.value.selectedAgent]);
const currentTargets = computed(() => connection.value.channelTargets?.[connection.value.selectedChannel] ?? []);
const activeTarget = computed(() => {
  const targetId = connection.value.activeTargetIds?.[connection.value.selectedChannel];
  return currentTargets.value.find((item) => item.id === targetId) ?? currentTargets.value[0] ?? defaultTarget(connection.value.selectedChannel, 1);
});
const activeTargetKey = computed(() => targetKey(activeTarget.value));
const channelTargetOptions = computed(() => collectChannelTargets(connection.value.selectedChannel));
const activeTargetLock = computed(() => targetLock(activeTargetKey.value));
const canStartBridge = computed(() => !bridgeRuntime.running && !operationRunning.value && !activeTargetLock.value);
const updateProgressPercent = computed(() => {
  if (!updateStatus.total) return 0;
  return Math.min(100, Math.round((updateStatus.downloaded / updateStatus.total) * 100));
});
const updateStateText = computed(() => {
  if (updateStatus.installing) return "正在下载并安装";
  if (updateStatus.installed) return "更新已安装，重启后生效";
  if (updateStatus.available) return `发现新版本 ${updateStatus.available.version}`;
  if (updateStatus.error) return "检查更新失败";
  if (updateStatus.checked) return "已是最新版本";
  return "尚未检查";
});
const currentTest = computed(() => connection.value.test ?? { messageType: "text", content: "" });
const canScan = computed(() => Boolean(currentChannel.value.scan));
const canDirectSendTest = computed(() => DIRECT_SEND_CHANNELS.has(connection.value.selectedChannel));
const hasReusableChannelConfig = computed(() => Boolean(findReusableChannelFields(connection.value.selectedChannel)));
const agentReady = computed(() => connection.value.selectedAgent === "mock" || Boolean((currentAgentFields.value.command ?? "").trim()));
const qrPanelVisible = computed(() => qrSetup.visible && qrSetup.platform === connection.value.selectedChannel);
const qrHelpText = computed(() => {
  if (qrSetup.platform === "weixin") return "扫码并在手机上确认后会自动保存 token。之后请先从微信给机器人发一条消息，用于缓存 context_token。";
  if (qrSetup.platform === "feishu" || qrSetup.platform === "lark") return "扫码后请在飞书/Lark 页面里继续确认创建或授权，下面日志会显示轮询状态。";
  if (qrSetup.qrUrl) return "该二维码会打开平台绑定或网关配置页面，请按页面提示完成配置。";
  return "网关型通道的二维码可能由外部网关显示，客户端会展示 setup 结果。";
});
const sendTestHelpText = computed(() => {
  if (canDirectSendTest.value) {
    return `当前 ${currentChannel.value.label} 支持客户端直发测试，可直接用下面的目标和内容做联调。`;
  }
  return `当前 ${currentChannel.value.label} 不支持客户端直发测试。请先启动 Bridge，再从真实聊天渠道发一条消息，或切换到 HTTP 通道做注入测试。`;
});

function labelStatus(value) {
  return STATUS_LABELS[value] ?? value ?? "未知";
}

function fieldHelp(name) {
  return FIELD_HELP[name] ?? "";
}

function localChannelReady(channel) {
  const fields = connection.value.channelFields[channel.id] ?? {};
  return channelFieldsReady(channel, fields);
}

function channelFieldsReady(channel, fields) {
  if (!channel.required?.length) return true;
  if ((channel.id === "qqbot" || channel.id === "weibo") && fields.token?.trim()) return true;
  return channel.required.every((key) => String(fields[key] ?? "").trim().length > 0);
}

function anyChannelReady(channel) {
  return state.connections.some((item) => channelFieldsReady(channel, item.channelFields?.[channel.id] ?? {}));
}

function channelBadge(channel) {
  if (channel.id === connection.value.selectedChannel) return labelStatus(status.bindingStatus);
  if (anyChannelReady(channel)) return "可复用";
  if (localChannelReady(channel)) return "已填写";
  if (channel.scan) return "可扫码";
  return "待配置";
}

function channelBadgeClass(channel) {
  const reusable = anyChannelReady(channel);
  return {
    good: channel.id === connection.value.selectedChannel ? status.bindingReady : reusable || localChannelReady(channel),
    warn: channel.scan && !localChannelReady(channel),
    bad: !channel.scan && !localChannelReady(channel) && !reusable
  };
}

function agentBadge(agent) {
  if (agent.id === "mock") return "内置";
  if (agentChecks[agent.id]) return labelStatus(agentChecks[agent.id].status);
  if (agent.id === connection.value.selectedAgent) return labelStatus(status.agentInstallStatus);
  return connection.value.agentFields[agent.id]?.command ? "待检测" : "未配置";
}

function agentBadgeClass(agent) {
  if (agent.id === "mock") return { good: true };
  if (agentChecks[agent.id]) return { good: agentChecks[agent.id].installed, bad: !agentChecks[agent.id].installed };
  if (agent.id === connection.value.selectedAgent) return { good: status.agentInstalled, bad: !status.agentInstalled };
  return { warn: true };
}

function chooseConnection(id) {
  activeConnectionId.value = id;
  refreshStatus();
}

function addConnection() {
  const next = createConnection(state.connections.length + 1);
  state.connections.push(next);
  activeConnectionId.value = next.id;
  syncChannelFromConfigured(next.selectedChannel, false);
  appendLog(`已创建连接：${next.name}`);
  refreshStatus();
}

function duplicateConnection() {
  const source = JSON.parse(JSON.stringify(connection.value));
  source.id = `conn-${Date.now()}`;
  source.name = `${connection.value.name} 副本`;
  source.project = `${connection.value.project}-copy`;
  state.connections.push(source);
  activeConnectionId.value = source.id;
  appendLog(`已复制连接：${source.name}`);
  refreshStatus();
}

function removeConnection() {
  if (state.connections.length <= 1) {
    appendLog("至少保留一个连接。");
    return;
  }
  const index = state.connections.findIndex((item) => item.id === activeConnectionId.value);
  const removed = state.connections.splice(index, 1)[0];
  activeConnectionId.value = state.connections[Math.max(0, index - 1)].id;
  appendLog(`已删除连接：${removed.name}`);
  refreshStatus();
}

function chooseChannel(channel) {
  if (qrSetup.visible && qrSetup.platform !== channel.id) {
    resetQrPanel();
  }
  connection.value.selectedChannel = channel.id;
  ensureTargets(channel.id);
  syncChannelFromConfigured(channel.id, false);
  refreshStatus();
}

function chooseAgent(agent) {
  connection.value.selectedAgent = agent.id;
  refreshStatus();
}

function ensureTargets(channelId = connection.value.selectedChannel) {
  if (!connection.value.channelTargets) connection.value.channelTargets = {};
  if (!connection.value.activeTargetIds) connection.value.activeTargetIds = {};
  if (!Array.isArray(connection.value.channelTargets[channelId]) || connection.value.channelTargets[channelId].length === 0) {
    const reusable = collectChannelTargets(channelId)[0]?.target;
    connection.value.channelTargets[channelId] = [reusable ? cloneTarget(reusable, channelId) : defaultTarget(channelId, 1)];
  }
  if (!connection.value.activeTargetIds[channelId]) {
    connection.value.activeTargetIds[channelId] = connection.value.channelTargets[channelId][0].id;
  }
}

function chooseTargetByKey(key) {
  ensureTargets();
  const lock = targetLock(key);
  if (lock) {
    notify("warning", "接收目标已被占用", `${lock.connectionName} 正在使用该目标连接到 ${lock.agentLabel}`);
    return;
  }
  const channelId = connection.value.selectedChannel;
  let target = connection.value.channelTargets[channelId].find((item) => targetKey(item) === key);
  if (!target) {
    const option = channelTargetOptions.value.find((item) => item.key === key);
    if (!option) return;
    target = cloneTarget(option.target, channelId);
    connection.value.channelTargets[channelId].push(target);
  }
  connection.value.activeTargetIds[channelId] = target.id;
}

function addTarget() {
  const channelId = connection.value.selectedChannel;
  ensureTargets(channelId);
  const next = defaultTarget(channelId, connection.value.channelTargets[channelId].length + 1);
  connection.value.channelTargets[channelId].push(next);
  connection.value.activeTargetIds[channelId] = next.id;
  appendLog(`已新增 ${currentChannel.value.label} 目标：${next.name}`);
}

function removeTarget() {
  const channelId = connection.value.selectedChannel;
  ensureTargets(channelId);
  if (connection.value.channelTargets[channelId].length <= 1) {
    appendLog("至少保留一个测试目标。");
    return;
  }
  const index = connection.value.channelTargets[channelId].findIndex((item) => item.id === activeTarget.value.id);
  const removed = connection.value.channelTargets[channelId].splice(Math.max(0, index), 1)[0];
  connection.value.activeTargetIds[channelId] = connection.value.channelTargets[channelId][0].id;
  appendLog(`已删除目标：${removed.name}`);
}

function applyDiscoveredTargets(items) {
  const channelId = connection.value.selectedChannel;
  ensureTargets(channelId);
  const targets = connection.value.channelTargets[channelId];
  let added = 0;
  for (const item of items) {
    const existing = targets.find((target) => target.sessionKey === item.sessionKey);
    const next = {
      id: existing?.id || `target-${Date.now()}-${channelId}-${added}`,
      name: item.name || item.userName || item.sessionKey,
      channel: channelId,
      receiveIdType: item.receiveIdType || "user_id",
      receiveId: item.receiveId || "",
      sessionKey: item.sessionKey,
      userId: item.userId || "",
      userName: item.userName || "",
      sessionWebhook: item.receiveIdType === "session_webhook" ? item.webhookUrl || "" : "",
      webhookUrl: item.webhookUrl || "",
      messageId: item.messageId || "",
      replyContext: item.replyCtx || "",
      contentPreview: item.contentPreview || ""
    };
    if (existing) {
      Object.assign(existing, next);
    } else {
      targets.push(next);
      added += 1;
    }
  }
  if (items[0]) {
    const first = targets.find((target) => target.sessionKey === items[0].sessionKey);
    if (first) connection.value.activeTargetIds[channelId] = first.id;
  }
  return added;
}

function cloneTarget(target, channelId) {
  return {
    ...JSON.parse(JSON.stringify(target)),
    id: `target-${Date.now()}-${channelId}-${Math.random().toString(36).slice(2, 8)}`,
    channel: channelId
  };
}

function targetReceiveValue(target) {
  return String(target?.receiveId || target?.webhookUrl || target?.sessionWebhook || "").trim();
}

function targetKey(target) {
  const value = targetReceiveValue(target);
  if (!value) return "";
  return `${target?.receiveIdType || "receive_id"}:${value}`;
}

function targetLabel(target) {
  const name = target?.name || "未命名目标";
  const value = targetReceiveValue(target) || "未填写接收 ID";
  return `${name} · ${target?.receiveIdType || "receive_id"} · ${value}`;
}

function pathStateLabel(kind) {
  const raw = kind === "exe" ? connection.value.exePath : connection.value.configPath;
  if (!String(raw ?? "").trim()) return "未配置";
  if (kind === "exe") return status.exeExists ? "存在" : "不存在";
  return status.configExists ? "存在" : "不存在";
}

function pathStateClass(kind) {
  const raw = kind === "exe" ? connection.value.exePath : connection.value.configPath;
  if (!String(raw ?? "").trim()) return { warn: true };
  if (kind === "exe") return { good: status.exeExists, bad: !status.exeExists };
  return { good: status.configExists, bad: !status.configExists };
}

function collectChannelTargets(channelId) {
  const seen = new Set();
  const items = [];
  for (const item of state.connections) {
    const targets = item.channelTargets?.[channelId] ?? [];
    for (const target of targets) {
      const key = targetKey(target);
      if (!key || seen.has(key)) continue;
      seen.add(key);
      items.push({
        key,
        target,
        lockedBy: targetLock(key)
      });
    }
  }
  return items;
}

function targetLock(key) {
  if (!key) return null;
  for (const item of state.connections) {
    if (item.id === connection.value.id) continue;
    if (item.selectedChannel !== connection.value.selectedChannel) continue;
    const targetId = item.activeTargetIds?.[item.selectedChannel];
    const target = (item.channelTargets?.[item.selectedChannel] ?? []).find((candidate) => candidate.id === targetId);
    if (targetKey(target) === key) {
      const agent = AGENTS.find((candidate) => candidate.id === item.selectedAgent);
      return {
        connectionId: item.id,
        connectionName: item.name,
        agentLabel: agent?.label || item.selectedAgent
      };
    }
  }
  return null;
}

function findReusableChannelFields(channelId) {
  const channel = CHANNELS.find((item) => item.id === channelId);
  if (!channel) return null;
  for (const item of state.connections) {
    if (item.id === connection.value.id) continue;
    const fields = item.channelFields?.[channelId] ?? {};
    if (channelFieldsReady(channel, fields)) return fields;
  }
  return null;
}

function syncChannelFromConfigured(channelId = connection.value.selectedChannel, force = true) {
  const channel = CHANNELS.find((item) => item.id === channelId);
  const reusable = findReusableChannelFields(channelId);
  if (!channel || !reusable) return false;
  const current = connection.value.channelFields[channelId] ?? {};
  if (!force && channelFieldsReady(channel, current)) return false;
  connection.value.channelFields[channelId] = { ...current, ...JSON.parse(JSON.stringify(reusable)) };
  appendLog(`已复用 ${channel.label} 的已配置密钥。`);
  return true;
}

function reuseChannelConfig() {
  if (syncChannelFromConfigured(connection.value.selectedChannel, true)) {
    notify("success", "已复用 Channel 配置", `${currentChannel.value.label} 的密钥已同步到当前连接。`);
    refreshStatus();
  } else {
    notify("warning", "没有可复用配置", "请先在任意连接里完成该 Channel 的配置。");
  }
}

function options() {
  const agentFields = currentAgentFields.value;
  return {
    exePath: connection.value.exePath,
    configPath: connection.value.configPath,
    project: connection.value.project,
    workDir: connection.value.workDir,
    platform: connection.value.selectedChannel,
    extra: "",
    operationMode: connection.value.operationMode || "api",
    agentType: connection.value.selectedAgent,
    agentBackend: agentFields.backend ?? "",
    agentCommand: agentFields.command ?? "",
    agentModel: agentFields.model ?? "",
    agentMode: agentFields.mode ?? "",
    reasoningEffort: agentFields.reasoningEffort ?? "",
    agentArgs: agentFields.args ?? "",
    channelFields: { ...currentChannelFields.value }
  };
}

function appendLog(message) {
  logText.value += `${new Date().toLocaleTimeString()} ${message}\n`;
  requestAnimationFrame(() => {
    const node = document.getElementById("log");
    if (node) node.scrollTop = node.scrollHeight;
  });
}

function toastDetail(value) {
  const text = String(value ?? "").trim();
  if (!text) return "";
  return text.length > 260 ? `${text.slice(0, 260)}...` : text;
}

function notify(type, title, detail = "", timeout) {
  const id = ++toastSeq;
  const duration = timeout ?? (type === "error" ? 7000 : type === "warning" ? 5200 : 3600);
  toasts.push({
    id,
    type,
    title,
    detail: toastDetail(detail)
  });
  window.setTimeout(() => dismissToast(id), duration);
}

function dismissToast(id) {
  const index = toasts.findIndex((item) => item.id === id);
  if (index >= 0) toasts.splice(index, 1);
}

function clientStatePayload() {
  return {
    activeConnectionId: activeConnectionId.value,
    connections: JSON.parse(JSON.stringify(state.connections)),
    updatePreferences: JSON.parse(JSON.stringify(updatePreferences))
  };
}

function looksLikeWindowsPath(value) {
  return /^[a-z]:[\\/]/i.test(String(value ?? "").trim());
}

function migrateLegacyPath(path) {
  const value = String(path ?? "").trim();
  if (!value) return "";
  let next = value
    .replaceAll("codex-chat-bridge-v0.1.0-windows-amd64", "agentlink-v0.1.0-windows-amd64")
    .replaceAll("dist/agentlink-v0.1.0-windows-amd64/agentlink.exe", "target/release/agentlink.exe")
    .replaceAll("dist\\agentlink-v0.1.0-windows-amd64\\agentlink.exe", "target/release/agentlink.exe")
    .replaceAll("codex-chat-bridge.exe", "agentlink.exe")
    .replaceAll("codex-chat-bridge-client.exe", "agentlink-desktop.exe")
    .replaceAll("examples/bridge.", "examples/agentlink.")
    .replaceAll("examples\\bridge.", "examples\\agentlink.")
    .replace(/examples[\\/]bridge$/i, DEFAULT_CONFIG_PATH);

  if (CURRENT_OS !== "windows") {
    next = next
      .replace(/(^|[\\/])agentlink\.exe$/i, "$1agentlink")
      .replace(/(^|[\\/])agentlink-desktop\.exe$/i, "$1agentlink-desktop");
  }

  if (/dist[\\/]agentlink-v[^\\/]+[\\/]examples[\\/]agentlink\.\d+\.toml$/i.test(next) || /examples[\\/]agentlink\.\d+\.toml$/i.test(next)) {
    next = DEFAULT_CONFIG_PATH;
  }

  return next;
}

function normalizeConnectionPaths(item, index) {
  item.exePath = migrateLegacyPath(item.exePath) || DEFAULT_EXE_PATH;
  item.configPath = migrateLegacyPath(item.configPath) || defaultConfigPath();
  if (!String(item.workDir ?? "").trim() || (CURRENT_OS !== "windows" && looksLikeWindowsPath(item.workDir))) {
    item.workDir = DEFAULT_WORK_DIR;
  }
  if (item.agentFields?.codex && (!String(item.agentFields.codex.workDir ?? "").trim() || (CURRENT_OS !== "windows" && looksLikeWindowsPath(item.agentFields.codex.workDir)))) {
    item.agentFields.codex.workDir = item.workDir;
  }
  return item;
}

function normalizeConnection(item, index) {
  const fallback = createConnection(index + 1);
  const channelTargets = { ...fallback.channelTargets, ...(item.channelTargets || {}) };
  for (const [channelId, targets] of Object.entries(channelTargets)) {
    if (!Array.isArray(targets) || targets.length === 0) {
      channelTargets[channelId] = [defaultTarget(channelId, 1)];
    }
  }
  return normalizeConnectionPaths({
    ...fallback,
    ...item,
    channelFields: { ...fallback.channelFields, ...(item.channelFields || {}) },
    agentFields: { ...fallback.agentFields, ...(item.agentFields || {}) },
    channelTargets,
    activeTargetIds: { ...initialActiveTargetIds(channelTargets), ...(item.activeTargetIds || {}) },
    test: { ...fallback.test, ...(item.test || {}) },
    operationMode: item.operationMode || "api"
  }, index);
}

async function loadClientState() {
  try {
    const saved = await invoke("load_client_state");
    if (saved?.connections?.length) {
      state.connections.splice(
        0,
        state.connections.length,
        ...saved.connections.map((item, index) => normalizeConnection(item, index))
      );
      activeConnectionId.value = saved.activeConnectionId;
      if (saved.updatePreferences) {
        Object.assign(updatePreferences, {
          autoCheckUpdates: saved.updatePreferences.autoCheckUpdates ?? true,
          autoInstallUpdates: saved.updatePreferences.autoInstallUpdates ?? true,
          lastUpdateCheckAt: saved.updatePreferences.lastUpdateCheckAt || "",
          lastUpdateError: saved.updatePreferences.lastUpdateError || ""
        });
      }
      appendLog(`已从 SQLite 加载 ${saved.connections.length} 个连接。`);
    } else {
      appendLog("SQLite 暂无连接数据，使用默认连接。");
      await saveClientState("初始化本地数据");
    }
  } catch (error) {
    appendLog(`加载 SQLite 状态失败：${error}`);
  } finally {
    clientStateLoaded.value = true;
    await refreshStatus();
    await loadAppVersion();
    window.setTimeout(() => {
      maybeAutoCheckForUpdate();
    }, 3500);
  }
}

async function saveClientState(label = "保存本地数据") {
  const result = await invoke("save_client_state", { state: clientStatePayload() });
  appendLog(`${label}: ${result}`);
  return result;
}

async function saveLocalState() {
  try {
    const result = await saveClientState("保存本地数据");
    notify("success", "保存本地数据成功", result);
  } catch (error) {
    appendLog(`保存本地数据失败：${error}`);
    notify("error", "保存本地数据失败", error);
  }
}

function persistSoon() {
  if (!clientStateLoaded.value) return;
  window.clearTimeout(saveTimer);
  saveTimer = window.setTimeout(() => {
    saveClientState("自动保存本地数据").catch((error) => appendLog(`自动保存失败：${error}`));
  }, 600);
}

function setBusy(value, timeout = 0) {
  window.clearTimeout(busyTimer);
  busy.value = value;
  if (timeout > 0) {
    busyTimer = window.setTimeout(() => {
      busy.value = "idle";
    }, timeout);
  }
}

async function run(label, task) {
  setBusy("running");
  try {
    const result = await task();
    appendLog(`${label}: ${result || "ok"}`);
    const resultText = String(result || "ok");
    const type = /失败|错误|未启动|暂未|缺少|不存在|not found|error/i.test(resultText) ? "warning" : "success";
    notify(type, type === "warning" ? `${label}提醒` : `${label}成功`, resultText);
    setBusy("ok", 1600);
  } catch (error) {
    appendLog(`${label}: ${error}`);
    notify("error", `${label}失败`, error);
    setBusy("error", 4000);
  } finally {
    await refreshStatus();
  }
}

async function pickPath(kind) {
  const current = kind === "exe" ? connection.value.exePath : kind === "config" ? connection.value.configPath : connection.value.workDir;
  try {
    const selected = await invoke("pick_path", { options: { kind, current } });
    if (!selected) return;
    if (kind === "exe") connection.value.exePath = selected;
    if (kind === "config") connection.value.configPath = selected;
    if (kind === "folder") connection.value.workDir = selected;
    appendLog(`已选择路径：${selected}`);
    await refreshStatus();
  } catch (error) {
    appendLog(`选择路径失败：${error}`);
    notify("error", "选择路径失败", error);
  }
}

async function resetDefaultPaths() {
  connection.value.exePath = DEFAULT_EXE_PATH;
  connection.value.configPath = defaultConfigPath();
  connection.value.workDir = DEFAULT_WORK_DIR;
  appendLog(`已恢复${IS_DEV_LAYOUT ? "开发态" : "正式版"}默认路径。`);
  await saveClientState("保存默认路径");
  await refreshStatus();
}

async function hideToTray() {
  try {
    await invoke("hide_to_tray");
  } catch (error) {
    appendLog(`隐藏到托盘失败：${error}`);
    notify("error", "隐藏到托盘失败", error);
  }
}

async function refreshStatus() {
  try {
    const next = await invoke("inspect_status", { options: options() });
    Object.assign(status, next);
    await refreshBridgeRuntime();
  } catch (error) {
    status.summary = `状态检查失败：${error}`;
  }
}

async function refreshBridgeRuntime() {
  try {
    const next = await invoke("bridge_runtime_status");
    bridgeRuntime.running = Boolean(next?.running);
    bridgeRuntime.pid = next?.pid ?? null;
    bridgeRuntime.status = next?.status ?? "unknown";
  } catch (error) {
    bridgeRuntime.running = false;
    bridgeRuntime.pid = null;
    bridgeRuntime.status = `状态未知：${error}`;
  }
}

async function loadAppVersion() {
  try {
    updateStatus.appVersion = await getVersion();
  } catch (error) {
    updateStatus.appVersion = "";
    appendLog(`读取桌面端版本失败：${error}`);
  }
}

function updateCheckFinished(error = "") {
  updateStatus.checked = true;
  updatePreferences.lastUpdateCheckAt = new Date().toISOString();
  updatePreferences.lastUpdateError = error;
  updateStatus.error = error;
  saveClientState("保存升级状态").catch((saveError) => appendLog(`保存升级状态失败：${saveError}`));
}

function applyAvailableUpdate(update) {
  updateStatus.available = {
    version: update.version,
    date: update.date,
    body: update.body
  };
  updateStatus.installed = false;
  updateStatus.downloaded = 0;
  updateStatus.total = 0;
}

function openUpdateDialog(update) {
  updateDialog.visible = true;
  updateDialog.version = update.version;
  updateDialog.date = update.date || "";
  updateDialog.body = update.body || "";
}

function closeUpdateDialog() {
  updateDialog.visible = false;
}

async function checkForUpdates(manual = true) {
  if (updateStatus.checking || updateStatus.installing) return updateStatus.available;
  updateStatus.checking = true;
  updateStatus.error = "";
  try {
    const update = await invoke("check_for_updates_bust");
    if (update) {
      applyAvailableUpdate(update);
      updateCheckFinished("");
      openUpdateDialog(update);
      return update;
    }

    updateStatus.available = null;
    updateCheckFinished("");
    if (manual) {
      notify("warning", "已是最新版本", "当前桌面端无需更新。");
    }
    return null;
  } catch (error) {
    const message = String(error);
    updateStatus.available = null;
    updateCheckFinished(message);
    if (manual) {
      notify("error", "检查更新失败", message);
    } else {
      appendLog(`自动检查更新失败：${message}`);
    }
    return null;
  } finally {
    updateStatus.checking = false;
  }
}

async function installFromUpdateDialog() {
  await installAvailableUpdate();
}

function maybeAutoCheckForUpdate() {
  if (!updatePreferences.autoCheckUpdates || updateStatus.checked) return;
  checkForUpdates(false)
    .then((update) => {
      if (update && updatePreferences.autoInstallUpdates) {
        return installAvailableUpdate({ auto: true });
      }
      return null;
    })
    .catch((error) => appendLog(`自动检查更新失败：${error}`));
}

async function installAvailableUpdate({ auto = false } = {}) {
  if (!updateStatus.available || updateStatus.installing) return;
  await refreshBridgeRuntime();
  if (bridgeRuntime.running) {
    if (auto) {
      const message = "AgentLink Bridge 正在运行，已跳过自动安装。停止 Bridge 后可手动完成更新。";
      updateStatus.error = message;
      updatePreferences.lastUpdateError = message;
      notify("warning", "已跳过自动安装", message);
      saveClientState("保存升级状态").catch((saveError) => appendLog(`保存升级状态失败：${saveError}`));
      return;
    }

    const confirmed = window.confirm("AgentLink Bridge 正在运行。安装更新前需要停止 Bridge，是否现在停止并继续安装？");
    if (!confirmed) {
      notify("warning", "已延后安装", "停止 Bridge 后可继续安装更新。");
      return;
    }
    await stopBridge();
    await refreshBridgeRuntime();
    if (bridgeRuntime.running) {
      notify("error", "无法安装更新", "Bridge 仍在运行，请先停止后重试。");
      return;
    }
  }

  updateStatus.installing = true;
  updateStatus.downloaded = 0;
  updateStatus.total = 0;
  updateStatus.error = "";
  try {
    await invoke("install_available_update_bust");
    updateStatus.installed = true;
    closeUpdateDialog();
    notify("success", "更新已安装", auto ? "更新已自动安装，请重启桌面端让新版本生效。" : "请重启桌面端让新版本生效。");
  } catch (error) {
    updateStatus.error = String(error);
    updatePreferences.lastUpdateError = updateStatus.error;
    notify("error", "安装更新失败", updateStatus.error);
    saveClientState("保存升级失败状态").catch((saveError) => appendLog(`保存升级失败状态失败：${saveError}`));
  } finally {
    updateStatus.installing = false;
  }
}

async function relaunchDesktop() {
  try {
    await relaunch();
  } catch (error) {
    notify("error", "重启失败", error);
  }
}

function toggleAutoUpdateChecks() {
  updatePreferences.autoCheckUpdates = !updatePreferences.autoCheckUpdates;
  saveClientState("保存升级偏好").catch((error) => appendLog(`保存升级偏好失败：${error}`));
}

function toggleAutoInstallUpdates() {
  updatePreferences.autoInstallUpdates = !updatePreferences.autoInstallUpdates;
  saveClientState("保存升级偏好").catch((error) => appendLog(`保存升级偏好失败：${error}`));
}

function saveConfig(label = "保存配置") {
  return run(label, async () => {
    const result = await invoke("save_config", { options: options() });
    await invoke("save_client_state", { state: clientStatePayload() });
    return result;
  });
}

function applyConfigSnapshot(snapshot) {
  connection.value.workDir = snapshot.workDir || connection.value.workDir;
  connection.value.selectedChannel = snapshot.selectedChannel || connection.value.selectedChannel;
  connection.value.selectedAgent = snapshot.selectedAgent || connection.value.selectedAgent;
  if (snapshot.channelFields && connection.value.channelFields[connection.value.selectedChannel]) {
    Object.assign(connection.value.channelFields[connection.value.selectedChannel], snapshot.channelFields);
  }
  if (snapshot.agentFields && connection.value.agentFields[connection.value.selectedAgent]) {
    Object.assign(connection.value.agentFields[connection.value.selectedAgent], snapshot.agentFields);
  }
}

function loadConfigFromFile() {
  return run("读取配置", async () => {
    const snapshot = await invoke("load_config_snapshot", { options: options() });
    applyConfigSnapshot(snapshot);
    await invoke("save_client_state", { state: clientStatePayload() });
    return `已读取 ${snapshot.selectedChannel} / ${snapshot.selectedAgent}`;
  });
}

async function checkAgent(agent) {
  const fields = connection.value.agentFields[agent.id] ?? {};
  const result = await invoke("check_agent", {
    options: {
      agentType: agent.id,
      command: fields.command ?? agent.command ?? ""
    }
  });
  agentChecks[agent.id] = result;
  return result;
}

function checkAllAgents() {
  return run("检测 Agent", async () => {
    const results = await Promise.all(AGENTS.map((agent) => checkAgent(agent)));
    const installed = results.filter((item) => item.installed).length;
    return `${installed}/${results.length} 可用`;
  });
}

function applyQrSetupStatus(next) {
  if (!next) return;
  qrSetup.visible = true;
  qrSetup.sessionId = next.sessionId;
  qrSetup.platform = next.platform;
  qrSetup.state = next.state;
  qrSetup.message = next.message;
  qrSetup.userCode = next.userCode || "";
  qrSetup.qrUrl = next.qrUrl || "";
  qrSetup.qrSvg = next.qrSvg || "";
  qrSetup.output = next.output || [];
  qrSetup.done = next.done;
  qrSetup.success = next.success;
}

function stopQrPolling() {
  window.clearInterval(qrPollTimer);
  qrPollTimer = 0;
}

function closeQrPanel() {
  resetQrPanel();
}

function resetQrPanel() {
  stopQrPolling();
  qrSetup.visible = false;
  qrSetup.sessionId = "";
  qrSetup.platform = "";
  qrSetup.state = "idle";
  qrSetup.message = "";
  qrSetup.userCode = "";
  qrSetup.qrUrl = "";
  qrSetup.qrSvg = "";
  qrSetup.output = [];
  qrSetup.done = false;
  qrSetup.success = null;
}

function startQrPolling(sessionId) {
  stopQrPolling();
  qrPollTimer = window.setInterval(async () => {
    try {
      const next = await invoke("get_qr_setup_status", { sessionId });
      applyQrSetupStatus(next);
      if (next?.done) {
        stopQrPolling();
        await refreshStatus();
        if (next.success) {
          await loadConfigFromFile();
        }
      }
    } catch (error) {
      appendLog(`扫码状态刷新失败：${error}`);
      stopQrPolling();
    }
  }, 1500);
}

async function scanBind() {
  if (!canScan.value) {
    appendLog(`${currentChannel.value.label}: 不支持扫码绑定，请填写密钥或 token。`);
    return;
  }
  appendLog(`准备在客户端打开 ${currentChannel.value.label} 扫码绑定...`);
  return run("扫码绑定", async () => {
    await invoke("save_config", { options: options() });
    const next = await invoke("start_qr_setup", { options: options() });
    applyQrSetupStatus(next);
    startQrPolling(next.sessionId);
    return "已在客户端打开扫码面板";
  });
}

function validateConfig() {
  return run("校验", () => invoke("validate_config", { options: options() }));
}

function startBridge() {
  if (activeTargetLock.value) {
    notify("warning", "接收目标已被占用", `${activeTargetLock.value.connectionName} 正在使用该目标连接到 ${activeTargetLock.value.agentLabel}`);
    return;
  }
  return run("启动", async () => {
    await invoke("save_config", { options: options() });
    await invoke("save_client_state", { state: clientStatePayload() });
    const result = await invoke("start_bridge", { options: options() });
    await refreshBridgeRuntime();
    return result;
  });
}

function stopBridge() {
  return run("停止", async () => {
    const result = await invoke("stop_bridge");
    await refreshBridgeRuntime();
    return result;
  });
}

function refreshBridgeLogs() {
  return run("刷新 Bridge 日志", async () => {
    const logs = await invoke("read_bridge_logs", { maxLines: 180 });
    appendLog(`Bridge 日志\n${logs.combined || "暂无 Bridge 日志"}`);
    return "已加载最新 Bridge/Agent 交互日志";
  });
}

function openAbout() {
  activeTab.value = "about";
}

function refreshDiscoveredTargets() {
  return run("刷新发现目标", async () => {
    await refreshBridgeRuntime();
    if (!bridgeRuntime.running) {
      return `Bridge 未启动。请先在“连接”页点击“启动”，再从 ${currentChannel.value.label} 给机器人或目标通道发一条消息。`;
    }
    const items = await invoke("discover_channel_targets", { options: options() });
    const added = applyDiscoveredTargets(items || []);
    await saveClientState("保存发现目标");
    if (!items?.length) {
      return "暂未发现目标。请先在当前 Channel 里给机器人发送一条消息。";
    }
    return `发现 ${items.length} 个目标，新增 ${added} 个`;
  });
}

function refreshTargetsIfEmpty() {
  if (!activeTarget.value.receiveId && !activeTarget.value.webhookUrl) {
    refreshDiscoveredTargets();
  }
}

async function sendTest() {
  ensureTargets();
  if (!canDirectSendTest.value) {
    notify("warning", "当前 Channel 不支持直发测试", sendTestHelpText.value);
    return;
  }
  return run("发送测试", async () => {
    const payload = {
      session_key: activeTarget.value.sessionKey,
      user_id: activeTarget.value.userId,
      user_name: activeTarget.value.userName || "Desktop Client",
      message_type: currentTest.value.messageType,
      content: currentTest.value.content,
      reply_ctx: "client-test",
      attachments:
        currentTest.value.messageType === "location"
          ? [{ kind: "location", text: "client location", metadata: { lat: 31.2, lng: 121.5 } }]
          : []
    };
    const result = await invoke("send_channel_message", {
      request: {
        options: options(),
        target: { ...activeTarget.value },
        content: currentTest.value.content,
        messageType: currentTest.value.messageType,
        payload
      }
    });
    return `${result.status} ${result.body}`;
  });
}

watch(
  () => [
    activeConnectionId.value,
    connection.value.exePath,
    connection.value.configPath,
    connection.value.project,
    connection.value.workDir,
    connection.value.operationMode,
    connection.value.selectedChannel,
    connection.value.selectedAgent
  ],
  () => refreshStatus()
);

watch(
  () => [activeConnectionId.value, connection.value.selectedAgent],
  () => {
    checkAgent(currentAgent.value).catch((error) => appendLog(`Agent 检测失败：${error}`));
  }
);

watch(
  () => [activeConnectionId.value, connection.value.selectedChannel],
  () => {
    if (qrSetup.visible && qrSetup.platform !== connection.value.selectedChannel) {
      resetQrPanel();
    }
  }
);

watch(() => state.connections, persistSoon, { deep: true });
watch(activeConnectionId, persistSoon);
watch(() => updatePreferences.autoCheckUpdates, persistSoon);
watch(() => updatePreferences.autoInstallUpdates, persistSoon);

loadClientState();
appendLog("客户端已就绪：可以创建多个连接，并分别选择 Channel 与 Agent。");

listen("tray-show-about", () => {
  activeTab.value = "about";
});
listen("tray-start-bridge", () => {
  if (!bridgeRuntime.running) startBridge();
});
listen("tray-stop-bridge", () => {
  if (bridgeRuntime.running) stopBridge();
});
listen("update-download-event", (event) => {
  const payload = event.payload || {};
  if (payload.event === "Started") {
    updateStatus.downloaded = 0;
    updateStatus.total = payload.data?.contentLength || 0;
  } else if (payload.event === "Progress") {
    if (payload.data?.contentLength) updateStatus.total = payload.data.contentLength;
    updateStatus.downloaded += payload.data?.chunkLength || 0;
  } else if (payload.event === "Finished") {
    updateStatus.downloaded = updateStatus.total || updateStatus.downloaded;
  }
});
</script>

<template>
  <main class="shell">
    <div v-if="updateDialog.visible" class="update-dialog-overlay" role="dialog" aria-modal="true" aria-labelledby="update-dialog-title">
      <div class="update-dialog">
        <h3 id="update-dialog-title">发现新版本</h3>
        <p class="update-dialog-version">v{{ updateDialog.version }} 已发布，当前版本 v{{ updateStatus.appVersion || "-" }}。</p>
        <p v-if="updateDialog.date" class="update-dialog-meta">发布时间：{{ updateDialog.date }}</p>
        <div v-if="updateDialog.body" class="update-notes">{{ updateDialog.body }}</div>
        <div v-if="updateStatus.installing" class="update-progress">
          <div><span :style="{ width: `${updateProgressPercent || 35}%` }"></span></div>
          <strong>{{ updateProgressPercent ? `正在下载并安装 ${updateProgressPercent}%` : "正在下载并安装" }}</strong>
        </div>
        <div class="action-row">
          <button type="button" :disabled="updateStatus.installing" @click="installFromUpdateDialog">立即更新</button>
          <button type="button" class="secondary" :disabled="updateStatus.installing" @click="closeUpdateDialog">稍后</button>
        </div>
      </div>
    </div>

    <div class="toast-stack" aria-live="polite" aria-atomic="false">
      <div v-for="toast in toasts" :key="toast.id" class="toast" :class="toast.type">
        <div>
          <strong>{{ toast.title }}</strong>
          <p v-if="toast.detail">{{ toast.detail }}</p>
        </div>
        <button type="button" class="toast-close" aria-label="关闭提示" @click="dismissToast(toast.id)">×</button>
      </div>
    </div>

    <nav class="tabs" aria-label="主导航">
      <button type="button" class="tab" :class="{ active: activeTab === 'connect' }" @click="activeTab = 'connect'">连接</button>
      <button type="button" class="tab" :class="{ active: activeTab === 'channel' }" @click="activeTab = 'channel'">Channel</button>
      <button type="button" class="tab" :class="{ active: activeTab === 'agent' }" @click="activeTab = 'agent'">Agent</button>
      <button type="button" class="tab" :class="{ active: activeTab === 'about' }" @click="activeTab = 'about'">关于我们</button>
      <span class="nav-spacer"></span>
      <button type="button" class="small-button nav-button" @click="hideToTray">隐藏到托盘</button>
      <div v-if="showBusy" class="run-state" :class="busy">{{ busyLabel }}</div>
    </nav>

    <section v-show="activeTab === 'connect'" class="tab-panel">
      <section class="grid connect-layout">
        <div class="panel">
          <div class="section-head">
            <div>
              <h2>连接列表</h2>
              <p>每个连接绑定一套项目空间、配置文件、Channel 和 Agent。</p>
            </div>
          </div>

          <div class="connection-list">
            <button
              v-for="item in state.connections"
              :key="item.id"
              type="button"
              class="connection-card"
              :class="{ active: item.id === activeConnectionId }"
              @click="chooseConnection(item.id)"
            >
              <strong>{{ item.name }}</strong>
              <span>{{ item.project }} · {{ item.selectedChannel }} · {{ item.selectedAgent }}</span>
            </button>
          </div>

          <div class="action-row">
            <button type="button" @click="addConnection">新建</button>
            <button type="button" class="secondary" @click="duplicateConnection">复制</button>
            <button type="button" class="danger" @click="removeConnection">删除</button>
          </div>
        </div>

        <div class="panel">
          <div class="section-head">
            <div>
              <h2>运行目标</h2>
              <p>当前连接会使用下面这组 Channel 和 Agent 启动。</p>
            </div>
            <button type="button" class="small-button" @click="refreshStatus">刷新</button>
          </div>

          <label>连接名称<input v-model="connection.name" /></label>
          <label>AgentLink 可执行文件
            <div class="input-with-button">
              <input v-model="connection.exePath" />
              <button type="button" class="small-button" @click="pickPath('exe')">选择</button>
            </div>
          </label>
          <label>config
            <div class="input-with-button">
              <input v-model="connection.configPath" />
              <button type="button" class="small-button" @click="pickPath('config')">选择</button>
            </div>
          </label>
          <div class="grid form-two">
            <label>project<input v-model="connection.project" /></label>
            <label>workspace
              <div class="input-with-button">
                <input v-model="connection.workDir" />
                <button type="button" class="small-button" @click="pickPath('folder')">选择</button>
              </div>
            </label>
          </div>
          <p class="hint">
            {{ IS_DEV_LAYOUT ? "开发态默认使用仓库里的 target/release 和 examples 相对路径。" : "正式版默认不预填 CLI 和配置文件路径，请先手动选择本机路径。" }}
          </p>
          <label>
            <span class="label-row">操作方式 <span class="help-dot" :title="SELECT_HELP.operationMode">?</span></span>
            <select v-model="connection.operationMode">
              <option value="api">接口优先</option>
              <option value="cli">命令行兼容</option>
            </select>
          </label>

          <div class="summary-grid">
            <div class="summary-card">
              <span>当前 Channel</span>
              <strong>{{ currentChannel.label }}</strong>
              <em :class="{ good: status.bindingReady, bad: !status.bindingReady }">{{ labelStatus(status.bindingStatus) }}</em>
              <button type="button" class="link-button" @click="activeTab = 'channel'">去配置</button>
            </div>
            <div class="summary-card">
              <span>当前 Agent</span>
              <strong>{{ currentAgent.label }}</strong>
              <em :class="{ good: status.agentInstalled, bad: !status.agentInstalled }">{{ labelStatus(status.agentInstallStatus) }}</em>
              <button type="button" class="link-button" @click="activeTab = 'agent'">去配置</button>
            </div>
          </div>

          <div class="action-row">
            <button type="button" class="secondary" @click="resetDefaultPaths">使用默认路径</button>
            <button type="button" @click="saveConfig()">保存配置</button>
            <button type="button" class="secondary" @click="saveLocalState">保存本地数据</button>
            <button type="button" class="secondary" @click="loadConfigFromFile">读取配置</button>
            <button type="button" @click="validateConfig">校验</button>
            <button type="button" :disabled="!canStartBridge" @click="startBridge">{{ bridgeRuntime.running ? "已启动" : "启动" }}</button>
            <button type="button" class="secondary" :disabled="!bridgeRuntime.running || operationRunning" @click="stopBridge">停止</button>
          </div>
        </div>

        <div class="panel status-panel">
          <div class="section-head">
            <div>
              <h2>状态</h2>
              <p>{{ status.summary }}</p>
            </div>
          </div>

          <div class="status-grid">
            <div class="state-card"><span>AgentLink 可执行文件</span><strong :class="pathStateClass('exe')">{{ pathStateLabel('exe') }}</strong></div>
            <div class="state-card"><span>Bridge 运行</span><strong :class="{ good: bridgeRuntime.running, bad: !bridgeRuntime.running }">{{ bridgeRuntime.running ? `PID: ${bridgeRuntime.pid}` : "未启动" }}</strong></div>
            <div class="state-card"><span>config</span><strong :class="pathStateClass('config')">{{ pathStateLabel('config') }}</strong></div>
            <div class="state-card"><span>project</span><strong :class="{ good: status.projectConfigured, bad: !status.projectConfigured }">{{ labelStatus(status.connectionStatus) }}</strong></div>
            <div class="state-card"><span>channel</span><strong :class="{ good: status.channelConfigured, bad: !status.channelConfigured }">{{ labelStatus(status.channelStatus) }}</strong></div>
            <div class="state-card"><span>agent 配置</span><strong :class="{ good: status.agentConfigured, bad: !status.agentConfigured }">{{ labelStatus(status.agentStatus) }}</strong></div>
            <div class="state-card"><span>agent 安装</span><strong :class="{ good: status.agentInstalled, bad: !status.agentInstalled }">{{ labelStatus(status.agentInstallStatus) }}</strong></div>
            <div class="state-card"><span>绑定</span><strong :class="{ good: status.bindingReady, bad: !status.bindingReady }">{{ labelStatus(status.bindingStatus) }}</strong></div>
          </div>
        </div>

      </section>
    </section>

    <section v-show="activeTab === 'channel'" class="tab-panel">
      <section class="grid two-columns wide-left">
        <div class="panel">
          <div class="section-head">
            <div>
              <h2>Channel</h2>
              <p>点击选择聊天渠道，卡片会显示当前填写或绑定状态。</p>
            </div>
            <span class="pill">{{ currentChannel.id }}</span>
          </div>
          <div class="select-list channel-list">
            <button
              v-for="channel in CHANNELS"
              :key="channel.id"
              type="button"
              class="select-card"
              :class="{ active: connection.selectedChannel === channel.id }"
              @click="chooseChannel(channel)"
            >
              <strong>{{ channel.label }}</strong>
              <span>{{ channel.kind }}</span>
              <em class="badge" :class="channelBadgeClass(channel)">{{ channelBadge(channel) }}</em>
            </button>
          </div>
        </div>

        <div class="panel">
          <div class="section-head">
            <div>
              <h2>{{ currentChannel.label }}</h2>
              <p>{{ currentChannel.summary }}</p>
            </div>
            <span class="pill" :class="{ good: status.bindingReady, bad: !status.bindingReady }">{{ labelStatus(status.bindingStatus) }}</span>
          </div>

          <div class="bind-row">
            <button type="button" class="toggle active">填写密钥</button>
            <button type="button" class="toggle" :disabled="!canScan" @click="scanBind">扫码绑定</button>
          </div>

          <div class="field-grid">
            <label v-for="field in currentChannel.fields" :key="field[0]">
              <span class="label-row">
                {{ field[1] }}
                <span v-if="fieldHelp(field[0])" class="help-dot" :title="fieldHelp(field[0])">?</span>
              </span>
              <select v-if="field[3] === 'select'" v-model="currentChannelFields[field[0]]">
                <option v-for="option in field[4]" :key="option" :value="option">{{ option }}</option>
              </select>
              <input v-else :type="field[3] || 'text'" v-model="currentChannelFields[field[0]]" />
            </label>
          </div>

          <div class="hint">
            {{ canScan ? "该通道支持扫码绑定。点击后会先保存配置，并在客户端内打开二维码面板。" : "该通道不支持扫码绑定，请填写密钥或 token。" }}
          </div>

          <section v-if="qrPanelVisible" class="qr-panel">
            <div class="section-head compact-head">
              <div>
                <h2>扫码绑定</h2>
                <p>{{ qrSetup.message }}</p>
              </div>
              <button type="button" class="small-button" @click="closeQrPanel">关闭</button>
            </div>
            <div class="qr-content">
              <div v-if="qrSetup.qrSvg" class="qr-box" v-html="qrSetup.qrSvg"></div>
              <div v-else class="qr-placeholder">
                {{ qrSetup.done ? "没有返回二维码" : "正在等待二维码..." }}
              </div>
              <div class="qr-meta">
                <span class="pill" :class="{ good: qrSetup.success === true, bad: qrSetup.success === false }">{{ qrSetup.state }}</span>
                <strong v-if="qrSetup.userCode" class="user-code">用户码：{{ qrSetup.userCode }}</strong>
                <a v-if="qrSetup.qrUrl" :href="qrSetup.qrUrl" target="_blank" rel="noreferrer">{{ qrSetup.qrUrl }}</a>
                <p>{{ qrHelpText }}</p>
              </div>
            </div>
            <pre class="qr-output">{{ qrSetup.output.join('\n') }}</pre>
          </section>

          <div class="action-row">
            <button type="button" :disabled="!canScan" @click="scanBind">扫码绑定</button>
            <button type="button" class="secondary" :disabled="!hasReusableChannelConfig" @click="reuseChannelConfig">复用已配置 Channel</button>
            <button type="button" @click="saveConfig('保存 Channel')">保存 Channel</button>
            <button type="button" class="secondary" @click="refreshStatus">检测状态</button>
          </div>
        </div>
      </section>

      <section class="panel test-panel">
        <h2>接收目标与测试</h2>
        <p>接收目标就是一个用户、群或 Webhook。一个接收目标同一时间只能绑定到一个连接的 Agent，已被占用的目标会在下拉里置灰。</p>
        <div class="grid form-two">
          <label>当前 Channel<input :value="currentChannel.label" readonly /></label>
          <label>
            <span class="label-row">接收目标 <span class="help-dot" :title="SELECT_HELP.channelTarget">?</span></span>
            <select :value="activeTargetKey" @change="chooseTargetByKey($event.target.value)">
              <option v-if="channelTargetOptions.length === 0" value="" disabled>尚未发现目标，请先刷新发现目标</option>
              <option v-for="item in channelTargetOptions" :key="item.key" :value="item.key" :disabled="Boolean(item.lockedBy)">
                {{ targetLabel(item.target) }}{{ item.lockedBy ? `（已被 ${item.lockedBy.connectionName} / ${item.lockedBy.agentLabel} 使用）` : "" }}
              </option>
            </select>
          </label>
        </div>
        <div v-if="activeTargetLock" class="hint warning-hint">
          当前目标已被 {{ activeTargetLock.connectionName }} 的 {{ activeTargetLock.agentLabel }} 使用。一个接收目标同一时刻只允许对应一个 Agent。
        </div>
        <div class="grid form-two">
          <label>目标名称<input v-model="activeTarget.name" /></label>
          <label>
            <span class="label-row">接收 ID 类型 <span class="help-dot" :title="SELECT_HELP.receiveIdType">?</span></span>
            <select v-model="activeTarget.receiveIdType">
              <option value="chat_id">chat_id</option>
              <option value="open_id">open_id</option>
              <option value="user_id">user_id</option>
              <option value="group_id">group_id</option>
              <option value="email">email</option>
              <option value="channel_id">channel_id</option>
              <option value="session_webhook">session_webhook</option>
              <option value="webhook">webhook</option>
            </select>
          </label>
        </div>
        <div class="grid form-two">
          <label>接收 ID<input v-model="activeTarget.receiveId" placeholder="为空时点击会尝试从已收到的消息里发现" @focus="refreshTargetsIfEmpty" /></label>
          <label>HTTP/Webhook<input v-model="activeTarget.webhookUrl" placeholder="HTTP webhook 或钉钉 sessionWebhook" /></label>
        </div>
        <details class="advanced-box" :open="targetAdvancedOpen" @toggle="targetAdvancedOpen = $event.target.open">
          <summary>高级路由字段</summary>
          <p>这些字段用于内部会话路由和回复原消息。通常通过“刷新发现目标”自动填充。</p>
          <div class="grid form-two">
            <label>session_key<input v-model="activeTarget.sessionKey" /></label>
            <label>user_id<input v-model="activeTarget.userId" /></label>
          </div>
          <div class="grid form-two">
            <label>
              <span class="label-row">message type <span class="help-dot" :title="SELECT_HELP.messageType">?</span></span>
              <select v-model="currentTest.messageType">
                <option value="text">text</option>
                <option value="image">image</option>
                <option value="file">file</option>
                <option value="audio">audio</option>
                <option value="video">video</option>
                <option value="location">location</option>
                <option value="card">card</option>
                <option value="sticker">sticker</option>
                <option value="raw">raw</option>
                <option value="mixed">mixed</option>
              </select>
            </label>
            <label>message id<input v-model="activeTarget.messageId" placeholder="可选：回复某条消息" /></label>
          </div>
        </details>
        <label>content<textarea v-model="currentTest.content"></textarea></label>
        <div class="action-row">
          <button type="button" :disabled="Boolean(activeTargetLock) || !canDirectSendTest" @click="sendTest">发送测试</button>
          <button type="button" class="secondary" @click="refreshDiscoveredTargets">刷新发现目标</button>
          <button type="button" class="secondary" @click="addTarget">新增目标</button>
          <button type="button" class="danger" @click="removeTarget">删除目标</button>
        </div>
        <div class="hint">
          {{ sendTestHelpText }}
        </div>
        <div class="hint">
          推荐流程：先启动 bridge，然后在当前聊天渠道里给机器人或目标通道发一条消息，再点击“刷新发现目标”。客户端会从已收到的消息里解析用户、群和会话信息。
        </div>
      </section>
    </section>

    <section v-show="activeTab === 'agent'" class="tab-panel">
      <section class="grid two-columns wide-left">
        <div class="panel">
          <div class="section-head">
            <div>
              <h2>Agent</h2>
              <p>选择编程 Agent。当前 Agent 会检测命令是否存在。</p>
            </div>
            <span class="pill">{{ currentAgent.id }}</span>
          </div>
          <div class="select-list agent-list">
            <button
              v-for="agent in AGENTS"
              :key="agent.id"
              type="button"
              class="select-card"
              :class="{ active: connection.selectedAgent === agent.id }"
              @click="chooseAgent(agent)"
            >
              <strong>{{ agent.label }}</strong>
              <span>{{ agent.kind }}</span>
              <em class="badge" :class="agentBadgeClass(agent)">{{ agentBadge(agent) }}</em>
            </button>
          </div>
        </div>

        <div class="panel">
          <div class="section-head">
            <div>
              <h2>{{ currentAgent.label }}</h2>
              <p>{{ currentAgent.summary || "通过 CLI 命令接入。" }}</p>
            </div>
            <span class="pill" :class="{ good: status.agentInstalled, bad: !status.agentInstalled }">{{ labelStatus(status.agentInstallStatus) }}</span>
          </div>

          <div v-if="connection.selectedAgent === 'codex'" class="field-stack">
            <div class="grid form-two">
              <label>
                <span class="label-row">backend <span class="help-dot" :title="SELECT_HELP.codexBackend">?</span></span>
                <select v-model="currentAgentFields.backend">
                  <option value="exec">exec</option>
                  <option value="app-server">app-server</option>
                </select>
              </label>
              <label>
                <span class="label-row">mode <span class="help-dot" :title="SELECT_HELP.codexMode">?</span></span>
                <select v-model="currentAgentFields.mode">
                  <option value="suggest">suggest</option>
                  <option value="auto-edit">auto-edit</option>
                  <option value="full-auto">full-auto</option>
                  <option value="yolo">yolo</option>
                </select>
              </label>
            </div>
            <label>codex_bin<input v-model="currentAgentFields.command" /></label>
            <div class="grid form-two">
              <label><span class="label-row">model <span class="help-dot" :title="SELECT_HELP.agentModel">?</span></span><input v-model="currentAgentFields.model" placeholder="可为空" /></label>
              <label>reasoning_effort<input v-model="currentAgentFields.reasoningEffort" placeholder="medium" /></label>
            </div>
            <label><span class="label-row">extra args <span class="help-dot" :title="SELECT_HELP.agentArgs">?</span></span><textarea v-model="currentAgentFields.args" placeholder="每行一个额外参数"></textarea></label>
          </div>

          <div v-else class="field-stack">
            <label>command<input v-model="currentAgentFields.command" /></label>
            <div class="grid form-two">
              <label><span class="label-row">model <span class="help-dot" :title="SELECT_HELP.agentModel">?</span></span><input v-model="currentAgentFields.model" placeholder="可为空" /></label>
              <label>mode<input v-model="currentAgentFields.mode" placeholder="可为空" /></label>
            </div>
            <label><span class="label-row">args <span class="help-dot" :title="SELECT_HELP.agentArgs">?</span></span><textarea v-model="currentAgentFields.args" placeholder="每行一个参数，支持 {prompt} / {session_id} / {model} / {mode}"></textarea></label>
          </div>

          <div class="hint">当前运行会使用 {{ currentAgent.label }} 处理来自 {{ currentChannel.label }} 的消息。</div>
          <div class="action-row">
            <button type="button" @click="saveConfig('保存 Agent')">保存 Agent</button>
            <button type="button" class="secondary" @click="checkAgent(currentAgent)">检测当前</button>
            <button type="button" class="secondary" @click="checkAllAgents">检测全部</button>
          </div>
        </div>
      </section>
    </section>

    <section v-show="activeTab === 'about'" class="tab-panel">
      <section class="panel about-panel">
        <div class="section-head">
          <div class="about-head">
            <img :src="appLogo" alt="AgentLink logo" class="about-logo" />
            <div>
              <h2>AgentLink</h2>
              <p>多 Agent 聊天连接器</p>
            </div>
          </div>
          <span class="pill">v{{ updateStatus.appVersion || "0.1.0" }}</span>
        </div>

        <div class="about-grid">
          <section class="about-update-section">
            <h2>版本与更新</h2>
            <div class="about-stats">
              <div class="about-stat">
                <span>当前版本</span>
                <strong>v{{ updateStatus.appVersion || "0.1.0" }}</strong>
              </div>
              <div class="about-stat">
                <span>更新状态</span>
                <strong :class="{ good: updateStatus.available || updateStatus.installed, bad: updateStatus.error }">{{ updateStateText }}</strong>
              </div>
              <div class="about-stat">
                <span>最新版本</span>
                <strong>{{ updateStatus.available?.version || "-" }}</strong>
              </div>
              <div class="about-stat">
                <span>上次检查</span>
                <strong>{{ updatePreferences.lastUpdateCheckAt || "-" }}</strong>
              </div>
            </div>
            <div v-if="updateStatus.available?.body" class="update-notes">{{ updateStatus.available.body }}</div>
            <div v-if="updateStatus.error" class="hint warning-hint">{{ updateStatus.error }}</div>
            <div v-if="updateStatus.installing" class="update-progress">
              <div><span :style="{ width: `${updateProgressPercent || 35}%` }"></span></div>
              <strong>{{ updateProgressPercent ? `正在下载并安装 ${updateProgressPercent}%` : "正在下载并安装" }}</strong>
            </div>
            <label class="checkbox-row">
              <input type="checkbox" :checked="updatePreferences.autoCheckUpdates" @change="toggleAutoUpdateChecks" />
              启动后自动检查更新
            </label>
            <label class="checkbox-row">
              <input type="checkbox" :checked="updatePreferences.autoInstallUpdates" @change="toggleAutoInstallUpdates" />
              发现新版本后自动下载安装
            </label>
            <div class="action-row">
              <button type="button" class="small-button" :disabled="updateStatus.checking || updateStatus.installing" @click="checkForUpdates(true)">
                {{ updateStatus.checking ? "检查中" : "检查更新" }}
              </button>
              <button type="button" class="small-button" :disabled="!updateStatus.available || updateStatus.installing || updateStatus.installed" @click="installAvailableUpdate">下载并安装</button>
              <button type="button" class="small-button" :disabled="!updateStatus.error || updateStatus.checking || updateStatus.installing" @click="checkForUpdates(true)">重试</button>
              <button type="button" class="small-button" :disabled="!updateStatus.installed" @click="relaunchDesktop">重启生效</button>
            </div>
          </section>
          <section>
            <h2>定位</h2>
            <p>AgentLink 用来把飞书、钉钉、微信、Telegram 等聊天渠道连接到 Codex、Claude Code、Gemini、Cursor Agent 等编程 Agent，并在本地统一管理渠道密钥、接收目标、Agent 会话和运行状态。</p>
          </section>
          <section>
            <h2>使用步骤</h2>
            <ol class="steps">
              <li>在“连接”页确认 AgentLink exe、配置文件和 workspace。</li>
              <li>在“Channel”页选择渠道，填写密钥或扫码绑定。</li>
              <li>启动 Bridge 后，从真实聊天渠道给机器人发一条消息。</li>
              <li>点击“刷新发现目标”，选择要绑定的用户、群或 Webhook。</li>
              <li>在“Agent”页选择 Codex 或其它编程 Agent，并检测命令是否可用。</li>
              <li>回到“连接”页启动，聊天消息会进入对应 Agent，结果再回到渠道。</li>
            </ol>
          </section>
          <section>
            <h2>托盘行为</h2>
            <p>点击窗口关闭按钮不会退出程序，而是隐藏到右下角托盘。托盘菜单可以显示窗口、打开关于、触发启动/停止 Bridge，只有点击“退出 AgentLink”才会真正退出并停止后台 Bridge。</p>
          </section>
          <section>
            <h2>设计约束</h2>
            <p>一个接收目标同一时刻只能绑定一个 Agent，避免同一个群或用户的消息被多个 Agent 同时处理。高级路由字段会保留在界面里，但日常使用只需要关注 Channel、接收目标和 Agent。</p>
          </section>
        </div>
      </section>
    </section>

    <section class="log-panel">
      <div class="section-head compact-head">
        <h2>日志</h2>
        <div class="action-row inline-actions">
          <button type="button" class="small-button" @click="refreshBridgeLogs">刷新 Bridge 日志</button>
          <button type="button" class="small-button" @click="logText = ''">清空</button>
        </div>
      </div>
      <pre id="log">{{ logText }}</pre>
    </section>
  </main>
</template>
