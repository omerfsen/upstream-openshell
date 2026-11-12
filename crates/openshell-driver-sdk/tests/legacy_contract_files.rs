// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Reading and writing the contract documents as files, preferring the
//! protobuf JSON file that sits beside the legacy one.

#![cfg(feature = "legacy-contract")]

use std::path::Path;

use openshell_driver_sdk::jwt::SupervisorAuthBundle;
use openshell_driver_sdk::legacy_contract::{
    BoundaryConfig, ContractDocument, SandboxRuntimeDescriptor, protobuf_json_path, read_document,
};
use serde_json::{Value, json};

const LEGACY: &str =
    include_str!("legacy_contract/examples/BoundaryConfig.runtime-initiated-unix.json");
const PROTOBUF: &str =
    include_str!("legacy_contract/proto_json/BoundaryConfig.runtime-initiated-unix.json");

fn config() -> BoundaryConfig {
    serde_json::from_str(LEGACY).unwrap()
}

#[test]
fn the_new_file_is_named_beside_the_legacy_one() {
    for (legacy, expected) in [
        (
            "/.openshell/channel/sandbox/bootstrap.json",
            "/.openshell/channel/sandbox/bootstrap.pb.json",
        ),
        ("auth.json", "auth.pb.json"),
        ("/run/descriptor", "/run/descriptor.pb.json"),
    ] {
        assert_eq!(protobuf_json_path(Path::new(legacy)), Path::new(expected));
    }
}

#[test]
fn encoding_matches_the_protobuf_json_example() {
    let written: Value = serde_json::from_slice(&config().to_protobuf_json().unwrap()).unwrap();
    assert_eq!(written, serde_json::from_str::<Value>(PROTOBUF).unwrap());
}

#[test]
fn the_legacy_encoding_is_unchanged() {
    let written: Value = serde_json::from_slice(&config().to_legacy_json().unwrap()).unwrap();
    assert_eq!(written, serde_json::from_str::<Value>(LEGACY).unwrap());
}

#[test]
fn decoding_gives_back_the_document() {
    let decoded = BoundaryConfig::from_protobuf_json(PROTOBUF.as_bytes()).unwrap();
    assert_eq!(
        serde_json::to_value(&decoded).unwrap(),
        serde_json::from_str::<Value>(LEGACY).unwrap()
    );
}

#[test]
fn every_document_type_round_trips_through_protobuf_json() {
    let descriptor: SandboxRuntimeDescriptor = serde_json::from_str(include_str!(
        "legacy_contract/examples/SandboxRuntimeDescriptor.runtime-initiated-routed.json"
    ))
    .unwrap();
    let bundle: SupervisorAuthBundle = serde_json::from_str(include_str!(
        "legacy_contract/examples/SupervisorAuthBundle.full.json"
    ))
    .unwrap();
    let again =
        SandboxRuntimeDescriptor::from_protobuf_json(&descriptor.to_protobuf_json().unwrap())
            .unwrap();
    assert_eq!(
        serde_json::to_value(&descriptor).unwrap(),
        serde_json::to_value(&again).unwrap()
    );
    let again =
        SupervisorAuthBundle::from_protobuf_json(&bundle.to_protobuf_json().unwrap()).unwrap();
    assert_eq!(
        serde_json::to_value(&bundle).unwrap(),
        serde_json::to_value(&again).unwrap()
    );
}

#[test]
fn a_reader_prefers_the_new_file() {
    let directory = tempfile::tempdir().unwrap();
    let legacy = directory.path().join("bootstrap.json");
    // The legacy file holds a different boundary so the source is visible.
    let mut other = serde_json::to_value(config()).unwrap();
    other["boundary_id"] = json!("from-the-legacy-file");
    std::fs::write(&legacy, other.to_string()).unwrap();
    std::fs::write(directory.path().join("bootstrap.pb.json"), PROTOBUF).unwrap();
    let read: BoundaryConfig = read_document(&legacy).unwrap();
    assert_eq!(read.boundary_id, "sbx-0001");
}

#[test]
fn a_reader_falls_back_to_the_legacy_file() {
    let directory = tempfile::tempdir().unwrap();
    let legacy = directory.path().join("bootstrap.json");
    std::fs::write(&legacy, LEGACY).unwrap();
    let read: BoundaryConfig = read_document(&legacy).unwrap();
    assert_eq!(read.boundary_id, "sbx-0001");
}

#[test]
fn an_invalid_new_file_is_an_error_not_a_fallback() {
    let directory = tempfile::tempdir().unwrap();
    let legacy = directory.path().join("bootstrap.json");
    std::fs::write(&legacy, LEGACY).unwrap();
    let mut invalid: Value = serde_json::from_str(PROTOBUF).unwrap();
    invalid["unexpected"] = json!(true);
    std::fs::write(
        directory.path().join("bootstrap.pb.json"),
        invalid.to_string(),
    )
    .unwrap();
    assert!(read_document::<BoundaryConfig>(&legacy).is_err());
}

#[test]
fn a_missing_pair_of_files_is_an_error() {
    let directory = tempfile::tempdir().unwrap();
    assert!(read_document::<BoundaryConfig>(&directory.path().join("bootstrap.json")).is_err());
}
