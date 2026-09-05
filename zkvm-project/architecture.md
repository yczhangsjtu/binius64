# 二元域 zkVM 架构文档（Binius64 fork）

> 版本: 2026-09-05 | 状态: 完整 zkVM 雏形（18 切片端到端验证，含多地址交替读写）
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

## 3. 已实现证据链：15 个切片（全部端到端 + soundness 拒假）

`crates/zkvm-slice/`。每个切片 = 最小 prove→verify + 一个"故意篡改被拒"的 soundness 测试。

| # | binary | 机制 | 证明系统 | 关键数据 |
|---|---|---|---|---|
| 1 | `inst_lookup` | 程序指令查表（AND 真值表 2^6） | logup* | 闭环 + 表 claim 闭合 |
| 2 | `mem_lookup` | 内存一致性 load 查表（2^3 地址） | logup* | 闭环 + 拒假 |
| 3 | `pc_glue` | 寄存器状态流转（3×xori） | Spartan | 8 mul |
| 4 | `pc_carry` | PC 整数进位（8-bit 全加器链） | Spartan | 位分解+`s=a^b^cin`,`cout=(a&b)\|(a&cin)\|(b&cin)` |
| 5 | `instr_step` | 完整单条 `xori` 指令闭环（取指→译码→执行→写回→PC+4） | Spartan | 128 mul |
| 6 | `multi_inst` | 多指令序列 trace（4×xori，寄存器依赖链+PC 续流） | Spartan | 512 mul |
| 7 | `branch` | **条件分支 beq**（位级乘法树相等检测 + 布尔 MUX 条件 PC） | Spartan | 128 mul |
| 8 | `factorial` | **含循环程序 5!=120**（移位加乘法器 + 全加器 + MUX 冻结） | Spartan | 512 mul, prove ~20ms |
| 9 | `combined` | **组合证明**：同一 transcript Spartan+logup* | combined | γ 序依赖 |
| 10 | `multi_combined` | 多指令组合（查表取指 + 约束执行） | combined | 256 mul |
| 11 | `mem_instr` | 内存 load/store + R-A-W 门（`mem_r==mem_w`） | Spartan | 128 mul |
| 12 | `mem_arg` | **内存论证**（多地址读⊆写 sub-multiset，store+load 同锁一表） | logup* | 8 lookers |
| 13 | `mem_arg_ts` | **带时间戳内存论证**（同址多写·最近写判别，两表拆分） | logup* | 3 store+2 load |
| 14 | `jolt_bridge` | **Jolt 前端 trace → 二元域后端**（`JoltTraceRow` u64 契约直连） | logup* | 3 store+2 load |
| 15 | `mem_arg_spice` | **完整 SPICE 排序内存论证**（任意次写·全局时间戳·时间排序状态表） | logup* | 5 store+2 load, 拒过期值 |
| 16 | `full_vm` | **完整 zkVM 状态机**（三机制整合：循环+内存+整数加法，同一 transcript） | combined | 512 mul, n_private=0 |
| 17 | `full_vm_store` | **zkVM + store 写内存**（读-改-写循环，读见最近写） | combined | 256 mul, n_private=0 |
| 18 | `full_vm_multi` | **zkVM 多地址反复交替读写**（读见最近写） | combined | 512 mul, n_private=0 |

### 3.1 核心里程碑
- **切片 8（阶乘）**：三要素（分支+多指令+整数乘法）合一，证明"成本 ∝ 指令数、与类型无关"在**含循环+乘法**的程序上成立。
- **切片 9/10（组合）**：两个证明系统在**同一个 Fiat-Shamir transcript** 串联，logup* 的 γ 在 Spartan observe 公共输入**之后**采样 → 查表挑战依赖状态证明。
- **切片 12/13/15（内存论证）**：zkVM **最难一环**。用 logup* 做**读⊆写子多重集合** + **最近写判别** + **完整 SPICE 排序**（任意次写·全局时间戳·时间排序状态表），避免 O(N·M) selectors 平方爆炸。
- **切片 14（Jolt 桥接）**：后端替换的**接口层实锤**——Jolt 前端产出的 u64 trace 直接喂给二元域论证。
- **切片 16（full_vm）**：**工程整合达成**——一台状态机同时证明"循环+内存+整数加法"
  （Spartan 状态机 + logup* 程序内存取指 + logup* 数据内存论证，同一 transcript，
  512 mul，n_private=0）。= 第一台证明含内存访问循环程序的二元域 zkVM。
- **切片 17（full_vm_store）**：**加 store 写内存**——读-改-写循环（load+store），
  时间排序表证明"读见最近写"（第2轮 load 读上轮 store 写的值，读旧值→拒），256 mul。
- **切片 18（full_vm_multi）**：**多地址反复交替读写**——两地址(addr=i&1)交替读-改-写，
  每地址多次写（地址0:轮0/2写，地址1:轮1/3写），load 读最近写（轮2读2非初始1；轮3读7非初始5），
  读初始值→拒，512 mul。= 最强内存论证测试（读见最近写 across 多地址）。

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
- 它与我们的二元域 logup* 内存论证**证明同一命题**（load 读到的值 == 该地址最近一次 store 的值）。
- **后端替换不需"实现二元域 PCS"**：Binius64 自带 BaseFold 二元域多线性 PCS，所有切片
  都直接在它上面跑。真正要做的不是"造 PCS/重写 sumcheck 到 char-2"（PCS 与 sumcheck 都是
  Binius64 现成的），而是**把 Jolt 的 sumcheck 关系式语义**（one-hot+increment）映射到
  Binius64 已有的 logup*/spartan 结构上——这已由切片 12/13/14/15 实证过。
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

> **整合状态**：工程整合已完成（切片 16 `full_vm`）——一台状态机同时证明"循环+内存+
> 整数加法"。以下仍为后续可扩展方向。

1. **扩 RV32I 子集与字宽到 32-bit**（andi/slli/...），trace 用 isasim.rs，跑更大程序。
2. **指令解码泛化**：full_vm 目前硬编码 opcode（load/addi 语义），泛化到任意 RV32I 指令。
3. **循环边界变量化**：full_vm 展开固定轮数，改为循环边界由 trace 决定。
4. **固化库 + 测试套件**：把 16 切片做成可复用 crate + native-vs-proof 交叉核对基准。

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

```
binius64/                       ← 项目根（fork 工作目录）
├── crates/zkvm-slice/          ← 15 切片权威代码（+ README, Cargo.toml）
└── zkvm-project/
    ├── README.md               ← 项目总览 + 15 切片速查表
    ├── PROGRESS.md             ← 构建顺序 / 进展 / 下一步
    ├── architecture.md         ← 本文档（勘误后）
    ├── HANDOFF_M-A2.md
    ├── designs/                ← 计划/决策（baseline-zkvm-design, constraint-proofs-and-zkvm-plan, ...）
    └── research/               ← 研究（jolt-binius-memory-argument-mapping, zkvm-backend-replacement-feasibility, ...）
```

> 对照：`workspace/jolt`（外部参考实现，未入本仓库）；`workspace/binaryfield-zkvm`（flock 时代的独立 crate，已废弃，仅留 isasim.rs 参考）。
