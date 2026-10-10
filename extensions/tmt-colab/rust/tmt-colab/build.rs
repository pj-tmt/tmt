#[path = "src/app_inventory.rs"]
mod app_inventory;
#[path = "build/assets.rs"]
mod assets;
#[path = "src/browser_policy.rs"]
mod browser_policy;
#[path = "build/hosting.rs"]
mod hosting;

fn main() {
    println!("cargo:rerun-if-env-changed=TMT_COLAB_APP_DIR");
    println!("cargo:rerun-if-env-changed=TMT_COLAB_HOSTING_DIR");
    let output =
        std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("Cargo supplies OUT_DIR"));
    let directory = std::env::var_os("TMT_COLAB_APP_DIR").map(std::path::PathBuf::from);
    let inputs = assets::generate(directory.as_deref(), &output)
        .unwrap_or_else(|error| panic!("Colab app build is unavailable: {error}"));
    for input in inputs {
        println!("cargo:rerun-if-changed={}", input.display());
    }
    let hosted = std::env::var_os("TMT_COLAB_HOSTING_DIR").map(std::path::PathBuf::from);
    let sdk = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tmt-remote/rust/tmt-remote/assets/remote-v1.js");
    let inputs = hosting::generate(hosted.as_deref(), &sdk, &output)
        .unwrap_or_else(|error| panic!("Colab hosted build is unavailable: {error}"));
    for input in inputs {
        println!("cargo:rerun-if-changed={}", input.display());
    }
}
