#![deny(missing_docs)]

//! Pipeline topology construction, validation, and instantiation for Torvyn.
//!
//! This crate builds pipeline topologies from configuration, validates them
//! (acyclic, connected, type-compatible), and instantiates components into
//! running flows.
