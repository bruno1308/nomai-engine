//! PyO3 Python bindings for the Nomai Engine.
//!
//! Exposes the engine's tick loop, manifest pipeline, and world manipulation
//! to Python. Manifest data is passed as Python dicts (JSON round-trip) for
//! compatibility with the existing `nomai-sdk` Python dataclasses.

#![deny(unsafe_code)]

use pyo3::prelude::*;

mod engine;

/// The `nomai._engine` native module.
#[pymodule]
fn _engine(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<engine::PyNomaiEngine>()?;
    Ok(())
}
