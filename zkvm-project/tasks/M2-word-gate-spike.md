# M2 任务书：执行层选型 spike（词级门 vs 位级 R1CS）

> 里程碑：M2（权威定义见 `zkvm-project/designs/milestone-roadmap.md` §4）
> 派发日期：2026-09-06 | 派发方：Leader Agent | 执行方：Worker Agent
> 前置依据：`zkvm-project/research/jolt-to-binary-field-migration-assessment.md`（Jolt 迁移难度分析）

---

## 1. 背景与目标

Jolt 的"每条指令成本均匀"靠素域整数嵌入（combined-operand trick：ADD 的查表索引=整数 x+y，
R1CS 用域加法绑定）实现，该机制在 char-2 二元域上**整体失效**。因此二元域 zkVM 的整数算术
（ADD 是 RV32I 最高频指令）必须重新选型：

- **W1 位级**：spartan-frontend 位级 R1CS，ADD=32 位全加器链（~64-96 mul/32-bit）。
  现有 21 个切片全部走这条路。缺点：加法成本 ∝ 位宽，与项目 thesis（成本∝指令数、与指令
  类型无关）相悖。
- **W2 词级**：Binius64 frontend 的词级约束门（`iadd_32`/`band`/`imul` 等），加法固定几个
  约束、IMUL 仅 3-4×AND。优点：恢复 Jolt 式均匀成本。**风险未验证**：frontend 电路的
  prove/verify 路径与 logup* 查表能否在同一 Fiat-Shamir transcript 组合（现有组合切片
  combined/multi_combined 用的都是 spartan-prover + logup*，不是 frontend）。

**M2 目标**：用一个最小 spike 实证 W2 的可行性，产出 W1/W2 选型决策。

**判定规则**：
- frontend 词级 `add` 切片 prove→verify + 拒假通过，**且**与 logup* 取指查表同 transcript
  组合成功 → 建议 W2；
- 组合不可行（或 frontend 证明路径无法与 logup* 交织）→ 退回 W1，并在决策记录中量化
  "ADD 成本 ∝ 位宽"对 thesis 的修正。

## 2. 范围与边界（严格遵守）

- **只允许**修改/新增：`crates/zkvm-slice/`（本项目代码）与 `zkvm-project/`（文档）。
- **禁止**修改 Binius64 上游 crates（compute/field/frontend/ip*/spartan-*/hash/math/utils 等），
  本项目仅调用其 API。若发现上游 API 缺失/疑问，记录在报告中，不要绕过。
- **禁止**任何 git 变更操作（commit/push/reset 等）。
- 诚实分级纪律：⭐=机制闭环+有意义；⚠️=演示/边界（核心困难未做进约束、靠 native 预计算）。
  报告中所有结论必须如实分级，⚠️ 区功能不得表述为"完整实现"。

## 3. 任务分解

### T1 调研：frontend 词级门的证明路径（先调研再动手）
- 读 `zkvm-project/designs/binius64-frontend-api-map.md`（frontend 门集 ↔ RISC-V 指令映射 +
  API 清单）和 `zkvm-project/designs/binius64-constraint-proofs-and-zkvm-plan.md`（约束证明机制）。
- 在上游 crates 中定位：`binius-frontend`（CircuitBuilder 词级门，如 `iadd_32`）的
  prove/verify 调用链——它走哪个 prover？transcript 类型是什么？与 spartan-prover 的
  `ProverTranscript<HasherChallenger<Sha256>>` 是否同一抽象？
- 重点回答：**frontend 电路证明的中间能否插入 logup* 的 gamma 采样与 prove/verify_reduction**
  （参照 `crates/zkvm-slice/src/slices/combined.rs:169-194` 的 spartan+logup* 同 transcript 模式：
  先 Spartan prove → `IPProverChannel::sample(&mut transcript)` 采样 gamma → logup* prove；
  verify 端镜像）。若 frontend 的 prove 是一次性黑盒调用（无法在中间插入 transcript 操作），
  这就是组合不可行的证据，如实记录并转向 W1 结论。

### T2 切片：词级 add（必做）
- 新增 `crates/zkvm-slice/src/slices/word_add.rs`：用 frontend CircuitBuilder 词级门构建
  单条 `add rd, rs1, rs2`（32-bit），prove→verify 闭环 + 一个 soundness 拒假用例
  （篡改 public 段结果值；遵循项目铁律：不篡改会导致 witness build 失败的内部位）。
- 在 `src/lib.rs` 注册模块（`#[path]` + `pub use`），附 `#[test]`。
- 对照组：同语义的位级 32-bit 全加器链版本（可直接引用 `alu.rs` 的 `fa`/`add_constant` 思路
  扩展到 32 位；若工作量过大，可用 8-bit 位级 + 线性推算 32-bit，但要在报告中注明是推算）。

### T3 切片：词级 add + logup* 取指同 transcript 组合（spike 核心，尽力而为）
- 新增 `crates/zkvm-slice/src/slices/word_add_combined.rs`：程序内存表 T[pc]=word（logup*
  查表取指）+ 词级门执行 add，同一 transcript。
- 若 T1 调研结论是"无法组合"，本任务改为：**记录不可行的具体技术原因**（哪个 API 是黑盒、
  transcript 类型不匹配在哪），这就是 spike 的阴性结果，同样有价值。

### T4 成本量化（必做）
- 词级 add 的约束数（ZERO/AND/IMUL/BMUL 分类统计，若 frontend 有 CircuitStat 类工具则用之）
  vs 位级 add 的 mul 约束数。表格呈现，给出 prove/verify 实测耗时（release 或 debug 注明）。

### T5 决策记录与送审（必做）
- 写 `zkvm-project/M2_REPORT.md`（沿用 M1_REPORT.md 的体例），内容：
  1. T1 调研结论（frontend 证明路径、可否与 logup* 组合，证据：文件:行号）；
  2. T2/T3 切片结果（真实测试输出摘录）；
  3. T4 成本对照表；
  4. **W1/W2 选型建议**（明确结论 + 理由 + 遗留风险）；
  5. 边界与诚实分级。
- 若新增了切片，`zkvm-project/PROGRESS.md` 和 `crates/zkvm-slice/README.md` 的切片清单
  顺带更新（切片计数、一行描述、⭐/⚠️ 分级）。

## 4. 验收标准（Leader 将逐项核对）

1. `cd /home/yczhang/workspace/binius64 && export RUSTFLAGS="-C target-cpu=native" && CARGO_BUILD_JOBS=4 cargo test -p binius-zkvm-slice` 全部通过（含新切片测试）。
2. 新切片确实用**词级门**（不是又把位级全加器链换个名字）；soundness 用例真实被拒
   （篡改后 verify 返回 Err 或断言失败，不是靠 panic/编译错误"拒假"）。
3. 组合问题（T1/T3）有明确的是非结论 + 代码级证据，不接受"应该可以"。
4. 成本对照表数字来自实际运行（CircuitStat 或约束计数），注明测量方式。
5. M2_REPORT.md 给出明确 W1/W2 建议；所有 ⚠️ 边界如实标注。

## 5. 环境与命令

- Rust 1.97.1（rust-toolchain.toml 已钉）；本机 i5-12400F（AVX2，无 AVX-512）。
- 构建必须：`export RUSTFLAGS="-C target-cpu=native"` + `CARGO_BUILD_JOBS=4`（防 OOM）。
- 单切片调试：`CARGO_BUILD_JOBS=4 cargo test -p binius-zkvm-slice --lib word_add -- --nocapture`。

## 6. 参考代码（先读这些再动手）

- `crates/zkvm-slice/src/slices/combined.rs` — spartan+logup* 同 transcript 组合的范本
  （gamma 采样时机、prove/verify 两侧镜像、soundness 篡改手法）。
- `crates/zkvm-slice/src/slices/reg_rw.rs` — logup* 写日志表 + 版本绑定的最新用法。
- `crates/zkvm-slice/src/alu.rs` / `encode.rs` — 共享位级工具与编码常量。
- Binius64 上游 examples（blake3/sha256/ethsign）— frontend CircuitBuilder 词级门的
  成功调用先例（上游 crate 内，只读参考）。

## 7. 送审要求

完成后：
1. 把送审材料落成文件 `zkvm-project/M2_REPORT.md`（见 T5）；
2. 输出一条**简短送审消息**（≤15 行）：完成/未完成的结论一句话、新增/修改的文件清单、
   测试结果一行、W1/W2 建议一句话、需要 Leader 复核的重点 1-3 条。细节一律在报告文件里，
   不要在送审消息里展开。
