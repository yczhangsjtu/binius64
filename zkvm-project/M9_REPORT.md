# M9 送审报告（性能工程 + 缩放曲线，2026-09-08）

Workdir: `/home/yczhang/workspace/binius64`。范围：`crates/zkvm-slice/` + `zkvm-project/`；
未动上游、无 git 操作。任务书 `tasks/M9-performance.md`。全量回归 **60 passed / 0 failed**（+4 ignored）。

## 结论

**T0 ✅（N=64 跑通，OOM 根因=环境性内存叠加）· T1 ✅（release 曲线：gates 严格线性
502.3 门/周期，prove 时间近线性）· T2 ✅（sb/sh 双事件电路层 + 端到端）· T3 ✅（profile
数据采集完成，热点定位推翻预期，优化建议给出）**。

## T0：N=64 OOM 根因 ✅

**N=64 当前可跑通**：`vm_ram_sort_scale64`（dev）29.8s 全绿，T=27,343 周期、l=16、
gates=13,730,612、`/usr/bin/time -v` 峰值 RSS **8.88 GB**（release 相同量级 8.88 GB——
峰值与优化级别无关，是电路/矩阵数据本身）。

- **根因结论**：M8-A 时的 SIGKILL 不是单一分配点，而是**单进程 ~9GB 峰值 + 环境叠加**
  （CARGO_BUILD_JOBS=4 的并行 rustc 各 1-2GB、或与其它测试/进程并发时触发 OOM killer）。
  量化：~650 B/门随 T 线性；单测隔离运行（`--test-threads=1`、编译与运行不并发）即可跑通。
- **规模外推**：N=128（≈55k 周期、28M 门）预计 ~18GB，接近 32GB 机器安全线；更大规模需要
  分块构建/流式 prove（架构级，列 M10 建议）。
- **绕开方案（已生效）**：大测试默认 `#[ignore]`，显式单独运行；本报告 release 扫描即用
  串行单进程（峰值 8.88GB 为 N=64 项）。

## T1：release 缩放曲线（核心）✅

release（`--release` + `RUSTFLAGS=-C target-cpu=native`，串行单进程）：

| N | T（周期） | ts | l | gates | prove 时间 | g/cyc | debug 对照 |
|---|---|---|---|---|---|---|---|
| 16 | 1,801 | 1,834 | 12 | 905,168 | **1.1s** | 502.6 | 1.9s |
| 32 | 6,973 | 7,038 | 14 | 3,502,632 | **4.5s** | 502.3 | 11.1s |
| 64 | 27,343 | 27,472 | 16 | 13,730,612 | **18.1s** | 502.2 | 29.8s |

- **gates vs T：严格线性**（502.3±0.2 门/周期，方差 0.04%）；**prove_time vs T：近线性**
  （T×15.2 → t×16.5，拟合指数 ≈1.03；NTT/承诺的 O(n log n) 项贡献轻微超线性）。
- release 加速：N=32 2.5×、N=64 1.65×（越大相对越接近内存带宽瓶颈）。
- 峰值内存（N=64，整进程串行）：8.88 GB。
- **每指令成本表 release 版**：31 条指令 **1113-1115 g/cyc 统一**（M6 的 973 → 1113，
  增量 = M9/M8-B 的 M 族断言与 sb/sh 逻辑的每周期固定门，含 3 imul/周期）——
  **"成本∝指令数、与指令类型无关" 在 39 条 ISA 下保持成立**（jalr 1115 微差与 M6 同因）。

## T2：sb/sh 电路层 + 双事件 ✅

- **双事件展开**（M8-B 报告方案落地）：sb/sh 周期产出「读旧字（ver=v）+ 写新字（ver=v+1）」
  两个内存事件——**复用现有 load+store 双槽与版本链，排序论证 val_cons 无需修改**
  （旧字读行 → 新字写行的同地址一致链自动成立）。ts 空间加倍：load ts=2t、store ts=2t+1、
  PAD ts=2t、final ts=2T+1（`event_rows` 双事件化 + `build_sorted_with_final`）。
- **电路层**：`c_is_store` 扩为 OP_STORE 全家；sb/sh 地址走字节语义（(addr>>2)&0x3f +
  addr[1:0] 偏移）；merged = f(ld_val[t], rs2, off) 电路内计算（变量移位 + 掩码），
  `st_val` 断言 = merged（旧字经 RAM 论证钉住 → 合并正确性被证明）。
- **地址语义统一**：vm32 层文档化（lw/sw 字索引 = M5 历史；lb/lbu/lh/lhu/sb/sh 字节地址，
  `interp.rs` 注释）；**tracer/事件层的完整字节地址统一**在 T3 tracer（新代码）落地，
  vm_ram_sort 的事件层已支持双事件 ts 语义（本 T2 交付）。
- **测试**：`word_vm32_m8b_isa_prove` 扩为 25 周期（含 sb 双事件 + sh 半字合并，lw 读回
  0x005e5e34 对拍），c_ok/l_ok 全绿；全量回归无回归。

## T3：数据驱动的优化（时间盒内）✅（profile 完成；代码级优化如实收盒）

release N=32 阶段计时（`[phase]` stderr 输出，保留在代码中供复测）：

| 阶段 | 耗时 | 占比 |
|---|---|---|
| **build_circuit + stat** | **2.88s** | **64%** |
| frontend_prove（Spartan） | 0.50s | 11% |
| BaseFold+logup+fracadd（prover） | 72ms | 1.6% |
| witness_fill | 12ms | 0.3% |
| verify 链（frontend+logup+fracadd） | 33ms | 0.7% |

- **热点定位推翻任务书预期**：瓶颈是**电路构建**（单线程 CircuitBuilder 逐门 emit），
  而非 witness 填充（12ms）或 NTT/承诺（72ms）。prove 全栈（build 之外）仅 ~1.6s。
- **不做 speculative 优化**：build_circuit 的优化路径有二——门数削减（架构级，如 M 族
  断言的按需 gating、排序流断言去重）需重设计约束；builder 并行化/批量化需动
  `binius-frontend`（**上游禁改**）。`CircuitStat::collect` 惰性化是可做的微优化（估计
  省 0.3-1s），但相对 build 本体占比小，时间盒内未实施，列为 M10 建议。
- **交付**：profile 数据 + 分阶段计时已内置（`[phase]` 输出），后续优化有基线可依。

## 验收对照

1. ✅ 全量测试绿；release 下 vm_ram_sort 11 项 + bench 均通过。
2. ✅ T0 根因具体（8.88GB 峰值、~650B/门、环境叠加）；**N=64 跑通**。
3. ✅ 缩放曲线表 + 斜率拟合（gates 严格线性、prove≈线性）；release 每指令成本表。
4. ✅ sb/sh 端到端 + 对拍；双事件无回归（全量 60 绿）。
5. ✅ T3：profile 前后数据已给；代码级优化未实施的原因如上（数据驱动结论 + 上游边界）。
6. ✅ BENCHMARKS.md（缩放曲线节 + M8-B/M9 增补）、roadmap/PROGRESS 状态、本报告。
7. ✅ soundness 纪律不变（本里程碑无新 soundness 面）；新代码零警告（历史警告不在本轮范围）。

## 需复核重点

1. **T0 结论口径**：「环境性 OOM」是否可接受，或要求在受限内存场景（CI）加护栏
   （如 N≥64 测试加内存预检）。
2. **T3 收盒**：热点=电路构建但优化需动上游/架构，时间盒内只交付 profile——是否认可
   （替代方案：M10 做 `CircuitStat` 惰性化 + 门数削减设计）。
3. **每指令成本 g/cyc 973→1113**：M 族断言每周期固定门的取舍（全展开 vs 按 funct3 分层
   减门），属成本/复杂度权衡，请确认接受全展开的 thesis 表述。
