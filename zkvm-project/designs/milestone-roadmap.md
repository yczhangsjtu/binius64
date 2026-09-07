# 里程碑路线重新评估与统一编号（基于 Jolt 迁移难度分析）

> 日期：2026-09-06 | 依据：`research/jolt-to-binary-field-migration-assessment.md`（四路代码审计）
> 对象：`research/zkvm-gap-analysis-jolt.md` 的 M2（查表化执行）/ M3（内存时序论证）、
> `M1_ACCEPTANCE.md` 的"版本链电路化"、`architecture.md` §6 路线图。
> 本文同时是**全项目里程碑编号的权威定义**（取代历史上的 M-A1/M-A2/M-B/旧M2/旧M3）。
> 结论先行：**原 M2/M3 的顺序正确、方向正确，但技术内涵都需要按迁移分析修正——
> 旧 M2 的"LookupQuery 查表化执行"不能照抄 Jolt（combined-operand trick 在 char-2 整体失效），
> 旧 M3 的"Jolt 式 one-hot+increment"表述建立在被高估的同构上（Twist 非 multiset）。**

---

## 0. 统一编号方案

编号混乱来源：flock 时代的 M-A1/M-A2（历史遗留，代码已不存在）、PROGRESS 的"M-B"（从未使用）、
gap-analysis 的旧 M1/M2/M3。现统一为：

- **flock 时代（M-A1/M-A2）**：标记为历史遗留，不再占用编号，相关文档仅供"为什么换线"的凭证。
- **Binius64 时代**：单序列 **M1–M6**，定义见 §4 表。其中 M1 已完成（reg_rw），
  与 gap-analysis 的 M1 一致、编号不变。

---

## 1. 评估前提：区分两种"迁移"

迁移分析覆盖了两种场景，里程碑属于 (A)，但 (B) 的结论会反过来修正 (A) 的技术选型：

- **(A) 借架构自建**（本项目当前路线）：在 Binius64 栈（spartan-prover + logup*）上重建 Jolt 式 zkVM 机制。切片 1-21 属于此。
- **(B) 移植 Jolt 代码库**：把 `~/workspace/jolt`（重构 fork）换域到 GF(2^128)。

关键：迁移分析中"uni-skip 失效、batch padding two_inv 失效、Dory 报废"等断点**只影响 (B)**——我们用 Binius64 自带的 sumcheck/PCS，这些不是问题。对里程碑有实质影响的是另外两条：**整数嵌入语义失效**和 **Twist 同构被高估**。

---

## 2. 对旧 M2（查表化执行）的重新评估

### 2.1 原定义的问题

gap-analysis 的旧 M2 是"把每指令一个定制电路换成 Jolt 式统一周期查表（LookupQuery + CircuitFlags）"。迁移分析揭示：**Jolt 的 LookupQuery 机制在二元域上不能照抄**——

- Jolt 的 ADD/SUB/MUL 靠 **combined-operand trick**：索引 = 整数 x+y（进位隐含在索引≥2^64）、x+2^64−y、128-bit 积 x·y，R1CS 约束用域加法/乘法绑定（`rv64.rs:207-238, 377-381`）。这在 char-2 下整体塌缩（`from_u64(a)+from_u64(b)=a⊕b`，2^64=0）。
- Jolt 每周期"成本与指令类型无关"是**靠素域整数嵌入拉平的**——这恰是二元域拿不到的东西。

因此新路线的真实内容不是"复刻 LookupQuery"，而是回答：**在二元域上，整数算术（ADD 是 RV32I 最高频指令）用什么承载？** 这是必须先决策的分叉点（即新 M2，见 §4）。

### 2.2 执行层分叉决策：位级 R1CS vs 词级门

| | 方案 W1：位级（当前切片路线） | 方案 W2：词级（Binius64 frontend 门） |
|---|---|---|
| 机制 | spartan-frontend 位级 R1CS，ADD=32 位全加器链（~64-96 mul/32-bit） | frontend `iadd_32`/`band`/`imul` 词级约束（`designs/binius64-frontend-api-map.md` 已映射），加法固定几个约束，IMUL 3-4×AND |
| 与 thesis 的契合 | 差：ADD 成本 ∝ 位宽，"成本与指令类型无关"不成立 | 好：每指令固定词级门数，恢复 Jolt 式均匀成本 |
| 与现有切片的连续性 | 高（21 个切片全部位级 spartan-frontend） | 低（需换证明框架） |
| 与 logup* 组合 | 已实证（combined 切片同 transcript） | **未验证**：frontend 电路与 logup* 能否同 transcript 组合、frontend 内部查表机制如何暴露——需在 M2 spike 验证 |
| 先例 | 切片 3-8 | Binius64 原生电路（blake3/sha256/ethsign） |

**M2 spike 的判定规则**：用 frontend 词级门做一条 `add` 指令的 prove→verify，并尝试与 logup* 取指查表同 transcript 组合。若组合可行 → 走 W2（词级），位级仅作对照；若不可行 → 退回 W1 并在文档中量化"ADD 成本 ∝ 位宽"对 thesis 的修正。该 spike 同时回答 `designs/binius64-constraint-proofs-and-zkvm-plan.md` 遗留的"frontend 路线 vs spartan 路线"分叉。

---

## 3. 对旧 M3（内存时序论证）的重新评估

### 3.1 原定义的问题

旧 M3 表述为"Jolt 式 one-hot+increment（而非排序器）"。迁移分析（assessment §3）表明这个参照系本身有误：

- Twist **不是** multiset 论证，也**没有版本/时间戳**——它是"已提交 inc 流 + Val 链式构造（prev/next_val）+ LT 加权累加 + Val_init/Val_final/OutputSumcheck 三件套"的函数式累加。
- 把 Twist 忠实翻译到 Binius64 需要**自定义 sumcheck 恒等式**（Val 链、LT），而我们 21 个切片只用过 spartan-prover + logup* 两个现成协议——binius64 `ip` crate 的 sumcheck 机器能否被切片级代码直接驱动自定义恒等式，是**未验证的风险点**。

### 3.2 两条候选机制

- **方案 T1：版本链路线（自有机制的延伸）**。reg_rw 的"写日志表 W[(addr,ver)] + 读==写绑定"在版本链电路化（新 M3 内容）之后**本身就是一个完备的声音内存论证**——不依赖 Twist。它用 logup* + Spartan 两个已实证组件即可闭合，无新协议风险。扩展 K（地址空间）和 init/final 状态检查即得 RAM 版。
- **方案 T2：忠实 Twist 翻译**（one-hot ra + inc + Val + LT 的自定义 sumcheck）。更贴近 Jolt 语义、与 (B) 移植路线兼容，但要先验证 Binius64 sumcheck 机器的可驱动性，风险和工作量都更高。

**决策**：RAM 论证走 **T1 优先**（registers→RAM 推广自有版本链机制），T2 降级为"若 (B) 移植路线启动再做"的备选。理由：目标是"时序做进论证"，T1 用已实证原语即可达成；T2 的价值在于与 Jolt 上游对齐，属 (B) 的范畴。迁移次序确认为 **bytecode（已由切片 9/10 的取指查表覆盖）→ registers（新 M3）→ RAM（新 M4）**，RAM 需额外处理 Val_init/program image/output 三件套。

---

## 4. 统一里程碑序列（权威定义）

| 里程碑 | 内容 | 验收标准 | 状态/依据 |
|---|---|---|---|
| **M1** | 寄存器读==写绑定（写日志表 W[(reg,ver)] + logup*） | reg_rw 切片 + 三种 soundness | ✅ 已完成；遗留：版本链未电路化 → **已在 M3 解决** |
| **M2** | **执行层选型 spike**：frontend 词级门 `add` 的 prove→verify + 与 logup* 取指同 transcript 组合；产出 W1/W2 决策 | 一个词级 add 切片 + 组合可行性结论 + 决策记录 | ✅ 已完成（2026-09-06）：建议 W2 词级；`word_add`/`word_add_combined` 切片 + `M2_REPORT.md` |
| **M3** | **通用单周期状态机**：word 驱动译码 + 跨行寄存器堆（读==写绑定）+ **版本链电路化**（`ver[rd]'=ver[rd]+IsWrite` 进电路——按 M2 的 W2 选型走 frontend 词级门，非 Spartan）+ PC 推进约束，多指令（add/addi/beq）单 transcript | prove→verify 闭环；soundness 覆盖过期读、版本篡改、非法取指 | **✅ 已完成（2026-09-06）**：合并旧 M2（查表化执行）+ M1 遗留 + 架构 §6.1/§6.3；`word_vm` 切片 24，24 passed，4/4 soundness |
| **M4** | **RAM 内存论证（T1）**：M3 机制推广到大地址空间 RAM + 内存初始态/终态（init/final/output）检查 | load/store 经 (addr,ver) 索引写日志表；soundness 覆盖过期读、版本篡改、越界地址 | **✅ 已完成（2026-09-06）**：`word_vm_ram` 切片 25，K=64 字，RAM 读值只由 logup* 钉住（3 表），init/final/output 三件套；25 passed，5/5 soundness（含 2 例分层拒绝）；`M4_REPORT.md` |
| **M5** | 扩 RV32I 子集与字宽到 32-bit，跑更大程序 | 指令覆盖表 + native-vs-proof 对拍 | **✅ 已完成（2026-09-07，v3）**：`word_vm32` 切片 26，30 条真 RV32I（标准编码+符号扩展）+ 32 寄存器（x0 硬零）+ K=64 RAM；torture（50 周期）+ **bubblesort（391 周期，382,660 gates，三件套 8/8）** 均诚实证明通过；修复 is_sra/is_sub 解码 bug 与 S-type store 地址 bug；`M5_REPORT.md` v3 |
| **M6** | 固化库 + 测试套件 + 基准 | 可复用 crate + CI 测试 + 每指令成本基准 | **✅ 已完成（2026-09-07，收官）**：`word_vm32.rs` 拆为 `src/vm32/`（isa/interp/circuit/proof），切片 26 为薄层；`encode.rs` 删除；39/39 测试绿（含 5 条 per-instruction × 边界向量 + 5 条独立 soundness）；每指令成本基准 `BENCHMARKS.md`（31 条指令，30/31 条 g/cyc=973 逐数字相同——thesis 定量证据）；`M6_REPORT.md` |

**明确排除**（属 (B) 移植路线，不进 M1-M6）：uni-skip 替代、batch padding 二元化、Dory→BaseFold 接口、ZK 链重建、忠实 Twist 翻译（T2）。

> **Phase 划分（2026-09-07）**：M1-M6 为 **Phase 1（切片验证阶段）**——证明每类机制
> 在二元域闭环，**不等于完整 zkVM**。完整 zkVM 的目标架构与 Phase 2 路线（M7-M10：
> 可扩展 RAM 论证 spike → 完整 ISA + 真实工具链 → 性能工程 → 工程化收官）见
> `designs/binary-zkvm-full-roadmap.md`。

---

## 5. 文档回填记录（2026-09-06 已执行）

- `research/zkvm-gap-analysis-jolt.md` 旧 M2/M3 段：加勘误指针到本文。
- `architecture.md` §6 路线图：加勘误指针到本文（统一编号）。
- `PROGRESS.md` "下一步"节：加勘误指针到本文。
- `M1_ACCEPTANCE.md` / `M1_REPORT.md` 中"版本链电路化属 M2"的提法：按统一编号改为 **M3**。
