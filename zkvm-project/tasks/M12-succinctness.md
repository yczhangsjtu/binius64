# M12 任务书：Verifier Succinctness + F6 收尾

> 里程碑：M12（原 M11 succinctness 顺延；规划：`designs/verifier-succinctness-plan.md`）
> 派发日期：2026-09-08 | 派发方：Leader Agent | 执行方：Worker Agent
> 前置：M11 ✅（安全修复包，65 绿）。本任务书含全部决策；每个 T 是 checkpoint。

---

## 1. 目标

把 verifier 从"线性读公开 inout + 每次重建电路"升级为**预处理模型下的 succinct
在线验证**（规划文档 §3）。同时清掉 M11 留下的 F6 中级尾巴。

## 2. 任务分解

### T1（S1，核心）：公开输入 O(T) → O(1)
按规划文档 M11-A 节施工：
- 逐周期列（inst/pc、排序流 8 列、init_vals）从公开 inout 改为 **committed-only**：
  全部经 BaseFold send_oracle 承诺；验证端只收开口值（oracle relation）。
- 公开输入收敛为：程序哈希 + init 镜像哈希 + 输出 + 规模参数（t_len/ts/l）。
- 注意改造点（规划已列）：leaf-claim 桥翻转（committed 列 + 验证端只拿开口）；
  fetch claims 与 den_check 的数据源从公开列改为开口值；验证端 sortedness_ok 的
  透明检查删除（正确性由电路断言承担——M11 F3 已把事件行钉扎进电路）。
- **验收硬指标**：N=16/32/64 三点的公开 inout 词数**恒定**（实测表）。
- soundness：篡改任一开口值/承诺 → verify 层拒（沿用并适配 M11 的 8+ 例）。

### T2（S2）：verifier 预处理拆分
- `vmrs_verify` 拆为 `vmrs_verifier_setup(shape, t_len, ts, l) -> VerifierKey`
  （一次性：建电路/CS/BaseFold 编译器）与 `vmrs_verify_online(&VerifierKey, proof,
  public_io)`（不重建电路）。
- VerifierKey 缓存复用：同一 key 连验两个不同输入的 proof 的测试。
- 验收：online 路径无 build_circuit 调用（代码证据 + 耗时对比）。

### T3（F6 收尾，批量中级项）
- M1：非标编码两侧语义对齐（LOAD/STORE funct3∈{3,6,7}、R-type funct7 非标值——
  interp 与电路统一行为，选 NOP 或拒绝，文档化）；
- M2：sh 奇地址对齐断言（与 lh 对称）+ 修 isa.rs 虚标注释；
- M3：vm32 引擎补程序哈希对照（对齐 vm_ram_sort）+ ld_val 的 32 位范围检查；
- M5：final 行唯一性断言（防 XOR 归约相消）；
- lh 有符号半字 e2e 覆盖；elf.rs 非 4 对齐 vaddr 段折叠错位修复或边界声明。
- vm_ram_sort 侧显式 ecall 终止断言（M11 F4 的残余弱化闭合）。
- BadEventRow 用例的偏移修正（当前翻转的是 s_kind[0] 而非 s_val 行，注释与
  实际不符——修对齐）。

### T4：报告与文档
- `M12_REPORT.md`；KNOWN_BOUNDARIES #3（verifier 线性）改写为"预处理模型 succinct"；
  SECURITY_REVIEW_PREP 同步；BENCHMARKS 补 online-verify 耗时列。

## 3. 验收标准

1. 全量绿；T1 的公开输入恒定实测表；T2 的 online 不重建电路证据。
2. soundness 在新结构下全部 verify 层拒绝（无 panic 单独成立）。
3. T3 各项有代码行证据 + 对拍。
4. 文档四处更新；新代码零警告。

## 4. 送审要求

`zkvm-project/M12_REPORT.md` + 简短送审消息（≤15 行）。
