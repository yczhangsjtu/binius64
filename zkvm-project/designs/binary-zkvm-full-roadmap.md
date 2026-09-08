# 完整二元域 zkVM：目标架构与实施路线（Phase 2）

> 日期：2026-09-07 | 地位：本文是 **M6 之后阶段的权威路线**，与
> `designs/milestone-roadmap.md`（Phase 1：切片验证，M1-M6）衔接。
> 依据：`research/jolt-to-binary-field-migration-assessment.md`、M1-M5 各报告、
> `designs/binius64-frontend-api-map.md`。
> **细化设计（逐协议"参考 Jolt 什么 / 二元域怎么换 / 切片怎么接"、M7 两路线构造细节、
> committed 列绑定升级）见 `designs/binary-zkvm-detailed-design.md`（2026-09-07）。**

---

## 0. 定位：M1-M6 是什么、不是什么

M1-M6 是**切片验证阶段**——按 ACCEPTANCE_BASIS 的口径，它证明"zkVM 的每一类机制
（查表、状态机、组合证明、寄存器/内存论证、真指令译码）都能在 Binius64 二元域栈上闭环"。
它**刻意**回避了两件事：可扩展性（固定展开、K=64、版本链 O(K·T)）与真实程序来源
（手写程序镜像）。Phase 2 的目标是消除这两个回避，交付**能证明真实编译程序的
完整二元域 zkVM**。

## 1. 目标架构（M5 已验证的各模块 = 拼图的已验证部分）

```
RISC-V 工具链 (riscv32 ELF)           ← M8 引入（当前：手写程序镜像）
      │ tracer（执行 → trace）
      ▼
每周期统一电路（frontend 词级门，W2）    ← M2/M3/M5 已验证
  ├─ 译码（真 RV32I/M，含字节访存）      ← M5 已验证 30 条；M8 补 mul/div/字节
  ├─ 32 寄存器值链+版本链（电路内）      ← M3 已验证（K=32 可留在电路）
  ├─ PC 链（分支/跳转）                 ← M3/M5 已验证
  └─ 事件 inout 钉扎（R1 纪律）         ← M3 v2 已验证
      │
logup* 查表层（同一 transcript）        ← M3/M4/M5 已验证组合模式
  ├─ 取指表（程序镜像=公开输入）         ← 已验证
  ├─ 寄存器写日志表                     ← 已验证
  └─ RAM 论证（★ Phase 2 的唯一协议缺口）← M4 验证了机制但 O(K·T) 不可扩展
      │
BaseFold PCS / M4 prover              ← Binius64 自带（W2 选型已验证）
```

**已验证 vs 缺口**（诚实对照）：

| 组件 | 状态 | 缺口 |
|---|---|---|
| 译码/执行/分支/跳转 | ✅ M5（30 条真 RV32I） | mul/div/字节访存/ecall |
| 寄存器论证 | ✅ M1/M3 | 无（K=32 电路化可接受） |
| 取指一致性 | ✅ M2/M3 | 程序镜像承诺方案 |
| RAM 论证 | ⚠️ M4 机制对、**复杂度错** | **O(K·T) → 必须换成亚线性论证**（§2） |
| 程序来源 | ⚠️ 手写镜像 | 真实工具链 trace（§4 M8） |
| 性能 | ⚠️ debug 级切片 | release 工程化（§4 M9） |

## 2. 核心协议缺口：可扩展 RAM 论证（Phase 2 唯一的新密码学工作）

M4 的版本链把 64 个计数器放进电路（O(K·T) gates），K=2^20 时完全不可行。
两条候选路线（都来自已完成的分析）：

- **路线 A：排序式离线内存检查（sorter-based）**。访问事件按 (addr, ts) 排序后，
  "读值==前一同址写值"变为排序序列上的相邻一致性；排序正确性用
  "相邻差值良构 + logup\* range check"证明——logup\* 做 range check 在二元域是原生
  强项。无自定义 sumcheck，全部复用已验证组件。
- **路线 B：Twist 忠实翻译**（one-hot ra + committed inc + Val 链 + LT 加权的自定义
  sumcheck 恒等式）。更贴近 Jolt、渐近更好，但要驱动 Binius64 `ip` crate 的
  sumcheck 机器跑自定义恒等式——**可驱动性未验证**（迁移评估 §3 标注的风险点）。

**决策点 = M7 spike**：两条路线各做一个 K=2^16 级切片，按"成本随 K/T 的缩放曲线 +
实现风险"选型。这是 Phase 2 的咽喉，其余都是工程。

## 3. 已知次级缺口（工程，无新协议）

- **mul**：frontend `imul` 门原生（3-4×AND），直接接入（M5 加分项遗留）。
- **div/rem**：照 Jolt 做法展开为虚拟指令序列（advice + 查表断言商/余数合法性），
  无新协议，纯工作量。
- **字节/半字访存**：字内字节提取用移位+band（词级门强项）；地址对齐检查用比较门。
- **程序镜像承诺**：fetch 表从"双方共享 native 表"升级为 prover 承诺 + 公开哈希
  （程序哈希作公共输入）。
- **可变 T**：电路按周期统一，padding 到 2 的幂；Binius64 M4 原生支持。
- **x0/生态**：ecall 做最小 I/O（或暂不模拟系统调用，guest 用裸机 ABI）。

## 4. Phase 2 里程碑序列（续统一编号 M7-M10）

| 里程碑 | 内容 | 验收标准 | 依赖 |
|---|---|---|---|
| **M7** | **可扩展 RAM 论证 spike**：路线 A（排序式）与路线 B（Twist 翻译）各一切片，K=2^16、T≥2^12 | 两路线的 gates/prove-time 随 K、T 缩放实测曲线；明确选型 + 报告 | **✅ 已完成（2026-09-08，v2）**：路线 A 胜（切片 27 `ram_sort`）；gates 与 K 完全无关（0 方差）、随 T 线性（×3.93）；fracaddcheck 自组装值多重集合等式；`M7_REPORT.md` |
| **M8-A** | **VM × RAM 论证整合 + 强承诺通道**（M8 上半）：ram_sort 迁移 BaseFold 通道；vm32 执行核心 + 排序式 RAM 论证接入（删 O(K·T) 版本链）；恒等式②强绑定；K=2^16 | 强通道证据；排序端到端（记录规模）；4 例 verify 层 soundness | **✅ 已完成（2026-09-08）**：切片 28 `vm_ram_sort`；零 naive 残留（BaseFold 通道两切片同型）；N=16 主测（1801 周期 / 905k gates / 1.9s）+ N=32 缩放点（T×3.87→gates×3.87 线性）；恒等式②按任务书 §2.3 降级授权采用 intmul phase5 式 witness 列方案（跨行比较在 quadratic mlecheck 不可表达；leaf-claim 桥列边界）；1024 字排序 O(N²) 不可达（外推 ≥5×10⁹ 门，§2.5 授权记录规模）；`M8_REPORT.md` |
| **M8-B** | **完整 ISA + 真实工具链**（M8 下半）：mul/div/字节访存/ecall 最小 I/O；riscv32 工具链编译 C/Rust → ELF → tracer → trace；跑真实编译程序 | 编译的 C 程序端到端 prove→verify；与参考模拟器逐指令对拍 | **✅ 已完成（2026-09-08）**：T0 fetch 公开化（committed 表 + 公开哈希 + SwapProgram 拒）、T1 leaf-claim 桥（公开列 + 重算对照）、T2 ISA（mul/div/rem/字节访存 9 条）；T3 工具链停项（本机无 riscv32 工具链，HANDOFF_M8B.md）；`M8B_REPORT.md`。遗留：sb/sh 电路层、succinctness 边界（verifier 线性）→ M9 |
| **M9** | **性能工程**：release 基准、witness 生成优化、并行化；产出"成本 vs 指令数"缩放曲线（thesis 的最终定量证据） | T=2^10..2^20 的 prove 时间/gates 曲线；每指令成本表（release） | **✅ 已完成（2026-09-08）**：N=64 跑通（8.88GB 峰值，OOM=环境性）；release 曲线 gates 严格线性 502.3 门/周期、prove≈线性（T×15.2→×16.5）；31 指令 release 成本表 1113 g/cyc 统一；sb/sh 双事件电路层；profile 定位瓶颈=电路构建（`M9_REPORT.md`） |
| **M10** | **工程化收官**：库 API 化（prove(program, input)→proof）、CI、文档、安全审查准备 | 外部调用者可 API 驱动；文档完整 | **✅ 已完成（2026-09-08，v2 验收通过）**：`vmrs_prove/vmrs_verify`（Proof=transcript bytes，bytes 往返端到端 + 自定义镜像注入）；CI 脚本 `tools/run_zkvm_tests.sh`（实跑 ALL GREEN）；`KNOWN_BOUNDARIES.md`/`SECURITY_REVIEW_PREP.md`/电路构建成本设计（`M10_REPORT.md`） |
| **M11** | **安全修复包**（审计驱动，最高优先级）：修复 AUDIT_2026-09-08 的 S1-S5 + 中级项 | 每项修复有"修复前 PoC 可复现/修复后 verify 层拒"对照 | **✅ 已完成（2026-09-08，v3）**：S1-S5 全修复（F3 事件钉扎默认激活、fetch 位置绑定、除法断言三层修复、ver_bound、final_pc_halt）；65 绿；`M11_REPORT.md` v3；诊断更正：F3 阻塞实为 io_* 第二参数 bug（Leader 定位修复） |
| **M12**（原 M11 顺延） | **Verifier succinctness**（用户指定优先，M11 安全修复后启动）：S1 公开输入 O(T)→O(1)（committed-only 列）+ S2 预处理模型拆分（verifier key / online verify） | 公开输入大小与 T 无关（实测）；online verify 不重建电路；proof 体积亚线性明确不做（Phase 3） | 规划 `designs/verifier-succinctness-plan.md` |

> 注（2026-09-08）：M8 按关键路径拆为 M8-A（RAM 论证整合 + 强通道，协议风险集中于此）
> 与 M8-B（ISA 补全 + 工具链，纯工程）。

**明确不在 Phase 2**（另立阶段，需要时再议）：ZK（Binius64 的 zk_mlecheck 路线）、
递归/证明聚合、RV64、(B) 移植 Jolt 代码库。

## 5. 风险登记（诚实版）

1. **M7 是成败咽喉**：若路线 A/B 的缩放都不达预期，thesis 的"成本∝指令数"在大内存
   程序上不成立（版本链底噪淹没执行成本）。缓解：M7 只做缩放曲线实测，数据说话。
2. **tracer 工程量**（M8）：ELF 加载 + 全指令解释器是已知最大纯工程量，无技术风险。
3. **witness 生成速度**（M9）：词级门电路的 witness fill 在 T=2^20 时可能成为瓶颈；
   Binius64 原生电路（blake3 等）证明该栈能跑大规模，风险中低。
4. **div 的 advice 正确性**：虚拟指令展开需要严格的断言覆盖，Jolt 的展开序列可直接
   借鉴（`research/jolt-to-binary-field-migration-assessment.md` §2 已核对）。

## 6. 与 Phase 1 文档的关系

- `designs/milestone-roadmap.md`（M1-M6）保持不变，标记为 Phase 1（切片验证）。
- M7-M10 的任务书在各自启动时生成（同 M2-M6 模式：Leader 写任务书 → Worker 实现 →
  Leader 验收）。
- 当前状态（2026-09-08）：M1-M7 + M8-A + M9 + M10 ✅；M8-B ◐（T0-T2 ✅，T3 工具链待外部依赖）。
- **Phase 2 完成度**：除 riscv32 工具链（M8-B T3）外全部收官。终态文档：`M10_REPORT.md`、
  `KNOWN_BOUNDARIES.md`、`SECURITY_REVIEW_PREP.md`、`tools/run_zkvm_tests.sh`。
