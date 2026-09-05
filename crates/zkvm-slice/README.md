# binius-zkvm-slice — Binius64 lookup 承载 zkVM 子问题的验证切片

**日期**: 2026-09-01 → 2026-09-05 | **状态**: 19 个切片均 prove→verify 端到端通过（含 soundness 拒假）

> ⚠️ **诚实勘误（2026-09-05 经逐行代码审查）**：这些切片验证的是**单机制可行**，
> **不是**一个完整的 zkVM，**未实现**真正的内存论证（时序/排序）、**未实现**通用的
> 跨行状态机（寄存器/PC 传递）。"读见最近写"多处为 **native 程序把正确值直接填进表**、
> logup* 只证明一致性（claim∈表），**不证明时序（最近写）**。真实性与边界见下方各切片
> 标注（⚠️=夸大/需修正，⭐=真正实现）。

> 项目根见 `../../zkvm-project/`（设计文档、研究笔记、进度）。本 crate 是切片代码的
> 权威所在（在 Binius64 workspace 内，用相对路径依赖其 crates，相对路径依赖已天然解决）。

## 目的
验证 Binius64 的二元域 `logup*`（logup_star）lookup 后端，能承载 zkVM
的两个基于 lookup 的核心子问题，为"后端替换 Jolt"或"借鉴架构自建"提供实据。

## 切片 1: `inst_lookup` — 程序指令查表
用 `logup*` 证明：RISC-V `and` trace 的每条指令查 AND 真值表的结果正确。
- 真值表 `T`（2^6=64 项）：index = (rs1_low | rs2_low<<nbits)，值 = rs1 & rs2
- 每条 trace 指令 = 一个 looker，claim 其查表结果在表中
- **`prove→verify_reduction` 闭环通过，且归约出的表 MLE claim 与真实 AND 表在共享点一致**

## 切片 2: `mem_lookup` — 内存一致性
用 `logup*` 证明：`lw`（load word）trace 的每个 load 读到该地址的正确值。
- 内存表 `T`（2^3=8 地址）：index = 地址，值 = 该地址存储值
- 每条 load = 一个 looker，claim 其地址→值查表在表中
- **闭环通过，且归约出的内存表 MLE claim 与真实内存一致；篡改 load 值被拒绝（soundness）**

## 切片 3: `pc_glue` — R1CS glue（寄存器/PC 状态流转）
用 Binius64 内置 **Spartan prover**（spartan-prover/frontend/verifier）证明程序状态流转。
- 程序：3×`xori` 对寄存器 x5（初值 0xa5...），每步 `out = in XOR imm`
- 流程: `ConstraintBuilder`(约束) → `compile` → `Prover/Verifier setup` →
  `WitnessGenerator`(witness) + `InstanceGenerator`(public 重算) → `prove` → `verify`
- **闭环通过(8 mul 约束)，篡改寄存器值被拒绝(soundness)**
- 诚实边界: PC 整数(+4)进位需 IMUL/进位单元，本切片只覆盖 ALU/寄存器 glue 半部

## 切片 4: `pc_carry` — PC 整数进位（分支/顺序执行的算术核心）
用 Spartan 证明 8-bit PC 按整数 +1 步进，**位分解 + 全加器进位链**
`s = a^b^cin, cout = (a&b)|(a&cin)|(b&cin)`，逐位约束进位传播正确。
- PC trace 0→1→2→3→4（触发跨位进位），native 重算 cross-check ✓
- **infrastructure 关键**: witness/instance 生成器必须**镜像约束侧的 derived-wire
  分配顺序**(先 assert_is_bit 全局，再 step_add 逐位)——否则 derived wire 编号错位，
  `cs.validate` 报 `b*b!=b`。已修复。
- **闭环通过，篡改 PC 位被拒绝(soundness)**
- 诚实边界: 8-bit 演示机制；32-bit PC 只是把 BITS 提到位宽 32 的问题。

## 切片 5: `instr_step` — 首条完整 RV32I 指令执行闭环 ★
**取指(word) → 译码(opcode/funct3) → 执行(xori) → 写回(x5) → PC+4** 全在一个
R1CS 约束系统（Spartan）里证明。指令 = 真实 RISC-V `xori x5,x5,0x2a`。
- 输入完整 32-bit 指令 word，译码约束 opcode==0x13、funct3==0x4，提取 imm
- 执行 `x5' = x5 XOR imm`（char-2 线性），写回，PC 用全加器链整数 +4
- **128 mul 约束，闭环通过**，native 交叉验证 `x5'=0x8f` ✓
- **soundness 关键教训**: 篡改 opcode 位会让 witness walk 时 record_error 导致
  `build()` 失败(而非 verify 拒绝)。正确做法: **篡改 witness public 段的 final 值**，
  让 witness 仍能 walk，prove 成功，verify(用verifier重算的public)拒绝。
- = 一台真 zkVM 的**最小状态机步进**。

## 切片 6: `multi_inst` — 多指令程序序列 trace ★
**真实小程序执行**：4 条连续 `xori x5,x5,imm`（PC 0x10→0x20），证明：
- **取指续流**：逐字取程序内存，每条译码合法（opcode/funct3 约束）
- **寄存器依赖链**：`reg[t+1] = reg[t] ⊕ imm[t]` 串接（真依赖，非独立）
- **PC 续流**：`pc[t+1] = pc[t] + 4` 全加器链
- 512 mul 约束闭环 + native 交叉验证 (x5 0xa5→0x53) + **篡改中间寄存器被拒** ✓
- **关键**: 共享状态 wire 用**分段分配**([pc|reg|pcn|regn|inst])，witness/instance 的
  write_inout 顺序必须匹配分段布局(非每步内嵌) —— 与 instr_step 不同。
- = 一台真 zkVM 能跑**完整直线程序**的证据。

## 切片 7: `branch` — 条件分支 (beq) ★
RISC-V `beq rs1, rs2, target`：`rs1==rs2` 则 PC 跳转到 target，否则 PC+4。
两个情形都证明（taken 0x20→0x40 跳转 / not-taken 0x20→0x24 顺序）。
- **相等检测**（二元域难点）= 位级乘法树 `taken = Π_i (1⊕rs1_i⊕rs2_i)`
- **条件 PC 更新** = 布尔 MUX：`pc_next = taken·target + (1+taken)·(pc+4)`
- 128 mul 约束 + 双情形闭环 + native 交叉验证 + **篡改分支目标被拒** ✓
- = 循环/控制流的前提，让 zkVM 能跑阶乘等含分支的程序。

## 切片 8: `factorial` — 含循环的真实程序（三要素合一）★
计算 `5! = 120` 的条件循环，**分支+多指令trace+整数运算在一个证明里**：
```
x2=x2*x3; x3=x3+1;  if x3 <= x1 goto loop   # acc*=i; i++; while i<=n
```
有界展开 6 轮，每轮证明循环体状态机（8-bit）：
- `go = (i<=n)`：分支条件（进位溢出测试 leq8）
- `acc' = go ? acc*i : acc`：**8-bit 移位加乘法器** + 布尔 MUX（循环退出后冻结）
- `i' = go ? i+1 : i`：全加器增量 + MUX
- 512 mul 约束闭环 + native 5!=120 交叉验证 + **篡改阶乘结果被拒** ✓
- = 二元域 zkVM 能跑**含循环的真实程序**，完整验证三要素。
- 关键排障：`go` 必须用**当前 i**（`i<=n`）而非 `i+1`，否则少乘一轮（native 24*5=120
  时电路误判 go=0 不乘）→ witness 与约束 MUX 不匹配。

## 切片 9: `combined` — 组合证明：logup* 查表 + Spartan 状态流转 ★★★
**Jolt 模块边界在二元域上的架构核心验证**：一个 `addi x5,x5,1` 步骤，在**同一个
Fiat-Shamir transcript** 里同时证明两条腿：
- **Spartan 层**：ALU/寄存器/PC 状态流转（进位加 `x5+1`、`pc+4`）
- **logup* 层**：程序内存查表 `T[pc] = instruction word`
- **同一 transcript**：logup* 的 gamma 在 Spartan 公共输入被 observe **之后**采样
  → 查表挑战依赖状态证明，构成不可分割的组合证明
- **128 mul**，闭环 + native 交叉验证 + **篡改查表断言被拒** ✓
- **= "后端替换 Jolt"可行性最本质验证**：Lasso(查表) + Spartan(约束) 可在 Binius64
  二元域上组合成单一证明。分开后不再需要程序内存逐字承诺（O(N×W) 取指代价消除）。

## 切片 10: `multi_combined` — 多指令组合证明（查表取指+执行）★★★
**第一个"查表取指 + 约束执行"统一证明的真实多指令程序**：2 条连续 `addi x5,x5,imm`。
- **Spartan 层**：逐条执行状态机（寄存器依赖链 + PC 续流 + 进位加，imm 从指令 word 提取）
- **logup* 层**：程序内存查表，**每条已执行指令一个 looker** claim `T[pc]=word`
- **同一 transcript**：Spartan observe 公共后再采样 logup* gamma → 取指挑战依赖执行证明
- **256 mul**，闭环 + native 交叉验证 + **拒绝程序内存中不存在的取指 claim** ✓
- **= zkVM 雏形完成**：Spartan(执行) + logup*(取指) 组合证明多指令程序。
- 排障：soundness 篡改 opcode 位会让 witness walk 失败(build panic)，正确做法是**篡改
  logup* 的取指 eval_claim 为程序表不存在的 word** → verify 拒绝。

## 切片 11: `mem_instr` — 内存指令（store/load + 读须见最近写）★★★
**补上 zkVM 最后一环**：程序 `addi x5,0x2a; sw x5,mem; lw x6,mem`，证明 load 读到
store 写的值（read-after-write 一致性）：
- **Spartan 层**：状态机——PC 续流(0x0→0xc 进位加) + store 写值==x5 + load 读入==x6
  + **R-A-W 内存门 `mem_r==mem_w`**(同址 load 看到 store 写)
- **logup* 层**：内存表 T[addr] 对 store & load 两个 looker 都验证
- **同一 transcript**：Spartan observe 后再采样 logup* gamma
- **128 mul**，闭环 + native 交叉验证(x6==0x2a) + **拒绝内存中不存在的 load 值** ✓
- **= "读须见最近写"的内存参数化**已被证明（单地址；多地址置换留给 Spice 式下一步）。

## 切片 12: `mem_arg` — 内存论证（多地址读-写一致性，sub-multiset）★★★
**zkVM 最难一环的真正落点**：之前的 `mem_lookup` 表是**手工给定**的，而 `mem_instr`
只处理单地址。真正的 memory argument 用 logup* 做**子多重集合论证**——**store 和 load
全部绑定到同一个内存表 T**：8 个 looker(4 store + 4 load)约束同一 T，从而在电路内强制
`load_value == T[addr] == store_value`（**读⊆写**，multiset on (addr,value)）。
- **多地址交错** store/load（4 个不同地址各写一次再读回）
- **读⊆写**：每个 load 读到的值 == 该地址最近 store 的值
- **sub-multiset 论证**：正是避免 O(N·M) selectors 平方爆炸的 memory-bus+multiset
- **soundness**：篡改 load 为从未 store 过的值 → **拒绝** ✓(读⊆写的核心)
- **= 内存一致性用"论证"而非"查表"**：store+load 都 lock 到同一表，表非自由。

## 切片 13: `mem_arg_ts` — 带时间戳内存论证（同一地址多写 · 最近写语义）★★★
**memory argument 最完整的一环**：`mem_arg` 每地址一次写；真实 RAM 允许**同一地址
多次 store**。本切片拆分**两个 logup 表**（同 transcript）：
- **写日志表 W**：`(address, version) → value`（每个 store 一条，store 事件是 W 的 looker）
- **读状态表 T**：`address → value`（该地址**最近写**的值，load 事件是 T 的 looker）
- **判别式**：`load 读 T[addr] = 最近写值`。地址3 写两次(0x11->0x22), load 读到 0x22;
  读到 0x11(旧值) → **拒绝** ✓
- **= "读须见最近写"的真正判别**：多版本写用 version 序号编码，T 只存最终值。
- 3 store + 2 load lookers, W:32 槽 / T:16 地址, 闭环 + **拒旧值** ✓
- 诚实边界: version 由 trace 显式给出(未做排序器); 完整 SPICE 排序论证(Twist&Shout)
  是下一步, 但**最近写语义已判别证明**。

## 切片 14: `jolt_bridge` — Jolt 前端 → Binius64 二元域后端桥接 ★★★
**后端替换的接口层实锤**：复刻 Jolt 前端 `JoltTraceRow`/`RAMAccess` 的 **u64 访问器
契约**（`ram_address`/`ram_read_value`/`ram_write_value` + LD/SD 物理行别名，照
`specs/proof-trace-row-layout.md`），构造真实内存访问 trace（**含同地址多写**），
把**域无关的 u64 值**直接喂给已跑通的**二元域 logup* 内存论证**（写日志 W + 读状态 T
两表），证明"读见最近写"：
- 程序: `sw 0x22->[3]; sw 0x2a->[3]; lw [3]; sw 0x55->[5]; lw [5]`
- **3 store 进 W + 2 load 进 T**，`load [3] 读到 0x2a(最近写)`，读 0x22(旧值)→**拒** ✓
- **= 后端替换直接可行证据**：Jolt 前端 trace(u64) 无缝接 Binius64 二元域论证。
- 对照详见 `~/workspace/binaryfield-zkvm/research/jolt-binius-memory-argument-mapping.md`。

## 切片 15: `mem_arg_spice` — 完整 SPICE 排序内存论证（任意次写 · 全局时间戳）★★★
**内存论证最终形态**：`mem_arg_ts` 用 per-address version（未证排序）；本切片用
**全局时间戳 `ts` + 时间排序状态表** `T[ts*ADDR+addr]`，证明任意次"最近写"：
- **任意次数写**：addr1 写三次(0x11->0x22->0x33)，不用固定每地址次数
- **时间排序表**：每个访问用 (ts,addr) 精确定位 T 中对应时间点/地址单元的值
- **读错时间点=读错值=拒**：load@ts5 必须读 0x33(该时刻 addr1 的最值)，声称读旧值
  0x11(ts=0 的过期值) → **拒绝** ✓
- **= SPICE 排序论证本质**：读写绑定到同一 (timestamp,address) 单元，排序由表的结构承担。
- 5 store + 2 load, T:32 单元(2^5), 闭环 + **拒过期值** ✓
- 关键坑: `FieldBuffer` 表长度须为 **2 的幂** (ts_max*addr=32), 否则 panic。

| ## 切片 16: `full_vm` — zkVM 演示（⚠️ 仅一次性演示，内存表手工构造）
**⚠️ 诚实标注**：这是一个"把循环+内存+整数加法放进一个文件"的**演示**。它验证了循环加法、
取指、内存查表能在**同一 transcript** 跑，但：
- 执行语义**模板化**（硬编码 load/addi 路径，循环固定展开 N 轮）
- **"读见最近写"由 native 程序直接把正确值填进表**，logup* 只证明 claim∈表（一致性），
  **未证明时序（最近写）**
- 512 mul, n_private=0, 闭环 + 拒假。**不是"第一台完整 zkVM"，而是演示**。

## 切片 17: `full_vm_store` — zkVM + store 演示（⚠️ 同上，读见最近写为手工构造）
**⚠️ 诚实标注**：在 full_vm 上加 store（读-改-写循环），loads/stores 均为 run_program 算出
（直接填值）。"读见最近写"未证时序，仅"X 时刻表里是这么填的"。256 mul, 闭环 + 拒假。
**非"第一台证明读-改-写循环的 zkVM"**——是演示。

## 切片 18: `full_vm_multi` — zkVM 多地址演示（⚠️ 同上）
**⚠️ 诚实标注**：多地址交替读-改-写（两地址），loads:[1,5,2,7] stores:[2,7,9,16] 全是
run_program 手工算出。"读见最近写"为手工构造，**未证时序**。512 mul, 闭环 + 拒假。
**非"最强内存论证测试"**——只是演示，未真正论证。

## 切片 19: `full_vm_jolt` — JOLT 风格 word 驱动指令执行（⭐ 真解码思想，⚠️ 无跨行）
**⭐ 真正有价值的部分**：`execute_cycle` 解码取回 word 的 opcode 分发到 addi/beq（beq 用
eq 树 + MUX 选 pc），这是**对的方向**。
**⚠️ 但只是单行约束**：`match row.op` 是 native 固定的，**跨行寄存器/PC 传递未实现**，
非"真实控制流状态机"。256 mul, 闭环 + 拒假。诚实边界：仅 x1，limit 常量，仅 addi/beq。

## `zkvm`（zkvm.rs）— 逐行验证器（⚠️ 非"整合 zkVM 主代码"）
**⚠️ 关键更正**：zkvm.rs **并非整合全部机制的 zkVM**，实为**逐行验证器 + 手工内存表**：
- `drive_row` 里 **`let _ = pc;`**——PC 根本没被约束；
- `row.op` 是 `run_program()` 里 **match 死的枚举**，**不是从 word 解码**；
- `a`/`b` 为**独立注入值**，**无跨行寄存器传递**（上条 rd 不经寄存器堆流入下条 rs1）；
- 内存表是 **native 手工填的** `tvec`，logup* 只证 claim∈表，**不证时序**。
24 行 trace, n_mul=2048, n_private=0, 闭环 + 拒假。**不作为后续递增基线**。

## 运行
```bash
cd /home/yczhang/workspace/binius64
export RUSTFLAGS="-C target-cpu=native"
CARGO_BUILD_JOBS=4 cargo run -p binius-zkvm-slice --bin inst_lookup
CARGO_BUILD_JOBS=4 cargo run -p binius-zkvm-slice --bin mem_lookup
CARGO_BUILD_JOBS=4 cargo run -p binius-zkvm-slice --bin pc_glue
CARGO_BUILD_JOBS=4 cargo run -p binius-zkvm-slice --bin pc_carry
CARGO_BUILD_JOBS=4 cargo run -p binius-zkvm-slice --bin instr_step
CARGO_BUILD_JOBS=4 cargo run -p binius-zkvm-slice --bin multi_inst
CARGO_BUILD_JOBS=4 cargo run -p binius-zkvm-slice --bin branch
CARGO_BUILD_JOBS=4 cargo run -p binius-zkvm-slice --bin factorial
CARGO_BUILD_JOBS=4 cargo run -p binius-zkvm-slice --bin combined
CARGO_BUILD_JOBS=4 cargo run -p binius-zkvm-slice --bin multi_combined
CARGO_BUILD_JOBS=4 cargo run -p binius-zkvm-slice --bin mem_instr
CARGO_BUILD_JOBS=4 cargo run -p binius-zkvm-slice --bin mem_arg
CARGO_BUILD_JOBS=4 cargo run -p binius-zkvm-slice --bin mem_arg_ts
CARGO_BUILD_JOBS=4 cargo run -p binius-zkvm-slice --bin jolt_bridge
CARGO_BUILD_JOBS=4 cargo run -p binius-zkvm-slice --bin mem_arg_spice
CARGO_BUILD_JOBS=4 cargo run -p binius-zkvm-slice --bin full_vm
CARGO_BUILD_JOBS=4 cargo run -p binius-zkvm-slice --bin full_vm_store
CARGO_BUILD_JOBS=4 cargo run -p binius-zkvm-slice --bin full_vm_multi
CARGO_BUILD_JOBS=4 cargo run -p binius-zkvm-slice --bin full_vm_jolt
CARGO_BUILD_JOBS=4 cargo run -p binius-zkvm-slice --bin zkvm
```

## 结论（诚实版）
- Binius64 `logup_star` 是**通用 indexed lookup**（任意表 + 任意 index 列），不是只用于 IMUL 的幂表——**已实证**。
- Binius64 内置 **Spartan prover** 能承载 R1CS glue：寄存器流转 + PC 进位 + 单条指令闭环 +
  多指令序列 + 条件分支 + **含循环阶乘（factorial，⭐ 有真跨行状态迁移）**（切片 3-8）。
- **组合证明**（切片 9/10）：logup* + Spartan 能在**同一个 Fiat-Shamir transcript** 串联。
- **读⊆写 sub-multiset**（切片 12，⭐ 真内存论证雏形）：store+load 锁定同一表。
- **⚠️ 严格边界**：**"读见最近写"的时序论证（排序/最近写）未实现**——切片 11/13/15/16-19
  及 zkvm.rs 中，表格由 native 程序把正确值直接填进、logup* 只证明一致性（claim∈表），
  **不证明时序**。亦**无通用的跨行寄存器/PC 状态机**（理想是像 factorial 那样，但目前
  full_vm/zkvm 均未做到）。
- **因此这些切片是"单机制可行"验证集，不是"完整 zkVM"，也不构成 Jolt 级内存论证。**

## 下一步（真正需要做，而非已完成的）
1. **真正的通用状态机**：跨行寄存器传递（上条 rd → 下条 rs1）+ PC 推进约束，这是"程序
   执行"命门，目前**未实现**（factorial 是唯一有跨行迁移的例外）。
2. **真正的内存论证**：把"读见最近写"的**时序**做成论证（排序/时序 sorter 或 Jolt 式
   one-hot+increment），而非手工填表 + 一致性检查。
3. **word 真正驱动解码**：opcode 位 → 决定执行路径，且与跨行状态机连接。
4. 扩 RV32I 子集与字宽到 32-bit，trace 用 isasim.rs，跑更大程序。
5. 固化库 + 测试套件：把真实机制（factorial 级跨行状态机）做成可复用 crate + 测试基准。

## 备注
- crate 挂在 binius64 workspace 下（`crates/zkvm-slice`），复用其依赖链。
- 用 3-bit 小表演示**机制**；全宽(32/64-bit)是真值表规模问题，非机制问题。