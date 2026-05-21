use super::{
    codec, BridgeAdapter, BridgeAdapterInfo, BridgePlatform, ErrorMessage, HealthResponse,
    PongMessage, MESSAGE_TYPE_ERROR, MESSAGE_TYPE_MESSAGE, MESSAGE_TYPE_PING, MESSAGE_TYPE_PONG,
    MESSAGE_TYPE_REGISTER, MESSAGE_TYPE_REGISTER_ACK,
};
use anyhow::{anyhow, Result};
use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::mpsc;

#[derive(Debug, Deserialize)]
pub(super) struct BridgeQuery {
    token: Option<String>,
}

pub(super) async fn ws_handler(
    State(platform): State<Arc<BridgePlatform>>,
    Query(query): Query<BridgeQuery>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    if let Err(status) = authenticate(&platform, &headers, query.token.as_deref()) {
        return status.into_response();
    }
    ws.on_upgrade(move |socket| handle_socket(platform, socket))
}

pub(super) async fn health(State(platform): State<Arc<BridgePlatform>>) -> Json<HealthResponse> {
    Json(HealthResponse {
        ok: true,
        platform: platform.name.clone(),
        adapters: platform.adapters.lock().await.len(),
    })
}

pub(super) async fn adapters(
    State(platform): State<Arc<BridgePlatform>>,
) -> Json<Vec<BridgeAdapterInfo>> {
    Json(platform.adapter_infos().await)
}

fn authenticate(
    platform: &BridgePlatform,
    headers: &HeaderMap,
    token_query: Option<&str>,
) -> std::result::Result<(), StatusCode> {
    let Some(expected) = &platform.token else {
        return if platform.insecure {
            Ok(())
        } else {
            Err(StatusCode::UNAUTHORIZED)
        };
    };
    if token_query.is_some_and(|token| codec::constant_time_eq(token, expected)) {
        return Ok(());
    }
    if codec::header_matches(headers, "x-bridge-token", expected) {
        return Ok(());
    }
    if let Some(value) = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
    {
        if let Some(token) = value.strip_prefix("Bearer ") {
            if codec::constant_time_eq(token, expected) {
                return Ok(());
            }
        }
    }
    Err(StatusCode::UNAUTHORIZED)
}

async fn handle_socket(platform: Arc<BridgePlatform>, socket: WebSocket) {
    let (mut ws_tx, mut ws_rx) = futures_util::StreamExt::split(socket);
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();
    let writer = tokio::spawn(async move {
        while let Some(payload) = out_rx.recv().await {
            if futures_util::SinkExt::send(&mut ws_tx, WsMessage::Text(payload))
                .await
                .is_err()
            {
                break;
            }
        }
    });

    let mut registered_platform: Option<String> = None;
    while let Some(frame) = futures_util::StreamExt::next(&mut ws_rx).await {
        let Ok(frame) = frame else {
            break;
        };
        let WsMessage::Text(text) = frame else {
            if matches!(frame, WsMessage::Close(_)) {
                break;
            }
            continue;
        };
        if let Err(err) = handle_text_frame(
            Arc::clone(&platform),
            out_tx.clone(),
            &mut registered_platform,
            &text,
        )
        .await
        {
            tracing::warn!(error = %err, "bridge frame handling failed");
            let _ = codec::send_ws_json(
                &out_tx,
                &ErrorMessage {
                    message_type: MESSAGE_TYPE_ERROR.to_string(),
                    code: "bad_message".to_string(),
                    message: err.to_string(),
                },
            );
        }
    }

    if let Some(name) = registered_platform {
        let mut adapters = platform.adapters.lock().await;
        if adapters
            .get(&name)
            .is_some_and(|adapter| adapter.tx.same_channel(&out_tx))
        {
            adapters.remove(&name);
        }
    }
    writer.abort();
}

async fn handle_text_frame(
    platform: Arc<BridgePlatform>,
    out_tx: mpsc::UnboundedSender<String>,
    registered_platform: &mut Option<String>,
    text: &str,
) -> Result<()> {
    let value: Value = serde_json::from_str(text)?;
    let message_type = value
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("missing message type"))?;
    match message_type {
        MESSAGE_TYPE_REGISTER => {
            let register: super::RegisterMessage = serde_json::from_value(value)?;
            if register.platform.trim().is_empty() {
                return Err(anyhow!("register.platform is required"));
            }
            let adapter_name = register.platform;
            platform.adapters.lock().await.insert(
                adapter_name.clone(),
                BridgeAdapter {
                    platform: adapter_name.clone(),
                    capabilities: register.capabilities,
                    metadata: register.metadata,
                    tx: out_tx.clone(),
                },
            );
            *registered_platform = Some(adapter_name.clone());
            tracing::info!(adapter = %adapter_name, "bridge adapter registered");
            codec::send_ws_json(
                &out_tx,
                &super::RegisterAckMessage {
                    message_type: MESSAGE_TYPE_REGISTER_ACK.to_string(),
                    ok: true,
                    error: String::new(),
                },
            )?;
        }
        MESSAGE_TYPE_MESSAGE => {
            let adapter_name = registered_platform
                .clone()
                .ok_or_else(|| anyhow!("adapter must register before sending messages"))?;
            let inbound: super::InboundMessage = serde_json::from_value(value)?;
            let Some(handler) = platform.handler.lock().await.clone() else {
                return Err(anyhow!("bridge platform is not started"));
            };
            let message = inbound.into_core(&platform.name, &adapter_name)?;
            let platform_dyn = platform as Arc<dyn crate::core::Platform>;
            tokio::spawn(async move {
                handler(platform_dyn, message).await;
            });
        }
        MESSAGE_TYPE_PING => {
            let ts = value.get("ts").and_then(Value::as_i64);
            codec::send_ws_json(
                &out_tx,
                &PongMessage {
                    message_type: MESSAGE_TYPE_PONG.to_string(),
                    ts,
                },
            )?;
        }
        other => return Err(anyhow!("unsupported bridge message type `{other}`")),
    }
    Ok(())
}
