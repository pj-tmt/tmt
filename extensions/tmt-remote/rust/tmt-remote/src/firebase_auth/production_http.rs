use super::{AuthError, Cancellation, Clock, Http, Project, Request, Response, UreqHttp};
#[cfg(feature = "test-emulator")]
use super::{AuthErrorKind, emulator_route::EmulatorRoute};
use std::sync::Arc;

pub struct ProductionHttp {
    real: UreqHttp,
    #[cfg(feature = "test-emulator")]
    route: Option<EmulatorRoute>,
}
impl ProductionHttp {
    pub fn new(project: &Project, clock: Arc<dyn Clock + Send + Sync>) -> Result<Self, AuthError> {
        #[cfg(feature = "test-emulator")]
        let route = match (
            std::env::var_os("FIREBASE_AUTH_EMULATOR_HOST"),
            std::env::var_os("FIRESTORE_EMULATOR_HOST"),
        ) {
            (None, None) => None,
            (Some(auth), Some(firestore)) => Some(EmulatorRoute::new(
                &project.project_id,
                auth.to_str()
                    .ok_or_else(|| AuthError::new(AuthErrorKind::InputInvalid))?,
                firestore
                    .to_str()
                    .ok_or_else(|| AuthError::new(AuthErrorKind::InputInvalid))?,
            )?),
            _ => return Err(AuthError::new(AuthErrorKind::InputInvalid)),
        };
        #[cfg(not(feature = "test-emulator"))]
        let _ = project;
        #[cfg(feature = "test-emulator")]
        let real = if route.is_some() {
            UreqHttp::loopback(clock)
        } else {
            UreqHttp::new(clock)
        };
        #[cfg(not(feature = "test-emulator"))]
        let real = UreqHttp::new(clock);
        Ok(Self {
            real,
            #[cfg(feature = "test-emulator")]
            route,
        })
    }
    pub fn emulator_active(&self) -> bool {
        #[cfg(feature = "test-emulator")]
        {
            self.route.is_some()
        }
        #[cfg(not(feature = "test-emulator"))]
        {
            false
        }
    }
}
impl Http for ProductionHttp {
    fn send(
        &self,
        request: &Request,
        deadline: u64,
        cancel: &dyn Cancellation,
    ) -> Result<Response, AuthError> {
        #[cfg(feature = "test-emulator")]
        if let Some(route) = &self.route {
            return route.send(&self.real, request, deadline, cancel);
        }
        self.real.send(request, deadline, cancel)
    }
}
