# M8-C 任务书：真实编译程序端到端（M8-B T3 续作，工具链已就绪）

> 派发日期：2026-09-08 | 派发方：Leader Agent | 执行方：Worker Agent
> 前置：M8-A/B ✅（vm_ram_sort，公共 API vmrs_prove/vmrs_verify）；
> **环境已解锁**：`riscv64-unknown-elf-gcc` 13.2.0 已装（验收命令
> `-march=rv32im -mabi=ilp32 -nostdlib` 编 hello.c 出 ELF32 已通过）。
> 断点与接口：`zkvm-project/HANDOFF_M8B.md`（方案段仍有效，本任务书为其正式版）。

---

## 1. 目标

证明一个**真实编译的 C 程序**（不再是手写镜像）：C 源码 → riscv32 ELF →
ELF 加载 → tracer 执行 → `vmrs_prove` → `vmrs_verify` 闭环 + 与参考执行对拍。
这是"这是一台真 zkVM"的最后一块拼图。

## 2. 任务分解

### T1：C 测试程序 + 构建脚本
- 写 1-2 个裸机 C 程序（冒泡排序必选 + 斐波那契可选），放在
  `crates/zkvm-slice/testdata/`（含 Makefile 或 build.sh）：
  `-march=rv32im -mabi=ilp32 -nostdlib -Wl,-Ttext=...`（自写入口 `_start`，
  不用 libc；输出经 memory-mapped I/O 区或固定内存地址写出）。
- 编译产物 ELF 提交到 testdata/（可复现：脚本 + 产物都在）。
- **约定**（写进 README/注释）：内存布局（I/O 区/程序区/栈），入口地址，输出位置。
  注意编译时**禁用压缩指令**（rv32im 不含 c ✓）。

### T2：ELF 加载器（vm32 库新模块 `vm32/elf.rs`）
- 解析 RV32 ELF（`object` crate——先确认它已在 workspace 依赖树可引用；
  否则手写最小 parser），取 LOAD 段 → 程序镜像 + 初始内存镜像 + 入口地址。
- 参照 Jolt `jolt-program/src/image/elf.rs` 的段过滤逻辑（RV32 版）。
- **初始内存非零**：这是 KNOWN_BOUNDARIES #1（init 全 0 假设）的解除点——
  init 记录的值改为从镜像读取，init 行 val 断言从"恒 0"改为"等于公共初始镜像词"
  （init 镜像进公共输入/承诺表，验证端可对照）。这是本轮唯一的协议面改动，
  设计见下 §3。

### T3：tracer 接入（vm32/interp 演进）
- fetch 从 ELF 加载的镜像来（`run_program_big` 的闭包已支持）；
- 地址语义统一为**字节地址**（M8-B 复核点④的方案落地：tracer 层统一，
  lw/sw 字索引语义在此映射 `>>2`，注意 vm32 历史测试不回退）；
- guard 上限按程序规模调整。

### T4：端到端 + 对拍 + soundness
- C 冒泡排序（≥16 元素，含重复/边界值）→ ELF → tracer → prove→verify；
  输出区内容与**独立参考实现**（同算法 Rust/Python 直接算）对拍。
- soundness ≥2 例（verify 层纪律不变）：篡改输出；换一个不同编译产物
  （程序哈希不符 → 拒）。
- 成本记录：周期数/gates/prove 时间进 BENCHMARKS.md（首个"真实编译程序"数据点）。

## 3. 协议面改动（唯一一处，Leader 决策）

**init 镜像非零化**：排序式内存论证的 init 记录当前断言 val==0。改为：
- init 记录的 val = 公共初始镜像词（来自 ELF 加载结果）；
- 验证端对照：初始镜像的哈希（或逐词）作为公共输入，init 行的 val 断言
  绑定到该公共值（仿 fetch 表哈希的模式：镜像列 committed + 公开哈希 inout）。
- RAM 初始全 0 之外的部分（未初始化区）仍按 0 处理。

## 4. 验收标准

1. 全量测试绿（不回退）；新增端到端测试走公共 API（vmrs_prove/vmrs_verify）。
2. C 程序真实编译产物（testdata/ 内 ELF + 构建脚本可复现）。
3. 端到端：编译的冒泡排序 prove→verify 通过，输出与独立参考对拍一致。
4. init 非零镜像的协议改动有 soundness 覆盖（篡改初始镜像词 → 拒）。
5. 文档：KNOWN_BOUNDARIES #1 更新为"已解除"；HANDOFF_M8B 标记 T3 完成归档；
   BENCHMARKS.md 增补；M8C_REPORT.md。
6. verify 层 soundness 纪律；新代码零警告。

## 5. 送审要求

完成后：`zkvm-project/M8C_REPORT.md` + 简短送审消息（≤15 行）。
