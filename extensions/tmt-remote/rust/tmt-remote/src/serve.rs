//! Binary-private lifecycle composition. Public Remote owners retain all authority.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    io::Write,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};
use tmt_extension_serve::{
    ErrorRecord, Handoff, Handshake, Launch, Signals, StartupError, Timing as StartupTiming, adopt,
    launch,
};
use tmt_remote::{
    approval::Approval,
    control::{self, Control},
    core::CoreClient,
    devices::Devices,
    error::RemoteError,
    http::{Door, Handler},
    message::MessageService,
    mount::{self, Extension, Mounts, ObjectDeclaration},
    object_service::{ActivateError, ObjectReadiness, ObjectService, Origins, ServiceBounds},
    objects::{IoBudget, Quotas, system_clock},
    operations::Operations,
    pages::Pages,
    pairing::{Pairing, Timing},
    routes::Routes,
    session::{self, DoorSessions},
    site::Site,
    state::{Layout, MachineKey},
    store::{Store, uuid_v4},
};

use tmt_remote::limits::{SERVE_RECORD_BYTES as FRAME_BYTES, SERVE_STARTUP as STARTUP};

mod native_factory;

fn startup_error() -> RemoteError {
    RemoteError::new(
        "REMOTE_STARTUP_UNCONFIRMED",
        "Remote startup could not be confirmed.",
    )
    .with_hint("inspect tmt remote status; use tmt remote stop before starting again")
}
fn cancelled() -> RemoteError {
    RemoteError::new(
        "REMOTE_STARTUP_CANCELLED",
        "Remote startup was cancelled before handoff.",
    )
}
fn fence(stop: &AtomicBool) -> Result<(), RemoteError> {
    if stop.load(Ordering::SeqCst) {
        Err(cancelled())
    } else {
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Ready {
    profile: String,
    binding: String,
    state: String,
    address: String,
    machine_id: String,
    window_id: String,
    startup_core_calls: u8,
}
impl Ready {
    fn validate(value: Value) -> Result<Self, RemoteError> {
        let ready: Self = serde_json::from_value(value).map_err(|_| startup_error())?;
        let (origin, symbols) = ready.address.split_once("/r/").ok_or_else(startup_error)?;
        let port = origin
            .strip_prefix("http://127.0.0.1:")
            .ok_or_else(startup_error)?;
        if !port
            .parse::<u16>()
            .is_ok_and(|n| n != 0 && n.to_string() == port)
            || !tmt_remote::canonical::route_prefix(&format!("/r/{symbols}"))
        {
            return Err(startup_error());
        }
        if ready.profile != "local-v1"
            || ready.binding != "loopback-http"
            || ready.state != "ready"
            || ready.startup_core_calls != 2
            || ready.address.len() > 256
            || !ready.address.starts_with("http://127.0.0.1:")
            || !tmt_remote::canonical::is_core_id(&ready.machine_id)
            || !tmt_remote::canonical::is_core_id(&ready.window_id)
        {
            return Err(startup_error());
        }
        Ok(ready)
    }
}

/// The handoff's two outcomes the launcher cannot settle, in Remote's words.
fn handshake() -> Handshake {
    Handshake {
        unconfirmed: to_startup(startup_error()),
        cancelled: to_startup(cancelled()),
        timing: StartupTiming {
            startup: STARTUP,
            cleanup_wait: tmt_remote::limits::STOP_WAIT,
            record_bytes: FRAME_BYTES,
        },
    }
}
fn to_startup(error: RemoteError) -> StartupError {
    let startup = StartupError::new(error.code, error.message);
    match error.hint {
        Some(hint) => startup.with_hint(hint),
        None => startup,
    }
}
fn from_startup(error: StartupError) -> RemoteError {
    let remote = RemoteError::new(&error.code, &error.message);
    match error.hint {
        Some(hint) => remote.with_hint(&hint),
        None => remote,
    }
}

/// Remote's fixed, sanitized codes for the private failure record.
fn record_failure(record: &mut ErrorRecord, error: &RemoteError) {
    let (phase, code, message) = match error.code.as_str() {
        "REMOTE_PORT_BUSY" => ("bind", "REMOTE_PORT_BUSY", "The requested port is busy."),
        "REMOTE_STATE_UNSAFE" => (
            "state",
            "REMOTE_STATE_UNSAFE",
            "Remote state was refused as unsafe.",
        ),
        "REMOTE_CORE_UNCERTAIN" => (
            "core",
            "REMOTE_CORE_UNCERTAIN",
            "Core invocation cleanup is unconfirmed.",
        ),
        "REMOTE_STARTUP_CANCELLED" => (
            "handoff",
            "REMOTE_STARTUP_CANCELLED",
            "Startup ended before handoff.",
        ),
        _ => (
            "serve",
            "REMOTE_SERVE_FAILED",
            "Remote serving failed; inspect status or use foreground mode.",
        ),
    };
    record.failure(phase, code, message);
}

pub(super) fn run(arguments: &clap::ArgMatches) -> Result<(), RemoteError> {
    let port = arguments.get_one::<u16>("port").copied();
    let json_output = arguments.get_flag("json");
    let stop = Arc::new(AtomicBool::new(false));
    let launcher = !arguments.get_flag("worker")
        && !arguments.get_flag("foreground")
        && (!json_output || arguments.get_flag("background"));
    let _signals = Signals::register(&stop, launcher)
        .map_err(|_| RemoteError::new("REMOTE_SIGNAL", "Could not register serve shutdown."))?;
    if arguments.get_flag("worker") {
        let mut handoff = adopt(&handshake(), &stop).map_err(from_startup)?;
        let result = foreground(port, json_output, &stop, Some(&mut handoff));
        if let Err(error) = &result {
            let message =
                if error.message.len() <= 1024 && !error.message.chars().any(char::is_control) {
                    error.message.clone()
                } else {
                    "Remote startup failed.".into()
                };
            let mut failure = to_startup(RemoteError::new(&error.code, &message));
            failure.hint = error.hint.clone();
            handoff.fail(&failure, error.code != "REMOTE_CORE_UNCERTAIN");
        } else {
            handoff.finish();
        }
        return result;
    }
    if arguments.get_flag("foreground") || (json_output && !arguments.get_flag("background")) {
        foreground(port, json_output, &stop, None)
    } else {
        background(port, json_output, &stop)
    }
}

fn publish(value: &Value, json_output: bool, detached: bool) -> Result<(), RemoteError> {
    let mut output = tmt_cli_style::stream::stdout(json_output);
    if json_output {
        writeln!(output, "{value}")?;
    } else {
        let terminal = output.terminal();
        tmt_cli_style::message::warning(
            &mut output,
            terminal,
            "Door ready; pair a device with tmt remote pair",
            None,
        )?;
        writeln!(
            output,
            "{}/",
            value["address"]
                .as_str()
                .expect("ready address")
                .split_once("/r/")
                .expect("native protocol base")
                .0
        )?;
    }
    if detached && !json_output {
        writeln!(
            output,
            "Manage this door: tmt remote status; tmt remote stop"
        )?;
    }
    output.flush()?;
    Ok(())
}

fn background(
    port: Option<u16>,
    json_output: bool,
    stop: &Arc<AtomicBool>,
) -> Result<(), RemoteError> {
    let ready = launch_background(port, stop)?;
    publish(&ready, json_output, true).map_err(|_| {
        RemoteError::new(
            "REMOTE_READY_OUTPUT",
            "Remote readiness output was not completed; startup may have succeeded.",
        )
        .with_hint("inspect tmt remote status; use tmt remote stop before starting again")
    })
}
pub(super) fn launch_background(
    port: Option<u16>,
    stop: &AtomicBool,
) -> Result<Value, RemoteError> {
    let mut args: Vec<std::ffi::OsString> = ["serve", "--foreground", "--worker"]
        .into_iter()
        .map(Into::into)
        .collect();
    if let Some(port) = port {
        args.extend(["--port".into(), port.to_string().into()]);
    }
    let program = std::env::current_exe().map_err(|_| startup_error())?;
    let ready = launch(
        &Launch {
            program: &program,
            args: &args,
            handshake: &handshake(),
        },
        stop,
        |value| {
            let ready = Ready::validate(value).map_err(to_startup)?;
            Ok(serde_json::to_value(ready).expect("ready serialization"))
        },
    )
    .map_err(from_startup)?;
    Ok(ready)
}

/// The replacement executable owns candidate admission, one safe stop and one start.
pub(super) fn restart(json_output: bool) -> Result<(), RemoteError> {
    let value =
        restart_once().unwrap_or_else(|_| json!({"outcome":"unconfirmed","reason":"unknown"}));
    let mut output = tmt_cli_style::stream::stdout(json_output);
    if json_output {
        writeln!(output, "{value}")?;
    } else {
        let terminal = output.terminal();
        match value["outcome"].as_str() {
            Some("restarted") => tmt_cli_style::message::success(
                &mut output,
                terminal,
                "Remote restarted in the background.",
            )?,
            Some("not-running") => {}
            _ => tmt_cli_style::message::warning(
                &mut output,
                terminal,
                "Remote restart was not confirmed",
                Some("inspect tmt remote status before restarting manually"),
            )?,
        }
    }
    Ok(())
}
fn restart_once() -> Result<Value, RemoteError> {
    let stop = AtomicBool::new(false);
    let core = CoreClient::discover()?;
    let root = core.storage_root(&stop)?;
    let Some(layout) = Layout::existing(&root)? else {
        return Ok(json!({"outcome":"not-running","reason":null}));
    };
    let snapshot = match control::restart_snapshot(&layout.directory) {
        Ok(Some(snapshot)) => snapshot,
        Ok(None) => {
            if let Some(lease) = layout.existing_serve_lock()? {
                Store::stopped_port(&lease)?;
            }
            return Ok(json!({"outcome":"not-running","reason":null}));
        }
        Err(error) if error.code == "REMOTE_SERVE_OUTDATED" => {
            return Ok(json!({"outcome":"deferred","reason":"legacy"}));
        }
        Err(error) => return Err(error),
    };
    if !snapshot.background {
        return Ok(json!({"outcome":"deferred","reason":"foreground"}));
    }
    let candidate = (|| {
        core_limits(&core, &stop)?;
        Store::replacement_check(&layout, &snapshot)?;
        if snapshot.registrations != control::registrations(&mount::EXTENSIONS) {
            return Err(startup_error());
        }
        Ok::<_, RemoteError>(())
    })();
    if candidate.is_err() {
        return Ok(json!({"outcome":"unconfirmed","reason":"candidate"}));
    }
    match control::request_restart_stop(&layout.directory, &snapshot.window_id) {
        Ok(false) => return Ok(json!({"outcome":"deferred","reason":"busy"})),
        Ok(true) => {}
        Err(error) => {
            // A lost reply never authorizes a second mutation. Observe once only.
            let _ = control::status(&layout.directory, false);
            return Err(error);
        }
    }
    {
        let _lease = layout.wait_for_release(Instant::now() + tmt_remote::limits::STOP_WAIT)?;
        match std::fs::symlink_metadata(layout.directory.join(control::SOCKET)) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            _ => return Err(startup_error()), // Never stop a possible successor.
        }
    }
    let port = snapshot
        .origin
        .strip_prefix("http://127.0.0.1:")
        .and_then(|port| port.parse::<u16>().ok())
        .ok_or_else(startup_error)?;
    let ready = match launch_background(Some(port), &stop) {
        Ok(ready) => ready,
        Err(_) => return Ok(json!({"outcome":"unconfirmed","reason":"startup"})),
    };
    if ready["address"] != format!("{}{}", snapshot.origin, snapshot.path)
        || ready["windowId"] == snapshot.window_id
    {
        return Err(startup_error());
    }
    Ok(json!({"outcome":"restarted","reason":null}))
}

fn core_limits(core: &CoreClient, stop: &AtomicBool) -> Result<usize, RemoteError> {
    let capabilities = core.capabilities(stop)?;
    if capabilities["version"] != 1
        || capabilities["limits"]["outputBytes"]
            .as_u64()
            .is_none_or(|n| n == 0 || n > tmt_remote::core::OUTPUT_LIMIT as u64)
    {
        return Err(RemoteError::new(
            "REMOTE_CORE_UNAVAILABLE",
            "Core advertised an unsupported protocol or output bound.",
        ));
    }
    capabilities["limits"]["inputBytes"]
        .as_u64()
        .filter(|n| *n > 0 && *n <= tmt_remote::limits::CORE_INPUT_BYTES as u64)
        .map(|n| n as usize)
        .ok_or_else(|| {
            RemoteError::new(
                "REMOTE_CORE_UNAVAILABLE",
                "Core did not advertise a valid input bound.",
            )
        })
}

/// A failed object candidate never changes the door's readiness or starts a retry.
#[derive(Debug, PartialEq, Eq)]
struct ObjectSetupFailure {
    extension: &'static str,
    error: ActivateError,
}

fn activate_objects(
    objects: &ObjectService<'_>,
    mounts: &Mounts,
    extensions: &'static [Extension],
    stop: &AtomicBool,
) -> Result<Vec<ObjectSetupFailure>, RemoteError> {
    let mut failures = Vec::new();
    for extension in extensions
        .iter()
        .filter(|extension| extension.objects == ObjectDeclaration::Local)
    {
        fence(stop)?;
        let deadline = Instant::now() + tmt_remote::limits::OBJECT_REACTIVATION;
        if let Err(error) = objects.activate_until(mounts, extension.name, deadline) {
            failures.push(ObjectSetupFailure {
                extension: extension.name,
                error,
            });
        }
        fence(stop)?;
    }
    Ok(failures)
}

fn object_warning(detail: &str) {
    let mut output = tmt_cli_style::stream::stderr();
    let terminal = output.terminal();
    // Diagnostic publication must not turn an optional setup failure into door failure.
    let _ = tmt_cli_style::message::warning(&mut output, terminal, detail, None);
}

fn foreground(
    port: Option<u16>,
    json_output: bool,
    stop: &Arc<AtomicBool>,
    handoff: Option<&mut Handoff>,
) -> Result<(), RemoteError> {
    foreground_with(
        CoreClient::discover()?,
        &mount::EXTENSIONS,
        port,
        json_output,
        stop,
        handoff,
    )
}

fn foreground_with(
    core: CoreClient,
    extensions: &'static [Extension],
    port: Option<u16>,
    json_output: bool,
    stop: &Arc<AtomicBool>,
    handoff: Option<&mut Handoff>,
) -> Result<(), RemoteError> {
    let mut declarations =
        tmt_remote::deploy_discovery::InstalledDeclarations::at(core.executable().to_owned(), stop)
            .map_err(|_| {
                RemoteError::new(
                    "REMOTE_STATE_UNAVAILABLE",
                    "Native declarations are unavailable.",
                )
            })?;
    foreground_with_native(
        core,
        extensions,
        port,
        json_output,
        stop,
        handoff,
        native_factory::NativeInputs {
            source: &mut declarations,
            ports: None,
        },
    )
}

fn foreground_with_native(
    core: CoreClient,
    extensions: &'static [Extension],
    port: Option<u16>,
    json_output: bool,
    stop: &Arc<AtomicBool>,
    mut handoff: Option<&mut Handoff>,
    native: native_factory::NativeInputs<'_>,
) -> Result<(), RemoteError> {
    let input_limit = core_limits(&core, stop)?;
    fence(stop)?;
    let root = core.storage_root(stop)?;
    let executable = core.executable().to_owned();
    fence(stop)?;
    let layout = Layout::open(&root)?;
    fence(stop)?;
    let serving = layout.serve_lock()?;
    let mut diagnostic = if handoff.is_some() {
        Some(ErrorRecord::clear(
            layout.file("serve-error.json")?,
            FRAME_BYTES,
        )?)
    } else {
        None
    };
    let result = (|| {
        serving.retain_for_invocations()?;
        fence(stop)?;
        let machine_key = MachineKey::open(&layout)?;
        fence(stop)?;
        let mut store = Store::open(&serving)?;
        // The origins of tunnels to object-declared extensions, shared by mounts and service.
        let origins = Origins::default();
        // Object readiness requires settled accounting, but its failure leaves the door usable.
        // The static registry alone selects object storage; missing listeners remain degraded.
        let objects = ObjectService::open(
            &serving,
            extensions,
            Quotas::contract(),
            system_clock(),
            ServiceBounds::contract(),
            origins.clone(),
            &IoBudget {
                deadline: Instant::now() + STARTUP,
                cancelled: stop,
            },
        );
        let objects = match objects {
            Ok(objects) => objects,
            Err(error) => {
                object_warning(&format!("Object storage unavailable: {}", error.code));
                None
            }
        };
        let cloud_ports = native.ports.clone();
        let mut native = if let Some(objects) = &objects {
            match native_factory::ProjectOwner::prepare(
                objects,
                &layout,
                extensions,
                native.source,
                native.ports,
            ) {
                Ok(owner) => owner,
                Err(error) => {
                    object_warning(&format!("Native sync unavailable: {}", error.code));
                    None
                }
            }
        } else {
            None
        };
        if let Some(notice) = native.as_ref().and_then(|owner| owner.permission_notice) {
            object_warning(notice);
        }
        fence(stop)?;
        let machine = store.machine()?;
        let cloud_keys = Arc::new(tmt_remote::cloud_keys::CloudKeys::new(
            layout.clone(),
            machine.id.clone(),
            machine_key.clone(),
        ));
        if let Err(error) = cloud_keys.maintain((system_clock())()) {
            object_warning(&format!("Cloud encryption unavailable: {}", error.code));
        }
        let requested = port;
        let remembered = store.remembered_port()?;
        let selected = requested.or(remembered).unwrap_or(0);
        fence(stop)?;
        let door = Door::bind(selected).map_err(|error| {
            if requested.is_none() && remembered.is_some() && error.code == "REMOTE_PORT_BUSY" {
                RemoteError::new(
                    "REMOTE_PORT_BUSY",
                    &format!("Remote's port {selected} is in use"),
                )
                .with_hint("stop what is using it to keep this browser paired, or run tmt remote serve --port <n> and pair again")
            } else {
                error
            }
        })?;
        let bound_port = door.socket_addr()?.port();
        let store = Arc::new(Mutex::new(store));
        // Each run is a new window; grants survive it, sessions do not.
        let window_id = uuid_v4()?;
        let pairing = Arc::new(Pairing::new(
            machine.id.clone(),
            window_id.clone(),
            machine_key.public(),
            door.origin.clone(),
            Arc::clone(&store),
            Timing::CONTRACT,
        ));
        let sessions = Arc::new(DoorSessions::new(
            machine.id.clone(),
            window_id.clone(),
            door.origin.clone(),
            format!("{}/{}/", machine.route_prefix, mount::MOUNT_SEGMENT),
            machine_key.clone(),
            Arc::clone(&store),
            session::IDLE,
        ));
        let devices = Arc::new(Devices::new(
            Arc::clone(&store),
            Some(Arc::clone(&sessions)),
        ));
        let firestore: Arc<dyn tmt_remote::readiness::FirestoreEvidenceSource> = Arc::new(
            tmt_remote::deploy_record::DeployRecordEvidence::new(&layout),
        );
        let operations = Arc::new(
            Operations::new(core, Arc::clone(stop), input_limit)
                .with_management(Arc::clone(&devices))
                .with_firestore(Arc::clone(&firestore)),
        );
        let messages = Arc::new(
            MessageService::new(Arc::clone(&sessions), input_limit)
                .with_operations(Arc::clone(&operations)),
        );
        let projects = native
            .as_ref()
            .map(|owner| owner.projects.clone())
            .unwrap_or_else(|| {
                Arc::new(tmt_remote::native_sync::projects::Projects::new(
                    layout.clone(),
                    cloud_ports,
                ))
            });
        let cloud = Arc::new(tmt_remote::cloud_pairing::service::Service::new(
            tmt_remote::cloud_pairing::service::Owner {
                layout: layout.clone(),
                store: store.clone(),
                key: machine_key,
                machine: machine.id.clone(),
                keys: cloud_keys.clone(),
                sessions: sessions.clone(),
                messages: messages.clone(),
                projects: projects.clone(),
                stop: stop.clone(),
            },
        ));
        let cloud_worker = cloud.worker()?;
        let routes = Routes::new(input_limit, machine.route_prefix.clone())?
            .with_pairing(Arc::clone(&pairing))
            .with_sessions(Arc::clone(&sessions))
            .with_messages(messages);
        let address = format!("{}{}", door.origin, routes.prefix());
        let approval = Arc::new(Approval::new(
            Arc::clone(&store),
            Arc::clone(&sessions),
            operations,
        ));
        fence(stop)?;
        approval.cancel_pending()?;
        fence(stop)?;
        let readiness = objects
            .as_ref()
            .map(ObjectService::readiness)
            .unwrap_or_else(|| ObjectReadiness::unavailable(extensions));
        let reactivation = objects
            .as_ref()
            .map(|objects| objects.reactivation(Arc::clone(stop)));
        let control = Control::start_with_views(
            &serving,
            Arc::clone(&pairing),
            Arc::clone(&devices),
            control::Door {
                origin: door.origin.clone(),
                prefix: machine.route_prefix.clone(),
            },
            Some(Arc::clone(&approval)),
            Arc::clone(stop),
            control::StatusViews {
                objects: readiness,
                layers: firestore,
                cloud: Some(cloud.clone()),
                restart: Some(control::RestartView {
                    background: handoff.is_some(),
                    registrations: control::registrations(extensions),
                }),
                native: reactivation.as_ref().map(|hook| {
                    Arc::clone(hook) as Arc<dyn tmt_remote::object_service::NativeActivation>
                }),
            },
        )?;
        let mut mounts = Mounts::with_extensions(
            root,
            &door.origin,
            &machine.route_prefix,
            sessions,
            extensions,
        )
        .with_origins(Arc::new(origins))
        .with_reads(tmt_remote::reads::Reads::new(executable, Arc::clone(stop)));
        if let Some(reactivation) = &reactivation {
            mounts = mounts.with_activation(reactivation.clone());
        }
        let site = Arc::new(
            Site::new(
                routes,
                Arc::new(mounts),
                Some(
                    Pages::new(
                        &door.origin,
                        machine.id.clone(),
                        window_id.clone(),
                        &machine.route_prefix,
                    )
                    .with_pairing(pairing),
                ),
            )
            .with_cloud_keys(cloud_keys),
        );
        fence(stop)?;
        let events = devices.start_events(Arc::clone(&site.mounts))?;
        fence(stop)?;
        if let Some(objects) = &objects {
            for failure in activate_objects(objects, &site.mounts, extensions, stop)? {
                // An extension that is not running is ordinary degraded storage,
                // visible through opt-in status rather than startup noise.
                if !matches!(
                    failure.error,
                    ActivateError::Connect(
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
                    )
                ) {
                    object_warning(
                        "attachments are unavailable: Colab couldn't connect to Remote's file transfer",
                    );
                }
            }
        }
        fence(stop)?;
        store
            .lock()
            .expect("store lock")
            .remember_port(bound_port)?;
        fence(stop)?;
        let ready = json!({"profile":"local-v1","binding":"loopback-http","state":"ready","address":address,"machineId":machine.id,"windowId":window_id,"startupCoreCalls":2});
        if let Some(handoff) = handoff.as_mut() {
            handoff.ready(&ready).map_err(from_startup)?;
        } else {
            publish(&ready, json_output, false)?;
        }
        let result = if let (Some(objects), Some(reactivation)) = (&objects, &reactivation) {
            objects.with_reactivation(reactivation, &site.mounts, || {
                door.run(stop, site.clone() as Arc<dyn Handler>)
            })
        } else {
            door.run(stop, site as Arc<dyn Handler>)
        };
        drop(cloud_worker);
        control.stop();
        let cloud_finish = projects.stop();
        let native_finish = if let Some(native) = native.take() {
            // This fences native work, joins carrier/provider workers, then
            // joins the minute tick and flushes before its writer/Serving end.
            native.finish()
        } else {
            if let Some(objects) = &objects {
                objects.shutdown();
            }
            Ok(())
        };
        drop(native);
        cloud_finish?;
        // Stopping cancels any pending pairing before state is released.
        approval.cancel_pending()?;
        drop(events);
        drop(objects);
        native_finish.and(result)
    })();
    if let (Some(diagnostic), Err(error)) = (&mut diagnostic, &result) {
        record_failure(diagnostic, error);
    }
    // Diagnostic writes/close happen while Serving is still held.
    drop(diagnostic);
    result
}

#[cfg(test)]
#[path = "serve/tests.rs"]
mod object_tests;

#[cfg(test)]
#[path = "serve/native_factory_tests.rs"]
mod native_factory_tests;

#[cfg(test)]
#[path = "serve/native_serve_tests.rs"]
mod native_serve_tests;

#[cfg(test)]
#[path = "serve/test_isolation.rs"]
mod test_isolation;
