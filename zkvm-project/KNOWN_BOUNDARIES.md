# 已知边界与假设（汇总页，M10 T3）

> 权威快照：2026-09-08（M12 收官后更新）。各详证见对应 M*_REPORT.md。

## 证明语义边界

1. **init 镜像锚定（M8-C 解除全 0 假设；M12 重构）**：默认模式（`vmrs_prove`）电路内
   **直接断言 init 行 val == 0**（in-circuit 钉零）。ELF 模式（`vmrs_prove_with_init`）
   接受非零初始镜像：init 行 val 经 d 侧 init 行钉扎 + 恒等式① + χ-dot 链绑定到 oracle；
   **init_hash（Sha256）为声明性外部锚，验证内不校验**（与调用方/ELF 加载结果比对的
   公共输入锚模式，同 prog_hash 的对照层级）。完全 in-proof 锚定需电路内哈希或
   O(n_touch) 公开 init 列——超出 M12 范围，如需更高保证转后续。
2. **fetch 哈希↔承诺 root 同源**：公开程序哈希（Sha256 of 镜像列，inout 词）与 BaseFold
   承诺 root 的等式由"同数据+同确定性套件"保证（诚实路径）；对抗强制需验证端读承诺 root
   对照——上游 `BaseFoldVerifierChannel.oracle_commitments` 私有，建议加只读 getter。
   "执行的==承诺表的"绑定本身是强制的（fetch logup + oracle relation）。
3. **verifier = 预处理模型下的 succinct 在线验证（M12-T1/T2）**：公开 inout **24 词恒定**
   （程序哈希 + init 哈希 + 输出 + 输出地址 + χ 挑战 + 6 个 χ-dot 声明；N=16/32/64 实测
   同值）；逐周期列全部 committed-only（7×BaseFold oracle）；在线验证零电路重建
   （`vmrs_verifier_setup` 一次性预处理 / `vmrs_verify_online`）。**非无条件 succinct**：
   预处理 O(T)（电路构建 ~20s @N=64）、proof 体积仍随 T 线性（~590KB@N=16 → 1.1MB@N=64）、
   无 ZK——proof 亚线性化（递归/聚合）与真·无预处理 succinct 属 Phase 3。
4. **is_zk = false**：BaseFold 无 ZK 掩码（`OracleSpec::is_zk` 现成未开）。witness 对
   verifier 是隐藏的，但 transcript 不抗统计泄漏模型。
5. **固定展开/参数化程序**：程序镜像手写（`prog_image(slot, n)`），无动态循环界；bubblesort
   O(N²) 使 N=1024 不可达（外推 ≥5×10⁹ 门）。真实编译程序 = M8-B T3（待工具链）。
6. **fetch 论证覆盖面（M12 更新）**：取指内容 = 单全点 looker（claim e 经 inst-oracle
   relation 绑定）；取指位置 = index claim 与 pc-oracle 在叶点 z 的 oracle relation
   （M11 F2 的 committed-only 等价物，无条件激活，无环境变量门控）。"程序哈希↔承诺 root"
   的显式对照仍受边界 2 限制。vm32 引擎的程序绑定 = committed fetch 表 + 声明性哈希
   （`M5Run.prog_hash`，M12-T3 M3）。

## ISA / 执行边界

7. **地址语义两套**：vm32 引擎 lw/sw =「地址即字索引」（M5 历史，imm 步长 1）；
   lb/lbu/lh/lhu/sb/sh = RISC-V 字节地址（`>>2` 取字索引 `&3` 取偏移）。两套在 vm32 内
   并存且注释声明；**tracer（M8-B T3，待工具链）落地时统一为字节地址**。
8. **sb/sh 双事件**：读旧字（ver=v）+ 写新字（ver=v+1）——排序论证 val_cons 覆盖；
   merged 值电路内计算（旧字经 RAM 论证钉住）。已端到端（M9 T2）。
9. **mulh/mulhsu/mulhu 未实现**（高位积论证未做）；ecall 仅作 halt。
10. **RAM 版本链已删**（M8-A）：读语义全由排序式内存论证承担；寄存器堆保留 32 计数器
    电路化版本链（K=32 电路化最优，设计决策 D3）。

## 工程边界

11. **内存规模**：~650 B/门（电路构建+prove 峰值）。N=64（13.7M 门）峰值 8.88GB——
    大测试须串行（`tools/run_zkvm_tests.sh` 已护栏）；N=128+ 需流式构建（未做，见
    `designs/circuit-build-cost-design.md`）。
12. **构建环境**：rustup 工具链 1.97.1（系统 cargo 1.75 不支持 edition2024）；
    `CARGO_BUILD_JOBS=4` 防 OOM；dev/release 双 profile 数字见 BENCHMARKS.md。
13. **M8-B T3（riscv32 工具链）已解除**（M8-C）：riscv64-unknown-elf-gcc 13.2.0 +
    `vm32/elf.rs`；非 4 对齐 vaddr 段**显式拒绝**（M12-T3；折叠错位风险的边界声明形态）。
14. **统一 oracle 长度 L = max(l, mp, L_FLOOR=11)**（M12）：batched opening 在过小域上
    触及 GaoMateer 基底边界（上游）；pad 行零值/ecall，成本可忽略。
15. **fib 形状 completeness 缺口——✅ 已修复（M13）**：根因 = fetch 表 pad 槽协议不变量
    「prog_table[pad_slot] == ECALL」未被构造性保证——注入镜像长于 2^mp 时（fib.elf 的
    单 PT_LOAD 覆盖 .text→.sdata 间隙，img.text 含零填充词），槽内为 ELF 原始零而非
    ECALL，e-relation（instOracle pad == 表 pad）失配 → finish 的 Phase A 终检拒绝。
    修复：`vmrs_prove_impl` 构造表后显式 `prog_table[pad_slot] = ECALL`（一行，按构造成立）。
    fib 端到端 honest prove→verify 恢复（`vm_ram_sort_elf_bubble16_e2e` 内 v_right 断言
    解除弱化）。排查全程见 M13_REPORT.md。
16. **四标志不再逐层隔离（M12）**：transcript 为单流且 frontend 段在最后——l 层篡改
    （finish 失败）使 frontend 段失配，c_ok 可能同为 false。任何标志 false 即拒绝
    （soundness 不受影响），仅诊断隔离性下降。
