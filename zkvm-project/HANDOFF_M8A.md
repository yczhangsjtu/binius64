# M8-A Handoff: VM × 可扩展 RAM 论证整合 + BaseFold 强承诺通道

> **【已完成归档 2026-09-08】** 本交接的全部断点（T1 诚实单测 + Tamper 4 例 + 报告 + 文档）已在
> 接手会话完成，权威状态见 `M8_REPORT.md`。接手时额外修复：① fix_vmrs25/26 参数化残留
> （data 数组/写循环上界，fix 脚本 str.replace 静默失败）；② 程序镜像段 1 步长 bug② 修复未落盘
> （`base + 4*j` → `base + j`，以 sim_run6 验证语义为准）；③ witness 填充约定与无条件 pinning
> 断言对齐（非访存周期填 native 复算值）；④ PAD 组不推 init 行（ts=0 与周期 0 占位事件冲突）；
> ⑤ run_vmrs 的 n 参数化缺口（fetch 闭包捕获 n）。环境注意：必须用 rustup 工具链
> （`~/.cargo/bin` 前置 PATH；系统 cargo 1.75 不支持 edition2024）。

> 交接日期：2026-09-08；接手会话可直接接续（会话上下文压缩后可凭本文档+文件恢复）。
> 送审报告 `M8_REPORT.md` 尚未产出——本文档是**进行中状态**交接。

## 一句话目标
切片 28 `vm_ram_sort`：真实状态机 VM（vm32 语义，RV32I 子集）× M7 排序式内存论证（替换 O(K·T) 版本链，K=2^16 字）× BaseFold 强承诺通道；bubblesort 端到端 prove→verify + soundness 4 例。

## 当前状态（T0-T4 各点）
- **T0（ram_sort 迁移 BaseFold）✅ 完成（checkpoint 达成）**：`slices/ram_sort.rs` 已从 NaiveProverChannel 换 `BaseFoldProverChannel`（`ProverMerkleTranscriptChannel` + `prover_compiler.create_channel`），verifier 换 `BaseFoldVerifierChannel`；`chan.finish()` 批量开口。9/9 绿（诚实 20k gates/0.2s + verify 层 4 例 + prover 数据 4 例），**无 naive 残留**（验收标准 2 的一半已满足）。API 差异点见下。
- **T1（切片 28）进行中，代码已写、语义已验证、单测最后一步未跑**：
  - `slices/vm_ram_sort.rs`（~860 行）已含：RV32I 编码器、bubblesort 程序镜像（`prog_image(slot, n)`，N 参数化）、`run_program_big`（K=2^16 字寻址 RAM；语义 = vm32::interp 复制放大）、`event_rows/build_sorted`、`build_circuit_vmrs`（执行 wire 全 witness + **RAM 版本链已删除** + 事件 pinning 断言 + 恒等式②词级断言 + final_out inout）、`run_vmrs(n, tamper)` 主流程（BaseFold 4 列 committed → ρ/c → fracaddcheck → 4×relation → finish；verifier 侧 Tamper 4 例）。
  - **程序镜像 3 个 bug 已修并经 python 模拟验证**（`/tmp/sim_run*.py`）：①内层 j 循环死循环（标签指向 `lui x14` 重置点）→ 标签移到检查点；②字节步长（addr=BASE+4j）→ 字寻址步 1（mem 是 u32 数组）；③外层上界被内层 `sub` 污染 → `l_out` 指回 `lui x15` 重算点。验证：**N=16 sorted=True、27343 周期**；Rust 执行器 N=64 26k 周期 halt（镜像正确）。
  - **当前断点（最后一步）**：N=64 诚实测试曾 SIGKILL（电路过大/OOM）→ 已把 `N` 常量改 16 + `run_vmrs` 参数化 `n`；脚本 `fix_vmrs25/26.py` 已改文件但**尚未编译验证**。下一步：`cargo build -p binius-zkvm-slice` → 跑 `vm_ram_sort_honest`。
- **T2（恒等式②强绑定）未启，方案已定**：跨行比较（16-bit 非降/ts 严增）在 quadratic mlecheck（逐行独立）内不可表达（缺位分解+跨行借位）→ 按任务书 §2.3 降级授权采 **intmul phase5 模式的电路 witness 列方案**（排序流列 = 前端 witness + 同值 committed oracle；绑定 = 恒等式①归约链 relation 开口 + 电路约束；逐元素 leaf-claim 桥列为 M8-B/边界）。
- **T3（verify 层 soundness）测试已写好未跑**：`Tamper::{BadFinalOut, BadRootDen, BadDenAddr, BadDenVal}`（M7 v2 纪律，全部断言落 verify 层，无 panic 单独成立）。
- **T4（报告+文档四处）未启**。

## 阻塞/风险
1. **T1 诚实单测最后一步未验证**（断点如上述）。若 N=16 仍 OOM/超时 → 降 N=8（任务书 §2.5 授权），报告记录。
2. **已记录知情边界（送入报告）**：
   - fetch 论证（程序固定性 logup）本切片省略——程序语义由执行约束+内存论证+输出断言承担；fetch 表论证留 M9。
   - 排序**完整正确性**靠 native 对拍（测试断言 sorted_ok + 电路输出断言）；证明的是"执行自洽 + RAM 读写一致 + OUT_ADDR 终值 = 声明值"。
   - 排序流 witness ↔ committed oracle 逐元素强绑定需要一个 leaf-claim 桥（intmul phase5 式），诚实路径同源；报告如实标注。
   - 1024 字排序不可达（预计 ≥5×10⁹ 门）；主测 N=16，报告给 N=16/32 缩放点与 M7 孤立切片对照。
3. 任务书验收 4"排序程序端到端"留缩写（记录规模）。

## 已确认的架构事实与 API（接手者快查）
- **BaseFold 通道组装（prover 侧）**：`BinaryMerkleTreeScheme::<LF, StdHashSuite>::new()` → `BaseFoldVerifierCompiler::new(&merkle_scheme, specs, log_inv_rate, calc_n_queries(100,1), &ConstantArityStrategy::new(arity))`，arity = `ConstantArityStrategy::with_optimal_arity::<LF,_>(&scheme, log_code_len).arity`（log_code_len = l+1）→ `BaseFoldProverCompiler::from_verifier_compiler(&vcomp, ntt)` → `ProverMerkleTranscriptChannel::<&mut ProverTranscript<StdChallenger>, StdChallenger, LF, StdHashSuite>::new(&mut pt)` → `pcomp.create_channel(merkle_chan, StdRng::from_seed([0u8;32]), GlobalAllocator)`（is_zk=false 时 RNG 不读，固定种子安全）→ `send_oracle`/`sample`/`prove_oracle_relation`/`finalize_oracle`/`finish()`。
- **NTT**：`NeighborsLastMultiThread::new(GaoMateerPreExpanded::<LF>::generate(log_code_len), 1)`（`binius_math::ntt`）。
- **verifier 侧**：`VerifierMerkleTranscriptChannel` + `BaseFoldVerifierChannel::new(merkle_v, &v_specs, verifier_compiler.fri_params())`（specs 提取为变量借 lifetime）；`verify_oracle_relation(oracle, Box::new({let rr=r.clone(); move |p: &[LF]| eq_ind(&rr,p)}), claim)`；最后 `vchan.finish()`。
- **fracaddcheck 恒等式①**：`FracAddCircuit::build(l, &alloc, Fraction::new(FieldBuffer::<LP,_>::from_values(&num), ...))` → 根分子 assert ZERO → `send_one(root_den)` → `frac.prove(FracAddEvalClaim{num_eval:ZERO, den_eval:root_den, point:vec![]}, &mut chan)`；verifier `fracaddcheck::verify::<LF,_>(l, claim, vchan)` + `den_check = c·Σeq + 绑定开口值 + (ONE−Σeq)`（pad 行 den=1 贡献勿漏）。
- **程序镜像陷阱（本会话 3 连踩）**：循环标签必须在状态更新之外；mem 字寻址（步 1 非 4）；跨循环复用寄存器（x15 上界）必须在每轮入口重算。
- **执行器**：`run_program_big` 的 guard 上限 150 万；`Cycle` 复用 `vm32::interp::Cycle`（字段 ramver 填 `[0usize; NRAM]` 占位）；事件行 ts = 周期号，无访问行 = (PAD_ADDR=0xffff, 0, kind=0)。
- **soundness 语义（M8 纪律）**：验证端 Tamper 4 例全部 `l_ok==false`/`c_ok==false`，无 panic 单独成立；prover 数据坏例（rejected by panic）只作次要证据。
- **OOM 防线**：N=64（26k 周期）电路被 SIGKILL → 主测 N=16（~2.7k 周期，Rust 执行器实测 halt）。

## 下一步（接手者执行序）
1. `cd ~/workspace/binius64 && export RUSTFLAGS="-C target-cpu=native" CARGO_BUILD_JOBS=4 && cargo build -p binius-zkvm-slice`（fix_vmrs25/26 已写入文件，检查编译）
2. `cargo test -p binius-zkvm-slice --lib vm_ram_sort::tests::vm_ram_sort_honest -- --nocapture`（N=16）；若 SIGKILL/OOM → N=8
3. Tamper 4 例（`vm_ram_sort::tests::vm_ram_sort_soundness_*`）
4. 全量回归 `cargo test -p binius-zkvm-slice`（预期 49+ 项：44 旧 + ram_sort 9 + vm_ram_sort 5）
5. `M8_REPORT.md`：T0 通道迁移记录（API 差异点：oracle=承诺非全系数、relation 延迟到 finish、specs 需静态）、绑定方案选择（§2.3 降级）、缩放/成本（对照 M7 孤立）、知情边界、文档四处更新（切片 28、lib.rs 导出、README/architecture/milestone-roadmap 的 M8-A 状态）
6. 简短送审消息（结论/文件/测试/复核重点）

## 关键文件
- `crates/zkvm-slice/src/slices/ram_sort.rs` — T0 完成（BaseFold 版，9 测试）
- `crates/zkvm-slice/src/slices/vm_ram_sort.rs` — T1 切片 28（主战场，断点在此）
- `crates/zkvm-slice/src/lib.rs` — 已挂载 `vm_ram_sort` + `pub use run_vmrs`
- `zkvm-project/tasks/M8-A-vm-ram-integration.md` — 任务书（T0-T4/验收 6 条）
- `zkvm-project/M7_REPORT.md` — §r2 迁移清单（本任务来源）
- `/tmp/sim_run*.py` — 程序语义模拟器（已证 sorted=True）
- `/home/yczhang/.hermes/cache/blocked-scripts/fix_vmrs*.py` — 迭代修复脚本（保留溯源）

## 构建命令与约束
```bash
cd ~/workspace/binius64
export RUSTFLAGS="-C target-cpu=native" CARGO_BUILD_JOBS=4
cargo test -p binius-zkvm-slice --lib        # 全量
```
只动 `crates/zkvm-slice/` + `zkvm-project/`；禁改上游 crates、禁 git 操作。

## 参考会话
- M7 送审（fracaddcheck 组装 + T0 绑定策略）：2026-09-07 会话（`M7_REPORT.md`）
- M8-A 实施（本会话，2026-09-08）：T0 完成 + T1 调试至断点（见本文件"当前状态"）