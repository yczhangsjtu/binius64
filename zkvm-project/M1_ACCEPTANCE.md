# M1 验收报告：register read-write matrix（宣称 vs 实际实现一致性核查）

> 验收 Agent | 日期：2026-09-06 | 基准：`ACCEPTANCE_BASIS.md`（"读见最近写"的时序论证是硬边界）
> 结论：**代码能跑通、能拒假（真实），但 M1 宣称的"⭐ 真实实现 / 非 native 手工填值 / 读见最近写 / Jolt ReadWriteChecking"
> 与实际实现**不一致**——它是 mem_arg_spice 级（⚠️）的"native 时间序表 + logup* 一致性"，**未把困难部分（时序/排序）做进约束**。
> 验收目标：指出不一致 + 给出把代码改造成**真正实现**的技术规范。**不得只调评级。**

---

## 1. 宣称 → 实际实现 逐项对照（证据来自 `crates/zkvm-slice/src/slices/reg_rw.rs`）

| # | 宣称（M1_REPORT.md） | 实际实现（代码证据） | 判定 |
|---|---|---|---|
| 1 | §5.1 "**⭐ 真实实现**…由 logup\* sub-multiset 论证强制（**非 native 手工填值**）" | 表 T 由 L131-143 **native 循环构造**：`current[a.reg]=a.val` 后逐 ts 写入 `t[ts*NREG+reg]` | ❌ **表确实是 native 手工填**；"非 native 手工填值"与代码直接矛盾 |
| 2 | §2/§3 "每条指令读到寄存器值 = 最近一次写该寄存器的值（读见最近写）" | "最近写"由 **native 覆盖 `current[reg]=val` 决定**（L137）；logup\* 仅证明 claim ∈ 表（L150-187） | ❌ **"读见最近写"是 native 预计算属性，未被证明**——只证一致性，不证时序 |
| 3 | §1 "**朝 Jolt 风格的第一步**——Jolt 的 registers/read_write_checking.rs 正是用读-写矩阵做寄存器一致性" | Jolt 的 ReadWriteChecking 用 **one-hot 寻址 + write-increment(ver) + sumcheck** 在论证内证时序；此处**并未**实现该机制 | ❌ 机制**不等价**——此实现是 mem_arg_spice 时间序表，非 Jolt 的 ReadWriteChecking |
| 4 | §1 "**补上 zkvm.rs 的独立注入弱点**" | 确比 zkvm.rs 强——读数绑定到寄存器表（消除自由注入） | ✅ **部分属实**（一致性绑定成立） |
| 5 | §5.4 边界 "未做寄存器值→ALU 结果绑定 / 未接入状态机" | 确认：无 rs1/rs2/rd 解码、无寄存器堆语义、无 ALU 自洽、无跨指令 rd→rs1 传递 | ✅ 边界已如实标注，但未能抵消 1-3 的夸大 |
| 6 | §2 "**logup\* 把所有读/写作为 lookers 绑定到同一张表**" | 属实（L150-159 全部 looker 绑到 `t_view`） | ✅ 但这是"一致性 sub-multiset"，非 mem_arg 式"读⊆写集合"，也非时序证明 |

---

## 2. 关键不一致（本质）

**宣称的核心命题是"寄存器读见最近写"被论证证明；实际只证明了"每个读/写值 ∈(native 算好的)时间序表一致"。**

`reg_rw` 与 `mem_arg_spice`（基线标 **⚠️**）是**同一技术**：
- 二者都用 native 循环建**时间序状态表**（`T[ts*ADDR+addr]` vs `T[ts*NREG+reg]`）；
- 二者都用 logup\* 只证明 **claim ∈ 表（一致性）**；
- 二者都**未把排序/时序论证做进约束**（无 sorter、无版本单调约束、无"读值=该版本写值"的电路绑定）。

因此 M1 与 `mem_arg_spice` 同类，**应判 ⚠️（演示/边界）**，而非 ⭐。三处文档 `M1_REPORT §5.1`、`architecture.md §3表 L112`、`PROGRESS.md §M1` 统一误标 ⭐。

---

## 3. 要成为"真正实现"，必须做进约束的硬部分（当前缺失）

真正证明"读见最近写"，必须**在电路/论证内**建立时序，而非 native 预计算快照：

**A. 版本单调（write-increment）**
每个寄存器带**版本 wire `ver[reg]`**（该寄存器已写次数）。状态机约束：
- `write rd`：`ver[rd]' = ver[rd] + 1`；`S[rd]' = new_val`（寄存器堆更新）。
- `read rs`：`ver[rs]' = ver[rs]`（读不改版本）；值取自 `S[rs]`。
版本序列 = 执行序（由状态机串联），**替代**当前随意赋的 native `ts`。

**B. 读值 ≡ 该版本写值（read==write binding，关键）**
每个读必须绑定到**该寄存器当前版本**那一次写：
- 构造**写日志表 `W[(reg, ver)] -> value`**，每个**写事件**是 W 的 looker（复现 mem_arg 的"表由写建立"）。
- 每个**读事件**是 W 的 looker，**index = (reg, ver[reg])**，**claim = read_value**。
- 于是电路强制 **read_value == W[(reg, ver[reg])]** = 最近一次写该寄存器的值。这才是"读见最近写"的**论证**。

**C. 一致性 + 拒假（soundness 必须更强）**
- 篡改读值为**另一版本的值**（非当前 ver 的写值）→ 拒绝。
- 篡改 **version wire**（如读时误用旧 ver）→ 拒绝。
- 篡改读值为**从未写过的值** → 拒绝。
（当前 reg_rw 只对"表外值"拒假，未覆盖"表内但错误版本"——这正是 true 实现要补的。）

**D. （新 M3 方向，编号见 `designs/milestone-roadmap.md`）** 与真实状态机融合：rd→rs1 跨指令寄存器传递、ALU 结果写回 rd、word 译码 opcode。当前 M1 完全独立，未接。

> **诚实口径**：A+B+C 用 **logup\* + 版本串联**（Jolt↔Binius64 §4 已证"one-hot+increment 与 logup\* 语义同构"，**无需**全局 sorter）即可实现"读见最近写"的论证。这是 documentation 语境下的"真正实现"，且是**可落地**的（不需要排序器）。

---

## 4. 需要修改的东西

**代码（核心，目标）**：
1. 重构/新增 `reg_rw` 切片，用 **写日志表 W[(reg,ver)]** 替代 native 时间序表 `T[ts*NREG+reg]`。
2. 增加**版本 wire** 并在状态机内串联（write→+1，read→不变）。
3. 每个读 looker 用 `(reg, ver[reg])` 索引 → 读值 == 该版本写值。
4. 至少补 3 类 soundness 拒假（错误版本/篡改版本/从未写过）。
5. （进阶，新 M3，编号见 `designs/milestone-roadmap.md`）接入 ALU + rd→rs1 跨指令传递 + word 解码。

**文档（如实反映，直到代码真实现）**：
- `M1_REPORT.md` §5.1：⭐ → **⚠️**；删"非 native 手工填值"，改述为"**native 时间序表 + logup\* 一致性**（mem_arg_spice 级）；时序/排序未做进约束，待重构"。
- `architecture.md` §3表 L112 与 `PROGRESS.md` §M1：`⭐ 真` → `⚠️`，注明"一致性论证；真实时序论证待版本次级（新 M3-M4，编号见 `designs/milestone-roadmap.md`）"。

---

## 5. 复验标准（真正的实现需满足）

- `cargo test -p binius-zkvm-slice` 全过（含新 soundness）。
- 不再存在"**native 时间序预计算快照**"作为"读见最近写"的来源；改由**写日志表 + 版本串联 + 读==写绑定**承载。
- 篡改**表内但错误版本**的读值与篡改 **version wire** 均被拒（当前只拒表外值，不够）。
- 文档分级与实现程度一致（未真正实现前为 ⚠️）。
