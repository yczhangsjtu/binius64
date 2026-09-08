# M14 任务书：性能基线确立（优化循环的起点）

> 派发日期：2026-09-08 | 派发方：Leader Agent | 执行方：Worker Agent
> 目的：在进入"优化→验收→提交/回退→再优化"循环之前，确立**可复现的双方基线**。
> 本轮只做测量与分析，**不改任何优化代码**。

---

## 1. 已查证的 Jolt 基线（Leader 调研结论，直接采用）

- **2025-08（Twist/Shout 集成后）**：>500K RISC-V cycles/sec（MacBook）、
  >1M cycles/sec（32 核 CPU）、proof <50KB。
  来源：[a16z: Jolt gets a 6× speedup](https://a16zcrypto.com/posts/article/jolt-6x-speedup/)。
- 2024-04 旧版：M3 Max ~100K cycles/sec。
  来源：[a16z: Building Jolt](https://a16zcrypto.com/posts/article/building-jolt/)。
- 口径差异（必须写进对照表）：Jolt=RV64、Dory 承诺、生产级优化多年；我们=RV32、
  BaseFold、切片级工程。对比的目的是**数量级定位**，不是精确对标。

## 2. 任务

### T1：我们的标准化基线（release，可复现）
- 写一个基准入口（`#[ignore]` 测试或 tools/ 脚本），对 N=16/32/64 三档测量并输出：
  周期数 T、gates、g/cyc、**分阶段耗时**（build_circuit / witness_fill /
  frontend_prove / BaseFold+logup+fracadd / verify-online）、总耗时、
  **cycles/sec（含构建与纯证明两个口径）**、峰值 RSS、proof 体积。
- 每档跑 3 次取中位数。串行单进程（内存护栏）。
- 落盘：`zkvm-project/BASELINE.md`（含机器/工具链/profile 口径 + 原始输出）。

### T2：Jolt 对照表 + 差距分解
- BASELINE.md 中列出对照表：双方 cycles/sec、gates（或等价物）/cycle、proof 体积、
  verifier 成本、机器口径。
- **差距分解**：用分阶段数据说明我们慢在哪（M9 已知：电路构建 64%——本轮用新数据
  复核该比例在 M12 结构下是否仍成立）。

### T3：优化候选清单（只分析，不实施）
按预期收益排序列出 ≥5 个候选，每个注明：目标阶段、预估收益、风险（是否动约束/
协议）、工作量。已知候选（供参考，可补充）：
1. 电路按形状缓存/序列化（同 shape 重复证明时消除重建——one-shot 无收益，须注明）；
2. 每周期门数削减（502 g/cyc 的构成分析：排序流断言/M 族断言/译码的按需 gating）；
3. χ-dot 累加的门数开销（M12 引入 +9.4%）是否可以摊薄；
4. witness 填充/序列化的并行化（rayon）；
5. BaseFold 开口批量的参数调优（log_inv_rate/queries/arity）；
6. CircuitStat 惰性化（M10 T5 设计文档已有）。

## 3. 验收标准

1. BASELINE.md 数字可复现（我会抽查一档重跑）。
2. 对照表口径声明完整（RV32/RV64、机器、含/不含构建与预处理）。
3. 候选清单每项有数据支撑的预估。
4. 不改优化代码；全量测试绿。

## 4. 送审要求

`zkvm-project/BASELINE.md` + 简短送审消息。

---

## 附：优化循环协议（M14 验收后生效，Leader 执行）

1. 每轮由 Leader 从候选清单指定**一个**目标（数据驱动），Worker 实施 + 测量。
2. 验收门槛：全量测试绿 + soundness 全套绿 + 同 harness 前后对比数字 +
   **不动语义**（任何约束/协议改动须附论证与 soundness 重跑）。
3. 通过 → Leader commit；不通过 → 回退（git restore），记录失败原因。
4. 每轮结果记入 `zkvm-project/OPT_LOG.md`（目标/改动/前后数字/结论）。
5. 回退不惩罚——负结果也是数据，写入 OPT_LOG。
