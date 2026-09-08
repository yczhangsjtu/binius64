# M8-A 任务书：VM × 可扩展 RAM 论证整合 + 强承诺通道

> 里程碑：M8 上半（拆分说明见下；权威设计 `designs/binary-zkvm-detailed-design.md` §3/§4）
> 派发日期：2026-09-08 | 派发方：Leader Agent | 执行方：Worker Agent
> 前置：M7 ✅（ram_sort 排序式内存论证 + committed 列 + naive 通道）、M6 ✅（vm32 库）。
> 本任务书含全部架构决策；Worker 施工，遇不可行处停下来记录并给替代方案。

---

## 0. M8 为什么拆成 A/B

原路线图 M8（完整 ISA + 工具链 + 真实程序）过大。按关键路径拆：
- **M8-A（本任务书）**：把 M7 的可扩展 RAM 论证**接入真实 VM**，并迁移到**强承诺通道**
  （BaseFold）——剩余的协议风险全在这里。
- **M8-B（M8-A 验收后另发）**：ISA 补全（mul/div/字节访存，设计已定 §2.6/2.7）+
  ELF/tracer + 真实编译程序。

## 1. 目标

交付切片 28 `vm_ram_sort`：一台**真实状态机 VM**（vm32 的译码/执行/寄存器/PC 全套）+
**RAM 用 M7 排序式论证**（替换 O(K·T) 版本链），**K=2^16 字地址空间**，
**强承诺通道**（BaseFold，非 naive）。程序：对 1024 字的 RAM 数组排序
（bubblesort 升级版），端到端 prove→verify + soundness。

## 2. 架构决策（照此实现）

### 2.1 整体数据流
```
执行电路（vm32/circuit 演进）
  ├─ 寄存器：32 个值链+版本链（不动，M3 方案，K=32 电路化最优）
  ├─ RAM：每周期产出访问事件 (addr, ts, val, is_write) 写入 witness 事件列
  │        ★ RAM 版本链删除；ld_val 不再钉任何电路内链，由内存论证承担
  └─ 事件列 = 电路 witness 的连续区域（电路照常对它们 assert 译码派生关系）
内存论证（M7 机制）
  ├─ 事件列 + init/final 记录 ‖ 排序流（prover witness）
  ├─ 恒等式①：fracaddcheck 多重集合等式（指纹 f = addr+ρ·val+ρ²·ts+ρ³·kind）
  └─ 恒等式②：排序流相邻一致性 + 良构（mlecheck/zerocheck on committed 列，
     或前端电路读 committed 列——见 §2.3 绑定）
承诺层：全部列经强通道（BaseFold）send_oracle；叶子 claim 经 oracle relation 归约。
```

### 2.2 强通道迁移（本里程碑的难点，先做）
- M7 用 `NaiveProverChannel`（测试语义，oracle=全系数）。M8-A 换成 **BaseFold 真实通道**。
- 先在 ram_sort 上单独完成通道迁移（电路不变，只换 channel 类型 + 处理 API 差异），
  验证证明仍闭环——这是独立的 checkpoint，出问题在此止步定位，不要带病进 VM 整合。
- 注意 BaseFold 通道的开口是 FRI 式的：`prove_oracle_relation` 可能批量延迟到
  `finish()`；zk 标志、log_msg_len 的约束按真实通道的 API 来。
- 参考：`crates/iop-prover/src/channel/`（trait 定义与 BaseFold 实现的注册方式）、
  `crates/prover/src/prove.rs`（M4 prover 用的通道是哪个类型——VM 电路证明与
  oracle 列承诺需要在**同一通道**上）。

### 2.3 恒等式②的绑定（M7 报告 §r2 迁移清单 ②）
- 排序流相邻一致性的约束对象必须是 **committed 列本身**（naive 通道下"两副本同源"
  的间隙必须在强通道下闭合）。
- 推荐路径：相邻一致性写成对 committed 列的**二次关系**（每对相邻行的差值约束），
  用 `quadratic_mlecheck_prover`（claim=0 的 zerocheck）检查；比较关系（非降/严增）
  需要位分解辅助列（也 committed，布尔性用 mlecheck 检查）+ 差的 range check。
  若此路径在二次复合内表达不了（如需要三次复合），降级方案：排序流列同时作为
  前端电路 witness + committed oracle，用 intmul phase5 模式绑定两者
  （`crates/prover/src/protocols/intmul/prove.rs:299-348`）。报告中说明选了哪条、为什么。

### 2.4 三件套（init/final/output）在新结构下的形态
- init 记录 val 对照公共初始镜像（排序流首行/每地址组首行，验证端检查）；
- final 记录承载 output（指定输出地址的 final 值为公开 inout）；
- 事件流与执行的钉扎沿用 M4 语义（st_val == rs2 读值、ld_val == 写回值等），
  但落到 witness 列而非公开 inout。

### 2.5 规模与性能预算
- K=2^16 字（256KB），1024 字排序（T 预计 10^5-10^6 周期量级——**先实测，
  若 debug prove 超过 ~5 分钟则降数组到 256 字并在报告记录**）。
- 报告必须给出 gates/prove 时间与 M7 孤立切片的对照（整合开销单列）。

## 3. 任务分解

- **T0 通道迁移**：ram_sort 换 BaseFold 强通道，闭环 + 报告 API 差异点。（checkpoint）
- **T1 VM 整合**：切片 28 `vm_ram_sort.rs`——vm32 执行核心 + RAM 版本链删除 +
  事件列 + 排序式论证 + 三件套，同一 transcript。
- **T2 恒等式②强绑定**（§2.3）。
- **T3 soundness ≥4（全部 verify 层拒绝，M7 v2 纪律）**：过期读（排序配对错）、
  丢/多塞事件（多重集合不等）、读值篡改、最终结果篡改。
- **T4 报告**：`zkvm-project/M8_REPORT.md`（含 T0 通道迁移记录、绑定方案选择、
  缩放/成本数据、边界）；文档四处更新（切片 28、M8-A 状态）。

## 4. 验收标准

1. `cargo test -p binius-zkvm-slice` 全绿。
2. **强通道证据**：指出 BaseFold 通道的类型与实例化行；无 NaiveProverChannel 残留
   （ram_sort 与 vm_ram_sort 都迁移）。
3. **整合证据**：RAM 读值无任何电路内值链/版本链（指出删除点）；事件列与执行的
   钉扎约束行；恒等式① fracaddcheck 调用行；恒等式②绑定方案行。
4. 排序程序端到端：1024（或记录规模的）字排序 native 对拍 + 证明闭环 + 三件套。
5. soundness 4 例全部 verify 层拒绝（无 panic 形态单独成立）。
6. 报告含成本/缩放数据与诚实边界。

## 5. 送审要求

完成后：`zkvm-project/M8_REPORT.md` + 简短送审消息（≤15 行）。
