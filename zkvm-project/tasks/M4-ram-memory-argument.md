# M4 任务书：RAM 内存论证（T1：写日志表 + 版本链，推广到大地址空间）

> 里程碑：M4（权威定义见 `zkvm-project/designs/milestone-roadmap.md` §4）
> 派发日期：2026-09-07 | 派发方：Leader Agent | 执行方：Worker Agent
> 前置：M3 ✅ v2（word_vm，切片 24：译码驱动 + 版本链电路化 + 事件 inout 绑定 + claims_from_inout）
> 本任务书 §3 是 Leader 的架构决策，Worker 按此实现，不要另起架构；发现不可行处停下来
> 在报告中记录原因并给替代方案，不要静默改架构。

---

## 1. 目标与定位

M4 把 M3 的"写日志表 + 版本链 + 读==写绑定"机制从 8 个寄存器推广到 **RAM（K=64 字地址空间）**，
并补上内存**初始态/终态（init/final/output）检查**。

**M4 与 M3 的本质区别（也是本里程碑的存在理由）**：
- M3 的寄存器读值被**双重钉扎**：电路内值链（`reg_cur` select 链）+ logup* 写日志表。
  8 个寄存器的值链电路玩得起，但这条路**不扩展**——真实 RAM 不可能把每个地址的值都拉进电路。
- M4 的 RAM 读值**只被 logup* 钉住**（电路内**没有**跨地址值链）；电路只维护
  **K 个版本计数器**（小整数 wire），值载荷全部在写日志表里。
- 因此 M4 的招牌 soundness 特征必须是**分层拒绝**：一个"读错值但执行自洽"的证明，
  **电路层通过、logup* 层拒绝**——这证明 RAM 读的正确性由论证承担，而非电路。

### 为什么这个设计是声音的（Leader 分析，Worker 照做即可）
- 版本链在电路内：每周期 `ver_{t+1}[a] = ver_t[a] + (本周期 store 到 a ? 1 : 0)`（K=64 个
  计数器，icmp_eq + select + iadd，同 M3 寄存器版本链模式，只是 K 从 8 到 64）。
- 读事件 `(addr, ver_at_read, val)` 与写事件 `(addr, new_ver, val)` 全部暴露为 inout 并钉到
  电路 wire（addr 钉到 `iadd_32(rs1_val, imm)` 的译码计算结果；ver 钉到版本链 mux 输出；
  store 的 val 钉到 rs2 读值；load 的 val 钉到 rd 写回值）。
- logup* 把每个事件 claim 绑定到写日志表 `W[addr*VER_MAX + ver]`。
- **W 的可靠性**：ver=0 行 = 公共初始内存镜像（验证端显式断言，见 T3）；ver≥1 行被写事件
  claim 钉回电路的 store 值 wire；未被 claim 的条目帮不了作弊者（版本链钉死了 claim 的索引）。
- 于是：load 值 == W[(addr, 当前版本)] == 最近一次 store 到该地址的值。**时序正确性由
  电路版本链 + 表绑定共同承担**，无 native 预填"最近写"快照。

## 2. 范围与边界（严格遵守）

- 只允许修改/新增：`crates/zkvm-slice/` 与 `zkvm-project/`。禁改上游 crates，禁 git 操作。
- 构建：`export RUSTFLAGS="-C target-cpu=native"` + `CARGO_BUILD_JOBS=4`。
- **不要动 `word_vm.rs`**（M3 交付物保持原样）。新机制写新切片。
- 规模纪律：固定轮数全展开；K=64 字地址、字=32-bit（装 64-bit word）；指令集 =
  M3 的 add/addi/beq + 新增 lw/sw；寄存器堆维持 M3 的 8 寄存器 + 值链方案**不变**。
- 诚实边界（必须在报告中声明）：
  - 表（程序表 + 寄存器写日志 + RAM 写日志）由 native 构建、双方共享——但 ver=0 行由
    验证端对照公共初始镜像**显式检查**，被 claim 的条目全部由电路钉回；
  - 固定展开、无动态轮数；
  - **版本链是 O(K·T) 电路状态**（K=64 计数器 × T 周期）——这是 T1 路线的已知边界，
    避免 O(K·T) 的 Twist 式 committed-inc 路线属 (B) 移植/后续工作，不在本里程碑；
  - 字节寻址/对齐/符号扩展 load（lb/lh/lwu 等）不做，word 寻址演示机制即可。

## 3. Leader 架构决策

### 3.1 指令与译码（沿用 M3 的简化编码，扩展两条）
- `lw rd, rs1, imm7`：OP=0x03；`rd = M[(rs1_val + sext(imm)) mod 2^32]`（字地址）。
- `sw rs2, rs1, imm7`：OP=0x23；`M[(rs1_val + sext(imm))] = rs2_val`。
- 地址计算 `addr = iadd_32(rs1_val, imm_sext)` 在电路内完成，load/store 事件的 addr inout
  用 assert_eq 钉到这个 wire。
- is_load/is_store 由译码产生；`is_alu_write`（寄存器写回）扩展为 `is_addi|is_add|is_load`。

### 3.2 RAM 状态与事件（核心）
- **版本链（电路内）**：`ver[t][0..64]`，初值 0，每周期按 3.1 的 is_store+addr 递增。
  用 M3 mux 模式做 `mux64(ver[t], addr)` 取读版本。
- **值（电路外，表内）**：RAM 写日志表 `W[addr*VER_MAX + ver]`，VER_MAX 取 4
  （ver0 初始 + ≤3 次写；K×VER_MAX=256=2^8，满足 logup* 表长 2 的幂要求）。
- 事件 inout（每周期，沿用 M3 的平铺块布局 + `io_*` 偏移函数）：
  - load 事件：`(ld_addr, ld_ver, ld_val, is_load)`；
    约束：`ld_addr == iadd_32 输出`、`ld_ver == mux64(ver[t], addr)`、
    `ld_val == 写回 rd 的值`（即寄存器写事件的 wr.val 在 is_load 时钉到 ld_val）、
    `is_load == 译码`。
  - store 事件：`(st_addr, st_new_ver, st_val, is_store)`；
    约束：`st_addr == iadd_32 输出`、`st_new_ver == mux64(ver[t], addr) + 1`、
    `st_val == rs2 读值`（寄存器读链）、`is_store == 译码`。
- `claims_from_inout` 扩展：fetch + 寄存器读写 + RAM load/store 三组 claim，
  全部从 inout_words 重建（**禁止 native trace 取 claim**——M3 v2 的教训）。

### 3.3 init/final/output 三件套
- **init**：验证端显式断言 `W[addr*VER_MAX + 0] == init_mem[addr]`（公共输入）对所有
  addr 成立；寄存器写日志表 ver=0 行同理对照 init_regs inout。
- **final**：每地址最终版本 `fin_ver[0..64]` 作为 inout 暴露，钉到版本链末端 `ver[T]`；
  验证端据 W 计算终态 `M_final[addr] = W[(addr, fin_ver[addr])]`。
- **output**：指定输出单元（如程序累加结果所在的内存字），验证端把 `M_final[输出地址]`
  与公共期望值比对（或把期望值作为 inout 断言）。选定后在报告说明。

### 3.4 参考程序（可微调，必须满足右侧性质）
```
# x4=limit(=3), x7=base(=8), 初始 mem[8]=5, mem[9]=7
0x00: lw   x6, x7, 0       # x6 = mem[8]
0x04: add  x1, x1, x6      # 累加（跨指令依赖：读见最近写）
0x08: addi x6, x6, 1
0x0c: sw   x6, x7, 0       # mem[8] = x6   （同地址反复写 → 版本递增）
0x10: lw   x2, x7, 1       # x2 = mem[9]   （第二地址，交错）
0x14: add  x1, x1, x2
0x18: addi x3, x3, 1       # i++
0x1c: beq  x3, x4, +8      # 退出
0x20: beq  x5, x5, -32     # 跳回 0x00
0x24: sw   x1, x7, 2       # exit: mem[10] = 累加结果（output 单元）
0x28: addi x0, x0, 0       # halt
```
性质要求：≥12 执行周期；≥2 个 RAM 地址、同地址写 ≥2 次（版本链被检验）；
load 读见最近 store（跨循环迭代 RAW）；至少 1 个 output 单元供 final 检查。

## 4. soundness 用例（≥4，全部 verify 层拒绝；第 1 例是 M4 招牌）

1. **过期读（分层拒绝，必须）**：构造一个"load 读旧版本值、其余执行自洽"的机器
   （仿 M3 soundness(3) 的 run_machine 手法：native trace 里让 load 读到过期值，
   寄存器链照常推进）——**电路层通过、logup* 层拒绝**。这证明 RAM 读值由论证承担。
2. **版本篡改**：load/store 事件的 ver inout 改错 → 拒（指明哪一层）。
3. **越界/未初始化读**：load  claim 一个从未写过且不在初始镜像的地址的非零值 → 拒。
4. **初始镜像篡改**：W 的 ver=0 行与公共 init_mem 不符 → 验证端 init 检查拒。
5. **结果篡改**：final/output 相关 inout 改值 → 拒。
（1/3/5 必做且须呈现正确的拒绝层；2/4 必做。）

## 5. 任务分解

- **T1 设计落实（先写后码）**：读 M3_REPORT.md（尤其 §0 v2 返工说明）、word_vm.rs、
  本任务书；把 inout 块布局、lw/sw 译码方案、三件套检查点写成报告 §1 设计小节（半页内）。
- **T2 电路层**：M3 状态机 + lw/sw 译码执行 + RAM 版本链（K=64）+ 事件 inout 钉扎。
- **T3 查表与三件套**：RAM 写日志表进 logup*（多表同 transcript，现共 3 表）；
  init/final/output 检查。
- **T4 soundness**：§4 五例。
- **T5 成本与报告**：CircuitStat（ZERO/AND/IMUL/BMUL）+ 每指令成本 + 版本链 O(K·T)
  开销单列分析；写 `zkvm-project/M4_REPORT.md`（沿用 M3 v2 体例）；更新
  `crates/zkvm-slice/README.md`（切片 25）、`zkvm-project/README.md`（切片表）、
  `zkvm-project/PROGRESS.md`（M4 段）、`zkvm-project/designs/milestone-roadmap.md`（M4 状态）。

### 交付切片
`crates/zkvm-slice/src/slices/word_vm_ram.rs`（切片 25），`pub fn run_word_vm_ram()` +
`#[test]`，lib.rs 注册 + `pub use`。优先单文件自包含（项目切片惯例）。

## 6. 验收标准（Leader 逐项核对）

1. `cargo test -p binius-zkvm-slice` 全过（应为 25 passed）。
2. **RAM 无电路值链**：能指出 RAM 读值仅经 logup* 钉住的证据（无跨地址 select 值链）；
   版本链确为电路约束（指出 ver 递增约束行）。
3. **事件绑定**：load/store 事件的 addr/ver/val inout 全部有 assert_eq 钉到电路 wire
   （指出约束行）；claims 全部从 inout 重建（指出 `claims_from_inout` 类函数，无 native
   trace 残留）。
4. soundness 五例全为 verify 层拒绝；**第 1 例必须呈现"电路过 + logup* 拒"的分层特征**。
5. init 检查为验证端显式断言（指出代码行）；final/output 检查存在。
6. 报告含成本统计、每指令分析、版本链 O(K·T) 边界的诚实标注；四处文档更新，切片计数=25。

## 7. 送审要求

完成后：送审材料落成 `zkvm-project/M4_REPORT.md`，回复一条**简短送审消息**（≤15 行：
结论、文件清单、测试结果一行、需 Leader 复核重点 1-3 条）。细节一律在报告里。
