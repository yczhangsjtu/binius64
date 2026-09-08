# M10 送审报告 v2（工程化收官：库 API + CI + 文档 + 安全审查准备，2026-09-08）

> v2 返工（`tasks/M10-rework-api.md`，唯一未达标项）：公开 API 去测试钩子——
> `pub fn vmrs_prove(n, program)` / `pub fn vmrs_verify(proof, expected_hash)`；
> 带 word_overrides/bad_hash/tamper 的实现收为私有 `*_impl`，`run_vmrs` 兼容包装
> 收进 `#[cfg(test)]`，lib.rs 导出面 = `{vmrs_prove, vmrs_verify, VmRsProof, VmRsVerifyOut}`。
> 全量 61 绿不回退、CI `QUICK=1` 实跑 ALL GREEN、BENCHMARKS 补 proof 体积行。
> 其余章节内容与 v1 相同，API 相关表述以本节为准（v1 中 `vmrs_prove(n, word_overrides,
> program, bad_hash)` 等签名描述已被本返工取代）。

Workdir: `/home/yczhang/workspace/binius64`。范围：`crates/zkvm-slice/` + `zkvm-project/`；
未动上游、无 git 操作。任务书 `tasks/M10-engineering-closeout.md`。**未加任何新证明机制。**
全量回归 **61 passed / 0 failed / 4 ignored**（新增 1 项 API 端到端测试）。

## 结论

**T1-T4 全部完成，T5 设计文档交付**。库获得稳定公共入口
`vmrs_prove(program, …) -> Proof`（Proof = transcript bytes + 公开 inout + 元数据）与
`vmrs_verify(&Proof, tamper, expected_hash) -> 四层标志`——proof 以标准 bytes 形态跨进程
携带，verify 端从 bytes 重建 transcript 与约束系统，独立完成全部四层检查。

## T1：库 API 化 ✅（逐数字一致对照）

- `run_vmrs` 拆分为 `vmrs_prove`（native 执行 → 电路 witness → frontend prove →
  BaseFold/logup/fracadd → `pt.finalize()` 出 bytes）与 `vmrs_verify`（`VerifierTranscript::
  new(challenger, bytes)` 重建 → 电路/cs 重建 → frontend verify → fetch logup 归约 →
  fracadd verify → 5×oracle relation → finish）。
- **验证端自洽化**：eq 开口值由验证端从 `vfinal.point` + 公开 inout 独立重算（不再借
  prove 段状态）；BaseFold compiler/fri_params 验证端独立重建。
- `run_vmrs` 保留为兼容包装（prove+verify 串跑），**现有 9 项测试零改动、数字一致**
  （gates=905,168 @N=16 与 M9 报告逐字相同；soundness 8 例全绿——对照见
  `vm_ram_sort::tests` 运行输出）。
- 新增 API 端到端测试 `vm_ram_sort_api_end_to_end`：prove → bytes → verify 全绿 +
  **自定义程序镜像注入**（program 参数，fetch 表/哈希跟随）+ 哈希不符拒绝。
- 公开输入清单显式化（`VmRsProof` 文档注释）：程序哈希、初始镜像（当前全 0 常量）、
  输出词；verifier 线性读入 inout 的 succinctness 边界已注明。

## T2：CI 化与内存护栏 ✅

`tools/run_zkvm_tests.sh`（实跑输出见下）：build → quick 段（串行）→ 大测试段
（`--ignored --test-threads=1`，脚本头注明 N=64 峰值 ~9GB 与 QUICK=1 跳过护栏）；
支持 debug/release 双 profile。**实跑 `QUICK=1` 输出：`== ALL GREEN ==`（61 passed）。**

## T3：文档收官 ✅

- `KNOWN_BOUNDARIES.md`（新）：13 条边界/假设汇总一页——init 全 0、fetch 哈希同源、
  verifier 线性 inout、is_zk=false、固定展开、地址语义两套、sb/sh 双事件、内存规模、
  工具链环境等，每条带权威详证指针。
- `architecture.md`：Phase 2 终态行（证据链 28 切片、vm32 库、BaseFold、M8-B/M9/M10 增补）。
- `README.md`：当前状态 = Phase 2 除 riscv32 工具链外完成。
- HANDOFF 归档核对：HANDOFF_M8A.md 已有归档标注（M8-B 时）；本轮补 HANDOFF_M8B.md
  （执行归档 + T3 方案仍有效）与 HANDOFF_M-A2.md（历史归档）。

## T4：安全审查准备 ✅

`SECURITY_REVIEW_PREP.md`（新）：威胁模型一页（目标陈述/信任假设/攻击面→防御→用例
索引表 A1-A4 程序绑定、B1-B3 内存论证、C1 排序良构、D1-D2 执行链、E1 inout 层）+
按里程碑的 soundness 用例计数。每例附可复跑测试名与拒绝层。

## T5：电路构建成本设计 ✅（设计文档，未实现）

`designs/circuit-build-cost-design.md`：① `CircuitStat` 惰性化（微优化，省 1-3s@N=64）；
② 门数削减两案（M 族断言批量归约、排序流断言折叠——注明破坏 gates 基线纪律的代价）；
③ N=128+ 流式构建（分块+continuation 证明 / 上游 SoA 建议）。含推荐顺序。

## 验收对照

1. ✅ 全量绿；API 端到端测试存在且走公共入口（bytes 往返）。
2. ✅ 重构后逐数字一致（gates/CircuitStat/9 项测试不变，run_vmrs 兼容包装保证）。
3. ✅ CI 脚本实跑通过（QUICK=1 ALL GREEN；大测试段命令与护栏已给）；文档三处更新；
   边界汇总页存在。
4. ✅ 威胁模型 + soundness 索引表完整。
5. ✅ 新代码零警告（vm_ram_sort 模块 0 警告；全 crate 剩余警告均为历史代码既有项）；
   verify 层纪律不变。

## 需复核重点

1. **API 形态**：`vmrs_verify(&Proof, tamper, expected_hash)` 把 tamper 暴露在 verify
   签名（soundness 测试需要）；真实外部调用应固定 `Tamper::None`——是否要把坏例路径
   拆到 `#[cfg(test)]` 或独立函数（当前为最小改动保留）。
2. **逐数字一致的范围**：gates/stat 与 inout 布局一致；**证明体积**为新量（proof_bytes
   ~557KB @N=16，`[phase]` 日志可查）——后续可做 proof size 基准。
3. **HANDOFF_M-A2.md 归档标注**为最小侵入（头部引言），内容未改——是否符合归档规范。
