# M5 返工任务：is_sra 解码 bug + 分层拒绝回归 + 任务书硬性缺口

> 派发日期：2026-09-07 | 派发方：Leader Agent | 前置：M5 首次送审（word_vm32.rs，切片 26）
> 任务书 `tasks/M5-rv32i-expansion.md` 继续有效；本文件是针对验收发现问题的增量返工规范。

## 验收发现（附代码证据）

### F1【真 bug，被测试数据掩盖】is_sra 的 I-type 检测位错误
`word_vm32.rs:536`：
```rust
b.select(is_risc, b.band(funct7, b.add_constant_64(0x20)), b.band(b.srl32(inst_w, 11), ...))
```
I-type 分支检测的是 **inst bit 11**（rd 字段第 4 位），而 srai 的标记是 **inst bit 30**
（imm[10]=0x400，`srai()` 编码 `:88` 置的就是它）。torture 唯一的 `srai(5,5,31)`
（rd=5 → bit11=0）被电路误走 shr 路径，因当时 x5=0（srl==sra）而未暴露。
**反例**：任何 `srli rd≥16`（bit11=1）会被电路误算成 srai；任何负数操作数的 srai
会被电路算成 srl。
**修复**：I-type 与 R-type 的 sra 标记都在 inst bit 30（I-type 的 imm[11:5] 正是 funct7
位置），直接统一为 `band(funct7, 0x20)`，删掉 select。

### F2【回归】分层拒绝用例丢失
M4 的招牌 soundness（`run_machine_full` + load_overrides → 电路过 + logup* 拒）在 M5
被丢：`run_machine_full`（`:689`）不再接受 overrides（`run_program` 仍有该参数但无人传），
5 个 soundness 全部走 `reverify`（篡改 inout → 前端 public-match 拒）。任务书 §4 用例 1/3
（RAM 过期读分层拒绝、非法取指分层拒绝）未达成。

### F3【硬性缺口】bubblesort 未实现；译码篡改用例未做；覆盖表未落报告
任务书 T4 要求两个程序（bubblesort + torture）；§4 用例 5（同表内换指令语义）未做；
验收标准 #3 要求指令覆盖表写进报告。

### F4【torture 操作数太弱】
多处移位/比较的操作数结果为平凡的 0（x5 在 srai 点 = 0；sll/slt/sltu 多处得 0），
无负数比较（slt/slti/sltiu 对负操作数）、无符号位翻转的移位用例。

## 返工要求

- **R1 修 F1**：统一 `is_sra` 检测为 `band(funct7, 0x20)`（R/I 同位）。
- **R2 补强 torture（防回归）**：新增用例——
  - `srai`/`sra` 负数操作数（如 `0x80000000 sra 31 → 0xFFFFFFFF`）；
  - `srli` 写 rd≥16 的寄存器（直接踩 F1 的旧 bug 路径）；
  - 负数 slt/slti/sltiu 比较（-1 vs 1，有/无符号各一）；
  - 至少一条产生非平凡高位结果的 sll。
- **R3 恢复分层拒绝**：`run_machine_full` 恢复 overrides 透传；补两例——
  (a) RAM 过期读：load 读旧版本值、执行自洽 → **c_ok=true 且 l_ok=false**；
  (b) 非法取指：表外合法编码字 → **c_ok=true 且 l_ok=false**。
  两例必须断言 `c_ok && !l_ok`（不是笼统的 reject）。
- **R4 补译码篡改用例**：把某周期指令字换成同表内但不同语义的另一条（如 add→sub），
  witness 相应重算 → verify 拒（证明译码语义被电路钉住）。
- **R5 补 bubblesort**：RAM 8 字排序程序（初始值含重复/0x80000000 边界值），
  native 对拍 + output 三件套检查通过。
- **R6 报告**：M5_REPORT.md 标 v2；补 30 条指令 × 覆盖位置 × 对拍结果的覆盖表；
  修正"accept{}"打印标签歧义（直接打印 rejected=true/false）；
  更新成本数字（如变化）。
- **R7 纪律**：不得为让测试通过而挑选"恰好不触发 bug"的操作数——R2 的每条用例必须
  在修复前失败、修复后通过（报告中给出这个对照证据）。

## 验收标准（增量，叠加在任务书 §6 之上）

1. 27+ passed 全过。
2. 指出 is_sra 修复行；torture 含 R2 全部用例。
3. 分层拒绝两例断言 `c_ok && !l_ok`。
4. bubblesort 端到端对拍通过。
5. 覆盖表在报告；R7 的对照证据在报告。

## 送审要求

更新 `zkvm-project/M5_REPORT.md`（标 v2 + 返工说明），回复简短送审消息。
