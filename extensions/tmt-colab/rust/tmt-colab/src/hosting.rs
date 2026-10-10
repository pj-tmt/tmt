//! Exact release-embedded public Hosting bytes, independent of native app overrides and state.
include!(concat!(env!("OUT_DIR"), "/colab_hosting.rs"));

pub fn bundle() -> crate::Result<&'static [u8]> {
    BUNDLE.ok_or_else(|| "This executable has no embedded hosted Colab inventory".into())
}

/// The same canonical manifest bytes will be consumed by the deployment declaration.
pub fn manifest() -> Option<&'static [u8]> {
    MANIFEST
}
