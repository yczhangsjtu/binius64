//! vm32 — the M5 word VM, consolidated into a reusable library (M6 T1).
//!
//! Layers:
//! - [`isa`] — real RV32I (subset) encoders/decoder + machine constants
//! - [`interp`] — native reference interpreter (`run_program`) + trace types
//! - [`circuit`] — gate-circuit builder (`build_circuit`) + inout layout
//! - [`proof`] — proof pipeline (`run_machine_full`, reverify, claims, wlogs)
pub mod isa;
pub mod elf;
pub mod interp;
pub mod circuit;
pub mod proof;
