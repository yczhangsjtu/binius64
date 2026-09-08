# 电路构建成本：设计与方案草拟（M10 T5）

> 2026-09-08。M9 T3 profile：build_circuit 占 prove 全程 64%（release N=32: 2.88s/4.5s），
> 峰值内存 ~650 B/门（N=64: 8.88GB @13.7M 门）。本文档给出三项改进的设计（不强制本轮实现）。

## 1. `CircuitStat::collect` 惰性化（微优化，低风险）
现状：`run_vmrs` 主路径无条件 `CircuitStat::collect(&circuit)`（全图遍历，估计 0.3-1s @N=32），
仅测试 `show()` 消费。方案：`VmRsProof.stat` 改 `Option<CircuitStat>` 或 `#[cfg(test)]`
收集；主路径零成本。预计 N=64 省 ~1-3s。

## 2. 门数削减（架构级，中风险）
- **M 族断言按 funct3 gating**：当前每周期恒定 3 imul + 比较/选择链（1113 g/cyc 的主要
  增量）。可按 `is_div_family` 用 zerocheck 归约批量验证（每周期断言 → 批量乘积断言），
  或把 div 验证改为「每 K 周期一次批量」——需要把 per-cycle 断言改为聚合式
  （grand-product 风格），省 ~80-120 门/周期。
- **排序流断言去重**：`assert_sortedness` 的 4 组断言/行可折叠为单挑战加权和
  （reduce 到一个 zerocheck），省 ~30% 排序流门。
- 风险：改约束结构 → gates 基线变化 → M6 纪律的「逐数字一致」不再适用（需重跑全量
  soundness 并重新送审数字）。

## 3. N=128+ 流式构建（工程级）
现状：CircuitBuilder 全图驻留（~650 B/门），build() 后约束矩阵再展开（峰值翻倍点）。
方案：a) 分块构建 + per-block prove（把 T 切片为 K 段，段间用排序论证衔接——协议级改动，
等价 "continuation" 证明，Jolt/SP1 同款）；b) 上游建议：builder 的门存储紧凑化
（PathSpec 已 u32 索引 ✓，剩余大头是每门 SideEffect 记录——建议 SoA 布局）。
推荐顺序：先 1（无风险），2 视 thesis 需要选做，3 是 M10 之后的主要工程项。
