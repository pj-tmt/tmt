//! Typed reading of an admitted JSON value. Structure (a missing, unknown or wrongly
//! typed field, an unknown discriminator) is `Shape`; a well-formed value outside its
//! grammar or range is `Value`. Admission already guarantees every number is an
//! unsigned integer no larger than 2^53-1 and every object member name is unique.
use super::*;
use serde_json::{Map, Value};

/// One JSON object whose fields are each taken exactly once; any field left
/// over when it is finished is unknown.
struct Object<'a> {
    members: &'a Map<String, Value>,
    taken: usize,
}
impl<'a> Object<'a> {
    fn new(value: &'a Value) -> Result<Self, ErrorClass> {
        let members = value.as_object().ok_or(ErrorClass::Shape)?;
        Ok(Self { members, taken: 0 })
    }
    fn take(&mut self, name: &str) -> Result<&'a Value, ErrorClass> {
        self.optional(name).ok_or(ErrorClass::Shape)
    }
    fn optional(&mut self, name: &str) -> Option<&'a Value> {
        let value = self.members.get(name)?;
        self.taken += 1;
        Some(value)
    }
    fn finish(self) -> Result<(), ErrorClass> {
        (self.taken == self.members.len())
            .then_some(())
            .ok_or(ErrorClass::Shape)
    }
}

fn text(value: &Value) -> Result<&str, ErrorClass> {
    value.as_str().ok_or(ErrorClass::Shape)
}
fn boolean(value: &Value) -> Result<bool, ErrorClass> {
    value.as_bool().ok_or(ErrorClass::Shape)
}
fn number(value: &Value) -> Result<u64, ErrorClass> {
    value.as_u64().ok_or(ErrorClass::Shape)
}
fn index(value: &Value) -> Result<u32, ErrorClass> {
    u32::try_from(number(value)?).map_err(|_| ErrorClass::Value)
}
fn uuid(value: &Value) -> Result<Uuid4, ErrorClass> {
    Uuid4::parse(text(value)?)
}

pub(super) fn frame(root: &Value) -> Result<Frame, ErrorClass> {
    let mut object = Object::new(root)?;
    if number(object.take("version")?)? != 1 {
        return Err(ErrorClass::Value);
    }
    match text(object.take("kind")?)? {
        "request" => request(object).map(Frame::Request),
        "result" => result(object).map(Frame::Result),
        "admit" => admit(object).map(Frame::Admit),
        "admission" => admission(object).map(Frame::Admission),
        "origin-state" => origin_state(object).map(Frame::OriginState),
        _ => Err(ErrorClass::Shape),
    }
}

fn request(mut object: Object) -> Result<Request, ErrorClass> {
    let generation = uuid(object.take("generation")?)?;
    let request_id = Counter::parse(text(object.take("requestId")?)?)?;
    let origin = origin(object.take("origin")?)?;
    let method = method(object.take("method")?)?;
    let input = object.take("input")?;
    object.finish()?;
    Ok(Request {
        generation,
        request_id,
        origin,
        call: call(method, input)?,
    })
}

fn origin(value: &Value) -> Result<Origin, ErrorClass> {
    let mut object = Object::new(value)?;
    let origin = match text(object.take("kind")?)? {
        "local-extension" => Origin::LocalExtension,
        "mounted" => Origin::Mounted(uuid(object.take("originId")?)?),
        _ => return Err(ErrorClass::Shape),
    };
    object.finish()?;
    Ok(origin)
}

fn method(value: &Value) -> Result<Method, ErrorClass> {
    Ok(match text(value)? {
        "objects.config" => Method::Config,
        "objects.begin" => Method::Begin,
        "objects.part" => Method::Part,
        "objects.commit" => Method::Commit,
        "objects.status" => Method::Status,
        "objects.read" => Method::Read,
        "objects.discard" => Method::Discard,
        _ => return Err(ErrorClass::Shape),
    })
}

fn call(method: Method, input: &Value) -> Result<Call, ErrorClass> {
    let mut object = Object::new(input)?;
    let call = match method {
        Method::Config => Call::Config(config_input(&mut object)?),
        Method::Begin => Call::Begin(begin_input(&mut object)?),
        Method::Part => Call::Part(PartInput {
            transfer_id: uuid(object.take("transferId")?)?,
            index: index(object.take("index")?)?,
            bytes: Chunk::parse(text(object.take("bytes")?)?)?,
        }),
        Method::Commit => Call::Commit(transfer(&mut object)?),
        Method::Discard => Call::Discard(transfer(&mut object)?),
        Method::Status => Call::Status(status_input(&mut object)?),
        Method::Read => Call::Read(read_input(&mut object)?),
    };
    object.finish()?;
    Ok(call)
}
fn config_input(object: &mut Object) -> Result<ConfigInput, ErrorClass> {
    Ok(ConfigInput {
        namespace: Bytes32::parse(text(object.take("namespace")?)?)?,
        policy: Policy::parse(text(object.take("policyInput")?)?)?,
    })
}
fn begin_input(object: &mut Object) -> Result<BeginInput, ErrorClass> {
    Ok(BeginInput {
        transfer_id: uuid(object.take("transferId")?)?,
        namespace: Bytes32::parse(text(object.take("namespace")?)?)?,
        opaque_key: Bytes32::parse(text(object.take("opaqueKey")?)?)?,
        policy: Policy::parse(text(object.take("policyInput")?)?)?,
        payload_sha256: Sha256Hex::parse(text(object.take("payloadSha256")?)?)?,
        payload_bytes: number(object.take("payloadBytes")?)?,
    })
}
fn status_input(object: &mut Object) -> Result<StatusInput, ErrorClass> {
    Ok(StatusInput {
        transfer_id: uuid(object.take("transferId")?)?,
        namespace: Bytes32::parse(text(object.take("namespace")?)?)?,
        policy: Policy::parse(text(object.take("policyInput")?)?)?,
    })
}
fn read_input(object: &mut Object) -> Result<ReadInput, ErrorClass> {
    Ok(ReadInput {
        namespace: Bytes32::parse(text(object.take("namespace")?)?)?,
        opaque_key: Bytes32::parse(text(object.take("opaqueKey")?)?)?,
        policy: Policy::parse(text(object.take("policyInput")?)?)?,
        payload_sha256: Sha256Hex::parse(text(object.take("payloadSha256")?)?)?,
        payload_bytes: number(object.take("payloadBytes")?)?,
        offset: number(object.take("offset")?)?,
        count: index(object.take("count")?)?,
    })
}
fn transfer(object: &mut Object) -> Result<TransferInput, ErrorClass> {
    Ok(TransferInput {
        transfer_id: uuid(object.take("transferId")?)?,
    })
}

fn result(mut object: Object) -> Result<ResultFrame, ErrorClass> {
    let generation = uuid(object.take("generation")?)?;
    let request_id = Counter::parse(text(object.take("requestId")?)?)?;
    let method = method(object.take("method")?)?;
    let transfer_id = object.optional("transferId").map(uuid).transpose()?;
    let outcome = match (object.optional("ok"), object.optional("error")) {
        (Some(ok), None) => Outcome::Success(success(ok)?),
        (None, Some(error)) => Outcome::Failure(error_code(error)?),
        _ => return Err(ErrorClass::Shape),
    };
    object.finish()?;
    Ok(ResultFrame {
        generation,
        request_id,
        method,
        transfer_id,
        outcome,
    })
}

fn success(value: &Value) -> Result<Success, ErrorClass> {
    let mut object = Object::new(value)?;
    let success = match text(object.take("result")?)? {
        "config" => Success::Config(config(&mut object)?),
        "pending" => Success::Pending {
            next_index: index(object.take("nextIndex")?)?,
            received: number(object.take("received")?)?,
            expires_at_ms: object.optional("expiresAtMs").map(number).transpose()?,
        },
        "progress" => Success::Progress {
            next_index: index(object.take("nextIndex")?)?,
            received: number(object.take("received")?)?,
        },
        "committed" => Success::Committed {
            opaque_key: Bytes32::parse(text(object.take("opaqueKey")?)?)?,
            payload_sha256: Sha256Hex::parse(text(object.take("payloadSha256")?)?)?,
            payload_bytes: number(object.take("payloadBytes")?)?,
        },
        "state" => Success::State(state(object.take("state")?)?),
        "read" => Success::Read {
            offset: number(object.take("offset")?)?,
            total_bytes: number(object.take("totalBytes")?)?,
            bytes: Chunk::parse(text(object.take("bytes")?)?)?,
        },
        _ => return Err(ErrorClass::Shape),
    };
    object.finish()?;
    Ok(success)
}

fn config(object: &mut Object) -> Result<Config, ErrorClass> {
    let projection = text(object.take("projection")?)?;
    let mut backend = Object::new(object.take("backend")?)?;
    let backend_id = text(backend.take("id")?)?.to_owned();
    if text(backend.take("source")?)? != "default" || boolean(backend.take("editable")?)? {
        return Err(ErrorClass::Value);
    }
    backend.finish()?;
    let mut capabilities = Object::new(object.take("capabilities")?)?;
    let immutable_create = boolean(capabilities.take("immutableCreate")?)?;
    let chunked_read = boolean(capabilities.take("chunkedRead")?)?;
    let recover_by_original_id = boolean(capabilities.take("recoverByOriginalId")?)?;
    capabilities.finish()?;
    let mut fields = Object::new(object.take("limits")?)?;
    let bounds = TransferBounds {
        payload_bytes: number(fields.take("payloadBytes")?)?,
        chunk_bytes: index(fields.take("chunkBytes")?)?,
    };
    let limits = match projection {
        "browser" => Limits::Browser(bounds),
        "local" => Limits::Local {
            bounds,
            namespace_bytes: number(fields.take("namespaceBytes")?)?,
            extension_bytes: number(fields.take("extensionBytes")?)?,
            installation_bytes: number(fields.take("installationBytes")?)?,
        },
        _ => return Err(ErrorClass::Value),
    };
    fields.finish()?;
    Ok(Config {
        backend_id,
        immutable_create,
        chunked_read,
        recover_by_original_id,
        limits,
    })
}

fn error_code(value: &Value) -> Result<ErrorCode, ErrorClass> {
    let mut object = Object::new(value)?;
    let code = match text(object.take("code")?)? {
        "denied" => ErrorCode::Denied,
        "unavailable" => ErrorCode::Unavailable,
        "invalid" => ErrorCode::Invalid,
        "conflict" => ErrorCode::Conflict,
        "not-found" => ErrorCode::NotFound,
        "unknown" => ErrorCode::Unknown,
        "capacity" => ErrorCode::Capacity(match text(object.take("limit")?)? {
            "namespace-bytes" => Limit::NamespaceBytes,
            "extension-bytes" => Limit::ExtensionBytes,
            "installation-bytes" => Limit::InstallationBytes,
            "namespace-entries" => Limit::NamespaceEntries,
            "extension-entries" => Limit::ExtensionEntries,
            "installation-entries" => Limit::InstallationEntries,
            "active-intents" => Limit::ActiveIntents,
            "retained-extension" => Limit::RetainedExtension,
            "retained-installation" => Limit::RetainedInstallation,
            "requests" => Limit::Requests,
            _ => return Err(ErrorClass::Value),
        }),
        _ => return Err(ErrorClass::Value),
    };
    object.finish()?;
    Ok(code)
}

fn admit(mut object: Object) -> Result<Admit, ErrorClass> {
    let generation = uuid(object.take("generation")?)?;
    let callback_id = Counter::parse(text(object.take("callbackId")?)?)?;
    let request_id = Counter::parse(text(object.take("requestId")?)?)?;
    let boundary = match text(object.take("boundary")?)? {
        "acquire" => Checkpoint::Acquire,
        "effect" => Checkpoint::Effect,
        "disclose" => Checkpoint::Disclose,
        _ => return Err(ErrorClass::Value),
    };
    let context = context(object.take("context")?)?;
    let operation = operation(object.take("operation")?)?;
    object.finish()?;
    Ok(Admit {
        generation,
        callback_id,
        request_id,
        boundary,
        context,
        operation,
    })
}

fn context(value: &Value) -> Result<Context, ErrorClass> {
    let mut object = Object::new(value)?;
    let context = match text(object.take("kind")?)? {
        "owner-session" => Context::OwnerSession {
            origin_id: uuid(object.take("originId")?)?,
            device_id: uuid(object.take("deviceId")?)?,
            grant_revision: number(object.take("grantRevision")?)?,
        },
        "mounted" => Context::Mounted {
            origin_id: uuid(object.take("originId")?)?,
        },
        "local-extension" => Context::LocalExtension,
        _ => return Err(ErrorClass::Shape),
    };
    object.finish()?;
    Ok(context)
}

fn operation(value: &Value) -> Result<Operation, ErrorClass> {
    let mut object = Object::new(value)?;
    let method = method(object.take("method")?)?;
    let input = admit_input(method, object.take("input")?)?;
    let disclosure = object.optional("disclosure").map(disclosure).transpose()?;
    object.finish()?;
    Ok(Operation { input, disclosure })
}

fn admit_input(method: Method, value: &Value) -> Result<AdmitInput, ErrorClass> {
    let mut object = Object::new(value)?;
    let input = match method {
        Method::Config => AdmitInput::Config(config_input(&mut object)?),
        Method::Begin => AdmitInput::Begin(begin_input(&mut object)?),
        Method::Status => AdmitInput::Status(status_input(&mut object)?),
        Method::Read => AdmitInput::Read(read_input(&mut object)?),
        Method::Part => AdmitInput::Part(PartAdmit {
            transfer_id: uuid(object.take("transferId")?)?,
            index: index(object.take("index")?)?,
            length: index(object.take("length")?)?,
            retained: retained(object.take("retained")?)?,
        }),
        Method::Commit => AdmitInput::Commit(transfer_admit(&mut object)?),
        Method::Discard => AdmitInput::Discard(transfer_admit(&mut object)?),
    };
    object.finish()?;
    Ok(input)
}
fn transfer_admit(object: &mut Object) -> Result<TransferAdmit, ErrorClass> {
    Ok(TransferAdmit {
        transfer_id: uuid(object.take("transferId")?)?,
        retained: retained(object.take("retained")?)?,
    })
}
fn retained(value: &Value) -> Result<Retained, ErrorClass> {
    let mut object = Object::new(value)?;
    let retained = Retained {
        namespace: Bytes32::parse(text(object.take("namespace")?)?)?,
        opaque_key: Bytes32::parse(text(object.take("opaqueKey")?)?)?,
        policy: Policy::parse(text(object.take("policyInput")?)?)?,
        payload_sha256: Sha256Hex::parse(text(object.take("payloadSha256")?)?)?,
        payload_bytes: number(object.take("payloadBytes")?)?,
    };
    object.finish()?;
    Ok(retained)
}

fn disclosure(value: &Value) -> Result<Disclosure, ErrorClass> {
    let mut object = Object::new(value)?;
    let disclosure = match text(object.take("class")?)? {
        "config" => Disclosure::Config {
            projection: match text(object.take("projection")?)? {
                "browser" => Projection::Browser,
                "local" => Projection::Local,
                _ => return Err(ErrorClass::Value),
            },
        },
        "status" => Disclosure::Status {
            next_index: index(object.take("nextIndex")?)?,
            received: number(object.take("received")?)?,
            expires_at_ms: object.optional("expiresAtMs").map(number).transpose()?,
        },
        "progress" => Disclosure::Progress {
            next_index: index(object.take("nextIndex")?)?,
            received: number(object.take("received")?)?,
        },
        "receipt" => Disclosure::Receipt {
            opaque_key: Bytes32::parse(text(object.take("opaqueKey")?)?)?,
            payload_sha256: Sha256Hex::parse(text(object.take("payloadSha256")?)?)?,
            payload_bytes: number(object.take("payloadBytes")?)?,
        },
        "terminal" => Disclosure::Terminal {
            state: state(object.take("state")?)?,
        },
        "bytes" => Disclosure::Bytes {
            offset: number(object.take("offset")?)?,
            length: index(object.take("length")?)?,
        },
        _ => return Err(ErrorClass::Shape),
    };
    object.finish()?;
    Ok(disclosure)
}

fn admission(mut object: Object) -> Result<Admission, ErrorClass> {
    let generation = uuid(object.take("generation")?)?;
    let callback_id = Counter::parse(text(object.take("callbackId")?)?)?;
    let request_id = Counter::parse(text(object.take("requestId")?)?)?;
    let decision = match text(object.take("decision")?)? {
        "allow" => Decision::Allow,
        "deny" => Decision::Deny,
        "unavailable" => Decision::Unavailable,
        _ => return Err(ErrorClass::Value),
    };
    object.finish()?;
    Ok(Admission {
        generation,
        callback_id,
        request_id,
        decision,
    })
}

fn origin_state(mut object: Object) -> Result<OriginState, ErrorClass> {
    let generation = uuid(object.take("generation")?)?;
    let origin_id = uuid(object.take("originId")?)?;
    let phase = match text(object.take("state")?)? {
        "established" => OriginPhase::Established,
        "closed" => OriginPhase::Closed,
        _ => return Err(ErrorClass::Value),
    };
    object.finish()?;
    Ok(OriginState {
        generation,
        origin_id,
        phase,
    })
}

fn state(value: &Value) -> Result<State, ErrorClass> {
    Ok(match text(value)? {
        "expired" => State::Expired,
        "discarded" => State::Discarded,
        "unavailable" => State::Unavailable,
        "unknown" => State::Unknown,
        "notObserved" => State::NotObserved,
        _ => return Err(ErrorClass::Value),
    })
}
