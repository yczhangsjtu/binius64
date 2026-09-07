# M5-R5 收尾任务：bubblesort 端到端电路证明（M5 唯一未闭合项）

> 派发日期：2026-09-07 | 派发方：Leader Agent | 前置：M5 v2 验收（R1-R4/R6/R7 已通过）
> 任务书 `tasks/M5-rv32i-expansion.md` §3.4/§6.6 的 bubblesort 要求仍未达成，本任务闭合它。

## 任务

1. **参数化程序镜像**（worker 在 v2 报告 §Honest 边界自给的方案 1）：
   `run_program`/`build_fetch_prog`/`run_machine_full` 接受程序镜像参数（或让
   `build_fetch_prog` 应用 overrides），`PC_START`/`HALT_ADDR` 配置化。
2. **bubblesort 程序**：对 RAM 中 8 个 32-bit 字冒泡排序，初始值须含重复值与
   `0x80000000` 边界值；用真 RV32I 编码（lw/sw/blt 或 bge/addi/bne/jal 等）。
3. **端到端**：native 解释器跑 bubblesort → 终态逐字对拍（排序结果正确）→
   电路 prove+verify（c_ok && l_ok）→ output 三件套检查（排序后数组或校验和为
   公共期望值，期望值**独立给定**，不从 trace 算）。
4. **报告**：M5_REPORT.md 标 v3，补 bubblesort 段（程序镜像、周期数、约束数、
   对拍输出）；把 milestone-roadmap 的 M5 状态从"部分"改为完整（若当前已标 ✅ 则确认
   表述不含 bubblesort 缺口）。
5. 顺带清理 `word_vm32.rs` 的编译警告（v2 有若干 `-->` 警告行；M4 曾做到零警告）。

## 验收标准

1. `cargo test -p binius-zkvm-slice` 全过。
2. bubblesort 的诚实证明 c_ok && l_ok，且排序结果与 native 独立对拍一致（报告中
   给出排序前/后的内存内容）。
3. torture 路径不回退（word_vm32_prove / word_vm32_rework 仍全绿）。
4. 无新增编译警告。

## 送审要求

更新 `zkvm-project/M5_REPORT.md`（标 v3），回复简短送审消息（结论、文件清单、
测试结果、需复核重点）。
