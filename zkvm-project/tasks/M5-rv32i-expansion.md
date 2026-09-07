# M5 任务书：真 RV32I 子集 + 32-bit 字宽完整性 + native 对拍

> 里程碑：M5（权威定义见 `zkvm-project/designs/milestone-roadmap.md` §4）
> 派发日期：2026-09-07 | 派发方：Leader Agent | 执行方：Worker Agent
> 前置：M2 ✅（W2 词级选型）、M3 ✅ v2（word_vm：译码驱动+版本链电路化+inout 绑定）、
> M4 ✅（word_vm_ram：RAM 写日志论证 + 三件套）。
> 本任务书 §3 是 Leader 的架构决策，Worker 按此实现；发现不可行处停下来在报告中记录
> 原因并给替代方案，不要静默改架构。

---

## 1. 目标与定位

M1-M4 的机制全部在**简化玩具编码**上验证（7-bit 立即数、8 寄存器、x0 非硬零、
自定义 funct3 用法）。M5 把这台状态机升级为**真实 RV32I**：

1. **真编码**：指令字必须是标准 RV32I 编码（opcode/funct3/funct7、I/B/S/U/J 五种立即数
   布局、符号扩展），废弃 M3/M4 的简化 imm7 布局。译码由词级门完成（band/srl32/shl/bor）。
2. **全子集**：RV32I word 级指令全覆盖（见 §3.1 清单），x0 硬零，32 个寄存器。
3. **跑更大程序**：交付 ≥1 个"真算法"程序（冒泡排序，见 §3.4），native-vs-proof 对拍。
4. **指令覆盖表**：每条指令至少在一个已证明程序中被执行 + 边界操作数（负数、符号扩展、
   移位 31、slt 有/无符号等）的 native 对拍。

**M5 不做**：字节/半字 load/store（lb/lh/lbu/lhu/sb/sh）、ecall/ebreak/fence、
RV32M 的 div/rem（div 需 Jolt 式虚拟指令展开，超出本里程碑）、可变轮数/动态 trace 长度。

## 2. 范围与边界（严格遵守）

- 只动 `crates/zkvm-slice/` 与 `zkvm-project/`；禁改上游；禁 git。
- 构建：`export RUSTFLAGS="-C target-cpu=native"` + `CARGO_BUILD_JOBS=4`。
- **不动** `word_vm.rs`/`word_vm_ram.rs`（M3/M4 交付物）。新机制写新切片。
- 诚实边界（报告必须声明）：表仍 native 构建双方共享（M3/M4 同一边界）；固定展开；
  版本链 O((32+K)·T) 已知边界；内存仍字寻址（无字节寻址/对齐检查）。
- **规模警戒线**：32 寄存器值链 + 32 寄存器版本链 + 64 RAM 版本链 ≈ 每周期
  ~128+ 条链约束。若 T=60 周期时 debug 构建 prove 超过 ~2s 或 OOM，缩小程序规模
  （减少周期数优先于砍指令覆盖），并在报告记录实测数据。

## 3. Leader 架构决策

### 3.1 指令子集（强制清单，30 条）
- U/J：`lui` `auipc` `jal` `jalr`
- B：`beq` `bne` `blt` `bge` `bltu` `bgeu`
- 访存：`lw` `sw`（字寻址，地址 = rs1+sext(imm)，沿用 K=64 + 掩码边界）
- I：`addi` `slti` `sltiu` `xori` `ori` `andi` `slli` `srli` `srai`
- R：`add` `sub` `sll` `slt` `sltu` `xor` `srl` `sra` `or` `and`
- 加分项（非强制）：`mul`（frontend `imul` 门，W2 路线下 3-4×AND；技术上属 RV32M，
  若做则在报告中单独标注）。
- 词级门参照 `zkvm-project/designs/binius64-frontend-api-map.md`（MSB-Boolean 约定，
  icmp_*/select/shl/shr/sar）。若某门不存在（如 sar 的 32 位变体），用门组合实现并在
  报告记录。

### 3.2 真 RV32I 编码与译码
- 使用**标准编码**（RISC-V  spec）：I-type `imm[11:0]`、S-type 分裂 `imm[11:5|4:0]`、
  B-type 分裂 `imm[12|10:5|4:1|11]`、U-type `imm[31:12]`、J-type 分裂。
  **废弃** M3/M4 的 imm7 布局。切片内的 `enc_*` 函数必须是标准编码——验收时会抽查
  若干指令字与手工计算的标准编码逐位一致。
- 译码用门从指令字提取全部字段；立即数拼接/符号扩展用 shl/bor/srl32/band/select 组合。
- **x0 硬零**：写回约束 `reg[0]' = 0`（或写回值经 `select(rd==0, 0, wb)` 丢弃），
  初值 0。必须有 soundness 或对拍覆盖"写 x0 被丢弃"。

### 3.3 状态机与论证架构（沿用 M3/M4，不另起）
- 32 寄存器值链 + 32 寄存器版本链 + 64 RAM 版本链（电路内）；RAM 值仍只由 logup\*
  写日志表钉住；寄存器仍双重钉扎（值链 + 写日志表）。
- 事件 inout 钉扎 + `claims_from_inout`（R2 纪律：禁止 native trace 取 claim）。
- 三件套（init/final/output）沿用 M4。
- 三表 logup\*（fetch + 寄存器写日志 + RAM 写日志）单 transcript。
- VER_MAX 按需参数化（程序中任何寄存器/地址的写次数上限 + 1），报告说明取值依据。

### 3.4 交付程序（两个，native 对拍）
1. **冒泡排序**：对 RAM 中 8 个 32-bit 字排序（初始值含乱序/重复/0x80000000 类边界值），
   输出单元 = 排序后数组 + 校验和。覆盖 lw/sw/blt(bge)/addi/beq(bne)/jal 或 jalr 等。
2. **torture 程序**：一条直线程序依次执行清单中每条指令至少一次，操作数选边界值
   （0、1、-1、0x7FFFFFFF、0x80000000、shamt=0/31、负数 slt/slti 对比、x0 写丢弃），
   最终寄存器状态与 native 参考解释器**逐寄存器对拍**。
- native 参考：扩展 slice 内的 `run_program` 为完整 RV32I 子集解释器（这就是对拍的
  ground truth；仓库无 isasim.rs，自包含即可）。

### 3.5 交付切片
`crates/zkvm-slice/src/slices/word_vm32.rs`（切片 26），`pub fn run_word_vm32()` +
`#[test]`，lib.rs 注册 + `pub use`。允许把解释器/编码拆为 `word_vm32` 的私有子模块
或共享辅助（如 `encode.rs` 的真编码版），但不动 M3/M4 文件。

## 4. soundness 用例（≥5，全部 verify 层拒绝）

1. **RAM 过期读（分层拒绝，招牌）**：load 读旧版本值、执行自洽 → 电路过 + logup\* 拒。
2. **版本篡改**：寄存器或 RAM 事件 ver inout 改错 → 拒。
3. **非法取指**：某周期 inst inout 换成表外合法编码字 → 电路过 + logup\* 拒。
4. **结果篡改**：final_regs / output 相关 inout 改值 → 拒。
5. **译码篡改**：把一个指令字换成**同表内但不同语义**的另一条指令（如 add→sub），
   其余 witness 相应重算 → final 对拍/verify 拒（证明译码语义被电路钉住）。
6. （加分）**x0 写丢弃**：声称 x0 被写入非零 → 拒。

## 5. 任务分解

- **T1 设计落实（先写后码）**：读 M4_REPORT.md、word_vm_ram.rs、api-map 文档；
  报告 §1 写清：真编码的字段提取门方案、x0 处理、32 寄存器链的成本估算、VER_MAX 取值。
- **T2 译码+执行层**：30 条指令的字段提取与执行（词级门）。
- **T3 状态机整合**：32 寄存器 + RAM 版本链 + 事件钉扎 + 三表 logup\* + 三件套。
- **T4 两个程序 + native 对拍**：冒泡排序 + torture；逐寄存器/逐内存字对拍输出。
- **T5 soundness**：§4 用例 1-5 必做，6 加分。
- **T6 成本与报告**：CircuitStat + 每指令成本（对比 M4 的 455 gate/周期，分析译码/寄存器
  扩展的代价）；`zkvm-project/M5_REPORT.md`；文档更新（crate README 切片 26、
  项目 README 切片表、PROGRESS M5 段、milestone-roadmap M5 状态）。

## 6. 验收标准（Leader 逐项核对）

1. `cargo test -p binius-zkvm-slice` 全过（应为 26 passed）。
2. **真编码**：抽查 ≥5 条指令字与手工计算的标准 RV32I 编码逐位一致；译码字段提取
   为门实现（指出代码行）。
3. **指令覆盖表**：报告列出 30 条指令 × 被执行的程序位置 × native 对拍结果。
4. x0 硬零有约束证据 + 对拍覆盖。
5. soundness 1-5 全部 verify 层拒绝；用例 1/3 为分层拒绝（电路过 + logup\* 拒）。
6. 冒泡排序端到端：排序结果与 native 一致，output 三件套检查通过。
7. 成本数据真实（CircuitStat），边界如实标注；四处文档更新，切片计数=26。

## 7. 送审要求

完成后：送审材料落成 `zkvm-project/M5_REPORT.md`，回复一条**简短送审消息**（≤15 行：
结论、文件清单、测试结果一行、需 Leader 复核重点 1-3 条）。细节一律在报告里。
