# M11-R2 返工任务（v2，含 Leader 精确诊断与解法）

> 派发日期：2026-09-08 | 派发方：Leader Agent | 前置：M11 首次送审（61 过 4 败）
> **Leader 已完成根因定位并修复了主根因（R1），本任务只剩 R2/R3 的执行。**

---

## Leader 诊断结论（已验证，Worker 按此执行，不要再排查）

### R1（4 个回归失败 + F3 激活阻塞的共同根因）——已修复

`vm_ram_sort.rs` 的 io_* 偏移函数第二参数语义是**排序流长度 `ts`**（链式偏移：
`io_s_ts = io_s_addr + ts` 等），但 M11 改动在 8 处调用点误传了 `init_rows_v.len()`
（=n_touch≈17）：verify 端的 `dot_pub`（原 :1354-1357）、`pub_sorted` 读取
（原 :1235-1238）、tamper 偏移（原 :1217/:1223/:1290）。验证端从错误的 inout 位置
读值 → den_check mismatch。**这就是"门控未激活也回归 4 败"和"F3 激活被 den_check
阻塞"的同一个 bug**（Worker 两轮排查排除的假设都不在此——`io_s_addr` 恰好忽略第二
参数，掩盖了问题的真实位置）。

**Leader 已应用修复**（第二参数全部改为 `ts`）：修复后 64 过 1 败（回归 4 项全绿），
且 **M11_F3_EV=1 门控开启全量测试同样 64 过 1 败——F3 激活阻塞同步解除**。

### R2（最后一个失败：`vm_ram_sort_soundness_final_line_forge`）——PoC 设计问题

`vmrs_prove_forge_final` 的伪造方式（只改排序流 OUT_ADDR 组 final 行值，`:881-889`）
使排序流自身违反 `val_cons`（final 行 val ≠ 前序写行 val）→ **witness 填充期就失败**
（prove 阶段 panic 于 `:1011`，走不到 verify 层）。这不是绑定机制问题。

**精确解法（按此改，二选一，推荐 A）**：
- **方案 A（首选，verify 层）**：把该测试改为**诚实 prove + verify 端篡改**：
  在 `Tamper` 枚举加 `BadEventRow`，在 `vmrs_verify_impl` 的 tamper 分支
  （`:1215-1221` 区域）把排序流某个读/写行的 `s_val` inout 词翻转 → 断言
  `c_ok == false`（frontend 的 transcript 绑定拒）。这直接证明"排序流 inout 被绑定"。
  原 `vmrs_prove_forge_final` 钩子删除（或保留并改写为"prove 必然失败"的
  catch_unwind 次要证据，注释说明 prove 期拒绝语义）。
- **方案 B**：自洽伪造（final 行 + 前序写行同改 fv），断言 prove 失败
  （F3 激活时 ev_val 钉扎在 witness 填充期拒）——同样是 prove 期形态，纪律上次级。

### R3（收尾）
- 移除 `M11_F3_EV` 环境变量门控（`:727-741` 两处 `ev_pin_active_m4` 分支），
  **F3 事件钉扎与 init_vals 断言改为默认激活**（门控已无存在必要——阻塞根因已修）。
- 全量测试绿（默认构建，无环境变量）。
- `M11_REPORT.md` 标 v3：如实记录 R1 根因（io 第二参数语义混淆）、R2 修法、
  门控移除；F3 状态从 ◐ 改 ✅。
- `AUDIT_2026-09-08.md` 的 S1 状态由 Worker 在报告中建议、**Leader 最终标注**。

## 验收标准

1. 默认构建全量绿（无环境变量），含 F3 激活后的全部门钉扎约束。
2. `BadEventRow`（或方案 B）用例 verify 层拒绝成立；forge 相关代码按方案 A/B 清理。
3. `cargo test -p binius-zkvm-slice` 输出全绿；vm_ram_sort 模块无新警告。
4. 报告 v3 如实记录根因（包括"门控阻塞其实是 io 参数 bug"这一诊断更正）。

## 送审要求

更新 `zkvm-project/M11_REPORT.md`（标 v3），回复简短送审消息。
