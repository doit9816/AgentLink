# Bridge WebSocket 协议

这是 `agentlink` 的通用聊天渠道协议，用于让外部聊天渠道适配器通过 WebSocket 接入 AgentLink。

目标是让任何外部聊天渠道适配器都可以用 WebSocket 接入：

```text
聊天渠道 Adapter <-> Bridge WebSocket <-> Engine <-> AgentSession <-> Codex/其他 Agent
```

## 启动配置

```toml
[[projects.platforms]]
type = "bridge"

[projects.platforms.options]
name = "bridge"
listen = "127.0.0.1:9810"
path = "/bridge/ws"
token = "change-me"
# insecure = true
```

连接地址：

```text
ws://127.0.0.1:9810/bridge/ws?token=change-me
```

认证支持：

- URL 参数：`?token=<token>`
- 请求头：`Authorization: Bearer <token>`
- 请求头：`X-Bridge-Token: <token>`

## Adapter -> Bridge

### register

外部适配器连接后第一条消息必须注册平台名。

```json
{
  "type": "register",
  "platform": "wechat",
  "capabilities": ["text"],
  "metadata": {
    "version": "1.0.0"
  }
}
```

Bridge 返回：

```json
{
  "type": "register_ack",
  "ok": true,
  "error": ""
}
```

### message

外部适配器把聊天消息传给 Engine。

```json
{
  "type": "message",
  "msg_id": "msg-001",
  "session_key": "wechat:room-1:user-1",
  "user_id": "user-1",
  "user_name": "Alice",
  "message_type": "mixed",
  "content": "帮我看下这个项目",
  "attachments": [
    {
      "kind": "image",
      "mime_type": "image/png",
      "data": "base64...",
      "file_name": "screenshot.png"
    },
    {
      "kind": "location",
      "text": "office",
      "metadata": {
        "lat": 31.2,
        "lng": 121.5
      }
    }
  ],
  "reply_ctx": "opaque-channel-context"
}
```

字段约定：

- `session_key`：稳定会话键，建议格式为 `{adapter}:{scope}:{user}`。
- `message_type`：支持 `text`、`image`、`file`、`audio`、`video`、`location`、`card`、`sticker`、`mixed`、`event`、`raw`。
- `attachments[].kind`：支持 `image`、`file`、`audio`、`video`、`location`、`card`、`sticker`、`raw`。
- `attachments[].data`：base64 字符串。大文件建议先落本地或对象存储，再传 `path` / `url`。
- `reply_ctx`：适配器自己的不透明上下文，Bridge 会在回复里原样带回。
- `/allow <approval_id>` 和 `/deny <approval_id>` 作为审批命令处理，不写入 Agent 对话历史。

### ping

```json
{
  "type": "ping",
  "ts": 1710000000000
}
```

Bridge 返回：

```json
{
  "type": "pong",
  "ts": 1710000000000
}
```

## Bridge -> Adapter

### reply

Engine 的最终回复会推送给原适配器。

```json
{
  "type": "reply",
  "session_key": "wechat:room-1:user-1",
  "reply_ctx": "opaque-channel-context",
  "content": "最终回复内容",
  "format": "text"
}
```

### error

```json
{
  "type": "error",
  "code": "bad_message",
  "message": "错误说明"
}
```

## 适配器实现要点

真实聊天渠道只需要做三件事：

1. 连接 Bridge WebSocket 并发送 `register`。
2. 收到渠道消息后组装 `message` 发给 Bridge。
3. 收到 Bridge 的 `reply` 后，调用真实渠道的发送接口。

v1 默认只发最终结果，不做流式编辑。后续可以继续扩展 `reply_stream`、`image`、`file`、`typing_start` 等能力。
