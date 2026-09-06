# zkVM 项目验收基准（ACCEPTANCE BASIS）

> 本文件是**验收 Agent 的对照基准**（2026-09-05 生成）。它的作用是给后续每一次
> 报告提供一个**独立可复现的事实基线**，防止任何"遇到困难走 shortcut 后谎报成功"。
> 所有内容都来自**实际可运行命令**，不是推测。若后续报告中出现与此不一致的表述，
> 以本文件的"可复现验证"为准，提请报告者提供命令输出作为证据。

---

## 1. 项目背景与定位

- **目标**：在**二元域（GF(2^128)）**上构建支持 RISC-V 指令集证明的 zkVM。核心论点：
  zkSNARK 证明成本只随"执行的指令条数"缩放，与指令类型无关。
- **技术路线**：不使用 Jolt（素域/BN254），而是用 **Binius64**（成熟二元域证明后端）的
  `logup*`（查表）+ `spartan-prover`（R1CS 约束）承载 zkVM 的各个子机制。
- **关键诚实前提**：本项目目前验证的是**"单机制可行"**，**不是**一个完整可证明任意程序的
  zkVM。完整 zkVM 所需的**内存时序论证**、**通用跨行状态机**等尚未实现（见 §4）。

### 仓库位置
- 工作目录（项目根）：`/home/yczhang/workspace/binius64`
- 这是 `binius-zk/binius64` 的 **fork**，`origin = https://github.com/yczhangsjtu/binius64.git`
- 当前分支：`main`；最近提交：`f1f5eed`（重构为 lib crate）、`5f31e95`（文档诚实化）、
  `6a76e11`（zkvm 切片）、`0eb0aa0`（mem_arg_spice）、`1443a6a`（架构文档）、`ada5e0b`（切片+文档）
- 旧目录 `~/workspace/binaryfield-zkvm` 是 flock 时代遗留（未参与当前验证），现仅副本

## 2. 代码结构（重构后，commit f1f5eed）

```
crates/zkvm-slice/
├── Cargo.toml            # 无 [[bin]]，只有 [lib]；依赖相对路径 ../compute ../field ...
├── scripts/migrate_slices.py  # 批量把 bin→lib 模块的迁移脚本
└── src/
    ├── lib.rs            # crate 根：pub mod alu / encode；#[path] 引入 20 个切片模块；
    │                     # 重导出 run_<name>
    ├── alu.rs            # 共享 ALU/位级工具
    ├── encode.rs         # 共享 RISC-V 风格 word 编码 + 字段提取
    └── slices/           # 20 个验证切片（原 src/bin/），每个 fn main → pub fn run_<name> + #[test]
        ├── inst_lookup.rs  mem_lookup.rs  pc_glue.rs  pc_carry.rs  instr_step.rs
        ├── multi_inst.rs  branch.rs  factorial.rs  combined.rs  multi_combined.rs
        ├── mem_instr.rs  mem_arg.rs  mem_arg_ts.rs  jolt_bridge.rs  mem_arg_spice.rs
        ├── full_vm.rs  full_vm_store.rs  full_vm_multi.rs  full_vm_jolt.rs  zkvm.rs
```

### 共享模块内容（去重成果）
- `alu.rs`：`to_bits(val,nbits)`、`fa`（全加器）、`add_constant`、`inc8`、`mul8`、`leq8`、
  `native_xor`、`assert_bits`。重构前 `to_bits` 有 13 份拷贝、`fa` 有 12 份；现各 1 处。
- `encode.rs`：`OP_*`/`F3_*`/`REG_*` 常量、`word_opcode/rd/funct3/rs1/rs2/imm/bimm`、
  `enc_addi/enc_add/enc_lw/enc_sw/enc_beq`。

## 3. 可复现的验证命令与输出

### 3.1 运行全部 20 个机制测试（唯一真相来源）
```bash
cd /home/yczhang/workspace/binius64
export RUSTFLAGS="-C target-cpu=native"
CARGO_BUILD_JOBS=4 cargo test -p binius-zkvm-slice
```
**真实输出**（2026-09-05 实测）：
```
running 20 tests
test inst_lookup::tests::inst_lookup ... ok
...（20 个全 ok）...
test result: ok. 20 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.11s
```
> 语义：这 20 个测试各自执行**prove → verify + 一次"故意篡改"被拒**。都通过说明
> *这一机制在二元域上闭环 + 能拒绝伪造*。**它不等价于"完整 zkVM"**。

### 3.2 单个切片测试
```bash
CARGO_BUILD_JOBS=4 cargo test -p binius-zkvm-slice --lib factorial
```

## 4. 逐切片诚实分级（验收时据此核对）

> ⭐ = 真正实现（机制闭环 + 有意义） ； ⚠️ = 演示/边界（机制可行但被简化或含夸大）。

| 切片 | 声称的机制 | 真实程度 | 关键边界 |
|---|---|---|---|
| `inst_lookup` | logup* 指令查表 | ⭐ 真 | 表 = AND 真值表，trace 为固定输入 |
| `mem_lookup` | 内存一致性查表 | ⚠️ | 表**手工给定**，只证明 claim∈表，非"latest" |
| `pc_glue` | 寄存器/PC 状态流转 | ⭐ 真 | 3×xori，ALU 多指令 |
| `pc_carry` | PC 整数进位(全加器链) | ⭐ 真 | 8-bit，演示机制 |
| `instr_step` | 单指令取指→译码→执行→写回→PC+4 | ⭐ 真 | `xori` 一条 |
| `multi_inst` | 多指令序列(寄存器依赖链+PC续流) | ⭐ 真 | 4×xori |
| `branch` | beq 条件分支(相等检测+MUX) | ⭐ 真 | 位级乘法树+布尔MUX |
| `factorial` | 含循环程序(乘法+加法+分支) | ⭐ 真 **有跨行状态迁移** | 8-bit，展开固定轮数 |
| `combined` | 组合证明(logup*+Spartan 同 transcript) | ⭐ 真 | 单条 addi |
| `multi_combined` | 多指令组合(查表取指+执行) | ⭐ 真 | 2 条 addi |
| `mem_instr` | store/load + R-A-W 门 | ⚠️ | **单地址硬编码** `mem_r==mem_w` |
| `mem_arg` | 读⊆写 sub-multiset | ⭐ 真**雏形** | store+load 同锁一表，但表 native 填 |
| `mem_arg_ts` | 带时间戳内存论证 | ⚠️ | 表手工构造，version trace 显式给出，未证排序 |
| `jolt_bridge` | Jolt 前端→后端桥接 | ⚠️ | 仅"数据形状兼容"，非"证明机制等价" |
| `mem_arg_spice` | SPICE 排序内存论证 | ⚠️ | 表 native 手工构造，仅证 value∈T[ts,addr]，**无 sorter/时序论证** |
| `full_vm` | 完整 zkVM(循环+内存+整数) | ⚠️ 演示 | 有跨行 x1/i/pc[t+1]；执行模板化、读见最近写为手工填值 |
| `full_vm_store` | zkVM+store(读写循环) | ⚠️ 演示 | 有跨行；"读见最近写"为 run_program 算出的常量 |
| `full_vm_multi` | 多地址交替读写 | ⚠️ 演示 | 有跨行；"读见最近写"手工构造 |
| `full_vm_jolt` | word 驱动 opcode 解码 | ⭐(部分)/⚠️ | ①word 位解码(word[7:6]→is_addi/is_beq)属实 ②跨行 x1/pc([c+1])属实；真实边界=单累加器 x1、无寄存器堆、仅 addi+beq、limit 常量、无内存操作 |
| `zkvm` | 整合 zkVM | ⚠️ **非整合** | `let _=pc`(PC未约束)、row.op 为 match 死枚举、a/b 无跨行传递、内存表手工填 |

### 4.1 最重要的诚实边界（必须如实呈现）
1. **"读见最近写"的时序论证未实现**：现状是 native 程序把正确值直接填进表，
   logup* 只证明**一致性**（claim∈表），**不证明时序**（该值确为最近一次写）。
   切片 `mem_instr/mem_arg_ts/mem_arg_spice/full_vm_*/zkvm` 均如此。
2. **"排序/时序论证"(Twist/Shout sorter)从未实现**。
3. **跨行状态机覆盖（已修正表述）**：`multi_inst`/`multi_combined`/`full_vm(16-18)`/
   `full_vm_jolt(19)` 均**有**跨行寄存器/PC 绑定（见各自 `[t+1]`/`[r+1]`/`[c+1]` 约束行）；
   **仅 `zkvm.rs(20)` 无跨行**（`let _=pc`，PC 未约束）。`full_vm_*` 的真实局限是**执行
   模板化、无真正指令译码/寄存器堆、内存时序为 native 手工填值**——而非"无跨行"。
4. **指令 word 解码不完整**：`zkvm.rs` 的 `row.op` 来自 `run_program()` 的枚举
   （native match 死），**不是**从 word 的 opcode 位解码。

## 5. 验收建议（如何核对一份新报告）

对每一份声称"已实现/已跑通 X"的报告，验收 Agent 应：
1. **要求可复现命令**：`cargo test -p binius-zkvm-slice`（或指定 `--lib <name>`），
   核对是否真的通过。
2. **对照 §4 边界**：该报告声称的功能是否落在 ⚠️ 区？若是，必须明确标注"这是演示/边界"
   而非"完整实现"。
3. **对抗"快捷方式谎报"**：重点追问——"你实现的 X 是否真正把困难部分做进约束/论证，
   还是用了 native 预计算 + 查表一致性？"（后者是本次积累的核心教训）。
4. **跨行/时序检查**：声称"状态机/内存论证"时，要求指出**跨行寄存器传递**和**时序排序**
   在代码中的具体约束点；若找不到，即未实现。

## 6. 环境（复现必需）
- Rust **1.97.1**（需 `rustup toolchain install 1.97.1`；本机默认 1.95）
- 构建参数：`export RUSTFLAGS="-C target-cpu=native"`（i5-12400F，AVX2 无 AVX-512）
- OOM 防护：`CARGO_BUILD_JOBS=4`（12 核 32GB 并行会 SIGKILL）
- 工具：`uv`/`cargo` 均已装；`gh` 已登录（yczhangsjtu）
