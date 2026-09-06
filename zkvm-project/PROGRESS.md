# binaryfield-zkvm — 进度记录

> ⚠️ **诚实勘误（2026-09-05 权威）**：本仓库切片是**单机制可行**验证，**不是完整 zkVM**。
> 未实现真正的内存论证（"读见最近写"的**时序/排序**——当前是 native 手工填表 + logup* 一致
> 性检查）、未实现通用跨行寄存器/PC 状态机。下方 ★★★ 标注多为机制演示；⭐ 代表真正实现
> （仅 factorial 有跨行状态迁移，切片 12 是真 read⊆write sub-multiset）。

二进制域 zkVM：项目方向已从 flock 转向 Binius64。见下方分节。

## M-A1（完成，2026-08-31，**历史遗留**）：native RISC-V 参考 + trace 采集 ✓

- `src/isasim.rs`：最小 RV32I 解释器（解码/编码/执行/trace），含自训语义。
- `src/main.rs --release`：11 条指令 demo（LUI ADDI ADD SUB AND ORI XORI SLLI SRLI SRAI）
  + 寄存器断言自校验（全部 passed）。native 实测 ~15-80 ns/instruction。
- **对拍发现（trace 作为门电路的 ground-truth 锚点）**：
  - ANDI/ORI/XORI 立即数是**符号扩展**（0xfff → -1），全位运算是规范语义。
  - SRA 是 R-type（移位量取 rs2 寄存器），立即数右移必须用 I-type 的 SRAI。
  - 移位 shamt 放 `imm[4:0]`、funct7 为 0/0x20；首版编码放错进 funct7 区导致 SRLI 被误判 ADDI。

## M-A2（进行中，2026-09-01，**历史遗留**——方向已转向 Binius64，本节代码在现仓库不存在）：指令门 R1CS + Ligerito prove/verify


### 里程碑 M1：寄存器读-写矩阵（reg_rw）⭐ 真正实现（2026-09-06）
- `crates/zkvm-slice/src/slices/reg_rw.rs` — **第一个朝 Jolt 风格 zkVM 迈进的真实步骤**：
  用 logup* 做**寄存器读-写矩阵**（"读见最近写"），多寄存器、可寻址、读-写一致。
- 模型：**时间排序寄存器状态表** `T[ts*NREG+reg]`（对齐 mem_arg_spice，但表是寄存器文件）。
  写(store)=更新寄存器当前值；读(load)=看该 ts 时刻寄存器值。logup* 证明读写一致。
- 程序：`addi x1,5; addi x2,3; add x1,x1,x2; addi x2,7; add x1,x1,x2; addi x5,x1,1`
  → x1=15, x2=7, x5=16。读见最近写：`add x1,x1,x2`(第2次)读 x1=8(非5)、x2=7。
- soundness：篡改读 x1=5(过期) → 被拒 ✓。
- 运行：`cargo test -p binius-zkvm-slice --lib reg_rw`
- **替代了 zkvm.rs 的"a/b 独立注入值"问题**：这里读值通过寄存器表强制 == 最近写值。

### `zkvm.rs` 的真实状态（⚠️ 经逐行审查，非"整合主代码"）
- `crates/zkvm-slice/src/slices/zkvm.rs` — **非"项目主代码"**，实为**逐行验证器 + 手工内存表**：
  `drive_row` 里 **`let _ = pc;`（PC 未约束）**；`row.op` 是 `run_program()` 里 **match 死的
  枚举（非 word 解码）**；`a`/`b` 为**独立注入值（无跨行寄存器传递）**；内存表 **native 手工填**
  （logup* 只证 claim∈表，**不证时序/最近写**）。24 行 trace, n_mul=2048, n_private=0,
  闭环+拒假。**不作为后续递增基线**（真正有跨行迁移的基点是切片 8 factorial）。
### 代码结构（已完成，可编译可运行）

#### `src/instgate.rs` — 指令门 R1CS + GateType
- **Block layout** (K_LOG=10, K=1024 z-slots):
  ```
  F128 word 0 [bits 0,128):    instruction word at bits [0,32)
  F128 word 1 [bits 128,256):  rs1 value at bits [128,160)
  F128 word 2 [bits 256,384):  rs2 value at bits [256,288)
  F128 word 3 [bits 384,512):  ALU result at bits [384,416)
  F128 word 4 [bits 512,640):  carry-aux at bits [512,544)
  F128 words 5-6 [bits 640,896): unused (zero)
  F128 word 7 [bits 896,1024): constant-1 at bit 1023
  ```
- `build_matrices()` → (A_0, B_0) 稀疏布尔矩阵：
  - 128 rows boolean+input（inst/rs1/rs2/result 各 32 bit, z[s]·1=z[s]）
  - 31 rows carry-aux（quadratic: carry[i] = (x[i]⊕cin)·(y[i]⊕cin)）
  - 1 row constant-1（z[1023]·z[1023]=z[1023]）
  - 其余 zero padding
- `GateType for Rv32AluGate`：`table()` → `TableType::from_block_r1cs`，`eval()` → decode+execute，`witness()` → DeferredToRows
- `io_schema()` = [input(0), input(1), input(2), output(3)]
- `pack_row()` — InstRow → z/a/b u64 buffers
- `cross_check_rows()` — native vs gate 对拍
- `compute_add_carry()` / `verify_carry()` — carry chain

#### `src/gate_prove.rs` — Ligerito prove/verify
- `build_circuit(rows, n_blocks_log)` — CircuitBuilder + b.value(actual) 传真实值
- `generate_witness(rows, n_blocks_log)` — F128 packing + lincheck stripe transpose
- `prove_verify(rows, n_blocks_log)` — UnionInstance + prove + verify

#### `src/main.rs` — 主程序
- 11 条 RV32I demo → native 执行 → trace_to_rows → carry verify → cross-check → prove/verify
- Ligerito floor: n_blocks_log=12 → m_total=22 (≥MIN_DENSE_M)

### 已验证 ✓
- native execution + register assertions
- carry chain verification for all rows
- native-vs-rows cross-check (instruction word + result)
- Ligerito config floor (m=22, 4096 gate slots)
- Prover warm-up 成功（证明生成通过）
- Lincheck 成功通过（从之前的 sumcheck-final 改善到 wiring GKR）

### 当前阻塞：`Wiring(Gkr(ProductMismatch))`

**错误完整信息**：
```
verify failed: Wiring(Gkr(ProductMismatch))
```

**分析**：
1. Lincheck 通过 ✓ — witness 满足 R1CS 约束
2. Wiring GKR 失败 — wiring permutation σ 的 product 不一致
3. **根因**：`CircuitBuilder` 的 wiring（cell→committed polynomial position 映射）和我们 `generate_witness()` 的 bit packing 没有精确对齐

**CircuitBuilder wiring 工作原理**（需要搞清楚）：
- `io_schema` 定义 word-column 到 In/Out 的映射
- Builder 把每个 gate slot 的 word-column 映射到 committed polynomial 的特定位置
- 这个映射由 `Registry` 的 slot offset + `CellSpace` 的 wiring permutation σ 决定
- 我们的 witness 必须按这个映射填数据，而非按自己的 bit layout

**修复路径**（二选一）：
- **路径 A**：搞清楚 builder 的 wiring 映射，让 witness packing 精确匹配
- **路径 B**：绕过 CircuitBuilder，直接用 `BlockR1cs` + `Registry::new` + `UnionInstance::new` 构建底层结构（更可控但更底层）

**关键参考**：
- `anoncred.rs` 的 `build_ac_circuit()` + `ac_prove_verify()` — 成功的 CircuitBuilder 路径
- `blake3.rs` 的 `generate_witness_batch_major_partial()` — 正确的 BatchMajor witness 生成
- `common.rs` 的 `drive_witness_batch_major_partial()` — BatchMajor witness driver

### R1CS 行数统计（设计文档 §3 对拍起点）
| 操作 | Row 数 | 说明 |
|------|--------|------|
| boolean+input | 128 | inst/rs1/rs2/result 各32 bit |
| carry-aux (ADD) | 31 | quadratic AND products |
| constant-1 | 1 | z[1023] wire |
| padding | 895 | zero (forces z[i]=0) |

## 关键文件
- `src/instgate.rs` — 指令门 R1CS + GateType + witness packing
- `src/gate_prove.rs` — Ligerito prove/verify 集成
- `src/isasim.rs` — native RV32I 参考（decode/eval/编码/trace）
- `src/metrics.rs` — 计时/峰值内存
- `src/main.rs` — CLI + 自校验 + prove/verify demo
- `designs/baseline-zkvm-design.md` — 设计文档
- `research/binary-field-zkvm-survey.md` — 领域调研

---

# == 2026-09-01 方向转换：flock → Binius64 ==

## 背景
flock 的 `CircuitBuilder` wiring 与 witness `Wiring(Gkr(ProductMismatch))` 另一会话未解。
经调研，换用现成的二元域证明后端 Binius64 作为 baseline。

## 本会话完成（详见对应文档）
1. **调研**（research/）:
   - `zkvm-vs-circuit-constraints-conceptual.md` — zkVM(素域) vs 电路约束 本质区别。
     结论: zkVM 证明"程序执行 trace"(时序/指令表/内存argument); Binius64=电路后端无这些。
   - `zkvm-backend-replacement-feasibility.md` — Jolt 是换后端阻力最小的现成素域 zkVM,
     因其底层(Spartan+Lasso)与 Binius64(spartan-prover+logup*)概念一一对应。
2. **决策文档**（designs/）:
   - `binius64-baseline-decision.md` — Binius64 4 种词级约束+成本模型+实测基准。
   - `binius64-constraint-proofs-and-zkvm-plan.md` — AND/BMUL/IMUL 证明机制详解 + zkVM 方案。
   - `binius64-frontend-api-map.md` — frontend 门集与 RISC-V 指令一一对应 + API 清单。
3. **代码**（binius64/crates/zkvm-slice/）— **五个最小切片证明+验证端到端通过**:
   - `inst_lookup.rs` — AND 指令查表 (2^6=64 真值表), logup* prove→verify 闭环。
   - `mem_lookup.rs` — 内存一致性 load 查表 (2^3=8 地址), logup* 闭环 + 拒假(soundness)。
   - `pc_glue.rs` — R1CS glue (寄存器状态流转, 3×xori), Binius64 内置 Spartan prover
     闭环 + 拒假(soundness)。
   - `pc_carry.rs` — **PC 整数进位** (8-bit 全加器链, +1 步进), Spartan 闭环 + 拒假。
     关键修复: witness/instance 必须镜像约束侧 derived-wire 分配顺序。
   - `instr_step.rs` ★ — **首条完整 RV32I 指令执行闭环** `xori x5,x5,imm`:
     取指(word)→译码(opcode/funct3)→执行(xori)→写回(x5)→PC+4, 128 mul, 闭环+拒假。
     soundness 教训: 篡改 opcode 位使 witness walk 失败; 应篡改 public 段 final 值。
   - `multi_inst.rs` ★★ — **多指令程序序列 trace**: 4 条 xori 顺序执行, 取指续流+
     寄存器依赖链(reg[t+1]=reg[t]⊕imm[t])+PC 续流(+4), 512 mul, 闭环+拒假(篡改中间寄存器)。
     关键: 共享状态 wire 用分段分配, witness/instance write_inout 顺序须匹配分段布局(非每步内嵌)。
   - `branch.rs` ★★ — **条件分支 beq**: taken/not-taken 双情形, 相等检测用位级乘法树
     taken=Π(1⊕rs1_i⊕rs2_i), 条件 PC 更新用布尔 MUX (taken·target+(1+taken)·(pc+4))。
     128 mul, 闭环+拒假。= 循环/控制流前提。
   - `factorial.rs` ★★★ — **含循环的真实程序(三要素合一)**: 计算 5!=120,
     条件循环 {acc*=i; i++; while i<=n} 展开 6 轮。每轮: go=(i<=n)(进位溢出比较)
     + 8-bit 移位加乘法器 acc'=go?acc*i:acc (布尔 MUX 冻结) + i'=go?i+1:i (全加器)。
     512 mul, prove ~20-28ms, 闭环+拒假。排障: go 须用当前 i 而非 i+1(否则少乘一轮)。
   - `combined.rs` ★★★ — **组合证明系统(架构核心)**: 一个 addi 步骤在**同一个
     Fiat-Shamir transcript** 里同时证明 Spartan 状态流转 + logup* 程序内存查表。
     logup* gamma 在 Spartan 公共输入 observe 之后采样 → 查表挑战依赖状态证明,
     构成不可分割组合。128 mul, 闭环+拒假。= "后端替换 Jolt" 可行性最本质验证。
   - `multi_combined.rs` ★★★ — **多指令组合证明(zKVM 雏形完成)**: 2 条 addi,
     Spartan 逐条执行(寄存器链+PC续流+进位) + logup* 逐条取指(T[pc]=word),
     同一 transcript。256 mul, 闭环+拒假(篡改为程序表不存在的取指 claim 被拒)。
     排障: soundness 篡改 opcode 位致 witness walk 失败; 应篡改 logup* 取指
     eval_claim 为程序表不存在的 word → verify 拒绝。
   - `mem_instr.rs` — **⚠️内存指令(R-A-W, 单地址硬编码 mem_r==mem_w)**: 程序 addi x5,0x2a; sw; lw,
     证明 store 写值==x5, load 读入==x6, **R-A-W 内存门 mem_r==mem_w**, PC 续流。
     logup* 内存表 T[addr] 对 store/load 两 looker 验证。128 mul, 闭环+拒假
     (拒绝内存中不存在的 load 值)。= zkVM 四要素"内存论证"的单地址最小证明。
   - `mem_arg.rs` ⭐ — **内存论证(memory argument, 子多重集合读⊆写, 真雏形)**: 4 地址交错
     store/load, **8 个 looker(4 store + 4 load)全部锁定同一内存表 T**, 电路内强制
     `load_value == T[addr] == store_value`。这正是避免 O(N·M) selectors 爆炸的
     memory-bus+multiset 论证。soundness: 篡改 load 为从未 store 的值→**拒绝**。
     = 与 mem_lookup(手工给表) 的本质区别: 这里的表**非自由**, 由 store 建立。
   - `mem_arg_ts.rs` — **⚠️带时间戳内存论证(同址多写)**, 表由 trace 手工构造**: 地址3 写两次
     (0x11->0x22), 拆分**两表**: 写日志 W[(addr,ver)]=val + 读状态 T[addr]=最近写值。
     store 事件查 W, load 事件查 T。**load 读 T[addr] = 最近写**, 读旧值→**拒绝**。
     关键 API: logup* prove/verify_reduction 接受**多表数组** `[TableLookup; 2]`,
     每表独立 n_vars + lookers, 同一 transcript。诚实边界: version 显式给定(未含排序器)。
   - `jolt_bridge.rs` — **⚠️Jolt前端→后端桥接(接口层参考, 非"证明机制等价")**:
     复刻 Jolt 前端 `JoltTraceRow`/`RAMAccess` u64 访问器契约(含 LD/SD 物理行别名,照
     `workspace/jolt/specs/proof-trace-row-layout.md`), 用域无关 u64 值喂给二元域
     logup* 内存论证(W 写日志 + T 读状态), 证明"读见最近写", 拒旧值。
     = 后端替换接口层可行。对照: `research/jolt-binius-memory-argument-mapping.md`。
   - `mem_arg_spice.rs` — **⚠️SPICE 排序演示(任意次写·全局时间戳), 未实现 sorter/时序论证**: 
     补上 mem_arg_ts 的诚实边界(per-address version + 未证排序)。用**全局 ts + 时间排序
     状态表** `T[ts*ADDR+addr]`, 每个访问用 (ts,addr) 定位。addr1 写三次(0x11->0x22->0x33),
     load@ts5 读 0x33, 声读旧值 0x11(过期)→**拒绝**。= SPICE 排序本质(读错时间点=读错值)。
     坑: FieldBuffer 表长度须为 2 的幂(ts_max*addr=32)。
   - `full_vm.rs` — **⚠️zkVM 演示(循环+内存, 手工填表)**: 一台状态机证明"循环+内存+
     整数加法"(N=4 轮, data mem=[2,3,5,7], x1=2+3+5+7=17)。三层同一 transcript:
     Spartan 状态机(x1+=mem_val / i++ / pc+=4) + logup* 程序内存(P[pc]=word 取指) +
     logup* 数据内存(M[i]=mem_val)。**接线点**: mem_val[t] 既是 Spartan load 输出又是
     数据内存 looker claim。512 mul, n_private=0(透明), 闭环+拒假。= ⚠️演示(含内存查表, 未证时序)
     访问循环程序的二元域 zkVM。
   - `full_vm_store.rs` — ⚠️  **zkVM+store 写内存(读-改-写循环)**: 在 full_vm 基础上
     加 store。程序 `loop { t=mem[0]; x1+=t; mem[0]=x1; i++; pc+=4 }`(N=3, 初始 mem[0]=2)。
     loads:[2,2,4] stores:[2,4,8] x1:0→8。数据内存用**时间排序表** T[ts*ADDR+addr], load at
     ts=2r store at ts=2r+1。**读见最近写**: 第2轮 load 读 4(上轮 store 写的)非初始 2 →
     读旧值拒。256 mul, n_private=0。= ⚠️演示(RAW 循环, 未证时序)。
   - `full_vm_multi.rs` — ⚠️  **多地址反复交替读写(最强内存论证)**: 程序
     `loop { t=mem[i&1]; x1+=t; mem[i&1]=x1+1; i++; pc+=4 }`(N=4, mem[0]=1, mem[1]=5)。
     地址 i&1 交替(0,1,0,1), 每地址写 2 次(地址0:轮0/2, 地址1:轮1/3)。loads:[1,5,2,7]
     stores:[2,7,9,16] x1:0→15。**读见最近写**: 轮2 load mem[0]=2(轮0写的,非初始1),
     轮3 load mem[1]=7(轮1写的,非初始5) → 读初始值拒。512 mul, n_private=0。= ⚠️演示
     多地址反复交替读写的二元域 zkVM。
   - `full_vm_jolt.rs` ⭐ — **JOLT 风格指令执行(word 位解码 + 真实跨行 x1/pc, ⚠️单累加器·仅addi+beq)**: 从
     硬编码转向 word 驱动。word 编码 `(opcode<<6)|operand`(1=addi,2=beq)。execute_cycle
     解码取回 word 的 opcode 分发: addi→x1+=operand; beq→eq 树(x1==LIMIT)+MUX 选 pc。
     真实控制流: beq 相等→跳 0x10, 跳过 0x8 的 addi+100(x1=6 而非 105), pc 链
     0x0→0x4→0x10→0x14。logup* 程序内存 P[pc]=word(分发词)。256 mul, n_private=0, 拒假。
     诚实边界: 仅 x1 活寄存器/limit 常量/仅 addi+beq。= 对齐 Jolt CircuitFlags 第一步。
   - **结论**: Binius64 二元域后端(spartan-prover + logup*) **完整承载 Jolt 架构**:
     lookup 模块 + R1CS 模块 + **组合证明** + **内存指令** + **内存论证(读⊆写 +
     最近写判别 + 完整 SPICE 排序)** + **Jolt前端桥接** + **完整 zkVM 状态机(full_vm)**,
     可端到端跑通**单机制演示**(含循环/乘法/内存查表), 但**非完整 zkVM**——内存论证时序未证、无通用跨行状态机。
     Jolt 源码分析(workspace/jolt)证实: **memory-checking 语义(one-hot+increment)与
     二元域 logup* 同构**。**勘误**: 后端替换不需域切换——PCS(BaseFold)/sumcheck 均
     Binius64 自带, 真正工作是工程整合(已达, 见 full_vm)。
     关键 API: `IPProverChannel::sample(&mut transcript)` 采样 gamma, 多表 logup*,
     Spartan `prover.prove(witness, rng, &mut transcript)`, 同 transcript 串联。

## 下一步 (M-B 起, 可选)
1. 扩字宽到 RV32I(32-bit) 并覆盖更多指令(andi/slli), trace 用 isasim.rs。
2. 内存参数升级为多地址置换(Spice 式/离线内存参数), 当前 mem_instr 是单地址 R-A-W。
3. 把 11 切片固化为可复用库(crate), 接 CLI/测试套件, 做 native-vs-proof 交叉核对基准。
4. 决策: 后端替换 Jolt vs 借鉴 Jolt 架构在 Binius64 内自建。

## 关键文件 (新)
- `research/zkvm-vs-circuit-constraints-conceptual.md`
- `research/zkvm-backend-replacement-feasibility.md`
- `designs/binius64-baseline-decision.md`
- `designs/binius64-constraint-proofs-and-zkvm-plan.md`
- `designs/binius64-frontend-api-map.md`
- `../binius64/crates/zkvm-slice/{README.md, src/slices/inst_lookup.rs, src/slices/mem_lookup.rs, src/slices/pc_glue.rs, src/slices/pc_carry.rs, src/slices/instr_step.rs, src/slices/multi_inst.rs, src/slices/branch.rs, src/slices/factorial.rs, src/slices/combined.rs, src/slices/multi_combined.rs, src/slices/mem_instr.rs}`
