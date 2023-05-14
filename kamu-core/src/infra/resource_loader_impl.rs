// Copyright Kamu Data, Inc. and contributors. All rights reserved.
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

use crate::domain::*;
use opendatafabric::serde::yaml::*;
use opendatafabric::*;

use dill::component;
use std::path::Path;
use url::Url;

#[component]
pub struct ResourceLoaderImpl {}

impl ResourceLoaderImpl {
    pub fn new() -> Self {
        Self {}
    }

    pub async fn load_dataset_snapshot_from_http(
        &self,
        url: &Url,
    ) -> Result<DatasetSnapshot, ResourceError> {
        match reqwest::get(url.clone())
            .await
            .int_err()?
            .error_for_status()
        {
            Ok(response) => {
                let bytes = response.bytes().await.int_err()?;
                YamlDatasetSnapshotDeserializer
                    .read_manifest(&bytes)
                    .map_err(|e| ResourceError::serde(e))
            }
            Err(err) if err.status() == Some(reqwest::StatusCode::NOT_FOUND) => {
                Err(ResourceError::not_found(url.as_str().to_owned(), None))
            }
            Err(err) => Err(ResourceError::unreachable(
                url.as_str().to_owned(),
                Some(err.into()),
            )),
        }
    }
}

#[async_trait::async_trait]
impl ResourceLoader for ResourceLoaderImpl {
    async fn load_dataset_snapshot_from_path(
        &self,
        path: &Path,
    ) -> Result<DatasetSnapshot, ResourceError> {
        let buffer = std::fs::read(path).int_err()?;
        let snapshot = YamlDatasetSnapshotDeserializer
            .read_manifest(&buffer)
            .map_err(|e| ResourceError::serde(e))?;
        Ok(snapshot)
    }

    async fn load_dataset_snapshot_from_url(
        &self,
        url: &Url,
    ) -> Result<DatasetSnapshot, ResourceError> {
        match url.scheme() {
            "file" => {
                let path = url.to_file_path().expect("Invalid file URL");
                self.load_dataset_snapshot_from_path(&path).await
            }
            "http" | "https" => self.load_dataset_snapshot_from_http(url).await,
            _ => unimplemented!("Unsupported scheme {}", url.scheme()),
        }
    }

    async fn load_dataset_snapshot_from_ref(
        &self,
        sref: &str,
    ) -> Result<DatasetSnapshot, ResourceError> {
        let path = Path::new(sref);
        if path.exists() {
            self.load_dataset_snapshot_from_path(path).await
        } else if let Ok(url) = Url::parse(sref) {
            self.load_dataset_snapshot_from_url(&url).await
        } else {
            self.load_dataset_snapshot_from_path(path).await
        }
    }
}
