use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id:      Option<Value>,
    pub method:  String,
    pub params:  Option<Value>,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id:      Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result:  Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error:   Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcError {
    pub code:    i32,
    pub message: String,
    pub data:    Option<Value>,
}

impl JsonRpcResponse {
    pub fn ok(id: Option<Value>, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result:  Some(result),
            error:   None,
        }
    }

    pub fn err(id: Option<Value>, code: i32, msg: &str) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: None,
            error:  Some(JsonRpcError {
                code,
                message: msg.into(),
                data:    None,
            }),
        }
    }

    pub fn method_not_found(id: Option<Value>, method: &str) -> Self {
        Self::err(id, -32601, &format!("method not found: {method}"))
    }

    pub fn invalid_params(id: Option<Value>, msg: &str) -> Self {
        Self::err(id, -32602, msg)
    }

    pub fn server_error(id: Option<Value>, msg: &str) -> Self {
        Self::err(id, -32000, msg)
    }
}
