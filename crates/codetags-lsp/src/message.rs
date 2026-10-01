//! JSON-RPC message kinds, as the recorder logs them.

use serde_json::Value;

/// Which side of the session sent a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// The editor (Claude Code, VS Code).
    Client,
    /// The language server.
    Server,
}

impl Side {
    /// `"client"` or `"server"`, as logged.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Client => "client",
            Self::Server => "server",
        }
    }
}

/// What a JSON-RPC message is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Has a `method` and an `id`.
    Request,
    /// Has a `method` and no `id`.
    Notification,
    /// Has an `id` and a `result` or `error`, and no `method`.
    Response,
    /// A JSON array (a JSON-RPC batch; LSP does not use them).
    Batch,
    /// Anything else, including a body that is not JSON.
    Invalid,
}

impl Kind {
    /// The kind of `message`.
    pub fn of(message: &Value) -> Self {
        match message {
            Value::Array(_) => Self::Batch,
            Value::Object(fields) => {
                let has_id = fields.get("id").is_some_and(|id| !id.is_null());
                match (fields.get("method").and_then(Value::as_str), has_id) {
                    (Some(_), true) => Self::Request,
                    (Some(_), false) => Self::Notification,
                    (None, true)
                        if fields.contains_key("result") || fields.contains_key("error") =>
                    {
                        Self::Response
                    }
                    _ => Self::Invalid,
                }
            }
            _ => Self::Invalid,
        }
    }

    /// The name logged for this kind.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Request => "request",
            Self::Notification => "notification",
            Self::Response => "response",
            Self::Batch => "batch",
            Self::Invalid => "invalid",
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::Kind;

    #[test]
    fn classifies_messages() {
        assert_eq!(Kind::of(&json!({"id": 1, "method": "a"})), Kind::Request);
        assert_eq!(Kind::of(&json!({"id": "x", "method": "a"})), Kind::Request);
        assert_eq!(Kind::of(&json!({"method": "a"})), Kind::Notification);
        assert_eq!(Kind::of(&json!({"id": 1, "result": null})), Kind::Response);
        assert_eq!(
            Kind::of(&json!({"id": 1, "error": {"code": -32601}})),
            Kind::Response
        );
        assert_eq!(Kind::of(&json!({"id": 1})), Kind::Invalid);
        assert_eq!(Kind::of(&json!([])), Kind::Batch);
        assert_eq!(Kind::of(&json!(3)), Kind::Invalid);
    }
}
