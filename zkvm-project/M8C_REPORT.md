# M8-C 送审报告（真实编译程序端到端，M8-B T3 续作，2026-09-08）

Workdir: `/home/yczhang/workspace/binius64`。范围：`crates/zkvm-slice/` + `zkvm-project/`；
未动上游、无 git 操作。任务书 `tasks/M8-C-toolchain-e2e.md`。全量回归
**62 passed / 0 failed / 4 ignored**（新增 ELF 端到端 1 项）。

## 结论

**C 源码 → riscv32 ELF（gcc 13.2.0）→ `vm32::elf` 加载 → tracer 执行 →
`vmrs_prove_with_init` → `vmrs_verify` 全链闭环**：16 元素冒泡排序（含重复与边界值
0x80000000/0xffffffff）863 周期 / 430,700 gates 端到端全绿，输出与独立 Rust 参考排序
对拍一致。**KNOWN_BOUNDARIES #1（init 全 0 假设）正式解除**——本轮唯一协议面改动
（任务书 §3）已实现并带 soundness 覆盖。

## T1：C 测试程序 + 构建脚本 ✅

- `crates/zkvm-slice/testdata/bubble16.c`：16 词 in-place 冒泡（重复 + 0x80000000/
  0xffffffff/0 边界值），完成标志字（字节 0x400 := 0xC0DE600D），`ecall` halt。
- `testdata/fib.c`（可选件）：迭代 fib(12)=144 写标志地址。
- `testdata/build.sh`：`riscv64-unknown-elf-gcc -march=rv32im -mabi=ilp32 -nostdlib
  -nostartfiles -ffreestanding -O1 -Wl,-Ttext=0x0 -e _start`（无压缩指令 ✓）；
  产物 `bubble16.elf`/`fib.elf`（+ 反汇编 .dis）已提交，脚本可复现。
- **内存布局约定**（脚本/C 注释与 README 口径一致）：text 0x0 起、.data 跟随 LOAD
  （bubble16 的数组在字节 0x1054 = 字 0x415）、栈顶 0x3fffc、标志字 0x400、
  RAM = 2^16 词（256KB，字节地址 mask 0x3ffff）。

## T2：ELF 加载器 ✅

`vm32/elf.rs`（新，~180 行）：**手写最小 ELF32 parser**（零新依赖——`object` crate 未在
workspace 依赖树，避免动依赖图）。解析 ELF header → PT_LOAD 段（Jolt 式过滤：非零
file size）→ 词粒度初始内存；section headers 的 `PROGBITS+AX+ALLOC` 段 → fetch 镜像
（section 被strip 时以 PF_X LOAD 段兜底）。产出 `(entry, text 镜像, init_words)`。
**修复史**：e_shoff 双读 bug（ELF header 字段错位，导致 .text 不进 fetch 镜像——
`cycles=2` 特征）与 PF_X 段计数缺失，均已修并有测试覆盖。

## T3：tracer 接入 + init 非零化 ✅

- **fetch 从镜像来**：`elf_fetch(&img)` 闭包接 `run_program_big`（接口 M8-B 已备）。
- **地址语义统一（M8-B 复核点④落地）**：`run_program_big` 与 vm_ram_sort 电路的
  ld/st 地址全部统一为**字节地址**（字索引 = `(addr>>2) & 0xffff`，全指令一致，
  含 sb/sh 双事件路径）；内置 bubblesort 镜像同步适配（元素步长 4、循环上界/变量
  字节量纲化）——vm32 库历史测试不回退（word_vm32 全绿）。gates 影响约 +8 门/周期
  （地址 srl32 变换），数字已按新基线记录。
- **init 非零化（任务书 §3，唯一协议面改动）**：
  - init 行 val = 初始镜像词（`build_sorted_with_final` / `event_rows` 接 `init_mem`）；
  - 电路删除 `init_val==0` 断言；新增 `init_vals` 公开 inout 列（按排序流 init 行顺序）；
  - **对照链**（同 fetch 哈希模式）：init_vals（transcript 承诺）← 验证端本地对照 ←
    `proof.init_words`（声明）← Sha256 ← `proof.init_hash`（**外部与 ELF 加载结果比对**，
    同 expected_hash 的公共输入锚模式）；
  - `sortedness_ok` 扩展 init 行对照（`s_ok` 覆盖）。
- 新公开 API：`vmrs_prove_with_init(n, program, init_mem)`（init 全 0 的
  `vmrs_prove` 保持不变；M10 v2 的公开签名纪律对新函数同样适用——无测试钩子）。

## T4：端到端 + 对拍 + soundness ✅

`vm_ram_sort_elf_bubble16_e2e`（走公共 API）：
- **对拍**：trace 输出 = Rust `sort_unstable` 独立参考（逐词相等）+ 标志字 0xC0DE600D；
  输入数组加载断言（边界值在位）。
- **成本**（首个真实编译程序数据点，已进 BENCHMARKS.md）：**cycles=863、ts=898、l=11、
  gates=430,700**、proof bytes ~430KB 量级（`[phase] proof_bytes` 日志）。
- **soundness 3 例**（verify 层，无 panic）：
  1. 篡改输出 final_out（Tamper::BadFinalOut 路径）→ c_ok=false；
  2. **换不同编译产物**（fib.elf 的 proof 对照 bubble 的哈希）→ hash_ok=false；
  3. **篡改初始镜像声明**（init_words 词翻转）→ s_ok=false（init 对照链拒绝）。
- fib.elf 同 init 环境端到端全绿（第二程序证据）。

## 验收对照

1. ✅ 全量 62/0/4；新增端到端走公共 API（`vmrs_prove_with_init`/`vmrs_verify`）。
2. ✅ testdata/ 内 ELF + build.sh 可复现（产物与脚本同提交）。
3. ✅ 编译冒泡 prove→verify 通过，输出与独立参考对拍一致。
4. ✅ init 非零镜像协议改动有 soundness 覆盖（篡改初始镜像词 → 拒）。
5. ✅ KNOWN_BOUNDARIES #1 标记解除；HANDOFF_M8B 归档（T3 完成）；BENCHMARKS 增补；本报告。
6. ✅ verify 层纪律；新代码零警告（vm_ram_sort/elf.rs 无警告）。

## 需复核重点

1. **init 对照链的锚**：外部锚 = `proof.init_hash`（调用者与 ELF 比对）；链内
   init_vals↔init_words 为本地对照（两者都是 prover 可伪造项，但伪造后 init_hash
   随之变化 → 外部对照失败）——与 expected_hash 同构，请确认接受该安全论证。
2. **地址语义统一范围**：vm_ram_sort 全栈已字节地址化；**vm32 库（word_vm32 切片）
   仍为字索引历史语义**（任务书"历史测试不回退"）——两套语义并存状态延续，最终
   归一需 vm32 电路迁移（未在本轮）。
3. **ELF loader 的边界**：仅支持小端 ELF32/PT_LOAD/简单 section 布局；无 bss 清零段
   处理（p_memsz > p_filesz 的零填充未显式做——init 未触及词按 0 处理语义等价）；
   无重定位处理（链接器已解析）。复杂 ELF（带初始化数组重定位、多 text 段）未验证。
4. **n 参数在 program 注入时的语义**：仅决定 fetch 表大小上限（`1<<m_prog(n)`）与
   兼容包装的 BASE 排序检查（注入时跳过）——调用者须保证镜像 ≤ 表容量。
