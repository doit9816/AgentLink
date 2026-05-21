use serde_json::Value;

#[derive(Debug, serde::Deserialize)]
pub(super) struct RpcEnvelope {
    pub(super) id: Option<i64>,
    pub(super) method: Option<String>,
    pub(super) params: Option<Value>,
    pub(super) result: Option<Value>,
    pub(super) error: Option<Value>,
}
