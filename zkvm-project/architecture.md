# 二元域 zkVM 架构文档（Binius64 fork）

> ⚠️ **诚实勘误（2026-09-05 权威版）**：本仓库代码验证的是**"机制可行"**，但**不构成
> 一个完整的 zkVM**，也**未实现真正的内存论证/通用状态机**。以下为经逐行代码审查后的事实：
>
> - **已真正实现**：logup* 二元域查表（切片 1）；Spartan **跨行状态机**（分支 beq、含循环
>   整数运算 factorial、多指令寄存器流转——切片 7/8/10）；读⊆写 sub-multiset（切片 12）。
> - **未实现（此前表述夸大）**：① 真正的内存论证（"读见最近写"的*时序正确性*——现在只是
>   **手工构造的表 + logup* 一致性检查**，**排序/时序论证（Twist/Shout sorter）从未实现**）；
>   ② **通用状态机**（跨行寄存器/PC 传递、word 真正的 opcode 解码——见 §2.5 对 zkvm.rs 的
>   关键更正）；③ 因此**不是"整合 zkVM"**，而是"逐行验证器 + 手工表"。
> - **纯证据**：切片 2/mem_instr/mem_arg_ts/mem_arg_spice/full_vm_*/zkvm 的"读见最近写"
>   由 **native 程序把正确值直接填进表**，logup* 仅证明*一致*（claim ∈ 表），**不证明*时序*
>   （最近写）。** 真实性分级与逐项证据见 §3 表与 §3.1 里程碑。

> 版本: 2026-09-05 | 状态: 切片实验（机制验证记录），**并无"整合 zkVM"主代码**。上方勘误为权威真相。
> 定位: 本仓库（`github.com/yczhangsjtu/binius64`）作为项目工作目录的**权威架构图景**。
> 它综合既有 plans（`designs/`）与研究成果（`research/`），并叠加当前**已实现 + 已实测**的证据链。

---

## 0. 核心命题（Thesis）

> **在二元域（GF(2^128)）上，zkSNARK 的证明成本只应随"zkVM 执行的指令条数"缩放，
> 与指令类型无关。**

- 位/字节/哈希操作（XOR=加法、AND=乘法、移位=reindexing）在二元域上**成本接近零**。
- 进位整数运算（ADD 链/MUL/DIV）是唯一成本中心，但 Binius64 用**原生词级 IMUL**
  约束（固定 3-4× AND）而非位级展开——成本可控、不随位宽平方爆炸。
- **推论**：不预编译/特化哈希或 bigint；聚焦指令集本身，度量每指令成本。

---

## 1. 技术栈 & 为什么选它

### 1.1 后端：Binius64（fork 自 `binius-zk/binius64`）
- **唯一的成熟二元域证明后端**（活跃，非 Archived）。PetraVM 已 Archived、RISC Zero×Binius 未落地、市场上**无现成 RISC-V 二元域 zkVM**——这正是本项目的 gap。
- **4 种 64-bit 词级约束**：`ZERO`（XOR/线性）、`AND`（按位与）、`IMUL`（64-bit 整数乘法，3-4×AND）、`BMUL`（GF(2^128) 域乘法）。`shifted value index` 免费给移位。
- **免 wiring bug**：`CircuitBuilder` trait 由 `ConstraintBuilder`（抽象）+ `WitnessGenerator`（具体）严格 1:1 实现，杜绝 flock 的抽象层与 witness 层脱节 bug。

### 1.2 内置证明栈（关键：可直接承载 zkVM 子问题）
- **`logup_star`**：通用 indexed lookup `(I*T)[i] = T[index[i]]`——任意表 + 任意 index 列，多表/多 looker 批处理。≈ Jolt 的 Lasso/Shout。
- **`spartan-prover`/`frontend`/`verifier`**：完整 **Spartan**（uniform R1CS `MulConstraint{a,b,c}`）——≈ Jolt 的约束满足层。
- **二元域 PCS = BaseFold（已自带，无需实现）**：`spartan-prover` 直接调用
  `BaseFoldProverCompiler<P, ProverNTT<P::Scalar>>`；`logup*`/Spartan 均在
  `OptimalB128`（GF(2^128)，底层 Ghash128b/Oblong）上运行。**所有切片端到端的
  prove→verify 都经过了 BaseFold 二元域多线性 PCS**——它不是本项目的待办，而是
  Binius64 开箱即用的底层。本节此前的"需自建 PCS"表述不准确，现予澄清。

### 1.3 环境（本机）
- Rust **1.97.1**（`rustup toolchain install 1.97.1`；本机默认 1.95）。
- 构建：`export RUSTFLAGS="-C target-cpu=native"`（i5-12400F：AVX2，无 AVX-512）。
- **OOM 防护**：`CARGO_BUILD_JOBS=4`（12 核 32GB 并行会 SIGKILL）。

---

## 2. 实现架构：Jolt 式三分层在二元域的落地

真正的 zkVM 需要**时序 trace + 程序内存 + 指令解码 + 随机访问内存论证**。Binius64 是电路后端不是 VM，所以这四要素全部自建。本项目采用**与 Jolt 的同构分层**：

| zkVM 模块 | Jolt（素域） | 本项目（二元域） | 切片 |
|---|---|---|---|
| 指令查表 | Shout/Lasso | logup* | 1-2 |
| 约束满足（R1CS） | Spartan | spartan-prover | 3-8 |
| 内存检查 | Twist/Spice | logup*（读⊆写 + 最近写） | 11-13 |
| 组合证明 | 3 份子证明合并 | **同一 transcript 串联** | 9-10 |
| 前端（trace） | tracer/`Cycle` | Jolt 前端 u64 trace 直连 | 14 |

**核心洞见**：Binius64 的 `logup* + spartan-prover` 与 Jolt 的 `Lasso/Shout + Spartan` 概念**一一对应**，且都能在**同一个 Fiat-Shamir transcript** 上组合成单一证明。

---

## 2.5 `zkvm.rs` 的真实状态（关键更正）

> ⚠️ **此文件并非"整合全部机制的 zkVM"，经逐行审查实为"逐行验证器"**：
> - `drive_row` 里 **`let _ = pc;`——PC 根本没被约束**；
> - `row.op` 是 `run_program()`（native）里 **match 死的枚举值**，**不是从 word 解码**；
> - `a`/`b` 是**独立注入值**，**没有跨行寄存器传递**（上一条的 `rd` 不经寄存器堆流入下一条的 `rs1`）；
> - 内存表是 **native 手工填的** `tvec`，logup* 只证明 claim ∈ 表（一致性），**不证明时序/最近写**。
>
> 因此 zkvm.rs **不是状态机**（无 PC 推进约束、无跨行传递），其"整合全部机制"表述**不成立**。
> 它是**逐行验证器 + 手工内存表**，用一个固定小 trace（24 行）跑通 prove→verify + 拒假，
> 但证明的强度**低于切片 8（factorial）**——后者才有真正的跨行状态迁移。`zkvm.rs` 不作为
> 后续递增基线，仅作为"把多块小机制塞进一个文件"的一次性演示。

## 3. 已实现证据链：19 个切片（全部端到端 + soundness 拒假）

`crates/zkvm-slice/`。每个切片 = 最小 prove→verify + 一个"故意篡改被拒"的 soundness 测试。

| # | binary | 机制 | 证明系统 | 关键数据 |
|---|---|---|---|---|
| 1 | `inst_lookup` | 程序指令查表（AND 真值表 2^6） | logup* | 闭环 + 表 claim 闭合 |
| 2 | `mem_lookup` | 内存**一致性查表**（⚠️ 表手工给定，仅证 claim∈表，非真正"latest"） | logup* | 闭环 + 拒假 |
| 3 | `pc_glue` | 寄存器状态流转（3×xori） | Spartan | 8 mul |
| 4 | `pc_carry` | PC 整数进位（8-bit 全加器链） | Spartan | 位分解+`s=a^b^cin`,`cout=(a&b)\|(a&cin)\|(b&cin)` |
| 5 | `instr_step` | 完整单条 `xori` 指令闭环（取指→译码→执行→写回→PC+4） | Spartan | 128 mul |
| 6 | `multi_inst` | 多指令序列 trace（4×xori，寄存器依赖链+PC 续流） | Spartan | 512 mul |
| 7 | `branch` | **条件分支 beq**（位级乘法树相等检测 + 布尔 MUX 条件 PC） | Spartan | 128 mul |
| 8 | `factorial` | **含循环程序 5!=120**（移位加乘法器 + 全加器 + MUX 冻结，**真跨行状态迁移**） | Spartan | 512 mul, prove ~20ms |
| 9 | `combined` | **组合证明**：同一 transcript Spartan+logup* | combined | γ 序依赖 |
| 10 | `multi_combined` | 多指令组合（查表取指 + 约束执行） | combined | 256 mul |
| 11 | `mem_instr` | 内存 load/store + R-A-W 门（⚠️ **单地址硬编码** `mem_r==mem_w`，非通用论证） | Spartan | 128 mul |
| 12 | `mem_arg` | **读⊆写 sub-multiset**（⭐ store+load 同锁一表，**真内存论证雏形**） | logup* | 8 lookers |
| 13 | `mem_arg_ts` | 带时间戳内存论证（⚠️ 表由 trace 手工构造，seq 未证；version 显式给出） | logup* | 3 store+2 load |
| 14 | `jolt_bridge` | **Jolt 前端 trace → 二元域后端**（`JoltTraceRow` u64 契约直连） | logup* | 3 store+2 load |
| 15 | `mem_arg_spice` | SPICE 排序内存论证（⚠️ **表 native 手工构造**，仅证"value ∈ T[ts,addr]"，**未实现 sorter/时序论证**） | logup* | 5 store+2 load, 拒过期值 |
| 16 | `full_vm` | 完整 zkVM 状态机（⚠️ 内核：循环+内存+整数加法，**但内存表手工构造**） | combined | 512 mul, n_private=0 |
| 17 | `full_vm_store` | zkVM + store 写内存（⚠️ "读见最近写"由 native 直接填值，未证时序） | combined | 256 mul, n_private=0 |
| 18 | `full_vm_multi` | zkVM 多地址反复交替读写（⚠️ 同上，"读见最近写"为手工构造，非论证） | combined | 512 mul, n_private=0 |
| 19 | `full_vm_jolt` | JOLT 风格指令执行（⭐ word 驱动 opcode 分发，**但这只是单行约束，跨行无传递**） | combined | 256 mul, n_private=0 |

### 3.1 核心里程碑（诚实分级）
- **切片 8（阶乘）⭐ 真正实现**：三要素（分支+多指令+整数乘法）合一，且有**跨行状态迁移**，
  证明"成本 ∝ 指令数、与类型无关"在**含循环+乘法**的程序上成立。这是最具现实意义的切片。
- **切片 9/10（组合）**: 两个证明系统在**同一个 Fiat-Shamir transcript** 串联，logup* 的 γ
  在 Spartan observe 公共输入**之后**采样 → 查表挑战依赖状态证明。
- **切片 12（读⊆写）⭐ 真内存论证雏形**: 用 logup* 做**读⊆写子多重集合**（store+load 同锁
  一表），证明了"load 值必须 ∈ 已 store 值的集合"。这是真逻辑，但表仍由 native 填。
- **切片 13/15（带时间戳/SPICE）⚠️ 夸大**："读见最近写"的表格由 **native 程序直接把正确值
  填进表**，logup* 只证明 claim ∈ 表（**一致性**），**不证明时序（最近写）**；**排序/时序论证
  （Twist/Shout sorter）从未实现**。此前标为"完整 SPICE 排序论证"**不准确**。
- **切片 14（Jolt 桥接）**: 后端替换的**接口层**参考——Jolt 前端产出的 u64 trace 的结构能喂给
  二元域 logup* 的表。仅证明"数据形状兼容"，非"证明机制等价"。
- **切片 16/17/18（full_vm 系列）⚠️ 夸大**: "读见最近写"为 **native 直接填值**（见跑通输出
  `loads=[...] stores=[...]` 均为 run_program 算出），电路/论证**未证时序**；且 full_vm 本身
  执行语义仍偏"模板化"（循环固定展开、仅少数指令）。
- **切片 19（full_vm_jolt）**: word 驱动 opcode 分发（⭐ 真正的解码思想），**但只是单行约束**——
  `drive_cycle` 内 match `row.op`，**跨行寄存器/PC 传递未实现**，非"真实控制流状态机"。

### 3.2 性能实测（release, i5-12400F）
| 切片 | mul 约束 | prove | verify |
|---|---|---|---|
| `instr_step` | 128 | 3.87 ms | 181 µs |
| `multi_inst` | 512 | 4.31 ms | 248 µs |
| `branch` | 128 | 4.48 ms | 203 µs |
| `factorial` | 512 | 20.3 ms | 416 µs |

**对照原生计算**（Binius64 自带）：blake3 184 bitand/0 imul/5.66ms；sha256 368/0/302µs；
ethsign 113,992/20,538 imul/501ms。→ 阶乘循环的 512 mul 证明与哈希同量级，印证命题。

---

## 4. Jolt ↔ Binius64 转译映射（后端替换可行性）

来源：`workspace/jolt`（clone 自 a16z/jolt，模块化 claims 权威定义）+ `research/jolt-binius-memory-argument-mapping.md`。

**Jolt 的 RAM read/write-checking 关系式（非全局排序器）**：
```
input:  ram_read_value + γ · ram_write_value
output: eq_cycle · ra · ( val + γ·(val + inc) )
```
用 **one-hot addressing（ra: K×T 稀疏矩阵）+ write-increment（inc）+ sumcheck**。
寄存器版本：`eq_cycle·[rd_wa·(inc+val) + γ·rs1_ra·val + γ²·rs2_ra·val]`。

**转译映射**（关键结论：语义同构）：
| Jolt 元素 | Binius64 对应 | 状态 |
|---|---|---|
| `ram_ra`(one-hot 地址) | logup* looker index | ✅ |
| `ram_val`(写前值) | 内存表 store 值 | ✅ |
| `ram_inc`(写增量) | 版本序号 `ver` / R-A-W 门 | ✅ |
| sumcheck γ 折叠 | logup* 多重集合等式 | ✅ 语义同构 |
| `eq_cycle` 绑定 | 表 index 位置编码 | ✅ |
| sumcheck over sparse K×T | logup* 通用查表 | ⚠️ 接口不同 |
| commit (Dory) | Binius PCS | ⚠️ 后端替换点 |

**诚实结论**：
- **Jolt 的 memory-checking 不是全局排序器**，而是 **one-hot + increment**，天然二元友好。
- **语义同构是"数据形状"层面**：Jolt 的读值/写值/地址这些 u64 字段，能塞进我们二元域
  logup* 的 looker（claim ∈ 表）。**但这不等于"证明机制等价"**——我们**没有**实现 Jolt 的
  one-hot+increment 时序论证，也没有实现其读见最近写的 *时序* 约束（见 §3.1）。本节的
  **"已由切片 12/13/14/15 实证过"表述不准确**，应改为"证实了**数据结构/接口可兼容**"。
- **后端替换不需"实现二元域 PCS"**：Binius64 自带 BaseFold 二元域多线性 PCS，所有切片
  都直接在它上面跑。这是真实且成立的（spartan-prover 直接调 `BaseFoldProverCompiler`）。
- **前端可复用**：Jolt 的 `tracer`/`Cycle`/`RAMAccess`/`JoltTraceRow` 全是**域无关 u64**，可直接复用。

> **勘误**：本节此前（及旧版路线图）把"域切换攻坚/实现二元域 PCS"列为最大工程，**这是不准确的**。
> 二元域 PCS 由 BaseFold 自带、sumcheck 由 Binius64 自带，二者都无需自建。经用户指正核实
> （spartan-prover 直接调 `BaseFoldProverCompiler`，logup* 在 `OptimalB128` 上运行），
> 已更正为：**不需要域切换，真正的工作是工程整合**（见 §6）。

---

## 5. 复用之别（Jolt → Binius64）

| 层 | 能否复用 Jolt | 处置 |
|---|---|---|
| executor / tracer / `Cycle` / `RAMAccess` / `JoltTraceRow` | ✅ 域无关 | **保留** |
| `MemoryLayout` / `remap_address` | ✅ 域无关 | 保留 |
| memory-checking 数据组织 (address_major) | ✅ 值域无关 | 保留/翻译 |
| **证明后端 (sumcheck/PCS/multiset)** | ❌ 素域耦合 | **替换为二元域**（logup* + 我们的论证） |

---

## 6. 下一步路线图

> **勘误**：旧版把"域切换攻坚（Jolt sumcheck → 二元域 PCS）"列为最大工程——**这是错误的**。
> 二元域 PCS = BaseFold（Binius64 自带），sumcheck 也由 Binius64 自带，二者都**无需自建**。
> 前后端是同一栈（logup*/spartan 都在 BaseFold 上）。真正的工作是**工程整合**，不是域切换。

> **整合状态（诚实更正）**：**并无"整合 zkVM"**。切片 16-19（full_vm 系列/zkvm.rs）是把
> 多块机制放进一个文件的一次性**演示**，但**无跨行状态机、内存论证时序未证**（见 §2.5/§3.1）。
> 真正有价值的基点是切片 8（factorial）的**跨行状态迁移**。以下为**真正需要做**的方向。

1. **真正的通用状态机**：跨行寄存器传递（上一条 `rd` → 下一条 `rs1`）+ PC 推进约束
   （`pc_next = branch ? target : pc+4`）——这是"程序执行"的命门，目前**未实现**。
2. **真正的内存论证**：把"读见最近写"的**时序**做成论证（排序/时序 sorter 或 Jolt 式
   one-hot+increment），而非手工填表 + logup* 一致性检查。
3. **word 真正驱动解码**：opcode 位 → 决定执行路径，且与跨行状态机连接（而非 `match row.op`）。
4. **扩 RV32I 子集与字宽到 32-bit**，trace 用 isasim.rs，跑更大程序。
5. **固化库 + 测试套件**：把真实机制（factorial 级跨行状态机）做成可复用 crate + 测试基准。

---

## 7. 关键 API 教训（硬编码到 skill）

- **logup***：`TableLookup{table, lookers}`；prover=`binius_ip_prover::logup_star::prove`，verifier=`binius_ip::logup_star::verify_reduction`；γ 用 `IPProverChannel::sample(&mut transcript)`。
- **Spartan**：`write_inout` 顺序必须与约束侧 allocation 顺序一致（layout inout 排列 = allocation 顺序）；分组布局 `[pc|reg|pcn|regn|inst]`。
- **多表 logup***：`prove/verify_reduction` 接受 `[TableLookup; N]`，每表独立 `n_vars`；主证+negative 都要 `.clone()` 表 view + lookers（否则 E0382）。
- **soundness 通用铁律**：篡改 witness public 段派生值（不重新 drive、不破坏 decode），已在组合证明中用"篡改 logup* claim 为表外值"。
- **借用冲突**：`b.constant(...)` 提为局部变量再传。

---

## 8. 文件索引

| 切片 | 机制 | 证明系统 |
|---|---|---|
| 1 `inst_lookup` | 指令查表 | logup* |
| 2 `mem_lookup` | 内存一致性查表 | logup* |
| 3-8 | PC 进位/单指令/多指令/分支/阶乘 | Spartan |
| 9-10 | 组合证明/多指令组合 | combined |
| 11 | 内存 store/load | Spartan |
| 12-13 | 读⊆写/最近写判别 | logup* |
| 14 | Jolt 前端桥接 | logup* |
| 15 | 完整 SPICE 排序内存论证 | logup* |
| 16 | full_vm 完整 zkVM 状态机 | combined |
| 17 | full_vm_store 完整 zkVM+store(读-改-写循环) | combined |
| 18 | full_vm_multi 多地址反复交替读写 | combined |
| 19 | full_vm_jolt Jolt 风格指令执行(word 驱动·单行约束·无跨行) | combined |

```
binius64/                       ← 项目根（fork 工作目录）
├── crates/zkvm-slice/          ← 19 切片权威代码（+ README, Cargo.toml）
└── zkvm-project/
    ├── README.md               ← 项目总览 + 19 切片速查表
    ├── PROGRESS.md             ← 构建顺序 / 进展 / 下一步
    ├── architecture.md         ← 本文档（勘误后）
    ├── HANDOFF_M-A2.md
    ├── designs/                ← 计划/决策（baseline-zkvm-design, constraint-proofs-and-zkvm-plan, ...）
    └── research/               ← 研究（jolt-binius-memory-argument-mapping, zkvm-backend-replacement-feasibility, ...）
```

> 对照：`workspace/jolt`（外部参考实现，未入本仓库）；`workspace/binaryfield-zkvm`（flock 时代的独立 crate，已废弃，仅留 isasim.rs 参考）。
