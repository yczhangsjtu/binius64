# M3 返工任务：闭合 logup* ↔ 电路的 inout 绑定

> 派发日期：2026-09-07 | 派发方：Leader Agent | 前置：M3 首次送审（word_vm.rs，切片 24）
> 任务书 `tasks/M3-word-vm.md` 继续有效；本文件是**针对一处未达标项的增量返工规范**。

## 问题（验收发现，附代码证据）

`word_vm.rs:303-346`：logup* 的 fetch/读写 claim 全部由 **native trace** 构建
（`trace.cycles[t].inst`、`c.reads` 的 native `ver`/`val`），原样传给 `verify_reduction`。
读事件的 (reg, ver, value) **不是电路 inout**（`InoutRefs` 仅 inst/pc/init_regs/final_regs，
`:144-149`）。验证端从未从 inout 重算 claim → 电路层与 logup* 层不绑定，一个作弊 prover
可以让电路执行程序 A、让 logup* 检查另一份编造的访问序列 B，两侧同时通过。
soundness 1-3 只在纯 native 数据上重放 logup*（`run_soundness`，`:377-470`），电路不参与。

这违反任务书 §3.2："所有需要被 logup* 引用的值，一律走电路 public inout word，
验证端从同一份 inout 重算 logup* claim"。

## 返工要求

### R1 读/写事件全部暴露为 inout，并与电路 wire 约束相等
每周期暴露（布局自定，报告中说明）：
- 读事件 ×2（rs1、rs2）：`(reg, ver, val)` 三个 inout word。
  电路约束：`read.val == mux8(reg_cur, rs1_decoded)`（即 ALU 实际输入 wire `a_val`/`b_val`）、
  `read.reg == rs1/rs2（译码值）`、`read.ver == mux8(ver[t], rs1/rs2)`（**电路版本链**在
  读时刻的值——mux8 同样适用于 ver[t]，复用现有 mux8）。
- 写事件 ×1：`(reg, ver, val, is_write)` 四个 inout word（is_write 0/1，beq 周期为 0）。
  电路约束：`write.val == sum`（ALU 输出）、`write.reg == rd`、`write.ver == ver[t][rd]+1`、
  `is_write == is_alu_write`（译码派生）。

### R2 验证端从 inout 重算全部 logup* claim
- fetch claim：index = `pc[t]`（inout），value = `inst[t]`（inout）。
- 读写 claim：index = `reg*VER_MAX + ver`、value = `val`，全部从 inout_words 切片重建。
- **禁止**把 native trace 的 claim 向量直接传给 `verify_reduction`。prover 侧 looker
  同样从 witness 的 inout 段取（保证两侧同源）。

### R3 soundness 用例改为"经 inout 篡改"（绑定成立的实证）
原有用例 1-3 在绑定后变为"API 层面不可能"（claim 由 inout 派生），改为：
1. **过期读**：把某读事件的 val inout 改为旧版本值 → 电路 assert_eq 失败或 logup* 拒
   （指明是哪一层拒，报告中说明）。
2. **版本篡改**：改读事件的 ver inout → 拒。
3. **非法取指**：把某 `inst[t]` inout 改成程序表外的合法编码字（如另一个 addi）→
   电路仍满足（译码正常）但 logup* 取指拒——**这一例直接证明"执行的指令==取指的指令"**。
4. 结果篡改（保留原有用例 4）。
每例必须是 verify 层 `is_err()`，不允许 panic/witness build 失败。

### R4 报告更正
`M3_REPORT.md` 中以下表述按返工后的实情重写：
- §1 表格"寄存器读==写绑定"行、§2"读==most-recent-write 如何被强制"段；
- §8 复核重点 3 的错误表述（"ver_at_read 来自电路版本链"在返工前不成立，返工后才成立）。
更新测试输出摘录与约束统计（inout 增多，约束数会变）。

## 验收标准（在 M3 任务书 §6 基础上替换第 3 条相关的核查方式）

1. `cargo test -p binius-zkvm-slice` 全过（24 passed）。
2. 能在代码中指出：读/写事件的 inout 分配、`read.val == a_val` 等 assert_eq 约束行、
   verifier 端从 `inout_words` 重建 claim 的代码行。
3. 新 soundness 用例 1-4 全部为 verify 层拒绝；用例 3（表外指令字）必须呈现
   "电路满足 + logup* 拒"的分层拒绝特征。
4. 报告 §1/§2/§8 更正完成，无"claim 来自 native trace"残留路径。

## 送审要求

更新 `zkvm-project/M3_REPORT.md`（标注 v2 + 返工说明），回复简短送审消息
（结论、改动文件、测试结果、需复核重点）。
