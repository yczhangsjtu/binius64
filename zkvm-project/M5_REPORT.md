# M5 送审报告 v3（2026-09-07，M5-R5-bubblesort 收尾）

Workdir: `/home/yczhang/workspace/binius64`。范围：`crates/zkvm-slice/` + `zkvm-project/`；未改动上游、无 git 操作。
约束：`RUSTFLAGS="-C target-cpu=native"`、`CARGO_BUILD_JOBS=4`。切片 26 `word_vm32`，M5 单代码骨架（`word_vm32.rs`）未另起架构。

版本说明：v1（首轮 30 条 torture）→ v2（M5-rework R1-R7 返工，R5 bubblesort 标记缺口）
→ **v3（本轮 M5-R5-bubblesort.md 收尾：R5 闭合 + 发现并修复 S 型 store 地址 soundness bug + VER_MAX 放大 + 全警告清理）**。

## 结论
M5 全部验收项闭合：F1 修复（is_sra/is_sub）、torture 50 条 + 对照证据、两例分层拒绝、译码篡改、
**bubblesort（任务书 T4 第二程序）端到端全门级证明通过**。全量 **29/29 测试绿**，word_vm32.rs 编译警告清零。

## R5：bubblesort 端到端（本轮核心）

### 1) 参数化程序镜像管线
`build_fetch_prog(word_overrides)` 现在**应用 word_overrides**：先按 fetch_word 填充 50 个 torture 槽位，
再把 override 词写入 fetch 表对应行。由此 `run_machine_full` 的 `word_overrides` 成为程序镜像的正式注入通道
（v2 中它不是合法镜像、只作恶意注入，语义已变更——见 §R3 语义迁移）。

### 2) bubblesort 程序（bubble_words，15 个槽位）
8 元素无符号升序冒泡，数据含重复与 `0x80000000`：

| 地址 | 指令 | 说明 |
|---|---|---|
| 0x00 | addi x3,x0,7 | 常量 7 |
| 0x04 | beq x1,x3,+48 | 外循环终止（i==7 → done 0x34） |
| 0x08 | addi x2,x0,0 | j=0 |
| 0x0c | add x4,x0,x2 | addr=j |
| 0x10 / 0x14 | lw x5,0(x4) / lw x6,1(x4) | mem[j], mem[j+1] |
| 0x18 | bltu x5,x6,+12 | a<b 跳过交换 |
| 0x1c / 0x20 | sw x6,0(x4) / sw x5,1(x4) | 交换（升序冒泡） |
| 0x24 | addi x2,x2,1 | j++ |
| 0x28 | bne x2,x3,-28 | 内循环 7 次 |
| 0x2c / 0x30 | addi x1,x1,1 / bne x1,x3,-44 | 外循环 7 次 |
| 0x34 | jal x0,+144 | 跳到 halt 0xc4 |
| 0xc4 | addi x0,x0,0 | halt |

寄存器写频：x2/x4/x5/x6 ≈ 56 次（内层 49 轮）→ **VER_MAX 16→128**（见 §架构调整）。
PC 只经过 0x00..0x34 与 0xc4（jal 一步到位），其余槽位保持 torture 词但从不被取指。

### 3) 端到端证明（word_vm32_bubble）
- **native 对拍**：`cycles=391`，final_mem[0..8] == `[1,3,3,5,5,0x80000000,0x80000000,0xffffffff]`
  == 独立求值的 `exp.sort()`（公共期望，不从 trace 导出）→ `match_exp=true`。
- **prove/verify/logup\***：`cycles=391 c_ok=true l_ok=true n_gates=382660 n_and=141805 n_bmul=181163`
  （≈979 gates/cycle，391 周期全门级电路 + 三表 logup\* 诚实证明）。
- **output 三件套交叉核对**：对每个排序地址 a∈0..8，`ram_wlog[a*VER_MAX + final_ramver[a]]`（最高版本行）== 期望值 → **8/8**。

## ★ 新发现并修复：S 型 store 地址 soundness bug（R5 暴露）
bubblesort 的 `sw` 用小偏移（0/1），首次暴露出 **store 地址计算用了 I 型 imm_i（inst[31:20]）**：
- 病根：S 型偏移在 `inst[31:25]+inst[11:7]`（imm_s），而 `inst[31:20]` 的低 5 位是 **rs2**。
  `sw x6,0(x4)` 被算成 `addr = x4 + rs2(=6)` → 写 mem[6] 而非 mem[0]，排序全乱且值损坏。
- 为何 v1/v2 torture 没暴露：唯一 store `sw(8,0,24)` 恰好 rs2=8，imm_i=inst[24:20]=8 → 读写错配
  （lw 读 mem[24]、sw 写 mem[8]）但 **native 与电路同错**，自洽闭环掩盖了两轮验收。
- 修复（native + 电路双侧一致）：
  - native `run_program`：store 分支用 `imm_s` 重算地址（load 仍 imm_i，正确）；
  - 电路：新增 `imm_s` wire（S 型位域拼装 + 符号扩展），`st_addr_w = select(is_store, rs1+imm_s, rs1+imm_i)`（非 store 周期回退 imm_i 以匹配 witness fallback），RAM 版本链与 `st_addr`/`st_ver` assert 全部改用 st 地址。
- 修复后 torture 全量回归通过（sw 现在真写 mem[24]——语义变真，native/电路/表三方一致）。

## 架构调整（R5 必需）
- **VER_MAX 16→128**：bubblesort 循环寄存器写频 ~56（x2/x4/x5/x6），16 上限会被版本链 logup 索引碰撞击穿。
  `M_W_REG` 9→12（32×128=4096 行）、`M_W_RAM` 10→13（64×128=8192 行）。torture 表同步放大，无行为变化。
- `Trace` 增加 `final_mem: [u32; NRAM]`（native 终态内存，供对拍输出三件套）。
- `run_program` guard 256→4096（bubble 391 周期）。

## R3 语义迁移（override 新合同）
v2 的 R3b（override 表外语义词 → fetch logup\* 拒）依赖"override 不进 fetch 表"。R5 参数化后 override 表是
**合法程序镜像**（进表），该构造不复存在——这是合理的新合同：程序镜像与 fetch 表恒一致（构造保证）。
- R3b 改为第二个分层拒绝例（RAM logup\* 层，程序跨 bubblesort）：load-override 把首个 mem[j] 读注入 `0x22222222`，
  native 真迹自洽 → `c_ok=true`；RAM 表行（0x80000000）≠ 注入值 → `l_ok=false`。实测 `(true,false)` ✓。
- 直接篡改 witness 无法得到 `c_ok=true`（final_regs/fin_ver 断言钉死值/版本链），分层拒绝必须经 native 注入，
  这正是 load_overrides 通道（R3a 同机制：torture + 0x11111111）。
- 坏取指类篡改由 R4 / soundness-5 在约束层覆盖（均正确拒绝）。

## 测试结果（全量）
`cargo test -p binius-zkvm-slice --lib` → **29 passed; 0 failed**（v2 的 28 + `word_vm32_bubble`）。
- torture honest：`cycles=50 c_ok=true l_ok=true n_gates=44784`；soundness 1-5 `rejected:true ×5`。
- 分层拒绝：`[R3a] (true,false)`、`[R3b bubble] (true,false)`；译码篡改 `[R4] rejected=true`；覆盖 **44 条**。
- bubblesort：native 391 cyc 排序正确 + prove `(true,true)` 382,660 gates + 三件套 8/8。

## 警告清理（任务书第 5 步）
word_vm32.rs 的 19 处编译警告全部清零（冗余括号、unused mut、is_load/is_store 死变量、未用 io_\* helper、
Cycle 死字段 alu_sum/pc_next、unused idx 变量等；`final_mem`/`bubble_words` 为测试专用标 `#[allow(dead_code)]`）。
其余切片（zkvm.rs 等 M1-M4 交付物）的 18 条历史警告未动，避免改坏已验收代码。

## 文件清单
- `crates/zkvm-slice/src/slices/word_vm32.rs` — v3 全部改动（R5 管线/镜像/测试 + store 修复 + VER_MAX + 清理）。
- `zkvm-project/M5_REPORT.md` — 本文件（v3）。
- 坏门基线：`/tmp/word_vm32.r1.bad_baseline.rs`（R7 对照证据，不入库）。

## 需复核重点
1. **S 型 store 地址修复**（本轮最重要的 soundness 修复）：确认 `imm_s` 位域拼装
   （`inst[31:25]<<5 | inst[11:7]`，12 位符号扩展）与 `st_addr_w` 的 is_store 门控（非 store 回退 imm_i）。
2. **VER_MAX=128** 的版本链预算：bubblesort x2 写 56 次；torture 各 reg ≤16。"更宽的程序"需在 M6 重估上限。
3. R3b 语义迁移（override=合法镜像新合同）——v2 报告中的"非法取指分层"构造已随参数化消失，由 R4/soundness5 覆盖。
4. 391 周期 × ~979 gates/cycle ≈ 383k gates 的成本量级（O(32+64)·T 版本链主项），M6 需正视。
5. 与 v1/v2 一致：`mul`（加分）未接入；bubble 数据规模 8 字（任务书下限）。