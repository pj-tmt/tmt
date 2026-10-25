//! Shared native installation/upgrade observations mapped to the CLI style component.
use std::time::{Duration, Instant};
use tmt_adapters::native_install::{Product, ProgressEvent};
use tmt_cli_style::{
    Interaction,
    progress::{Progress, State, Step, StepProgress, Transfer},
};

fn product_label(product: Product) -> &'static str {
    if product == Product::Cli {
        "tmt"
    } else {
        product.as_str()
    }
}

pub(crate) struct InstallProgress {
    progress: Progress,
    product: Product,
    action: &'static str,
    enabled: bool,
    started: Instant,
    last_draw: Option<Instant>,
    step: Step,
    output_failed: bool,
    transfer: Option<Transfer>,
}

impl InstallProgress {
    pub(crate) fn new(action: &'static str, product: Product, interaction: Interaction) -> Self {
        Self {
            progress: Progress::new(
                &format!("{action} {}", product_label(product)),
                interaction.stdout && !interaction.dumb,
            ),
            product,
            action,
            enabled: !interaction.json,
            started: Instant::now(),
            last_draw: None,
            step: Step::Download,
            output_failed: false,
            transfer: None,
        }
    }

    pub(crate) fn observe(&mut self, event: ProgressEvent<'_>) {
        let (step, state, transfer, detail) = match event {
            ProgressEvent::Selected {
                version,
                archive_bytes,
            } => {
                self.progress.title(&format!(
                    "{} {} {version}",
                    self.action,
                    product_label(self.product)
                ));
                self.started = Instant::now();
                (
                    Step::Download,
                    State::Running,
                    Some(Transfer {
                        received_bytes: 0,
                        total_bytes: Some(archive_bytes),
                        elapsed: Duration::ZERO,
                    }),
                    String::new(),
                )
            }
            ProgressEvent::Download {
                received_bytes,
                total_bytes,
            } => (
                Step::Download,
                State::Running,
                Some(Transfer {
                    received_bytes,
                    total_bytes,
                    elapsed: self.started.elapsed(),
                }),
                String::new(),
            ),
            ProgressEvent::Downloaded => (
                Step::Download,
                State::Done,
                self.transfer,
                "archive downloaded".into(),
            ),
            ProgressEvent::Verifying => (
                Step::Verify,
                State::Running,
                None,
                "checking release record and archive".into(),
            ),
            ProgressEvent::Verified => (
                Step::Verify,
                State::Done,
                None,
                "checksum matches the release record; archive checked".into(),
            ),
            ProgressEvent::Activating => (
                Step::Activate,
                State::Running,
                None,
                "publishing verified release".into(),
            ),
            ProgressEvent::Activated => (
                Step::Activate,
                State::Running,
                None,
                "settling bundled agent skills".into(),
            ),
        };
        self.step = step;
        if step == Step::Download {
            self.transfer = transfer;
        }
        self.progress.update(StepProgress {
            step,
            state,
            transfer,
            detail: &detail,
        });
        let throttled = matches!(event, ProgressEvent::Download { .. })
            && self
                .last_draw
                .is_some_and(|last| last.elapsed() < Duration::from_millis(100));
        if !throttled {
            self.draw();
        }
    }

    pub(crate) fn settled(&mut self, warning: bool) {
        self.progress.update(StepProgress {
            step: Step::Activate,
            state: if warning { State::Warning } else { State::Done },
            transfer: None,
            detail: if warning {
                "installed; skills need attention"
            } else {
                "release and bundled skills ready"
            },
        });
        self.draw();
    }

    pub(crate) fn failed(&mut self) {
        self.progress.update(StepProgress {
            step: self.step,
            state: State::Failed,
            transfer: None,
            detail: "stopped; inspect the reported installation outcome",
        });
        self.draw();
    }

    fn draw(&mut self) {
        if self.output_failed || !self.enabled {
            return;
        }
        let mut output = tmt_cli_style::stream::stdout(false);
        let terminal = output.terminal();
        self.output_failed = self.progress.write(&mut output, terminal).is_err();
        self.last_draw = Some(Instant::now());
    }
}

/// Private new-CLI observations. The final stdout document remains the legacy protocol.
pub(crate) mod child {
    use super::*;
    use serde_json::{Value, json};
    use std::io::Write;

    pub(crate) const CAPABILITY: &[u8] = b"{\"upgradeProgress\":1}\n";
    pub(crate) const LIMIT: usize = 1024 * 1024;
    const LINE_LIMIT: usize = 1024;

    #[derive(Debug)]
    pub(crate) enum UpgradeProgress {
        Selected {
            version: String,
            archive_bytes: u64,
        },
        Download {
            received_bytes: u64,
            total_bytes: Option<u64>,
        },
        Downloaded,
        Verifying,
        Verified,
        Activating,
        Activated,
        Settled {
            warning: bool,
        },
        Failed,
    }
    impl UpgradeProgress {
        fn from_observation(event: ProgressEvent<'_>) -> Self {
            match event {
                ProgressEvent::Selected {
                    version,
                    archive_bytes,
                } => Self::Selected {
                    version: version.into(),
                    archive_bytes,
                },
                ProgressEvent::Download {
                    received_bytes,
                    total_bytes,
                } => Self::Download {
                    received_bytes,
                    total_bytes,
                },
                ProgressEvent::Downloaded => Self::Downloaded,
                ProgressEvent::Verifying => Self::Verifying,
                ProgressEvent::Verified => Self::Verified,
                ProgressEvent::Activating => Self::Activating,
                ProgressEvent::Activated => Self::Activated,
            }
        }
        pub(crate) fn present(&self, progress: &mut InstallProgress) {
            progress.observe(match self {
                Self::Selected {
                    version,
                    archive_bytes,
                } => ProgressEvent::Selected {
                    version,
                    archive_bytes: *archive_bytes,
                },
                Self::Download {
                    received_bytes,
                    total_bytes,
                } => ProgressEvent::Download {
                    received_bytes: *received_bytes,
                    total_bytes: *total_bytes,
                },
                Self::Downloaded => ProgressEvent::Downloaded,
                Self::Verifying => ProgressEvent::Verifying,
                Self::Verified => ProgressEvent::Verified,
                Self::Activating => ProgressEvent::Activating,
                Self::Activated => ProgressEvent::Activated,
                Self::Settled { warning } => return progress.settled(*warning),
                Self::Failed => return progress.failed(),
            });
        }
    }

    fn event_document(event: UpgradeProgress) -> Value {
        match event {
            UpgradeProgress::Selected {
                version,
                archive_bytes,
            } => json!({"kind":"selected","version":version,"archive_bytes":archive_bytes}),
            UpgradeProgress::Download {
                received_bytes,
                total_bytes,
            } => {
                json!({"kind":"download","received_bytes":received_bytes,"total_bytes":total_bytes})
            }
            UpgradeProgress::Settled { warning } => json!({"kind":"settled","warning":warning}),
            UpgradeProgress::Downloaded => json!({"kind":"downloaded"}),
            UpgradeProgress::Verifying => json!({"kind":"verifying"}),
            UpgradeProgress::Verified => json!({"kind":"verified"}),
            UpgradeProgress::Activating => json!({"kind":"activating"}),
            UpgradeProgress::Activated => json!({"kind":"activated"}),
            UpgradeProgress::Failed => json!({"kind":"failed"}),
        }
    }

    fn parse(bytes: &[u8]) -> Option<(Product, UpgradeProgress)> {
        let record: Value = serde_json::from_slice(bytes).ok()?;
        if record.as_object()?.len() != 3 || record["version"].as_u64() != Some(1) {
            return None;
        }
        let product = Product::ALL.into_iter().find(|p| {
            *p != Product::Cli && p.as_str() == record["product"].as_str().unwrap_or("")
        })?;
        let event = &record["event"];
        let fields = event.as_object()?.len();
        let parsed = match event["kind"].as_str()? {
            "selected" if fields == 3 => {
                let version = event["version"].as_str()?;
                if version.is_empty() || version.len() > 128 {
                    return None;
                }
                UpgradeProgress::Selected {
                    version: version.into(),
                    archive_bytes: event["archive_bytes"].as_u64()?,
                }
            }
            "download" if fields == 3 => UpgradeProgress::Download {
                received_bytes: event["received_bytes"].as_u64()?,
                total_bytes: if event["total_bytes"].is_null() && event.get("total_bytes").is_some()
                {
                    None
                } else {
                    Some(event["total_bytes"].as_u64()?)
                },
            },
            "settled" if fields == 2 => UpgradeProgress::Settled {
                warning: event["warning"].as_bool()?,
            },
            "downloaded" if fields == 1 => UpgradeProgress::Downloaded,
            "verifying" if fields == 1 => UpgradeProgress::Verifying,
            "verified" if fields == 1 => UpgradeProgress::Verified,
            "activating" if fields == 1 => UpgradeProgress::Activating,
            "activated" if fields == 1 => UpgradeProgress::Activated,
            "failed" if fields == 1 => UpgradeProgress::Failed,
            _ => return None,
        };
        Some((product, parsed))
    }

    #[derive(Default)]
    pub(crate) struct Sender {
        written: usize,
        disabled: bool,
        last_download: Option<Instant>,
    }
    impl Sender {
        pub(crate) fn observe(&mut self, product: Product, event: ProgressEvent<'_>) {
            if let ProgressEvent::Download {
                received_bytes,
                total_bytes,
            } = event
            {
                if total_bytes != Some(received_bytes)
                    && self
                        .last_download
                        .is_some_and(|last| last.elapsed() < Duration::from_millis(100))
                {
                    return;
                }
                self.last_download = Some(Instant::now());
            }
            self.send(product, UpgradeProgress::from_observation(event));
        }
        pub(crate) fn send(&mut self, product: Product, event: UpgradeProgress) {
            if self.disabled {
                return;
            }
            let record =
                json!({"version":1,"product":product.as_str(),"event":event_document(event)});
            let Ok(mut bytes) = serde_json::to_vec(&record) else {
                self.disabled = true;
                return;
            };
            bytes.push(b'\n');
            if bytes.len() > LINE_LIMIT || self.written + bytes.len() > LIMIT {
                self.disabled = true;
                return;
            }
            self.written += bytes.len();
            let mut stderr = tmt_cli_style::stream::stderr();
            if stderr
                .write_all(&bytes)
                .and_then(|_| stderr.flush())
                .is_err()
            {
                self.disabled = true;
            }
        }
    }

    #[derive(Default)]
    pub(crate) struct Decoder {
        pending: Vec<u8>,
        invalid: bool,
        received: usize,
    }
    impl Decoder {
        pub(crate) fn feed(
            &mut self,
            bytes: &[u8],
            observe: &mut dyn FnMut(Product, UpgradeProgress),
        ) {
            self.received = self.received.saturating_add(bytes.len());
            if self.received > LIMIT {
                self.invalid = true;
            }
            if self.invalid {
                return;
            }
            for &byte in bytes {
                self.pending.push(byte);
                if self.pending.len() > LINE_LIMIT {
                    self.invalid = true;
                    return;
                }
                if byte == b'\n' {
                    let Some((product, event)) = parse(&self.pending) else {
                        self.invalid = true;
                        return;
                    };
                    self.pending.clear();
                    observe(product, event);
                }
            }
        }
        pub(crate) fn complete(&self) -> bool {
            !self.invalid && self.pending.is_empty()
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn many_valid_records_cannot_exceed_the_stream_bound() {
            let bytes = b"{\"version\":1,\"product\":\"ops\",\"event\":{\"kind\":\"verified\"}}\n";
            let mut decoder = Decoder::default();
            for _ in 0..LIMIT / bytes.len() {
                decoder.feed(bytes, &mut |_, _| {});
            }
            assert!(decoder.complete());
            decoder.feed(bytes, &mut |_, _| panic!("over-bound record"));
            assert!(!decoder.complete());
        }
        #[test]
        fn records_arrive_across_arbitrary_chunks_and_bad_records_stay_invalid() {
            let bytes = b"{\"version\":1,\"product\":\"ops\",\"event\":{\"kind\":\"download\",\"received_bytes\":2,\"total_bytes\":4}}\n";
            for width in [1, 3, bytes.len()] {
                let mut decoder = Decoder::default();
                let mut seen = 0;
                for chunk in bytes.chunks(width) {
                    decoder.feed(chunk, &mut |p, e| {
                        assert_eq!(p, Product::Ops);
                        assert!(matches!(
                            e,
                            UpgradeProgress::Download {
                                received_bytes: 2,
                                total_bytes: Some(4)
                            }
                        ));
                        seen += 1;
                    });
                }
                assert_eq!(seen, 1);
                assert!(decoder.complete());
            }
            for bytes in [
                b"bad\n".as_slice(),
                b"{}\n",
                b"{\"version\":2}\n",
                b"partial",
                &vec![b'x'; LINE_LIMIT + 1],
            ] {
                let mut decoder = Decoder::default();
                decoder.feed(bytes, &mut |_, _| panic!("invalid record"));
                assert!(!decoder.complete());
            }
        }
    }
}

#[cfg(test)]
mod presentation_tests {
    use super::*;
    #[test]
    fn observed_install_and_upgrade_share_bounded_plain_frames() {
        for action in ["Installing", "Upgrading"] {
            let mut view = InstallProgress::new(
                action,
                Product::Cli,
                Interaction {
                    json: true,
                    stdin: false,
                    stdout: false,
                    stderr: false,
                    dumb: true,
                },
            );
            // JSON suppresses writes while retaining the exact presentation model for this check.
            for event in [
                ProgressEvent::Selected {
                    version: "1.2.3",
                    archive_bytes: 4,
                },
                ProgressEvent::Download {
                    received_bytes: 4,
                    total_bytes: Some(4),
                },
                ProgressEvent::Downloaded,
                ProgressEvent::Verifying,
                ProgressEvent::Verified,
                ProgressEvent::Activating,
                ProgressEvent::Activated,
            ] {
                view.observe(event);
            }
            view.settled(false);
            for width in [20, 40, 80] {
                let mut bytes = Vec::new();
                view.progress
                    .write(
                        &mut bytes,
                        tmt_cli_style::Terminal {
                            width: Some(width),
                            ..tmt_cli_style::Terminal::PLAIN
                        },
                    )
                    .unwrap();
                let text = String::from_utf8(bytes).unwrap();
                assert!(!text.contains('\x1b'));
                assert_eq!(text.lines().count(), 4);
                assert!(
                    text.lines()
                        .all(|l| l.chars().count() <= usize::from(width)),
                    "{text}"
                );
                if width == 80 {
                    assert!(text.contains("4 B of 4 B"), "{text}");
                    assert!(text.contains("bundled skills ready"));
                    eprintln!("{action} plain frame:\n{text}");
                }
            }
        }
    }
}
