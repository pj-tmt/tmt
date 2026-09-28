//! Isolated state and loopback call ownership for local resource scenarios.

use super::{
    Request,
    tests::{call_api, test_receipt},
};
use tmt_adapters::config::ConfigPaths;

pub(super) struct HttpFixture {
    pub(super) paths: ConfigPaths,
    pub(super) root: std::path::PathBuf,
}
impl HttpFixture {
    pub(super) fn new() -> Self {
        let root = std::env::temp_dir().join(format!("tmt-resource-http-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        Self {
            paths: ConfigPaths::resolve(&root, &root, Some(&root), None),
            root,
        }
    }
    pub(super) fn call(&self, request: Request) -> String {
        call_api(request, &self.paths, &test_receipt())
    }
}
impl Drop for HttpFixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}
