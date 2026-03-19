#![deny(missing_docs)]

//! `torvyn-packaging` — OCI artifact assembly, signing, distribution, and inspection.
//!
//! This crate implements the packaging layer for Torvyn components:
//! - Artifact assembly (`torvyn pack`): [artifact::pack]
//! - Artifact inspection (`torvyn inspect`): [inspection::inspect]
//! - OCI manifest and reference types: [oci]
//! - Signing provider trait: [signing::SigningProvider]
//! - Provenance generation: [provenance::ProvenanceRecord]
//! - Local cache management: [cache::ArtifactCache]
//! - Component resolution: [resolution::resolve]
//!
//! # Architecture
//!
//! Per HLI Doc 08, Torvyn artifacts are OCI-compatible archives containing
//! a Wasm Component binary, WIT definitions, a manifest, and provenance.
//! The crate is structured as a library consumed by the `torvyn-cli` crate.
//!
//! # Phase 0 Scope
//!
//! - Artifact pack/unpack: fully implemented.
//! - OCI push/pull: trait defined, stub implementation for testing.
//!   Real OCI client is Phase 2 (per MR-16).
//! - Signing: trait defined, stub + test-only local key provider.
//!   Sigstore integration is Phase 2 (per MR-17).
//! - Cache and resolution: fully implemented for local workflows.

pub mod artifact;
pub mod cache;
pub mod digest;
pub mod error;
pub mod inspection;
pub mod manifest;
pub mod media_types;
pub mod oci;
pub mod provenance;
pub mod resolution;
pub mod signing;

// Re-exports for convenience
pub use artifact::{pack, unpack, ArtifactContents, PackInput, PackOutput};
pub use cache::{ArtifactCache, CacheConfig};
pub use digest::ContentDigest;
pub use error::PackagingDetailError;
pub use inspection::{inspect, InspectionResult};
pub use manifest::{ArtifactManifest, WitPackageRef};
pub use oci::{OciReference, RegistryClient};
pub use provenance::ProvenanceRecord;
pub use resolution::{resolve, ResolutionSource, ResolvedArtifact};
pub use signing::{SignatureInfo, SigningMethod, SigningProvider};
