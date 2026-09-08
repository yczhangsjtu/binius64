# 环境需求：RISC-V 工具链（zkVM 项目 M8-B T3 解锁条件）

> 提交：Leader Agent（binius64 zkVM 项目） | 日期：2026-09-08 | 对象：环境维护 Agent

## 需求一句话

在开发机上安装 **RISC-V 32 位交叉编译工具链**，使 riscv32 目标的 C 程序可以编译为裸机 ELF。

## 具体需求

1. **必须**：能编译 `riscv32` 裸机（freestanding）ELF，任选一：
   - `riscv64-unknown-elf-gcc`（Ubuntu 包 `gcc-riscv64-unknown-elf`，用 `-march=rv32im -mabi=ilp32` 产出 32 位）——**推荐**，一条 apt 即可；
   - 或 `riscv32-unknown-elf-gcc`；或支持 riscv32 的 clang/lld。
2. **用途**：把 C 测试程序（冒泡排序/斐波那契级）编译为 ELF，供项目的 ELF 加载器 + tracer
   执行并证明（M8-B T3，接口已就绪，见 `zkvm-project/HANDOFF_M8B.md`）。
3. **验收**：`riscv64-unknown-elf-gcc -march=rv32im -mabi=ilp32 -nostdlib hello.c -o hello.elf`
   能产出 ELF32 文件（`file hello.elf` 显示 ELF 32-bit LSB, RISC-V）。
4. **环境注意**：本机 Rust 用 rustup（`~/.cargo/bin` 前置 PATH，rust-toolchain.toml 钉 1.97.1）；
   系统 /usr/bin/cargo 是 1.75，不要动它；安装用 apt 需 sudo（请用户授权或代执行）。
5. **替代交付**（若不便安装）：在任何有工具链的机器上编译几个测试 ELF（riscv32im、裸机、
   静态、无压缩指令集扩展 `-march=rv32im` 不含 c），把二进制交付到
   `crates/zkvm-slice/testdata/`。

## 背景（可选读）

- 阻塞点详录：`zkvm-project/HANDOFF_M8B.md`、`zkvm-project/M8B_REPORT.md` §T3。
- ELF 加载器规范：参照 Jolt `jolt-program/src/image/elf.rs` 的段过滤逻辑（RV32 版）。
