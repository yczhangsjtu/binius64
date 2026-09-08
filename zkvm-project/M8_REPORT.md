# M8-A 送审报告（VM × 可扩展 RAM 论证整合 + BaseFold 强承诺通道，2026-09-08）

Workdir: `/home/yczhang/workspace/binius64`。范围：`crates/zkvm-slice/` + `zkvm-project/`；
未动上游、无 git 操作。按任务书 `tasks/M8-A-vm-ram-integration.md` T0→T4 执行。

## 结论

**切片 28（`vm_ram_sort`）落地并全绿**：真实状态机 VM（vm32 语义 RV32I 子集）× M7 排序式
RAM 论证（K=2^16 字地址空间，O(K·T) 版本链删除）× BaseFold 强承诺通道。bubblesort 端到端
prove→verify：诚实 1 + verify 层 soundness 4 全部按预期拒绝；全量回归
**53 passed / 0 failed（+3 ignored：2 旧 + N=32 缩放点）**。T0 checkpoint（ram_sort 通道迁移）
先期完成（9/9 绿）。

- 主测 N=16：T=1801 周期、ts=1834、l=12、**gates=905,168**、honest prove **1.9s**（dev profile）。
- 缩放点 N=32：T=6973、gates=3,502,632、11.1s。**T×3.87 → gates×3.87，线性**（与 M7 结论一致）。

## T0：BaseFold 强通道迁移（checkpoint，先行完成）

`ram_sort.rs` 与 `vm_ram_sort.rs` 全部走 BaseFold 真实通道，**全 crate 零 `NaiveProverChannel` 残留**
（grep 证实）。实例化证据（两切片同型）：

- prover：`BaseFoldVerifierCompiler::new(&merkle_scheme, specs, log_inv_rate, queries, &arity_strategy)`
  → `BaseFoldProverCompiler::from_verifier_compiler(&vcomp, ntt)` →
  `ProverMerkleTranscriptChannel::new(&mut pt)` → `pcomp.create_channel(merkle_chan, StdRng::from_seed([0u8;32]), GlobalAllocator)`
  （`vm_ram_sort.rs:727-736`、`ram_sort.rs:382-393`）；4 列 `send_oracle`（`vm_ram_sort.rs:737-740`）。
- verifier：`VerifierMerkleTranscriptChannel` + `BaseFoldVerifierChannel::new(merkle_v, &v_specs, fri_params)`
  + `verify_oracle_relation` ×4 + `vchan.finish()`（`vm_ram_sort.rs:793-833`）。

**API 差异点**（vs M7 naive 通道，后续迁移者快查）：
1. oracle = 承诺（Merkle 根），verifier 不可见全系数——naive 时代"验证端重建列"的路径物理消失；
2. `prove_oracle_relation` 的 FRI 开口**批量延迟到 `chan.finish()`**（verifier 对应 `vchan.finish()`）；
3. oracle specs（`OracleSpec{log_msg_len, is_zk}`）须以变量持有供 verifier 侧借用（lifetime）；
4. `is_zk=false` 时 `create_channel` 的 RNG 不被读取，固定种子安全；log_code_len = l + log_inv_rate；
5. arity 用 `ConstantArityStrategy::with_optimal_arity::<LF,_>(&scheme, log_code_len)` 求最优；
6. NTT：`NeighborsLastMultiThread::new(GaoMateerPreExpanded::<LF>::generate(log_code_len), 1)`。

## T1：VM 整合（切片 28）

- **执行核心**：`run_program_big`（`vm_ram_sort.rs:229`）——vm32::interp 语义复制放大到
  K=2^16 字寻址 RAM（mem 为 u32 数组，地址 = 数组索引，guard 150 万周期）；程序镜像
  `prog_image(slot, n)` 手写 RV32I 编码 bubblesort（段 1 伪随机数据写入 + 段 2 冒泡 + halt），
  n 参数化。镜像语义经 python 模拟器对拍验证（`/tmp/sim_run*.py`，sorted=True）后落地；
  修复三处镜像 bug（循环标签须在状态更新之外 / 字寻址步长 1 / 外层上界每轮重算）。
- **RAM 版本链删除点**：`vm_ram_sort.rs:568`「★ RAM 版本链已删除」——`ld_val` 为自由 witness，
  不钉任何电路内值链/版本链，读语义由内存论证承担；地址计算仅 K-1 掩码常量
  （`:525`，K 不进入约束数）。
- **事件列—执行钉扎**（R1 语义，`:585-596`）：每周期无条件断言
  `rd1/rd2/wr_* == 译码复算`、`ld_addr == (rs1+imm_i)&0xffff`、`st_addr == select(is_store, rs1+imm_s, rs1+imm_i)&0xffff`、
  `st_val == rs2v`、`is_load/is_store` 布尔；事件行 (addr, ts=周期, val, kind) 是电路 witness 的
  派生结果。**witness 填充约定**：pinning 无条件 ⇒ 非访存周期也必须填 native 复算值
  （rd 字段、mem_addr、rs2v）——首次跑通时 3945 个 populate 失败全部源于此约定错位，已修。
- **恒等式①**：fracaddcheck 多重集合等式（`FracAddCircuit::build` `:755`，根分子 assert ZERO
  `:760`，`send_one(root_den)` `:759`，`frac.prove(...)` `:762`；指纹 f = addr+ρ·val+ρ²·ts+ρ³·kind）。
  列布局 = 排序流 ‖ 事件侧（init+事件+final），pad 行 num=0/den=1；verifier
  `den_check = c·Σeq + 开口绑定值 + (1−Σeq)`（`:810-827`）。
- **恒等式②**：`assert_sortedness`（`:442-465`，调用 `:598`）——组首 init 形状、16-bit 非降、
  同地址 ts 严增、同地址读值一致、组切换必须是 init；外加三件套 final_out inout（`:606`，
  OUT_ADDR=BASE 的 final 值 = 排序后最小元素，公开输出一词）。
- **排序流构造**：每触及地址 init 首 + 事件按 ts + final 尾；PAD（占位）组不推 init 行——
  占位事件行本身 kind=0/val=0 具备 init 形状，推 ts=0 的 init 行会与周期 0 占位事件违反
  ts 严增（本会话发现并修复）。

## T2：恒等式②绑定——采用任务书 §2.3 降级授权方案

**选择：intmul phase5 模式的电路 witness 列方案**。排序流列 = 前端电路 private witness
（`s_addr/s_ts/s_val/s_kind`，承受恒等式②全部词级断言）+ 同值 committed oracle（4 列
send_oracle，经恒等式①归约链 + `prove/verify_oracle_relation` 绑定）。

**为什么不是推荐路径**（committed 列上的 quadratic mlecheck）：非降比较需要 16-bit 位分解 +
跨行借位链，ts 严增同理——都是**跨行**关系；quadratic mlecheck 的被检函数逐行独立（行内
二次式），无法表达跨行约束。若强行单行化需要把"相邻行对"编码进行本身（列翻倍 + 布尔
选择器），等价于把恒等式②整体搬回执行电路，与现状同价但复杂度更高。

**间隙（如实声明）**：witness 列 ↔ committed oracle 的**逐元素**强绑定需要一个 leaf-claim 桥
（intmul phase5 式逐叶子相等检查）；本切片两副本在 prover 本地同源（诚实路径成立，
对抗路径靠恒等式①对 oracle 侧的多重集合约束 + 电路对 witness 侧的执行钉扎分别封锁）。
leaf-claim 桥列入 M8-B/后续。

## 成本与缩放（dev profile，RUSTFLAGS=-C target-cpu=native）

| 场景 | T（周期） | ts（排序流行） | l | gates（ZERO/AND/BMUL） | honest prove |
|---|---|---|---|---|---|
| **N=16（主测）** | 1,801 | 1,834 | 12 | **905,168**（32,810/274,143/279,279） | **1.9s** |
| **N=32（缩放点）** | 6,973 | 7,038 | 14 | **3,502,632**（126,716/1,060,767/1,081,163） | 11.1s |
| M7 孤立对照（合成事件，无执行电路） | 2^10 访存 | 1,156 | — | 81,880 | 0.2s |

- **T×3.87 → gates×3.87**：gates 随执行周期数线性（与 M7 的 T 线性结论同型）。
- **K=2^16 不进入约束**：地址仅以 K−1 掩码常量出现（与 M7"K 无关"结论一致）。
- **整合开销单列**：M7 孤立切片只有内存论证（82k gates @T=2^10 访存）；M8-A 整合后每
  **执行周期**一行（含 PAD 占位行），gates 由执行电路主导（译码+ALU+寄存器值链，~500 门/周期）。
  内存论证本身（恒等式①+②）与 M7 同型、随 ts 线性。
- **1024 字排序在本结构下不可达**：bubblesort O(N²) ⇒ N=1024 外推 ≥5×10⁹ 门（任务书 §2.5
  的降规模授权适用，报告记录规模 N=16/32）。

## Soundness（verify 层 4 例，M7 v2 纪律：无 panic 单独成立）

| 例 | 篡改点 | 层 | 结果 |
|---|---|---|---|
| 1 `BadFinalOut` | 验证端篡改公开输出 final_out | 电路 | **`c_ok == false`** ✓ |
| 2 `BadRootDen` | 篡改分数和声明 root_den | logup | **`l_ok == false`** ✓（c_ok 不受影响）|
| 3 `BadDenAddr` | 篡改 addr 开口值 | logup | **`l_ok == false`** ✓ |
| 4 `BadDenVal` | 篡改 val 开口值 | logup | **`l_ok == false`** ✓ |

覆盖任务书 T3 的四类语义：最终结果篡改（1）、过期读/排序配对错（3/4 开口层）、
丢/多塞事件（2 使多重集合声明失衡）。

## 已知边界（知情声明）

1. **fetch 论证省略**：程序固定性 logup 表（fetch 表）未做——程序语义由执行约束 + 内存
   论证 + 输出断言承担；指令字是 witness（钉扎断言保证译码自洽，不保证 == 镜像）。
   fetch 表论证留 M9。
2. **排序完整正确性靠 native 对拍**：测试断言 `sorted_ok`（native final_mem 非降检查）+
   电路断言 OUT_ADDR 终值 = 声明值；证明系统内证明的是"执行自洽 + RAM 读写一致 +
   输出一致"，**不是**"内存已排序"的全序断言。
3. **witness↔oracle 逐元素绑定需 leaf-claim 桥**（见 T2 间隙）；诚实路径同源。
4. **规模**：主测 N=16、缩放点 N=32（`#[ignore]` 测试，`--ignored` 运行）；N=64 电路在
   本机构建即 OOM（SIGKILL），1024 字不可达（见成本表）。
5. `is_zk = false`：BaseFold 无 ZK 掩码（现成能力，`OracleSpec::is_zk`，未开启）。
6. **环境**：构建必须用 rustup 工具链（`~/.cargo/bin` 前置 PATH；`rust-toolchain.toml`=1.97.1）。
   系统 `/usr/bin/cargo`（1.75）不支持 edition2024，会报 workspace manifest 解析错误。

## 验收对照（任务书 §4 六条）

1. ✅ `cargo test -p binius-zkvm-slice` 全绿：**53 passed / 0 failed**（+3 ignored）。
2. ✅ 强通道证据：BaseFold 实例化行见 T0 节；零 naive 残留（grep 证实，两切片均迁移）。
3. ✅ 整合证据：版本链删除点 `vm_ram_sort.rs:568`；钉扎约束 `:585-596`；恒等式① `:755-762`；
   恒等式②绑定方案 = §2.3 降级（witness 列方案，`assert_sortedness` `:442-465`）。
4. ✅ 端到端（记录规模 N=16/32）：native 对拍（sorted_ok）+ 证明闭环（c_ok/l_ok）+ 三件套
   （init 形状/final 记录/final_out 公开 inout）。
5. ✅ soundness 4 例全部 verify 层拒绝，无 panic 形态单独成立。
6. ✅ 成本/缩放数据与诚实边界见上。
