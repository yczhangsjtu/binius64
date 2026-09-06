# 与完整 zkVM 的差距分析（基于 Jolt 源码）

> 2026-09-06 | 依据：Jolt 仓库 `~/workspace/jolt/crates/jolt-prover-legacy/src/zkvm/`
> （指令查表 / R1CS / registers / ram），对照本项目 `crates/zkvm-slice/`。

## 1. 当前已具备（20 切片，单机制可行）

| 机制 | 切片 | 真实程度 |
|---|---|---|
| logup* 二元域查表 | inst_lookup/mem_lookup | ⭐ 真（后端可行） |
| 位级 ALU（全加器链/乘/分支/进位） | pc_carry/instr_step/multi_inst/branch | ⭐ 真 |
| 跨行状态机雏形 | factorial/multi_inst/multi_combined/full_vm* | ⭐ 真（有 `[t+1]` 绑定） |
| 指令 word 解码 | full_vm_jolt | ⭐ 真（word[7:6]→is_addi/is_beq） |
| 双系统组合（Spartan+logup* 同 transcript） | combined/multi_combined | ⭐ 真 |
| "读⊆写"sub-multiset | mem_arg | ⭐ 真雏形 |
| 内存"最近写"（手工填值） | mem_arg_ts/mem_arg_spice/full_vm* | ⚠️ 仅一致性，未证时序 |

## 2. 与完整 zkVM 的差距（Jolt 已实现、我们未实现）

### 差距 ① 寄存器状态流（ReadWriteChecking）——【最核心，缺失】
- **Jolt 做法**（`registers/read_write_checking.rs`）：用一个**读-写矩阵**
  `ReadWriteMatrix`，sumcheck 证明组合恒等式
  ```
  Σ_j eq(r_cycle, j) · ( RdWriteValue(j) + γ·ReadVals(j) ) = rd_wv_claim + γ·rs1_rv_claim + γ²·rs2_rv_claim
  ```
  → 把"本条读 rs1 = 最近一条写 rd 的值"（寄存器"读见最近写"）**做成了论证**。
- **我们现状**：`multi_inst`/`factorial` 只做了**定点寄存器**（x1）的 `[t+1]` 链，
  没有**多寄存器、可寻址、读-写一致的寄存器堆**。`zkvm.rs` 的 `a/b` 是独立注入值，完全无关。

### 差距 ② 指令执行 = 查表（LookupQuery）
- **Jolt 做法**（`instruction/addi.rs`）：每条指令实现 `LookupQuery` trait
  - `to_instruction_inputs()` → `(rs1, imm)`（或 rs1,rs2）
  - `to_lookup_operands()` → 左/右操作数
  - `to_lookup_output()` → `x + y`（查 RangeCheckTable 的结果）
  - `circuit_flags()` → AddOperands / WriteLookupOutputToRD / Load / Store / Branch / Jump
  → **执行 = 一次查表**，由 `circuit_flags` 分发；ALU 不再是硬编码加法器，统一走查表。
- **我们现状**：ALU 是**硬编码进位加法器**（fa 链），`row.op` 用 match 枚举分发，
  **没有** `LookupQuery`/`circuit_flags` 抽象、**没有**查表式执行。

### 差距 ③ 内存时序论证（RAM read-write checking）
- **Jolt 做法**（`ram/`）：`RamInc` one-hot + increment，非全局排序器。我们判断一致。
- **我们现状**：仅 native 手工填表 + logup* 一致性（claim ∈ 表），**未证时序**。
  我们曾判断"排序/时序 sorter（Twist/Shout）从未实现"——与 Jolt 相反，Jolt 用
  increment（one-hot 计数），需要的是**递增论证**而非排序器。这是方向性修正点。

### 差距 ④ 完整指令集 + 译码
- Jolt：RV64IMAC 完整译码（opcode→funct3→imm→rs1/rs2→rd，`to_instruction_inputs` 按 XLEN 分支）。
- 我们：仅 addi/beq 两个简化指令，`full_vm_jolt` 只解 word[7:6] 两个 bit。

### 差距 ⑤ 字宽
- Jolt：RV64（50+ bit 数据路径）。我们：8-bit（教学截断）。

## 3. 差距量化（诚实）

如果把"完整端到端 zkVM"（能证明一条任意 RV32/64 程序的执行 trace）设为 100%：
- 我们目前约 **25-30%**：底层的"二元域查表 / 位级 ALU / 跨行状态 / 双系统组合"已通；
- 缺的 70% 全部是 Jolt 的**核心命题**：寄存器读-写矩阵、指令查表化执行、内存时序论证。

**核心命题（Jolt 的命门）**：zkSNARK 证明成本 ∝ 指令数，靠的是"**每条指令 → 一次
统一周期的查表 + 一次读写矩阵检查**"。我们目前是在做"每条指令 → 一个定制算术电路"，
这在切面上恰好相反——所以注定无法做到"成本只看指令数"。

## 4. 下一步实现计划（选定：寄存器读-写矩阵 + 查表化执行）

**为什么选这个**：差距 ① + ② 是 Jolt "成本 ∝ 指令数"的根基，也是我们与真实 zkVM
最本质的隔阂。先做它，才能让后续累加更多指令时"成本不随指令类型爆炸"。内存时序（③）
是第二步（需要在读写矩阵之上）。

### 里程碑 M1：读写矩阵（registers read-write checking）⭐ 真正实现
- 目标：证明"每条指令读到的 rs1/rs2 = 最近一条写 rd 的值"，**多寄存器、可寻址、读-写一致**，
  用 Jolt 的 ReadWriteMatrix sumcheck 恒等式（见 gap ①）。
- 技术：在 Binius64 上，用我们已有的 **logup\***（sub-multiset）来承载"寄存器读⊆写"：
  - 构造寄存器表 `RegT[reg_index] = 最近写值`（同一寄存器多次写 → 最近写覆盖）；
  - `rd_write` 事件（写 rd）= 表更新；`rs1_read`/`rs2_read` 事件（读 rs1/rs2）= 查表 claim；
  - logup\* 证明"读值 ∈ RegT"且与写序列一致 → 即寄存器"读见最近写"。
- 这一步**替代**当前 `zkvm.rs` 的"a/b 独立注入值"，让它真正读上一条的 rd。
- 交付：新切片 `reg_rw`（或并入真实 zkvm），多寄存器（x1/x2/x5）+ 读-写一致 + soundness 拒假。

### 里程碑 M2：查表化执行（LookupQuery + CircuitFlags）
- 在 M1 基础上，把"addi 执行"从**硬编码进位加法器**改为 **logup\* 查表**：
  `to_lookup_operands→(rs1,imm)`、`to_lookup_output→rs1+imm`，用查表证明结果。
- 引入 `CircuitFlags`（AddOperands/WriteLookupOutputToRD/Load/Store/Branch/Jump）做**通用分发**，
  `row.op` 不再是 match 死的枚举，而是由 word 解码 + flags 决定。
- 交付：真实 zkvm 支持 addi + 任意累加指令，执行=查表 + 读写矩阵。

### 里程碑 M3：内存时序（可选，M1/M2 之后）
- 用 increment（one-hot 计数）而非排序器，做 RAM 读写检查。替换当前手工填表。

## 5. 为什么必须参考 Jolt
不参考 Jolt，我们会继续做"定制算术电路"，永远到不了"成本 ∝ 指令数"。Jolt 的
`R1CSCycleInputs` + `LookupQuery` + `ReadWriteChecking` 才是 zkVM 的正确骨架。
我们已经验证过的（logup*/Spartan/跨行/组合）是它的**地基**，缺的是它**上面的骨架**。
