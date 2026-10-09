//! Real private deployment files; existing declaration/envelope reference fixtures.
use std::{fs, path::PathBuf};
use tmt_remote::{state::Layout, store::uuid_v4};
pub const ID: &str = "3f2b8c1e-5d4a-4e7b-9c1d-2a6f8e0b4c11";
pub struct Root(pub PathBuf);
impl Root {
    pub fn new() -> Self {
        let root = std::env::temp_dir().join(format!("tmt-deploy-{}", uuid_v4().unwrap()));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
    pub fn layout(&self) -> Layout {
        Layout::open(&self.0).unwrap()
    }
    pub fn remote(&self) -> PathBuf {
        self.0.join("remote")
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
