# M7 送审报告 v2（可扩展 RAM 论证 spike，Phase 2 咽喉，2026-09-08）

> v2 返工（任务书 `tasks/M7-rework.md` F1-F3）：F1 soundness 增加 verify 层拒绝形态
> （诚实 prove + 验证端篡改 → `l_ok/c_ok == false`）；F2 `den_check` 改用 verify_oracle_relation
> 绑定的开口值（删除验证端从 native case 重建列的路径）；F3 §r2 路线 B 表述更正
> （Twist ≠ 置换网络；阻塞点是 degree-3 自定义复合 evaluator）。
> 核心成果（fracaddcheck 多重集合、K 无关缩放）在 v1 已验收，本次为纪律性返工，协议不变。

Workdir: `/home/yczhang/workspace/binius64`。范围：`crates/zkvm-slice/` + `zkvm-project/`；未动上游、无 git 操作。
按任务书 T0 → T1 执行；T2（完整 VM 接入）未启动，等 M8 任务书。

## 结论

**切片 27（`ram_sort`，排序式离线内存检查）落地并全绿**：9/9 测试通过（诚实 1 + soundness 8：
prover 数据 4 + **verify 层 4**），全量回归 **48/48**。核心新协议工作 = 用 `FracAddCircuit`
自组装"值多重集合等式"（logup_star 成品 API 分子形态不符）。**缩放结论不变：电路 gates 与
地址空间 K 完全无关（K 翻倍 0 方差）、随访问条数 T 线性（T×4 → gates×3.93）**。

## T0：committed 列绑定升级（v1 已验收，本次无改动）

- 4 列（addr/val/ts/kind）`send_oracle` 承诺（承诺先于挑战 ρ/c），fracaddcheck 归约出口
  `prove_oracle_relation` ×4 绑定；公开 inout 仅 `final_out` 一词，不随 T 增长。

## T1：路线 A（v1 已验收协议，本次 F1/F2 触达验证端）

### F2（返工）：den_check 数据源 = 绑定开口值
v1 的验证端曾 `build_cols(&case, nrows)` 从 native case 重建列算 a/v/t/k_eval
（naive crutch）；**v2 删除该路径**——`den_check` 直接用
`addr_r/val_r/ts_r/kind_r`（prover 经 `prove_oracle_relation` 传输、验证端经
`verify_oracle_relation` 绑定的列开口值），一行表达式换数据源。**naive 通道的 crutch 现在
只剩"承诺非密码学强度"一项**（强通道迁移属 M8，表述见 §r2/边界 1）。

### F1（返工）：soundness 8 例（4 prover 数据 + 4 verify 层）

| # | 篡改 | 期望层 | 结果 |
|---|---|---|---|
| 1p 过期读 | read val 改旧值（两侧同步，prover 数据）| 电路 | rejected ✓（prover 构造失败 panic，次要证据）|
| 2p 丢访问 | 事件侧删一行（prover 数据）| logup | rejected ✓（同上）|
| 3p 多塞 | 排序侧多塞一行（prover 数据）| logup | rejected ✓（同上）|
| 4p 改读值 | 只改事件侧 val（prover 数据）| logup | rejected ✓（同上）|
| **1v** 过期读 verify 形态 | 诚实 prove + **验证端篡改公开输出 final_out** | 电路 | **`c_ok == false`** ✓（无 panic；电路拒后 transcript 已消费，l_ok 不续验）|
| **2v** 丢访问 verify 形态 | 诚实 prove + **验证端篡改分数和声明 root_den**（Σ 声明与列不符的验证端表象）| logup | **`l_ok == false`** ✓（fracaddcheck 归约 `Verification(InvalidAssert)` Err）|
| **3v** 多塞 verify 形态 | 诚实 prove + **验证端篡改 addr 开口值**（den 组合先行揭穿）| logup | **`l_ok == false`** ✓ |
| **4v** 改读值 verify 形态 | 诚实 prove + **验证端篡改 val 开口值** | logup | **`l_ok == false`** ✓ |

verify 层 4 例全部**无 panic、断言落在 verify 层**（`l_ok/c_ok == false`），满足
ACCEPTANCE_BASIS 纪律（panic 不再单独成立，只作 prover 数据例的次要证据）。

### 缩放（v1 复测，不变）
| K | T | gates（ZERO/AND/BMUL）| 时间 |
|---|---|---|---|
| 2^12 | 2^10 | 81,880 | 0.2s |
| 2^16 | 2^10 | **81,880（与 K 无关，0 方差）** | 0.2s |
| 2^12 | 2^12 | 321,770 | 0.7s |
| 2^16 | 2^12 | **321,770（与 K 无关，0 方差）** | 0.6s |

T×4 → gates×3.93（线性）。T=2^8 诚实单测 20,000 gates（ts=572）。

## 已知边界（v2 精确化）

1. **承诺强度**：切片用 naive 通道（oracle = 全系数）——即"承诺非密码学强度"，
   未涵盖 ZK 掩码/BaseFold；强承诺 + ZK 是现成能力（`OracleSpec::is_zk`），迁移属 M8
   （BaseFold 通道 + 归约点批量 open）。
2. **恒等式②电路的绑定**：电路作用于 private witness 列，与 committed 列在强承诺通道下的
   严格绑定需要 quadratic-mlecheck 归约；诚实路径两副本同源，本切片不伪造该绑定。
3. **左值集合的推导性**：事件流为合成（本切片无执行电路）；与真执行电路的产线接合
   （执行 wire → 事件列）属 M8 的 word_vm_ram committed 化。
4. 电路拒（例 1v）后 verifier 已消耗 transcript，logup 流不再续验——l_ok 无意义，断言只落在 c_ok。
5. `fracaddcheck::verify` 的 `k = ceil(log2(2·ts))`（列长 2^l，pad 分子 0/分母 1 零分数）。

## §r2：备选路线评估（v2 重写，F3 更正）

- **路线 A（本切片落地）**：排序 + 相邻断言。成本 = 恒等式①一个 fracaddcheck（GKR 全归约）
  + 恒等式②词级电路（~20k 门 @T=256，随 T 线性）。**K 不进入任何约束**——咽喉卖点实证成立。
- **路线 B（Twist 忠实翻译）——正确表述**：Twist 不是排序/置换论证，而是
  **one-hot rang-check + committed increment 链 + Val 链 + LT（less-than）比较的 sumcheck 族**；
  其真正阻塞点是**degree-3 自定义复合式无现成 evaluator**（设计详案 §3.2：需要为
  三次复合式（如 Val 链的乘法结构）自写 sumcheck 归约求值器，Binius64 现成 evaluator
  覆盖二次）。选型结论（路线 A 胜）不变：A 的恒等式①归属 fracaddcheck 成品归约（GKR），
  B 需从零写 degree-3 evaluator 且叠加 committed wiring 绑定（与 A 恒等式②的绑定问题同构）。
- **ZK 可行性**：全部列/电路在 Binius64 frontend+iop 栈（BinaryField128b / 词级 gate）；
  ZK 掩码在 BaseFold 通道是现成能力，迁移路径清晰。
- **M8 迁移清单**：① 执行电路（word_vm_ram committed 化）→ 事件列产线；② 恒等式②的
  mlecheck 绑定（强承诺）；③ 强承诺通道迁移（BaseFold、批量 open）；④ 左值集合推导性
  由执行电路 witness 保证。

## 文件清单

- `crates/zkvm-slice/src/slices/ram_sort.rs`（切片 27）——gen_case/电路/fracaddcheck 组装/
  prover/verifier（F1: `Tamper` 枚举 + verify 层篡改；F2: den_check 绑定开口值）/测试
  （诚实 + 8 soundness + 缩放 #[ignore]）
- `crates/zkvm-slice/src/lib.rs` — 切片 27 注册 + `pub use run_ram_sort`
- `crates/zkvm-slice/Cargo.toml` — +binius-iop、binius-iop-prover
- 未动：word_vm32 及其它老切片、上游任何 crate、无 git 操作

## 测试结果

- `cargo test -p binius-zkvm-slice --lib` → **48 passed / 0 failed / 2 ignored**
  （44 旧 + F1 verify 层 4 例；ram_sort_scale 与 bench_instruction 为 #[ignore]）
- 缩放（--ignored ram_sort_scale）→ 4 点全过，K 无关断言 ratio=1.000，T 线性 3.93
- 新代码零警告

## 需复核重点

1. F1 verify 层 4 例的断言落点（c_ok/l_ok false，无 panic）与任务书修法对照；
2. F2 后验证端数据源只有 inout/transcript/绑定开口值（`build_cols` 在验证路径零引用）；
3. §r2 路线 B 的新表述（Twist 真实内涵 + degree-3 evaluator 阻塞）是否与设计详案 §3.2 一致；
4. 缩放 0 方差与 T 线性（咽喉定量证据，v1 复测不变）；
5. 边界 4（例 1v 后 l_ok 不续验的表述）。