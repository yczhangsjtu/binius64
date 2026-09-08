# M13 任务书：fib 形状 completeness 缺口定位（专项排查）

> 派发日期：2026-09-08 | 派发方：Leader Agent | 执行方：Worker Agent
> 背景：M12 报告 §3.9——fib 微程序（T=65/ts=68/L=11）的**诚实**证明在
> BaseFold batched finish 层报 InvalidAssert（l_ok=false）。主测形状（N=16/32/64、
> ELF bubble16）全绿。已排除：维度不一致、挑战流失配、relation claim 失配、
> χ relation 组。怀疑上游 batched-opening 对特定形状的边界，未定位。
> **为什么必须查清**：定位不了的完整性失败可能有未被理解的结构性原因；
> 在查清前，"任意程序可证明"的主张对该形状不成立。

## 任务

1. **最小复现**：从 fib 形状出发做二分——逐步缩小到**最小的失败形状/数据组合**
   （变 T、变 ts、变 L、变列数、变是否含某列），找到触发边界的确切条件。
2. **定位**：在最小复现上追踪到具体的失败断言（上游 finish/batched opening 的哪一行、
   什么不变量被破坏）。允许给上游 crate 加临时 eprintln 调试（不提交改动）。
3. **修复或绕行**：若是我们的用法错误 → 修；若是上游边界 → 给出规避
   （形状约束/padding 规则）并在 KNOWN_BOUNDARIES 如实登记。
4. **回归测试**：fib 形状的完整 honest prove→verify 断言恢复（删除 M12 的弱化注释）。

## 验收标准

1. 最小复现 + 根因（具体行/不变量）。
2. 修复或规避后 fib 端到端全绿；全量测试不回退。
3. M13_REPORT.md 记录排查过程与结论。

## 送审要求

`zkvm-project/M13_REPORT.md` + 简短送审消息。
