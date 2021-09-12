// Copyright Kamu Data, Inc. and contributors. All rights reserved.
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

use opendatafabric::{DatasetRef, DatasetRefBuf, Sha3_256};
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use url::Url;

use std::backtrace::Backtrace;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Repository {
    pub url: Url,
}

pub trait RepositoryClient {
    fn read_ref(&self, dataset_id: &DatasetRef) -> Result<Option<Sha3_256>, RepositoryError>;

    fn write(
        &mut self,
        dataset_ref: &DatasetRef,
        expected_head: Option<Sha3_256>,
        new_head: Sha3_256,
        blocks: &mut dyn Iterator<Item = (Sha3_256, Vec<u8>)>,
        data_files: &mut dyn Iterator<Item = &Path>,
        checkpoint_dir: &Path,
    ) -> Result<(), RepositoryError>;

    fn read(
        &self,
        dataset_ref: &DatasetRef,
        expected_head: Sha3_256,
        last_seen_block: Option<Sha3_256>,
        tmp_dir: &Path,
    ) -> Result<RepositoryReadResult, RepositoryError>;

    /// Deletes a dataset from the repository.
    ///
    /// Note: Some repos may not permit this operation.
    fn delete(&self, dataset_ref: &DatasetRef) -> Result<(), RepositoryError>;

    fn search(&self, query: Option<&str>) -> Result<RepositorySearchResult, RepositoryError>;
}

pub struct RepositoryReadResult {
    pub blocks: Vec<Vec<u8>>,
    pub data_files: Vec<PathBuf>,
    pub checkpoint_dir: PathBuf,
}

pub struct RepositorySearchResult {
    pub datasets: Vec<DatasetRefBuf>,
}

type BoxedError = Box<dyn std::error::Error + Send + Sync>;

#[derive(Debug, Error)]
pub enum RepositoryError {
    #[error("Dataset does not exist")]
    DoesNotExist,
    #[error("Operation is not allowed")]
    NotAllowed,
    #[error("Local dataset ({local_head}) and remote ({remote_head}) have diverged")]
    Diverged {
        local_head: Sha3_256,
        remote_head: Sha3_256,
        //uncommon_blocks_in_local: usize,
        //uncommon_blocks_in_remote: usize,
    },
    #[error("Dataset was updated concurrently")]
    UpdatedConcurrently,
    #[error("Repository appears to have corrupted data: {message}")]
    Corrupted {
        message: String,
        #[source]
        source: Option<BoxedError>,
    },
    #[error("IO error: {source}")]
    IOError {
        #[from]
        source: std::io::Error,
        #[backtrace]
        backtrace: Backtrace,
    },
    #[error("Credentials error: {source}")]
    CredentialsError {
        #[source]
        source: BoxedError,
        #[backtrace]
        backtrace: Backtrace,
    },
    #[error("Protocol error: {source}")]
    ProtocolError {
        #[source]
        source: BoxedError,
        #[backtrace]
        backtrace: Backtrace,
    },
}

impl RepositoryError {
    pub fn credentials(e: BoxedError) -> Self {
        Self::CredentialsError {
            source: e,
            backtrace: Backtrace::capture(),
        }
    }

    pub fn protocol(e: BoxedError) -> Self {
        Self::ProtocolError {
            source: e,
            backtrace: Backtrace::capture(),
        }
    }

    pub fn corrupted(message: String) -> Self {
        Self::Corrupted {
            message: message,
            source: None,
        }
    }

    pub fn corrupted_from<E: std::error::Error + Send + Sync + 'static>(
        message: String,
        source: E,
    ) -> Self {
        Self::Corrupted {
            message: message,
            source: Some(source.into()),
        }
    }
}
