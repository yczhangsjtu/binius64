# Binary-field zkVM over Binius64

This fork of [binius64](https://github.com/binius-zk/binius64) extends it with a
**binary-field RISC-V zkVM validation slice** — the project's core research work:
proving that Binius64's binary-field proof stack (spartan-prover + logup*)
can carry the full Jolt-style zkVM architecture, on GF(2^128).

Everything lives in this repo (no submodules, no external crate deps):
- **`crates/zkvm-slice/`** — the 25 validation slices (source of truth for the code;
  now a lib crate, see below)
- **`zkvm-project/`** — design docs, research notes, progress log, acceptance basis
  (the written record)

> ⚠️ **诚实分级约定**：⭐ = 真实现（机制闭环 + 有意义）；⚠️ = 演示 / 边界（机制可行但被
> 简化，核心困难未做进约束，多由 native 预计算 + 查表一致性完成）。⚠️ 区功能一律不得表述为
> "完整实现"。详细边界见 `zkvm-project/ACCEPTANCE_BASIS.md`。

## Why a zkVM on a binary field?

A **zkVM** proves "a program's execution *trace* is valid". On a **binary field**
(GF(2^128)), bit/byte/hash operations are FREE (XOR = addition, AND =
multiplication). The project thesis: **proving cost scales with the number of
instructions, not the instruction type** — hashes and bit-ops cost ~nothing on
a binary field, unlike prime-field SNARKs where bigint/ECDSA dominates.

## The 28 validation slices

Each slice is a minimal, end-to-end `prove → verify` (+ a soundness rejection)
demonstrating one zkVM mechanism on a binary field. Slices live under
`crates/zkvm-slice/src/slices/`, each with `pub fn run_<name>()` exposed as a
`#[test]`.

| # | binary | mechanism | proof system | grade |
|---|--------|-----------|--------------|-------|
| 1 | `inst_lookup` | instruction table lookup | logup* | ⭐ |
| 2 | `mem_lookup` | memory-consistency lookup | logup* | ⚠️ |
| 3 | `pc_glue` | register/PC state transition | Spartan | ⭐ |
| 4 | `pc_carry` | PC integer carry (full-adder chain) | Spartan | ⭐ |
| 5 | `instr_step` | full single RV32I instruction closed-loop | Spartan | ⭐ |
| 6 | `multi_inst` | multi-instruction program trace (cross-row) | Spartan | ⭐ |
| 7 | `branch` | conditional branch (beq) | Spartan | ⭐ |
| 8 | `factorial` | loop program (5! = 120, cross-row) | Spartan | ⭐ |
| 9 | `combined` | single-transcript Spartan + logup* | combined | ⭐ |
| 10 | `multi_combined` | multi-instruction fetch+execute combined | combined | ⭐ |
| 11 | `mem_instr` | memory load/store (single-address R-A-W) | Spartan | ⚠️ |
| 12 | `mem_arg` | memory argument (reads ⊆ writes sub-multiset) | logup* | ⭐ |
| 13 | `mem_arg_ts` | timestamped memory argument (most-recent-write) | logup* | ⚠️ |
| 14 | `jolt_bridge` | Jolt-frontend trace → binary-field backend | logup* | ⚠️ |
| 15 | `mem_arg_spice` | SPICE-style memory argument (global ts, no sorter) | logup* | ⚠️ |
| 16 | `full_vm` | loop + memory + integer (demo, cross-row) | combined | ⚠️ |
| 17 | `full_vm_store` | load/store read-modify-write (demo, cross-row) | combined | ⚠️ |
| 18 | `full_vm_multi` | multi-address interleaved r/w (demo, cross-row) | combined | ⚠️ |
| 19 | `full_vm_jolt` | word-driven opcode decode + cross-row x1/pc | combined | ⭐/⚠️ |
| 20 | `zkvm` | row-wise verifier (no cross-row, PC unconstrained) | combined | ⚠️ |
| 21 | `reg_rw` | register read==write binding (write-log W[(reg,ver)]); version chain not circuitized (→M3) | logup* | ⭐/⚠️ |
| 22 | `word_add` | word-level `iadd_32` 32-bit add (1 AND + 1 ZERO) vs bit-level control (256 mul) — M2 | frontend (M4) | ⭐ |
| 23 | `word_add_combined` | frontend word gate + logup* fetch, ONE transcript (transcript-level composition; fetched word does not drive execution) — M2 | combined (M4+logup*) | ⭐ |
| 24 | `word_vm` | **generic single-cycle state machine** — word-driven decode (addi/add/beq) + register read==most-recent-write via logup* + **version chain circuitized** + PC advance, 10-cycle multi-instruction program, ONE transcript — M3 | combined (M4 frontend + logup*) | ⭐ |
| 25 | `word_vm_ram` | **RAM memory argument** — K=64-word lw/sw on the M3 state machine; RAM read value pinned ONLY by logup* (no cross-address value chain), 64-version-counter chain circuitized (O(K·T)); 3-table logup* (fetch+reg-wlog+ram-wlog) in ONE transcript; init/final/output checks — M4 | combined (M4 frontend + logup*) | ⭐ |
| 26 | `word_vm32` | **true RV32I subset, 32-bit word width** — standard I/S/B/U/J encodings + sign-extended immediates, 32 registers (x0 hard-zero), K=64 RAM, on a full 30-instruction gate-level ALU (add/sub/sll/slt/sltu/xor/srl/sra/or/and + addi/slti/sltiu/xori/ori/andi/slli/srli/srai + lui/auipc/jal/jalr + lw/sw + 6 branch conditions) — M5 | combined (M4 frontend + logup*) | ⭐ |
| 27 | `ram_sort` | **sorted offline memory checking** — fracaddcheck multiset equality (fingerprint addr+ρ·val+ρ²·ts+ρ³·kind) over 4 committed columns; gates independent of K, linear in T; **BaseFold strong channel** — M7/M8-A | fracaddcheck + BaseFold | ⭐ |
| 28 | `vm_ram_sort` | **real VM × scalable RAM argument** — vm32-semantics RV32I execution + sorted RAM argument (O(K·T) version chain **deleted**), K=2^16 words; event/sorted columns BaseFold-committed, ONE transcript; bubblesort end-to-end prove→verify + init/final/output; N=16: 1801 cyc, 905k gates, 1.9s — M8-A | combined (frontend + fracaddcheck + BaseFold) | ⭐ |

> 注：`full_vm` 家族（16-18）**有跨行状态绑定**（`x1`/`i`/`pc` 的 `[t+1]`），局限性在执行
> 模板化（无真正指令译码/寄存器堆）+ 内存时序为手工填值；仅 `zkvm`（20）**无跨行**（PC 未
> 约束）。见 `ACCEPTANCE_BASIS.md` / `ACCEPTANCE_REPORT.md`。


## Current state (2026-09-08, M1-M7 + M8-A complete; Phase 2 in progress)

- **Phase 1 (M1-M6) ✅** (see `designs/milestone-roadmap.md` §4 and the M1..M6 reports).
  Slices 24-26 (`word_vm` / `word_vm_ram` / `word_vm32`) form the modern evidence chain:
  generic single-cycle state machine → RAM memory argument → true RV32I 32-bit VM with
  bubblesort end-to-end.
- **Phase 2 (M7-M10)** (see `designs/binary-zkvm-full-roadmap.md`):
  **M7 ✅** — scalable RAM argument, sorted offline memory checking, slice 27 (`ram_sort`),
  gates independent of K / linear in T (`M7_REPORT.md`).
  **M8-A ✅** — slice 28 (`vm_ram_sort`): VM × RAM argument integration on the **BaseFold
  strong commitment channel**, O(K·T) version chain deleted, K=2^16 words, bubblesort
  end-to-end prove→verify + 4 verify-layer soundness rejections (`M8_REPORT.md`).
  Next: M8-B (full ISA + riscv32 toolchain + compiled programs).
- **Thesis evidence**: `zkvm-project/BENCHMARKS.md` — per-instruction cost table for all
  31 RV32I instructions (N=16 micro-programs, full prove+verify): **30/31 rows identical
  (gates=22388, g/cyc=973)**; instruction type does not affect per-cycle cost.
- **M6 consolidation**: `crates/zkvm-slice/src/vm32/` — the M5 VM, split into
  `isa.rs` / `interp.rs` / `circuit.rs` / `proof.rs` (slice 26 is now a thin layer);
  39/39 tests (per-instruction boundary tests + independent soundness tests).
- **Known boundaries (honest summary)**:
  1. Fixed/unrolled programs (no dynamic loop bound in-circuit);
  2. fetch / reg-wlog / ram-wlog tables are native-provided — logup* proves consistency
     (claim ∈ table), version chains in-circuit enforce most-recent-write semantics from M3 on;
     slice 28 replaces the RAM wlog with the sorted argument (fetch argument still omitted);
  3. ~~O((32+64)·T) version-chain cost dominates~~ → **solved in M8-A for RAM** (sorted
     argument, gates independent of K); register version chain (32 regs) remains in-circuit by design;
  4. word addressing (no byte/alignment), address OOB silently masked to &0xffff (K=2^16, slice 28);
  5. `mul` (RV32M) not integrated (→ M8-B).

## Building & running

The crate is now a **lib** (no `[[bin]]`, so `cargo run --bin` does not work). Run
the slices as tests:

```bash
export RUSTFLAGS="-C target-cpu=native"   # i5-12400F: AVX2, no AVX-512
CARGO_BUILD_JOBS=4                         # avoid OOM on 12-core/32GB
cargo test -p binius-zkvm-slice            # runs all slice tests (53 + 3 ignored)
# single slice, e.g. factorial:
cargo test -p binius-zkvm-slice --lib factorial
```

Note: use the rustup toolchain (rust-toolchain.toml pins 1.97.1; ensure `~/.cargo/bin`
precedes `/usr/bin` in PATH — the system cargo 1.75 cannot parse the workspace manifests).

## Rounding out

- **`architecture.md`** — the authoritative system-architecture doc (thesis, tech stack,
  the 25-slice evidence chain, Jolt↔Binius64 transcription map, roadmap). **Start here.**
- **`ACCEPTANCE_BASIS.md`** — the independent verification baseline (paths, commands,
  per-slice honest grading, key boundaries). **Source of truth for grading.**
- **`ACCEPTANCE_REPORT.md`** — the acceptance Agent's audit of this project (2026-09-06).
- `zkvm-project/PROGRESS.md` — build order (note: legacy M-A1/M-A2 sections are historical).
- `zkvm-project/research/jolt-binius-memory-argument-mapping.md` — the
  Jolt ↔ Binius64 memory-argument transcription map (backend-swap feasibility).
