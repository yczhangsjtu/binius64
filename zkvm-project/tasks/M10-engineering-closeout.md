# M10 任务书：工程化收官（库 API + CI + 文档 + 安全审查准备）

> 里程碑：M10（路线图最后一个；`designs/binary-zkvm-full-roadmap.md` §4）
> 派发日期：2026-09-08 | 派发方：Leader Agent | 执行方：Worker Agent
> 前置：M1-M9 ✅（M8-B T3 工具链停项为外部依赖，不在本轮）。
> 本任务书含全部决策；每个 T 是 checkpoint，做不完停在最近 checkpoint 如实送审。
> **M10 不加新证明机制**——本轮是把已验证的东西变成可交付形态。

---

## 1. 目标

让外部调用者能 `prove(program, input) -> Proof` / `verify(proof, program_hash, io)`，
并让项目的验证证据可复现、可审查。

## 2. 任务分解

### T1：库 API 化
- 在 `vm32`（或新 facade 模块 `vm32::api`）上收敛出稳定入口：
  `prove(program_image, init_mem, input) -> Proof` 与
  `verify(proof, program_hash, expected_io) -> bool`。
- 内部细节（transcript 串联、oracle 通道、三表）不外泄；现有切片改为经 API 调用
  （行为不变，CircuitStat/测试逐数字一致——M6 的重构纪律）。
- 公开输入清单显式化：程序哈希、初始内存镜像（公开部分）、输出。文档注明
  verifier 线性读入 inout 的现状（succinctness 边界）。

### T2：CI 化与内存护栏
- 一个可复跑的脚本（如 `tools/run_zkvm_tests.sh` 或 crate scripts/）：
  全量测试（默认）+ 大测试串行段（N≥64 加 `--test-threads=1` + 内存预检提示）。
- 说明文档：debug/release 双 profile 的运行方式与预期耗时。

### T3：文档收官
- `architecture.md`：更新到 M1-M9 终态（证据链 28 切片、vm32 库、BaseFold 通道、
  thesis 定量结论）。
- `zkvm-project/README.md`：当前状态 = Phase 2 除工具链外完成。
- 已知边界与假设**汇总成一页**（放 README 或独立 KNOWN_BOUNDARIES.md）：
  init 镜像全 0 假设、sb/sh 双事件、地址语义两套（vm32 字索引 vs 字节）、
  fetch 哈希↔承诺 root 同源、verifier 线性 inout、is_zk=false（无 ZK）、
  固定展开、表/镜像的承诺方案现状。
- HANDOFF_M-A2.md / HANDOFF_M8A.md / HANDOFF_M8B.md 等交接文档的归档标注核对。

### T4：安全审查准备
- 威胁模型一页：信任假设（公开程序哈希、初始镜像、挑战源）、攻击面
  （witness 注入点、表篡改、换程序）、每类攻击对应的防御机制与 soundness 用例索引。
- soundness 用例汇总表：每个里程碑切片 × 每例 × 拒绝层（电路/logup/fracadd/哈希），
  供审查者按图索骥。

### T5（时间盒内尽力，可选）：电路构建成本
- `CircuitStat::collect` 惰性化或门数削减（M 族断言按需 gating）的**设计文档**
  （不强制实现）；N=128+ 的流式构建方案草拟。

## 3. 验收标准

1. 全量测试绿；经新 API 的端到端测试存在（prove→verify 走公共入口）。
2. API 重构后 CircuitStat/测试与重构前逐数字一致（报告给对照）。
3. CI 脚本实跑通过（报告贴输出）；文档三处更新；边界汇总页存在。
4. 威胁模型 + soundness 索引表完整。
5. 新代码零警告；verify 层 soundness 纪律不变。

## 4. 送审要求

完成后：`zkvm-project/M10_REPORT.md` + 简短送审消息（≤15 行）。
