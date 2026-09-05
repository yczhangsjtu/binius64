# zkVM（素域） vs 电路约束证明系统 —— 本质区别 + Binius64 重新定位

日期: 2026-09-01 | 调研来源: RISC Zero proof-system-in-detail.pdf / Jolt (Lasso, ACL2),
SP1 (Plonky3+LogUp), CertiK zkVM survey, Fenbushi zkVM benchmarking, zkm zkVM overview,
SoK Understanding zkVM, Oleg Fomenko memory checking guide

---

## Part 1: 素域 zkVM 架构的本质（它们共同的东西）

定义: zkVM 证明"一段程序按某 ISA 的执行轨迹是否正确"，而非"某个固定电路被满足"。

### 1.1 执行轨迹（trace）是灵魂，且是矩形阵
- 程序先被真实执行（executor/tracer），产生 trace。
- trace = 矩形数组：**宽 = 机器组件数（寄存器、PC、内存访问记录、flag 列……）**，
  **长 = 时钟周期数**。
- trace 的"长度"就是程序的执行步数 —— **运行时才知道，非编译期固定**。
- AIR 的一大优势就是天然表达这种"逐周期重复结构"：每条约束关联一行内或相邻行
  （taps：某字段在几个相邻周期 c, c+1, c+k 的值）。

### 1.2 zkVM 必须证明的三类东西（RISC Zero / SP1 / Jolt 共同）
1. **指令编码正确**：trace 的 opcode/rd/rs1/rs2/imm 列确实来自 PC 处取值的指令。
   → 需要"程序内存表"约束指令列与程序字节码一致。
2. **每条指令执行结果正确**：按 ISA 语义，输入寄存器 → 输出寄存器/PC 更新。
   → 用 selectors（opcode 解码出一热编码）+ 每类指令的算术关系。
3. **内存读写一致性（memory argument）**：任何 load 读到的是该地址最近一次 store 的值。

### 1.3 内存是最大难点 —— 随机访问 vs 顺序 trace
- CPU 内存访问是**随机的**（任意地址任意时刻），但 trace 是**顺序的**。
- 不能"逐行约束内存列"，因为 cycle 5 对 addr 1000 的 store 可能到 cycle 500000 才被读。
- **解法 = 内存总线（memory bus）+ 排列/多重集合论证**：把每次访问记到一个 log，
  证明 log 中读写集合是"排列关系"（读序列 ⊆ 写序列，且值一致）。
- 不用排列论证，就得把每个 load/store 用 N×M 个 selector 全连接 → **约束数 O(N·M) 平方爆炸**。
  - 具体方案: Spice/Lasso (Jolt), Twist & Shout (Jolt 新版), Permem (非排序), LogUp (SP1), PLONK permutation (RISC Zero), 排序论证 (generic)
- Jolt 用 lookup 做 3 处内存检查: 指令 lookup / 字节码归属 / 寄存器与 RAM 读写。
  时间戳范围检查也转成 lookup 到 [0,m-1] 表（实现 range check via Lasso）。

### 1.4 分支/选择用 lookup 或 selector
- opcode 越界值 → selectors 全非零 → 加权混合所有操作 → 必须约束 opcode 合法。
- 生产 zkVM 用 lookup（opcode→表）或 selector 一热编码约束。

### 1.5 证明来自三种子证明拼接
- Jolt: 程序执行后，调用三个 prover —— memory-checking + lookups + constraint
  satisfaction —— 三份子证明合并成总证明。
- RISC Zero: 三段式 —— 每个 segment 一个 STARK → segment 证明聚合(STARK) → Groth16 收口。
- SP1: 多 chips（CPU/ALU/memory）+ LogUp 描述 chip 互连 → Plonky3 STARK。
- trace 太大 → 切 chunks/segments + 递归折叠/聚合。

### 1.6 结构共性总结
| 关键要素 | 说明 |
|---|---|
| trace 行 = 一个 clock cycle | 长 = 执行步数（运行时可变） |
| trace 列 = 机器组件 | 寄存器/PC/flag/内存访问记录 |
| AIR/约束关联 taps | 相邻周期的字段值 |
| 程序内存表 | 指令列 ↔ 字节码一致性 |
| selectors / opcode lookup | 指令解码、分支 |
| memory argument | 随机访问内存一致性（排列/多重集合） |
| 切分+折叠 | 大 trace 分布式证明 |

---

## Part 2: 电路约束证明系统（R1CS/CCS/Binius64-style）的本质

定义: 证明一个**编译期固定的"计算图"（DAG of gates）**被某个赋值满足。

### 2.1 静态门图，无"指令流"
- 电路结构**编译期完全固定**，输入是 witness 赋值。
- 没有"逐周期状态"，没有 PC/时钟，没有"程序"概念。每个 witness 一次性满足整个图。

### 2.2 内存只能是"显式可寻址"或"数组元素作为 witness"
- 若需要内存，得显式把所有槽位列为 witness，用 select/多路选择器做随机访问 → **O(N·M) 膨胀**。
- 或退化成"数组元素逐个称 witness + 无随机访问"（只适合编译期已知访问模式）。

### 2.3 无"程序"，无"ISA 语义解耦"
- 每个新程序 = 重新画一张图。
- 结构变化（循环次数、分支路径、递归深度）→ 需要动态图或预分配上界。

### 2.4 优势：针对算法穷优化
- 常数 propag、CSE 等优化能显著减约束。
- 无需解释指令、无需内存总线排列论证、整体更小更快（custom-circuit 常比 VM 快一个数量级）。

---

## Part 3: 本质区别（一句话版）

> **zkVM 证明"一段程序按 ISA 执行的一个 trace（时序状态序列）正确"；
> 电路系统证明"一个编译期固定的门图被一个赋值满足"。**
>
> 差别的根源 = **是否有时序/指令流/随机访问内存**。
> zkVM 把这三点作为一等公民（AIR taps + 程序表 + 内存排列论证）；
> 电路系统没有时序，需显式编码状态流转与内存(常 O(N·M) 膨胀)。

| 维度 | zkVM (素域) | 电路约束 (R1CS/Binius64) |
|---|---|---|
| 证明对象 | 程序执行 trace | 静态门图 |
| 时序 | 有（trace 长=执行步数） | 无（编译期固定） |
| 程序指令 | 有（程序内存表+解码） | 无 |
| 内存 | memory argument（排列论证） | O(N·M) 多路选择器或预分配 |
| 每次实验可变 | trace 长可变 | 需预分配上界 |
| 成本 | 每周期固定宽度 × 周期数 | 每门固定，按算法优化 |
| 开发体验 | 写 Rust→编译器→ELF | 写电路/gadget |

---

## Part 4: Binius64 重新定位 —— 它是"电路后端"，不是 zkVM

### 4.1 Binius64 属于 Part 2（电路约束），尽管它很强
- 前面确认: 4 种词级约束(ZERO/AND/IMUL/BMUL) + shifted index + frontend API + M4 批量证明。
- **它没有**：时序/PC/指令流/程序内存表/内存排列 argument。
- 它的 AND/IMUL 归约是"per-约束-列"的 zerocheck/sumcheck，**不是 per-cycle tangent taps**。
- 它是一个**优秀的电路证明后端**（自定义专门电路能逼近哈希/ECDSA 最优），
  但**不是为通用指令集 VM 设计的** —— 缺时序与随机内存机制。

### 4.2 我想做二元域 zkVM → 真正要做的是"给 Binius64 补上 zkVM 那层"
如果坚持"用 Binius64 的约束后端 + 词级门 + M4 批量"来搭一个 RISC-V zkVM，我必须
**自建 zkVM 的全部"时序+内存"机制**：
1. **时序**：把 RISC-V trace 组织成"行=周期"的矩形阵，行间状态关联
   （Binius64 无 taps 概念 → 需自建"寄存器组每步快照 + 行间 wire 连接"）。
2. **程序内存表**：约束指令列与字节码一致（自建）。
3. **内存 argument**：随机访问读写一致性 —— 这是最大工作量。Binius64 没有现成的
   排列/multiset 论证（除非用它的 lookup(IMUL 里的 logup*) 或自己写）。
4. **trace 长可变**：Binius64 约束数是编译期定的，无法直接表达"运行 n 步"；
   → 需预分配最大步数上界 + 用零行 padding（类似 Jolt 的弹性 trace）。
5. **指令解码**：opcode→selectors/lookup（自建，利用 band/select/bxor_multi）。

换句话说: Binius64 提供"乘法/位运算/二元域的纸笔"，但"VM 的骨架（时序、PC、
内存一致性、指令解码）"**全都要我手动搭** —— 这正是 zkVM 里最难和工作量最大的 60%。

### 4.3 重新审视：要不要 / 值不值得把 Binius64 拉成 zkVM？

**收费点**：把 IVM 拉上去 = 自建 memory argument + 时序骨架，工作量和 SP1/Jolt
一样大（它们那套 AIR/chips/LogUp 是专门为这设计的）。Binius64 的电路后端优势
（按算法优化）在通用 VM 场景会被 VM 的固定 trace 宽度稀释。

**诚实结论（需与用户澄清）**：
- 若目标是"通用 RISC-V 程序都能证明" → 素域 zkVM（RISC Zero/SP1/Jolt) 是把这套
  时序/内存机制打磨好了的成熟系统，直接用它们的执行器+证明器，仅把后端换 Binius64
  意义不大（因为 Binius64 后端不天然吃 AIR trace）。
- 若目标仍是"研究二元域自带的每指令成本优势，证明'成本与指令数相关'" →
  更合理的切入点可能不是"造一个完整 VM"，而是:
  a. 用 Binius64 做一个**专门的、面向某类计算的电路**(如 ECDSA/bigint/哈希),
     直接展示二元域零位展开优势 —— 但这就不是 zkVM，是自定义电路。
  b. 若要真 zkVM，critical path 是**自己实现 memory argument + 时序**，
     需要评估是否借用 Binius64 的 logup* / M4 批量 or 引入独立排列论证。
  c. 回头审阅: BinarySpartan/flock 那条路正是 R1CS 证明，和 Binius64 是同一思想
     (电路约束) —— 之前 wiring bug 不是概念错,是工程 bug; 是否值得修 vs 换 Binius64。

---

## Part 5: 需与用户对齐的决策点（我应问的）
1. 目标是否为**通用 RISC-V 程序**（任意 Rust/C 编译）? 若是, memory argument 和
   时序骨架 = 90% 工作量，需要严肃评估是否值得自建于 Binius64 之上。
2. 是否接受"先做**自定义电路**演示二元域优势"(非 zkVM), 作为第一步快速出成果?
3. 若是真 zkVM, memory argument 方案选型: Binius64 的 logup* / 自写排列论证 / M4。
4. 是否考虑回到 flock(同为电路思想) —— 修那个 wiring 工程 bug 而非换核。