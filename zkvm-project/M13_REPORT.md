# M13 报告：fib 形状 completeness 缺口定位与修复

> 里程碑：M13（专项排查）| 派发：2026-09-08 Leader Agent | 执行：Worker Agent
> 任务书：`tasks/M13-fib-completeness.md`；前置：M12_REPORT §3.9/§3.10

---

## 0. 结论（TL;DR）

**根因**：M12-T1 的 fetch 重构引入了一条**未按构造保证的协议不变量**——

> inst oracle 列的 pad 行值（ECALL）必须等于 fetch 表在 pad 槽位的值
> （`prog_table[pad_slot] == ECALL`，pad_slot = 2^mp − 1）。

该不变量当时只靠"镜像短于 2^mp ⇒ resize 以 ECALL 填充"**偶然成立**。fib 的 ELF 镜像
长于 2^mp（1036 词 > 128），且 `parse_elf32` 会把 ELF 的零间隙字节折叠进 img.text——
于是 `prog_table[127] = 0`（ELF 原始零）≠ ECALL，导致：

```
e_claim = Σ_j eq_rf(j)·prog_table[look_idx[j]]      （实例化时取到 0）
  ≠
⟨instOracle, eq_rf⟩ = Σ_j eq_rf(j)·instOracle[j]    （pad 行 = ECALL）
```

inst-oracle 的 relation claim 与其承诺列的真实内积差一个常数 → BaseFold batched finish
的 Phase A 终检 `assert_zero(sumcheck_reduced_eval − expected)` 失败 → `l_ok = false`
（frontend 段随后失配 → c_ok 亦 false）。

**修复**（一行，按构造成立）：`vmrs_prove_impl` 构造 fetch 表后显式
`prog_table[pad_slot] = ECALL`。fib 端到端 honest prove→verify **恢复全绿**
（M12 的弱化注释与 `let _ = v_right` 已删除，断言恢复）。

**测试**：全量 **73 passed / 0 failed / 4 ignored**；`QUICK=1` CI **ALL GREEN**；
改动文件零警告；上游 crate 无残留改动（临时调试已全部移除）。

---

## 1. 排查过程（按任务书四步）

### 1.1 最小复现（T 二分 → 排除 T 本身）

参数化 K-addi 程序（lui + K×addi + sw + ecall，T = K+3）在 K ∈ {30,60,61,62,63,64,100}
（T ∈ {34,64,65,66,67,68,104}）**全部通过**——排除"T=65 触发 2^6 边界"假设。
结论：触发条件在 **fib 的数据/结构**，不在形状尺寸。

### 1.2 定位：上游 finish 加临时 eprintln（已全部移除）

在 `crates/iop/src/basefold/channel.rs` 的 `verify_batch_zk_basefold` 与
`crates/iop-prover/src/basefold/channel.rs` 的 `prove_batch_zk_basefold` 分阶段插桩：

| 检查点 | 结果（fib） |
|---|---|
| `batch_relations_per_oracle` 的 λ | prove/verify **一致**（0x4ece…）✓ |
| 7 个 combined claim | prove/verify **逐值一致** ✓ |
| `sumcheck::batch_verify`（round polys） | **通过**（无 Err）✓ |
| alphas（recv） | 与 prover 发送一致（by construction）✓ |
| transparent 在点的求值：prover 缓冲区直算 vs verifier 闭包 | **逐 oracle 一致** ✓ |
| **`assert_zero(reduced − expected)`** | **失败** ← 唯一断点 |
| Phase B（组合 FRI） | 未到达 |

### 1.3 缩小到 oracle：claim vs 真实内积

对每个 oracle 在 prover 侧直算 `⟨π_i, T_i⟩` 与 queued claim 比较：

```
oracle#0 (prog) ok  #1 (addr) ok  #2 (val) ok  #3 (ts) ok  #4 (kind) ok
oracle#5 (inst) **CLAIM MISMATCH**   #6 (pc) ok
```

**inst-oracle 的 queued claim（e）≠ 其承诺列的真实内积。**

### 1.4 根因确认：逐行找 e 的两个数据源分歧

```
e        = Σ_j eq_rf(j)·prog_table[look_idx[j]]
⟨msg,eq⟩ = Σ_j eq_rf(j)·instOracle[j]
```

逐 j 对比：**全部 1983 个 pad 行（j ≥ 65）失配**——`instOracle[j] = ECALL(0x73)` 而
`prog_table[127] = 0x00`。

溯源：fib.elf 的**单个 PT_LOAD 段**（PF_X）同时覆盖 .text 与 .sdata（文件偏移跨度
0..0x1030 字节），`parse_elf32` 把这段的**零间隙字节**折叠进 init_words 并写入
img.text（词 11..1035 = ELF 原始零）。fib 镜像（1036 词）> 2^mp（128）⇒
`resize(2048, ECALL)` 不再触及槽 127 ⇒ pad 槽保持 ELF 零。
对照：bubble16 镜像 21 词 < 128 ⇒ resize 填 ECALL ⇒ 不变量偶然成立 ⇒ 通过。
（解释了为什么"同 L=11、同 7 oracle"下 bubble 绿 fib 红——纯粹是镜像长度的偶然。）

### 1.5 为什么 M12 的排查没有命中

- "维度/挑战/claim 失配已排除"的比对对象是 **logup 层的 claim 值**（ic/tclaim/r 开口/
  e 两侧一致 ✓——它们确实一致！）；问题不在两侧不一致，而在 **prover 自己的 claim 与
  自己的承诺列不一致**（sumcheck 不验证 claim，只归约它；终检才暴露）。
- "χ relation 组已排除"的零关系替换实验中 inst e-relation 仍在位 → 失配仍在 ✓ 与观察一致。

---

## 2. 修复

`crates/zkvm-slice/src/slices/vm_ram_sort.rs`（`vmrs_prove_impl`）：

```rust
// M13 根因修复：协议不变量「inst pad = ECALL = prog_table[pad_slot]」必须按构造成立。
let pad_slot = (1u64 << mp) - 1;
let prog_table = {
    let mut t = image_for_table;
    t.resize(l_pow2, ECALL);
    t[pad_slot as usize] = ECALL;
    t
};
```

- 一行赋值使不变量对**任意镜像长度**按构造成立；fetch 闭包从不读取未执行槽位，
  trace 不受影响；logup 的 looker index pad = pad_slot 与强制值自洽。
- soundness 无回退：pad 槽是协议指定的"着陆行"，其值进公共表承诺并被 e-relation/
  index-eval relation 覆盖（与 M12 的设计意图一致，现在真的成立了）。
- **上游 crate 零改动**：全部临时 eprintln/Debug bound 已还原
  （`crates/iop/src/basefold/{channel,compiler}.rs`、`crates/iop-prover/src/basefold/channel.rs`
  grep 无 M13 残留，编译零错误）。

## 3. 回归测试

- `vm_ram_sort_elf_bubble16_e2e`：fib 的 `v_right` 断言**解除弱化、恢复全绿**
  （M12_REPORT §3.9 的边界注释删除）。
- 全量：**73 passed / 0 failed / 4 ignored**（无回退）；`QUICK=1` CI **ALL GREEN**；
  改动模块零警告。
- KNOWN_BOUNDARIES #15 已由"未解缺口"改写为"✅ 已修复（M13）"（含根因摘要）。

## 4. 需复核重点

1. **修复的定位**：只强制 pad 槽一处。镜像内其它非执行槽（ELF 零/garbage）不被任何
   lookup 触及、不受约束——协议上无害（证明的是"执行的指令 == 承诺表对应槽"）。
   若 Leader 认为"表尾全 ECALL"应是更强的协议陈述，需要额外约束（成本 O(表长)），
   本轮未做。
2. **parse_elf32 的行为**：单 PT_LOAD 覆盖 `.text`→`.sdata` 间隙时，零间隙词进入
   img.text（并被 M13 的强制值部分覆盖——仅 pad 槽一处）。gap 词进入 init 镜像
   （ELF 模式 init 行）本身是"镜像即初始内存"语义的如实呈现，未改。
3. **上游零残留**：请复核 `crates/iop`、`crates/iop-prover` 与 M12 前一致
   （本轮只加过临时 eprintln/Debug bound，已全部手工还原并 grep 验证）。

## 5. 文件清单

| 文件 | 变更 |
|---|---|
| `crates/zkvm-slice/src/slices/vm_ram_sort.rs` | 修复（prog_table pad 槽强制 ECALL）+ fib 断言恢复 + 清理 |
| `crates/iop/src/basefold/{channel,compiler}.rs`、`crates/iop-prover/src/basefold/channel.rs` | 临时调试已**全部还原**（净改动为零） |
| `zkvm-project/KNOWN_BOUNDARIES.md` | #15 改写为已修复（根因摘要） |
| `zkvm-project/M13_REPORT.md` | 本报告 |
