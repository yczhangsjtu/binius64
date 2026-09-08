# 安全审查准备（M10 T4）：威胁模型 + soundness 用例索引

> 2026-09-08。供独立审查者按图索骥；各用例的测试名可直接 `cargo test --lib <name>` 复跑。

## 威胁模型一页

**目标陈述（M12 更新）**：证明"公开程序（哈希 H 绑定）在给定初始内存上的执行自洽，
RAM 读写一致，且公开输出（公开地址 out_addr 的 final 值）= 声明值"。验证者信任：
哈希函数（Sha256/StdHashSuite）与 Fiat-Shamir 挑战源；不信任 prover 的任何本地数据。
验证形态 = **预处理模型下的 succinct 在线验证**（KNOWN_BOUNDARIES #3）。

**信任假设**：① 程序哈希为公共输入（对照语义见 KNOWN_BOUNDARIES.md #2）；② 初始内存
镜像（默认模式电路钉零；ELF 模式声明性锚，见 #1）；③ 挑战由 transcript 挑战器派生
（双方同源重建）；④ **χ 挑战在 oracle 承诺之后采样**（M12-T1 的 soundness 关键序，
M12_REPORT 附录 A）。

**攻击面 → 防御 → soundness 用例索引**：

| # | 攻击面 | 防御机制 | 拒绝层 | 用例（测试名 / Tamper） |
|---|---|---|---|---|
| A1 | 篡改公开输出 final_out | frontend 电路 final_out 断言 + **final_unique 命中计数==1**（M12 M5，防 XOR 相消）+ transcript assert 绑定 | 电路 | `vm_ram_sort_soundness_bad_final_out` (BadFinalOut)；`vm_ram_sort_soundness_dup_final`（M5 PoC，native 断言拒） |
| A2 | 换程序执行（执行≠承诺表） | fetch 单全点 looker（claim e = inst-oracle relation 绑定）+ 表 claim oracle relation（M12-T1 重构） | logup (FracAdd GKR) | `vm_ram_sort_soundness_swap_program` (SwapProgram) |
| A3 | 篡改取指 claim | recv 的 e 与归约一致性（e 一物三用：消息/product claim/oracle relation） | logup | `vm_ram_sort_soundness_bad_fetch_claim` (BadFetchClaim) |
| A4 | 冒充程序哈希（公共输入对照） | inout 哈希词 vs expected 对照（声明性；见边界 #2） | 哈希对照 | `vm_ram_sort_soundness_bad_prog_hash` (BadProgHash) |
| B1 | 丢/多塞内存事件（多重集合失衡） | fracaddcheck 恒等式①（成对分数相消）+ r 点开口 oracle relation | fracaddcheck/logup | `vm_ram_sort_soundness_bad_root_den` (BadRootDen) |
| B2 | 读值篡改（r 点开口声明与承诺列不符） | 开口声明经 verify_oracle_relation 绑定 + den_check（M7 v2 形态：验证端数据源 = 绑定开口） | logup (oracle relation FRI) | `vm_ram_sort_soundness_bad_den_addr` / `bad_den_val` (BadDenAddr/Val) |
| B3 | witness 排序流与 oracle 列分裂（**witness↔oracle 绑定**） | **χ-dot 锚**（M12-T1 新机制）：χ 承诺后采样 + 电路 bmul 累加断言 + 同泛函 oracle relation | 电路+logup | `vm_ram_sort_soundness_bad_event_row`（prove 端 witness 篡改 → l_ok=false）；`bad_dot_claim`（篡改公开声明词 → c_ok+l_ok 拒） |
| B4 | 篡改公开 χ 词 / χ-dot 声明 | χ 预检（公开词 == transcript 挑战）+ 电路 dot 断言 | 预检+电路 | `vm_ram_sort_soundness_bad_chi` (BadChi) |
| C1 | 排序流良构破坏（非降/ts 严增/读一致/init 形状） | 电路内 assert_sortedness（**验证端透明检查已删除**——M12 后排序流 committed，正确性全由电路断言承担） | 电路 | c_ok（honest 测试断言） |
| C2 | 取指位置与执行 pc 脱节（M11 F2 的 committed-only 形态） | index claim 与 pc-oracle 在叶点 z 的 oracle relation（无条件激活，无门控） | logup+oracle relation | `vm_ram_sort_soundness_fetch_position` |
| D1 | 换算术结果（ALU/寄存器链） | 译码-执行电路断言 + 寄存器值/版本双链（M3） | 电路 | word_vm32 `soundness_tamper_alu_wr` / `_wr_ver` / `_x0_write` |
| D2 | 过期 RAM 读（vm32 引擎） | 三表 logup*（fetch/reg/ram wlog）+ 版本链电路化 | logup | word_vm32 `soundness_tamper_ld_val` / `soundness_fetch`（切片 25/26 共 5+5 例） |
| E1 | 公开词任意篡改（inout 层） | transcript assert 消息绑定 + 电路对输出/χ/声明词的断言 | transcript+电路 | BadFinalOut/BadDotClaim/BadChi 覆盖 |
| E2 | 终止截断（证明任意前缀） | 电路末周期指令 ecall 断言（M12-T3；vm32 侧 M11 F4 HALT 断言） | 电路 | honest 测试隐含（所有用例的 trace 均以 ecall 结束） |

**纪律**：全部 soundness 用例的断言落在 verify 层（`c_ok/l_ok/hash_ok/s_ok == false`），
panic 不作为独立证据（prover 数据坏例仅次要）；无 panic 形态单独成立。

## soundness 用例计数（按里程碑，M12 更新）

- `vm_ram_sort`（M12 终态）：Tamper 用例 9（BadFinalOut/RootDen/DenAddr/DenVal/FetchClaim/
  SwapProgram/ProgHash/DotClaim/Chi）+ prove 端 mutant 2（BadEventRow→verify 层拒；
  dup_final→native 断言拒，次要形态）+ fetch_position；诚实 4（honest/api+key复用/
  small_program/elf_e2e）+ scale 点 2（ignored）
- M5/M6 `word_vm32`：诚实 1 + soundness 5（D1/D2）+ per-inst native 对拍 ~160 断言
- M9 `word_vm32_m8b_isa_prove`：9 条新指令端到端 + native 对拍
- M12 `word_vm32 m12_tests`：M1 非标编码 NOP×2、M2 sh 对齐拒绝、M3 程序哈希绑定、lh e2e
- M7 `ram_sort`：9 例（verify 层 4，切片 27；本轮无改动）
- 全量：73 passed / 0 failed / 4 ignored（`tools/run_zkvm_tests.sh`）
- **已知 completeness 边界**：fib 形状诚实证明被拒（非 soundness；KNOWN_BOUNDARIES #15）
