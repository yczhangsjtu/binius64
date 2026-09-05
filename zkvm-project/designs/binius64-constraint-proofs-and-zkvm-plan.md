# Binius64 词级约束的证明机制 + zkVM 实现方案

日期: 2026-09-01 | 关联: designs/binius64-baseline-decision.md

---

## Part A: 三类词级约束的证明机制

### A.0 统一归约框架

所有约束的证明都把"约束集合是否被 witness 满足"归约为**多项式恒等式 zerocheck**，
再经 sumcheck + PCS 验证。每类约束组织成"列"（每条约束一行），witness 词被逐位提升为
boolean hypercube 上的多重线性多项式，证明组合多项式在 hypercube 上恒为零。

流程: 采样挑战 r → 算 ∑_x eq(r,x)·P(x) = 0 → MLE-check sumcheck →
归约到各列在某点求值 → PCS/BaseFold(FRI) 打开 → 与提交 witness 一致。

### A.1 ZERO 约束（线性，无独立协议）
ZERO = operand(XOR of shifted values) = 0，operator 天然是 word 的 GF(2)-线性组合。
- 不需要乘性协议，直接编码为线性关系，由 Shift Reduction（见下）一并处理。
- 生成 witness 时 operator 求值必须为 0。
- 成本 ≈ 0.1× AND（融合进 ZERO reduction，无 prover message）。

### A.2 AND 约束: oblong univariate-skip zerocheck
文件: `crates/prover/src/and_reduction/`（prover.rs / prove.rs / ntt_lookup.rs）
关系: A & B = C，即 A·B ⊕ C = 0（词级按位）。

**关键洞察**: C 列不传给证明者，逐 word 推导 C = A & B。sound 性:
- 诚实 witness 每行满足 C = A & B；
- folding 在 word 位上保持 GF(2)-线性；
- 等值 word 折叠得等值域元素 → 诚实证明者发不出不同 transcript；
- 作弊 witness 仍被后续 shift reduction 检测 C 求值与提交 witness 不一致。

**协议两阶段**:
1. Phase 1: 构造单变量多项式 R0(Z)（NTT lookup 高效预计算），在扩展域求值发送。
   证明者消息域 = `BinarySubspace::<B8>::with_dim(Word::LOG_BITS + 1)` (dim 65 位)。
2. Phase 2: Fiat-Shamir 采样单变量挑战 z → fold 掉 hypercube 第一维 →
   跑 quadratic_mlecheck_prover 证明:
   R0(z) = ∑_{x∈{0,1}^n} Ã(z,x)·B̃(z,x)·eq(x;r)
   → 归约到 A,B,C 三个点的求值 claims。

**相对成本**: 基准 1×。无进位传播 → polylog 深度。

### A.3 BMUL 约束（GF(2^128) 域乘法）: quadratic MLE-check zerocheck
文件: `crates/prover/src/protocols/binmul.rs`
关系: A ⊗ B = C（GHASH 域乘法，无进位）。

**本质**: 域乘法 = 多项式系积模约化，本身是 GF(2)-二次关系，直接 quadratic zerocheck:
1. (lo,hi) word 对拼成 128-bit 域元素的多重线性表（a,b,c）:
   ⟪z_lo,z_hi⟫_x = ∑_{i<64} z_lo,x,i·X^i + ∑_{i<64} z_hi,x,i·X^{64+i}
2. 采样 zerocheck 挑战 r_z，证明:
   ∑_x eq(r_z,x)·(Ã(x)B̃(x) - C̃(x)) = 0
3. quadratic_mlecheck_prover 归约到 A,B,C 公共点求值。

**成本**: ~1×。二元域域乘法零放大（印证哈希/线性代数方向）。

### A.4 IMUL 约束（64-bit 整数乘法）: 指数幂表 + GKR + logup*
文件: `crates/prover/src/protocols/intmul/`（prove.rs / witness.rs）
关系: A × B = LO ∥ HI（整数乘法，128-bit 积拆两半）。
**这是进位整数算术的成本黑洞，Binius64 为其专门设计 5 阶段协议。**

核心技巧: 用生成元 G 把乘法编码进幂表（G^{a_i}），用 GKR product check 树验证，
用 logup* committed lookup 读出幂。

**五阶段**（IntMulProver::prove）:
- Phase 1: GKR 树根求值 + product check。b 的位作变量基底，product-check 树根 = 完整
  乘积 c 的 MLE 根。采样点 r 求值后归约到 (b·(G^{a_i}-1)+1)^{2^i} 的求值 claims。
- Phase 2: Frobenius twist（(·)^{2^i} 是自同态），扭回 b·(G^{a_i}-1)+1 的形式。
- Phase 3: selector sumcheck 把 claims 归约到 G^{a_i} 和 b；r_I^b 重组 2^k 个 per-bit b claims；
  对 c_lo∥c_hi 组合树跑第一层 GPA。
- Phase 4: 常量基底 product check。共享幂表 i↦G^i，对 a,c_lo,c_hi 三棵树批量 product check，
  根归约到 per-limb 求值 claims。
- Phase 5: logup* 查询 + overflow 校验。per-limb claims 经 Frobenius 扭到幂表，用 committed
  logup* lookup 读出幂；overflow parity zerocheck 验证 a0·b0⊕c_lo,0 奇偶（防进位溢出逃逸）；
  最后把 index claim、b 重组 claim、overflow zerocheck 批量 sumcheck 到公共点。
- 输出: a,b,c_lo,c_hi 四列在公共点的 per-bit 求值 claims。

**成本**: 3-4× AND。不退化位级连加法器（那要 64² 门），用幂表 lookup 压缩。

### A.5 技术层次对比表
| 约束 | 数学本质 | 归约技术 | 相对成本 |
|---|---|---|---|
| ZERO | GF(2) 线性 | Shift Reduction（无独立协议） | ~0.1× |
| AND | 词级按位与（无进位） | oblong univariate-skip zerocheck | 1× |
| BMUL | GF(2^128) 域乘法（无进位） | quadratic MLE-check zerocheck | ~1× |
| IMUL | 64-bit 整数乘法（有进位） | 幂表 + GKR product check + Frobenius + logup* + overflow | 3-4× |

---

## Part B: 用 Binius64 实现 zkVM 的方案

### B.0 核心认知
Binius64 提供的是**词级约束后端 + frontend**，不是 VM。zkVM = "通用指令集执行轨迹
的算术化"。因此工作 = 把一段 RV32I 程序 + 每条指令如何改变寄存器/内存/PC，
编码成 Binius64 的词级约束。

### B.1 分三层
1. **执行层（native reference）**: 复用 M-A1 的 `isasim.rs` RV32I 解释器，
   生成程序 trace（每步的寄存器值/PC/内存读写），作为 ground-truth。
2. **约束层（词级电路）**: 用 `binius_frontend::CircuitBuilder` 把 trace 编码成
   word 级约束电路。每条指令一个"块"，连接相邻指令的状态。
3. **证明层（Binius64 原生）**: build → WitnessFiller 填充 → prove → verify。

### B.2 状态表示（词级）
- 寄存器组: 32 个 RISC-V 寄存器 → 每步一个快照，或追踪当前值。每条指令读 2 写 1。
- PC: 一个 word，指令执行时更新（顺序/分支）。
- 内存: 有界数组，每 word 一条 inout witness（load/store 读写作约束）。
- 所有状态都是 committed witness（private）或公开 inout（statement 部分）。

### B.3 指令 → 词级约束映射
| RV32I 指令 | 词级约束 | 说明 |
|---|---|---|
| XOR | ZERO 三项 [rd, rs1, rs2] | 特征2加法，零成本 |
| AND/OR | AND (+ XOR 组合) | OR 用德摩根 |
| SLL/SRL/SRA | shifted value index | **纯 reindex，成本≈0** |
| ADD (进位) | IMUL 展开进位 + 校验 | 大整数加法核心 |
| SUB | ADD + 2 补码 | 复用加法 |
| SLT/SLTU | IMUL 展开差值符号比较 | 下一位比较指令 |
| MUL/MULH (RV32M) | IMUL | 原生 3-4×AND |
| LW/SW | 内存一致性约束 | array select + equality |

### B.4 最小纵向切片（M-B）
1. 写单条 ADD 指令门: 两寄存器进 → 和出，用 IMUL 约束进位正确。
2. 端到端 prove/verify 一条真实 ADD trace（native 对拍 ground-truth）。
3. 扩展 AND/XOR/SLL: 复用 isasim.rs trace。
4. 加入寄存器组状态流转 + PC 顺序，连接多条指令。
5. 加入分支 BEQ/BNE 的 PC 更新。
6. 加入 LW/SW 有界内存。
7. 跑通阶乘/冒泡程序（设计文档既定 M-B 目标）。

### B.5 成本量化（M-C 起）
用 `CircuitStat` 输出每条指令的 ZERO/AND/IMUL/BMUL 约束数，替换设计文档 §3 的
估计表。目标: 验证"性能与指令数相关、与类型无关"（哈希零 IMUL、ECDSA 大整数
走固定 3-4×IMUL）。

### B.6 待确认
- Binius64 frontend 是否天然支持"寄存器文件读写 + PC 步进"这类时序状态
  （需查是否适合做 VM 风格的逐步电路，还是需要外部每步显式传入）。
- 内存模型的实现细节（array select、read-only vs read-write）。
- 零知识 (--zk) 的取舍。