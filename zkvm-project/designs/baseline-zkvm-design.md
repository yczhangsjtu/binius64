# Baseline 设计：二进制域 RISC-V zkVM（RV32IM）

**状态**：研究纲领（优化起点，非最终架构）
**日期**：2026-08-31
**项目**：`~/workspace/binaryfield-zkvm`
**前置**：`research/binary-field-zkvm-survey.md`（领域现状与 gap）

---

## 0. 设计目标（为什么是"指令集"为焦点）

- **Focus 在真正的 RISC-V 指令集**，不做任何具体计算（哈希/大整数）的预编译或特化。
- Baseline 的意义 = **先有一台能跑通 RISC-V 程序、能显式量化「每条指令的证明成本」的机器**，作为后续优化的出发点。
- 一切优化（打包、lookup、字级约束）的宾语是**指令**，评判标准是**每类指令的（门数 / overhead / proof 比例）**。

### 非目标（baseline 阶段明确排除）
- 不做哈希/ECDSA 等领域预编译门（那是后续"指令子集之外的 extension"，不是 baseline）。
- 不做随机访问内存证明、不做递归/IVC、不做零知识性——均为后续里程碑。
- 不追求高速证明，只追求**正确、可解释、可量化**。

---

## 1. 技术栈选型（现有技术复用）

| 层 | 选择 | 理由 |
|---|---|---|
| 二进制域 | flock 的 GF(2^128)（Ligerito tower），底层 32-bit 塔域 | 已在手、M1 实证、后量子、位运算原生 |
| 证明后端 | flock：`GateType` + `CircuitBuilder` + `prove_fast_ligerito_union_circuit` + verify | 已在 M1（匿名凭据门）跑通任意自定义门 + wiring + 公开绑定 |
| 算术化模型 | **行式指令 trace → 逐指令「门」的展开** | focus 在指令，推理/量化按指令粒度 |
| 值表示 | **bit-vector**（每个 32-bit 值 = 32 条 0/1 线） | baseline 最直接：逻辑=逐位乘/加、移位=重排，全是最廉价域运算；打包/lookup 作为后续优化 |
| 对照参考 | 内部 native RISC-V 解释器（模拟器） | 交叉验证 trace 正确性（延续 M1 的 native-vs-rows 验证风格） |

**备选（文档记录，baseline 不采用）**：
- Binius64 的 64-bit 词级约束（SVI）：更强，但依赖社区工程、成熟度/运维风险高；作为**优化期**参考（见 §7）。
- 完整 Rust→ELF→RISC-V 工具链：baseline 用手写/生成的汇编即可，工具链接入放后续。

---

## 2. Baseline RISC-V 指令子集（RV32IM 精简）

选 `RV32IM`：纯整数 + 基础乘除，**不含 F/D 浮点、不含位操作扩展 B、不含 vector**（后续加）。

**RV32I（base，部分）：**
- 逻辑：`AND ANDI OR ORI XOR XORI`
- 移位：`SLL SRL SRA SLLI SRLI SRAI`
- 算术：`ADD SUB ADDI`（`SLTI SLTIU SLT SLTU` 比较）
- 立即数/寻址：`LUI AUIPC`
- 存取：`LB LBu LW SB SW`（baseline 先 `LW SW`，字节存取后续）
- 控制流：`BEQ BNE JAL JALR`
- 系统：`ECALL`（I/O 进出 witness，最小化）

**RV32M（可选入 baseline，因 M1 已验证乘法类）：**
- `MUL MULH MULHU MULHSU DIV DIVU REM REMU`

**裁剪依据**：这是"寄存器 + 少量内存 + 分支/跳转 + 乘除"的最小闭合集，能编译出循环/函数/条件，足以验证"通用指令执行"。

---

## 3. 核心：每类指令在二进制域的算术化（bit-vector 门方案）

**值表示约定**：32-bit 字 → 32 条线 `x[0..31]`，每条约束 `x[i]*(x[i]-1)=0`（布尔）。
逻辑运算在 F_2 逐位天然：
- `AND`：每 bit `y[i]=x[i]·z[i]`（域乘法，1 门/bit）
- `XOR`：每 bit `y[i]=x[i]+z[i]`（域加法，免费/线性）
- `NOT`：`y[i]=x[i]+1`
- **移位**：`y[i]=x[i-shift]`（重排，线性，零乘法门）

**进位算术（作业量主体）：**
- `ADD/SUB`：全加器逐 bit：`s=x⊕y⊕c`，`c'=(x&y)|(c&(x⊕y))`。约 **≤5 门/bit，32bit ≈ 130 门**。
- 比较 `SLT/SLTU`：差值符号提取，复用减法 + 少量门。

**乘法（baseline 最贵）：**
- `MUL(x,y)`：bit 级，`MULH` 同构。约 **O(32²) 门量级（数千门）**。这是 optimize 靶点（见 §7）。
- `DIV/REM`：逐位长除（最贵，可 baseline 初期禁用，标 future）。

**存储/加载：**
- baseline 用有界线性内存（`2^k` 字）。每条 `LW/SW` 一行：地址 → 解码选字（`k`-bit 译码门）+ 读写值。内存态串行传播到下一指令行。
- **不做随机访问证明**（置换论证留 §7）。

**控制流：**
- PC 是普通值（32-bit bit-vector）。`JAL/JALR/BEQ/BNE` 用加法算出下一 PC，`BEQ=0` 判定用 `XOR` 归约。分支一致性通过"两条后继 PC 的谓词选择"约束。

**指令解码**：32-bit 指令字取 opcode/funct 字段（常量位片，线性提取），分派到相应指令门。

### baseline 每类指令"行成本"表（一阶估计，实测定数）

| 指令类 | 门数（bit-vector 估计） | native 指令数（对照） | 备注 |
|---|---|---|---|
| AND/XOR/NOT | ~32 | 1 | 二进制域最优 |
| 移位 SLL/SRL | ~32（重排）| 1 | 线性 |
| ADD/SUB | ~130 | 1 | 进位传播 |
| SLT/SLTU | ~150 | 1 | 复用减法 |
| MUL/MULH | ~2000–4000 | 1 | **优化主靶点** |
| LW/SW | ~50 + 内存行 | 1 | 有界内存 |
| BEQ/BNE/JAL/JALR | ~50 | 1 | PC 更新 |

> 这张表就是 baseline 的**核心交付物**：把"focus 在指令集"落成可量化、可逐类优化的指标。

---

## 4. 执行 Trace 结构与证明

```
program (RISC-V machine code, 预先 witness)
  → 内部解释器执行，采集每行：
     [PC, 32×32bit 寄存器文件, 内存读写, 指令字段, 下一PC]
  → 组装成 flock 电路：
     CircuitBuilder
       .slot(指令门 for 第 行)
       ... 每行一个/一组指令门，行间用寄存器/内存/PC 共享 wire 串链
       .publish(公开输出 witness)
       .finish()
  → 所有行归为 union → prove_fast_ligerito_union_circuit → verify
```

- **寄存器文件**：32 个 32-bit 状态在行间以 wire 传递（读前一指令、写后一指令）。
- **正确性锚点**：native 解释器的输出与电路 `public_value` 交叉 assert（延续 M1 风格：rows 与 native walk 对拍）。
- **I/O**：`ECALL`/起点输入输出写入 witness public，保证"程序语义"被证明。

---

## 5. 端到端 pipeline（baseline 验收路径）

1. **RV32IM 汇编/机器码** → 内部汇编器/字节码（手写或单位指令拼接）。
2. **内部解释器**（native RISC-V 模拟器）执行 + 采集 trace（参考实现）。
3. **trace → 门电路**：逐行指令门 + 行间串链。
4. **prove + verify**（flock Ligerito）+ 对拍 assert。
5. 输出**每行/每类指令的 metrics**（prover_time / rows / prover_mem / proof_bytes / overhead=prover/native）。

**首个 gating 程序**：阶乘 + 冒泡排序（循环、条件分支、乘除、存取全覆盖）——证明它，证明"通用指令执行"成立。

---

## 6. 分阶段里程碑（baseline 本身）

- **M-A（baseline 0）**：`ADD SUB AND OR XOR SLL SRL SRA LUI AUIPC ADDI` 等基础指令门 + trace 骨架 + 单程序跑通（略 MUL/DIV）。→ 打平"指令门 pipeline"。
- **M-B（baseline 1）**：+ `LW SW BEQ BNE JAL JALR` + 有界内存 + 控制流一致 → 跑通阶乘/冒泡。
- **M-C（baseline 2）**：+ `RV32M MUL/MULH/...`，补每类指令指标表，产出 **baseline 成本量化文档**。
- **M-D（baseline 收敛）**：与 native 对拍全覆盖，benchmark 脚本化，沉淀为可复现基准。

每个里程碑交付：跑通 + 指标表 + 设计文档相应章节。

---

## 7. 优化路线（baseline 之后的靶点，源自调研）

- **指令成本拉平**：Diamond–Posen 二进制塔 Lasso-lookup → 把 MUL/进位/存取经 lookup 摊平（joins §2 备选 Binius64 SVI 词级约束）。
- **打包表示**：用 F_2^32 打包多个 bit（tower packing）降逻辑/移位门数，把 32-bit 算术丢给词级约束。
- **内存证明**：从有界线性 → 随机访问（置换论证/内存读取一致性）。
- **递归/IVC + 零知识性**：验证端简洁 + witness 隐私（后续研究里程碑，非 baseline 主题）。

---

## 8. 主要风险与缓解

- **指令门成本高（尤 MUL/DIV）** → 是优化靶点而非阻碍；baseline 先量测，后续 lookup/打包降。
- **flock 无 VM 语义** → 我们自己提供 trace→门 翻译层（自建核心，属于本项目贡献）。
- **行式 trace 电路规模随程序长度增长** → baseline 接受；递归/分段留后续。
- **外部依赖运维**（Binius64/PetraVM 社区维护）→ baseline 只依赖已有稳定的 flock，外部仅作优化期参考。

---

## 9. 结论

Baseline = 「**RV32IM 指令门 + bit-vector 表示 + flock Ligerito 证明层 + native 对拍**」。它刻意**不预编译任何具体计算**，把全部注意力放在 RISC-V 指令本身，以**每类指令的门数/overhead 表**为优化起点——完全落在"通用指令执行、逐指令降成本"的命题上。