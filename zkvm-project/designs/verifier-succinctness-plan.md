# Verifier Succinctness 规划（M11 前置调研与方案）

> 日期：2026-09-08 | 地位：M8-C 之后**优先推进**的方向的规划文档（用户指定）。
> 依据：Jolt 五路调研、`crates/verifier/src/verify.rs` 核实、KNOWN_BOUNDARIES #3。

---

## 1. 参照系：Jolt 怎么做到 succinct

Jolt verifier = polylog(T)：sumcheck 链（O(log T) 轮）+ Dory 开口（O(log T) 群运算），
公开输入仅程序/IO。两个支撑设计：
- **witness 全 committed**，验证端只见开口值；
- **uniform R1CS**：约束矩阵只有单周期一份（O(1) 大小），验证端靠
  `M̃(r_x, r_y) = eq̃(cycle) · M̃_local(con, var)` 因子分解本地求值，从不物化 T 份。

可借鉴的是这两点**架构思想**；Dory/素域 sumcheck 的代码不可移植（迁移评估已有结论）。

## 2. 我们的 verifier 线性成本解剖（现状，实测/代码核实）

| 来源 | 量级 | 证据 |
|---|---|---|
| 公开 inout 读入（inst/pc 逐周期 + 排序流 8 列） | O(T) 词（~18.5k @N=16） | KNOWN_BOUNDARIES #3 |
| 电路描述：verifier 构建/持有完整 ConstraintSystem | O(T) 门（按周期展开） | `crates/verifier/src/verify.rs:260` `setup(constraint_system,…)` |
| 密码学工作（BaseFold 开口、fracadd/logup 归约、sumcheck 验证） | polylog(T)（FRI 层） | 实测 verify ~39ms @N=32，大头是前两项 |

**结论：succinct 缺口 = S1（inout 线性）+ S2（电路描述线性），两个都要处理。**

## 3. 方案：预处理模型下的 succinct 在线验证

采用 PLONK/Jolt 式**预处理模型**合法化 S2：把约束系统构建定义为一次性
**verifier 预处理（verifier key 生成）**，在线验证只读 O(1) 公开输入 + proof +
polylog 工作。这是文献中合法的 succinct 口径（Jolt 的 bytecode/RAM preprocessing
同型）。**不必**为此重写协议栈。

### M11 分两步（每步独立可验收）

**M11-A（S1：公开输入 O(T) → O(1)）**
- 把 inout 里的逐周期列（inst/pc、排序流 8 列、寄存器事件列）全部改为
  **committed-only**：prover 提交 oracle，验证端只收开口值（oracle relation）。
- 公开输入收敛为：程序哈希 + 初始镜像哈希 + 输出 + 规模参数（T、l）。
- 关键改造点：M8-B 的 leaf-claim 桥当前靠"公开列 + 验证端重算"——翻转为
  "committed 列 + 验证端只拿开口"；`claims_from_inout` 的取指/事件 claims 改为从
  开口值重建；恒等式②的验证端透明检查（sortedness_ok 读公开列）删除，
  正确性完全由电路内断言承担。
- 风险：恒等式①的 den_check 当前用公开列重算值——改用开口值后语义不变
  （M7 v2 已证明这条路径）；fetch 的 claims 从 pc/inst 公开词重建的习惯要改成
  开口绑定。
- 验收：证明闭环不变（全测试绿）；**公开输入大小与 T 无关**（实测：N=16/32/64
  的 inout 词数恒定）；新增 soundness：篡改某开口值 → 拒。

**M11-B（S2：verifier 预处理 / 在线拆分）**
- `vmrs_verify` 拆为 `vmrs_verifier_setup(program_shape, t_len) -> VerifierKey`
  （一次性，构建电路/CS/BaseFold 编译器）与 `vmrs_verify_online(&VerifierKey, proof,
  public_io)`（不重建电路）。
- VerifierKey 可缓存/复用（同程序同规模多次验证不重复构建）。
- 验收：online 验证不再调用 build_circuit（代码证据）；同一 VerifierKey 连验两个
  不同输入的 proof；测试绿。

### 明确不做（记录，防范围蔓延）
- **proof 体积亚线性**（当前 ~557KB@N=16 随 T 线性）：需要递归/证明聚合，
  属更后期阶段（Phase 3）。
- **真·无预处理 succinct**（验证端连 O(T) 预处理都不做）：需要 uniform 约束系统
  重写（绕开 frontend 门列表，直接在 trace 列上写 shift 约束）或递归——评估为
  Phase 3 议题，届时以 M11 的数据决策。

## 4. 与路线图的衔接

- 编号：**M11**（succinct verifier），在 M8-C（真实编译程序端到端）验收后启动——
  用户指定的优先级。
- M11 完成后，口头口径可从"verifier 线性"更新为"预处理模型下 succinct 在线验证"
  （KNOWN_BOUNDARIES #3 相应改写）。
- M10 的 API（vmrs_prove/vmrs_verify）是 M11 的改造对象：verify 签名演化为
  setup/online 两段。

## 5. 验收 M11 时的核查点（预登记）

1. 公开输入词数 vs T 的实测表（必须恒定）。
2. online verify 不触发 build_circuit（代码路径证据 + 耗时对比）。
3. 全部 soundness 用例在新结构下仍 verify 层拒绝。
4. 报告须区分"预处理模型 succinct"与"无条件 succinct"，不得混用表述。
