# 已知边界与假设（汇总页，M10 T3）

> 权威快照：2026-09-08（M9 收官后终态）。各详证见对应 M*_REPORT.md。

## 证明语义边界

1. **init 镜像全 0 假设**：vm_ram_sort 的初始内存 = 全 0（init 记录 val==0 电路断言 +
   排序流 init 行）。非零初始镜像（如 ELF 数据段）需把 init 值纳入公共输入（T3 工具链的
   ELF loader 一并处理），当前引擎不接受。
2. **fetch 哈希↔承诺 root 同源**：公开程序哈希（Sha256 of 镜像列，inout 词）与 BaseFold
   承诺 root 的等式由"同数据+同确定性套件"保证（诚实路径）；对抗强制需验证端读承诺 root
   对照——上游 `BaseFoldVerifierChannel.oracle_commitments` 私有，建议加只读 getter。
   "执行的==承诺表的"绑定本身是强制的（fetch logup + oracle relation）。
3. **verifier 线性读入 inout**：公开 inout 含 inst/pc 逐周期词 + 排序流 8 列（~18.5k 词
   @N=16）。verifier 时间/空间与 T 线性——非 succinct 验证。这是当前 frontend 事件钉扎
   结构的形态（M8-B T1 桥的前提），succinct 化需要把事件列改为 committed-only + leaf-claim
   逐元素开口（协议扩展，未做）。
4. **is_zk = false**：BaseFold 无 ZK 掩码（`OracleSpec::is_zk` 现成未开）。witness 对
   verifier 是隐藏的，但 transcript 不抗统计泄漏模型。
5. **固定展开/参数化程序**：程序镜像手写（`prog_image(slot, n)`），无动态循环界；bubblesort
   O(N²) 使 N=1024 不可达（外推 ≥5×10⁹ 门）。真实编译程序 = M8-B T3（待工具链）。
6. **fetch 论证覆盖面**：取指 claim 与承诺表绑定（M8-B T0）；指令字与镜像一致已证明；
   但"程序哈希↔承诺 root"的显式对照受边界 2 限制。

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
13. **M8-B T3（riscv32 工具链）停项**：本机无工具链，外部依赖；方案见 HANDOFF_M8B.md。
