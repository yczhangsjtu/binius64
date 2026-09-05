# 二进制域 zkVM 现状调研报告

**日期**：2026-08-31
**目标**：评估「在二进制域（BinarySpartan/Ligerito/Binius）+ 通用 RISC-V zkVM 上，降低每条指令证明成本」这一方向的现状与空白。
**背景**：BinarySpartan(ePrint 2026/1656) 是二进制域上的 Spartan for R1CS；主流 zkVM（RISC Zero/SP1/OpenVM/Ceno/Valida/Jolt/Aztec）全部跑在素域（BabyBear/Mersenne-31 等小素数域或 BN254）。项目目标是证明「通用计算」，需明确二进制域 zkVM 是否有人做、做到哪、gap 在哪。

---

## 1. 核心结论（TL;DR）

- **工程层已经存在二进制域 zkVM**，但都**极其早期、非 RISC-V、且无商业化稳定背书**。
- **学术层是空白**：没有「二进制域 + 标准 RISC-V 通用 zkVM」的论文或工程。这是真实的 research/engineering gap。
- **可复用地基**：Binius64 的 64-bit 词级约束（SVI）后端 + Diamond–Posen 的「二进制塔上的 Lasso lookup 适配」是最接近「拉平指令成本」的现成武器；PetraVM 提供二进制域自定义-ISA VM 架构可借鉴；flock(BinarySpartan) 提供 R1CS/自定义门/Ligerito union 证明层（已在我方 M1 实证）。
- **需要自己从零搭**：标准 RISC-V 执行轨迹算术化翻译层、RAM/内存证明、证明递归(IVC)、零知识性(witness 隐私)、bigint/ECDSA 预编译。

---

## 2. 已有关键工程全景

| 工程 | 域 | VM 还是电路 | ISA | 仓库/状态 | 成熟度 | bigint/ECDSA |
|---|---|---|---|---|---|---|
| **PetraVM** (Polygon×Irreducible) | 二进制塔域 | **通用 VM** | **自定义 ISA**（RISC-V 风格助记符**非二进制兼容**，加 B32_MUL/B128_ADD/B128_MUL/Groestl 原语） | `PetraProver/PetraVM` (Apache-2.0) | **早期 WIP**；仅 VROM、**无 RAM**（future work）；软件「many instructions supported」 | 经 B128 ADD/MUL 承载，未宣示 ECDSA |
| **Binius64** | 二进制塔域 | **电路/约束**（非 VM） | —（SVI 词级约束，原生 64-bit 字 AND/MUL，非域乘） | `binius-zk/binius64` (Apache/MIT) | **较成熟**，1804 commits，性能多个基准第一 | **有**：big-number 库、ethsign ECDSA、XMSS；宣称 CPU 比 SP1/R0 GPU(L40S) 快 ~5× |
| **原始 Binius V0** | 二进制塔域 | VM 框架(PLONKish/AIR)，曾为 Petra 后端 | — | `IrreducibleOSS/binius` | **已归档/sunset**（随 Binius64 发布） | — |
| **BinarySpartan** (2026/1656) | 二进制域(Ligerito PCS) | 电路/R1CS 后端（非 VM） | — | 学术预印本，无正式开源工程 | 研究原型（M4 Max：BLAKE3 547k/s） | — |
| **Ligerito** | 二进制塔域 | 仅 PCS | — | Bain 笔记，无完整开源工程 | 研究 | — |
| **Open-Binius** (Ingonyama) | 二进制塔域 | FPGA 硬件 RTL（zk 加速 IP） | — | `ingonyama-zk/open-binius` | 社区硬件 | — |

**重要风险**：Irreducible 已于 **2025-11-12 关停**（[reinventing-irreducible](https://www.irreducible.com/posts/reinventing-irreducible)）。Binius64/PetraVM 已交社区（binius-zk、PetraProver）续更。**核心二进制域工程无公司背书**，依赖的可持续性是重要评估项。

### ⚠️ 2026-09 重大更新（本会话新确认）
- **PetraVM 已于 2026-01-08 被 owner 归档（read-only）**。这是决定性变化：历史上唯一"Binius 之上的通用 VM"，现已停止维护。其 ISA 为自定义（RISC-V 助记符风格但**非二进制兼容**）、WIP、无 RAM，即便 fork 续写也不适合作现实 baseline。
- **RISC Zero × Binius 集成为"未落地合作"**：20025-01-16 双方宣布要把 Binius 集成进 RISC Zero 的 RISC-V VM，但截至 2026 未见到可用的集成产物；risc0 主推仍是素域 STARK（BabyBear-class）+ Groth16。**不可作为现成 baseline**。
- **Binius64 = 二元域证明/约束后端，不是 zkVM**：它"能证明任意计算"，但表达方式是**64-bit 词级 SVI 约束电路**（非虚拟机、非指令集）。它自带 sha512/fibonacci/ethsign-ECDSA/bigint 示例，性能远强于 flock（x86-mt：SHA-512 65536B 证明 557ms/187KiB；ECDSA 322KiB）。这使它成为**最成熟的现成二元域证明后端**。
- **结论：市场上不存在"现成、成熟、非归档、标准 RISC-V 指令集"的二元域 zkVM。** 这个 gap 正是本项目要填补的。可用的现成二元域地基只有一个成熟选择 = **Binius64**（作为证明后端 + 词级约束层）。
- **✔ 2026-09-01 已实测选定 Binius64 为 baseline**（详见 `designs/binius64-baseline-decision.md`）：
  - 本机 i5-12400F 端到端证明+验证均成功：blake3(5.66ms/27KiB,零IMUL)、sha256、ethsign-ECDSA(501ms/314KiB, 20,538 IMUL)。
  - 确认其 4 种词级约束(ZERO/AND/IMUL/BMUL) + shifted-value-index 免费移位，是"哈希/位运算零放大 + 乘法为原生3-4×AND约束"的正确实现路径。

---

## 3. 学术现状（补查确认）

**主流 zkVM 论文全部素数域**，无一二进制域：
- Jolt（Arun/Setty/Thaler, Eurocrypt'24）= Mersenne-31 + Lasso lookup
- Ceno（Zhang et al., J.Cryptology 2025）= Mersenne-31/BabyBear + GKR
- HyperNova/SuperNova（Kothapalli/Setty）= 素域可定制约束系统
- Valida（Thomas et al., Lita）= Mersenne-31 + 对数导数 lookup

**二进制域侧**：
- **Diamond & Posen, "Succinct Arguments over Towers of Binary Fields"**(Binius, ePrint 2023/1784 及 2024/504, cited 93)——本质是**多项式承诺/论证系统**，含 **「Lasso lookup 到二进制塔的适配」**（见 Springer 版摘要）。这是把「用 lookup 统一/拉平任意指令成本」落到二进制域上的**核心学术支点**，但论文本身**不是 VM**，不涉及指令集。
- **BinarySpartan**(2026/1656)——R1CS for 二进制域，调用了 Binius64 的 byte-lookup 加速外层 sumcheck。非 VM。

**学术 gap 明确**：**未发现任何在二进制域上算术化「标准 RISC-V / 通用 ISA 指令集」的学术论文**。二进制域证明后端（论证系统、PCS、R1CS、词级约束）已成熟，但「把这些装进一个通用 RISC-V VM 并分析/优化每条指令成本」的桥在学术上无人走通。

---

## 4. 专查：Zinc 与 flock 现状

- **Zinc**：搜索未确认 zkemail 存在一个活跃的「Zinc 二进制域/任意域 zkVM」仓库（zkemail 官方仓库以 circom/SP1/halo2 实现为主）。**Zinc 是否为二进制域 zkVM 无确凿证据**，不采信为既有地基（避免凭记忆断言）。
- **flock / BinarySpartan**：从我方已本地照代码确认——flock = **R1CS/门电路证明后端**（`GateType` trait + `CircuitBuilder` + `prove_fast_ligerito_union_circuit` + `verify_ligerito_union_circuit`），**无 CPU 执行 RISC-V/VM 语义代码**；BinarySpartan 论文通篇只讲 R1CS 上的哈希证明，无 VM 章节。二者都是**证明层**，不是 VM。

---

## 5. 对项目的直接启示（gap → 本项目定位）

**本项目要做的「二进制域 + 标准 RISC-V 通用 zkVM（降低每条指令成本）」当前无人占位。** 可开垦路径：

1. **ISA 空白**：所有二进制域 VM（Petra）用自定义 ISA；**二进制域 + 二进制兼容的标准 RISC-V（吃到 SP1/R0/OpenVM 的现成编译器/语言生态）是彻底空缺**。
2. **每指令成本均匀化的工具已有大半**：Binius64 的 SVI 词级约束 + Diamond-Posen 的二进制塔 Lasso 适配，是「把进位整数指令和位运算指令都拉平到相近每指令成本」的现成机制——正好命中本项目把 add/sub/mul/div/hash 成本都压低的目标。
3. **补齐清单（当前全缺，需自建或化用）**：
   - RISC-V 执行轨迹 → 二进制域约束的翻译层（自建核心）
   - RAM/内存证明（Petra 无，需自建）
   - 证明递归 / IVC（需引入折叠或递归验证）
   - 零知识性（Binius64 当前仅 SNARK 非 zk-SNARK）
   - bigint/ECDSA 预编译（Binius64 已有示例，可借）
4. **地基选择**：flock(BinarySpartan) 已在本机跑通 R1CS+自定义门+Ligerito union 证明 → 可作证明层；Binius64 作词级约束/speed 参考；PetraVM 作 VM 架构参考；Lasso-in-binary-tower 作查询论证参考。

---

## 6. 参考资料

- PetraVM: https://github.com/PetraProver/PetraVM · spec https://petraprover.github.io/PetraVM/specification.html
- Binius64: https://github.com/binius-zk/binius64 · https://www.binius.xyz/blueprint/
- 原始 Binius(归档): https://github.com/IrreducibleOSS/binius
- Irreducible 关停: https://www.irreducible.com/posts/reinventing-irreducible
- Diamond & Posen Binius: ePrint 2023/1784, 2024/504; Springer 版 https://dl.acm.org/doi/10.1007/978-3-031-91134-7_4
- BinarySpartan: ePrint 2026/1656
- OpenVM: https://github.com/openvm-org/openvm · Ceno: https://github.com/scroll-tech/ceno · Lita(原 Valida): https://github.com/lita-xyz
- Brookstone 二进制塔上的 Lasso 适配、SuperSpartan(Setty/Thaler/Wahby)