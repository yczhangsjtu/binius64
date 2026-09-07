# M3 任务书：通用单周期状态机（word 译码 + 寄存器堆 + 版本链电路化 + PC 推进）

> 里程碑：M3（权威定义见 `zkvm-project/designs/milestone-roadmap.md` §4）
> 派发日期：2026-09-06 | 派发方：Leader Agent | 执行方：Worker Agent
> 前置：M1 ✅（reg_rw 写日志表读==写绑定）、M2 ✅（词级门 + logup* 同 transcript，W2 选型）
> 本任务书包含 Leader 已完成的设计决策（§3），Worker 按此实现，不要另起架构。

---

## 1. 目标与定位

M3 把 M1 和 M2 两块已实证的基石**合流**，并补上 M2 遗留的语义绑定缺口：

- M1 给了"读==写绑定"（logup* 写日志表 W[(reg,ver)]），但**版本链是 native 算的**；
- M2 给了"词级门执行 + logup* 同 transcript"，但**取指 word 未驱动执行**（无译码绑定）。

**M3 交付一台最小但真实的单周期状态机**：多指令程序（add/addi/beq，含循环），
单一 Fiat-Shamir transcript 内同时证明：
1. **取指**：logup* 程序表 T[pc]=word；
2. **译码**：指令 word 经词级门解码出 opcode/rd/rs1/rs2/imm——**word 驱动执行**（语义绑定）；
3. **执行**：add/addi 走词级门（iadd_32），beq 走 icmp_eq + select；
4. **寄存器堆**：8 个 32-bit 寄存器，每个读事件经 logup* 绑定到写日志表
   W[(reg, ver_at_read)]（读==最近写）；
5. **版本链电路化**：`ver_{t+1}[r] = ver_t[r] + (该周期写 r ? 1 : 0)` 由**电路约束**承载
   （不再由 native 预计算）——这是 M1 验收遗留的硬边界，M3 必须闭合；
6. **PC 推进**：`pc_{t+1} = beq_taken ? target : pc + 4`，词级门约束。

这是"逐行验证器 zkvm.rs"和"演示 full_vm_*"都不曾做到的东西：**读值由论证强制
（非注入）、版本时序由电路强制（非 native）、执行由 word 译码驱动（非 match 枚举）**。

## 2. 范围与边界（严格遵守）

- 只允许修改/新增：`crates/zkvm-slice/` 与 `zkvm-project/`。禁止改上游 crates，禁止 git 操作。
- 构建：`export RUSTFLAGS="-C target-cpu=native"` + `CARGO_BUILD_JOBS=4`。
- 诚实分级纪律同前：⚠️ 区不得表述为完整实现；soundness 必须是 verify 返回 Err/断言失败，
  不得靠 witness build panic"拒假"。
- **规模纪律**：固定轮数全展开、无 RAM（M4 的事）、程序表 native 给定（只证取指一致性，
  与 M2 同边界）。不要顺手做 M4/M5 的内容。

## 3. Leader 设计决策（Worker 按此实现）

### 3.1 技术栈
- 执行/译码/PC/版本链：**binius_frontend 词级门**（W2 路线，M2 已选定）。
  参考 `zkvm-project/designs/binius64-frontend-api-map.md` 的门集映射；MSB-Boolean 约定
  （布尔在 bit 63，select/icmp 读 MSB）。
- 查表：logup*（取指表 + 寄存器写日志表，两表同 transcript，参照
  `mem_arg_ts.rs` 的多表用法与 `word_add_combined.rs` 的同 transcript 模式）。
- transcript：`ProverTranscript<HasherChallenger<Sha256>>`（即 StdChallenger），
  顺序：frontend prove → sample γ → logup* prove → into_verifier → frontend verify →
  sample γ → logup* verify_reduction。

### 3.2 语义绑定方案（关键设计，闭合 M2 遗留缺口）
所有需要被 logup* 引用的值，一律走**电路 public inout word**，验证端从同一份 inout
重算 logup* claim：
- 指令 word：每周期一个 inout word。电路内由它译码出全部字段（译码正确性由门约束承载）；
  logup* 取指 claim 的值 = 该 inout word。任一侧篡改即 verify 失败——**word 驱动执行成立**。
- 寄存器读/写事件：每事件暴露 (reg, ver, value) 为 inout（或打包进 word，自行选择布局，
  报告中说明）。读事件的 value 同时是执行层的 ALU 输入 wire——**读值被论证强制**，
  不是注入。写事件的 value 是 ALU 输出/译码结果的 wire。
- logup* 表 claim 的 index 由 (reg, ver) 计算（`reg * VER_MAX + ver`，同 reg_rw），
  index/claim 都在 verify 端从 inout 重建——Worker 需确认 logup* 的 LookerClaim 值
  如何与 inout 对齐（参照 reg_rw.rs 与 word_add_combined.rs 的现有做法）。

### 3.3 版本链电路化（M1 遗留硬边界）
- 每周期每寄存器一条 ver wire（NREG=8，T≈8 周期，规模可控）。
- 约束：`ver_{t+1}[r] = ver_t[r] + is_write_r`，其中
  `is_write_r = (译码得 rd==r) AND (该指令写回)`——用 icmp_eq + select/band 实现，
  增量用 iadd_32（或 inc 类门）。
- 初始 `ver_0[r] = 0`（常量约束）；读事件用的 ver = 该周期**读时刻**的版本 wire。
- **禁止**由 native 代码算好 ver 序列再填进去——ver 必须由电路从初始值逐周期推出
  （native 只提供指令序列本身）。

### 3.4 参考程序（可微调，但必须满足右侧性质）
```
0x00: addi x1, x1, 1      # x1 计数
0x04: add  x2, x2, x1     # x2 累加 x1        （跨指令寄存器依赖：读见最近写）
0x08: addi x3, x3, 1      # i++
0x0c: beq  x3, x4, +8     # i==limit 则跳到 0x14 退出（x4 预置 limit）
0x10: beq  x5, x5, -16    # 无条件跳回 0x00（x5==x5 恒真）
0x14: （结束/pad）
```
性质要求：≥6 个执行周期；≥3 个活跃寄存器；**同一寄存器被写 ≥2 次**（版本绑定被真正
检验）；taken 与 not-taken 分支各 ≥1 次；x1/x2 初值、x4=limit 由输入给定。
x0 处理：可以不实现 x0 硬零（NREG=8 全部当真实寄存器），程序避免依赖 x0 即可；
若实现了 x0 恒零约束，作为加分项写进报告。

### 3.5 交付切片
- 新切片 `crates/zkvm-slice/src/slices/word_vm.rs`（一个文件装下；若确实过长可拆
  `word_vm.rs` + `word_vm` 辅助模块，但优先单文件），`pub fn run_word_vm()` + `#[test]`，
  lib.rs 注册 + `pub use`。
- 结构骨架沿用项目惯例：native 跑 ground truth → 建电路 → witness → 单 transcript
  prove → verify → soundness 用例组。

## 4. soundness 用例（≥4，全部必须是 verify 层拒绝）

1. **过期读**：某次读 claim 该寄存器旧版本的值 → 拒。
2. **版本篡改**：读事件的 ver 改成错的版本号 → 拒。
3. **非法取指**：某周期 claim 程序表中不存在的指令 word → 拒。
4. **结果篡改**：最终寄存器 public 输出改值 → 拒。
5. （加分）**分支目标篡改**：beq taken 时 pc_next 改成非 target → 拒。

## 5. 任务分解

- **T1 设计落实（先写后码）**：读 M2_REPORT.md、word_add_combined.rs、reg_rw.rs、
  binius64-frontend-api-map.md；把 inout 布局、译码门方案、ver wire 布局写成报告 §1
  的设计小节（半页以内）。发现本任务书 §3 有不可行处时，**停下来在报告中记录原因并
  给出替代方案**，不要静默改架构。
- **T2 电路层**：译码 + 执行（add/addi/beq）+ PC 推进 + 版本链，词级门实现。
- **T3 查表层**：取指表 + 写日志表，同 transcript 组合。
- **T4 soundness**：§4 的用例 1-4 必做，5 加分。
- **T5 成本与报告**：CircuitStat 统计（ZERO/AND/IMUL/BMUL），按"每指令平均约束数"
  分析（thesis 指标）；写 `zkvm-project/M3_REPORT.md`（体例沿用 M1/M2 报告）；
  更新 `crates/zkvm-slice/README.md`（切片 24）、`zkvm-project/PROGRESS.md`（M3 节）、
  `zkvm-project/designs/milestone-roadmap.md`（M3 状态）、`zkvm-project/README.md`
  （切片表加一行）。

## 6. 验收标准（Leader 逐项核对）

1. `cargo test -p binius-zkvm-slice` 全过（应为 24 passed）。
2. **版本链确由电路承载**：报告中能指出 ver 递增约束的具体代码行；把 ver 序列改由
   native 预计算的做法一律不收。
3. **word 译码驱动执行**：能指出 word→opcode/操作数字段的门约束代码行；`match` 枚举
   驱动执行的做法一律不收。
4. soundness 用例 1-4 全部为 verify 层拒绝，且各自独立触发。
5. 报告含成本统计与每指令成本分析；边界（固定展开、无 RAM、程序表 native 给定）
   如实标注。
6. 文档四处更新齐全，切片计数=24。

## 7. 送审要求

完成后：送审材料落成 `zkvm-project/M3_REPORT.md`，然后只回一条**简短送审消息**
（≤15 行：结论一句话、文件清单、测试结果一行、需 Leader 复核重点 1-3 条）。
细节一律在报告里。
