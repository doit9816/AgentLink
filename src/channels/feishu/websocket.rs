use super::{payload, FeishuPlatform, FEISHU_WS_ENDPOINT_PATH};
use anyhow::{anyhow, Result};
use futures_util::{SinkExt, StreamExt};
use prost::Message as ProstMessage;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message as WsMessage;

const HEADER_TYPE: &str = "type";
const HEADER_BIZ_RT: &str = "biz_rt";
const MESSAGE_TYPE_EVENT: &str = "event";
const MESSAGE_TYPE_PING: &str = "ping";
const FRAME_TYPE_CONTROL: i32 = 0;
const FRAME_TYPE_DATA: i32 = 1;

#[derive(Clone, PartialEq, ProstMessage)]
struct FeishuWsHeader {
    #[prost(string, required, tag = "1")]
    key: String,
    #[prost(string, required, tag = "2")]
    value: String,
}

#[derive(Clone, PartialEq, ProstMessage)]
struct FeishuWsFrame {
    #[prost(uint64, required, tag = "1")]
    seq_id: u64,
    #[prost(uint64, required, tag = "2")]
    log_id: u64,
    #[prost(int32, required, tag = "3")]
    service: i32,
    #[prost(int32, required, tag = "4")]
    method: i32,
    #[prost(message, repeated, tag = "5")]
    headers: Vec<FeishuWsHeader>,
    #[prost(string, optional, tag = "6")]
    payload_encoding: Option<String>,
    #[prost(string, optional, tag = "7")]
    payload_type: Option<String>,
    #[prost(bytes, optional, tag = "8")]
    payload: Option<Vec<u8>>,
    #[prost(string, optional, tag = "9")]
    log_id_new: Option<String>,
}

#[derive(Debug, Clone)]
struct FeishuWsEndpoint {
    url: String,
    ping_interval: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FeishuWsExit {
    Reconnect,
    Shutdown,
}

pub(super) async fn run(platform: Arc<FeishuPlatform>, mut shutdown_rx: oneshot::Receiver<()>) {
    loop {
        match run_once(Arc::clone(&platform), &mut shutdown_rx).await {
            Ok(FeishuWsExit::Shutdown) => return,
            Ok(FeishuWsExit::Reconnect) => {}
            Err(err) => {
                tracing::warn!(error = %err, "feishu websocket disconnected");
            }
        }
        tokio::select! {
            _ = &mut shutdown_rx => return,
            _ = tokio::time::sleep(Duration::from_secs(30)) => {}
        }
    }
}

async fn run_once(
    platform: Arc<FeishuPlatform>,
    shutdown_rx: &mut oneshot::Receiver<()>,
) -> Result<FeishuWsExit> {
    let endpoint = websocket_endpoint(&platform).await?;
    let service_id = query_param(&endpoint.url, "service_id")
        .and_then(|value| value.parse::<i32>().ok())
        .unwrap_or(0);
    let (stream, _) = connect_async(&endpoint.url).await?;
    tracing::info!(url = %mask_ws_url(&endpoint.url), "feishu websocket connected");

    let (mut write, mut read) = stream.split();
    let mut ping = tokio::time::interval(Duration::from_secs(endpoint.ping_interval.clamp(1, 600)));
    loop {
        tokio::select! {
            _ = &mut *shutdown_rx => {
                let _ = write.close().await;
                return Ok(FeishuWsExit::Shutdown);
            }
            _ = ping.tick() => {
                let frame = new_ping_frame(service_id);
                write.send(WsMessage::Binary(encode_ws_frame(&frame)?)).await?;
            }
            message = read.next() => {
                let Some(message) = message else {
                    return Ok(FeishuWsExit::Reconnect);
                };
                match message? {
                    WsMessage::Binary(bytes) => {
                        if let Some(response) = handle_frame(Arc::clone(&platform), &bytes).await? {
                            write.send(WsMessage::Binary(response)).await?;
                        }
                    }
                    WsMessage::Ping(bytes) => {
                        write.send(WsMessage::Pong(bytes)).await?;
                    }
                    WsMessage::Close(_) => return Ok(FeishuWsExit::Reconnect),
                    _ => {}
                }
            }
        }
    }
}

async fn websocket_endpoint(platform: &FeishuPlatform) -> Result<FeishuWsEndpoint> {
    #[derive(Debug, Deserialize)]
    struct EndpointResponse {
        code: i64,
        msg: Option<String>,
        data: Option<EndpointData>,
    }
    #[derive(Debug, Deserialize)]
    struct EndpointData {
        #[serde(rename = "URL")]
        url: String,
        #[serde(rename = "ClientConfig")]
        client_config: Option<EndpointClientConfig>,
    }
    #[derive(Debug, Deserialize)]
    struct EndpointClientConfig {
        #[serde(rename = "PingInterval")]
        ping_interval: Option<u64>,
    }

    let resp: EndpointResponse = platform
        .client
        .post(format!(
            "{}{}",
            platform.config.api_base, FEISHU_WS_ENDPOINT_PATH
        ))
        .header("locale", "zh")
        .json(&json!({
            "AppID": platform.config.app_id,
            "AppSecret": platform.config.app_secret,
        }))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    if resp.code != 0 {
        return Err(anyhow!(
            "feishu websocket endpoint failed: code={}, msg={}",
            resp.code,
            resp.msg.unwrap_or_default()
        ));
    }
    let data = resp
        .data
        .ok_or_else(|| anyhow!("feishu websocket endpoint response missing data"))?;
    if data.url.trim().is_empty() {
        return Err(anyhow!("feishu websocket endpoint response missing URL"));
    }
    let cfg = data.client_config;
    Ok(FeishuWsEndpoint {
        url: data.url,
        ping_interval: cfg
            .as_ref()
            .and_then(|value| value.ping_interval)
            .unwrap_or(120),
    })
}

async fn handle_frame(platform: Arc<FeishuPlatform>, bytes: &[u8]) -> Result<Option<Vec<u8>>> {
    let mut frame = FeishuWsFrame::decode(bytes)?;
    let frame_type = frame.method;
    let message_type = header_value(&frame.headers, HEADER_TYPE).unwrap_or_default();
    if frame_type == FRAME_TYPE_CONTROL {
        return Ok(None);
    }
    if frame_type != FRAME_TYPE_DATA || message_type != MESSAGE_TYPE_EVENT {
        return Ok(None);
    }

    let started = std::time::Instant::now();
    let payload = frame.payload.clone().unwrap_or_default();
    let status = match serde_json::from_slice::<Value>(&payload) {
        Ok(value) => match payload::dispatch(platform, &value).await {
            Ok(_) => axum::http::StatusCode::OK.as_u16(),
            Err(err) => {
                tracing::warn!(error = %err, "feishu websocket event parse failed");
                axum::http::StatusCode::INTERNAL_SERVER_ERROR.as_u16()
            }
        },
        Err(err) => {
            tracing::warn!(error = %err, "feishu websocket payload is not json");
            axum::http::StatusCode::INTERNAL_SERVER_ERROR.as_u16()
        }
    };

    upsert_header(
        &mut frame.headers,
        HEADER_BIZ_RT,
        started.elapsed().as_millis().to_string(),
    );
    frame.payload = Some(serde_json::to_vec(&json!({
        "code": status,
        "headers": {},
        "data": null
    }))?);
    Ok(Some(encode_ws_frame(&frame)?))
}

fn new_ping_frame(service_id: i32) -> FeishuWsFrame {
    FeishuWsFrame {
        seq_id: 0,
        log_id: 0,
        service: service_id,
        method: FRAME_TYPE_CONTROL,
        headers: vec![FeishuWsHeader {
            key: HEADER_TYPE.to_string(),
            value: MESSAGE_TYPE_PING.to_string(),
        }],
        payload_encoding: None,
        payload_type: None,
        payload: None,
        log_id_new: None,
    }
}

fn encode_ws_frame(frame: &FeishuWsFrame) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    frame.encode(&mut buf)?;
    Ok(buf)
}

fn header_value(headers: &[FeishuWsHeader], key: &str) -> Option<String> {
    headers
        .iter()
        .find(|header| header.key == key)
        .map(|header| header.value.clone())
}

fn upsert_header(headers: &mut Vec<FeishuWsHeader>, key: &str, value: String) {
    if let Some(header) = headers.iter_mut().find(|header| header.key == key) {
        header.value = value;
        return;
    }
    headers.push(FeishuWsHeader {
        key: key.to_string(),
        value,
    });
}

fn query_param(input: &str, name: &str) -> Option<String> {
    let query = input.split_once('?')?.1;
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        if key == name {
            return Some(value.to_string());
        }
    }
    None
}

fn mask_ws_url(input: &str) -> String {
    let Some((base, query)) = input.split_once('?') else {
        return input.to_string();
    };
    let masked = query
        .split('&')
        .map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            if matches!(key, "device_id" | "access_key" | "ticket" | "conn_id") {
                format!("{key}=***")
            } else {
                format!("{key}={value}")
            }
        })
        .collect::<Vec<_>>()
        .join("&");
    format!("{base}?{masked}")
}
