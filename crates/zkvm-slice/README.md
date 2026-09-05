# binius-zkvm-slice — Binius64 lookup 承载 zkVM 子问题的验证切片

**日期**: 2026-09-01 → 2026-09-05 | **状态**: 14 个切片均 prove→verify 端到端通过（含 soundness 拒假）

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
```

## 结论
- Binius64 `logup_star` 是**通用 indexed lookup**（任意表 + 任意 index 列），
  不是只用于 IMUL 的幂表。
- 已实证它能承载 zkVM 的三个 lookup 子问题: 指令查表(片1) + 内存一致性(片2)。
- 且 Binius64 内置 **Spartan prover** 能承载 R1CS glue:
  + 寄存器流转 + PC 进位 + 单条指令闭环 + 多指令序列 + 条件分支 + 含循环阶乘(片3-8)
  + **组合证明**(片9) + **多指令组合(查表取指+执行)**(片10) + **内存指令(store/load)**(片11)
  + **内存论证(mem_arg, 子多重集合读⊆写)**(片12) + **带时间戳内存论证(mem_arg_ts,
    同址多写·最近写判别)**(片13)。
- 十三个切片(均含 soundness 拒假) = Jolt 架构在二元域上的**完整 zkVM 雏形**：
  lookup 子系统(Lasso≡logup*) + 约束子系统(Spartan) 可在同一个 Fiat-Shamir
  transcript 组合，**已能端到端证明含循环/乘法/内存访问/读写一致性的真实程序**。
  其中 **mem_arg/mem_arg_ts 用 logup* 做多地址、多版本(带时间戳)的读-写一致性论证**
  (store+load 全锁定同表、T 存最近写值) = zkVM 最难一环(内存论证)的机制落点。

## 下一步（从切片到真实 zkVM）
1. 扩字宽到 RV32I(32-bit) 并覆盖更多指令(andi/slli), trace 用 isasim.rs。
2. 把 `mem_arg_ts` 升为**完整 SPICE 排序论证(Twist&Shout)**: version 由 trace 内打
   时间戳而非显式给出, 配排序器保证任意地址任意次"最近写"。
3. 把 13 切片固化为可复用库(crate)，接上 CLI/测试套件，做 native-vs-proof 交叉核对基准。
4. 决策：后端替换 Jolt vs 借鉴 Jolt 架构在 Binius64 内自建，**mem_arg_ts 已证明最近写判别**。

## 备注
- crate 挂在 binius64 workspace 下（`crates/zkvm-slice`），复用其依赖链。
- 用 3-bit 小表演示**机制**；全宽(32/64-bit)是真值表规模问题，非机制问题。