//! The door's handler: `<prefix>/x/<extension>/` mounts, `/pair/` and `/sdk/`
//! browser assets (including the static root landing), and everything else the
//! remote binding, whose routes are
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
        } else if Pages::serves(&request.path) {
            Some(self.pages.as_ref().map_or(Reply::empty(404), |pages| {
                pages.handle(&request, &self.mounts)
            }))
        } else {
            self.routes.handle(request, client)
        }
    }
    fn shutdown(&self) {
        self.routes.shutdown();
        self.mounts.shutdown();
    }
}
