//! The door's handler: `<prefix>/x/<extension>/` mounts, root browser assets
//! and public extension entries, and otherwise the remote binding, whose routes are
//! disjoint from the mount space.
use crate::{
    http::{Handler, Head, Reply, Request},
    mount::Mounts,
    pages::Pages,
    routes::Routes,
};
use std::{net::TcpStream, sync::Arc};

pub struct Site {
    pub routes: Routes,
    pub mounts: Arc<Mounts>,
    /// Present while serve can pair browsers; without it those paths are 404.
    pub pages: Option<Pages>,
    cloud_keys: Option<Arc<crate::cloud_keys::CloudKeys>>,
}
impl Site {
    pub fn new(routes: Routes, mounts: Arc<Mounts>, pages: Option<Pages>) -> Self {
        Self {
            routes,
            mounts,
            pages,
            cloud_keys: None,
        }
    }
    pub fn with_cloud_keys(mut self, keys: Arc<crate::cloud_keys::CloudKeys>) -> Self {
        self.cloud_keys = Some(keys);
        self
    }
}
impl Handler for Site {
    fn admit(&self, head: &Head<'_>) -> Result<usize, Reply> {
        if self.mounts.serves(head.path) {
            self.mounts.admit(head)
        } else if Pages::serves(head.path) {
            self.pages
                .as_ref()
                .map_or(Err(Reply::empty(404)), |pages| pages.admit(head))
        } else {
            self.routes.admit(head)
        }
    }
    fn handle(&self, request: Request, client: &mut TcpStream) -> Option<Reply> {
        if self.mounts.serves(&request.path) {
            self.mounts.handle(request, client)
        } else if Mounts::short_route(&request.path) {
            let path = request.path.clone();
            self.mounts
                .handle_short(request, client)
                .map(|reply| crate::pages::page_refusal(&path, reply.status))
        } else if Pages::serves(&request.path) {
            Some(self.pages.as_ref().map_or(Reply::empty(404), |pages| {
                pages.handle(&request, &self.mounts)
            }))
        } else {
            self.routes.handle(request, client)
        }
    }
    fn maintain(&self) -> Result<(), crate::error::RemoteError> {
        self.routes.maintain()?;
        if let Some(keys) = &self.cloud_keys {
            // Renewal failure is a cloud-only refusal; it never stops the local door.
            if let Err(error) = keys.maintain((crate::objects::system_clock())()) {
                let mut output = tmt_cli_style::stream::stderr();
                let terminal = output.terminal();
                let _ = tmt_cli_style::message::warning(
                    &mut output,
                    terminal,
                    &format!("Cloud encryption unavailable: {}", error.code),
                    None,
                );
            }
        }
        Ok(())
    }
    fn shutdown(&self) {
        self.routes.shutdown();
        self.mounts.shutdown();
    }
}
