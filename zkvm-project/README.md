# Binary-field zkVM over Binius64

This fork of [binius64](https://github.com/binius-zk/binius64) extends it with a
**binary-field RISC-V zkVM validation slice** — the project's core research work:
proving that Binius64's binary-field proof stack (spartan-prover + logup*)
can carry the full Jolt-style zkVM architecture, on GF(2^128).

Everything lives in this repo (no submodules, no external crate deps):
- **`crates/zkvm-slice/`** — the 14 validation slices (source of truth for the code)
- **`zkvm-project/`** — design docs, research notes, progress log (the written record)
- **`src/`** — RV32I native reference interpreter (legacy flock-era, for cross-check)

## Why a zkVM on a binary field?

A **zkVM** proves "a program's execution *trace* is valid". On a **binary field**
(GF(2^128)), bit/byte/hash operations are FREE (XOR = addition, AND =
multiplication). The project thesis: **proving cost scales with the number of
instructions, not the instruction type** — hashes and bit-ops cost ~nothing on
a binary field, unlike prime-field SNARKs where bigint/ECDSA dominates.

## The 14 validation slices

Each slice is a minimal, end-to-end `prove → verify` (+ a soundness rejection)
demonstrating one zkVM mechanism on a binary field.

| # | binary | mechanism | proof system |
|---|--------|-----------|--------------|
| 1 | `inst_lookup` | instruction table lookup | logup* |
| 2 | `mem_lookup` | memory-consistency lookup | logup* |
| 3 | `pc_glue` | register/PC state transition | Spartan |
| 4 | `pc_carry` | PC integer carry (full-adder chain) | Spartan |
| 5 | `instr_step` | full single RV32I instruction closed-loop | Spartan |
| 6 | `multi_inst` | multi-instruction program trace | Spartan |
| 7 | `branch` | conditional branch (beq) | Spartan |
| 8 | `factorial` | loop program (5! = 120) | Spartan |
| 9 | `combined` | single-transcript Spartan + logup* | combined |
| 10 | `multi_combined` | multi-instruction fetch+execute combined | combined |
| 11 | `mem_instr` | memory load/store (R-A-W) | Spartan |
| 12 | `mem_arg` | memory argument (reads ⊆ writes) | logup* |
| 13 | `mem_arg_ts` | timestamped memory argument (most-recent-write) | logup* |
| 14 | `jolt_bridge` | Jolt-frontend trace → binary-field backend | logup* |

## Building & running

```bash
export RUSTFLAGS="-C target-cpu=native"   # i5-12400F: AVX2, no AVX-512
CARGO_BUILD_JOBS=4                         # avoid OOM on 12-core/32GB
cargo run -p binius-zkvm-slice --bin <name>
```

## Rounding out

- **`architecture.md`** — the authoritative system-architecture doc (thesis, tech stack,
  the 14-slice evidence chain, Jolt↔Binius64 transcription map, roadmap). **Start here.**
- `zkvm-project/PROGRESS.md` — build order & next steps.
- `zkvm-project/research/jolt-binius-memory-argument-mapping.md` — the
  Jolt ↔ Binius64 memory-argument transcription map (backend-swap feasibility).
