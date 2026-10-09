//! Fixed public extension commands supply release-embedded declaration bytes.
//! No installed-root parsing, artifact path reads, or extension-carried executable.
use crate::{
    canonical, declaration,
    deploy_plan::{self, CloudBackend, Enabled, Supplied, Target},
    limits, rules, wire,
};
use serde_json::Value;
use std::{ffi::OsString, path::PathBuf, sync::atomic::AtomicBool, time::Instant};
use tmt_invoke::{EnvironmentPolicy, LaunchOptions, Request};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscoveryRefusal {
    Unavailable,
    Envelope,
    Declaration,
    Digest,
    Plan(deploy_plan::PlanError),
    Rules(rules::RulesError),
}
/// One bounded reply from a trusted enabled extension's public command.
pub trait DeclarationSource {
    fn declaration(&mut self, extension: &str) -> Result<Option<Vec<u8>>, DiscoveryRefusal>;
}
/// Same public TMT_EXECUTABLE dispatch edge as Colab's door discovery.
pub struct InstalledDeclarations<'a> {
    executable: PathBuf,
    stop: &'a AtomicBool,
}
impl<'a> InstalledDeclarations<'a> {
    pub fn new(stop: &'a AtomicBool) -> Result<Self, DiscoveryRefusal> {
        Ok(Self {
            executable: tmt_invoke::invoking_tmt().map_err(|_| DiscoveryRefusal::Unavailable)?,
            stop,
        })
    }
    pub fn at(executable: PathBuf, stop: &'a AtomicBool) -> Result<Self, DiscoveryRefusal> {
        if !executable.is_absolute() {
            return Err(DiscoveryRefusal::Unavailable);
        }
        Ok(Self { executable, stop })
    }
}
impl DeclarationSource for InstalledDeclarations<'_> {
    fn declaration(&mut self, extension: &str) -> Result<Option<Vec<u8>>, DiscoveryRefusal> {
        if !canonical::extension_name(extension) {
            return Err(DiscoveryRefusal::Envelope);
        }
        let args = [extension, "deploy-declaration", "--json"].map(OsString::from);
        let environment = [
            "HOME",
            "PATH",
            "XDG_CONFIG_HOME",
            "XDG_DATA_HOME",
            "XDG_STATE_HOME",
            "XDG_CACHE_HOME",
            "TMT_HOME",
        ]
        .map(OsString::from);
        let output = tmt_invoke::invoke(
            Request {
                program: &self.executable,
                args: &args,
                input: &[],
                deadline: Instant::now() + limits::DEPLOY_DECLARATION_CALL,
                max_stream_bytes: limits::DEPLOY_DECLARATION_REPLY_BYTES,
                launch: LaunchOptions {
                    current_dir: Some(PathBuf::from("/")),
                    environment: EnvironmentPolicy::ClearAllowlist(&environment),
                    ..Default::default()
                },
            },
            Some(self.stop),
        )
        .map_err(|_| DiscoveryRefusal::Unavailable)?;
        if !output.status.success() {
            // An old installed command does not provide a declaration. All other failures
            // remain unavailable; stderr and raw exception text are never surfaced.
            let reply = wire::strict_json(&output.stdout).ok_or(DiscoveryRefusal::Unavailable)?;
            return match reply["error"]["code"].as_str() {
                Some("USAGE_ERROR" | "EXTENSION_NOT_INSTALLED" | "COLAB_INPUT_INVALID") => Ok(None),
                _ => Err(DiscoveryRefusal::Unavailable),
            };
        }
        Ok(Some(output.stdout))
    }
}
struct Bytes {
    name: String,
    declaration: Vec<u8>,
    artifact: Vec<u8>,
}
/// The exact snapshot composed once, then passed to the existing command/engine.
pub struct DiscoveredPlan {
    pub extensions: deploy_plan::Plan,
    pub artifacts: rules::Composed,
}
pub fn discover(
    source: &mut dyn DeclarationSource,
    enabled: &[&str],
) -> Result<DiscoveredPlan, DiscoveryRefusal> {
    if enabled.len() > limits::PLAN_EXTENSIONS {
        return Err(DiscoveryRefusal::Envelope);
    }
    let mut names = enabled.to_vec();
    names.sort_unstable();
    if names.windows(2).any(|p| p[0] == p[1]) || names.iter().any(|n| !canonical::extension_name(n))
    {
        return Err(DiscoveryRefusal::Envelope);
    }
    let mut supplied = Vec::new();
    for name in &names {
        if let Some(reply) = source.declaration(name)? {
            supplied.push(parse(name, &reply)?);
        }
    }
    let inputs: Vec<_> = names
        .iter()
        .map(|name| Enabled {
            name,
            supplied: supplied.iter().find(|b| b.name == *name).map(|b| Supplied {
                declaration: &b.declaration,
                artifact: &b.artifact,
            }),
        })
        .collect();
    let extensions = deploy_plan::compose(
        Target {
            backend: CloudBackend::Firestore,
            physical_ttl: false,
        },
        &inputs,
    )
    .map_err(DiscoveryRefusal::Plan)?;
    let fragments: Vec<_> = supplied
        .iter()
        .map(|b| rules::Fragment {
            extension: &b.name,
            source: &b.artifact,
        })
        .collect();
    let artifacts = rules::compose(&extensions, &fragments).map_err(DiscoveryRefusal::Rules)?;
    Ok(DiscoveredPlan {
        extensions,
        artifacts,
    })
}
fn parse(name: &str, bytes: &[u8]) -> Result<Bytes, DiscoveryRefusal> {
    if bytes.len() > limits::DEPLOY_DECLARATION_REPLY_BYTES {
        return Err(DiscoveryRefusal::Envelope);
    }
    let value: Value = wire::strict_json(bytes).ok_or(DiscoveryRefusal::Envelope)?;
    let object = value.as_object().ok_or(DiscoveryRefusal::Envelope)?;
    if object.len() != 6
        || value["version"] != 1
        || value["extension"] != name
        || value["backend"] != "firestore"
    {
        return Err(DiscoveryRefusal::Envelope);
    }
    let declaration = value["declaration"]
        .as_str()
        .ok_or(DiscoveryRefusal::Envelope)?
        .as_bytes();
    let artifact = value["artifact"]
        .as_str()
        .ok_or(DiscoveryRefusal::Envelope)?
        .as_bytes();
    if artifact.len() > limits::DECLARATION_ARTIFACT_BYTES {
        return Err(DiscoveryRefusal::Envelope);
    }
    if value["declarationDigest"].as_str() != Some(deploy_plan::sha256_hex(declaration).as_str()) {
        return Err(DiscoveryRefusal::Digest);
    }
    let parsed = declaration::parse(declaration).map_err(|_| DiscoveryRefusal::Declaration)?;
    if parsed.extension != name || parsed.backend != declaration::Backend::Firestore {
        return Err(DiscoveryRefusal::Declaration);
    }
    Ok(Bytes {
        name: name.to_owned(),
        declaration: declaration.to_vec(),
        artifact: artifact.to_vec(),
    })
}
