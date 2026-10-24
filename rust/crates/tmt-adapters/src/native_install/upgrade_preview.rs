//! Read-only upgrade selection and application of the exact consented release.

use super::*;
use crate::native_install as install;

/// Read-only release selection. It does not authorise publication or run a candidate.
pub struct PreparedUpgrade {
    pub(super) current: install::ManagedInstallation,
    pub(super) selected: Option<semver::Version>,
    pub(super) metadata: Option<release::UpgradeMetadata>,
    pub(super) selection: UpgradeSelection,
    pub(super) explicit_channel: bool,
    pub(super) product: install::Product,
    pub(super) allow_schema_ahead: bool,
}

impl PreparedUpgrade {
    pub fn current(&self) -> &install::ManagedInstallation {
        &self.current
    }
    pub fn selected_version(&self) -> Option<&semver::Version> {
        self.selected.as_ref()
    }

    pub fn archive_bytes(&self) -> Option<u64> {
        self.metadata
            .as_ref()
            .map(|metadata| metadata.archive_bytes)
    }

    /// Observe an existing store read-only; unavailable evidence never promises a migration.
    pub fn migration(&self) -> MigrationPreview {
        let Some(schema) = self
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.application_schema.as_ref())
        else {
            return MigrationPreview::Unknown;
        };
        let Ok(local) = install::local_application_schema(self.product) else {
            return MigrationPreview::Unknown;
        };
        migration_status(&schema.databases, &local)
    }

    pub(super) fn lagging_pointer(&self) -> bool {
        self.metadata
            .as_ref()
            .is_some_and(|metadata| self.selected.as_ref() != Some(&metadata.version))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationPreview {
    Required,
    NotRequired,
    Unknown,
}

fn migration_status(
    candidate: &[install::pr_catalog::DatabaseSchema],
    local: &[install::pr_catalog::DatabaseSchema],
) -> MigrationPreview {
    if candidate.is_empty() {
        return MigrationPreview::Unknown;
    }
    let mut required = false;
    for database in candidate {
        let Some(current) = local
            .iter()
            .find(|current| current.domain == database.domain)
        else {
            return MigrationPreview::Unknown;
        };
        if database.version < current.version {
            return MigrationPreview::Unknown;
        }
        required |= database.version > current.version;
    }
    if required {
        MigrationPreview::Required
    } else {
        MigrationPreview::NotRequired
    }
}

pub fn prepare_upgrade(
    product: install::Product,
    request: UpgradeRequest<'_>,
    allow_schema_ahead: bool,
) -> Result<PreparedUpgrade, UpgradeFailure> {
    let current = install::inspect_product(product, request.executable)?;
    let selection = select_upgrade(
        &current.state,
        request.channel,
        request.exact,
        request.unpin,
    )
    .map_err(io::Error::other)?;
    let client = client_for_selection(&selection)?;
    prepare_product_with(
        product,
        request,
        allow_schema_ahead,
        |url, accept, limit, deadline| {
            if matches!(
                selection,
                UpgradeSelection::Fetch {
                    channel: Channel::Pr(_),
                    ..
                }
            ) && url.ends_with("/zip")
            {
                client.get_actions(url, limit, deadline)
            } else {
                client.get(url, accept, limit, deadline)
            }
        },
    )
}

pub(super) fn prepare_product_with(
    product: install::Product,
    request: UpgradeRequest<'_>,
    allow_schema_ahead: bool,
    mut get: impl FnMut(&str, &str, usize, Instant) -> io::Result<crate::release_http::Response>,
) -> Result<PreparedUpgrade, UpgradeFailure> {
    let current = install::inspect_product(product, request.executable)?;
    let selection = select_upgrade(
        &current.state,
        request.channel,
        request.exact,
        request.unpin,
    )
    .map_err(io::Error::other)?;
    let UpgradeSelection::Fetch {
        channel, ref exact, ..
    } = selection
    else {
        return Ok(PreparedUpgrade {
            current,
            selected: None,
            metadata: None,
            selection: UpgradeSelection::Pinned,
            explicit_channel: request.channel.is_some(),
            product,
            allow_schema_ahead,
        });
    };
    let deadline = Instant::now() + Duration::from_secs(60);
    let metadata = if let Channel::Pr(pr) = channel {
        install::pr_resolver::preview_current(
            install::pr_resolver::Request {
                product,
                pr,
                exact: exact.as_ref(),
                target: &current.target,
                opt_in: allow_schema_ahead,
                deadline,
            },
            &mut get,
        )?
    } else {
        release::upgrade_metadata(
            product,
            channel,
            exact.as_ref(),
            &current.target,
            deadline,
            &mut get,
        )?
    };
    // A lagging channel pointer is advisory, as in the acquisition path.
    let selected = if !matches!(channel, Channel::Pr(_))
        && channel == current.state.channel
        && exact.is_none()
        && !request.unpin
        && metadata.version.cmp_precedence(&current.state.version) == std::cmp::Ordering::Less
    {
        current.state.version.clone()
    } else {
        metadata.version.clone()
    };
    if selected == metadata.version {
        let current_identity = current
            .provenance
            .as_ref()
            .map(install::receipt::Provenance::pr_identity)
            .transpose()?
            .flatten();
        let UpgradeSelection::Fetch { pin, .. } = selection else {
            unreachable!()
        };
        plan_candidate_version(
            Some(&current.state),
            &metadata.version,
            channel,
            pin,
            PrVersionContext {
                current: current_identity.as_ref(),
                candidate: metadata.identity.as_ref(),
                explicit_channel: request.channel.is_some(),
            },
        )
        .map_err(io::Error::other)?;
    }
    Ok(PreparedUpgrade {
        current,
        selected: Some(selected),
        metadata: Some(metadata),
        selection,
        explicit_channel: request.channel.is_some(),
        product,
        allow_schema_ahead,
    })
}

/// Apply only the installation and target observed before consent. Publication
/// still belongs to the candidate CLI or the product's existing activation owner.
pub fn upgrade_prepared(
    prepared: &PreparedUpgrade,
    request: UpgradeRequest<'_>,
    verifier: Option<ReleaseVerifier<'_>>,
    checkpoint: impl FnMut() -> io::Result<()>,
    progress: install::ProgressObserver<'_>,
) -> Result<UpgradeReport, UpgradeFailure> {
    let client = client_for_selection(&prepared.selection)?;
    upgrade_product_with_options(
        prepared.product,
        request,
        verifier,
        Acquisition {
            prepared: Some(prepared),
            selected: prepared.selected.as_ref(),
            allow_schema_ahead: prepared.allow_schema_ahead,
            deadline: Instant::now() + Duration::from_secs(60),
            progress,
        },
        checkpoint,
        |url, accept, maximum, deadline| {
            if matches!(
                prepared.selection,
                UpgradeSelection::Fetch {
                    channel: Channel::Pr(_),
                    ..
                }
            ) && url.ends_with("/zip")
            {
                client.get_actions(url, maximum, deadline)
            } else {
                client.get_observed(
                    url,
                    accept,
                    maximum,
                    deadline,
                    &|received_bytes, total_bytes| {
                        if accept == "application/octet-stream" && url.ends_with(".tar.gz") {
                            progress(install::ProgressEvent::Download {
                                received_bytes,
                                total_bytes,
                            });
                        }
                    },
                )
            }
        },
        |current, downloaded, channel, pin, checkpoint| {
            install::handoff::upgrade_observed(
                current,
                downloaded,
                channel,
                pin,
                checkpoint,
                &crate::process::UnixCommandRunner,
                progress,
            )
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_copy_requires_complete_nondowngrade_store_evidence() {
        let database = |version| install::pr_catalog::DatabaseSchema {
            domain: "tmt-core-db".into(),
            version,
        };
        assert_eq!(
            migration_status(&[database(68)], &[database(67)]),
            MigrationPreview::Required
        );
        assert_eq!(
            migration_status(&[database(67)], &[database(67)]),
            MigrationPreview::NotRequired
        );
        assert_eq!(
            migration_status(&[database(66)], &[database(67)]),
            MigrationPreview::Unknown
        );
        assert_eq!(
            migration_status(&[database(67)], &[]),
            MigrationPreview::Unknown
        );
        assert_eq!(
            migration_status(&[], &[database(67)]),
            MigrationPreview::Unknown
        );
    }
}
