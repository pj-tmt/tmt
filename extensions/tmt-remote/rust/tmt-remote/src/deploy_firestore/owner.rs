//! Explicit owner-access step. Ordinary Auth keeps credentials private; the
//! authorized helper receives only the native public pin and Firebase uid.
use super::*;
use crate::{
    cloud_keys::CloudKeys,
    firebase_auth::{AuthState, Client, FileStore, Project},
    native_sync::projects::ProjectPorts,
    state::{Layout, MachineKey},
    store::Store,
};
use base64::{Engine, engine::general_purpose::STANDARD};

pub(super) struct OwnerContext {
    layout: Layout,
    ports: Option<ProjectPorts>,
}
struct Registration {
    machine: String,
    key: [u8; 32],
    descriptor: Vec<u8>,
    uid: String,
}
impl OwnerContext {
    fn identity(
        &self,
        config: &Value,
        sign_in: bool,
        stop: &AtomicBool,
    ) -> Result<Option<Registration>, crate::error::RemoteError> {
        let mapped = |error: crate::firebase_auth::AuthError| {
            crate::error::RemoteError::new(error.code(), "Machine sign-in could not be confirmed.")
        };
        let project = Project::from_public_config(config).map_err(mapped)?;
        let ports = match &self.ports {
            Some(ports) => ports.clone(),
            None => ProjectPorts::production(&project)?,
        };
        let credentials = FileStore::new(&self.layout, &project.project_id).map_err(mapped)?;
        let mut client = Client::load(
            project,
            &credentials,
            ports.http.as_ref(),
            ports.clock.as_ref(),
            ports.jitter.as_ref(),
            stop,
        )
        .map_err(mapped)?;
        // Match sync sign-in: reuse an active uid; only this authorized effect
        // may create one. Observation and Verify never sign in or refresh it.
        if client.status().state != AuthState::Active {
            if !sign_in {
                client.stop();
                return Ok(None);
            }
            let result = client.sign_in();
            if let Err(error) = result {
                client.stop();
                return Err(mapped(error));
            }
        }
        let uid = client.status().uid.ok_or_else(|| {
            crate::error::RemoteError::new(
                "REMOTE_CLOUD_UNAVAILABLE",
                "Machine sign-in could not be confirmed.",
            )
        })?;
        client.stop();
        // Live state is reached through control; a stopped Store opens only
        // under its existing serve lease. Never open another live connection.
        let machine = if let Some(status) = crate::control::status(&self.layout.directory, true)? {
            status["machineId"]
                .as_str()
                .ok_or_else(crate::operations::invalid)?
                .to_owned()
        } else {
            let lease = self.layout.serve_lock()?;
            Store::open(&lease)?.machine()?.id
        };
        let key = MachineKey::open(&self.layout)?;
        let keys = CloudKeys::new(self.layout.clone(), machine.clone(), key.clone());
        let now = crate::pairing::now_ms()?;
        keys.maintain(now)?;
        let descriptor = keys.advertised(now)?.bytes().to_vec();
        Ok(Some(Registration {
            machine,
            key: key.public(),
            descriptor,
            uid,
        }))
    }
}
impl DeployFirestore<'_> {
    pub(crate) fn prepare_owner_access(&mut self, layout: &Layout) {
        self.owner = Some(OwnerContext {
            layout: layout.clone(),
            ports: None,
        });
    }
    pub(super) fn owner_step(
        &mut self,
        apply: bool,
        plan: &DeployPlan,
        deadline: Instant,
    ) -> Result<Value, DeployProviderError> {
        let app = self
            .hosting
            .as_ref()
            .and_then(|cp| cp.app_id.as_deref())
            .or_else(|| {
                plan.view()
                    .hosting
                    .as_ref()
                    .and_then(|h| h.web_app.as_deref())
            })
            .ok_or_else(unknown)?;
        let config = self
            .exchange(
                "owner-access-config",
                json!({"project":plan.view().project,"appId":app}),
                Some(plan.account()),
                deadline,
            )
            .map_err(provider_error)?;
        if config["projectId"] != plan.view().project {
            return Err(unknown());
        }
        let identity = self
            .owner
            .as_ref()
            .ok_or_else(unknown)?
            .identity(&config, apply, self.stop)
            .map_err(|_| {
                DeployProviderError::Unconfirmed(StepCause::new(
                    CauseCode::CallIncomplete,
                    CausePhase::Credentials,
                ))
            })?;
        let Some(identity) = identity else {
            return Ok(json!("absent"));
        };
        self.exchange(
            if apply { "apply-owner-access" } else { "observe-owner-access" },
            json!({"project":plan.view().project,"machine":identity.machine,"uid":identity.uid,
                "authenticationKey":STANDARD.encode(identity.key),"descriptor":STANDARD.encode(identity.descriptor)}),
            Some(plan.account()), deadline,
        ).map_err(provider_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        canonical,
        firebase_auth::{
            AuthError, Cancellation, Http, Jitter, Operation, Request, Response, SystemClock,
        },
    };
    use std::sync::{Arc, atomic::AtomicUsize};
    struct Signup(AtomicUsize);
    impl Http for Signup {
        fn send(
            &self,
            request: &Request,
            _: u64,
            _: &dyn Cancellation,
        ) -> Result<Response, AuthError> {
            assert_eq!(
                request.operation(),
                Operation::SignUp,
                "only the explicit owner effect may sign in"
            );
            self.0.fetch_add(1, Ordering::SeqCst);
            let token = format!("e30.{}.test", canonical::base64url(json!({"aud":"demo-owner-access","sub":"native-owner","firebase":{"sign_in_provider":"anonymous"}}).to_string().as_bytes()));
            Ok(Response { status:200, retry_after_ms:None,
                body:serde_json::to_vec(&json!({"localId":"native-owner","idToken":token,"refreshToken":"private-fixture-refresh","expiresIn":"3600"})).unwrap() })
        }
    }
    struct NoJitter;
    impl Jitter for NoJitter {
        fn milliseconds(&self, _: u64) -> u64 {
            0
        }
    }
    #[test]
    fn owner_observation_never_signs_in_and_explicit_setup_reuses_the_ordinary_identity() {
        let root = std::env::temp_dir().join(format!(
            "tmt-owner-access-{}",
            crate::store::uuid_v4().unwrap()
        ));
        let layout = Layout::open(&root).unwrap();
        let wire = Arc::new(Signup(AtomicUsize::new(0)));
        let ports = ProjectPorts {
            http: wire.clone(),
            clock: Arc::new(SystemClock::default()),
            jitter: Arc::new(NoJitter),
        };
        let context = OwnerContext {
            layout: layout.clone(),
            ports: Some(ports.clone()),
        };
        let config = json!({"projectId":"demo-owner-access","appId":"1:12345:web:fixture","apiKey":"public-fixture","authDomain":"demo-owner-access.firebaseapp.com"});
        let stop = AtomicBool::new(false);
        assert!(context.identity(&config, false, &stop).unwrap().is_none());
        assert_eq!(wire.0.load(Ordering::SeqCst), 0);
        assert!(!layout.directory.join("machine.key").exists());
        assert!(!layout.directory.join("remote.db").exists());
        let first = context.identity(&config, true, &stop).unwrap().unwrap();
        assert_eq!(first.uid, "native-owner");
        assert_eq!(wire.0.load(Ordering::SeqCst), 1);
        drop(context);
        let context = OwnerContext {
            layout: layout.clone(),
            ports: Some(ports),
        };
        for apply in [false, true] {
            let next = context.identity(&config, apply, &stop).unwrap().unwrap();
            assert_eq!(next.uid, first.uid);
            assert_eq!(next.machine, first.machine);
            assert_eq!(next.key, first.key);
        }
        assert_eq!(
            wire.0.load(Ordering::SeqCst),
            1,
            "restart and Verify reuse the same uid without any Auth request"
        );
        drop(context);
        std::fs::remove_dir_all(root).unwrap();
    }
}
