# Baseline 换线决策：废弃 flock，改用 Binius64 (2026-09-01)

## 背景
flock 的 `CircuitBuilder` wiring 与 witness packing 的 `Wiring(Gkr(ProductMismatch))` 问题，
另一会话未解决。经调研，放弃 flock 路径，改用**现成、活跃、二元域**的 Binius64 作为 baseline。

## 关键调研结论（更新时间 2026-09 现查）
- **PetraVM 已 Archived (2026-01-08, read-only)** — 历史上唯一 Binius 之上的通用 VM 已停更，不可用。
- **RISC Zero × Binius 未落地** — 2025-01 宣布合作但无产物，risc0 仍是素域 STARK。
- **Binius64 (binius-zk/binius64, 1804 commits, active)** — **唯一成熟可用的二元域证明后端**。
  不是 VM，而是词级约束系统 + 证明层。表达任意计算 = 写词级电路。
- **市场上无"现成成熟 RISC-V 二元域 zkVM"** — 这正是本项目 gap。

## Binius64 架构核心（为何能免 Wiring bug）
- **4 种 64-bit 词级约束**表达任意计算，直接支撑"指令类型无关成本"：
  - `ZERO`: operand(XOR of shifted values)=0 → char-2 加法/线性关系
  - `AND`: `A & B = C` 词级按位与
  - `IMUL`: `A * B = (HI<<64)|LO` **64-bit 无符号整数乘法** (大整数/进位算术基础)
  - `BMUL`: GHASH 域 GF(2^128) 乘法
- **shifted value index** → 词带免费 SLL/SRL/SAR 移位参数，RISC-V 移位指令变 reindexing（成本≈0）
- **`CircuitBuilder` trait 由 `ConstraintBuilder`(抽象)+`WitnessGenerator`(具体) 共同实现，严格 1:1**
  → 从根本避免 flock 的抽象层与 witness 层脱节 bug
- consumer-facing `binius_frontend::CircuitBuilder`：`add_witness`/`add_inout`/`band`/`bxor`/
  `iadd_32`/`assert_eq`(name,a,b)/`hint` → 词级指令操作，底层自动编译成 ZERO/AND/IMUL/BMUL
- witness 填充：`WitnessFiller` from circuit, 赋值 + `populate_wire_witness()`

## 成本模型（binius_frontend 官方文档，直接量化每类指令）
- **AND 约束** = 基准单位成本
- **IMUL 约束** = 3-4× AND
- **线性操作**(XOR/移位) = 虚拟线性约束, 编译时融合进相邻 AND 门 ≈ 近零 (<0.1× AND, 移位略多)
- **常数** = 0 约束成本
- **提交值** = ~0.2× AND
- gate fusion / ZERO reduction 自动实现上述优化

## 端到端实测验证 (i5-12400F, 2026-09-01)
| 计算 | hidden_words | bitand | imul | Prove | Verify | 证明大小 |
|---|---|---|---|---|---|---|
| blake3 (哈希, msg=32B) | 304 | 184 | **0** | 5.66ms | 289µs | 27 KiB |
| sha256 (哈希, msg=8B) | 486 | 368 | **0** | — | 302µs | 26 KiB |
| ethsign (ECDSA 1 签名) | 189,236 | 113,992 | **20,538** | 501ms | 15.6ms | 314 KiB |

→ 精确印证领域失配：哈希=位运算→零 IMUL；ECDSA=整数乘法→20k+ IMUL。
但 IMUL 是**原生词级约束**(1 个=1 纪律约束, 固定 3-4× AND)，非位级展开 → 成本可控。

## 构建/运行环境 (关键)
- 工具链: Binius64 要求 Rust **1.97.1** (rust-toolchain.toml)。本机默认 1.95.0，
  需 `rustup toolchain install 1.97.1`。装好后 `cargo` 自动用 1.97.1。
- 构建: `export RUSTFLAGS="-C target-cpu=native"`
- **OOM 警告**: i5-12400F (12 核, 32GB RAM)。release 全量并行编译会 OOM 被 SIGKILL。
  **必须用低并行**: `CARGO_BUILD_JOBS=4 cargo build [--release] --example <name>`
- 跑示例: `cargo run [--release] --example <name> -- prove [--message-len N | --random-message | ...]`
  - blake3: `--message-len 32 --random-message`
  - sha256: `--message-len 8`
  - ethsign: `-n 1` (默认)
  - 通用 CLI: prove(默认) / stat / composition / check-snapshot / save

## 下一步建议 (M-B 起)
- **验证核心 API**: 用 `binius_frontend::CircuitBuilder` 手写一个最小 RV32I 指令门
  (ADD/AND/XOR/SLL)，走 prove→verify 端到端 —— 验证"写指令门"的开发体验
- **把 M-A1 的 isasim.rs RV32I 解释器接入**: 每条指令 trace 出一个词级约束子电路，
  复用 native 对拍作为 ground-truth
- **成本量化表**: 用 CircuitStat 输出每类指令的 bitand/imul 约束数，替代设计文档 §3 的估计
- **SubCircuit/组合**: 用 `build_gadget` 把每条指令做成 gadget，支持复用和并行

## 待确认/风险
- Binius64 社区维护 (Irreducible 关停)。但 crates 文档质量高,算活跃。
- "zero-knowledge" 需 `--zk` 显式开启, 默认是 SNARK 非 zk-SNARK。
- 无现成 RISC-V 前端, 需自建 (复用 isasim.rs)。