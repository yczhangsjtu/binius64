# 汇报：M2 执行层选型 spike（词级门 vs 位级 R1CS）

> 汇报 Agent（Hermes）→ 验收 Agent | 日期：2026-09-06 | 基准：`ACCEPTANCE_BASIS §1/§4`、`tasks/M2-word-gate-spike.md`
> 前置：`research/jolt-to-binary-field-migration-assessment.md`（JUjolt 迁移难点）；`designs/binius64-frontend-api-map.md`；`designs/binius64-constraint-proofs-and-zkvm-plan.md`
> 结论先行：**spike 完成并给出**阳性**结果——frontend 词级 `add`（`iadd_32` 门）prove→verify + 拒假通过，**且**与 logup* 取指查表在**同一 Fiat-Shamir transcript 中组合成功**。因此**建议选型 W2（词级）**，位级 R1CS 仅作对照。全量 `cargo test` **23 passed**（原 21 + 新增 2）。

---

## 1. T1 调研结论：frontend 词级门的证明路径 + 与 logup* 组合可行性

### 1.1 frontend 词级门的证明调用链

`binius_frontend::CircuitBuilder`（词级门，如 `iadd_32`）编译出的电路**不经过 spartan-prover**，而是走 Binius64 原生的 **M4 prover/verifier**：

```
binius_frontend::CircuitBuilder
  ├─ iadd_32(a,b) -> Wire          (crates/frontend/src/builder/mod.rs:1097)
  ├─ add_inout() / add_witness()   (crates/frontend/src/builder/mod.rs:921/934)
  └─ build() -> Circuit            (crates/frontend/src/builder/mod.rs:477)
       └─ circuit.constraint_system() -> binius_core ConstraintSystem
            │                          (crates/frontend/src/artifact/circuit.rs:223)
            ├─ witnessed by WitnessFiller -> ValueVec -> inout() -> &[Word]
            └─ proved/verified by:
                 binius_prover::Prover::prove(&ValueVec, &mut ProverTranscript<C>)
                                            (crates/prover/src/prove.rs:452)
                 binius_verifier::Verifier::verify(&[Word], &mut VerifierTranscript<C>)
                                            (crates/verifier/src/verify.rs:353)
```

### 1.2 transcript 类型是否与 spartan+logup* 同一抽象？——**是**

| 环节 | spartan+logup* 路径（`combined.rs`） | frontend 词级路径（本次） |
|---|---|---|
| prover | `binius_spartan_prover::Prover` | `binius_prover::Prover`（M4） |
| verifier | `binius_spartan_verifier::Verifier` | `binius_verifier::Verifier`（M4） |
| 挑战者类型 | `HasherChallenger<Sha256>`（`combined.rs:50`） | `StdChallenger = HasherChallener<StdDigest> = HasherChallener<Sha256>`（`crates/verifier/src/config.rs:24` + `crates/hash/src/lib.rs:30`） |
| transcript 载体 | `binius_transcript::ProverTranscript<C>` | **同一个 `binius_transcript::ProverTranscript<C>`**（`crates/prover/src/prove.rs:452` 参数类型） |

**证据（文件:行号）**：
- `crates/transcript/src/transcript.rs:239` —— `ProverTranscript<Challenger>` 泛型定义；`:391` —— `into_verifier()`。
- `crates/transcript/src/transcript.rs:43` —— `VerifierTranscript<Challenger>` 泛型定义。
- `crates/prover/src/prove.rs:452` —— `pub fn prove<Challenger_: Challenger + Clone>(&self, witness: &ValueVec, transcript: &mut ProverTranscript<Challenger_>)`：**M4 prover 接受任意 `ProverTranscript<C>`**，非固定类型。
- `crates/verifier/src/verify.rs:353` —— `pub fn verify<Challenger_: Challenger>(&self, inout: &[Word], transcript: &mut VerifierTranscript<Challenger_>)`。
- `crates/ip-prover/src/channel.rs:107` —— `impl<F, Challenger_> IPProverChannel<F> for ProverTranscript<Challenger_>`：**gamma 可对任意 `ProverTranscript<C>` 采样**。
- `crates/field/…/arch`、`crates/ip-prover/src/channel.rs:128` —— `sample::<F>()` 从挑战者抽取域元素。

### 1.3 能否在 frontend 证明中间插入 logup* gamma 采样？——**能（非黑盒）**

`prover.prove(&witness_vec, &mut pt)` 是一次对 transcript 的**顺序写入**（其内部 `channel.finish()` 把消息刷回 transcript），完成后 transcript 停在"frontend 证明消息末尾"，可立即 `IPProverChannel::<LF>::sample(&mut pt)` 采样 gamma。验证端镜像同序。**因此不是一次性黑盒调用**，`combined.rs:169-194` 的"先证明→采样 gamma→logup*；verify 端镜像"模式可**直接移植**到 frontend 路径。

> **T1 判定：组合可行。** 前端电路与 logup* 取指可同 transcript 组合（`T3` 已实证）。

---

## 2. T2/T3 切片结果（真实测试输出摘录）

### T2 `word_add`（词级 add，必做）

真实输出（`--lib word_add -- --nocapture`）：

```
word-level add: rs1=0xfffffffe rs2=0x00000005 -> rd=0x00000003 (mod 2^32)
✅ WORD-LEVEL add proved & verified (Binius64 native prover, iadd_32 gate)
   constraints: ZERO=1 AND=1 IMUL=0 BMUL=0 (gates=2, eval-insns=2)
   values: const=1 inout=3 witness=0 internal=1 | committed trace words=4
   timing (debug build): setup=5.7ms prove=3.2ms verify=0.2ms
   soundness: verifier REJECTED tampered public rd = 0x00000004 ✓
```

- 真词级门：`builder.iadd_32(rs1, rs2)`（`word_add.rs:53`），**不是**位级全加器链改名。
- 拒假为真：`verifier.verify(&bad_inout, &mut vt2).is_err()`（`word_add.rs:125`）——篡改 **public rd**（`rd=a+b+1`），verify 返回 **Err**，非 panic/编译错误。
- 对照组：同语义 32-bit 全加器链（`alu::fa`）在 spartan R1CS（`word_add.rs:137-245`），约束 `mul=256`，prove=35.1ms，拒假通过。

### T3 `word_add_combined`（组合 spike 核心，尽力而为）

真实输出（`--lib word_add_combined -- --nocapture`）：

```
program: add x5,x6,x7 @ pc 0x00 (word 0x007302b3)
  x6=0xdeadbeef x7=0x11111111 -> x5=0xefbed000 (mod 2^32)
✅ WORD-ADD COMBINED proof: frontend iadd_32 + logup* fetch, ONE transcript
   word-gate: x5 = x6 + x7 = 0xefbed000 (iadd_32, 1 AND + 1 ZERO)
   logup*:    T[pc=0x0] = word 0x007302b3 found in program table
   transcript: ProverTranscript<HasherChallenger<Sha256>> shared by both layers
   soundness(1): verifier REJECTED tampered public rd ✓
   soundness(2): verifier REJECTED tampered instruction word in lookup ✓
```

- **单 transcript 组合成功**：`frontend prove → IPProverChannel::sample(gamma) → logup* prove → into_verifier → frontend verify → IPVerifierChannel::sample → logup* verify`，`assert_eq!(verifier_gamma, gamma)` 与 output 一致性均通过（`word_add_combined.rs:70-95`）。
- 两个 soundness 拒假都真实（分别篡改 public rd、logup* 表内指令字 → verify 返回 Err）。

### 全量测试（验收标准 #1）

```bash
cd /home/yczhang/workspace/binius64 && export RUSTFLAGS="-C target-cpu=native" && CARGO_BUILD_JOBS=4 cargo test -p binius-zkvm-slice
```
→ `running 23 tests ... ok; 23 passed; 0 failed`（原 21 + `word_add` + `word_add_combined`）。

---

## 3. T4 成本对照表（实际运行，debug 构建）

> 测量方式：词级用 `CircuitStat::collect(&circuit)`（`word_add.rs:78`）统计 ZERO/AND/IMUL/BMUL 约束数；位级用 `cs.mul_constraints().len()`（`word_add.rs:171`）。耗时用 `std::time::Instant`（`cargo test` 默认 **debug** 构建）。

| 维度 | 词级 `iadd_32`（W2） | 位级 32-bit 全加器链（W1） |
|---|---|---|
| ZERO 约束 | **1** | — |
| AND 约束 | **1** | — |
| IMUL / BMUL | 0 / 0 | — |
| R1CS mul 约束 | — | **256**（编译后实测 128 = 96 booleanity + 32 进位链（经 wire 消除）；prover 侧 padding/blinding 齐 2 的幂 → 256） |
| prove 耗时 | **3.2ms** | 35.1ms |
| verify 耗时 | **0.2ms** | 0.5ms |

**解读**：一条 32-bit `add`，词级门只需 **1 AND + 1 ZERO**（≈1× 基准成本，且**与位宽无关**）；位级全加器链需要 **256 个 mul**（成本 ∝ 位宽）。prove 时间约 11× 差距。这正是"成本与指令数相关、与指令类型无关"的 thesis 在 W2 下的恢复。

**诚实边界（必须标注）**：
- 两条路径走**不同证明后端**：词级 → `binius_prover`（M4 prover）；位级 → `binius_spartan_prover`（R1CS prover）。**约束数是可比主指标**（都是约束系统计数）；**耗时是跨后端对比**，受 prover 实现影响，仅供量级参考。
- 位级约束数以实测为准：编译后 `n_mul=128`（96 booleanity + 32 进位链，经 wire 消除），prover 侧 padding/blinding 齐 2 的幂后 `n_mul=256`。**对比主指标取 256**（prover 实际负担）。

---

## 4. W1/W2 选型建议（明确结论）

### 结论：**建议 W2（词级门）**。

**理由**：
1. **组合可行性已实证**（T1 + T3）：frontend `iadd_32` 与 logup* 取指查表在**同一 `ProverTranscript<HasherChallener<Sha256>>`** 上闭环成立，两个 soundness 拒假通过。任务书标为"未验证"的**最大风险已排除**。
2. **成本恢复 Jolt thesis**：词级 add = 1 AND + 1 ZERO，位级 = 256 mul。ADD 成本不再 ∝ 位宽，而 ∝ 指令数。
3. **扩展性**：frontend 词级门集与 RV32I 指令几乎一一对应（`binius64-frontend-api-map.md`），sub/mul/和/或/比较/分支（`isub_bin_bout`/`imul`/`band`/`bor`/`bxor`/`icmp_*`/`select`）可直接复用，无需自定义位级翻译。

**遗留风险 / 待办**：
1. **内存时序论证仍未做**（属 M3）：词级门解决了 ALU/整数算术，但寄存器堆/内存的"读见最近写" + **版本链电路化**（`ver[rd]'=ver[rd]+IsWrite` 进约束）仍是 M3 目标。`iadd_32` 门本身不承担时序。
2. **两套证明后端并存**：当前 zkvm-slice 其余切片全走 spartan R1CS；选 W2 后后续切片须统一走 `binius_prover`（M4）。存在迁移/统一成本，需在新切片中逐步切换并记录。这是**工程性**风险（非可行性风险）。
3. **单条 add 规模太小**：`iadd_32` 的进位只在 32-bit 半字内（64-bit word 上、下半各自独立加）；**跨 32-bit 进位**（真实 32-bit 溢出 carry-out 需单独抽取 cout）与 **MUL/64-bit 加法**的代价对比未测。建议 M3/M5 补齐。

**决策记录中的量化修正**（若退 W1 的本应表述）：位级 "ADD 成本 ∝ 位宽" 与项目 thesis 相悖——现以 W2 恢复 thesis，无需修正。

---

## 5. 边界与诚实分级

| 切片 | 声称机制 | 分级 | 关键边界 |
|---|---|---|---|
| `word_add`（新增） | 词级 `iadd_32` 门 32-bit add，prove→verify + 拒假 | **⭐ 真** | 真词级门（非位加器改名）；单条 add；rd/rs 均为 inout 公开，未做寄存器堆 |
| `word_add_combined`（新增） | frontend 词级门 + logup* 取指，**单一 transcript** | **⭐ 真（限 transcript 层组合）** | 单条 add + 单地址取指；logup* 程序表为 native 构造（查表一致性，非内存时序论证）；**取指 word 未驱动执行**（x6/x7 直接注入 inout，无"译码→操作数"绑定），组合证明的是"两证明系统可共享 transcript"，不证明"执行的指令==取指的指令"——语义绑定属 M3 |

**诚实要点**：
- `word_add_combined` 的 logup* 表 `T[pc]=inst` 由**程序直接给定**（`prog[init_pc]=inst_word`），logup* 只证明"执行的指令字∈程序表"，**不证明**可执行内存/时序——与项目基线 `mem_instr/mem_arg_*` 的既有诚实边界一致，未夸大。
- 本切片解决的是**执行层算术选型（W1 vs W2）**，**不**解决内存时序/排序（留给 M3）。任何将本结果表述为"完整 zkVM"均为不实。
- 词级 `iadd_32` 结果 `rd = (rs1+rs2) mod 2^32` 为**32-bit 语义**（upper 32-half 独立加法），未做 64-bit 进位链扩展（属后续）。

---

## 6. 变更文件

- `crates/zkvm-slice/src/slices/word_add.rs`（新增，T2+T4：词级 add + 位级对照 + 成本统计）
- `crates/zkvm-slice/src/slices/word_add_combined.rs`（新增，T3：frontend + logup* 单 transcript 组合）
- `crates/zkvm-slice/src/lib.rs`（注册 `word_add`、`word_add_combined` 两个模块 + `run_*` 重导出）
- `zkvm-project/PROGRESS.md`（切片清单：新增两条 + 计数 + ⭐/⚠️ 分级）
- `crates/zkvm-slice/README.md`（切片清单：新增两条 + 计数 + ⭐/⚠️ 分级）
- `zkvm-project/designs/milestone-roadmap.md`（M2 状态：未开始 → 已完成）
- `zkvm-project/M2_REPORT.md`（本文件）

## 7. 提交与环境

- 本改动未做任何 git 操作（任务书禁止）。
- Rust 1.97.1（`rust-toolchain.toml` 钉）；`export RUSTFLAGS="-C target-cpu=native"` + `CARGO_BUILD_JOBS=4`；i5-12400F（AVX2 无 AVX-512）。
