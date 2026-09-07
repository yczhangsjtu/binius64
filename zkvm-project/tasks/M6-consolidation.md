# M6 任务书：固化库 + 测试套件 + 每指令成本基准（收官里程碑）

> 里程碑：M6（权威定义见 `zkvm-project/designs/milestone-roadmap.md` §4，最后一个）
> 派发日期：2026-09-07 | 派发方：Leader Agent | 执行方：Worker Agent
> 前置：M1-M5 全部 ✅。M6 不是新机制，而是把已验证的机制**固化为可复用资产**并产出
> **thesis 的定量证据**（每指令成本表）。
> 本任务书 §3 是 Leader 的架构决策，Worker 按此实现；发现不可行处在报告中记录并给
> 替代方案，不要静默改架构。

---

## 1. 目标

1. **库化**：M5 的 `word_vm32.rs`（867→998 行单文件，含解释器/编码/电路/管线/查表/对拍
   全部职责）拆分为 crate 内可复用模块；`word_vm32` 切片变为薄调用层。**行为不变**
   （重构纪律：所有现有测试保持绿色，不改任何电路/证明语义）。
2. **测试套件**：建立分层测试结构——native 单元测试（每指令 × 边界操作数向量）/
   prove 集成测试 / soundness 测试，并文档化运行方式。
3. **每指令成本基准**：产出**每条指令的约束成本表**（thesis"成本∝指令数、与指令类型
   无关"的定量证据），可复现、落成文档。
4. **文档收官**：架构文档与 README 反映 M1-M6 完成态。

## 2. 范围与边界

- 只动 `crates/zkvm-slice/` 与 `zkvm-project/`；禁改上游；禁 git。
- 构建：`export RUSTFLAGS="-C target-cpu=native"` + `CARGO_BUILD_JOBS=4`。
- **不动** M1-M4 的老切片文件（`word_vm.rs`/`word_vm_ram.rs` 及更早）——它们是已验收的
  历史证据，重构成共享库的收益不抵风险。只有 `word_vm32.rs`（M5）拆分为库。
- 老切片的 16 条历史编译警告不动；新代码零警告。
- 诚实纪律不变；基准数字必须来自真实运行，标注构建 profile（debug/release）与机器。

## 3. Leader 架构决策

### 3.1 库结构（在 `crates/zkvm-slice/src/` 下新增 `vm32/` 模块）
```
src/vm32/
  mod.rs        — pub 导出
  isa.rs        — 真 RV32I 编码器（enc_r/i/s/b/u/j + 30 指令构造函数 + OP_*/FUNCT3 常量）
  interp.rs     — native 参考解释器（run_program，参数化程序镜像/init_mem/overrides）
  circuit.rs    — build_circuit（译码/执行/32 寄存器链/RAM 版本链/事件钉扎/inout 布局）
  proof.rs      — run_machine_full（prove+三表 logup*+verify+三件套）+ reverify + claims_from_inout
```
- 拆分基准：`word_vm32.rs` 现状为唯一权威实现；拆完后 `slices/word_vm32.rs` 只保留
  程序镜像（torture/bubblesort）+ `run_word_vm32()` + 测试，逻辑全部委托 `vm32::*`。
- **验收关键**：拆分前后 `CircuitStat`（ZERO/AND/IMUL/BMUL/gates）与全部测试结果
  必须逐数字一致——在报告中给出对照。
- 顺带处理：`src/encode.rs` 目前全 crate 无人使用（旧简化编码）。M6 决策：**删除**
  `encode.rs`（其职责由 `vm32/isa.rs` 的真编码取代），并在 lib.rs 移除 `pub mod encode`。
  若有任何文件引用它则改为引用 vm32::isa。
- `src/alu.rs` 保留（老切片在用）。

### 3.2 测试套件结构
- **native 单元测试**（快，每条指令一组边界向量）：`vm32::interp` 的 per-instruction
  测试——操作数向量含 0/1/-1/0x7FFFFFFF/0x80000000/shamt=0/31/负立即数/x0 写丢弃。
  这是"译码正确性"的第一道网（独立于证明）。
- **prove 集成测试**（慢）：torture + bubblesort 端到端（现有，改为经库调用）。
- **soundness 测试**：现有用例保留为独立 `#[test]`（每个用例一个测试函数，不再挤在
  一个 test 里）——失败时能定位到具体用例。
- 文档化：`crates/zkvm-slice/README.md` 写明快/慢测试的运行命令。

### 3.3 每指令成本基准（thesis 定量证据，本里程碑的核心交付）
- 对 §3.1 清单的 30 条指令，每条构造一个 **N 周期微程序**（同一指令重复 N 次，
  N≥8，操作数固定且有非平凡位型），经库管线 prove，记录：总 gates、ZERO/AND/IMUL/BMUL、
  gates/周期、prove/verify 耗时。
- **输出**：`zkvm-project/BENCHMARKS.md`——每指令成本表（按 gates/周期排序），
  标注：构建 profile、机器（i5-12400F AVX2）、版本链等固定开销的扣除口径
  （每周期成本含 ~O((32+64)) 版本链底噪，报告中单列"译码+执行增量"与"链底噪"两个分量，
  若无法分离则说明口径）。
- 运行方式：一个 `#[test]`（如 `bench_instruction_costs`），默认 `cargo test` 跳过
  （`#[ignore]`），用 `cargo test -p binius-zkvm-slice -- --ignored --nocapture` 复现。
  表中的数字必须来自该测试的真实输出（粘贴进 BENCHMARKS.md 并注明日期）。

### 3.4 文档收官
- `architecture.md`：§3 证据链表更新到 26 切片（补 word_vm/word_vm_ram/word_vm32），
  §6 路线图标记 M1-M6 完成态，§2/§3.1 中已被 M2-M5 超越的表述加注（指向
  milestone-roadmap 与各 M 报告）。
- `zkvm-project/README.md`：补"当前状态"一段（M1-M6 完成、thesis 证据位置、
  已知边界汇总：固定展开/表 native 共享/O(K·T) 版本链/字寻址）。
- `zkvm-project/PROGRESS.md`：M6 段。
- `zkvm-project/designs/milestone-roadmap.md`：M6 状态。
- `crates/zkvm-slice/README.md`：库结构说明 + 测试运行方式。

## 4. 任务分解

- **T1 库拆分**（§3.1）：拆 `word_vm32.rs` → `vm32/`；报告给出拆分前后 CircuitStat/测试
  逐数字对照。删 `encode.rs`。
- **T2 测试套件**（§3.2）：per-instruction native 单元测试 + soundness 用例独立成
  `#[test]` + README 运行说明。
- **T3 成本基准**（§3.3）：30 指令微程序基准 + `BENCHMARKS.md`。
- **T4 文档收官**（§3.4）+ `M6_REPORT.md`（含拆分对照、测试清单、基准表、边界汇总）。

## 5. 验收标准（Leader 逐项核对）

1. `cargo test -p binius-zkvm-slice` 全绿；`-- --ignored` 基准测试可复现。
2. 拆分前后 CircuitStat 数字逐一致（报告对照表）；老切片零改动（git diff 佐证）。
3. 30 条指令 × 边界向量的 native 单元测试存在且通过；soundness 每用例一个 `#[test]`。
4. `BENCHMARKS.md` 成本表数字与复现运行一致；口径（profile/机器/底噪分量）标注完整。
5. 四处文档更新 + `encode.rs` 删除无残留引用；新代码零警告。

## 6. 送审要求

完成后：送审材料落成 `zkvm-project/M6_REPORT.md`，回复一条**简短送审消息**（≤15 行：
结论、文件清单、测试结果一行、需 Leader 复核重点 1-3 条）。细节一律在报告里。
