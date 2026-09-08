# M10 返工任务：公开 API 签名去测试钩子

> 派发日期：2026-09-08 | 派发方：Leader Agent | 前置：M10 首次送审（其余全部通过）
> 任务书 `tasks/M10-engineering-closeout.md` 继续有效；本文件是唯一未达标项的增量返工。

## 问题

T1 的目标是"稳定公共入口、内部细节不外泄"，但当前公开签名挂着测试钩子：
- `vmrs_prove(n, word_overrides, program, bad_hash)` —— word_overrides/bad_hash 是测试用；
- `vmrs_verify(proof, tamper, expected_hash)` —— Tamper 枚举是 soundness 测试用。

外部调用者不应看到/误用这些参数。

## 返工要求

1. **公开签名**（lib.rs 导出的形态）：
   - `pub fn vmrs_prove(n: usize, program: Option<&[u64]>) -> VmRsProof`
   - `pub fn vmrs_verify(proof: &VmRsProof, expected_hash: Option<[u64; 4]>) -> VmRsVerifyOut`
2. 测试钩子内收：带 tamper/overrides 的变体改为 `#[cfg(test)]`（或 `pub(crate)` + cfg(test)
   调用点）；`Tamper` 枚举本身可留在模块内但不出现在公开签名。
3. 现有 9 项 vm_ram_sort 测试行为不变（测试内部可改用 cfg(test) 变体）；
   `run_vmrs` 兼容包装若仅测试使用则一并收进 cfg(test)。
4. 全量测试 61 绿不回退；`QUICK=1 bash tools/run_zkvm_tests.sh` 仍 ALL GREEN。
5. 顺带：BENCHMARKS.md 补一行 proof 体积（~557KB @N=16，注明口径）。
6. M10_REPORT.md 标 v2 记录本返工。

## 送审要求

更新 `zkvm-project/M10_REPORT.md`（标 v2），回复简短送审消息。
