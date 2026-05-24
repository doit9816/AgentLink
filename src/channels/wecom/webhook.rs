use super::WeComPlatform;
use crate::channels::support::*;

pub(super) async fn wecom_verify(
    State(platform): State<Arc<WeComPlatform>>,
    Query(params): Query<BTreeMap<String, String>>,
) -> Response {
    let echostr = params.get("echostr").cloned().unwrap_or_default();
    if platform.config.callback_aes_key.is_some() {
        let sig = params
            .get("msg_signature")
            .map(String::as_str)
            .unwrap_or("");
        let ts = params.get("timestamp").map(String::as_str).unwrap_or("");
        let nonce = params.get("nonce").map(String::as_str).unwrap_or("");
        if !wecom_verify_signature(&platform, sig, ts, nonce, &echostr) {
            return StatusCode::FORBIDDEN.into_response();
        }
        match wecom_decrypt(&platform, &echostr) {
            Ok(plain) => plain.into_response(),
            Err(err) => (StatusCode::BAD_REQUEST, err.to_string()).into_response(),
        }
    } else {
        echostr.into_response()
    }
}

pub(super) async fn wecom_webhook(
    State(platform): State<Arc<WeComPlatform>>,
    Query(params): Query<BTreeMap<String, String>>,
    body: Bytes,
) -> Response {
    let text = String::from_utf8_lossy(&body).to_string();
    let payload = serde_json::from_slice::<Value>(&body).ok();
    let plain = if payload.is_none() && xml_tag(&text, "Encrypt").is_some() {
        let encrypt = xml_tag(&text, "Encrypt").unwrap_or_default();
        if platform.config.callback_aes_key.is_some() {
            let sig = params
                .get("msg_signature")
                .map(String::as_str)
                .unwrap_or("");
            let ts = params.get("timestamp").map(String::as_str).unwrap_or("");
            let nonce = params.get("nonce").map(String::as_str).unwrap_or("");
            if !wecom_verify_signature(&platform, sig, ts, nonce, &encrypt) {
                return StatusCode::FORBIDDEN.into_response();
            }
        }
        match wecom_decrypt(&platform, &encrypt) {
            Ok(plain) => plain,
            Err(err) => return (StatusCode::BAD_REQUEST, err.to_string()).into_response(),
        }
    } else {
        text
    };
    match wecom_message_from_payload(&platform, payload.as_ref(), &plain) {
        Ok(Some(message)) => {
            dispatch_webhook(
                Arc::clone(&platform) as Arc<dyn Platform>,
                &platform.handler,
                message,
            )
            .await;
            "success".into_response()
        }
        Ok(None) => "success".into_response(),
        Err(err) => (StatusCode::BAD_REQUEST, err.to_string()).into_response(),
    }
}

fn wecom_message_from_payload(
    platform: &WeComPlatform,
    payload: Option<&Value>,
    raw: &str,
) -> Result<Option<Message>> {
    let (from, content, msg_id, agent_id) = if let Some(v) = payload {
        (
            v.get("FromUserName")
                .or_else(|| v.get("from_user"))
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string(),
            v.get("Content")
                .or_else(|| v.get("content"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string(),
            v.get("MsgId")
                .or_else(|| v.get("msg_id"))
                .map(value_to_string),
            v.get("AgentID")
                .or_else(|| v.get("agent_id"))
                .map(value_to_string),
        )
    } else {
        (
            xml_tag(raw, "FromUserName").unwrap_or_else(|| "unknown".to_string()),
            xml_tag(raw, "Content").unwrap_or_default(),
            xml_tag(raw, "MsgId"),
            xml_tag(raw, "AgentID"),
        )
    };
    if content.trim().is_empty() {
        return Ok(None);
    }
    simple_message(
        &platform.config.name,
        format!("{}:{}", platform.config.name, from),
        from.clone(),
        msg_id.clone().as_deref(),
        content,
        SimpleReplyContext {
            channel: platform.config.name.clone(),
            target: from,
            message_id: msg_id,
            extra: json!({ "agent_id": agent_id.or_else(|| platform.config.agent_id.clone()).unwrap_or_default() }),
        },
    )
}

fn wecom_verify_signature(
    platform: &WeComPlatform,
    expected: &str,
    timestamp: &str,
    nonce: &str,
    encrypt: &str,
) -> bool {
    let token = platform
        .config
        .callback_token
        .as_deref()
        .or(platform.config.token.as_deref())
        .unwrap_or("");
    if expected.is_empty() || token.is_empty() {
        return false;
    }
    let mut parts = [
        token.to_string(),
        timestamp.to_string(),
        nonce.to_string(),
        encrypt.to_string(),
    ];
    parts.sort();
    let mut hasher = Sha1::new();
    hasher.update(parts.join("").as_bytes());
    let actual = format!("{:x}", hasher.finalize());
    actual == expected
}

fn wecom_decrypt(platform: &WeComPlatform, cipher_base64: &str) -> Result<String> {
    let key = platform
        .config
        .callback_aes_key
        .as_deref()
        .ok_or_else(|| anyhow!("wecom callback_aes_key is required for encrypted XML"))?;
    let key = decode_wecom_aes_key(key)?;
    let cipher_data = base64::engine::general_purpose::STANDARD.decode(cipher_base64)?;
    if cipher_data.len() % 16 != 0 {
        return Err(anyhow!("invalid wecom ciphertext length"));
    }
    let iv = &key[..16];
    let mut buf = cipher_data;
    let plain = cbc::Decryptor::<aes::Aes256>::new_from_slices(&key, iv)?
        .decrypt_padded_mut::<Pkcs7>(&mut buf)
        .map_err(|_| anyhow!("wecom pkcs7 unpad failed"))?;
    if plain.len() < 20 {
        return Err(anyhow!("wecom decrypted data too short"));
    }
    let msg_len = u32::from_be_bytes([plain[16], plain[17], plain[18], plain[19]]) as usize;
    if 20 + msg_len > plain.len() {
        return Err(anyhow!("wecom decrypted message length is invalid"));
    }
    let msg = std::str::from_utf8(&plain[20..20 + msg_len])?.to_string();
    let corp_id = std::str::from_utf8(&plain[20 + msg_len..]).unwrap_or("");
    if let Some(expected) = &platform.config.corp_id {
        if !expected.is_empty() && corp_id != expected {
            return Err(anyhow!("wecom corp_id mismatch"));
        }
    }
    Ok(msg)
}

fn decode_wecom_aes_key(value: &str) -> Result<Vec<u8>> {
    if value.len() != 43 {
        return Err(anyhow!(
            "wecom callback_aes_key must be 43 characters, got {}",
            value.len()
        ));
    }
    Ok(base64::engine::general_purpose::STANDARD.decode(format!("{value}="))?)
}
