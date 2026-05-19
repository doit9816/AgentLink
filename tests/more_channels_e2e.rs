use aes::cipher::{block_padding::Pkcs7, BlockEncryptMut, KeyIvInit};
use agentlink::core::Platform;
use agentlink::mock::MockAgent;
use agentlink::channels::line::{LinePlatform, LinePlatformConfig};
use agentlink::channels::max::{max_config_from_options, MaxPlatform};
use agentlink::channels::qqbot::{qqbot_config_from_options, QqBotPlatform};
use agentlink::channels::wecom::{WeComPlatform, WeComPlatformConfig};
use agentlink::channels::weibo::{weibo_config_from_options, WeiboPlatform};
use agentlink::channels::weixin::{weixin_config_from_options, WeixinPlatform};
use agentlink::{Engine, SessionStore};
use axum::extract::ws::{Message as AxumWsMessage, WebSocketUpgrade};
use axum::extract::Query;
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine as Base64Engine;
use futures_util::StreamExt;
use serde_json::{json, Value};
use sha1::{Digest, Sha1};
use std::collections::BTreeMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::net::TcpListener;
use tokio::time::{timeout, Duration};

#[tokio::test]
async fn line_webhook_text_round_trip_e2e() {
    let platform = LinePlatform::new(LinePlatformConfig {
        name: "line".to_string(),
        listen: "127.0.0.1:0".to_string(),
        callback_path: "/line/webhook".to_string(),
        dry_run: true,
        ..LinePlatformConfig::default()
    });
    let engine = start_engine("test", Arc::clone(&platform) as Arc<dyn Platform>).await;
    let addr = platform.local_addr().await.unwrap();
    let response = reqwest::Client::new()
        .post(format!("http://{addr}/line/webhook"))
        .json(&json!({
            "events": [{
                "type": "message",
                "source": { "type": "user", "userId": "U1" },
                "message": { "id": "LMSG1", "type": "text", "text": "hello line" }
            }]
        }))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    let outbound = timeout(Duration::from_secs(2), platform.wait_for_outbound())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(outbound.target, "U1");
    assert_eq!(outbound.content, "mock: hello line");
    engine.stop().await.unwrap();
}

#[tokio::test]
async fn wecom_webhook_text_round_trip_e2e() {
    let platform = WeComPlatform::new(WeComPlatformConfig {
        name: "wecom".to_string(),
        listen: "127.0.0.1:0".to_string(),
        callback_path: "/wecom/callback".to_string(),
        dry_run: true,
        ..WeComPlatformConfig::default()
    });
    let engine = start_engine("test", Arc::clone(&platform) as Arc<dyn Platform>).await;
    let addr = platform.local_addr().await.unwrap();
    let response = reqwest::Client::new()
        .post(format!("http://{addr}/wecom/callback"))
        .json(&json!({
            "FromUserName": "wecom-user-1",
            "Content": "hello wecom",
            "MsgId": "WECOMMSG1",
            "AgentID": "1000002"
        }))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    let outbound = timeout(Duration::from_secs(2), platform.wait_for_outbound())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(outbound.target, "wecom-user-1");
    assert_eq!(outbound.content, "mock: hello wecom");
    engine.stop().await.unwrap();
}

#[tokio::test]
async fn wecom_encrypted_xml_round_trip_e2e() {
    let aes_key = wecom_test_aes_key();
    let platform = WeComPlatform::new(WeComPlatformConfig {
        name: "wecom".to_string(),
        listen: "127.0.0.1:0".to_string(),
        callback_path: "/wecom/callback".to_string(),
        corp_id: Some("corp-test".to_string()),
        callback_token: Some("token-test".to_string()),
        callback_aes_key: Some(aes_key.clone()),
        dry_run: true,
        ..WeComPlatformConfig::default()
    });
    let engine = start_engine("test", Arc::clone(&platform) as Arc<dyn Platform>).await;
    let addr = platform.local_addr().await.unwrap();
    let plain = r#"<xml><FromUserName><![CDATA[encrypted-user]]></FromUserName><CreateTime>1</CreateTime><MsgType><![CDATA[text]]></MsgType><Content><![CDATA[hello encrypted wecom]]></Content><MsgId>991</MsgId><AgentID>1000002</AgentID></xml>"#;
    let encrypt = wecom_encrypt_for_test(&aes_key, "corp-test", plain);
    let timestamp = "123";
    let nonce = "nonce";
    let sig = wecom_signature_for_test("token-test", timestamp, nonce, &encrypt);
    let outer = format!("<xml><ToUserName><![CDATA[to]]></ToUserName><Encrypt><![CDATA[{encrypt}]]></Encrypt><AgentID><![CDATA[1000002]]></AgentID></xml>");
    let response = reqwest::Client::new()
        .post(format!(
            "http://{addr}/wecom/callback?msg_signature={sig}&timestamp={timestamp}&nonce={nonce}"
        ))
        .body(outer)
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    let outbound = timeout(Duration::from_secs(2), platform.wait_for_outbound())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(outbound.target, "encrypted-user");
    assert_eq!(outbound.content, "mock: hello encrypted wecom");
    engine.stop().await.unwrap();
}

#[tokio::test]
async fn max_long_poll_text_round_trip_e2e() {
    let served = Arc::new(AtomicBool::new(false));
    let app = Router::new().route(
        "/updates",
        get({
            let served = Arc::clone(&served);
            move || {
                let served = Arc::clone(&served);
                async move {
                    if served.swap(true, Ordering::SeqCst) {
                        Json(json!({ "updates": [], "marker": "2" }))
                    } else {
                        Json(json!({
                            "marker": "1",
                            "updates": [{
                                "update_type": "message_created",
                                "message": {
                                    "sender": { "user_id": 7, "name": "alice" },
                                    "recipient": { "chat_id": 42 },
                                    "body": { "mid": "MAXMSG1", "text": "hello max" }
                                }
                            }]
                        }))
                    }
                }
            }
        }),
    );
    let addr = serve(app).await;
    let mut opts = toml::value::Table::new();
    opts.insert("name".to_string(), toml::Value::String("max".to_string()));
    opts.insert(
        "token".to_string(),
        toml::Value::String("MAXTOKEN".to_string()),
    );
    opts.insert(
        "api_base".to_string(),
        toml::Value::String(format!("http://{addr}")),
    );
    opts.insert("dry_run".to_string(), toml::Value::Boolean(true));
    let platform = MaxPlatform::new(max_config_from_options(opts).unwrap());
    let engine = start_engine("test", Arc::clone(&platform) as Arc<dyn Platform>).await;
    let outbound = timeout(Duration::from_secs(2), platform.wait_for_outbound())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(outbound.target, "42");
    assert_eq!(outbound.content, "mock: hello max");
    engine.stop().await.unwrap();
}

#[tokio::test]
async fn max_webhook_text_round_trip_e2e() {
    let mut opts = toml::value::Table::new();
    opts.insert("name".to_string(), toml::Value::String("max".to_string()));
    opts.insert(
        "token".to_string(),
        toml::Value::String("MAXTOKEN".to_string()),
    );
    opts.insert(
        "api_base".to_string(),
        toml::Value::String("http://127.0.0.1:9".to_string()),
    );
    opts.insert("dry_run".to_string(), toml::Value::Boolean(true));
    opts.insert(
        "webhook_url".to_string(),
        toml::Value::String("https://example.invalid/max/webhook".to_string()),
    );
    opts.insert(
        "webhook_listen".to_string(),
        toml::Value::String("127.0.0.1:0".to_string()),
    );
    opts.insert(
        "webhook_path".to_string(),
        toml::Value::String("/max/webhook".to_string()),
    );
    opts.insert(
        "webhook_secret".to_string(),
        toml::Value::String("secret".to_string()),
    );
    let platform = MaxPlatform::new(max_config_from_options(opts).unwrap());
    let engine = start_engine("test", Arc::clone(&platform) as Arc<dyn Platform>).await;
    let addr = wait_max_local_addr(&platform).await;
    let response = reqwest::Client::new()
        .post(format!("http://{addr}/max/webhook"))
        .header("x-max-bot-api-secret", "secret")
        .json(&json!({
            "update_type": "message_created",
            "message": {
                "sender": { "user_id": 8, "name": "bob" },
                "recipient": { "chat_id": 43 },
                "body": { "mid": "MAXMSG2", "text": "hello max webhook" }
            }
        }))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    let outbound = timeout(Duration::from_secs(2), platform.wait_for_outbound())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(outbound.target, "43");
    assert_eq!(outbound.content, "mock: hello max webhook");
    engine.stop().await.unwrap();
}

#[tokio::test]
async fn weixin_long_poll_text_round_trip_e2e() {
    let served = Arc::new(AtomicBool::new(false));
    let app = Router::new().route(
        "/ilink/bot/getupdates",
        post({
            let served = Arc::clone(&served);
            move |headers: HeaderMap| {
                let served = Arc::clone(&served);
                async move {
                    assert_eq!(
                        headers
                            .get("authorizationtype")
                            .and_then(|value| value.to_str().ok()),
                        Some("ilink_bot_token")
                    );
                    assert!(headers.get("x-wechat-uin").is_some());
                    assert_eq!(
                        headers
                            .get("skroutetag")
                            .and_then(|value| value.to_str().ok()),
                        Some("test-route")
                    );
                    if served.swap(true, Ordering::SeqCst) {
                        Json(json!({ "ret": 0, "msgs": [], "get_updates_buf": "2" }))
                    } else {
                        Json(json!({
                            "ret": 0,
                            "get_updates_buf": "1",
                            "msgs": [{
                                "message_id": 88,
                                "from_user_id": "wx-user-1",
                                "client_id": "client-1",
                                "message_type": 1,
                                "context_token": "ctx-1",
                                "item_list": [{
                                    "type": 1,
                                    "text_item": { "text": "hello weixin" }
                                }]
                            }]
                        }))
                    }
                }
            }
        }),
    );
    let addr = serve(app).await;
    let mut opts = toml::value::Table::new();
    opts.insert(
        "name".to_string(),
        toml::Value::String("weixin".to_string()),
    );
    opts.insert(
        "token".to_string(),
        toml::Value::String("WXTOKEN".to_string()),
    );
    opts.insert(
        "api_base".to_string(),
        toml::Value::String(format!("http://{addr}")),
    );
    opts.insert(
        "route_tag".to_string(),
        toml::Value::String("test-route".to_string()),
    );
    opts.insert("dry_run".to_string(), toml::Value::Boolean(true));
    let platform = WeixinPlatform::new(weixin_config_from_options(opts).unwrap());
    let engine = start_engine("test", Arc::clone(&platform) as Arc<dyn Platform>).await;
    let outbound = timeout(Duration::from_secs(2), platform.wait_for_outbound())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(outbound.target, "wx-user-1");
    assert_eq!(outbound.content, "mock: hello weixin");
    engine.stop().await.unwrap();
}

#[tokio::test]
async fn qqbot_gateway_text_round_trip_e2e() {
    let app = Router::new().route("/qqbot", get(qqbot_ws));
    let addr = serve(app).await;
    let mut opts = toml::value::Table::new();
    opts.insert("name".to_string(), toml::Value::String("qqbot".to_string()));
    opts.insert(
        "token".to_string(),
        toml::Value::String("QQBOTTOKEN".to_string()),
    );
    opts.insert(
        "gateway_url".to_string(),
        toml::Value::String(format!("ws://{addr}/qqbot")),
    );
    opts.insert("dry_run".to_string(), toml::Value::Boolean(true));
    let platform = QqBotPlatform::new(qqbot_config_from_options(opts).unwrap());
    let engine = start_engine("test", Arc::clone(&platform) as Arc<dyn Platform>).await;
    let outbound = timeout(Duration::from_secs(2), platform.wait_for_outbound())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(outbound.target, "group-open-1");
    assert_eq!(outbound.content, "mock: hello qqbot");
    engine.stop().await.unwrap();
}

async fn qqbot_ws(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(|mut socket| async move {
        while let Some(Ok(message)) = socket.next().await {
            if let AxumWsMessage::Text(text) = message {
                let value: Value = serde_json::from_str(&text).unwrap();
                if value.get("op").and_then(Value::as_i64) == Some(2) {
                    socket
                        .send(AxumWsMessage::Text(
                            json!({
                                "op": 0,
                                "d": {
                                    "id": "QQBOTMSG1",
                                    "group_openid": "group-open-1",
                                    "member_openid": "member-open-1",
                                    "content": "hello qqbot"
                                }
                            })
                            .to_string(),
                        ))
                        .await
                        .unwrap();
                    break;
                }
            }
        }
    })
}

#[tokio::test]
async fn qqbot_gateway_fetches_access_token_e2e() {
    let app = Router::new()
        .route("/qqbot-token", post(qqbot_token))
        .route("/qqbot", get(qqbot_ws_expect_token));
    let addr = serve(app).await;
    let mut opts = toml::value::Table::new();
    opts.insert("name".to_string(), toml::Value::String("qqbot".to_string()));
    opts.insert(
        "app_id".to_string(),
        toml::Value::String("APPID".to_string()),
    );
    opts.insert(
        "app_secret".to_string(),
        toml::Value::String("APPSECRET".to_string()),
    );
    opts.insert(
        "token_endpoint".to_string(),
        toml::Value::String(format!("http://{addr}/qqbot-token")),
    );
    opts.insert(
        "gateway_url".to_string(),
        toml::Value::String(format!("ws://{addr}/qqbot")),
    );
    opts.insert("dry_run".to_string(), toml::Value::Boolean(true));
    let platform = QqBotPlatform::new(qqbot_config_from_options(opts).unwrap());
    let engine = start_engine("test", Arc::clone(&platform) as Arc<dyn Platform>).await;
    let outbound = timeout(Duration::from_secs(2), platform.wait_for_outbound())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(outbound.content, "mock: token qqbot");
    engine.stop().await.unwrap();
}

async fn qqbot_token() -> impl IntoResponse {
    Json(json!({ "access_token": "FETCHED_QQBOT_TOKEN", "expires_in": 7200 }))
}

async fn qqbot_ws_expect_token(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(|mut socket| async move {
        while let Some(Ok(message)) = socket.next().await {
            if let AxumWsMessage::Text(text) = message {
                let value: Value = serde_json::from_str(&text).unwrap();
                if value.pointer("/d/token").and_then(Value::as_str) == Some("FETCHED_QQBOT_TOKEN")
                {
                    socket
                        .send(AxumWsMessage::Text(
                            json!({
                                "op": 0,
                                "d": {
                                    "id": "QQBOTMSG2",
                                    "group_openid": "group-open-2",
                                    "member_openid": "member-open-2",
                                    "content": "token qqbot"
                                }
                            })
                            .to_string(),
                        ))
                        .await
                        .unwrap();
                    break;
                }
            }
        }
    })
}

#[tokio::test]
async fn weibo_websocket_text_round_trip_e2e() {
    let app = Router::new().route("/weibo", get(weibo_ws));
    let addr = serve(app).await;
    let mut opts = toml::value::Table::new();
    opts.insert("name".to_string(), toml::Value::String("weibo".to_string()));
    opts.insert(
        "token".to_string(),
        toml::Value::String("WEIBOTOKEN".to_string()),
    );
    opts.insert(
        "ws_endpoint".to_string(),
        toml::Value::String(format!("ws://{addr}/weibo")),
    );
    opts.insert("dry_run".to_string(), toml::Value::Boolean(true));
    let platform = WeiboPlatform::new(weibo_config_from_options(opts).unwrap());
    let engine = start_engine("test", Arc::clone(&platform) as Arc<dyn Platform>).await;
    let outbound = timeout(Duration::from_secs(2), platform.wait_for_outbound())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(outbound.target, "weibo-user-1");
    assert_eq!(outbound.content, "mock: hello weibo");
    engine.stop().await.unwrap();
}

async fn weibo_ws(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(|mut socket| async move {
        socket
            .send(AxumWsMessage::Text(
                json!({
                    "id": "WEIBOMSG1",
                    "from_user_id": "weibo-user-1",
                    "text": "hello weibo"
                })
                .to_string(),
            ))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
    })
}

#[tokio::test]
async fn weibo_websocket_fetches_ws_token_e2e() {
    let app = Router::new()
        .route("/weibo-token", post(weibo_token))
        .route("/weibo", get(weibo_ws_expect_token));
    let addr = serve(app).await;
    let mut opts = toml::value::Table::new();
    opts.insert("name".to_string(), toml::Value::String("weibo".to_string()));
    opts.insert(
        "app_id".to_string(),
        toml::Value::String("APPID".to_string()),
    );
    opts.insert(
        "app_secret".to_string(),
        toml::Value::String("APPSECRET".to_string()),
    );
    opts.insert(
        "token_endpoint".to_string(),
        toml::Value::String(format!("http://{addr}/weibo-token")),
    );
    opts.insert(
        "ws_endpoint".to_string(),
        toml::Value::String(format!("ws://{addr}/weibo")),
    );
    opts.insert("dry_run".to_string(), toml::Value::Boolean(true));
    let platform = WeiboPlatform::new(weibo_config_from_options(opts).unwrap());
    let engine = start_engine("test", Arc::clone(&platform) as Arc<dyn Platform>).await;
    let outbound = timeout(Duration::from_secs(2), platform.wait_for_outbound())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(outbound.content, "mock: token weibo");
    engine.stop().await.unwrap();
}

async fn weibo_token() -> impl IntoResponse {
    Json(json!({ "data": { "token": "FETCHED_WEIBO_TOKEN", "expire_in": 7200 } }))
}

async fn weibo_ws_expect_token(
    Query(params): Query<BTreeMap<String, String>>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(|mut socket| async move {
        if params.get("app_id").map(String::as_str) == Some("APPID")
            && params.get("token").map(String::as_str) == Some("FETCHED_WEIBO_TOKEN")
        {
            socket
                .send(AxumWsMessage::Text(
                    json!({
                        "id": "WEIBOMSG2",
                        "from_user_id": "weibo-user-2",
                        "text": "token weibo"
                    })
                    .to_string(),
                ))
                .await
                .unwrap();
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    })
}

async fn start_engine(name: &str, platform: Arc<dyn Platform>) -> Arc<Engine> {
    let engine = Engine::new(
        name,
        Arc::new(MockAgent::new()),
        vec![platform],
        SessionStore::in_memory().unwrap(),
    );
    engine.start().await.unwrap();
    engine
}

async fn serve(app: Router) -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    addr
}

async fn wait_max_local_addr(platform: &Arc<MaxPlatform>) -> std::net::SocketAddr {
    timeout(Duration::from_secs(2), async {
        loop {
            if let Some(addr) = platform.local_addr().await {
                break addr;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}

fn wecom_test_aes_key() -> String {
    let key = [7u8; 32];
    base64::engine::general_purpose::STANDARD
        .encode(key)
        .trim_end_matches('=')
        .to_string()
}

fn wecom_encrypt_for_test(aes_key: &str, corp_id: &str, plain_xml: &str) -> String {
    let key = base64::engine::general_purpose::STANDARD
        .decode(format!("{aes_key}="))
        .unwrap();
    let mut plain = Vec::new();
    plain.extend_from_slice(&[1u8; 16]);
    plain.extend_from_slice(&(plain_xml.len() as u32).to_be_bytes());
    plain.extend_from_slice(plain_xml.as_bytes());
    plain.extend_from_slice(corp_id.as_bytes());
    let iv = &key[..16];
    let mut buf = vec![0u8; plain.len() + 16];
    buf[..plain.len()].copy_from_slice(&plain);
    let encrypted = cbc::Encryptor::<aes::Aes256>::new_from_slices(&key, iv)
        .unwrap()
        .encrypt_padded_mut::<Pkcs7>(&mut buf, plain.len())
        .unwrap();
    base64::engine::general_purpose::STANDARD.encode(encrypted)
}

fn wecom_signature_for_test(token: &str, timestamp: &str, nonce: &str, encrypt: &str) -> String {
    let mut parts = [
        token.to_string(),
        timestamp.to_string(),
        nonce.to_string(),
        encrypt.to_string(),
    ];
    parts.sort();
    let mut hasher = Sha1::new();
    hasher.update(parts.join("").as_bytes());
    format!("{:x}", hasher.finalize())
}
