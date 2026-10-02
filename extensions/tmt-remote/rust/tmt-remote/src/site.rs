//! The door's handler: `/x/<extension>/` mounts, everything else the remote binding.
use crate::{
    http::{Handler, Head, Reply, Request},
    mount::Mounts,
    routes::Routes,
};
use std::net::TcpStream;

pub struct Site {
    pub routes: Routes,
    pub mounts: Mounts,
}
fn mounted(path: &str) -> bool {
    path.starts_with("/x/")
}
impl Handler for Site {
    fn admit(&self, head: &Head<'_>) -> Result<usize, Reply> {
        if mounted(head.path) {
            self.mounts.admit(head)
        } else {
            self.routes.admit(head)
        }
    }
    fn handle(&self, request: Request, client: &mut TcpStream) -> Option<Reply> {
        if mounted(&request.path) {
            self.mounts.handle(request, client)
        } else {
            self.routes.handle(request, client)
        }
    }
}
