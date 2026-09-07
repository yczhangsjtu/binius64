# M7 任务书：可扩展 RAM 论证 spike（Phase 2 咽喉）

> 里程碑：M7（权威设计见 `designs/binary-zkvm-detailed-design.md` §3/§4.1，**先读它**）
> 派发日期：2026-09-07 | 派发方：Leader Agent | 执行方：Worker Agent
> 前置：Phase 1（M1-M6）✅；Binius64 协议构件调研结论（下引）。
> 本任务书包含全部设计决策（§2-§4）；Worker 负责实现，遇不可行处停下来记录并给替代方案，
> 不要静默改架构。

---

## 1. 目标

把 M4 的 RAM 论证从 **O(K·T) 版本链**升级为**与 K 无关**的可扩展论证。
M7 是 Phase 2 唯一的新协议工作，交付物是**两条候选路线的实测对比 + 选型决策**。

**范围纪律（与 M3-M5 不同）**：M7 是**独立内存论证切片**，不挂完整 VM——
用合成的对抗性访问流（含同地址反复写、交错地址、读旧值陷阱），K=2^16 地址、
T≥2^12 访问。接入 VM 是 M8 的事。

## 2. T0（前置，必做）：committed 列绑定升级

M3 v2 的"事件 inout + claims_from_inout"在切片规模正确，但 inout 是公开输入，
O(T) 公开输入不可扩展。T0 把访问事件列改为 **committed witness 列**：

- 用 IOP channel：`send_oracle(buffer) -> Oracle` 提交派生列
  （参照 `crates/iop-prover/src/logup_star.rs:95-100` 的 pushforward 承诺范例）；
- 叶子 claim 经 `prove_oracle_relation` 归约到 oracle 开口（参照
  `crates/iop-prover/src/logup_star.rs:118-125` 与 intmul phase5
  `crates/prover/src/protocols/intmul/prove.rs:299-348` 的 index-claim 绑定模式）；
- T0 的验收形态：**M4 的 word_vm_ram 语义不变、但 RAM 事件不再占公开 inout**——
   verifier 公开输入只剩 init/final/output 相关。若 T0 证明 oracle 绑定胶水不可行，
  记录为路线阻塞点并给出最小替代（这是允许阴性结论的一步，但要给出具体失败点）。

## 3. T1（必做）：路线 A —— 排序式离线内存检查

构造（设计已定，照此实现）：

1. **事件流**：每条访问 `(addr, ts, val, is_write)`，ts = 访问序号（0..T）。
2. **排序流**（prover witness，committed 列）：同样记录按 `(addr, ts)` 排序，
   外加每触及地址一条 **init 记录**（ts=0，val=初始内存值）与一条 **final 记录**
   （ts=T，kind=final）。
3. **恒等式①多重集合相等**：排序流 == init ∪ 事件流 ∪ final。
   指纹 `f = addr + ρ·val + ρ²·ts + ρ³·kind`（ρ 为 transcript 挑战；注意 char-2：
   ρ 幂次系数的 GF(2)-线性无关性需按域次数论证，在报告中给出依据）。
   **用 `fracaddcheck` 组装真 logUp 分数和** `Σ 1/(c−f_排序) − Σ 1/(c−f_事件侧) = 0`：
   分子 = 全 1 透明列，分母 = `c − f(列)`。
   ⚠️ **不要用 `logup_star::prove`**——其分子固定为 `γ^j·eq_r`、分母为 `c−位置`，
   是 indexed-lookup 形态，证不了值多重集合（构件调研 §6 结论）。
   参照 `crates/ip-prover/src/fracaddcheck/circuit.rs:60` 的 `FracAddCircuit::build`。
4. **恒等式②排序良构 + 读一致性**：排序流相邻对 (S_j, S_{j+1})：
   (a) addr 非降；(b) 同 addr ⇒ ts 严格增；(c) 同 addr 且 S_{j+1} 为读/final ⇒
   `val_{j+1} == val_j`。比较用前端词级门（icmp_ult）或对列做 quadratic mlecheck
   （`quadratic_mlecheck_prover`，claim=0 即 zerocheck）——二选一，报告说明理由。
5. **三件套**：init 记录对照公共初始镜像；final 记录供 output 检查（M4 语义）。
6. **soundness ≥4 例（verify 层拒绝）**：过期读（排序流里配对到旧值）、丢一条访问、
   排序流多塞一条、读值≠排序前驱值。

## 4. T2（条件触发）：路线 B —— Twist 忠实翻译

**触发条件**：T1 完成且数字达标（见 §6.3）后**由 Leader 决定**是否启动 T2
（默认预期 A 胜出则 T2 可取消）。Worker 本任务只做 T0+T1+T3 的路线 A 部分；
T2 会另发任务书。

## 5. 参考代码（先读）

- `crates/ip-prover/src/fracaddcheck/`（circuit.rs / prove.rs）— 路线 A 核心构件
- `crates/ip/src/fracaddcheck.rs` — verifier 侧归约
- `crates/iop-prover/src/logup_star.rs:95-125` — send_oracle + prove_oracle_relation 范例
- `crates/prover/src/protocols/intmul/prove.rs:299-348` — index claim 绑定回 witness 的完整模式
- `crates/ip-prover/src/sumcheck/quadratic_mle_evaluator.rs:98` — 任意二次复合 mlecheck
- `crates/zkvm-slice/src/vm32/`（M6 库）与 `slices/word_vm_ram.rs`（M4，inout 模式现状）

## 6. 验收标准

1. `cargo test -p binius-zkvm-slice` 全绿（新增切片测试计入）。
2. **T0**：RAM 事件 committed 化的证据（指出 send_oracle / prove_oracle_relation 调用行）；
   公开 inout 不再随 T 线性增长。
3. **T1 路线 A**：K=2^16、T≥2^12 合成访问流下 prove→verify 闭环 + 4 例 soundness；
   **缩放实测**：K ∈ {2^12, 2^16} × T ∈ {2^10, 2^12} 至少 4 点的 gates/prove 时间，
   证明成本与 K 无关（K 加倍、T 不变时成本基本不变）。
4. soundness 全部 verify 层拒绝；多重集合等式必须真的用 fracaddcheck 组装
   （指出 FracAddCircuit 调用行），不是 logup_star 套用。
5. 报告 `zkvm-project/M7_REPORT.md`：构造细节、缩放数据表、路线 A/B 初步对比
   （B 只需接口可行性判断）、诚实边界。
6. 文档更新：crate README（切片 27）、项目 README 切片表、PROGRESS M7 段、
   milestone-roadmap（M7 状态）。

## 7. 送审要求

完成后：`zkvm-project/M7_REPORT.md` + 简短送审消息（≤15 行：结论、文件清单、
测试结果一行、缩放结论一句话、需复核重点 1-3 条）。
