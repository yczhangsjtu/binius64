# M7 返工任务：soundness 改 verify 层 + den_check 去 native 化 + 报告更正

> 派发日期：2026-09-08 | 派发方：Leader Agent | 前置：M7 首次送审（ram_sort.rs，切片 27）
> 任务书 `tasks/M7-scalable-ram.md` 继续有效。核心成果（fracaddcheck 多重集合、K 无关缩放）
> 已验收通过；本文件是针对三处纪律性问题的增量返工规范。

## 验收发现（附代码证据）

### F1 soundness 拒绝形态为 prover 侧 panic（违反任务书 §6.4）
`ram_sort.rs:530-571`：4 例 soundness 全部经 `catch_unwind` 捕获 panic 判定
（witness 填充断言 / 根分子断言 `:384`）。项目纪律（ACCEPTANCE_BASIS、M2 任务书 §4.2）：
soundness 必须是 **verify 层拒绝**（verify 返回 Err/false），panic 只能算"证明构造失败"
的辅助证据，不能单独成立。

**修法（可行，不大）**：每例增加 verify 层形态——**诚实 prove + 验证端篡改**：
- 例 1（过期读/电路层）：诚实 prove 后篡改 `final_out` public inout → `c_ok == false`。
- 例 2-4（多重集合/logup 层）：诚实 prove 后，验证端用**篡改过的列副本**重建 den 组合
  （如把某行 val 换成旧值/删一行的指纹贡献）→ `l_ok == false`。
  现有 `rejected()` 辅助函数保留 panic 形态作为次要证据，但断言必须落在 verify 层。

### F2 den_check 从 native case 重建列（naive crutch 比报告披露的更深）
`ram_sort.rs:454`：验证端 `build_cols(&case, nrows)` 直接读 native case 算
`a_eval/v_eval/t_eval/k_eval`。但 `:403-410` 的 `addr_r/val_r/ts_r/kind_r` 已经过
`verify_oracle_relation` 与 committed 列绑定且在同一作用域——**den_check 直接改用这四个
开口值即可彻底摆脱 native 重建**（`:473-477` 一行表达式换数据源）。改完后 naive 通道的
crutch 只剩"承诺非密码学强度"（强通道迁移属 M8，报告中如此表述即可）。

### F3 报告更正：路线 B 的表述
M7_REPORT §r2 把路线 B（Twist 忠实翻译）写成"Waksman 置换网络"——**Twist 不是
排序/置换论证**（它是 one-hot ra + committed inc + Val 链 + LT 的 sumcheck 族），其真正的
阻塞点是 degree-3 自定义复合式无现成 evaluator（设计详案 §3.2）。选型结论（路线 A 胜）
不变，但 §r2 需重写为准确表述。

## 验收标准（增量）

1. 全量测试绿；4 例 soundness 各有 **verify 层**断言（l_ok==false / c_ok==false，
   非仅 panic）。
2. den_check 不再出现 `build_cols`（验证端数据源 = oracle 绑定的开口值）。
3. M7_REPORT.md 标 v2：§r2 重写、拒绝形态表更新、naive 通道边界表述精确化。
4. 文档数字/计数如需同步则同步。

## 送审要求

更新 `zkvm-project/M7_REPORT.md`（标 v2 + 返工说明），回复简短送审消息。
