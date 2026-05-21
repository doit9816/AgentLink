use super::*;

#[async_trait]
pub(crate) trait SimpleWsPlatform: Platform {
    fn cfg(&self) -> &PollPlatformConfig;
    fn handler_ref(&self) -> &Arc<Mutex<Option<MessageHandler>>>;
}

#[async_trait]
impl SimpleWsPlatform for QqBotPlatform {
    fn cfg(&self) -> &PollPlatformConfig {
        &self.config
    }

    fn handler_ref(&self) -> &Arc<Mutex<Option<MessageHandler>>> {
        &self.handler
    }
}

#[async_trait]
impl SimpleWsPlatform for WeiboPlatform {
    fn cfg(&self) -> &PollPlatformConfig {
        &self.config
    }

    fn handler_ref(&self) -> &Arc<Mutex<Option<MessageHandler>>> {
        &self.handler
    }
}

pub(crate) async fn run_simple_ws(
    platform: Arc<impl SimpleWsPlatform + 'static>,
    mut shutdown_rx: oneshot::Receiver<()>,
    identify: bool,
) {
    loop {
        match run_simple_ws_once(Arc::clone(&platform), &mut shutdown_rx, identify).await {
            Ok(true) => return,
            Ok(false) => {}
            Err(err) => tracing::warn!(error = %err, "websocket channel disconnected"),
        }
        tokio::select! {
            _ = &mut shutdown_rx => return,
            _ = tokio::time::sleep(Duration::from_secs(5)) => {}
        }
    }
}

async fn run_simple_ws_once<P: SimpleWsPlatform + 'static>(
    platform: Arc<P>,
    shutdown_rx: &mut oneshot::Receiver<()>,
    identify: bool,
) -> Result<bool> {
    let token = resolve_platform_token(platform.cfg()).await?;
    let url = websocket_url(platform.cfg(), identify, &token);
    let mut request = url.into_client_request()?;
    if !token.is_empty() {
        request
            .headers_mut()
            .insert("Authorization", format!("Bearer {token}").parse()?);
    }
    let (stream, _) = connect_async(request).await?;
    let (mut write, mut read) = stream.split();
    if identify {
        write
            .send(WsMessage::Text(
                json!({ "op": 2, "d": { "token": token }})
                    .to_string()
                    .into(),
            ))
            .await?;
    }
    loop {
        tokio::select! {
            _ = &mut *shutdown_rx => {
                let _ = write.close().await;
                return Ok(true);
            }
            message = read.next() => {
                let Some(message) = message else { return Ok(false); };
                match message? {
                    WsMessage::Text(text) => {
                        let value: Value = serde_json::from_str(&text)?;
                        let parsed = if identify {
                            qqbot_message_from_value(platform.cfg(), &value)
                        } else {
                            weibo_message_from_value(platform.cfg(), &value)
                        };
                        if let Ok(Some(message)) = parsed {
                            let handler = platform.handler_ref().lock().await.clone();
                            if let Some(handler) = handler {
                                let dyn_platform = platform.clone() as Arc<dyn Platform>;
                                tokio::spawn(async move {
                                    handler(dyn_platform, message).await;
                                });
                            }
                        }
                    }
                    WsMessage::Ping(bytes) => {
                        write.send(WsMessage::Pong(bytes)).await?;
                    }
                    WsMessage::Close(_) => return Ok(false),
                    _ => {}
                }
            }
        }
    }
}

pub(crate) async fn resolve_platform_token(cfg: &PollPlatformConfig) -> Result<String> {
    if !cfg.token.trim().is_empty() {
        return Ok(cfg.token.clone());
    }
    match cfg.name.as_str() {
        "qqbot" => fetch_qqbot_access_token(cfg).await,
        "weibo" => fetch_weibo_ws_token(cfg).await,
        _ => Ok(String::new()),
    }
}

async fn fetch_qqbot_access_token(cfg: &PollPlatformConfig) -> Result<String> {
    let app_id = cfg
        .app_id
        .as_deref()
        .ok_or_else(|| anyhow!("qqbot app_id is required when token is empty"))?;
    let app_secret = cfg
        .app_secret
        .as_deref()
        .ok_or_else(|| anyhow!("qqbot app_secret is required when token is empty"))?;
    let endpoint = cfg
        .token_endpoint
        .as_deref()
        .unwrap_or("https://bots.qq.com/app/getAppAccessToken");
    let value: Value = reqwest::Client::new()
        .post(endpoint)
        .json(&json!({
            "appId": app_id,
            "clientSecret": app_secret
        }))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    value
        .get("access_token")
        .or_else(|| value.get("accessToken"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow!("qqbot token response missing access_token"))
}

async fn fetch_weibo_ws_token(cfg: &PollPlatformConfig) -> Result<String> {
    let app_id = cfg
        .app_id
        .as_deref()
        .ok_or_else(|| anyhow!("weibo app_id is required when token is empty"))?;
    let app_secret = cfg
        .app_secret
        .as_deref()
        .ok_or_else(|| anyhow!("weibo app_secret is required when token is empty"))?;
    let endpoint = cfg
        .token_endpoint
        .as_deref()
        .unwrap_or("https://open-im.api.weibo.com/open/auth/ws_token");
    let value: Value = reqwest::Client::new()
        .post(endpoint)
        .json(&json!({
            "app_id": app_id,
            "app_secret": app_secret
        }))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    value
        .pointer("/data/token")
        .or_else(|| value.get("token"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow!("weibo token response missing data.token"))
}

fn websocket_url(cfg: &PollPlatformConfig, identify: bool, token: &str) -> String {
    let base = cfg
        .ws_endpoint
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(&cfg.api_base);
    if identify {
        return base.to_string();
    }
    let app_id = cfg.app_id.as_deref().unwrap_or("");
    append_query_params(base, &[("app_id", app_id), ("token", token)])
}

fn append_query_params(url: &str, pairs: &[(&str, &str)]) -> String {
    let query = pairs
        .iter()
        .filter(|(_, value)| !value.trim().is_empty())
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("&");
    if query.is_empty() {
        return url.to_string();
    }
    let separator = if url.contains('?') { '&' } else { '?' };
    format!("{url}{separator}{query}")
}

fn qqbot_message_from_value(cfg: &PollPlatformConfig, value: &Value) -> Result<Option<Message>> {
    let event = value.get("d").unwrap_or(value);
    let text = event
        .get("content")
        .or_else(|| event.get("text"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if text.is_empty() {
        return Ok(None);
    }
    let target = event
        .get("group_openid")
        .or_else(|| event.get("channel_id"))
        .or_else(|| event.get("user_openid"))
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let user_id = event
        .get("member_openid")
        .or_else(|| event.get("user_openid"))
        .or_else(|| event.get("author_id"))
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let msg_id = event
        .get("id")
        .or_else(|| event.get("msg_id"))
        .map(value_to_string);
    let target_kind = if event.get("group_openid").is_some() {
        "group"
    } else if event.get("channel_id").is_some() {
        "channel"
    } else {
        "user"
    };
    simple_message(
        &cfg.name,
        format!("{}:{}:{}", cfg.name, target, user_id),
        user_id,
        msg_id.clone().as_deref(),
        text,
        SimpleReplyContext {
            channel: cfg.name.clone(),
            target,
            message_id: msg_id,
            extra: json!({ "target_kind": target_kind }),
        },
    )
}

fn weibo_message_from_value(cfg: &PollPlatformConfig, value: &Value) -> Result<Option<Message>> {
    let text = value
        .get("text")
        .or_else(|| value.pointer("/message/text"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if text.is_empty() {
        return Ok(None);
    }
    let user_id = value
        .get("from_user_id")
        .or_else(|| value.get("sender_id"))
        .or_else(|| value.pointer("/message/from_user_id"))
        .map(value_to_string)
        .unwrap_or_else(|| "unknown".to_string());
    let msg_id = value
        .get("id")
        .or_else(|| value.get("mid"))
        .map(value_to_string);
    simple_message(
        &cfg.name,
        format!("{}:{}", cfg.name, user_id),
        user_id.clone(),
        msg_id.clone().as_deref(),
        text,
        SimpleReplyContext {
            channel: cfg.name.clone(),
            target: user_id,
            message_id: msg_id,
            extra: Value::Null,
        },
    )
}
