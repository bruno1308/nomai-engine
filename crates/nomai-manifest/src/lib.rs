//! Nomai Manifest - Structured semantic game state for AI verification.
//!
//! This crate provides the manifest pipeline for the Nomai Engine. The manifest
//! captures a structured, queryable record of every ECS state change with full
//! causality metadata, enabling AI-driven verification and debugging.
//!
//! # Modules
//!
//! - [`journal`]: Change journal that records every component mutation with
//!   causality metadata (entity, component type, old/new values, issuing system,
//!   causal reason, command index, tick).

#![deny(unsafe_code)]

pub mod journal;
