# Jolt → 二元域（GF(2^128)）迁移评估

> 日期：2026-09-06 | 调研对象：`~/workspace/jolt`（本地 fork）| 方法：四路并行代码审计
> （域耦合/指令查表/内存检查/证明后端），逐点核对了 25 个 relation、R1CS 约束、sumcheck 引擎与 PCS 层。
> 本文是"后端替换 Jolt"路线的**难度评估基准**，供后续里程碑（M2-M4，编号见
> `designs/milestone-roadmap.md`）及移植决策使用。

---

## 0. 关键事实：本地 jolt 不是上游 a16z/jolt

`~/workspace/jolt` 是一个**深度重构的分叉**，已经做了一次"域抽象化 + PCS trait 化"的预迁移：

- 域层抽象为 `JoltField` trait（`crates/jolt-field/src/algebra.rs:523`），契约层与后端层单向依赖；
- 已有**两个素域后端**：arkworks BN254（默认）+ 自研 Solinas 伪梅森素域 `Fp128 = 2^128 − C`（注意是素域，不是 GF(2^128)）；
- 已有**两条 PCS 路径**：Dory（BN254 配对）+ Akita（LayerZero 的格基透明 PCS，经 `jolt-openings` 的 `CommitmentScheme` trait 接入）。

**含义**：该代码库已是"换后端"的半成品形态——"在非 BN254 域上跑通整个 prover/verifier"已被 Akita 路径实证过。迁移难度评估必须基于这个 fork，而非上游。

---

## 1. 分层契合度总表

| 层 | Jolt 组件 | 二元域契合度 | 判定 |
|---|---|---|---|
| 前端 trace/witness | `JoltTraceRow`、`RAMAccess`、指令语义（u64/i128 整数层） | 完全域无关 | ✅ **零成本保留** |
| claim 代数 | jolt-claims（25 个 relation 的符号声明 + derive 宏） | 表达式只用环运算 | ✅ **最大复用资产** |
| 查表论证 | Shout 式 read+RAF、prefix-suffix、booleanity/Hamming、RA virtualization | eq/one-hot/`ra(ra−1)` 在 char-2 全部照旧成立；2^128 索引空间与 GF(2^128) 元素一一对应 | ✅ **高度同构，最省力的一层** |
| 布尔表 | XOR/AND/EQ/LT/Pow2/Bitmask 等 MLE | {0,1}-值多项式恒等式，任意域成立 | ✅ 可迁 |
| uniform R1CS 骨架 | `guard·(left−right)=0` 门控等值约束 | −1=1 在 char-2 对 0/1 保持 NOT 语义，等值形式幸存 | ⚠️ 骨架可留，系数要重写 |
| 内存检查（Twist） | one-hot ra + write-increment + Val 链 + LT + init/final/output 三件套 | 结构 char-2 幸存；`inc` 可由"值差"重定义为 `val⊕post`，关系式闭合 | ⚠️ **机制可迁，但与 logup* 的"同构"被高估（见 §3）** |
| sumcheck 引擎 | batched prover、round 循环、`s(0)+s(1)==claim` | 主循环 char-2 成立 | ⚠️ **两个 char≠2 假设断裂**（见 §4） |
| PCS | Dory（BN254 配对、GT 承诺、MSM、Pedersen ZK） | 本质绑死曲线 | ❌ **整块报废**（接口已被 trait 隔离） |
| transcript | spongefish duplex + Poseidon(BN254) | 128-bit 挑战宽度恰好吻合 GF(2^128) | ⚠️ 小改：弃 Poseidon，换 HasherChallenger 风格 |

---

## 2. 最省力的部分（为什么"后端替换"方向成立）

1. **trace/指令语义层完全域无关**：`JoltTraceRow` 全是 u64（`jolt-riscv/src/trace_row.rs:57-84`），指令语义在 u64/u128 整数层计算（`LookupTable::materialize_entry(index: u128) -> u64`），只在边界处一次性 `F::from_u64` 嵌入。换域零成本。
2. **jolt-claims 是形式化的协议账本**：25 个 relation 的输入 claim 表达式、输出 opening、挑战集、轮数/度全部符号化声明，域泛型。换后端时这层几乎原样保留——它是把 Jolt 几十个子协议"翻译"到 Binius64 的对照表。
3. **查表论证骨架（Shout read-RAF）与二元域天然同构**：eq 多项式、one-hot ra 分块乘积、booleanity（`x²=x ⟺ x∈{0,1}` 在 GF(2^128) 成立）、Hamming weight、prefix-suffix 分解全部是域泛型代码。Binius64 的 logup* 承载同类查表已被本项目 21 个切片实证。
4. **PCS 接口已 trait 化**：`jolt-openings` 的 `CommitmentScheme`/`StreamingCommitment`/`BatchOpeningScheme` 已被 Akita 适配验证能承载 merkle 式 PCS。Dory 报废是"写一个新后端"，不是"改协议栈"。

---

## 3. 对旧研究文档的更正：Twist ↔ logup* 的"语义同构"被高估

`research/jolt-binius-memory-argument-mapping.md` 称 Jolt 的 `ra·(val+γ(val+inc))` 与 logup* 的 multiset "证明同一命题（读=最近写）"，并把 `ram_inc → 版本序号 ver` 标为 ✅。**经代码逐行核对，该说法需降级**：

- **同功但不同构**。Twist（`jolt-claims/.../ram/read_write_checking.rs:88-103`）不是多重集合论证：没有 permutation、没有 multiset 等式。它证明的是"Spartan 声明的 rv/wv 与**已提交的 inc 流**和**虚拟 Val 多项式**在随机点一致"；全局时序由 (i) Val 的 `prev_val/next_val` **链式构造**、(ii) val_evaluation 的 **LT 多项式加权 inc 累加**（`ram/val_check.rs:45-58`）、(iii) Val_init/Val_final/OutputSumcheck（io_mask）三道独立检查共同钉死。
- `inc` 是**值的增量**（`post − pre`，i128 嵌入），**不是版本计数器**；Twist 全代码库无 timestamp 概念。本项目 `mem_arg_ts`/`reg_rw` 的"写日志表 + version"方案在 Jolt 里**没有对应物**。
- 映射表遗漏的 Twist 组件：LT 多项式、prev/next 链式构造、Val_init 分解（advice/program image）、OutputSumcheck 的 io_mask、Hamming booleanity + ra virtualization。换后端时这些必须一并转译。
- **说对的部分**：ra one-hot、eq 绑定、H²−H booleanity 在 char-2 原样成立；前端 trace（`RamCycleMajorEntry` 的 prev/next_val 就是裸 u64）确实域无关可保留；无环绕计数器问题。
- **迁移顺序建议（据此修正）**：bytecode read-RAF（只读、无 inc、结构与 logup* 最接近）→ 寄存器（K=128 小、无 init/io 复杂度）→ RAM（有 Val_init/advice/output 三件套）放最后。这与本项目 M3（reg_rw 版本链电路化，编号见 `designs/milestone-roadmap.md`）的顺序一致。

---

## 4. 阻力点清单（按严重性排序）

### 4.1 协议级断裂（公式在 char-2 下错或塌缩）——真正的硬骨头

1. **整数嵌入语义系统性失效**（最严重）。Jolt 把 u64 值嵌入域后用**域加法表达整数加法（含进位、二补）**，char-2 下 `from_u64(a)+from_u64(b) = a⊕b` 全部语义失效。渗透在四个层面：
   - **R1CS 约束系数**：`rv64.rs` 中 `RamAddress = Rs1 + Imm`、`RightLookupSub = left − right + 2^64`（`TWOS_COMPLEMENT_BIAS`）、PC+4、压缩指令 −2、约束 19 `Product = LeftInput × RightInput`（域乘法承担整数乘）。char-2 下 2=0、−1=1，约束形状直接失效。
   - **combined-operand 查表 trick**：ADD 的索引 = 整数 x+y（进位隐含在索引 ≥ 2^64 里）、SUB 的索引 = x+2^64−y、MUL 的索引 = 128-bit 整数积 x·y——全部依赖 p 足够大使整数加/乘 = 域加/乘。**这是 Jolt 让 ADD/MUL 免费的核心机制，在二元域整体失效**：ADD 需改回进位链（位分解 + carry gadget，logup* 可查），MUL 需位级乘法器或 Binius64 原生 IMUL 约束（3-4×AND）。
   - **带符号嵌入**：`from_i128` 负立即数（分支 `NextUnexpPC = PC + Imm`）、SLT 表的 `x_sign − y_sign + lt`、`VirtualNegateIf` 的 `value − 2·sign·value + 2^64·sign`——2·x 与 2^64 在 char-2 全塌缩，需重写为 GF(2) 多项式恒等式。
   - **表 MLE 的整数重建**：RangeCheck 的 `Σ 2^i·r_i` 在 GF(2^128) 里重建的是位向量打包而非整数；表本身可迁，但表与 R1CS 的接口语义要换（不能再与域内加/乘联动）。
2. **uni-skip 首轮在 char-2 不可用**：`centered_lagrange_evals` 在含负节点的整数域上做 Lagrange（节点 `F::from_i64(domain_start+k)`、阶乘权重 + 求逆）；char-2 下节点 s 与 s+2 重合、阶乘权重为零。Spartan outer 的 uni-skip 首轮（degree 27/domain size 10）需回退为普通首轮（`SumcheckDomain` 已抽象出 `BooleanHypercube` vs `CenteredInteger`，机制上可退）。
3. **批 sumcheck 的 dummy-round 折半失效**：非激活成员靠 `claim·two_inv` 维持（`jolt-sumcheck/src/prover.rs:236-240`，注释明写 "Jolt fields are large-prime"）。char-2 下 2=0，整套 `BatchPrelude` padding 缩放/减半机制需重设计——Binius64 的替代方案是"同维批次 + batching 变量"（`ip/src/sumcheck/batch.rs`），只允许同 n_vars 批次。

### 4.2 整体报废（绑死曲线/素域，无对应物）

- **jolt-dory + jolt-crypto/ec**：配对、GT 承诺、行级 MSM、AFGHO16 inner-product opening、one-hot MSM 优化。性能模型整体失效，需重新设计打包/稀疏承诺策略（好消息：BaseFold 对小域元素的打包提交是原生能力，one-hot 优化自动获得更强版本）。
- **ZK 链**：`ZkOpeningScheme`（Pedersen 承诺）、`CommittedSumcheckProof`、jolt-blindfold、lattice 协议——全部报废。Binius64 的 ZK 走 `zk_mlecheck` + masking，需重新选型或放弃 BlindFold 路线。
- **同态批 opening**：`AdditivelyHomomorphic::combine` + `HomomorphicBatch` 作废（merkle 承诺非同态）；替代是 PCS 原生 batch opening 或 fracaddcheck 归约。
- **Poseidon sponge**（BN254 专属）：弃用，换 Keccak/Blake2b（域无关）。

### 4.3 契约层小改

- `jolt-field` 的 `Field::two_inv`/`half` 显式 `expect("field has characteristic two")`（`algebra.rs:200-205`）；`from_i64/from_i128` 的负数嵌入语义需收缩契约（建议改为"有符号常数 → ±标记 + 幅值嵌入"，char-2 下负号即正号——多数 ±1 系数因此自动正确）。
- transcript：`OptimizedChallenge::challenge_128() -> Fr` 硬编码返回 BN254 Fr，需泛型化；BN254 的 125-bit 掩码挑战惯例替换为 16 字节直映。
- i128 小标量 MSM 优化体系（`CompactPolynomial<i128>`、`FusedInc`/`BalancedIncDigit`）在二元域失去存在意义（"标量位长决定 MSM 成本"的模型不存在）。

---

## 5. 整体难度评估

**结论：可行性高，但不是"换泛型参数"，而是"保骨架、换血液"。**

- **架构阻力小**：这个 fork 的泛型化程度远超上游——trace 层域无关、claims 层域泛型、PCS 已 trait 化且有 Akita 先例。分层看，约 60-70% 的代码（trace/witness/claims/poly 机器/sumcheck 主循环/查表论证骨架）是可保留或近直接复用的。
- **真正的硬点集中且已定位**，就三处：
  1. **整数算术重建**（ADD/SUB/MUL/DIV/比较/PC/立即数）——Jolt 靠素域整数嵌入免费获得的东西，二元域必须用进位链 + 查表 + Binius64 原生 IMUL 重建。**这恰好是本项目 thesis 的镜像**：Jolt 的"成本∝指令数、与指令类型无关"是靠域原生整数运算拉平的；搬到二元域后，整数运算重新成为成本中心（但 Binius64 词级 IMUL 只需 3-4×AND，不随位宽平方爆炸，成本仍可控）。
  2. **PCS 更换**（Dory → BaseFold）——单块工作量最大，但接口已被隔离，是"写新后端"而非"改协议"。
  3. **两个 char≠2 假设**（uni-skip centered domain、batch padding two_inv）——局部、已定位、有 Binius64 现成替代方案。
- **ZK 是独立大件**：Jolt 的 BlindFold/Pedersen ZK 链整体报废，若目标含 ZK 需按 Binius64 的 zk_mlecheck 路线重新设计，工作量未计入上述评估。
- **内存论证迁移的次序**应按 §3 修正后的顺序：bytecode → registers → RAM。

**与本项目切片工作的衔接**：21 个切片已实证的能力（logup* 查表、Spartan 状态机、同 transcript 组合、读⊆写、写日志+版本绑定）恰好覆盖迁移清单中"查表层"和"组合层"的机制；切片尚未覆盖的（uni-skip 替代、批 sumcheck 二元化、BaseFold 批 opening 策略、完整 Twist 三件套转译）就是迁移工程的主体。下一步（M3，编号见 `designs/milestone-roadmap.md`）应优先做"寄存器 ReadWriteChecking 的完整转译"（含 LT/链式 Val，而非仅版本绑定），它是 RAM 的前置、规模最小、且能直接验证 §3 的修正结论。
