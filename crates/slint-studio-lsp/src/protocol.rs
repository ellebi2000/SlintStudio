//! JSON-RPC 2.0 message encoding and decoding.

use serde::Deserialize;
use serde_json::{Value, json};

/// Id of a request sent by us. (Server requests may use any JSON id and are
/// answered with the id they carried.)
pub type RequestId = i64;

pub const METHOD_NOT_FOUND: i64 = -32601;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ResponseError {
    pub code: i64,
    pub message: String,
}

/// A message received from the server.
#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    /// Answer to one of our requests. `id` is `None` if it was not one of ours.
    Response {
        id: Option<RequestId>,
        result: Result<Value, ResponseError>,
    },
    /// A request from the server, which must be answered.
    Request {
        id: Value,
        method: String,
        params: Value,
    },
    Notification {
        method: String,
        params: Value,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("JSON non valido: {0}")]
    Json(#[from] serde_json::Error),
    #[error("messaggio JSON-RPC non riconosciuto")]
    Unrecognized,
}

pub fn parse(text: &str) -> Result<Incoming, ProtocolError> {
    let mut value: Value = serde_json::from_str(text)?;
    let object = value.as_object_mut().ok_or(ProtocolError::Unrecognized)?;
    let id = object.remove("id");
    let method = object.remove("method");
    let params = object.remove("params").unwrap_or(Value::Null);

    match (id, method) {
        (Some(id), Some(Value::String(method))) => Ok(Incoming::Request { id, method, params }),
        (None, Some(Value::String(method))) => Ok(Incoming::Notification { method, params }),
        (Some(id), None) => {
            let result = match object.remove("error") {
                Some(error) => Err(serde_json::from_value(error)?),
                None => Ok(object.remove("result").unwrap_or(Value::Null)),
            };
            Ok(Incoming::Response {
                id: id.as_i64(),
                result,
            })
        }
        _ => Err(ProtocolError::Unrecognized),
    }
}

pub fn request(id: RequestId, method: &str, params: Value) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }).to_string()
}

pub fn notification(method: &str, params: Value) -> String {
    json!({ "jsonrpc": "2.0", "method": method, "params": params }).to_string()
}

pub fn response(id: &Value, result: Value) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string()
}

pub fn error_response(id: &Value, code: i64, message: &str) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_three_message_kinds() {
        let response = parse(r#"{"jsonrpc":"2.0","id":3,"result":[1]}"#).unwrap();
        assert_eq!(
            response,
            Incoming::Response {
                id: Some(3),
                result: Ok(json!([1]))
            }
        );

        let request = parse(r#"{"jsonrpc":"2.0","id":"x","method":"workspace/configuration","params":{"items":[]}}"#).unwrap();
        assert!(
            matches!(request, Incoming::Request { ref id, ref method, .. } if id == "x" && method == "workspace/configuration")
        );

        let notification =
            parse(r#"{"jsonrpc":"2.0","method":"slint/lsp_to_preview","params":"Quit"}"#).unwrap();
        assert_eq!(
            notification,
            Incoming::Notification {
                method: "slint/lsp_to_preview".into(),
                params: json!("Quit")
            }
        );
    }

    #[test]
    fn parses_errors_and_null_results() {
        let error = parse(
            r#"{"jsonrpc":"2.0","id":5,"error":{"code":-32601,"message":"Cannot handle request"}}"#,
        )
        .unwrap();
        let Incoming::Response {
            id,
            result: Err(err),
        } = error
        else {
            panic!("expected an error response");
        };
        assert_eq!(id, Some(5));
        assert_eq!(err.code, METHOD_NOT_FOUND);

        let null = parse(r#"{"jsonrpc":"2.0","id":4,"result":null}"#).unwrap();
        assert_eq!(
            null,
            Incoming::Response {
                id: Some(4),
                result: Ok(Value::Null)
            }
        );
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse("not json").is_err());
        assert!(parse("[1,2]").is_err());
        assert!(parse(r#"{"jsonrpc":"2.0"}"#).is_err());
    }

    #[test]
    fn encodes_messages() {
        let req: Value = serde_json::from_str(&request(1, "shutdown", Value::Null)).unwrap();
        assert_eq!(req["id"], 1);
        assert_eq!(req["method"], "shutdown");

        let resp: Value = serde_json::from_str(&response(&json!(0), json!([null]))).unwrap();
        assert_eq!(resp["result"], json!([null]));

        let err: Value =
            serde_json::from_str(&error_response(&json!("a"), METHOD_NOT_FOUND, "no")).unwrap();
        assert_eq!(err["error"]["code"], METHOD_NOT_FOUND);
    }
}
