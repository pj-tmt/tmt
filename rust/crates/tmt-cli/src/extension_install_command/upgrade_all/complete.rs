//! Versioned whole-upgrade plan. Legacy installed-only callers keep their original protocol.
use super::super::setup::{self, Installation};
use super::*;
use crate::setup_command::guided::{extensions::choose, upgrade as providers};
use tmt_cli_style::stream::Stream;
use tmt_core::native_install::Channel;

pub(crate) const CAPABILITY: &[u8] = b"{\"upgradePlan\":2}\n";
pub(crate) struct Plan {
    pub(crate) installed: super::Plan,
    pub(crate) missing: Vec<Value>,
    pub(crate) providers: Vec<Value>,
}

impl Plan {
    pub(crate) fn read(prefix: &Path) -> Self {
        Self::read_with(prefix, |product| {
            native_install::latest_release_version(product, Channel::Alpha).map(|v| v.to_string())
        })
    }

    fn read_with(prefix: &Path, mut discover: impl FnMut(Product) -> io::Result<String>) -> Self {
        let installed = super::plan(prefix);
        let mut missing = Vec::new();
        for entry in setup::inventory(prefix) {
            let product = entry.product;
            let retry = format!(
                "tmt extension install {} --yes --prefix {}",
                product.as_str(),
                crate::output::shell_word(&prefix.to_string_lossy())
            );
            let planned=match &entry.installation {
                Installation::Missing => Some(discover(product).map(|version| json!({"product":product.as_str(),"action":"install","version":version,"channel":"alpha","status":"pending","selected":true,"hint":retry}))),
                Installation::Managed(current) if current.state.pinned_version.is_none()
                    && installed.products.iter().any(|row|row["product"]==product.as_str() && row["status"]=="unchanged") => {
                    match entry.plan() {
                        Ok(state) if !state.skills_current => Some(Ok(json!({"product":product.as_str(),"action":"skills","version":state.version,"status":"pending","selected":true,"hint":retry}))),
                        Err(error) => Some(Err(io::Error::other(error.to_string()))),
                        _=>None,
                    }
                }
                _=>None,
            };
            if let Some(planned) = planned {
                missing.push(planned.unwrap_or_else(|error|json!({"product":product.as_str(),"action":"install","status":"failed","selected":false,"message":error.to_string(),"hint":retry})));
            }
        }
        Self {
            installed,
            missing,
            providers: providers::read(prefix),
        }
    }

    pub(crate) fn document(&self) -> Value {
        json!({"version":2,"installed":self.installed.document(),"missing":self.missing,"providers":self.providers})
    }

    pub(crate) fn parse(bytes: &[u8]) -> Option<Self> {
        if bytes.len() > LIMIT {
            return None;
        }
        let v: Value = serde_json::from_slice(bytes).ok()?;
        if v.as_object()?.len() != 4 || v["version"] != 2 {
            return None;
        }
        let installed = super::Plan::parse(&serde_json::to_vec(&v["installed"]).ok()?)?;
        let missing = v["missing"].as_array()?.clone();
        let providers = v["providers"].as_array()?.clone();
        if missing.len() > setup::PRODUCTS.len() || providers.len() > 16 {
            return None;
        }
        let mut seen = Vec::new();
        for row in &missing {
            if row.as_object()?.keys().any(|key| {
                ![
                    "product", "action", "version", "channel", "status", "selected", "hint",
                    "message",
                ]
                .contains(&key.as_str())
            }) {
                return None;
            }
            let name = row["product"].as_str()?;
            super::protocol::product(name)?;
            if seen.contains(&name)
                || !row["selected"].is_boolean()
                || !row["hint"].is_string()
                || !matches!(row["action"].as_str(), Some("install" | "skills"))
            {
                return None;
            }
            seen.push(name);
            match row["status"].as_str()? {
                "pending" => {
                    super::protocol::version(&row["version"])?;
                    if row["action"] == "install" && row["channel"] != "alpha" {
                        return None;
                    }
                }
                "failed" if row["message"].is_string() && row["selected"] == false => {}
                _ => return None,
            }
        }
        let mut seen = Vec::new();
        for row in &providers {
            if row.as_object()?.keys().any(|key| {
                ![
                    "provider", "status", "selected", "skills", "hooks", "hint", "message",
                ]
                .contains(&key.as_str())
            }) {
                return None;
            }
            let name = row["provider"].as_str()?;
            if seen.contains(&name) || !row["selected"].is_boolean() || !row["hint"].is_string() {
                return None;
            }
            seen.push(name);
            match row["status"].as_str()? {
                "pending"
                    if tmt_adapters::drivers::Registry::builtin()
                        .find(name)
                        .is_some()
                        && row["skills"].is_array()
                        && row["hooks"].is_array() => {}
                "needsApproval" | "skipped"
                    if row["selected"] == false && row["message"].is_string() => {}
                _ => return None,
            }
        }
        Some(Self {
            installed,
            missing,
            providers,
        })
    }

    pub(crate) fn pending(&self) -> bool {
        self.missing
            .iter()
            .chain(&self.providers)
            .any(|r| r["status"] == "pending")
    }

    /// --yes never authorizes a new provider scope, including at a human terminal.
    pub(crate) fn defaults(&mut self, extension_consent: bool) {
        for row in &mut self.missing {
            row["selected"] = (extension_consent && row["status"] == "pending").into();
        }
        for row in &mut self.providers {
            row["selected"] = false.into();
        }
    }

    pub(crate) fn select(
        &mut self,
        output: &mut Stream<impl Write>,
        interactive: bool,
    ) -> io::Result<bool> {
        let labels = self
            .missing
            .iter()
            .map(|row| {
                format!(
                    "{}: {} {} (bundled skills included){}",
                    row["product"].as_str().unwrap(),
                    row["action"].as_str().unwrap(),
                    row["version"].as_str().unwrap_or("unknown"),
                    if row["product"] == "colab" {
                        "; needs remote"
                    } else {
                        ""
                    }
                )
            })
            .chain(self.providers.iter().map(|row| {
                format!(
                    "{} integration: {}",
                    row["provider"].as_str().unwrap(),
                    row["hint"].as_str().unwrap()
                )
            }))
            .collect::<Vec<_>>();
        let mut selected = self
            .missing
            .iter()
            .chain(&self.providers)
            .map(|row| row["selected"] == true)
            .collect::<Vec<_>>();
        let remote = self
            .missing
            .iter()
            .position(|row| row["product"] == "remote");
        let colab = self
            .missing
            .iter()
            .position(|row| row["product"] == "colab");
        let eligible = self
            .missing
            .iter()
            .chain(&self.providers)
            .map(|row| row["status"] == "pending")
            .collect::<Vec<_>>();
        let accepted = if !interactive {
            for (label, selected) in labels.iter().zip(&selected) {
                writeln!(output, "  [{}] {label}", if *selected { "x" } else { " " })?;
            }
            true
        } else {
            choose(output, &labels, &mut selected, |index, selected| {
                if !eligible[index] {
                    return Some("This item needs the displayed recovery command.".into());
                }
                selected[index] = !selected[index];
                if let (Some(remote), Some(colab)) = (remote, colab) {
                    if index == remote && !selected[remote] && selected[colab] {
                        selected[colab] = false;
                        return Some("Colab was unchecked because it needs Remote.".into());
                    }
                    if index == colab && selected[colab] && !selected[remote] {
                        if !eligible[remote] {
                            selected[colab] = false;
                            return Some(
                                "Remote needs attention before Colab can be installed.".into(),
                            );
                        }
                        selected[remote] = true;
                        return Some("Remote was checked because Colab needs it.".into());
                    }
                }
                None
            })?
        };
        for (row, selected) in self
            .missing
            .iter_mut()
            .chain(&mut self.providers)
            .zip(selected)
        {
            row["selected"] = selected.into();
        }
        if self.providers.iter().any(|r| r["selected"] == true) {
            let hooks = self
                .providers
                .iter()
                .filter(|r| r["selected"] == true)
                .flat_map(|r| r["hooks"].as_array().into_iter().flatten())
                .collect::<Vec<_>>();
            if hooks.iter().any(|hook| hook["usage"] == true) {
                writeln!(output, "{}", crate::setup_command::USAGE_NOTE)?;
            } else if !hooks.is_empty() {
                writeln!(output, "{}", crate::setup_command::ANSWERS_NOTE)?;
            }
            for row in self.providers.iter().filter(|r| r["selected"] == true) {
                for skill in row["skills"].as_array().into_iter().flatten() {
                    writeln!(
                        output,
                        "  skill: {}",
                        skill["path"].as_str().unwrap_or("unknown")
                    )?;
                }
                for hooks in row["hooks"].as_array().into_iter().flatten() {
                    writeln!(
                        output,
                        "  {}: {}",
                        hooks["path"].as_str().unwrap_or("unknown"),
                        hooks["events"].as_str().unwrap_or("hooks")
                    )?;
                }
            }
        }
        Ok(accepted)
    }

    /// Consent covers scope and exact versions, independent of selection and diagnostic wording.
    pub(crate) fn same_scope(&self, other: &Self) -> bool {
        crate::native_upgrade_command::extension_targets(&self.installed)
            == crate::native_upgrade_command::extension_targets(&other.installed)
            && self.missing.len() == other.missing.len()
            && self.providers.len() == other.providers.len()
            && self.missing.iter().zip(&other.missing).all(|(a, b)| {
                ["product", "action", "version", "status"]
                    .iter()
                    .all(|key| a[key] == b[key])
            })
            && self
                .providers
                .iter()
                .zip(&other.providers)
                .all(|(a, b)| providers::same_scope(a, b))
    }

    pub(crate) fn without_consent(mut self) -> (Vec<Value>, Vec<Value>) {
        let mut products = super::without_consent(self.installed, Ok(false));
        products.retain(|row| {
            !self
                .missing
                .iter()
                .any(|missing| missing["product"] == row["product"])
        });
        for row in &mut self.missing {
            if row["status"] == "pending" {
                row["status"] = "consentRequired".into();
            }
        }
        for row in &mut self.providers {
            if row["status"] == "pending" {
                row["status"] = "consentRequired".into();
            }
        }
        products.extend(self.missing);
        (products, self.providers)
    }
}

pub(crate) fn apply(
    prefix: &Path,
    plan: Plan,
    progress: bool,
    checkpoint: &impl Fn() -> bool,
) -> Value {
    apply_with(prefix, plan, progress, checkpoint, setup::apply_at)
}

fn apply_with(
    prefix: &Path,
    plan: Plan,
    progress: bool,
    checkpoint: &impl Fn() -> bool,
    mut install: impl FnMut(
        Product,
        &Path,
        Option<&str>,
        native_install::ProgressObserver<'_>,
    ) -> Result<Value, Failure>,
) -> Value {
    let mut products = plan.installed.products;
    products.retain(|row| {
        !plan
            .missing
            .iter()
            .any(|missing| missing["product"] == row["product"])
    });
    // New scopes are checked and applied before any extension publication can add bundled skills.
    let (providers, mut interrupted) = providers::apply(prefix, &plan.providers, checkpoint);
    for (product, previous, selected) in plan.installed.pending {
        interrupted |= checkpoint();
        if interrupted {
            products.push(skipped(product));
            continue;
        }
        match super::apply_pending(product, prefix, &previous, &selected, progress) {
            Ok(Some((document, _))) => products.push(super::upgraded(product, document)),
            Ok(None) => unreachable!("consented upgrade"),
            Err(error) => {
                interrupted = error.status == 130;
                let mut row = super::failed(product, error);
                row["interrupted"] = interrupted.into();
                products.push(row);
            }
        }
    }
    for mut row in plan.missing {
        let product = super::protocol::product(row["product"].as_str().expect("validated product"))
            .expect("official product");
        if row["status"] != "pending" {
            products.push(row);
            continue;
        }
        if row["selected"] != true {
            row["status"] = "consentRequired".into();
            products.push(row);
            continue;
        }
        interrupted |= checkpoint();
        if interrupted {
            products.push(skipped(product));
            continue;
        }
        if setup::dependencies(product).contains(&Product::Remote) && !remote_ready(prefix) {
            let mut result = skipped(product);
            result["message"] = "Remote is not ready; Colab needs Remote.".into();
            products.push(result);
            continue;
        }
        let expected_missing = row["action"] == "install";
        let state = setup::inventory(prefix)
            .into_iter()
            .find(|entry| entry.product == product)
            .expect("official inventory");
        let unchanged = match &state.installation {
            Installation::Missing => expected_missing,
            Installation::Managed(current) => {
                !expected_missing
                    && current.state.pinned_version.is_none()
                    && current.state.version.to_string() == row["version"].as_str().unwrap_or("")
            }
            _ => false,
        };
        if !unchanged {
            row["status"] = "failed".into();
            row["message"] = "Installation changed after approval; run tmt upgrade again.".into();
            products.push(row);
            continue;
        }
        let sender = std::cell::RefCell::new(crate::native_progress::child::Sender::default());
        let result = install(
            product,
            prefix,
            expected_missing.then(|| row["version"].as_str().unwrap()),
            &|event| {
                if progress {
                    sender.borrow_mut().observe(product, event);
                }
            },
        );
        match result {
            Ok(document) => {
                if progress {
                    sender.borrow_mut().send(
                        product,
                        crate::native_progress::child::UpgradeProgress::Settled {
                            warning: document["skills"]["status"] == "warning",
                        },
                    );
                }
                row["status"] = if document["changed"] == true {
                    "changed"
                } else {
                    "unchanged"
                }
                .into();
                row["details"] = document;
                products.push(row);
            }
            Err(error) => {
                interrupted = error.status == 130;
                if progress {
                    sender.borrow_mut().send(
                        product,
                        crate::native_progress::child::UpgradeProgress::Failed,
                    );
                }
                let mut result = super::failed(product, error);
                result["interrupted"] = interrupted.into();
                products.push(result);
            }
        }
    }
    interrupted |= checkpoint();
    json!({"products":products,"providers":providers,"interrupted":interrupted})
}
fn remote_ready(prefix: &Path) -> bool {
    native_install::inspect_product_prefix(Product::Remote, prefix).is_ok()
}
fn skipped(product: Product) -> Value {
    json!({"product":product.as_str(),"status":"failed","message":"Upgrade interrupted; no later item was changed.","hint":"tmt upgrade"})
}

pub(crate) fn execute(planning: bool, progress: bool, mode: OutputMode) -> io::Result<u8> {
    let result = (|| -> Result<(Value, u8), Failure> {
        let executable =
            std::env::current_exe().map_err(|e| failure("EXTENSION_UPGRADE_FAILED", e))?;
        let current = native_install::inspect(&executable)
            .map_err(|e| failure("EXTENSION_UPGRADE_FAILED", e))?;
        if planning {
            tmt_cli_style::stream::stderr()
                .write_all(CAPABILITY)
                .map_err(|e| failure("EXTENSION_UPGRADE_FAILED", e))?;
            return Ok((Plan::read(current.prefix()).document(), 0));
        }
        let mut bytes = Vec::new();
        io::stdin()
            .take((LIMIT + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|e| failure("EXTENSION_UPGRADE_FAILED", e))?;
        let plan = Plan::parse(&bytes).ok_or_else(|| {
            Failure::new(
                "EXTENSION_UPGRADE_PLAN_INVALID",
                "Invalid whole-upgrade plan; run tmt upgrade again.",
                1,
            )
        })?;
        let interrupt = tmt_adapters::interrupt::Interrupt::install()
            .map_err(|e| failure("EXTENSION_UPGRADE_FAILED", e))?;
        let document = apply(current.prefix(), plan, progress, &|| {
            interrupt.is_interrupted()
        });
        let status = if document["interrupted"] == true {
            130
        } else {
            u8::from(
                document["products"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .chain(document["providers"].as_array().unwrap())
                    .any(|row| row["status"] == "failed"),
            )
        };
        Ok((document, status))
    })();
    match result {
        Ok((document, status)) => {
            writeln!(tmt_cli_style::stream::stdout(mode.json), "{document}")?;
            Ok(status)
        }
        Err(error) => error.publish(mode),
    }
}

#[cfg(test)]
pub(crate) const PRINTED_HINTS: &[crate::cli_style_tests::HintSpec] = &[
    crate::cli_style_tests::HintSpec::core(
        "tmt extension install {} --yes --prefix {}",
        &[""],
        &[("{}", "ops")],
    ),
    crate::cli_style_tests::HintSpec::core("tmt upgrade", &[""], &[]),
    crate::cli_style_tests::HintSpec::core(
        "Installation changed after approval; run tmt upgrade again.",
        &[" again."],
        &[],
    ),
    crate::cli_style_tests::HintSpec::core(
        "Invalid whole-upgrade plan; run tmt upgrade again.",
        &[" again."],
        &[],
    ),
];

#[cfg(test)]
#[path = "complete/tests.rs"]
mod tests;
