# 二元域 zkVM 详细设计（Phase 2 设计详案）

> 日期：2026-09-07 | 地位：Phase 2（M7-M10）的**权威设计文档**，细化
> `designs/binary-zkvm-full-roadmap.md`。
> 依据：Jolt 五路代码调研（域耦合/查表/内存/后端/tracer）、Binius64 协议构件调研
> （logup*/fracaddcheck/quadratic mlecheck/oracle 通道）、M1-M5 切片实证。
> 本文回答三件事：**①哪些协议直接参考 Jolt；②二元域切换怎么做；③现有切片代码怎么接入。**
> 设计决策全部在此给出，Worker 只负责按图施工。

---

## 1. 总架构：证明的组装形态

一个程序执行的完整证明 = **单一 Fiat-Shamir transcript** 上串联四层：

```
层 0  trace        riscv32 工具链 → ELF → tracer → 周期 trace（u32 值，域无关）
层 1  执行电路      每周期统一的前端词级门电路（M4 prover，W2 路线）
                   译码 + ALU + PC 链 + 32 寄存器值链/版本链 + 事件钉扎
层 2  查表/论证     取指表（indexed logup*）+ 寄存器写日志 + RAM 论证（M7 选型）
层 3  承诺与开口    BaseFold PCS；派生列经 IOP channel send_oracle 承诺，
                   叶子 claim 经 prove_oracle_relation / batch_eval 归约
```

**与 Jolt 的形态对照**：Jolt = Spartan outer（每周期 R1CS）+ Shout（指令查表）+
Twist（内存）+ Dory 开口。我们 = 前端词级门电路（M4 prover）+ indexed logup*（取指）+
M7 内存论证 + BaseFold 开口。**形态一一对应，但每一层用的协议不同**（见 §2）。

---

## 2. 逐协议设计：参考 Jolt 什么 / 二元域怎么换 / 切片接哪块

### 2.1 指令执行层

| | Jolt | 本项目 | 切片依据 |
|---|---|---|---|
| 机制 | Spartan R1CS（38 var/22 约束）+ Shout 查表求指令语义 | **前端词级门电路**（译码门 + iadd_32/imul/icmp_* 门） | M2 选型：词级 add = 1 AND+1 ZERO，Jolt 的查表化执行动机（成本均匀）由词级门达成 |
| 参考点 | "每周期统一约束"的架构、CircuitFlags 的 flag 思想 | 已实现为译码门 + is_* 信号（M5） | M3/M5 |
| 二元域切换 | combined-operand trick（x+y 作查表索引）在 char-2 失效 | **不用**：整数加法走 iadd_32 门（进位在门内），不经域嵌入 | M2 已证 |

**设计决策 D1**：执行层**不移植** Jolt 的 Shout 查表语义，用 W2 词级门。Jolt 的指令查表
在我们这里只保留**取指**（程序镜像查表）一处。理由：M2 实测词级门已恢复
"成本∝指令数"；Shout 的 2^128 大表 + prefix-suffix 是素域整数嵌入的配套物，搬过来
反而要重建进位逻辑。

### 2.2 译码与 flags

- 参考 Jolt：`CircuitFlags`（14 个 opflag 驱动 R1CS）+ `InstructionFlags`（witness 路由）
  的两层划分（`jolt-riscv/src/flags.rs:24-97`）。
- 我们的对应物：M5 的 is_* 译码信号（门内 icmp_eq + funct3/funct7 判别）。
- **设计决策 D2**：M8 扩指令时，每条指令登记一行"flag 行"（写回？/读内存？/写内存？/
  分支？/跳转？/操作数来源），电路按 flag 组合——避免 30+ 指令的 if-else 爆炸。
  这是纯工程模式借鉴，无协议内容。

### 2.3 取指（程序内存）

- 参考 Jolt：bytecode read-RAF（`bytecode/read_raf_checking.rs:86-148`，只读、无 inc，
  PC one-hot 分块）。
- 我们：indexed logup* 取指表 T[pc]=word（M2/M3 已验证，与 Jolt read-RAF 语义最接近）。
- **M8 升级**：程序镜像从"双方共享 native 表"改为 **committed 表 + 公开程序哈希**：
  verifier 收哈希作公共输入，表内容经 oracle 开口绑定。切片现有模式（native 共享表）
  在此止步。

### 2.4 寄存器读写

- 参考 Jolt：registers read_write_checking（K=128，rd_wa/rs1_ra/rs2_ra one-hot +
  inc，`relations/registers/read_write_checking.rs:95-115`）。
- 我们：**保留 M1/M3 的版本链 + 写日志表方案**。K=32 时电路内版本链仅 32 计数器/周期，
  成本可接受（M5 实测含在 895 gate/周期内），不需要 Twist 的稀疏机器。
- **设计决策 D3**：寄存器不走 M7 的可扩展论证——K=32 太小，电路化就是最优。
  可扩展论证只用于 RAM。

### 2.5 RAM 论证（核心，M7 详设见 §3）

- 参考 Jolt：Twist 的 **one-hot + committed inc + Val 链 + LT**（这是路线 B 的蓝本）。
- 我们：M4 已验证机制正确性（写日志 + 版本链 + 三件套），M7 解决复杂度
  （O(K·T) → O(T log T)）。

### 2.6 除法/余数（M8）

- **直接借鉴 Jolt 的虚拟指令展开序列**（`jolt-program/src/expand/division/div.rs:4-41`）：
  advice 商 → assert_valid_div0 → negate_if×2（取绝对值）→ xor（符号）→
  assert_mulu_no_overflow → mul → assert_lte → sub → assert_valid_unsigned_remainder。
- 二元域切换：断言表用 indexed logup* 或词级门实现；`negate_if` 在 char-2 不能写成
  `v − 2·sign·v`（Jolt 的素域式），改写为 `v ⊕ (sign_mask) + sign`（词级门：
  mask = 0−sign，select + iadd_32）。
- mul 本身：frontend `imul` 门原生（3-4×AND）——**不采用** Jolt 的
  RangeCheck/UpperWord 大表方案（那是素域查表路线的产物）。

### 2.7 字节/半字访存（M8）

- **直接借鉴 Jolt 的展开层方案**：电路里 RAM 永远字粒度（我们 32-bit 字）；
  lb/lh/sb/sh 在 trace 时展开为"对齐字访问 + 窗口掩码 + 移位提取"序列
  （Jolt 用 VirtualWindowMask/Pext 查表，我们用词级门：band/srl32/select 原生表达）。
- 对齐断言： lh/sh 半字对齐、lw/sw 字对齐（比较门）。

### 2.8 tracer / 内存布局 / I-O（M8）

- 参考 Jolt 的布局约定（`common/src/jolt_device.rs`）：I/O 区在低地址
  （input/output/panic/termination），程序区在 RAM_START 之上，stack 向下/heap 向上；
  地址 remap 为 dense 字索引（`(addr − lowest) / 4`，我们 4 字节字）。
- tracer 自写：M5 的 `vm32/interp` 解释器扩展为 ELF 加载 + 全指令 + 每周期 trace
  （字段对齐 M5 Cycle：pc/inst/寄存器读写/内存读写/版本快照）。
- I/O：input/output 区是公共初始/终态内存的一部分，走 M4 三件套检查。

### 2.9 claims 纪律

- 参考 Jolt：jolt-claims 的符号化 claim 记账（25 个 relation 单源声明）。
- 我们规模小（<10 个 claim 族），**不引入 derive 宏系统**；保留文档化的
  `claims_from_inout` 纪律（M3 v2），M7 起升级为"committed 列 + 归约"（见 §4）。

---

## 3. M7 详设：可扩展 RAM 论证（Phase 2 咽喉）

目标：K（地址空间）与 T（周期数）解耦，成本 O(T log T)，与 K 无关。
两条候选路线，M7 spike 各做切片后选型。

### 3.1 路线 A：排序式离线内存检查（**默认倾向**）

**构造**（全部构件已确认存在于 Binius64）：

1. **事件流**（执行序）：每条 lw/sw 一条记录 `(addr, ts, val, is_write)`，
   ts = 全局周期号。由执行电路钉扎（M3 v2 事件钉扎模式）。
2. **排序流**（prover 提供，committed 列）：同样记录按 `(addr, ts)` 排序，
   外加 **init 记录**（每触及地址一条，ts=0，val=初始值，kind=init）与
   **final 记录**（每触及地址一条，ts=∞，kind=final）。
3. **恒等式①（多重集合相等）**：排序流 == init ∪ 事件流 ∪ final。
   指纹 `f(addr,val,ts,kind) = addr + ρ·val + ρ²·ts + ρ³·kind`（ρ 为 transcript 挑战），
   用 **fracaddcheck 直接组装真 logUp 分数和**：`Σ 1/(c − f(排序流)) − Σ 1/(c − f(事件侧)) = 0`。
   - ⚠️ **不能用现成 `logup_star::prove`**：其分子固定为 `γ^j·eq_r`、分母是
     `c−位置`（indexed-lookup 形态，证明的是"位置直方图"而非"值多重集合"）。
     正确做法：`FracAddCircuit::build` 接受任意分子/分母列（分子=全 1 透明列，
     分母=`c − f(列)`），叶子 claim 归约为对 committed 列的单点求值。
     需自写外层组装（witness 构造 + 叶子 claim 导出），核心 GKR 全复用。
4. **恒等式②（排序良构 + 读一致性）**：排序流相邻对 (S_j, S_{j+1})：
   (a) addr 非降；(b) 同 addr ⇒ ts 严格增；(c) 同 addr 且 S_{j+1} 为读/终态 ⇒
   `val_{j+1} == val_j`（读见同地址最新条目的值）；(d) init 是每地址首条。
   - 逐对检查 O(T)，可用前端电路（词级门：比较+select）或
     `quadratic_mlecheck_prover`（任意二次复合式，claim=0 即 zerocheck）。
   - addr/ts 的"非降/严格增"比较：ts 差值断言在 [1, 2^32) 用位分解或
     logup* range check（二元域原生强项）。
5. **三件套**：init 记录对照公共初始镜像；final 记录供 output 检查
   （M4 三件套语义自然落入此结构）。

**为什么这是对的**：多重集合等式保证排序流不多不少正是那些访问；相邻约束保证
同地址内按 ts 序，读必复制前一条（最近写）的值。时序正确性由"排序+相邻一致"承担，
无需版本链电路。

**成本**：列长 O(T)；承诺 O(T log T)（BaseFold 打包）；检查 O(T)。**与 K 无关**。

### 3.2 路线 B：Twist 忠实翻译（备选）

- ra(k,j) one-hot 分块承诺（GF(2) 元素打包，BaseFold 原生优势）；
- committed inc，**inc := val ⊕ post**（char-2 下 `val + inc = post` 自动成立——
  Jolt 的 i128 域减法技巧不需要了，这是二元域反而更干净的一点）；
- read-write checking sumcheck：`eq_cycle·ra·(val + γ(val+inc))`（degree 3）；
- val evaluation：LT 多项式加权 inc 累加（LT 在二元域成立）+ init/final/output 三角。
- **风险**：degree-3 自定义复合式**无现成 evaluator**，需自写 `MleCheckRoundEvaluator`
  （三次复合不能走 quadratic_mlecheck_prover）；Jolt 的 Gruen 优化/one-hot MSM 优化
  不可移植。工作量与协议风险都高于路线 A。

### 3.3 M7 spike 决策标准

各做 K=2^16 级切片，测 gates/prove 时间随 K、T 的缩放。**默认预期路线 A 胜**
（全现成件、无新协议风险）；路线 B 仅在渐近优势明显且自定义 sumcheck 被证明
可驱动时选。两切片都保留为证据。

---

## 4. 切片代码接入方案（现有资产 → 最终架构的映射）

| 现有资产 | 去向 | 说明 |
|---|---|---|
| `vm32/isa.rs`（M6，真 RV32I 编码） | **保留** → M8 的 tracer/电路共用 | 30 指令编码已验收 |
| `vm32/interp.rs`（M6，native 解释器） | **演进为 tracer**（M8：+ELF 加载 +trace 记录） | 对拍基准 |
| `vm32/circuit.rs`（M6，每周期电路） | **保留为执行核心** | M7 起 RAM 版本链部分被路线 A/B 取代；寄存器部分不动 |
| `vm32/proof.rs` + `claims_from_inout`（M6） | **演进**：inout 重建 claim 模式在 M7 升级为 committed 列 | 见下 |
| 切片 1-23（M1 前-M4） | **冻结为回归测试** | 不再演进 |
| `word_vm`/`word_vm_ram`/`word_vm32`（24-26） | **保留为集成测试** | 机制档案 |
| `BENCHMARKS.md`（M6） | 演进到 M9 的缩放曲线 | |

### 4.1 关键演进：inout 绑定 → committed 列绑定（M7 必做）

M3 v2 的"事件 inout + claims_from_inout"纪律在切片规模正确，但 **inout 是公开输入，
O(T) 的公开输入让 verifier 线性读入**——完整 VM 不可接受。升级路径（Binius64 已
备齐构件）：
- 事件列（addr/ts/val/flag）作为 witness 的一部分，经 **IOP channel `send_oracle`**
  成为 committed oracle（参照 `crates/iop-prover/src/logup_star.rs:95-100` 的
  pushforward 承诺范例和 intmul phase5 的 index-claim 绑定模式）；
- logup*/fracaddcheck 的叶子 claim 经 `prove_oracle_relation` / `batch_eval`
  归约到这些 oracle 的单点开口；
- 执行电路与事件列的一致性由"事件列本身就是电路 wire 的打包"保证
  （绑定胶水：mlecheck 关联 trace oracle 与派生列，或让电路直接读写派生列区域）。
- **这一升级是 M7 的前置任务**（路线 A/B 都建立在 committed 列上）。

### 4.2 crate 结构演进

- M7/M8：继续在 `crates/zkvm-slice` 内（机制验证，切片惯例）。
- M9/M10：产品化为独立 crate（`crates/zkvm`），切片库留作回归测试。
  目标 API：`prove(program_elf, input) -> Proof`、`verify(proof, program_hash, io)`。

---

## 5. M7-M10 任务细化

### M7：可扩展 RAM 论证 spike（唯一的新协议工作）
- T0（前置）：committed 列绑定升级（§4.1）——把 M4 的 RAM 事件从 inout 改为
  committed 列 + oracle 开口，重跑 M4 测试套。
- T1：路线 A 切片（K=2^16，T≥2^12）：fracaddcheck 多重集合 + 排序流相邻一致性。
- T2：路线 B 切片（同规模）：验证自定义 sumcheck 可驱动性（这是 B 的生死题）。
- T3：缩放曲线实测（K、T 各扫 4 个点）+ 选型报告。
- 验收：两路线数据齐全、选型理由明确；A 线必须闭合（B 允许阴性结论）。

### M8：完整 ISA + 真实工具链
- T1：mul（imul 门）+ div/rem（Jolt 展开序列翻译）+ 字节访存（展开层）。
- T2：ELF 加载 + tracer（vm32/interp 演进）+ 内存布局（§2.8）。
- T3：真实编译 C 程序（排序/斐波那契/简单哈希）端到端 prove→verify + 对拍。
- 验收：riscv32 编译的二进制直接证明；native 对拍逐指令一致。

### M9：性能工程
- T1：release 基准（T=2^10..2^20 缩放曲线，thesis 最终证据）。
- T2：witness 生成优化、并行化（rayon/打包）。
- 验收：缩放曲线斜率 ≈ 1（线性）且常数项报告；每指令成本表（release）。

### M10：工程化收官
- API 化 + CI + 文档 + 安全审查准备（诚实边界汇总、威胁模型）。

---

## 6. 风险登记

1. **M7-T0 的绑定胶水**（派生列↔M4 trace oracle）是最大技术不确定性；intmul phase5
   的 iota 嵌入 + per-bit evals 是可抄的完整范例，风险中。
2. **fracaddcheck 外层组装**需自写（分子=全 1、分母=c−指纹、叶子归约）——
   构件全在，胶水工作量约等于"重写 logup* 的 witness 构造层"，风险中低。
3. **路线 B 的 degree-3 evaluator**：无现成件，若 T2 证明可驱动性失败则路线 B 出局
   （A 保底）。
4. **tracer 工程量**（M8-T2）：纯工作量，无技术风险。
5. **ZK/递归**不在 Phase 2；Binius64 的 zk_mlecheck 路线预留。
