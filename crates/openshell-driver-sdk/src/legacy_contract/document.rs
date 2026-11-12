// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! The contract documents as protobuf JSON files.
//!
//! A driver that writes the new format writes it beside the legacy file, in a
//! file named like the legacy one with `.pb.json` in place of `.json`
//! (`bootstrap.json` and `bootstrap.pb.json`). A reader looks for the new file
//! first and falls back to the legacy one, so an old driver, which writes only
//! the legacy file, keeps working, and an old reader, which looks only for the
//! legacy file, ignores the new one.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use prost_reflect::{DescriptorPool, DeserializeOptions, DynamicMessage};
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::boundary::{BoundaryConfig, SandboxRuntimeDescriptor};
use super::convert::ContractConversionError;
use crate::jwt::{SandboxLaunchAuthentication, SupervisorAuthBundle};
use crate::proto::compute::v1 as compute;
use crate::proto::sandbox_runtime::v1 as pb;

/// Why a document could not be read or decoded. The text never quotes field
/// values, which include credentials, except where the underlying JSON decoder
/// does.
#[derive(Debug, thiserror::Error)]
pub enum ContractDocumentError {
    #[error("read {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("decode {path}: {message}")]
    Decode { path: PathBuf, message: String },
    #[error("{0}")]
    Invalid(#[from] ContractConversionError),
}

fn pool() -> &'static DescriptorPool {
    static POOL: OnceLock<DescriptorPool> = OnceLock::new();
    POOL.get_or_init(|| {
        DescriptorPool::decode(crate::proto::FILE_DESCRIPTOR_SET)
            .expect("the embedded descriptor set decodes")
    })
}

/// The path of the protobuf JSON file that sits beside `legacy`.
#[must_use]
pub fn protobuf_json_path(legacy: &Path) -> PathBuf {
    let mut name = legacy
        .file_name()
        .map(std::ffi::OsString::from)
        .unwrap_or_default();
    let text = name.to_string_lossy();
    let renamed = text.strip_suffix(".json").map_or_else(
        || format!("{text}.pb.json"),
        |stem| format!("{stem}.pb.json"),
    );
    name = renamed.into();
    legacy.with_file_name(name)
}

fn encode<M: prost::Message>(
    message_name: &str,
    message: &M,
) -> Result<Vec<u8>, ContractConversionError> {
    let descriptor = pool()
        .get_message_by_name(message_name)
        .ok_or_else(|| ContractConversionError::new("the message is not in the descriptor set"))?;
    let dynamic = DynamicMessage::decode(descriptor, message.encode_to_vec().as_slice())
        .map_err(|_| ContractConversionError::new("the message does not match its descriptor"))?;
    serde_json::to_vec(&dynamic)
        .map_err(|_| ContractConversionError::new("the message cannot be written as JSON"))
}

fn decode<M: prost::Message + Default>(
    message_name: &str,
    bytes: &[u8],
) -> Result<M, ContractConversionError> {
    let descriptor = pool()
        .get_message_by_name(message_name)
        .ok_or_else(|| ContractConversionError::new("the message is not in the descriptor set"))?;
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let dynamic = DynamicMessage::deserialize_with_options(
        descriptor,
        &mut deserializer,
        // Unknown fields are never ignored: a reader that skipped a field it
        // does not understand would skip a claim.
        &DeserializeOptions::new().deny_unknown_fields(true),
    )
    .map_err(|error| ContractConversionError::new(format!("not a valid document: {error}")))?;
    deserializer
        .end()
        .map_err(|_| ContractConversionError::new("unexpected data after the document"))?;
    dynamic
        .transcode_to::<M>()
        .map_err(|_| ContractConversionError::new("the document does not match its message"))
}

/// A contract document that has a legacy JSON form and a protobuf JSON form.
pub trait ContractDocument: Sized + Serialize + DeserializeOwned {
    /// Encode this document as protobuf JSON.
    fn to_protobuf_json(&self) -> Result<Vec<u8>, ContractConversionError>;

    /// Decode a document from protobuf JSON and validate it.
    fn from_protobuf_json(bytes: &[u8]) -> Result<Self, ContractConversionError>;

    /// Encode this document as the legacy JSON that drivers write today.
    fn to_legacy_json(&self) -> Result<Vec<u8>, ContractConversionError> {
        serde_json::to_vec(self)
            .map_err(|_| ContractConversionError::new("the document cannot be written as JSON"))
    }
}

macro_rules! contract_document {
    ($legacy:ty, $message:ty, $name:literal) => {
        impl ContractDocument for $legacy {
            fn to_protobuf_json(&self) -> Result<Vec<u8>, ContractConversionError> {
                encode($name, &<$message>::from(self))
            }

            fn from_protobuf_json(bytes: &[u8]) -> Result<Self, ContractConversionError> {
                <$legacy>::try_from(decode::<$message>($name, bytes)?)
            }
        }
    };
}

contract_document!(
    BoundaryConfig,
    pb::BoundaryConfig,
    "openshell.sandbox.runtime.v1.BoundaryConfig"
);
contract_document!(
    SandboxRuntimeDescriptor,
    pb::SandboxRuntimeDescriptor,
    "openshell.sandbox.runtime.v1.SandboxRuntimeDescriptor"
);
contract_document!(
    SupervisorAuthBundle,
    compute::SupervisorAuthBundle,
    "openshell.compute.v1.SupervisorAuthBundle"
);
contract_document!(
    SandboxLaunchAuthentication,
    compute::SandboxLaunchCredentials,
    "openshell.compute.v1.SandboxLaunchCredentials"
);

/// Read the document whose legacy file is `legacy`, preferring the protobuf JSON
/// file beside it.
pub fn read_document<T: ContractDocument>(legacy: &Path) -> Result<T, ContractDocumentError> {
    let preferred = protobuf_json_path(legacy);
    match std::fs::read(&preferred) {
        Ok(bytes) => return Ok(T::from_protobuf_json(&bytes)?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(ContractDocumentError::Io {
                path: preferred,
                source,
            });
        }
    }
    let bytes = std::fs::read(legacy).map_err(|source| ContractDocumentError::Io {
        path: legacy.to_path_buf(),
        source,
    })?;
    serde_json::from_slice(&bytes).map_err(|error| ContractDocumentError::Decode {
        path: legacy.to_path_buf(),
        message: error.to_string(),
    })
}
