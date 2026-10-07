//! Canonical compact JSON for a checked frame: fixed field order, no whitespace.
//! Every string written is a constrained spelling (UUID, counter, base64url, hex or
//! a fixed word), so none needs escaping.
use super::*;
use std::fmt::{Display, Write};

/// A compact JSON object under construction.
struct Object(String);
impl Object {
    fn new() -> Self {
        Self("{".to_owned())
    }
    fn key(&mut self, name: &str) -> &mut String {
        if self.0.len() > 1 {
            self.0.push(',');
        }
        let _ = write!(self.0, "\"{name}\":");
        &mut self.0
    }
    fn text(&mut self, name: &str, value: impl Display) -> &mut Self {
        let _ = write!(self.key(name), "\"{value}\"");
        self
    }
    fn number(&mut self, name: &str, value: impl Display) -> &mut Self {
        let _ = write!(self.key(name), "{value}");
        self
    }
    fn raw(&mut self, name: &str, value: impl Display) -> &mut Self {
        let _ = write!(self.key(name), "{value}");
        self
    }
    fn finish(mut self) -> String {
        self.0.push('}');
        self.0
    }
}

pub(super) fn frame(frame: &Frame) -> String {
    let mut object = Object::new();
    object.number("version", 1);
    match frame {
        Frame::Request(request) => {
            object
                .text("kind", "request")
                .text("generation", request.generation)
                .text("requestId", request.request_id)
                .raw("origin", origin(request.origin))
                .text("method", method(request.call.method()))
                .raw("input", call(&request.call));
        }
        Frame::Result(result) => {
            object
                .text("kind", "result")
                .text("generation", result.generation)
                .text("requestId", result.request_id)
                .text("method", method(result.method));
            if let Some(transfer_id) = result.transfer_id {
                object.text("transferId", transfer_id);
            }
            match &result.outcome {
                Outcome::Success(value) => object.raw("ok", success(value)),
                Outcome::Failure(code) => object.raw("error", error_code(*code)),
            };
        }
    }
    object.finish()
}

fn origin(origin: Origin) -> String {
    let mut object = Object::new();
    match origin {
        Origin::LocalExtension => object.text("kind", "local-extension"),
        Origin::Mounted(id) => object.text("kind", "mounted").text("originId", id),
    };
    object.finish()
}

fn method(method: Method) -> &'static str {
    match method {
        Method::Config => "objects.config",
        Method::Begin => "objects.begin",
        Method::Part => "objects.part",
        Method::Commit => "objects.commit",
        Method::Status => "objects.status",
        Method::Read => "objects.read",
        Method::Discard => "objects.discard",
    }
}

fn call(call: &Call) -> String {
    let mut object = Object::new();
    match call {
        Call::Config(input) => {
            object
                .text("namespace", input.namespace)
                .text("policyInput", &input.policy);
        }
        Call::Begin(input) => {
            object
                .text("transferId", input.transfer_id)
                .text("namespace", input.namespace)
                .text("opaqueKey", input.opaque_key)
                .text("policyInput", &input.policy)
                .text("payloadSha256", input.payload_sha256)
                .number("payloadBytes", input.payload_bytes);
        }
        Call::Part(input) => {
            object
                .text("transferId", input.transfer_id)
                .number("index", input.index)
                .text("bytes", &input.bytes);
        }
        Call::Commit(input) | Call::Discard(input) => {
            object.text("transferId", input.transfer_id);
        }
        Call::Status(input) => {
            object
                .text("transferId", input.transfer_id)
                .text("namespace", input.namespace)
                .text("policyInput", &input.policy);
        }
        Call::Read(input) => {
            object
                .text("namespace", input.namespace)
                .text("opaqueKey", input.opaque_key)
                .text("policyInput", &input.policy)
                .text("payloadSha256", input.payload_sha256)
                .number("payloadBytes", input.payload_bytes)
                .number("offset", input.offset)
                .number("count", input.count);
        }
    }
    object.finish()
}

fn success(success: &Success) -> String {
    let mut object = Object::new();
    match success {
        Success::Config(value) => {
            object.text("result", "config");
            config(&mut object, value);
        }
        Success::Pending {
            next_index,
            received,
            expires_at_ms,
        } => {
            object
                .text("result", "pending")
                .number("nextIndex", next_index)
                .number("received", received);
            if let Some(expiry) = expires_at_ms {
                object.number("expiresAtMs", expiry);
            }
        }
        Success::Progress {
            next_index,
            received,
        } => {
            object
                .text("result", "progress")
                .number("nextIndex", next_index)
                .number("received", received);
        }
        Success::Committed {
            opaque_key,
            payload_sha256,
            payload_bytes,
        } => {
            object
                .text("result", "committed")
                .text("opaqueKey", opaque_key)
                .text("payloadSha256", payload_sha256)
                .number("payloadBytes", payload_bytes);
        }
        Success::State(state) => {
            object.text("result", "state").text(
                "state",
                match state {
                    State::Expired => "expired",
                    State::Discarded => "discarded",
                    State::Unavailable => "unavailable",
                    State::Unknown => "unknown",
                    State::NotObserved => "notObserved",
                },
            );
        }
        Success::Read {
            offset,
            total_bytes,
            bytes,
        } => {
            object
                .text("result", "read")
                .number("offset", offset)
                .number("totalBytes", total_bytes)
                .text("bytes", bytes);
        }
    }
    object.finish()
}

fn config(object: &mut Object, config: &Config) {
    let mut backend = Object::new();
    backend
        .text("id", &config.backend_id)
        .text("source", "default")
        .raw("editable", false);
    let mut capabilities = Object::new();
    capabilities
        .raw("immutableCreate", config.immutable_create)
        .raw("chunkedRead", config.chunked_read)
        .raw("recoverByOriginalId", config.recover_by_original_id);
    let mut limits = Object::new();
    let (projection, bounds) = match config.limits {
        Limits::Browser(bounds) => ("browser", bounds),
        Limits::Local { bounds, .. } => ("local", bounds),
    };
    limits
        .number("payloadBytes", bounds.payload_bytes)
        .number("chunkBytes", bounds.chunk_bytes);
    if let Limits::Local {
        namespace_bytes,
        extension_bytes,
        installation_bytes,
        ..
    } = config.limits
    {
        limits
            .number("namespaceBytes", namespace_bytes)
            .number("extensionBytes", extension_bytes)
            .number("installationBytes", installation_bytes);
    }
    object
        .text("projection", projection)
        .raw("backend", backend.finish())
        .raw("capabilities", capabilities.finish())
        .raw("limits", limits.finish());
}

fn error_code(code: ErrorCode) -> String {
    let mut object = Object::new();
    match code {
        ErrorCode::Denied => object.text("code", "denied"),
        ErrorCode::Unavailable => object.text("code", "unavailable"),
        ErrorCode::Invalid => object.text("code", "invalid"),
        ErrorCode::Conflict => object.text("code", "conflict"),
        ErrorCode::NotFound => object.text("code", "not-found"),
        ErrorCode::Unknown => object.text("code", "unknown"),
        ErrorCode::Capacity(limit) => object.text("code", "capacity").text(
            "limit",
            match limit {
                Limit::NamespaceBytes => "namespace-bytes",
                Limit::ExtensionBytes => "extension-bytes",
                Limit::InstallationBytes => "installation-bytes",
                Limit::NamespaceEntries => "namespace-entries",
                Limit::ExtensionEntries => "extension-entries",
                Limit::InstallationEntries => "installation-entries",
                Limit::ActiveIntents => "active-intents",
                Limit::RetainedExtension => "retained-extension",
                Limit::RetainedInstallation => "retained-installation",
                Limit::Requests => "requests",
            },
        ),
    };
    object.finish()
}
