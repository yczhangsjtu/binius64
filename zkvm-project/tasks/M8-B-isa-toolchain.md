# M8-B 任务书：fetch 论证恢复 + ISA 补全 + 真实工具链

> 里程碑：M8 下半（权威设计 `designs/binary-zkvm-detailed-design.md` §2/§5）
> 派发日期：2026-09-08 | 派发方：Leader Agent | 执行方：Worker Agent
> 前置：M8-A ✅（vm_ram_sort：VM × 排序式 RAM 论证 × BaseFold 通道）。
> **M8-A 遗留的两项是本轮的 T0/T1，优先于一切新功能。**
> 本任务书含全部架构决策；Worker 施工；每个 T 是一个 checkpoint，做不完就停在
> checkpoint 如实报告（参考 M5 的诚实先例），不要赶工缩水。

---

## 0. 背景：M8-A 留下的两个语义缺口（必须先闭合）

M8-A 的 vm_ram_sort 里，**程序是私有的**（指令字是纯 witness，无 fetch 表绑定）——
证明陈述是"存在某个程序的执行产生 final_out"，而非"这个公开程序的执行"。
这是 VM 的语义硬伤，M8-B 的 T0 闭合它。（注：M8-A 报告把 fetch 写成"留 M9"——
**更正：fetch 是正确性组件，属本轮 T0**，M9 是性能里程碑。）

## 1. 任务分解（按序，每步一个 checkpoint）

### T0（checkpoint 1，必须）：fetch 论证恢复 + 程序公开化
- 程序镜像作为 committed 表 + **公开程序哈希**（哈希做公共输入）；
  取指走 indexed logup*（M3/M5 已验证的模式，但表改为 committed）。
- 指令字 witness 与取指 claim 绑定：每周期 inst[t] witness 列 ↔ fetch 表
  `T[pc[t]]`（沿用 claims 从 witness 重建的纪律；committed 列绑定用 M8-A 的
  oracle relation 模式）。
- **soundness**：换程序（同一哈希/不同镜像）或篡改取指 claim → verify 层拒。
  此例直接证明"执行的==取指的==公开的程序"。

### T1：leaf-claim 桥（witness↔oracle 逐元素绑定）
- M8-A 的 witness 列与 committed oracle"同源但无逐元素密码学绑定"。
  用 intmul phase5 模式（`crates/prover/src/protocols/intmul/prove.rs:299-348`：
  iota 嵌入 + per-bit evals + shift reduction）把两者绑死。
- 验收形态：篡改 witness 列而保留 oracle（或反之）→ verify 层拒。

### T2：ISA 补全（设计已定，照设计详案 §2.6/§2.7 施工）
- `mul`（frontend imul 门，W2 原生）；`div/divu/rem/remu`（Jolt 虚拟指令展开：
  advice 商 + 断言序列，见设计详案 §2.6 的逐步序列）；
- 字节/半字访存 `lb/lh/lbu/lhu/sb/sh`（展开为对齐字访问 + 掩码/移位 + 对齐断言）；
- 每条新指令进 per-instruction native 单测（M6 测试网的惯例）。

### T3：真实工具链（纯工程量，无新协议）
- riscv32 ELF 加载器（参照 Jolt `jolt-program/src/image/elf.rs` 的段过滤逻辑，
  RV32 版）+ tracer（vm32/interp 演进：读 ELF 镜像执行、产出 trace）+
  内存布局（I/O 区/程序区/stack/heap，参照设计详案 §2.8，4 字节字粒度）。
- 端到端：riscv32 工具链编译的 C 程序（冒泡排序或斐波那契）→ ELF → tracer →
  prove→verify + 与参考执行对拍。
- 若本机无 riscv32 工具链：记录该外部依赖，用预编译的测试二进制（提交到
  `zkvm-project/` 或 crate 的 testdata/）替代现场编译。

## 2. 验收标准

1. 全量测试绿；T0-T2 各有独立 soundness/对拍测试。
2. **程序公开性**：指出公开程序哈希作为公共输入的行 + 取指 claim 与 inst witness
   的绑定行；"换程序被拒"用例通过。
3. leaf-claim 桥的绑定证据行 + 篡改用例。
4. ISA 覆盖：mul/div/rem/字节访存各至少一条端到端 + native 对拍。
5. 真实编译程序端到端（或工具链缺失的诚实记录 + 预编译二进制证据）。
6. 成本数据更新（BENCHMARKS.md 增补新指令行）；文档四处更新（切片/里程碑状态）。
7. 全程 verify 层 soundness 纪律；新代码零警告。

## 3. 送审要求

完成后：`zkvm-project/M8B_REPORT.md` + 简短送审消息（≤15 行：结论、各 T 状态、
文件清单、测试结果、需复核重点）。若只完成部分 T，停在最近的 checkpoint 如实送审，
报告里写清哪些 T 未完成及原因。
