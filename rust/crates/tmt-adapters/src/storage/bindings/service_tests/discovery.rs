//! Real SQLite transactions with deterministic native target replacement.
use super::{
    super::test_support::Fixture,
    endpoint::{EndpointFailure, FakeEndpoint},
};
use tmt_core::{
    binding::{
        self, Binding, BindingEndpoint, BindingError, GuardedBindingEndpoint, GuardedIdentity,
    },
    discovery::DiscoveryTarget,
    endpoint::{EndpointProbe, EndpointSnapshot, ProcessIncarnation, ServerEvidence},
    identity::{Identity, IdentityReader, Lifetime, create_or_resolve},
};

struct GuardedEndpoint {
    inner: FakeEndpoint,
    reads: usize,
    replace_at: Option<usize>,
    marker_at: Option<usize>,
}
impl GuardedEndpoint {
    fn new() -> Self {
        let mut inner = FakeEndpoint::new(&["%1", "%2"]);
        inner.starts.insert(100, "pane-native".into());
        inner.pane_mut("%1").pane_incarnation = Some("pane-native".into());
        Self {
            inner,
            reads: 0,
            replace_at: None,
            marker_at: None,
        }
    }
    fn target(&mut self) -> DiscoveryTarget {
        self.discovery_target("%1").unwrap().unwrap()
    }
}
impl BindingEndpoint for GuardedEndpoint {
    type Error = EndpointFailure;
    fn begin_coordination(&mut self) {
        self.inner.begin_coordination()
    }
    fn budget_available(&self) -> bool {
        self.inner.budget_available()
    }
    fn current_host(&self) -> tmt_core::host::HostKind {
        self.inner.current_host()
    }
    fn current_snapshot(&mut self, panes: &[String]) -> Result<EndpointSnapshot, Self::Error> {
        self.inner.current_snapshot(panes)
    }
    fn probe_binding(
        &mut self,
        server: &ServerEvidence,
        panes: &[String],
    ) -> Result<EndpointProbe, Self::Error> {
        self.inner.probe_binding(server, panes)
    }
    fn publish(&mut self, b: &Binding, i: &Identity) -> Result<(), Self::Error> {
        self.inner.publish(b, i)
    }
    fn clear(&mut self, b: &Binding, i: &Identity) -> Result<bool, Self::Error> {
        self.inner.clear(b, i)
    }
    fn pane_incarnation(
        &mut self,
        s: &ServerEvidence,
        p: u64,
    ) -> Result<Option<String>, Self::Error> {
        self.inner.pane_incarnation(s, p)
    }
}
impl GuardedBindingEndpoint for GuardedEndpoint {
    fn discovery_target(&mut self, pane: &str) -> Result<Option<DiscoveryTarget>, Self::Error> {
        self.reads += 1;
        let server = self.inner.server.clone();
        let observed = self.inner.pane_mut(pane).clone();
        let mut target = DiscoveryTarget {
            socket: server.socket_path,
            server: ProcessIncarnation::new(server.server_pid, "server-native").unwrap(),
            server_id: Some(server.server_id),
            server_start_time: server.server_start_time,
            pane: pane.into(),
            process: ProcessIncarnation::new(
                observed.pane_pid,
                self.inner
                    .starts
                    .get(&observed.pane_pid)
                    .expect("known native start"),
            )
            .unwrap(),
            marker: observed.marker,
            runtime: None,
        };
        if self.replace_at == Some(self.reads) {
            target.process = ProcessIncarnation::new(observed.pane_pid, "reused-native").unwrap();
        }
        if self.marker_at == Some(self.reads) {
            target.server =
                ProcessIncarnation::new(target.server.pid(), "restarted-server").unwrap();
        }
        Ok(Some(target))
    }
}

#[test]
fn guarded_add_refuses_drift_at_both_read_only_admissions_before_identity_or_host_effects() {
    for read in [2, 3] {
        let fixture = Fixture::new();
        let mut storage = fixture.open();
        let mut endpoint = GuardedEndpoint::new();
        let target = endpoint.target();
        endpoint.replace_at = Some(read);
        let error = binding::bind_guarded(
            &mut storage,
            &mut endpoint,
            &target,
            GuardedIdentity::Name("new"),
            true,
        )
        .unwrap_err();
        assert!(matches!(error, BindingError::TargetChanged(_)));
        assert!(storage.list_identities().unwrap().is_empty());
        assert_eq!(endpoint.inner.current_calls, 0);
        assert_eq!(endpoint.inner.publish_calls, 0);
        storage.close().unwrap();
    }
}

#[test]
fn guarded_add_binds_uuid_not_a_name_and_is_idempotent_with_a_fresh_expectation() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = GuardedEndpoint::new();
    let target = endpoint.target();
    let first = binding::bind_guarded(
        &mut storage,
        &mut endpoint,
        &target,
        GuardedIdentity::Name("first"),
        false,
    )
    .unwrap();
    assert!(first.bound.created);
    assert!(!first.already_bound);
    assert_eq!(
        first
            .bound
            .presence
            .binding
            .as_ref()
            .unwrap()
            .pane_incarnation
            .as_deref(),
        Some("pane-native")
    );
    let id = first.bound.presence.identity.id.clone();
    binding::rename_identity::<_, EndpointFailure>(&mut storage, "first", "renamed").unwrap();
    let next = endpoint.target();
    let second = binding::bind_guarded(
        &mut storage,
        &mut endpoint,
        &next,
        GuardedIdentity::Uuid(&id),
        true,
    )
    .unwrap();
    assert!(second.already_bound);
    assert!(!second.bound.created);
    assert_eq!(second.bound.presence.identity.name, "renamed");
    assert_eq!(second.bound.presence.identity.lifetime, Lifetime::Saved);
    assert_eq!(endpoint.inner.publish_calls, 1);
    assert_eq!(second.bound.presence.binding, first.bound.presence.binding);
    assert_eq!(storage.list_identities().unwrap().len(), 1);
    let before = endpoint.inner.current_calls;
    let old = binding::bind_guarded(
        &mut storage,
        &mut endpoint,
        &target,
        GuardedIdentity::Uuid(&id),
        true,
    )
    .unwrap_err();
    assert!(matches!(old, BindingError::TargetChanged(_)));
    assert_eq!(endpoint.inner.current_calls, before);
    storage.close().unwrap();
}

#[test]
fn existing_canonical_name_and_absent_uuid_refuse_before_effects() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = GuardedEndpoint::new();
    let identity = create_or_resolve(&mut storage, "Known", Lifetime::Saved)
        .unwrap()
        .identity;
    let target = endpoint.target();
    let error = binding::bind_guarded(
        &mut storage,
        &mut endpoint,
        &target,
        GuardedIdentity::Name("known"),
        true,
    )
    .unwrap_err();
    assert!(matches!(error, BindingError::NameTaken(_)));
    assert_eq!(endpoint.inner.current_calls, 0);
    let absent = binding::bind_guarded(
        &mut storage,
        &mut endpoint,
        &target,
        GuardedIdentity::Uuid("missing-uuid"),
        false,
    )
    .unwrap_err();
    assert!(matches!(absent, BindingError::NameNotFound(_)));
    let bound = binding::bind_guarded(
        &mut storage,
        &mut endpoint,
        &target,
        GuardedIdentity::Uuid(&identity.id),
        false,
    )
    .unwrap();
    assert_eq!(bound.bound.presence.identity.id, identity.id);
    assert!(!bound.bound.created);
    assert_eq!(storage.list_identities().unwrap().len(), 1);
    storage.close().unwrap();
}

#[test]
fn publication_failure_reports_partial_and_rolls_back_new_saved_identity() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = GuardedEndpoint::new();
    endpoint.inner.publish_failure = true;
    let target = endpoint.target();
    let error = binding::bind_guarded(
        &mut storage,
        &mut endpoint,
        &target,
        GuardedIdentity::Name("new saved"),
        true,
    )
    .unwrap_err();
    assert!(matches!(error, BindingError::Partial(_)));
    assert!(storage.list_identities().unwrap().is_empty());
    let marker = endpoint.inner.pane_mut("%1").marker.clone();
    assert_eq!(marker, None);
    storage.close().unwrap();
}

#[test]
fn native_replacement_after_admission_is_partial_and_never_creates_identity() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = GuardedEndpoint::new();
    let target = endpoint.target();
    endpoint.inner.replace_pane_after_current = Some(1);
    let error = binding::bind_guarded(
        &mut storage,
        &mut endpoint,
        &target,
        GuardedIdentity::Name("new"),
        true,
    )
    .unwrap_err();
    assert!(matches!(error, BindingError::Partial(_)));
    assert!(storage.list_identities().unwrap().is_empty());
    assert_eq!(endpoint.inner.publish_calls, 0);
    storage.close().unwrap();
}

#[test]
fn live_uuid_elsewhere_and_an_occupied_pane_refuse_without_promotion_or_publication() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = GuardedEndpoint::new();
    let target = endpoint.target();
    let first = binding::bind_guarded(
        &mut storage,
        &mut endpoint,
        &target,
        GuardedIdentity::Name("first"),
        false,
    )
    .unwrap();
    let id = first.bound.presence.identity.id;
    let other = endpoint.discovery_target("%2").unwrap().unwrap();
    let calls = endpoint.inner.current_calls;
    let error = binding::bind_guarded(
        &mut storage,
        &mut endpoint,
        &other,
        GuardedIdentity::Uuid(&id),
        true,
    )
    .unwrap_err();
    assert!(matches!(error, BindingError::NameAlreadyActive));
    assert_eq!(endpoint.inner.current_calls, calls);
    assert_eq!(endpoint.inner.publish_calls, 1);
    assert_eq!(
        storage.list_identities().unwrap()[0].lifetime,
        Lifetime::Temporary
    );
    let current = endpoint.target();
    let error = binding::bind_guarded(
        &mut storage,
        &mut endpoint,
        &current,
        GuardedIdentity::Name("replacement"),
        true,
    )
    .unwrap_err();
    assert!(matches!(error, BindingError::PaneAlreadyBound));
    assert_eq!(endpoint.inner.current_calls, calls);
    assert_eq!(storage.list_identities().unwrap().len(), 1);
    storage.close().unwrap();
}

#[test]
fn a_reused_native_pane_cannot_reuse_stored_marker_authority() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = GuardedEndpoint::new();
    let target = endpoint.target();
    let first = binding::bind_guarded(
        &mut storage,
        &mut endpoint,
        &target,
        GuardedIdentity::Name("first"),
        true,
    )
    .unwrap();
    let id = first.bound.presence.identity.id;
    endpoint
        .inner
        .starts
        .insert(100, "replacement-start".into());
    endpoint.inner.pane_mut("%1").pane_incarnation = Some("replacement-start".into());
    let reused = endpoint.target();
    let calls = endpoint.inner.current_calls;
    let error = binding::bind_guarded(
        &mut storage,
        &mut endpoint,
        &reused,
        GuardedIdentity::Uuid(&id),
        true,
    )
    .unwrap_err();
    assert!(matches!(error, BindingError::Unverified));
    assert_eq!(endpoint.inner.current_calls, calls);
    assert_eq!(endpoint.inner.publish_calls, 1);
    storage.close().unwrap();
}

#[test]
fn read_only_discovery_preserves_rows_and_distinguishes_known_stale_hook_only_and_duplicates() {
    use crate::{discovery::annotate, host::DiscoveryObservation};
    use tmt_core::{binding::BindingRepository, discovery::DiscoveryCandidate};
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = GuardedEndpoint::new();
    let target = endpoint.target();
    binding::bind_guarded(
        &mut storage,
        &mut endpoint,
        &target,
        GuardedIdentity::Name("first"),
        true,
    )
    .unwrap();
    let known = endpoint.target();
    let candidate = |fence: DiscoveryTarget| DiscoveryCandidate {
        pane: fence.pane.clone(),
        session: Some("work".into()),
        window: Some("0".into()),
        target: Some("work:0.0".into()),
        title: "raw title".into(),
        command: "agent".into(),
        cwd: "/tmp".into(),
        fence: Some(fence),
        identity: None,
        state: "unknown",
        reason: None,
        duplicate: false,
        text_truncated: false,
    };
    let observation = |fence: DiscoveryTarget| DiscoveryObservation {
        socket: fence.socket.clone(),
        candidates: vec![candidate(fence)],
        truncated: false,
    };
    let before = storage
        .with_binding_transaction(|records| records.binding_entries())
        .unwrap();
    let mut live = observation(known.clone());
    annotate(
        &mut live,
        &fixture.database,
        std::time::Instant::now() + std::time::Duration::from_secs(2),
    )
    .unwrap();
    assert_eq!(live.candidates[0].state, "known");
    assert!(live.candidates[0].adoptable());
    let after = storage
        .with_binding_transaction(|records| records.binding_entries())
        .unwrap();
    assert_eq!(after, before);
    let mut stale = known.clone();
    stale.process = ProcessIncarnation::new(stale.process.pid(), "replacement").unwrap();
    let mut stale = observation(stale);
    annotate(
        &mut stale,
        &fixture.database,
        std::time::Instant::now() + std::time::Duration::from_secs(2),
    )
    .unwrap();
    assert_eq!(stale.candidates[0].state, "stale");
    assert!(!stale.candidates[0].adoptable());
    let absent = Fixture::new();
    let mut hook = observation(known.clone());
    annotate(
        &mut hook,
        &absent.database,
        std::time::Instant::now() + std::time::Duration::from_secs(2),
    )
    .unwrap();
    assert_eq!(hook.candidates[0].state, "hook_only");
    assert!(!hook.candidates[0].adoptable());
    assert!(!absent.database.exists());
    let other = create_or_resolve(&mut storage, "old other", Lifetime::Saved)
        .unwrap()
        .identity;
    let mut server = endpoint.inner.server.clone();
    server.server_id = "old-server".into();
    let pane = endpoint.inner.pane_mut("%1").clone();
    storage
        .with_binding_transaction(|records| records.insert_binding(&other, &server, &pane))
        .unwrap();
    let mut duplicate = observation(known);
    annotate(
        &mut duplicate,
        &fixture.database,
        std::time::Instant::now() + std::time::Duration::from_secs(2),
    )
    .unwrap();
    assert_eq!(duplicate.candidates[0].state, "duplicate");
    assert!(duplicate.candidates[0].duplicate);
    assert!(!duplicate.candidates[0].adoptable());
    storage.close().unwrap();
}
