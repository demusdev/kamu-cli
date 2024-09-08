// Copyright Kamu Data, Inc. and contributors. All rights reserved.
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

use std::sync::Arc;

use chrono::{TimeZone, Utc};
use datafusion::arrow::array::{RecordBatch, StringArray, UInt64Array};
use datafusion::arrow::datatypes::*;
use datafusion::prelude::*;
use kamu::domain::*;
use kamu::*;
use kamu_adapter_http::data::query_types::IdentityConfig;
use kamu_ingest_datafusion::DataWriterDataFusion;
use opendatafabric::*;
use serde_json::json;
use testing::MetadataFactory;

use crate::harness::*;

////////////////////////////////////////////////////////////////////////////////////////////////////////////////////////

struct Harness {
    #[allow(dead_code)]
    run_info_dir: tempfile::TempDir,
    server_harness: ServerSideLocalFsHarness,
    root_url: url::Url,
    dataset_handle: DatasetHandle,
    dataset_url: url::Url,
}

impl Harness {
    async fn new() -> Self {
        // TODO: Need access to these from harness level
        let run_info_dir = tempfile::tempdir().unwrap();

        let identity_config = IdentityConfig {
            private_key: ed25519_dalek::SigningKey::from_bytes(
                &[123; ed25519_dalek::SECRET_KEY_LENGTH],
            )
            .into(),
        };

        let catalog = dill::CatalogBuilder::new()
            .add_value(RunInfoDir::new(run_info_dir.path()))
            .add_value(identity_config)
            .add::<DataFormatRegistryImpl>()
            .add::<QueryServiceImpl>()
            .add::<PushIngestServiceImpl>()
            .add::<EngineProvisionerNull>()
            .build();

        let server_harness = ServerSideLocalFsHarness::new(ServerSideHarnessOptions {
            multi_tenant: true,
            authorized_writes: true,
            base_catalog: Some(catalog),
        });

        let system_time = Utc.with_ymd_and_hms(2050, 1, 1, 12, 0, 0).unwrap();
        server_harness.system_time_source().set(system_time);

        let alias = DatasetAlias::new(
            server_harness.operating_account_name(),
            DatasetName::new_unchecked("population"),
        );
        let create_result = server_harness
            .cli_create_dataset_use_case()
            .execute(
                &alias,
                MetadataFactory::metadata_block(MetadataFactory::seed(DatasetKind::Root).build())
                    .system_time(system_time)
                    .build_typed(),
                Default::default(),
            )
            .await
            .unwrap();

        for event in [
            SetAttachments {
                attachments: Attachments::Embedded(AttachmentsEmbedded {
                    items: vec![AttachmentEmbedded {
                        path: "README.md".to_string(),
                        content: "Blah".to_string(),
                    }],
                }),
            }
            .into(),
            SetInfo {
                description: Some("Test dataset".to_string()),
                keywords: Some(vec!["foo".to_string(), "bar".to_string()]),
            }
            .into(),
            SetLicense {
                short_name: "apache-2.0".to_string(),
                name: "apache-2.0".to_string(),
                spdx_id: None,
                website_url: "https://www.apache.org/licenses/LICENSE-2.0".to_string(),
            }
            .into(),
        ] {
            create_result
                .dataset
                .commit_event(
                    event,
                    CommitOpts {
                        system_time: Some(system_time),
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
        }

        let ctx = SessionContext::new();
        let mut writer = DataWriterDataFusion::builder(create_result.dataset.clone(), ctx.clone())
            .with_metadata_state_scanned(None)
            .await
            .unwrap()
            .build();

        writer
            .write(
                Some(
                    ctx.read_batch(
                        RecordBatch::try_new(
                            Arc::new(Schema::new(vec![
                                Field::new("city", DataType::Utf8, false),
                                Field::new("population", DataType::UInt64, false),
                            ])),
                            vec![
                                Arc::new(StringArray::from(vec!["A", "B"])),
                                Arc::new(UInt64Array::from(vec![100, 200])),
                            ],
                        )
                        .unwrap(),
                    )
                    .unwrap(),
                ),
                WriteDataOpts {
                    system_time,
                    source_event_time: system_time,
                    new_watermark: None,
                    new_source_state: None,
                    data_staging_path: run_info_dir.path().join(".temp-data"),
                },
            )
            .await
            .unwrap();

        let root_url = url::Url::parse(
            format!("http://{}", server_harness.api_server_addr()).trim_end_matches('/'),
        )
        .unwrap();

        let dataset_url =
            server_harness.dataset_url_with_scheme(&create_result.dataset_handle.alias, "http");

        Self {
            run_info_dir,
            server_harness,
            root_url,
            dataset_handle: create_result.dataset_handle,
            dataset_url,
        }
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////////////////////////

#[test_group::group(engine, datafusion)]
#[test_log::test(tokio::test)]
async fn test_data_tail_handler() {
    let harness = Harness::new().await;

    let client = async move {
        let cl = reqwest::Client::new();

        // All points
        let tail_url = format!("{}/tail", harness.dataset_url);
        let res = cl
            .get(&tail_url)
            .query(&[("includeSchema", "false")])
            .send()
            .await
            .unwrap();

        assert_eq!(res.status(), http::StatusCode::OK);
        assert_eq!(
            res.json::<serde_json::Value>().await.unwrap(),
            json!({"data": [{
                "city": "A",
                "event_time": "2050-01-01T12:00:00Z",
                "offset": 0,
                "op": 0,
                "population": 100,
                "system_time": "2050-01-01T12:00:00Z"
            }, {
                "city": "B",
                "event_time": "2050-01-01T12:00:00Z",
                "offset": 1,
                "op": 0,
                "population": 200,
                "system_time": "2050-01-01T12:00:00Z"
            }]})
        );

        // Limit
        let res = cl
            .get(&tail_url)
            .query(&[("limit", "1"), ("includeSchema", "false")])
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();

        assert_eq!(
            res.json::<serde_json::Value>().await.unwrap(),
            json!({"data": [{
                "city": "B",
                "event_time": "2050-01-01T12:00:00Z",
                "offset": 1,
                "op": 0,
                "population": 200,
                "system_time": "2050-01-01T12:00:00Z"
            }]})
        );

        // Skip
        let res = cl
            .get(&tail_url)
            .query(&[("skip", "1"), ("includeSchema", "false")])
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();

        assert_eq!(
            res.json::<serde_json::Value>().await.unwrap(),
            json!({"data": [{
                "city": "A",
                "event_time": "2050-01-01T12:00:00Z",
                "offset": 0,
                "op": 0,
                "population": 100,
                "system_time": "2050-01-01T12:00:00Z"
            }]})
        );
    };

    await_client_server_flow!(harness.server_harness.api_server_run(), client);
}

////////////////////////////////////////////////////////////////////////////////////////////////////////////////////////

#[test_group::group(engine, datafusion)]
#[test_log::test(tokio::test)]
async fn test_data_query_handler_full() {
    let harness = Harness::new().await;

    let client = async move {
        let cl = reqwest::Client::new();

        let head = cl
            .get(format!("{}/refs/head", harness.dataset_url))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .text()
            .await
            .unwrap();

        let query = format!(
            "select offset, city, population from \"{}\" order by offset desc",
            harness.dataset_handle.alias
        );
        let query_url = format!("{}query", harness.root_url);
        let res = cl
            .get(&query_url)
            .query(&[("query", query.as_str())])
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();

        assert_eq!(
            res.json::<serde_json::Value>().await.unwrap(),
            json!({
                "data": [{
                    "city": "B",
                    "offset": 1,
                    "population": 200,
                }, {
                    "city": "A",
                    "offset": 0,
                    "population": 100,
                }],
                "schema": "{\"fields\":[{\"name\":\"offset\",\"data_type\":\"Int64\",\"nullable\":true,\"dict_id\":0,\"dict_is_ordered\":false,\"metadata\":{}},{\"name\":\"city\",\"data_type\":\"Utf8\",\"nullable\":false,\"dict_id\":0,\"dict_is_ordered\":false,\"metadata\":{}},{\"name\":\"population\",\"data_type\":\"UInt64\",\"nullable\":false,\"dict_id\":0,\"dict_is_ordered\":false,\"metadata\":{}}],\"metadata\":{}}",
                "dataHash": "f9680c001200b3483eecc3d5c6b50ee6b8cba11b51c08f89ea1f53d3a334c743199f5fe656e",
                "state": {
                    "inputs": [{
                        "id": "did:odf:fed01df230b49615d175307d580c33d6fda61fc7b9aec91df0f5c1a5ebe3b8cbfee02",
                        "blockHash": head,
                    }]
                }
            })
        );
    };

    await_client_server_flow!(harness.server_harness.api_server_run(), client);
}

////////////////////////////////////////////////////////////////////////////////////////////////////////////////////////

#[test_group::group(engine, datafusion)]
#[test_log::test(tokio::test)]
async fn test_data_query_handler_v2() {
    let harness = Harness::new().await;

    let client = async move {
        let cl = reqwest::Client::new();

        let head = cl
            .get(format!("{}/refs/head", harness.dataset_url))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .text()
            .await
            .unwrap();

        let query = format!(
            "select offset, city, population from \"{}\" order by offset desc",
            harness.dataset_handle.alias
        );

        // 1: Defaults - output only
        let res = cl
            .post(&format!("{}query", harness.root_url))
            .json(&json!({
                "query": query,
                // TODO: Remove after V2 transition
                "queryDialect": "SqlDataFusion",
            }))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();

        pretty_assertions::assert_eq!(
            res.json::<serde_json::Value>().await.unwrap(),
            json!({
                "output": {
                    "data": [
                        {"city": "B", "offset": 1, "population": 200},
                        {"city": "A", "offset": 0, "population": 100},
                    ],
                    "dataFormat": "JsonAos",
                }
            })
        );

        // 2: Input and schema
        let res = cl
            .post(&format!("{}query", harness.root_url))
            .json(&json!({
                "query": query,
                "include": ["input", "schema"],
            }))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();

        let response = res.json::<serde_json::Value>().await.unwrap();
        let ignore_schema = &response["output"]["schema"];

        pretty_assertions::assert_eq!(
            response,
            json!({
                "input": {
                    "include": ["Input", "Schema"],
                    "query": query,
                    "queryDialect": "SqlDataFusion",
                    "dataFormat": "JsonAos",
                    "schemaFormat": "ArrowJson",
                    "skip": 0,
                    "limit": 100,
                    "datasets": [{
                        "alias": "kamu-server/population",
                        "blockHash": head,
                        "id": harness.dataset_handle.id.as_did_str().to_string(),
                    }],
                },
                "output": {
                    "data": [
                        {"city": "B", "offset": 1, "population": 200},
                        {"city": "A", "offset": 0, "population": 100},
                    ],
                    "dataFormat": "JsonAos",
                    "schema": ignore_schema,
                    "schemaFormat": "ArrowJson",
                }
            })
        );

        // 3: Full with proof
        let res = cl
            .post(&format!("{}query", harness.root_url))
            .json(&json!({
                "query": query,
                "include": ["schema", "proof"],
            }))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();

        let response = res.json::<serde_json::Value>().await.unwrap();
        let ignore_schema = &response["output"]["schema"];

        pretty_assertions::assert_eq!(
            response,
            json!({
                "input": {
                    // Note: Proof automatically adds Input
                    "include": ["Input", "Proof", "Schema"],
                    "query": query,
                    "queryDialect": "SqlDataFusion",
                    "dataFormat": "JsonAos",
                    "schemaFormat": "ArrowJson",
                    "skip": 0,
                    "limit": 100,
                    "datasets": [{
                        "alias": "kamu-server/population",
                        "blockHash": head,
                        "id": harness.dataset_handle.id.as_did_str().to_string(),
                    }],
                },
                "output": {
                    "data": [
                        {"city": "B", "offset": 1, "population": 200},
                        {"city": "A", "offset": 0, "population": 100},
                    ],
                    "dataFormat": "JsonAos",
                    "schema": ignore_schema,
                    "schemaFormat": "ArrowJson",
                },
                "subQueries": [],
                "commitment": {
                    "inputHash": "f162001ff67ca8970bcb4f4f8b25e79b3c6db3fcd2ac0501d131e446591fd0475a2af",
                    "outputHash": "f1620fa841fae69710c888fdf82d8fd63948469c0fd1e2a37c16e2067127e2eec1ea8",
                    "subQueriesHash": "f1620ca4510738395af1429224dd785675309c344b2b549632e20275c69b15ed1d210",
                },
                "proof": {
                    "type": "Ed25519Signature2020",
                    "verificationMethod": "did:key:z6Mko2nqhQ9wYSTS5Giab2j1aHzGnxHimqwmFeEVY8aNsVnN",
                    "proofValue": "u-k8Rd9dB5ERqbTU9ymUvpySTQEh8HMPAqcEBrtZviNOBFoe-FXZtJUGcvwud39dxC659bkVz4iYHhDYUexmiCQ",
                }
            })
        );

        // Verify the commitment
        assert_eq!(
            response["commitment"]["inputHash"].as_str().unwrap(),
            Multihash::from_digest_sha3_256(
                canonical_json::to_string(&response["input"])
                    .unwrap()
                    .as_bytes()
            )
            .to_string()
        );
        assert_eq!(
            response["commitment"]["outputHash"].as_str().unwrap(),
            Multihash::from_digest_sha3_256(
                canonical_json::to_string(&response["output"])
                    .unwrap()
                    .as_bytes()
            )
            .to_string()
        );
        assert_eq!(
            response["commitment"]["subQueriesHash"].as_str().unwrap(),
            Multihash::from_digest_sha3_256(
                canonical_json::to_string(&response["subQueries"])
                    .unwrap()
                    .as_bytes()
            )
            .to_string()
        );

        let signature =
            Signature::from_multibase(response["proof"]["proofValue"].as_str().unwrap()).unwrap();

        let did = DidKey::from_did_str(response["proof"]["verificationMethod"].as_str().unwrap())
            .unwrap();

        let commitment = canonical_json::to_string(&response["commitment"]).unwrap();

        did.verify(commitment.as_bytes(), &signature).unwrap();
    };

    await_client_server_flow!(harness.server_harness.api_server_run(), client);
}

////////////////////////////////////////////////////////////////////////////////////////////////////////////////////////

#[test_group::group(engine, datafusion)]
#[test_log::test(tokio::test)]
async fn test_data_query_handler_error_sql_unparsable() {
    let harness = Harness::new().await;

    let client = async move {
        let cl = reqwest::Client::new();

        let query_url = format!("{}query", harness.root_url);
        let res = cl
            .post(&query_url)
            .json(&json!({
                "query": "select ???"
            }))
            .send()
            .await
            .unwrap();

        let status = res.status();
        let body = res.text().await.unwrap();
        assert_eq!(status, 400, "Unexpected response: {status} {body}");
        assert_eq!(
            body,
            "sql parser error: Expected end of statement, found: ?"
        );
    };

    await_client_server_flow!(harness.server_harness.api_server_run(), client);
}

////////////////////////////////////////////////////////////////////////////////////////////////////////////////////////

#[test_group::group(engine, datafusion)]
#[test_log::test(tokio::test)]
async fn test_data_query_handler_error_sql_missing_function() {
    let harness = Harness::new().await;

    let client = async move {
        let cl = reqwest::Client::new();

        let query = format!(
            "select foobar(offset) from \"{}\"",
            harness.dataset_handle.alias
        );

        let query_url = format!("{}query", harness.root_url);
        let res = cl
            .post(&query_url)
            .json(&json!({
                "query": query
            }))
            .send()
            .await
            .unwrap();

        let status = res.status();
        let body = res.text().await.unwrap();
        assert_eq!(status, 400, "Unexpected response: {status} {body}");
        assert_eq!(
            body,
            "Error during planning: Invalid function 'foobar'.\nDid you mean 'floor'?"
        );
    };

    await_client_server_flow!(harness.server_harness.api_server_run(), client);
}

////////////////////////////////////////////////////////////////////////////////////////////////////////////////////////

#[test_group::group(engine, datafusion)]
#[test_log::test(tokio::test)]
async fn test_data_query_handler_dataset_does_not_exist() {
    let harness = Harness::new().await;

    let client = async move {
        let cl = reqwest::Client::new();

        let query_url = format!("{}query", harness.root_url);
        let res = cl
            .post(&query_url)
            .json(&json!({
                "query": "select offset, city, population from does_not_exist"
            }))
            .send()
            .await
            .unwrap();

        let status = res.status();
        let body = res.text().await.unwrap();
        assert_eq!(status, 400, "Unexpected response: {status} {body}");
        assert_eq!(
            body,
            "Error during planning: table 'kamu.kamu.does_not_exist' not found"
        );
    };

    await_client_server_flow!(harness.server_harness.api_server_run(), client);
}

////////////////////////////////////////////////////////////////////////////////////////////////////////////////////////

#[test_group::group(engine, datafusion)]
#[test_log::test(tokio::test)]
async fn test_data_query_handler_dataset_does_not_exist_bad_alias() {
    let harness = Harness::new().await;

    let client = async move {
        let cl = reqwest::Client::new();

        let query = format!(
            "select offset, city, population from \"{}\"",
            harness.dataset_handle.alias
        );

        let query_url = format!("{}query", harness.root_url);
        let res = cl
            .post(&query_url)
            .json(&json!({
                "query": query,
                "aliases": [{
                    "alias": harness.dataset_handle.alias,
                    "id": DatasetID::new_seeded_ed25519(b"does-not-exist"),
                }]
            }))
            .send()
            .await
            .unwrap();

        let status = res.status();
        let body = res.text().await.unwrap();
        assert_eq!(status, 404, "Unexpected response: {status} {body}");
        assert_eq!(
            body,
            "Dataset not found: \
             did:odf:fed011ba79f25e520298ba6945dd6197083a366364bef178d5899b100c434748d88e5"
        );
    };

    await_client_server_flow!(harness.server_harness.api_server_run(), client);
}

////////////////////////////////////////////////////////////////////////////////////////////////////////////////////////

#[test_group::group(engine, datafusion)]
#[test_log::test(tokio::test)]
async fn test_data_query_handler_ranges() {
    let harness = Harness::new().await;

    let client = async move {
        let cl = reqwest::Client::new();

        let query = format!(
            "select offset, city, population from \"{}\" order by offset desc",
            harness.dataset_handle.alias
        );
        let query_url = format!("{}query", harness.root_url);

        // Limit
        let res = cl
            .get(&query_url)
            .query(&[
                ("query", query.as_str()),
                ("limit", "1"),
                ("includeSchema", "false"),
                ("includeState", "false"),
                ("includeDataHash", "false"),
            ])
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();

        assert_eq!(
            res.json::<serde_json::Value>().await.unwrap(),
            json!({"data": [{
                "city": "B",
                "offset": 1,
                "population": 200,
            }]})
        );

        // Skip
        let res = cl
            .get(&query_url)
            .query(&[
                ("query", query.as_str()),
                ("skip", "1"),
                ("includeSchema", "false"),
                ("includeState", "false"),
                ("includeDataHash", "false"),
            ])
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();

        assert_eq!(
            res.json::<serde_json::Value>().await.unwrap(),
            json!({"data": [{
                "city": "A",
                "offset": 0,
                "population": 100,
            }]})
        );
    };

    await_client_server_flow!(harness.server_harness.api_server_run(), client);
}

////////////////////////////////////////////////////////////////////////////////////////////////////////////////////////

#[test_group::group(engine, datafusion)]
#[test_log::test(tokio::test)]
async fn test_data_query_handler_data_formats() {
    let harness = Harness::new().await;

    let client = async move {
        let cl = reqwest::Client::new();

        let query = format!(
            "select offset, city, population from \"{}\" order by offset desc",
            harness.dataset_handle.alias
        );
        let query_url = format!("{}query", harness.root_url);
        let res = cl
            .get(&query_url)
            .query(&[
                ("query", query.as_str()),
                ("dataFormat", "json-aos"),
                ("includeSchema", "false"),
                ("includeState", "false"),
                ("includeDataHash", "false"),
            ])
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();

        assert_eq!(
            res.json::<serde_json::Value>().await.unwrap(),
            json!({"data": [{
                "city": "B",
                "offset": 1,
                "population": 200,
            }, {
                "city": "A",
                "offset": 0,
                "population": 100,
            }]})
        );

        let res = cl
            .get(&query_url)
            .query(&[
                ("query", query.as_str()),
                ("dataFormat", "json-soa"),
                ("includeSchema", "false"),
                ("includeState", "false"),
                ("includeDataHash", "false"),
            ])
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();

        assert_eq!(
            res.json::<serde_json::Value>().await.unwrap(),
            json!({"data": {
                "offset": [1, 0],
                "city": ["B", "A"],
                "population": [200, 100],
            }})
        );

        let res = cl
            .get(&query_url)
            .query(&[
                ("query", query.as_str()),
                ("dataFormat", "json-aoa"),
                ("includeSchema", "false"),
                ("includeState", "false"),
                ("includeDataHash", "false"),
            ])
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();

        assert_eq!(
            res.json::<serde_json::Value>().await.unwrap(),
            json!({"data": [
                [1, "B", 200],
                [0, "A", 100],
            ]})
        );
    };

    await_client_server_flow!(harness.server_harness.api_server_run(), client);
}

////////////////////////////////////////////////////////////////////////////////////////////////////////////////////////

#[test_group::group(engine, datafusion)]
#[test_log::test(tokio::test)]
async fn test_data_query_handler_schema_formats() {
    let harness = Harness::new().await;

    let client = async move {
        let cl = reqwest::Client::new();

        let query = format!(
            "select offset, city, population from \"{}\"",
            harness.dataset_handle.alias
        );
        let query_url = format!("{}query", harness.root_url);
        let res = cl
            .get(&query_url)
            .query(&[
                ("query", query.as_str()),
                ("schemaFormat", "arrow-json"),
                ("includeSchema", "true"),
                ("includeState", "false"),
                ("includeDataHash", "false"),
            ])
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();

        assert_eq!(
            res.json::<serde_json::Value>().await.unwrap()["schema"]
                .as_str()
                .unwrap(),
            r#"{"fields":[{"name":"offset","data_type":"Int64","nullable":true,"dict_id":0,"dict_is_ordered":false,"metadata":{}},{"name":"city","data_type":"Utf8","nullable":false,"dict_id":0,"dict_is_ordered":false,"metadata":{}},{"name":"population","data_type":"UInt64","nullable":false,"dict_id":0,"dict_is_ordered":false,"metadata":{}}],"metadata":{}}"#
        );

        let res = cl
            .get(&query_url)
            .query(&[
                ("query", query.as_str()),
                ("schemaFormat", "ArrowJson"),
                ("includeSchema", "true"),
                ("includeState", "false"),
                ("includeDataHash", "false"),
            ])
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();

        assert_eq!(
            res.json::<serde_json::Value>().await.unwrap()["schema"]
                .as_str()
                .unwrap(),
            r#"{"fields":[{"name":"offset","data_type":"Int64","nullable":true,"dict_id":0,"dict_is_ordered":false,"metadata":{}},{"name":"city","data_type":"Utf8","nullable":false,"dict_id":0,"dict_is_ordered":false,"metadata":{}},{"name":"population","data_type":"UInt64","nullable":false,"dict_id":0,"dict_is_ordered":false,"metadata":{}}],"metadata":{}}"#
        );

        let res = cl
            .get(&query_url)
            .query(&[
                ("query", query.as_str()),
                ("schemaFormat", "parquet"),
                ("includeSchema", "true"),
                ("includeState", "false"),
                ("includeDataHash", "false"),
            ])
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();

        assert_eq!(
            res.json::<serde_json::Value>().await.unwrap()["schema"]
                .as_str()
                .unwrap(),
            indoc::indoc!(
                r#"message arrow_schema {
                  OPTIONAL INT64 offset;
                  REQUIRED BYTE_ARRAY city (STRING);
                  REQUIRED INT64 population (INTEGER(64,false));
                }
                "#
            )
        );

        let res = cl
            .get(&query_url)
            .query(&[
                ("query", query.as_str()),
                ("schemaFormat", "parquet-json"),
                ("includeSchema", "true"),
                ("includeState", "false"),
                ("includeDataHash", "false"),
            ])
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();

        assert_eq!(
            res.json::<serde_json::Value>().await.unwrap()["schema"]
                .as_str()
                .unwrap(),
            r#"{"name": "arrow_schema", "type": "struct", "fields": [{"name": "offset", "repetition": "OPTIONAL", "type": "INT64"}, {"name": "city", "repetition": "REQUIRED", "type": "BYTE_ARRAY", "logicalType": "STRING"}, {"name": "population", "repetition": "REQUIRED", "type": "INT64", "logicalType": "INTEGER(64,false)"}]}"#
        );
    };

    await_client_server_flow!(harness.server_harness.api_server_run(), client);
}

////////////////////////////////////////////////////////////////////////////////////////////////////////////////////////

#[test_group::group(engine, datafusion)]
#[test_log::test(tokio::test)]
async fn test_metadata_handler_aspects() {
    let harness = Harness::new().await;

    let client = async move {
        let cl = reqwest::Client::new();

        let head = cl
            .get(format!("{}/refs/head", harness.dataset_url))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .text()
            .await
            .unwrap();

        // Default (seed only)
        let url = format!("{}/metadata", harness.dataset_url);
        let res = cl
            .get(&url)
            //.query(&[])
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();

        pretty_assertions::assert_eq!(
            res.json::<serde_json::Value>().await.unwrap(),
            json!({
                "output": {
                    "seed": {
                        "datasetId": harness.dataset_handle.id.to_string(),
                        "datasetKind": "Root",
                    }
                }
            })
        );

        // Full
        let url = format!("{}/metadata", harness.dataset_url);
        let res = cl
            .get(&url)
            .query(&[("include", "attachments,info,license,refs,schema,seed,vocab")])
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();

        let res = res.json::<serde_json::Value>().await.unwrap();
        let ignore_schema = &res["output"]["schema"];
        pretty_assertions::assert_eq!(
            res,
            json!({
                "output": {
                    "attachments": {
                        "attachments": {
                            "kind": "Embedded",
                            "items": [{
                                "path": "README.md",
                                "content": "Blah",
                            }],
                        }
                    },
                    "info": {
                        "description": "Test dataset",
                        "keywords": ["foo", "bar"],
                    },
                    "license": {
                        "name": "apache-2.0",
                        "shortName": "apache-2.0",
                        "websiteUrl": "https://www.apache.org/licenses/LICENSE-2.0",
                    },
                    "refs": [{
                        "name": "head",
                        "blockHash": head,
                    }],
                    "schema": ignore_schema,
                    "schemaFormat": "ArrowJson",
                    "seed": {
                        "datasetId": harness.dataset_handle.id.to_string(),
                        "datasetKind": "Root",
                    },
                    "vocab": {
                        "eventTimeColumn": "event_time",
                        "offsetColumn": "offset",
                        "operationTypeColumn": "op",
                        "systemTimeColumn": "system_time",
                    }
                }
            })
        );
    };

    await_client_server_flow!(harness.server_harness.api_server_run(), client);
}

////////////////////////////////////////////////////////////////////////////////////////////////////////////////////////

#[test_group::group(engine, datafusion)]
#[test_log::test(tokio::test)]
async fn test_metadata_handler_schema_formats() {
    let harness = Harness::new().await;

    let client = async move {
        let cl = reqwest::Client::new();

        let query_url = format!("{}/metadata", harness.dataset_url);
        let res = cl
            .get(&query_url)
            .query(&[("include", "schema"), ("schemaFormat", "ArrowJson")])
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();

        pretty_assertions::assert_eq!(
            res.json::<serde_json::Value>().await.unwrap(),
            json!({
                "output": {
                    "schemaFormat": "ArrowJson",
                    "schema": {
                        "fields": [
                            {
                                "data_type": "Int64",
                                "dict_id": 0,
                                "dict_is_ordered": false,
                                "metadata": {},
                                "name": "offset",
                                "nullable": true
                            },
                            {
                                "data_type": "Int32",
                                "dict_id": 0,
                                "dict_is_ordered": false,
                                "metadata": {},
                                "name": "op",
                                "nullable": false
                            },
                            {
                                "data_type": {
                                    "Timestamp": [
                                        "Millisecond",
                                        "UTC"
                                    ]
                                },
                                "dict_id": 0,
                                "dict_is_ordered": false,
                                "metadata": {},
                                "name": "system_time",
                                "nullable": false
                            },
                            {
                                "data_type": {
                                    "Timestamp": [
                                        "Millisecond",
                                        "UTC"
                                    ]
                                },
                                "dict_id": 0,
                                "dict_is_ordered": false,
                                "metadata": {},
                                "name": "event_time",
                                "nullable": true
                            },
                            {
                                "data_type": "Utf8",
                                "dict_id": 0,
                                "dict_is_ordered": false,
                                "metadata": {},
                                "name": "city",
                                "nullable": false
                            },
                            {
                                "data_type": "UInt64",
                                "dict_id": 0,
                                "dict_is_ordered": false,
                                "metadata": {},
                                "name": "population",
                                "nullable": false
                            }
                        ],
                        "metadata": {}
                    },
                }
            })
        );

        let query_url = format!("{}/metadata", harness.dataset_url);
        let res = cl
            .get(&query_url)
            .query(&[("include", "schema"), ("schemaFormat", "ParquetJson")])
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();

        pretty_assertions::assert_eq!(
            res.json::<serde_json::Value>().await.unwrap(),
            json!({
                "output": {
                    "schemaFormat": "ParquetJson",
                    "schema": {
                        "name": "arrow_schema",
                        "type": "struct",
                        "fields": [
                            {
                                "name": "offset",
                                "repetition": "OPTIONAL",
                                "type": "INT64"
                            },
                            {
                                "name": "op",
                                "repetition": "REQUIRED",
                                "type": "INT32"
                            },
                            {
                                "logicalType": "TIMESTAMP(MILLIS,true)",
                                "name": "system_time",
                                "repetition": "REQUIRED",
                                "type": "INT64"
                            },
                            {
                                "logicalType": "TIMESTAMP(MILLIS,true)",
                                "name": "event_time",
                                "repetition": "OPTIONAL",
                                "type": "INT64"
                            },
                            {
                                "logicalType": "STRING",
                                "name": "city",
                                "repetition": "REQUIRED",
                                "type": "BYTE_ARRAY"
                            },
                            {
                                "logicalType": "INTEGER(64,false)",
                                "name": "population",
                                "repetition": "REQUIRED",
                                "type": "INT64"
                            }
                        ],
                    },
                }
            })
        );

        let query_url = format!("{}/metadata", harness.dataset_url);
        let res = cl
            .get(&query_url)
            .query(&[("include", "schema"), ("schemaFormat", "Parquet")])
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();

        pretty_assertions::assert_eq!(
            res.json::<serde_json::Value>().await.unwrap(),
            json!({
                "output": {
                    "schemaFormat": "Parquet",
                    "schema": "message arrow_schema {\n  OPTIONAL INT64 offset;\n  REQUIRED INT32 op;\n  REQUIRED INT64 system_time (TIMESTAMP(MILLIS,true));\n  OPTIONAL INT64 event_time (TIMESTAMP(MILLIS,true));\n  REQUIRED BYTE_ARRAY city (STRING);\n  REQUIRED INT64 population (INTEGER(64,false));\n}\n",
                }
            })
        );
    };

    await_client_server_flow!(harness.server_harness.api_server_run(), client);
}

////////////////////////////////////////////////////////////////////////////////////////////////////////////////////////
