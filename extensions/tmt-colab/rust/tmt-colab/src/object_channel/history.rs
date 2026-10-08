//! Detached ciphertext paging over the actual current peer's history entitlement.
//! It reuses sync's catchup codec, opens no socket and cannot append or publish.
use super::admission::{CapturedTarget, PeerIdentity, RequestCapture};
use crate::{
    page,
    sync::{Access, Admission, CatchupContext, Code, DetachedCatchup, SyncScope},
};
use std::{sync::Arc, time::Instant};
use tmt_colab_model::values;

struct HistoryAdmission {
    capture: Arc<RequestCapture>,
    historical: SyncScope,
}
impl HistoryAdmission {
    fn check(&self, principal: &str, scope: &SyncScope) -> Result<(), Code> {
        if principal != self.capture.peer.principal || *scope != self.historical {
            return Err(Code::Denied);
        }
        self.capture.current().map_err(|_| Code::Denied)?;
        self.capture
            .peer
            .admission
            .attachment_context(
                principal,
                &self.capture.scope,
                values::decimal(&scope.epoch, false)?,
            )
            .map_err(|_| Code::Denied)?;
        Ok(())
    }
}
impl Admission for HistoryAdmission {
    fn alive(&self, principal: &str) -> Result<(), Code> {
        self.check(principal, &self.historical)
    }
    fn authorize(
        &self,
        principal: &str,
        scope: &SyncScope,
        access: Access<'_>,
    ) -> Result<[u8; 32], Code> {
        self.check(principal, scope)?;
        if !matches!(access, Access::Read) {
            return Err(Code::Denied);
        }
        self.capture
            .peer
            .admission
            .authorize(principal, &self.capture.scope, Access::Read)
    }
    fn catchup_context(
        &self,
        principal: &str,
        scope: &SyncScope,
        store: &crate::store::Store,
    ) -> Result<CatchupContext, Code> {
        self.check(principal, scope)?;
        let mut context =
            self.capture
                .peer
                .admission
                .catchup_context(principal, &self.capture.scope, store)?;
        context.baseline = store
            .baseline(&scope.page, values::decimal(&scope.epoch, false)?)
            .map_err(|_| Code::Denied)?
            .map(|baseline| baseline.descriptor);
        Ok(context)
    }
}
pub(super) struct History {
    pub id: String,
    pub capture: Arc<RequestCapture>,
    frames: DetachedCatchup<HistoryAdmission>,
}
impl History {
    pub fn new(
        peer: Arc<PeerIdentity>,
        scope: SyncScope,
        id: String,
        epoch: &str,
        deadline: Instant,
    ) -> crate::Result<Self> {
        values::generated_id(&id)?;
        let original = values::decimal(epoch, false)?;
        let current = values::decimal(&scope.epoch, false)?;
        if original >= current {
            return Err(page::Fault::Invalid.into());
        }
        let context = peer.capture_scope(&scope, false)?;
        peer.admission
            .attachment_context(&peer.principal, &scope, original)?;
        let source = peer
            .admission
            .save_source()
            .ok_or(page::Fault::Unavailable)?;
        let view = source()?;
        let base = page::revision(&view.store, &view.keyring, &scope.page)?;
        let historical = SyncScope {
            epoch: epoch.into(),
            ..scope.clone()
        };
        let capture = Arc::new(RequestCapture {
            peer: Arc::clone(&peer),
            scope,
            context,
            target: CapturedTarget::Scope { base },
            source,
            write: false,
            mutation: false,
            deadline,
        });
        let frames = DetachedCatchup::new(
            view.store,
            HistoryAdmission {
                capture: Arc::clone(&capture),
                historical: historical.clone(),
            },
            historical,
            peer.principal.clone(),
            current,
        )?;
        capture.current()?;
        Ok(Self {
            id,
            capture,
            frames,
        })
    }
    pub fn next(&mut self) -> crate::Result<(serde_json::Value, bool)> {
        self.capture.current()?;
        let (raw, finished) = self.frames.next()?.ok_or(page::Fault::Invalid)?;
        let frame: serde_json::Value = serde_json::from_str(&raw)?;
        self.capture.current()?;
        Ok((
            serde_json::json!({"ok":{"result":"history","frame":frame,"more":!finished}}),
            finished,
        ))
    }
}
