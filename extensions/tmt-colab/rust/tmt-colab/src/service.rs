//! Native sign-in/session authority. The loopback door admits Host/Origin before dispatch.
use crate::{
    Result,
    auth::SignInCode,
    http::{Handler, Reply, Request},
    keyring::{Keyring, Layout, MemberKeys},
    store::Store,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Deserialize;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};
use tmt_colab_model::{auth::decode_signin, crypto, values};
const STRIP_FRAGMENT: &str = "history.replaceState(null, '', location.pathname);";
const COOKIE: &str = "tmt_colab_session";
pub struct SessionService {
    space: String,
    bootstrap: String,
    state: Mutex<State>,
    stop: Arc<AtomicBool>,
}
struct State {
    store: Store,
    member: MemberKeys,
    code: SignInCode,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SignInWire {
    input: String,
    proof: String,
    signature: String,
}
impl SessionService {
    pub fn open(layout: &Layout, mut store: Store, stop: Arc<AtomicBool>) -> Result<Self> {
        let keyring = Keyring::open(layout)?;
        let member = MemberKeys::open(layout)?;
        let code = SignInCode::issue(&mut store, &keyring, &member, now()?)?;
        let space = keyring.space_id.clone();
        let bootstrap = format!("/s/{space}/signin/{}", code.id);
        Ok(Self {
            space,
            bootstrap,
            stop,
            state: Mutex::new(State {
                store,
                member,
                code,
            }),
        })
    }
    pub fn close(self) -> Result<()> {
        let state = self
            .state
            .into_inner()
            .map_err(|_| "Space cleanup encountered poisoned state.")?;
        state.store.close()
    }
    pub fn space(&self) -> &str {
        &self.space
    }
    /// Foreground startup alone deliberately discloses this single-use URL.
    pub fn signin_url(&self, base: &str) -> Result<String> {
        let state = self.state.lock().map_err(|_| "Space state unavailable.")?;
        Ok(format!(
            "{}{}#{}",
            base.trim_end_matches('/'),
            self.bootstrap,
            state.code.fragment()
        ))
    }
    fn signin(&self, body: &[u8]) -> Result<Reply> {
        let wire: SignInWire = serde_json::from_slice(body)?;
        let input = values::binary(&wire.input, 1024)?;
        let proof = values::binary(&wire.proof, 32)?;
        let signature = values::binary(&wire.signature, 64)?;
        let input = decode_signin(&input)?;
        let mut state = self.state.lock().map_err(|_| "Space state unavailable.")?;
        if self.stop.load(Ordering::Acquire) {
            return Err("Service stopped.".into());
        }
        let State {
            store,
            member,
            code,
        } = &mut *state;
        let issued = code.enroll(store, member, &input, &proof, &signature, now()?)?;
        let mut reply = Reply::text(200, &issued.chain);
        reply.content_type = "application/json";
        reply.cookie = Some(format!(
            "{COOKIE}={}; Path=/s/{}/; Max-Age=86400; HttpOnly; SameSite=Strict",
            issued.token, self.space
        ));
        Ok(reply)
    }
    fn authenticated(&self, cookie: Option<&str>) -> Result<bool> {
        let mut token = None;
        for field in cookie.unwrap_or("").split(';') {
            if let Some((name, value)) = field.trim().split_once('=')
                && name == COOKIE
                && token.replace(value).is_some()
            {
                return Ok(false);
            }
        }
        let Some(token) = token else {
            return Ok(false);
        };
        let token = values::binary(token, 32)?;
        if token.len() != 32 {
            return Ok(false);
        }
        let hash = crypto::digest(&token);
        let state = self.state.lock().map_err(|_| "Space state unavailable.")?;
        Ok(state.store.session(&hash, &self.space, now()?)?.is_some())
    }
}
impl Handler for SessionService {
    fn navigation(&self, path: &str) -> bool {
        path == "/" || path == self.bootstrap
    }
    fn handle(&self, request: Request) -> Reply {
        if self.stop.load(Ordering::Acquire) {
            return Reply::text(403, b"DENIED");
        }
        if request.method == "GET" && self.navigation(&request.path) && !request.upgrade {
            let html = format!(
                "<!doctype html><html lang=\"en\"><meta charset=\"utf-8\"><title>TMT Colab</title><h1>TMT Colab</h1><p>Native space is running. Browser enrollment is supplied by the Colab client.</p><script>{STRIP_FRAGMENT}</script></html>"
            );
            let mut reply = Reply::text(200, html.as_bytes());
            reply.content_type = "text/html; charset=utf-8";
            reply.script_hash = Some(STANDARD.encode(crypto::digest(STRIP_FRAGMENT.as_bytes())));
            return reply;
        }
        if request.method == "POST"
            && request.path == format!("/s/{}/signin", self.space)
            && !request.upgrade
        {
            return self
                .signin(&request.body)
                .unwrap_or_else(|_| Reply::text(403, b"DENIED"));
        }
        // Authentication is checked even though management/sync remain unavailable in this slice.
        let _ = self.authenticated(request.cookie.as_deref());
        Reply::text(403, b"DENIED")
    }
}
fn now() -> Result<u64> {
    let millis = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let now = u64::try_from(millis)?;
    values::time(now)?;
    Ok(now)
}
