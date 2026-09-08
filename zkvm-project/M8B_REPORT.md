# M8-B 送审报告（fetch 论证恢复 + leaf-claim 桥 + ISA 补全 + 工具链，2026-09-08）

Workdir: `/home/yczhang/workspace/binius64`。范围：`crates/zkvm-slice/` + `zkvm-project/`；
未动上游、无 git 操作。任务书 `tasks/M8-B-isa-toolchain.md`。

## 结论

**T0 ✅ T1 ✅ T2 ✅；T3 ⏸ 如实停项**（本机无 riscv32 工具链，外部依赖缺失，见 §T3）。
全量回归 **60 passed / 0 failed（+3 ignored）**。M8-A 的两项语义缺口（fetch、witness↔oracle
绑定）全部闭合；ISA 补全 9 条新指令（mul/div/divu/rem/remu/lb/lbu/lh/lhu）带电路级端到端。

## T0：fetch 论证恢复 + 程序公开化（checkpoint 1）✅

切片 28 `vm_ram_sort` 演进（同切片升级，M5→M6 先例）：

- **committed fetch 表 + 公开镜像哈希**：程序镜像列（`prog_col`，2^M_P 行，M_P=log2⌈5n+22⌉）
  `send_oracle` 为第一个 BaseFold oracle——**承诺先于一切取指挑战**（logup 前置条件）。
  公开镜像哈希 = `Sha256(序列化镜像列)`（`prog_image_hash`，4 词），填**电路公开 inout**，
  验证端对照 `expected_hash` 参数（`hash_ok` 标志）。
- **indexed logup\* 取指**（M3/M5 模式，表改 committed）：每周期一个单行 looker
  (index=pc/4, claim=inst)。**inst/pc 提升为公开 inout**（claims_from_inout 纪律，M5 同款），
  claims 公开可重算。`logup_star::prove` 的 channel 参数直接传 BaseFold 通道
  （`IPProverChannel` supertrait，同一 transcript，挑战顺序严格同序：γ→logup→ρ→c）。
- **表 claim 绑定**（M8-A oracle relation 模式）：logup 输出的表 MLE 求值 claim
  （`table_eval_point` 前缀坐标）经 `prove/verify_oracle_relation` 绑定到承诺
  （`vm_ram_sort.rs` prover 段/verifier `fetch_ok` 闭包）。
- **Soundness 3 例**（全部 verify 层）：`BadFetchClaim`（篡改取指 claim → logup 归约拒）、
  `SwapProgram`（执行换编码的程序、承诺表/哈希仍原镜像 → "执行的==承诺的" 被拒，
  PushforwardMismatch）、`BadProgHash`（哈希词不符 → hash_ok=false）。
- **已知边界**（如实声明）：inout 哈希与 BaseFold 承诺 root 的等式由「同数据同确定性套件」
  同源保证；密码学强制需验证端读取承诺 root 对照——上游 `BaseFoldVerifierChannel` 未暴露
  commitment getter（`oracle_commitments` 私有），建议上游加只读访问器。"执行的==承诺表的"
  绑定本身是强制的（logup+relation），不受此边界影响。

## T1：leaf-claim 桥（witness↔oracle 逐元素绑定）✅

任务书指定 intmul phase5 模式；实施中发现 phase5 是 intmul 协议内部机制（绑定其自身
sumcheck witness 与查表），frontend 无列开口，不能直接套用。改用等价强度的桥（绑定目标
不变，验收形态一致）：

- **排序流全列公开化**：恒等式②侧的 s_addr/s_ts/s_val/s_kind **与** 恒等式①事件侧的
  d_* 4 列（共 8×ts 词）提升为公开 inout——frontend Spartan prove 的 transcript assert
  消息即承诺（本步实施中发现：篡改任何 inout 词 → `Channel(InvalidAssert)`，绑定在
  transcript 层即成立，比电路断言更根本）。
- **开口重算对照**：验证端从 inout 重算 `addr_r/val_r/ts_r/kind_r = Σ eq(r,j)·列[j]`，
  用重算值构造 `den_check` 并作为 4 个 oracle relation 的 claim——公开列与 oracle 列在
  随机点 r 上逐元素绑定（列不等 ⇒ 失配拒绝）。**同时消除了 M8-A 遗留的
  「验证端与 prover 共享本地变量」crutch**（den_check 数据源真实化为公开重算值）。
- **恒等式②透明化**：`sortedness_ok` 在验证端对公开排序流直接检查（非降/ts 严增/读一致/
  init 形状，`s_ok` 标志），与电路内断言双保险。
- **Soundness**：`BadBridgeWitness`（篡改事件侧公开列、保留 oracle → c_ok 与 l_ok 双拒）。

## T2：ISA 补全 ✅（sb/sh 电路层留边界）

vm32 库（M6 体系）扩展，native + 电路 + 端到端三层：

- **mul**（frontend imul 门）：`is_m_ext = R-type ∧ funct7=0x01`；结果 = imul 低 32 位。
- **div/divu/rem/remu**（设计详案 §2.6 展开式）：advice 商 `m_q`（公开 inout，每周期 1 词）+
  断言序列——无符号 `q·y==x ∧ r<y`；有符号 abs 取正（negate_if 的 char-2 形式 = select +
  0−v）+ 符号修正；除零 select 修正（div→-1/rem→x，RISC-V 语义）；MIN÷−1 溢出在模 2^32
  下自动正确。余数 = x−q·y 电路导出（无独立 advice）。断言仅 guard 到 funct3∈4..7。
- **字节/半字访存**（§2.7）：lb/lbu/lh/lhu——**RAM 永远字粒度**：事件列记录整字 raw 值
  （与排序论证 val_cons 语义一致），提取（变量移位 + 掩码 + sext）在写回级；lh/lhu
  半字对齐断言（addr[0]==0）。lw/sw 沿用 M5「地址即字索引」语义（历史程序零破坏）。
- **测试**：`per_inst_m_extension`（17 断言：边界/除零/溢出/符号）、`per_inst_byte_half_mem`
  （合并/提取/符号扩展）、`word_vm32_m8b_isa_prove`（21 周期微程序 mul/divu/remu/div/rem/
  lb/lbu/lh/lhu **电路级端到端 prove→verify** + native 对拍，gates=23,368，imul 门 63）。
- **边界（如实声明）**：**sb/sh 的电路层验证未做**——字节合并需要「读改写」双事件展开
  （store 前需读旧字事件，M5 事件结构每周期至多 1+1 且排序论证 val_cons 需扩展），
  native 语义已实现并对拍（`per_inst_byte_half_mem`）。列为后续（与 M9 tracer 展开方案
  一并设计：tracer 层展开为字读+字写两条微操作，Jolt 同款）。
- **mulh/mulhsu/mulhu 未做**（高位积需要 64 位积高位提取的额外论证；任务书未列）。

## T3：真实工具链 ⏸ 停项（外部依赖）

- 本机无 `riscv32-unknown-elf-gcc` / `riscv64-*` / `clang`（已实测）。
- 任务书的替代路径「预编译二进制」同样需要外部产出（本机无法生成）。
- **建议（下一会话执行）**：安装 `gcc-riscv64-unknown-elf`（Ubuntu 包）或要求 Leader 提供
  预编译 ELF；ELF 加载器按 Jolt `jolt-program/src/image/elf.rs` 段过滤逻辑写 RV32 版；
  tracer = `vm32/interp` 加 ELF 镜像装载；内存布局按 §2.8（I/O 区低地址、字粒度 remap）。
- 需要的接口已就绪：`run_program_big` 的 fetch 闭包 + `prog_image` 参数化，
  ELF 加载器只需产出「镜像函数 + 初始内存」。

## 验收对照（任务书 §2）

1. ✅ 全量测试绿（60/0，+3 ignored）；T0/T1/T2 各有独立 soundness/对拍测试。
2. ✅ 程序公开性：哈希公开 inout 词（`prog_hash`，IO_HASH 布局行）+ 取指 claim 绑定行
   （fetch_ok 闭包的 verify_reduction + verify_oracle_relation）；「换程序被拒」= SwapProgram ✅。
3. ✅ leaf-claim 桥证据：dot_pub 重算对照 + BadBridgeWitness 用例。
4. ✅ mul/div/rem/字节访存各至少一条端到端（word_vm32_m8b_isa_prove）+ native 对拍
   （per_inst_m_extension / per_inst_byte_half_mem）。⚠️ sb/sh 电路层边界如上。
5. ⏸ 工具链缺失诚实记录（见 T3 节）。
6. ✅ 成本数据：N=16 主测 1801 周期 / 905,168 gates / 1.9s（+fetch/logup/桥的开销 <5%；
   inout 从 1 词增至 ~18.5k 词）；ISA 微程序 21 周期 23,368 gates。
7. ✅ verify 层纪律（新 soundness 4 例全部 c_ok/l_ok/hash_ok 断言，无 panic 单独成立）；
   新代码零警告。

## 文件清单

- `crates/zkvm-slice/src/slices/vm_ram_sort.rs` — T0（committed fetch 表 + logup* + 哈希 inout）
  + T1（排序流 8 列公开化 + 开口重算对照 + sortedness_ok + Tamper 5-8）
- `crates/zkvm-slice/src/vm32/{isa,interp,circuit,proof}.rs` — T2（9 条新指令三层）
- `crates/zkvm-slice/src/vm32/per_inst_tests.rs` — T2 native 单测
- `crates/zkvm-slice/src/slices/word_vm32.rs` — T2 端到端测试（m8b_tests 模块）

## 需复核重点

1. **T0 哈希边界**：inout 哈希 ↔ 承诺 root 的同源声明是否可接受，或要求上游加
   commitment getter 后强制（影响验收 2 的表述）。
2. **T1 桥形态偏离任务书字面**（intmul phase5 → 公开化+重算对照）：绑定强度论证见报告，
   请确认接受。
3. **sb/sh 电路层缺口**：是否接受「native 完成 + 电路边界」在本轮通过，展开方案并入 M9。
4. **地址语义并存**：lw/sw 字索引（M5 历史）vs lb 族字节地址（RISC-V 标准），T3 tracer
   需要统一（建议 tracer 层统一字节地址 + lw/sw 按 >>2 重映射）。
