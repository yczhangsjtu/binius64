//! # binius-zkvm-slice — zkVM validation crate
//!
//! Consolidated from 20 independent slice *binaries* (each with its own `main`)
//! into a single library crate. The slice code now lives under `src/bin/` but is
//! included here as modules via `#[path]`, and each slice's `fn main()` was turned
//! into `pub fn run_<name>()` exposed as a `#[test]`.
//!
//! Shared arithmetic / encoding helpers were extracted:
//! - [`alu`] — `to_bits`, `fa`, `add_constant`, `inc8`, `mul8`, `leq8`, `assert_bits`
//! - [`encode`] — RISC-V-style `enc_*` word encoders + field extractors
//!
//! ## Run the tests
//! Run `cargo test -p binius-zkvm-slice` from the workspace root (or `cargo test`
//! inside this crate). Each slice's `run_<name>` becomes a `#[test]`.

pub mod alu;
pub mod vm32;


// Slice modules. Each file under src/bin/ had its `fn main()` renamed to
// `pub fn run_<stem>()` by `scripts/migrate_slices.py`. They are pulled in here
// as modules so the crate exposes them and the tests exercise them.
#[path = "slices/inst_lookup.rs"]
mod inst_lookup;
#[path = "slices/mem_lookup.rs"]
mod mem_lookup;
#[path = "slices/pc_glue.rs"]
mod pc_glue;
#[path = "slices/pc_carry.rs"]
mod pc_carry;
#[path = "slices/instr_step.rs"]
mod instr_step;
#[path = "slices/multi_inst.rs"]
mod multi_inst;
#[path = "slices/branch.rs"]
mod branch;
#[path = "slices/factorial.rs"]
mod factorial;
#[path = "slices/combined.rs"]
mod combined;
#[path = "slices/multi_combined.rs"]
mod multi_combined;
#[path = "slices/mem_instr.rs"]
mod mem_instr;
#[path = "slices/mem_arg.rs"]
mod mem_arg;
#[path = "slices/mem_arg_ts.rs"]
mod mem_arg_ts;
#[path = "slices/jolt_bridge.rs"]
mod jolt_bridge;
#[path = "slices/mem_arg_spice.rs"]
mod mem_arg_spice;
#[path = "slices/full_vm.rs"]
mod full_vm;
#[path = "slices/full_vm_store.rs"]
mod full_vm_store;
#[path = "slices/full_vm_multi.rs"]
mod full_vm_multi;
#[path = "slices/full_vm_jolt.rs"]
mod full_vm_jolt;
#[path = "slices/zkvm.rs"]
mod zkvm;
#[path = "slices/reg_rw.rs"]
mod reg_rw;
#[path = "slices/word_add.rs"]
mod word_add;
#[path = "slices/word_add_combined.rs"]
mod word_add_combined;
#[path = "slices/word_vm.rs"]
mod word_vm;
#[path = "slices/word_vm_ram.rs"]
mod word_vm_ram;
#[path = "slices/word_vm32.rs"]
mod word_vm32;
#[path = "slices/ram_sort.rs"]
mod ram_sort;
#[path = "slices/vm_ram_sort.rs"]
#[allow(dead_code)] // 历史本地编码器（blt/bltu/sll/srl 等）与 prog_fetch 常量保留为公共 API 面
mod vm_ram_sort;

// Re-export the run functions so integration (or future bins) can call them.
pub use inst_lookup::run_inst_lookup;
pub use mem_lookup::run_mem_lookup;
pub use pc_glue::run_pc_glue;
pub use pc_carry::run_pc_carry;
pub use instr_step::run_instr_step;
pub use multi_inst::run_multi_inst;
pub use branch::run_branch;
pub use factorial::run_factorial;
pub use combined::run_combined;
pub use multi_combined::run_multi_combined;
pub use mem_instr::run_mem_instr;
pub use mem_arg::run_mem_arg;
pub use mem_arg_ts::run_mem_arg_ts;
pub use jolt_bridge::run_jolt_bridge;
pub use mem_arg_spice::run_mem_arg_spice;
pub use full_vm::run_full_vm;
pub use full_vm_store::run_full_vm_store;
pub use full_vm_multi::run_full_vm_multi;
pub use full_vm_jolt::run_full_vm_jolt;
pub use zkvm::run_zkvm;
pub use reg_rw::run_reg_rw;
pub use word_add::run_word_add;
pub use word_add_combined::run_word_add_combined;
pub use word_vm::run_word_vm;
pub use word_vm_ram::run_word_vm_ram;
pub use word_vm32::run_word_vm32;
pub use ram_sort::run_ram_sort;
pub use vm_ram_sort::{vmrs_prove, vmrs_prove_with_init, vmrs_verify, vmrs_verify_online, vmrs_verifier_setup, VmRsProof, VmRsVerifierKey, VmRsVerifyOut};
