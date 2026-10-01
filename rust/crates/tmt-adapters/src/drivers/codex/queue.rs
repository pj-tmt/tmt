//! Native queue request/receipt contract. A receipt proves queue acceptance,
//! never model processing or completion of the durable TMT request.

use serde_json::{Value, json};
use tmt_core::binding::session::ProviderSessionId;

pub const MESSAGE_LIMIT: usize = 64 * 1024;
pub const RESPONSE_LIMIT: usize = 1024 * 1024;
const ID_LIMIT: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueOutcome {
    Accepted {
        submission_id: String,
    },
    /// A qualified pre-enqueue provider rejection for this exact thread.
    /// Correlation or an error code alone does not prove absence of side effects.
    Refused {
        code: i64,
    },
    Uncertain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidRequest {
    MessageTooLarge,
    InvalidCorrelation,
}

/// One immutable attempt. Retaining its correlation is diagnostic evidence,
/// not permission to retry: the provider does not deduplicate caller IDs.
#[derive(Debug)]
pub struct QueueRequest {
    request_id: String,
    client_message_id: String,
    thread_id: ProviderSessionId,
    message: String,
}

impl QueueRequest {
    pub fn new(thread_id: ProviderSessionId, message: &str) -> Result<Self, InvalidRequest> {
        Self::with_ids(
            thread_id,
            message,
            uuid::Uuid::new_v4().to_string(),
            uuid::Uuid::new_v4().to_string(),
        )
    }

    fn with_ids(
        thread_id: ProviderSessionId,
        message: &str,
        request_id: String,
        client_message_id: String,
    ) -> Result<Self, InvalidRequest> {
        if message.len() > MESSAGE_LIMIT {
            return Err(InvalidRequest::MessageTooLarge);
        }
        if !valid_id(&request_id) || !valid_id(&client_message_id) || !valid_id(thread_id.as_str())
        {
            return Err(InvalidRequest::InvalidCorrelation);
        }
        Ok(Self {
            request_id,
            client_message_id,
            thread_id,
            message: message.to_owned(),
        })
    }

    pub fn id(&self) -> &str {
        &self.request_id
    }

    pub fn frame(&self) -> Value {
        json!({
            "id": self.request_id,
            "method": "thread/queue/add",
            "params": {
                "threadId": self.thread_id.as_str(),
                "clientUserMessageId": self.client_message_id,
                "input": [{"type": "text", "text": self.message}]
            }
        })
    }

    pub fn receipt(&self, bytes: &[u8]) -> QueueOutcome {
        if bytes.len() > RESPONSE_LIMIT {
            return QueueOutcome::Uncertain;
        }
        let Ok(response) = serde_json::from_slice::<Value>(bytes) else {
            return QueueOutcome::Uncertain;
        };
        if response.get("id").and_then(Value::as_str) != Some(self.id())
            || response.get("method").is_some()
        {
            return QueueOutcome::Uncertain;
        }
        match (response.get("result"), response.get("error")) {
            (None, Some(error)) => match (
                error.get("code").and_then(Value::as_i64),
                error.get("message").and_then(Value::as_str),
            ) {
                (Some(code), Some(message)) if self.pre_enqueue_refusal(code, message) => {
                    QueueOutcome::Refused { code }
                }
                _ => QueueOutcome::Uncertain,
            },
            (Some(result), None) => self.accepted(result).unwrap_or(QueueOutcome::Uncertain),
            _ => QueueOutcome::Uncertain,
        }
    }

    fn pre_enqueue_refusal(&self, code: i64, message: &str) -> bool {
        // Only exact require_thread failures qualify. Generic internal errors
        // can follow enqueue (including response conversion failure). See the
        // source and retained-event provenance in contracts/codex-channel-v1.md.
        let thread = self.thread_id.as_str();
        match code {
            -32600 => {
                message
                    == format!(
                        "session {thread} is archived. Run `codex unarchive {thread}` to unarchive it first."
                    )
            }
            -32603 => {
                message
                    == format!(
                        "failed to read thread: invalid thread-store request: no rollout found for thread id {thread}"
                    )
            }
            _ => false,
        }
    }

    fn accepted(&self, result: &Value) -> Option<QueueOutcome> {
        let submission = result.get("queuedSubmission")?;
        let submission_id = submission.get("id")?.as_str()?;
        let caller_id = submission.get("clientUserMessageId")?.as_str()?;
        let input = submission.get("input")?.as_array()?;
        if !valid_id(submission_id)
            || caller_id != self.client_message_id
            || input.len() != 1
            || input[0].get("type")?.as_str()? != "text"
            || input[0].get("text")?.as_str()? != self.message
        {
            return None;
        }
        Some(QueueOutcome::Accepted {
            submission_id: submission_id.to_owned(),
        })
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= ID_LIMIT && !id.chars().any(char::is_control)
}

#[cfg(test)]
mod tests;
