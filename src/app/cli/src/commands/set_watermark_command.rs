// Copyright Kamu Data, Inc. and contributors. All rights reserved.
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

use std::sync::Arc;

use chrono::DateTime;
use kamu::domain::*;
use opendatafabric::*;

use super::{CLIError, Command};

pub struct SetWatermarkCommand {
    dataset_repo: Arc<dyn DatasetRepository>,
    remote_alias_reg: Arc<dyn RemoteAliasesRegistry>,
    pull_svc: Arc<dyn PullService>,
    refs: Vec<DatasetRefAnyPattern>,
    all: bool,
    recursive: bool,
    watermark: String,
}

impl SetWatermarkCommand {
    pub fn new<I, S>(
        dataset_repo: Arc<dyn DatasetRepository>,
        remote_alias_reg: Arc<dyn RemoteAliasesRegistry>,
        pull_svc: Arc<dyn PullService>,
        refs: I,
        all: bool,
        recursive: bool,
        watermark: S,
    ) -> Self
    where
        S: Into<String>,
        I: IntoIterator<Item = DatasetRefAnyPattern>,
    {
        Self {
            dataset_repo,
            remote_alias_reg,
            pull_svc,
            refs: refs.into_iter().collect(),
            all,
            recursive,
            watermark: watermark.into(),
        }
    }
}

#[async_trait::async_trait(?Send)]
impl Command for SetWatermarkCommand {
    async fn run(&mut self) -> Result<(), CLIError> {
        if self.refs.len() != 1 {
            return Err(CLIError::usage_error(
                "Only one dataset can be provided when setting a watermark",
            ));
        }
        if self.recursive || self.all {
            return Err(CLIError::usage_error(
                "Can't use --all or --recursive flags when setting a watermark",
            ));
        }
        if self.refs[0].is_pattern() {
            return Err(CLIError::usage_error(
                "Cannot use a pattern when setting a watermark",
            ));
        }

        let watermark = DateTime::parse_from_rfc3339(&self.watermark).map_err(|_| {
            CLIError::usage_error(format!(
                "Invalid timestamp {} should follow RFC3339 format, e.g. 2020-01-01T12:00:00Z",
                self.watermark
            ))
        })?;

        let dataset_ref = self.refs[0]
            .as_dataset_ref_any()
            .unwrap()
            .as_local_ref(|_| !self.dataset_repo.is_multi_tenant())
            .map_err(|_| CLIError::usage_error("Expected a local dataset reference"))?;

        let aliases = self
            .remote_alias_reg
            .get_remote_aliases(&dataset_ref)
            .await
            .map_err(CLIError::failure)?;
        let pull_aliases: Vec<_> = aliases
            .get_by_kind(RemoteAliasKind::Pull)
            .map(ToString::to_string)
            .collect();

        if !pull_aliases.is_empty() {
            // TODO: Should this check be performed at domain model level?
            return Err(CLIError::usage_error(format!(
                "Setting watermark on a remote dataset will cause histories to diverge. Existing \
                 pull aliases:\n{}",
                pull_aliases.join("\n- ")
            )));
        }

        match self
            .pull_svc
            .set_watermark(&dataset_ref, watermark.into())
            .await
        {
            Ok(PullResult::UpToDate(_)) => {
                eprintln!("{}", console::style("Watermark was up-to-date").yellow());
                Ok(())
            }
            Ok(PullResult::Updated { new_head, .. }) => {
                eprintln!(
                    "{}",
                    console::style(format!(
                        "Committed new block {}",
                        new_head.as_multibase().short()
                    ))
                    .green()
                );
                Ok(())
            }
            Err(
                e @ (SetWatermarkError::IsDerivative
                | SetWatermarkError::IsRemote
                | SetWatermarkError::NotFound(_)
                | SetWatermarkError::Access(_)),
            ) => Err(CLIError::failure(e)),
            Err(e @ SetWatermarkError::Internal(_)) => Err(CLIError::critical(e)),
        }
    }
}
