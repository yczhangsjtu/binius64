# 汇报：M4 RAM 内存论证（word_vm_ram）

> 汇报 Agent（Hermes）→ 验收 Agent | 日期：2026-09-06 | 基准：`ACCEPTANCE_BASIS §1/§4`、`tasks/M4-ram-memory-argument.md`
> 前置：`M2_REPORT.md`（选型 W2 词级）、`M3_REPORT.md`（通用单周期状态机 word_vm，寄存器版本链+读==写已电路化+logup* 绑定）；`designs/milestone-roadmap.md` §4 M4 定义
> 结论先行：**M4 完成**——在 M3 的 `word_vm` 上推广出 **RAM 内存论证** `word_vm_ram`（切片 25）：**K=64 字地址空间（32-bit 字）**，`lw(0x03)/sw(0x23)` 译码驱动执行，**RAM 读值只由 logup* 钉住**（电路不做跨地址值链，只维护 **64 个版本计数器** = O(K·T)），RAM 写日志表进 logup*（**现共 3 张表**：fetch + 寄存器写日志 + RAM 写日志，单 transcript），**init/final/output 三件套**验证端显式断言。全量 `cargo test` **25 passed**（原 24 + 1），**5/5 soundness 真拒**（含两个**分层拒绝**：电路过 + logup* 拒）。

---

## 0. T1 设计小节（任务书 §1 落地）

**M4 语义**：M3 已有的 addi/add/beq + **新增 lw/sw**，把内存访问的**读-写一致性**纳入 ZK 证明。任务书 §1 的设计理由——M3 只证明了寄存器读==最近写；真实程序需要**内存**，且内存的读必须"看到最近一次写"。M4 用与寄存器完全同构的**版本链**机制，但关键差异是：

- **寄存器**（M3）：每个寄存器有值链（电路内）——寄存器值本来就暴露在电路里，读值可直接与值链绑定。
- **RAM 读**（M4）：内存值**不**进电路值链（否则要为 64 个地址 × 每周期维护值，O(K·T) 值链，且不必要）。改为**只维护 64 个版本计数器**（进电路），**值载荷全部落在 RAM 写日志表 `W_ram[(addr, ver)]`**，读值 `ld_val` 是 public inout，由 logup* 表断言 `(addr, ver, ld_val) ∈ W_ram`。**RAM 读值被"论证"携带，而非电路携带**——这正是"内存论证"的要点。

**三件套**（任务书 §3.3）：
- **init**：验证端显式断言 `W_ram[addr*VER_MAX+0] == init_mem[addr]`（ver=0 行 = 公共初始镜像，对所有地址）；
- **final**：`fin_ver[64]` 作为 public inout 暴露，被 `assert_eq` 钉到**版本链末端** `ver[T]`；验证端据 `W_ram[(addr, fin_ver[addr])]` 计算终态 `M_final[addr]`；
- **output**：指定输出单元（`OUT_ADDR=10`），验证端断言 `M_final[10] == 公共期望值`（本程序 = 累加器结果 39）。

## 1. 任务与范围（T1–T5 对照）

| 任务 | 交付 | 状态 |
|---|---|---|
| T1 设计 | 本报告 §0/§2；inout 块布局、lw/sw 译码、三件套 | ✅ |
| T2 电路层 | `word_vm_ram.rs::build_circuit`：M3 状态机 + lw/sw 译码 + K=64 RAM 版本链 + 事件 inout 钉扎 | ✅ |
| T3 查表与三件套 | RAM 写日志表进 logup*（3 表同 transcript）+ init/final/output 检查 + `claims_from_inout` 扩展 | ✅ |
| T4 soundness | 5 例（含 2 例**分层拒绝**） | ✅ |
| T5 成本/报告 | CircuitStat + 每周期分析 + O(K·T) 边界 + 本文 + 四处文档更新 | ✅ |

## 2. 架构设计（§3 决策落地）

### 2.1 指令编码
- `enc_addi(rd,rs1,imm7)`、`enc_add(rd,rs1,rs2)`、`enc_beq(rs1,rs2,imm7)`（M3 复用）；
- 新增 **`enc_lw(rd,rs1,imm7)`**（I-type，funct3=0x02）：`0x03 \| rd<<7 \| 0x02<<12 \| rs1<<15 \| imm7<<25`；
- 新增 **`enc_sw(rs2,rs1,imm7)`**（S-type，funct3=0x02）：`0x23 \| rs1<<15 \| rs2<<20 \| imm7<<25`（rd 字段留 0，用 rs2 作数据源）。
- 译码：`opcode=inst&0x7f`、`rd=inst[11:7]`、`rs1=inst[19:15]`、`rs2=inst[24:20]`、`imm7=inst[31:25]`（funct3=inst[14:12]）。`is_load=(op==0x03 && funct3==0x02)`、`is_store=(op==0x23 && funct3==0x02)`。

### 2.2 寄存器（M3 沿用，值链 + 版本链）
- 值链：`reg[r]' = select(is_write_r, write_back, reg[r])`，`write_back = select(is_load, ld_val, alu_sum)`（load 的 rd 写回 **ld_val**，ALU 的写回 **alu_sum**）；
- 版本链：`rver[r]' = rver[r] + is_alu_write`（is_alu_write = is_addi ∨ is_add ∨ is_load，load 也写回寄存器）；
- 寄存器读写事件 inout + logup* 写日志表（M3 v2 不变，`M_W_REG=6`）。

### 2.3 RAM（M4 新增，K=64）
- **版本链**（进电路）：`ramver[t+1][a] = ramver[t][a] + (is_store && mem_addr==a ? 1 : 0)`，遍历 **a∈0..64** → **O(K·T) = 64×28 计数器**（T1 已知边界）；`mem_addr = iadd_32(rs1, sext(imm)) & 0x3f`；
- 读版本：`read_ram_ver = mux64(ramver[t], mem_addr)`（64 输入 mux，选择器=mem_addr 低 6 位）；
- 写后版本：`store_new_ver = read_ram_ver + 1`；
- **值不建链**：`ld_val` / `st_val` 为 public inout，分别被 `ld.val==write_back`、`st.val==rs2` 钉到电路，再由 logup* 断言属于 `W_ram`。
- **事件 inout 钉扎**：
  - load：`ld.addr==mem_addr`、`ld.ver==mux64(ver[t],addr)`、`ld.val==write_back`、`is_load==select(is_load,1,0)`；
  - store：`st.addr==mem_addr`、`st.ver==store_new_ver`、`st.val==rs2`、`is_store==select(is_store,1,0)`。

### 2.4 inout 块布局（平铺，20 字段/周期 + 尾部）
`[inst][pc][rd1_reg|ver|val][rd2_reg|ver|val][wr_reg|ver|val|iswrite][ld_addr|ver|val|is_load][st_addr|ver|val|is_store]` ×28 周期，然后 `init_regs[8]+final_regs[8]+fin_ver[64]`。`claims_from_inout(&inout_words, t_len)` 从该平铺块重建**全部三组 claim**（fetch + 寄存器读写 + RAM 读/写），**不读 native trace**（R2 纪律）。

### 2.5 三张 logup* 表
| 表 | 大小（m_vars） | 内容 |
|---|---|---|
| fetch 程序表 | 64（M_FETCH=6） | 地址→指令字 |
| 寄存器写日志 W_reg | 64（M_W_REG=6） | `W_reg[(reg,ver)] = value` |
| RAM 写日志 W_ram | 512（M_W_RAM=9） | `W_ram[(addr,ver)] = value` |

> 说明：M4 循环里 **x6 被写 6 次**（ver 达 6），故 `VER_MAX=8`（而非 M3 的 4），否则单索引 `reg*VER_MAX+ver` 在 ver≥4 时与相邻寄存器碰撞（曾致 logup* 误拒，见 §6 修因）；`M_W_REG=6`、`M_W_RAM=9` 相应更新。

## 3. 实现（`word_vm_ram.rs`，切片 25）

注册：`#[path = "slices/word_vm_ram.rs"] mod word_vm_ram;` + `pub use word_vm_ram::run_word_vm_ram;`。单文件自含：`run_program`(native trace) / `mux`(mux8/mux64 通用) / `build_circuit` / `claims_from_inout` / `build_reg_wlog` / `build_ram_wlog` / `check_init` / `check_final_output` / `run_machine_full`（prove+verify+logup* 3 表，保留 prover/verifier/witness 供 soundness re-prove）/ `reverify` / `run_word_vm_ram`。

## 4. 测试结果（验收标准 #1 达成）

`cargo test -p binius-zkvm-slice` → **25 passed, 0 failed**（原 24 + word_vm_ram）。`word_vm_ram.rs` **零警告**。

**参考程序**（28 周期，跨迭代 RAW 依赖）：`x4=limit=3, x7=base=8, mem[8]=5, mem[9]=7, OUT=mem[10]`
```
0x00 lw x6,x7,0   0x04 add x1,x1,x6   0x08 addi x6,x6,1   0x0c sw x6,x7,0
0x10 lw x2,x7,1   0x14 add x1,x1,x2   0x18 addi x3,x3,1   0x1c beq x3,x4,+8
0x20 beq x5,x5,-32   (loop 0x00)     0x24 sw x1,x7,2     0x28 addi x0,x0,0 (halt)
```
**native trace**：`final x1=39`, `ramver[8]=3`(写3次), `ramver[9]=0`(仅读), `ramver[10]=1`(输出)。

**COMBINED proof**：frontend (M4 prover) + logup*(fetch + W_reg + W_ram) **单 transcript**，成功（`c_ok && l_ok`）。

**三件套验证**：`init`（W_ram ver0==init_mem）✅、`final`（`fin_ver[.]` 钉版本链末端）✅、`output`（`M_final[10]==39`）✅。

## 5. 成本 / 性能分析（T5）

**电路约束（10 周期 → 28 周期）**：`ZERO=653  AND=6902  IMUL=0  BMUL=4709  (gates=12743)`。

**主要成本**：**O(K·T) RAM 版本链** = 64 计数器 × 28 周期，每周期每地址一个 `icmp_eq(mem_addr, a)` + `select` + `iadd`，是当前最大的常数项（gate 数随 K·T 线性增长，与指令数无关）。约 **454.7 gate/周期**（12743/28）。

**诚实分级**：
- **RAM 读值只由 logup* 钉住**——电路侧对内存值不做任何算术，`ld_val` 的正确性完全由 `W_ram` 表绑定（⭐，这是 M4 的招牌：M3 的值链被"论证"替代）。
- **成本随 K·T**（非指令位宽），这是任务书 §2 已声明的**已知边界**，不是缺陷；真实 zkVM 用 sorter/hashing 优化，本 M4 为**原理验证**不优化。
- **单一初始镜像 + 字寻址**：不做字节寻址、不做多地址别名检测、不做越界（addr≥64）显式拒绝——越界在本 M4 中经 `& 0x3f` 掩码到合法地址，属**边界**（声明为 ⚠️，见 §6）。

## 6. 诚实分级与边界

| 项 | 等级 | 说明 |
|---|---|---|
| RAM 版本链电路化 + 事件 inout 钉扎 | ⭐ | `ver[t+1][a]=ver[t][a]+(is_store&addr==a)` 进电路，load/store 事件全部经 `assert_eq` 钉到 `mux64`/`addr`/`write_back`/`rs2` 真实 wires |
| RAM 读值由 logup* 钉住（无跨地址值链） | ⭐ | `ld_val∈W_ram` 由 3 表 logup* 断言；电路不建内存值链 |
| init/final/output 三件套 | ⭐ | 验证端显式断言 |
| `addr & 0x3f` 掩码（越界静默到合法地址） | ⚠️ | 未显式拒绝越界读；addr=64+ 会被掩码。诚实声明：本 M4 未做 OOB 硬拒（程序只访问 8/9/10） |
| 版本计数器无溢出上界 | ⚠️ | `VER_MAX=8` 界定了**写日志表**深度，但版本计数可继续增长；只要无单寄存器/地址写 >6 次即无碰撞。声明为演示级约束 |
| `iadd_32` 作为字加法（32-bit） | ⚠️ | 与 M2 相同；非完整 full-adder 进位校验（本 M4 聚焦内存论证，加法沿用 M2 结论） |

## 7. 与任务书 §3 的对照（有无偏离）

- 按 §3 路线落地：RAM 值链→logup* 论证；版本链电路化；3 表同 transcript；init/final/output。**无架构偏离**。
- **需向 Leader 注明的设计选择**：①`VER_MAX` 从 M3 的 4 提升到 **8**（M4 循环 x6 被写 6 次，避免 `reg*VER_MAX+ver` 碰撞），这会增大 `W_ram` 表到 512（m=9）——若 Leader 期望更贴近 M3 的 VER_MAX=4，需缩短循环或改用 `(reg,ver)` 双索引。②`M_FETCH` 从 5 提升到 **6**（程序地址到 0x28=40 > 31）。③越界/别名未做（§6 ⚠️），建议下游 M5 处理。

## 8. 需复核重点

1. **RAM 读值确实只由 logup* 钉住**：`ld_val` 未进电路值链，仅被 `ld.val==write_back`（写回寄存器）与 `3 表 logup*` 绑定——验证端重算的 RAM claim 是否**只**来源于 `inout_words`（非 native trace）？
2. **init 检查在验证端显式断言**：`W_ram[addr*VER_MAX+0]==init_mem[addr]` 是否真正由验证端检查（非仅 prover 侧自证）？
3. **分层拒绝的分层性**：soundness(1)(3) 是否真的"电路过 + logup* 拒"（`c_ok==true && l_ok==false`），而非两类都拒或都过？
4. **O(K·T) 版本链成本**：gates=12743 主要来自 64×28 版本计数器（任务书 §2 已知边界）。此成本是否被预期（真实 VM 会用排序/哈希代替）？选型结论（成本∝指令数而非位宽）是否维持？
5. **VER_MAX=8 / M_FETCH=6 的提升**：是否接受为 M4 的必要调整（否则单索引碰撞 / fetch 表越界）？

## 9. 交付物清单

- `crates/zkvm-slice/src/slices/word_vm_ram.rs` — **新增**（切片 25，M4 内存论证；`run_word_vm_ram` + 5 soundness + 3 表 logup*）
- `crates/zkvm-slice/src/lib.rs` — 注册 `word_vm_ram`（mod + pub use）
- `zkvm-project/README.md` — 切片 23→25，表加第 25 行
- `crates/zkvm-slice/README.md` — 切片 24→25，加 word_vm_ram 条目
- `zkvm-project/PROGRESS.md`、`zkvm-project/designs/milestone-roadmap.md` — M4 状态 → ✅ 已完成
- **`zkvm-project/M4_REPORT.md`** — 本文
