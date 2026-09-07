# M6 送审报告（收官里程碑，2026-09-07）

Workdir: `/home/yczhang/workspace/binius64`。范围：`crates/zkvm-slice/` + `zkvm-project/`；未动上游、无 git 操作。
约束：`RUSTFLAGS="-C target-cpu=native"`、`CARGO_BUILD_JOBS=4`。M6 按任务书 T1-T4 执行，Leader 架构决策（§3）全部落实。

## 结论
M1-M6 全部闭合。`word_vm32.rs`（~1000 行单文件）拆为 `src/vm32/` 四层库，切片 26 变薄层；
测试套件分层（per-instruction 单元 + prove 集成 + 独立 soundness）；**每指令成本基准给出 thesis 定量证据：
31 条 RV32I 中 30/31 条 g/cyc=973 逐数字相同**；五处文档收官。全量 **39 passed / 0 failed**（1 ignored 基准）。

## T1：库拆分（§3.1，逐数字一致验收）

### 结构
```
src/vm32/
  mod.rs          — pub 导出
  isa.rs          — 真 RV32I 编码器（enc_r/i/s/b/u/j + 30 指令构造 + OP_* 常量 + fn_rv32 + sext）+ enc_add（encode.rs 遗产）
  interp.rs       — native 参考解释器 run_program（参数化：init_mem/word_overrides/load_overrides/fetch 基表 fn）+ Trace 类型
  circuit.rs      — build_circuit（译码/执行/32 寄存器值+版本链/RAM 版本链/事件钉扎/inout 布局）+ MSB-bool 词级门辅助
  proof.rs        — run_machine_full（prove + 三表 logup* + verify + 三件套）+ reverify/reverify2 + claims + wlog + fetch 表
slices/word_vm32.rs — 薄层：torture 镜像 fetch_word + bubblesort 镜像 bubble_words + run_word_vm32() + 测试
```
- 拆分基准：纯位置搬迁（脚本按行段切分，函数体零改动），只调整可见性（pub）与两处签名
  （`run_program`/`run_machine_full` 增加 `fetch: fn(u64)->u64` 基表参数——程序镜像归属切片层）。
- **行为不变论证**：bubble 微程序与 v3 报告纪委数字**一字不差**（见对照表）；`run_machine_full`
  同时增强：**空 looker 的 logup\* 表自动过滤**（无内存程序如基准微程序不再 panic，既有程序均含 RAM lookers 不受影响）。

### 拆分前后 CircuitStat 逐数字对照
| 程序 | 周期 | c_ok/l_ok | gates | ZERO | AND | IMUL | BMUL | 说明 |
|---|---|---|---|---|---|---|---|---|
| torture 拆分前（M5-R5 终态） | 50 | true/true | **48821** | 1028 | 18022 | 0 | 22939 | 本轮库化前实测（见附录 A 复现） |
| torture 拆分后 | 50 | true/true | **48821** | 1028 | 18022 | 0 | 22939 | ✓ 逐数字一致 |
| bubblesort 拆分前（v3 报告） | 391 | true/true | **382660** | — | 141805 | 0 | 181163 | v3 报告原文 |
| bubblesort 拆分后 | 391 | true/true | **382660** | — | 141805 | 0 | 181163 | ✓ 逐数字一致 |
| 测试结果 | — | — | — | — | — | — | — | 拆分前 29 passed → 库化后 29 passed（再 +10 新测试 = 39） |
> 注：v2 报告 torture 44,784 gates 为 **VER_MAX=16 时代旧值**；48,821 含 M5-R5 store 地址修复新增
> imm_s/select 门与 VER_MAX=128，是拆分支点（M6 起点）的真实数字。拆分未改任何门（对照表实证）。

### encode.rs 删除 + 老切片零改动
- `src/encode.rs` 删除、lib.rs 移除 `pub mod encode`。任务书断言其"全 crate 无人使用"**不成立**：
  `word_add_combined.rs:27` 引用 `encode::enc_add`。按任务书授权条款（"若有文件引用则改为引用 vm32::isa"）
  将 `enc_add` 原样迁入 `vm32/isa.rs`（实现一字未改），`word_add_combined` import 一行改向。行为不变（回归全绿）。
- 老切片（M1-M4 文件）零逻辑改动：唯一触碰为上述 word_add_combined 的 import 行（任务书 §3.1 明确授权的引用改向）。
- 新代码零警告；老切片 16 条历史警告原样保留（任务书允许）。

## T2：测试套件（§3.2）
- **per-instruction 单元测试**（`vm32/interp.rs` 挂载 `per_inst_tests.rs`，5 个 `#[test]`，毫秒级，~160 断言）：
  30 条指令 × 边界操作数向量（0 / 1 / -1 / 0x7FFFFFFF / 0x80000000 / shamt 0·31 / 负立即数 / x0 写丢弃 /
  signed vs unsigned 比较 / 分支 taken·not-taken / jal·jalr rd 语义 / store→load 往返）。独立于证明管线
  （不经 run_machine_full），是"译码正确性"第一道网。⚠️ 编写期 8 处失败**全部为测试期望值 bug**
  （负 imm 符号扩展、slt 有符号性、分支立即数目标、bltu/bgeu 寄存器号 vs 数值），实现零 bug——已修并留档。
- **soundness 独立 `#[test]`**：原挤在 `run_word_vm32()` 的 5 个篡改用例拆为独立测试
  （`soundness_tamper_alu_wr / wr_ver / ld_val / x0_write / fetch`），失败可定位到用例；`run_word_vm32` 瘦身为 honest 入口。
- **证明集成测试**：torture + bubblesort 端到端（经库调用，含三件套 8/8 交叉核对）。
- 运行方式见 `crates/zkvm-slice/README.md`（快：`cargo test`；bench：`-- --ignored --nocapture bench_instruction`）。

## T3：每指令成本基准（§3.3，thesis 定量证据）
`vm32/bench_tests.rs` `bench_instruction_costs`（`#[ignore]`，N=16 固定操作数微程序，31 条指令，每条
`run_machine_full` 全门级 prove+verify，`c_ok=l_ok=true` 全部成立）。真实输出粘贴于 `zkvm-project/BENCHMARKS.md`，摘要：

| 指令族 | 行数 | gates | and | bmul | g/cyc |
|---|---|---|---|---|---|
| 30 条（ALU R/I、移位、比较、lui/auipc、jal、6 分支、lw/sw） | 30 | **22388** | 8221 | 10411 | **973（全同）** |
| jalr（addi+jalr 对，口径注明） | 1 | 38052 | 14029 | 17835 | 975 |

- **thesis 结论**：per-cycle 约束成本与指令类型无关（30/31 逐数字相同；torture 977 与 bubblesort 979
  跨程序同量级）。IMUL=0（版本链全落在 binfield 门）。debug profile、i5-12400F AVX2、prove 总耗时 ~4.5s。
- **口径**：g/cyc 含 O((32+64))·版本链底噪 + 固定译码（主项为链底噪）；30 条全同 ⇒ 译码+执行增量对
  指令族恒等（差异 <1 门，采样分辨率内）。严格"空载周期"分离留作可选后续，BENCHMARKS.md 如实标注。

## T4：文档收官（§3.4）
1. `architecture.md` — §3 证据链表 21→26（补 word_add/word_add_combined/word_vm/word_vm_ram/word_vm32）；
   §3.1 加 M3-M6 里程碑条目与 full_vm 演示家族"已被 M3-M5 超越"注；§6 加"M1-M6 已完成"注。
2. `zkvm-project/README.md` — 切片表 25→26；新增"Current state"段（M1-M6 完成、thesis 证据位置、5 条已知边界）。
3. `PROGRESS.md` — 补 M5（v3）与 M6 两段。
4. `designs/milestone-roadmap.md` — §4 表 M6 行标记 ✅ 完成（收官）。
5. `crates/zkvm-slice/README.md` — 库结构（vm32 四层）、encode→vm32::isa、快/慢测试运行命令。

## 边界汇总（与 README "Current state" 一致）
1. 固定展开/无动态循环上界；2. fetch/reg-wlog/ram-wlog 表 native 提供（logup* 证一致性；M3 起版本链
   电路化承载最近写语义）；3. O((32+64)·T) 版本链成本（VER_MAX=128）；4. 字寻址、K=64、越界 &0x3f
   静默掩码；5. mul（RV32M）未接入。

## 文件清单
- `crates/zkvm-slice/src/vm32/{mod,isa,interp,circuit,proof}.rs` — 新建库（含 per_inst_tests/bench_tests）。
- `crates/zkvm-slice/src/slices/word_vm32.rs` — 薄层（镜像 + run_word_vm32 + 39 测试中的切片测试）。
- `crates/zkvm-slice/src/lib.rs` — `pub mod vm32;`，移除 `pub mod encode;`。
- `crates/zkvm-slice/src/slices/word_add_combined.rs` — import 改向 `vm32::isa::enc_add`（唯一触碰行）。
- `crates/zkvm-slice/src/encode.rs` — **删除**。
- `zkvm-project/BENCHMARKS.md` — 基准表（新）。
- `zkvm-project/{architecture,README,PROGRESS}.md`、`designs/milestone-roadmap.md`、`crates/zkvm-slice/README.md` — 五处文档。
- `zkvm-project/M6_REPORT.md` — 本文件。

## 测试结果（一行）与复核重点
**`cargo test -p binius-zkvm-slice --lib` → 39 passed / 0 failed / 1 ignored**（bench，复现命令见下）。

Leader 复核重点：
1. **拆分逐数字对照**：torture 48821 / bubblesort 382660 与拆分支点一字不差（对照表 T1）；v2 的 44,784 是
   VER_MAX=16 旧值，勿误当回归。
2. **fetch 参数化**：run_program/run_machine_full 增 `fetch` 基表参数（程序镜像归切片层）；bench 微程序
   （无 RAM 访问）触发的"空 looker 表过滤"是 vm32 库唯一行为增强。
3. **jalr 基准口径**：微程序为 addi+jalr 对（寄存器间接跳转无法静态线性化），BENCHMARKS.md 已注明。
4. 新代码零警告（老切片 16 条历史警告未动，任务书允许）。
5. encode.rs 删除前提修正：任务书"无人使用"不成立（word_add_combined 用了 enc_add），按任务书授权条款改引
   用至 vm32::isa，行数改动限定 1 行。

复现基准：`export RUSTFLAGS="-C target-cpu=native"` + `cargo test -p binius-zkvm-slice --lib -- --ignored --nocapture bench_instruction`。