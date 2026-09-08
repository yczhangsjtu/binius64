# M9 任务书：性能工程 + 缩放曲线（thesis 最终定量证据）

> 里程碑：M9（权威路线 `designs/binary-zkvm-full-roadmap.md` §4）
> 派发日期：2026-09-08 | 派发方：Leader Agent | 执行方：Worker Agent
> 前置：M8-A/B ✅（vm_ram_sort：VM × 排序式 RAM 论证 × BaseFold；ISA 39 条）。
> 本任务书含全部决策；每个 T 是 checkpoint，做不完停在最近 checkpoint 如实送审。

---

## 1. 目标

把"能证明"升级为"证明得动、且成本曲线可测"：
1. **release 级缩放曲线**（thesis 的最终定量证据）：成本 vs 指令数 T 的实测曲线；
2. **M8-A 遗留的规模瓶颈**：N=64 电路构建 OOM 的根因诊断与修复/绕开；
3. **M8-B 遗留的电路缺口**：sb/sh 读改写展开；
4. **witness/证明性能优化**：在实测数据指导下做，不盲优化。

## 2. 任务分解

### T0（诊断，先行）：N=64 OOM 根因
- M8-A 报告：N=64 时**电路构建期**被 SIGKILL。诊断内存大头
  （CircuitBuilder 的门对象？witness 缓冲？NTT 表预生成？）——用
  `/usr/bin/time -v` 或 heaptrack 量级即可，不必深挖；给出根因与修复/绕开方案
  （如分块构建、降低内存复制、释放顺序）。
- 交付：诊断结论 + 修复后 N=64（或更大）可跑通。

### T1（核心）：release 缩放曲线
- release 构建（`--release` + RUSTFLAGS）跑 vm_ram_sort 的 T 扫描：
  N ∈ {16, 32, 64}（T≈1.8k/7k/28k 周期），若机器允许再推一档；
  记录 gates、prove 时间、verify 时间、峰值内存。
- 拟合 prove_time vs T 的斜率（期望 ≈ 线性）；产出 `BENCHMARKS.md` 的
  "缩放曲线"一节（release 数字，与 debug 对照）。
- 每指令成本表升级为 release 版（M6 的 bench 在 release 下重跑）。

### T2：sb/sh 电路层（M8-B 遗留）
- 按 M8-B 报告的方案：sb/sh 展开为"读旧字 + 写新字"双事件（tracer/事件结构扩展），
  排序论证的 val_cons 语义相应覆盖；native + 电路 + 端到端三层 + per-inst 单测。
- 地址语义统一：tracer/事件层统一字节地址（lw/sw 按 >>2 重映射，lb 族天然），
  M8-B 复核点 ④ 的方案落地。

### T3（时间盒内尽力）：性能优化
- 由 T0/T1 数据驱动：最大的 1-2 个热点（预计是 witness 填充或 NTT/承诺），
  做针对性优化（如并行 fill、减少克隆）；报告前后对比。
- **不做** speculative 优化；无数据不动手。

## 3. 验收标准

1. 全量测试绿（release 下至少跑一遍单切片确认无误）。
2. T0 根因结论具体（数据支撑），N=64 跑通或给出确凿不可行原因。
3. T1 缩放曲线表：gates 与 prove_time 随 T 的实测 + 斜率拟合；release 每指令成本表。
4. T2：sb/sh 端到端 + 对拍；地址语义统一无回归。
5. T3 若做：前后对比数字；若未做：说明时间盒内排到的原因。
6. 文档：BENCHMARKS.md 更新、PROGRESS/roadmap 状态、M9_REPORT.md。
7. verify 层 soundness 纪律不变；新代码零警告。

## 4. 送审要求

完成后：`zkvm-project/M9_REPORT.md` + 简短送审消息（≤15 行）。
