# 安全审查准备（M10 T4）：威胁模型 + soundness 用例索引

> 2026-09-08。供独立审查者按图索骥；各用例的测试名可直接 `cargo test --lib <name>` 复跑。

## 威胁模型一页

**目标陈述**：证明"公开程序（哈希 H 绑定）在给定初始内存上的执行自洽，RAM 读写一致，
且公开输出（final_out）= 声明值"。验证者信任：哈希函数（Sha256/StdHashSuite）与
Fiat-Shamir 挑战源；不信任 prover 的任何本地数据。

**信任假设**：① 程序哈希为公共输入（对照语义见 KNOWN_BOUNDARIES.md #2）；② 初始内存
镜像公开（当前引擎固定全 0，见 #1）；③ 挑战由 transcript 挑战器派生（双方同源重建）。

**攻击面 → 防御 → soundness 用例索引**：

| # | 攻击面 | 防御机制 | 拒绝层 | 用例（测试名 / Tamper） |
|---|---|---|---|---|
| A1 | 篡改公开输出 final_out | frontend 电路 final_out 断言 + transcript assert 绑定 | 电路 | `vm_ram_sort_soundness_bad_final_out` (BadFinalOut) |
| A2 | 换程序执行（执行≠承诺表） | fetch indexed logup*（claims 公开、表 committed）+ 表 claim oracle relation | logup (FracAdd GKR) | `vm_ram_sort_soundness_swap_program` (SwapProgram) |
| A3 | 篡改取指 claim | 同上（looker claims 与归约一致性） | logup | `vm_ram_sort_soundness_bad_fetch_claim` (BadFetchClaim) |
| A4 | 冒充程序哈希（公共输入对照） | inout 哈希词 vs expected 对照（声明性；见边界 #2） | 哈希对照 | `vm_ram_sort_soundness_bad_prog_hash` (BadProgHash) |
| B1 | 丢/多塞内存事件（多重集合失衡） | fracaddcheck 恒等式①（成对分数相消）+ 开口重算对照 | fracaddcheck/logup | `vm_ram_sort_soundness_bad_root_den` (BadRootDen) |
| B2 | 读值篡改（oracle 列 vs 公开列） | leaf-claim 桥：den_check/4×oracle relation 用 inout 重算值 | logup (oracle relation FRI) | `vm_ram_sort_soundness_bad_den_addr` / `bad_den_val` (BadDenAddr/Val) |
| B3 | 篡改公开排序流列（witness↔oracle 分裂） | transcript assert 绑定 + 桥对照双重拒绝 | 电路+logup | `vm_ram_sort_soundness_bad_bridge_witness` (BadBridgeWitness) |
| C1 | 排序流良构破坏（非降/ts 严增/读一致/init 形状） | 电路内 assert_sortedness + 验证端 `sortedness_ok` 透明检查 | 电路+本地 | s_ok 标志（honest 测试断言） |
| D1 | 换算术结果（ALU/寄存器链） | 译码-执行电路断言 + 寄存器值/版本双链（M3） | 电路 | word_vm32 `soundness_tamper_alu_wr` / `_wr_ver` / `_x0_write` |
| D2 | 过期 RAM 读（vm32 引擎） | 三表 logup*（fetch/reg/ram wlog）+ 版本链电路化 | logup | word_vm32 `soundness_tamper_ld_val` / `soundness_fetch`（切片 25/26 共 5+5 例） |
| E1 | 公开列任意词篡改（inout 层） | transcript assert 消息绑定（Channel(InvalidAssert)） | transcript | 覆盖于 BadBridgeWitness/BadFinalOut |

**纪律**：全部 soundness 用例的断言落在 verify 层（`c_ok/l_ok/hash_ok/s_ok == false`），
panic 不作为独立证据（prover 数据坏例仅次要）；无 panic 形态单独成立。

## soundness 用例计数（按里程碑）

- M8-A/M8-B/M9 `vm_ram_sort`：verify 层 8 例（上表 A1-A4、B1-B3、+ scale 点诚实断言）
- M5/M6 `word_vm32`：诚实 1 + soundness 5（D1/D2）+ per-inst native 对拍 ~160 断言
- M9 `word_vm32_m8b_isa_prove`：9 条新指令端到端 + native 对拍
- M7 `ram_sort`：9 例（verify 层 4，切片 27；本轮无改动）
- 全量：61 passed / 0 failed / 4 ignored（`tools/run_zkvm_tests.sh`）
