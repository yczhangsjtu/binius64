# M11 任务书：安全修复包（审计驱动，最高优先级）

> 里程碑：M11（原 M11 succinctness 顺延为 M12；见 `designs/binary-zkvm-full-roadmap.md`）
> 派发日期：2026-09-08 | 派发方：Leader Agent | 执行方：Worker Agent
> 依据：`zkvm-project/AUDIT_2026-09-08.md`（全部发现已 Leader 复核确认）。
> **本里程碑高于一切功能开发。** 纪律：每处修复必须给"修复前失败/修复后通过"的对照证据；
> 每个修复配 verify 层 soundness 用例；先复现攻击（PoC）再修复。

---

## F1（S3，执行层）：除法断言修复

`vm32/circuit.rs:264-290`。
1. 归一化 bug：`s_bad = select(icmp_eq(s_ok, zero), one, zero)` 改为直接读 MSB
   （`s_bad = select(s_ok, zero, one)`——MSB-bool 直读，禁止再经全词 icmp_eq 归一）。
   uns_bad 同理。
2. 关系修正：断言 `q·y + r == x ∧ r < y`（非整除也合法），用 imul 结果，
   并**断言 imul 高位 == 0**（防 mod 2^32 环绕多解）。
3. 除零分支保持 RISC-V 语义（div→-1、rem→x）。
4. **先写 PoC**：一个 m_q 篡改用例（商+1）在当前代码应**通过**（漏洞实证），
   修复后必须 verify 层拒绝。报告给对照。

## F2（S2，组合层）：取指位置绑定

logup* 的 index claim 是 prover 消息（ip-prover/logup_star/prove.rs:426-433），
验证端须主动核对：
1. 验证端从公开 pc 列重算索引期望（`LF::from(pc_t / 4)` 之类），与
   `verify_reduction` 输出的 `index_eval_claims` 逐点对照（或等价地在归约点核对
   index 多项式与 pc 列的一致）。
2. 新 soundness 用例：把某周期指令换成**表中另一槽位存在**的指令字（成员关系满足、
   位置不符）→ 修复前通过（漏洞实证）、修复后拒绝。
3. 同步修正所有声称"T[pc]=word"的文档为修复后的真实语义（旧切片若共享同一形态，
   在 ACCEPTANCE 类文档中加一条勘误）。

## F3（S1，组合层）：vm_ram_sort 事件列绑定

恢复 M3 v2 纪律：
1. 事件列 `d_*` 改为由电路派生并 assert_eq 钉到执行 witness（每周期事件行
   addr/ts/val/kind == 电路派生值），或直接让 d_* 成为电路 witness 而非 inout。
2. `ld_val` 的唯一来源必须是内存论证链（d_*→committed 列→fracaddcheck）。
3. 新 soundness 用例（审计场景 X）：改 ld_val + 重造排序流 → 修复前四层全过
   （漏洞实证）、修复后拒绝。

## F4（S5，执行层）：终止约束

电路末尾断言 `pc[t_len-1] == HALT_ADDR` 且末条指令为停机指令；
vm32 与 vm_ram_sort 的 ecall/halt 语义统一（择一并文档化）。

## F5（S4，执行层）：版本上界

电路断言所有 ver 计数 `< VER_MAX`（或等价防爆设计）；加"同地址写满 VER_MAX 次"
的回归测试（修复前别名实证、修复后拒绝或安全失败）。

## F6（中级批量）

- M1：非标编码两侧语义对齐（统一为 trap-like NOP 或精确判等，消除分歧）；
- M2：sh 奇地址对齐断言（与 lh 对称），修 isa.rs 虚标注释；
- M3：vm32 引擎补程序哈希对照（对齐 vm_ram_sort）+ ld_val 的 32 位范围检查；
- M4：verify 内校验 init_hash == hash(init_words)，init_vals 列要么接入要么删除；
- M5：final 行唯一性断言；
- 轻微项：`let _native_ok` 改回 assert；lh e2e 覆盖；guard 上限写进 KNOWN_BOUNDARIES。

## 验收标准

1. 全量测试绿 + 每个 F 的 PoC 对照证据（修复前漏洞可复现/修复后 verify 层拒绝）。
2. F1-F5 逐条指出修复代码行。
3. 文档更新：AUDIT 记录标注"已修复"、KNOWN_BOUNDARIES/SECURITY_REVIEW_PREP 同步、
   S2 相关的"T[pc]=word"夸大表述勘误。
4. M11_REPORT.md（含每例 PoC 输出）。

## 送审要求

`zkvm-project/M11_REPORT.md` + 简短送审消息（≤15 行：各 F 状态、测试结果、需复核重点）。
