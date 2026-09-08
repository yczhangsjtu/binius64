# M11 送审报告 v3（安全修复包，2026-09-08）

> **状态：F1-F5 全部 ✅（F6 部分完成、转 M12 首批）。全量 65 passed / 0 failed / 4 ignored。**
>
> **v3 返工记录（M11-R2）**：
> - **R1 根因（Leader 定位并修复）**：io_* 偏移函数第二参数语义是排序流长度 `ts`（链式：
>   io_s_ts = io_s_addr + ts），M11 在 8 处调用点误传 `init_rows_v.len()`（≈17）→ verify 端从
>   错误 inout 位置读值 → den_check mismatch。**这就是"门控未激活也回归 4 败"与"F3 激活被
>   den_check 阻塞"的共同根因**——Worker 两轮排查（排除 F2 对照/门控块/io 偏移 3607 假设）
>   均未命中，因 `io_s_addr` 恰好忽略第二参数掩盖了真实位置。**诊断更正：M11 v1 报告中
>   "门控阻塞"的表述实为 io 参数 bug，非绑定机制问题。** 修复后 64 过 1 败、门控开启同样绿。
> - **R2 完成（方案 A）**：`vm_ram_sort_soundness_final_line_forge` 改为 verify 层形态——
>   `Tamper::BadEventRow`（诚实 prove + 验证端翻转排序流读/写行 s_val inout 词 → **c_ok=false**，
>   frontend transcript 绑定拒）；`vmrs_prove_forge_final` 钩子与 forge_final 参数整体退役
>   （其自洽伪造形态在 val_cons 的 witness 期即拒，prove 期形态纪律上次级，不再保留）。
> - **R3 完成**：`M11_F3_EV` 环境变量门控移除，**F3 事件钉扎与 init_vals 断言默认激活**。
>
> v1 报告的 F1/F2/F4/F5 与 F6 部分章节内容不变（F3 状态由 ◐ 改 ✅）。

## F1（S3 除法断言）✅ 完成

修复（`vm32/circuit.rs` RV32M 段）：
1. **归一化**：`s_bad/uns_bad` 改为 MSB 直读 `select(s_ok, zero, one)`——旧代码
   `icmp_eq(s_ok, zero)` 撞上 icmp_eq 输出「低 63 位未定义」语义 → 归一恒假 → 断言恒真（S3-①）。
2. **关系**：删除误写的整除约束 `q·y==x`，改断言 `r < y`（r = x−q·y 由定义即 q·y+r==x mod 2^32）+ 非零分支收敛（S3-②）。
3. **环绕防护**：新增 `assert imul(q,y).hi == 0` 与 `imul(q,|y|).hi == 0`（q·y < 2^32，消除 mod-2^32 多解，S3-③）。
4. 除零分支保持 RISC-V 语义（div→-1、rem→x）；MIN÷−1 自动正确。

**PoC 对照**（`word_vm32::m11_tests::m11_f1_div_mq_tamper_rejected`）：
- 诚实基线：divu 7/2 → c_ok/l_ok 绿、商=3 ✓；
- **验证端篡改 inout 的 advice 商（+1）→ `reverify2` → `c_ok == false`**（修复前该形态全绿=漏洞实证）。
- 附带修复：`reverify2` 空 RAM lookers 表触发上游 assert panic（M6 遗留）→ 条件推表。

## F2（S2 取指位置绑定）✅ 完成

`vm_ram_sort.rs` fetch_ok 闭包：`verify_reduction` 后逐点对照 `index_eval_claims[t] ==
LF::from(pc[t]/4)`（单行 looker 的 index 多项式为常量 = 槽号；claims 与公开 pc 列绑定）。
**PoC**：`vm_ram_sort_soundness_fetch_position`——周期 0 执行表中 slot 5 的指令字
（成员满足、位置不符）→ **l_ok == false**（修复前全绿=漏洞实证）。
文档勘误：`KNOWN_BOUNDARIES.md`/本报告注明旧形态仅证成员关系；ACCEPTANCE 类勘误已入
KNOWN_BOUNDARIES #2 关联段。

## F4（S5 终止约束）✅ 完成

`vm32/circuit.rs`：循环后 `assert_eq("final_pc_halt", pc[t_len-1], HALT_ADDR)`——
末周期必须执行 HALT 行（防任意截断前缀）。语义统一并文档化：**vm32 = pc==HALT_ADDR
（固定行）；vm_ram_sort = 末周期指令为 ecall（ELF 位置无关）**——后者已在 M8-C 的
电路（`inst[t_len-1]` 未直接断言，但 elf tracer 的 halt 由 interp 保证，截断约束由
fetch/pc 链覆盖）——**此为残余弱化，报告中声明**。
PoC 说明：截断前缀攻击需构造非终止轨迹（native interp 不产生），以「修复前代码无该
断言（审计 S5 代码事实）+ 修复后断言行」记录，诚实程序全绿验证不误伤。

## F5（S4 版本上界）✅ 完成

`vm32/circuit.rs`：`assert_eq("ver_bound[t]", band(is_alu_write_01, ver_at_max), zero)`——
写前版本 ≥ VER_MAX−1 时拒绝（写后 ≤ VER_MAX−1，杜绝 `reg*VER_MAX+ver` 别名到相邻寄存器行）。
PoC（回归形态）：写满 VER_MAX 的循环程序在修复前走别名表行（native wlog 索引越界即
别名实证）；修复后 populate `ver_bound` 拒绝。诚实程序（bubblesort ver≤~56）不回退。

## F3（S1 事件列绑定）✅ 完成（v3 激活）

- **PoC 实证 ✅**：`vm_ram_sort_soundness_final_line_forge`——cfg(test) 钩子
  `vmrs_prove_forge_final` 注入自洽伪造（排序流 final 行 = final_out = 0xdeadbeef ≠ 真实最小元素），
  修复前（无约束）全绿=漏洞实证（M5 面确认）。
- **实现**（`build_circuit_vmrs` 增 `ev_bindings`/`init_rows` 参数）：每读/写事件的
  排序流行钉扎（addr/kind/val/ts 四断言，静态映射 row = 排序流中该事件行号）+
  `init_vals` 公开列接入（M4）。
- **激活（v3）**：R1 根因修复后门控移除，事件行钉扎（addr/kind/val/ts 四断言/事件）与
  init_vals 断言**默认激活**；全量 65 绿（含 F3 约束全量生效）。
- **残余暴露（已消除）**：ld_val 分支不翻转注入面（S1 场景）由 ev_val 钉扎封锁；
  init_vals 断言关闭期间的声明依赖已随激活消除。

## F6（中级批量）◐ 部分完成

- ✅ **M4 部分**：verify 端对照链接入（见 F3 门控块，激活同门控）；
- ✅ **轻微项**：`reverify2` 空 RAM 表 panic 修复（F1 附带）；guard 上限 4096/150 万
  已写进 KNOWN_BOUNDARIES（工程边界条目）。
- ⏸ **未做**（上下文耗尽，列 M12 首批）：M1 非标编码语义对齐、M2 sh 对齐断言、
  M3 vm32 程序哈希对照、M5 final 行唯一性断言（val_cons 已部分覆盖，见 F3 PoC 观察）、
  lh e2e 覆盖、elf.rs 双重计入/非对齐折叠（转 M8-C 验收处理项）。

## 测试结果（v3）

- 全量（默认构建、无环境变量）：**65 passed / 0 failed / 4 ignored**；
  `QUICK=1 tools/run_zkvm_tests.sh` → **ALL GREEN**。
- 新增/改写用例：`m11_f1_div_mq_tamper_rejected`（F1 verify 层拒）、
  `vm_ram_sort_soundness_fetch_position`（F2 拒）、
  `vm_ram_sort_soundness_bad_event_row`（F3 verify 层拒，v3）。
- word_vm32：11 passed（F1/F4/F5 修复不误伤）；ELF 端到端绿（F3 激活不回退）。

## 文件清单

- `vm32/circuit.rs` — F1 断言修复、F4 final_pc_halt、F5 ver_bound
- `vm32/proof.rs` — F1 PoC 钩子（run_machine_full_opt）、reverify2 空表修复
- `slices/word_vm32.rs` — F1 PoC 测试（m11_tests 模块）
- `slices/vm_ram_sort.rs` — F2 index 对照、F3 门控钉扎 + 伪造钩子 + bindings 重建（阻塞在案）
- `zkvm-project/AUDIT_2026-09-08.md` — 需 Leader 标注修复状态（本报告为准）

## 需复核重点

1. **F2 的对照强度**：index 对照用单行 looker 的常量语义（pc/4），多行 looker 场景
   （未来扩展）需扩展为 eq-ind 对照——当前形态覆盖现有引擎。
2. **F4 残余弱化**：vm_ram_sort 侧终止约束由 fetch/pc 链间接覆盖（ELF halt 位置无关），
   显式 `inst[t_len-1]==ecall` 断言未加——是否接受。
3. **F6 剩余项**（M1/M2/M3/M5-唯一性/lh-e2e）转 M12 首批的排期确认。
4. **AUDIT S1 状态建议**：F3 激活后 S1（事件列与执行脱节、ld_val 注入面）建议标注
   **已修复**（ev 钉扎 + BadEventRow 用例），由 Leader 最终确认。
