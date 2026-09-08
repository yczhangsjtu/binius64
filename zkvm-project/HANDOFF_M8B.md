# Handoff: M8-B 剩余工作（T3 真实工具链）

> 交接日期：2026-09-08。T0/T1/T2 已完成并送审（`M8B_REPORT.md`，测试 60 passed/0 failed）。
> 本文件只含 T3 的断点与方案，勿重新采集上下文。

## T3 断点
- 本机无 riscv32 工具链（gcc/clang 均缺）。需先装 `gcc-riscv64-unknown-elf`（apt）或由
  Leader 提供预编译 ELF（放进 `zkvm-project/testdata/`）。
- 实施清单（设计详案 §2.8）：
  1. ELF 加载器：解析 RV32 ELF section headers，段过滤（LOAD 段）→ 程序镜像 + 初始内存
     （参照 Jolt `jolt-program/src/image/elf.rs`；可用 `object` crate 或手写 parser——
     注意禁改上游，`object` 若已在依赖树可直接引用 workspace 依赖声明）。
  2. tracer：`vm32/interp::run_program` 演进——fetch 从镜像函数来（`run_program_big`
     的闭包形态已支持），mem 初始化用加载结果。
  3. 内存布局：I/O 区低地址（input/output/termination），程序区之上 stack 向下/heap 向上；
     字粒度 remap = (addr − lowest)/4。**注意统一地址语义**（lw/sw 字索引 vs lb 族字节
     地址，见 M8B_REPORT 复核重点 4——建议 tracer 层统一字节地址）。
  4. 端到端：C 冒泡/斐波那契 → ELF → tracer → `run_program_big` + vm_ram_sort 证明闭环
     + 参考执行对拍。
- 工作目录纪律不变：只动 `crates/zkvm-slice/` + `zkvm-project/`；禁改上游；禁 git；
  rustup 工具链（`~/.cargo/bin` 前置 PATH），RUSTFLAGS/CARGO_BUILD_JOBS=4。
- 相关代码：`crates/zkvm-slice/src/slices/vm_ram_sort.rs`（run_vmrs/prog_image），
  `crates/zkvm-slice/src/vm32/interp.rs`（run_program），测试 60 项为回归基线。
