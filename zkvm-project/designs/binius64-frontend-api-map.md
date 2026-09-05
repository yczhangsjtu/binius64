# Binius64 Frontend API 全景 —— zkVM 指令级原语（2026-09-01）

confirmation: `crates/frontend/src/builder/mod.rs` 排查确认。

## 关键结论
Binius64 的 consumer-facing `CircuitBuilder` 门集与 RISC-V 指令**几乎一一对应**，
且内置 32 位操作。实现 zkVM 无需自定义位级翻译，直接组合这些原生词级门即可。

## 可用原语一览（builder/mod.rs pub fn）
- 值声明: `add_witness()`(private) / `add_inout()`(public) / `add_constant()` / `add_constant_64()` / `add_constant_zx_8()`
- 算术: `iadd_32` / `iadd32_cin_cout` / `iadd_cin_cout` / `isub_bin_bout` / `imul`(→hi,lo) / `smul`(有符号)
- 位运算: `band`(1×AND) / `bor` / `bxor` / `bxor_multi` / `bnot` / `bmul`(GF(2^128))
- 移位: `shl` / `shr` / `sar` / `rotl` / `rotr` / `sll32` / `srl32` / `sra32` / `rotl32` / `rotr32`
- 比较: `icmp_eq` / `icmp_ne` / `icmp_ult/ule/ugt/uge` / `smul`
- 控制: `select`(MSB-bool) / `assert_*` / `assert_eq(name,a,b)` / `assert_eq_cond` / `assert_non_zero`
- 组合: `subcircuit` / `build_gadget` / `register_chip`(gadget→chip) / `add_chip` / `call_chip`
- 其他: `extract_byte` / `fax` / `force_commit` / `mark_inout`(内部→公开输出) / `hint`

## Opcode 枚举（编译后约束）
AssertEq / AssertEqCond / AssertFalse / AssertNonZero / AssertTrue / AssertZero /
Band / Bmul / Bor / Bxor / BxorMulti / Fax / Iadd32 / Iadd32CinCout / IaddCinCout /
IcmpEq / IcmpUlt / Imul / IsubBinBout / Select / Shift

## MSB-Boolean 约定
布尔值编码在 64-bit word 的**最高位(bit 63)**；MSB=1 true, MSB=0 false，低 63 位
"don't care"。`select`/`icmp_*` 都读 MSB。→ 比较/分支/条件选择都走这个约定。

## 成本规律（builder/mod.rs 内联注释）
- band = 1 约束（或两端同 wire 时 0）
- imul → hi,lo 两输出，构成 IMUL 约束（3-4×AND）
- XOR/移位 = 线性，融合进相邻门 ≈ 近零
- select = MSB 读一位

## 对 zkVM 的映射意义
| RISC-V | frontend 原语 |
|---|---|
| ADD/ADDI | iadd_32 / iadd_cin_cout (+iadd32_cin_cout 并行两半) |
| SUB | isub_bin_bout |
| AND/OR/XOR | band / bor / bxor |
| SLLI/SRLI/SRAI | shl / shr / sar (含 sll32/srl32/sra32 半字) |
| MUL/MULH | imul → (hi,lo) ; smul 有符号 |
| SLTI(U) | icmp_* (+sign 处理) |
| BEQ/BNE | icmp_eq/ne → select (PC) |
| 寄存器文件 | wire 数组, mark_inout 暴露最终状态 |
| 内存 LW/SW | add_inout witness + select/assert_eq |

## zkVM 实现架构（与 designs/binius64-constraint-proofs-and-zkvm-plan.md 呼应）
1. 复用 M-A1 isasim.rs 生成 native trace（ground-truth）。
2. 每类指令 = 一个 frontend gadget 函数（用上表原语组合），输入=寄存器/PC/内存,
   输出=更新后状态。
3. 指令序列 = 顺序连接各 gadget 的 wire（数据流），每步寄存器快照或追踪当前值。
4. build → WitnessFiller 填充 native trace 值 → prove/verify。
5. CircuitStat 输出每指令约束数 → 成本量化表。

## 待验证
- 前端对"寄存器文件时序状态（每步读写）"的支持方式（是否需每步显式传寄存器快照，
  还是有 stateful 模式）。
- 内存（LRW/SW）的 array select 实现。
- 多指令组合是否 CSE/gate-fusion 自动优化共享子表达式。