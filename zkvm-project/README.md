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

## The 26 validation slices

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

> 注：`full_vm` 家族（16-18）**有跨行状态绑定**（`x1`/`i`/`pc` 的 `[t+1]`），局限性在执行
> 模板化（无真正指令译码/寄存器堆）+ 内存时序为手工填值；仅 `zkvm`（20）**无跨行**（PC 未
> 约束）。见 `ACCEPTANCE_BASIS.md` / `ACCEPTANCE_REPORT.md`。


## Current state (2026-09-07, M1-M6 complete — project closed out)

- **All six milestones are ✅ complete** (see `designs/milestone-roadmap.md` §4 and the
  M1..M6 reports). Slices 24-26 (`word_vm` / `word_vm_ram` / `word_vm32`) form the modern
  evidence chain: generic single-cycle state machine → RAM memory argument → true RV32I
  32-bit VM with bubblesort end-to-end.
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
  3. O((32+64)·T) version-chain cost dominates (VER_MAX=128);
  4. word addressing (no byte/alignment), RAM size K=64, address OOB silently masked to &0x3f;
  5. `mul` (RV32M) not integrated.

## Building & running

The crate is now a **lib** (no `[[bin]]`, so `cargo run --bin` does not work). Run
the slices as tests:

```bash
export RUSTFLAGS="-C target-cpu=native"   # i5-12400F: AVX2, no AVX-512
CARGO_BUILD_JOBS=4                         # avoid OOM on 12-core/32GB
cargo test -p binius-zkvm-slice            # runs all 26 slice tests
# single slice, e.g. factorial:
cargo test -p binius-zkvm-slice --lib factorial
```

## Rounding out

- **`architecture.md`** — the authoritative system-architecture doc (thesis, tech stack,
  the 25-slice evidence chain, Jolt↔Binius64 transcription map, roadmap). **Start here.**
- **`ACCEPTANCE_BASIS.md`** — the independent verification baseline (paths, commands,
  per-slice honest grading, key boundaries). **Source of truth for grading.**
- **`ACCEPTANCE_REPORT.md`** — the acceptance Agent's audit of this project (2026-09-06).
- `zkvm-project/PROGRESS.md` — build order (note: legacy M-A1/M-A2 sections are historical).
- `zkvm-project/research/jolt-binius-memory-argument-mapping.md` — the
  Jolt ↔ Binius64 memory-argument transcription map (backend-swap feasibility).
