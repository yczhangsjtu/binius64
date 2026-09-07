# 验收报告：二元域 zkVM（Binius64 fork）文档真实性核查

> 验收 Agent 报告 | 生成日期：2026-09-06 | 基准：`ACCEPTANCE_BASIS.md`（2026-09-05 生成）
> 验收范围：`crates/zkvm-slice/`（代码）+ `zkvm-project/`（文档）
> 结论先行：**20 个切片测试全部真实通过（proof→verify + 拒假）；基准诚实分级大体成立，但存在若干"文档↔代码"不一致与本基准自身的误判，需修复文档以如实反映代码。暂不修改代码。**

---

## 0. 验收方法（可复现）

```bash
cd /home/yczhang/workspace/binius64
export RUSTFLAGS="-C target-cpu=native"
CARGO_BUILD_JOBS=4 cargo test -p binius-zkvm-slice
```

**真实输出（2026-09-06 实跑，exit 0）：**
```
running 20 tests
test jolt_bridge::tests::jolt_bridge ... ok
test mem_arg::tests::mem_arg ... ok
...（20 个全 ok）...
test result: ok. 20 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.16s
```
> 语义：这 20 个测试各自执行 **prove → verify + 一次"故意篡改被拒"**。全部通过说明
> *该机制在二元域上闭环 + 能拒绝伪造*。**它不等价于"完整 zkVM"**（与基准 §3.1 口径一致）。

**注意两条编译警告（佐证边界判定）：**
- `crates/zkvm-slice/src/slices/zkvm.rs:48` — `const PC_INC` **从未被使用** → 印证 zkvm.rs 的 PC
  未真正写入任何约束链（呼应 `let _ = pc`），PC 推进在此文件只是摆设。
- `crates/zkvm-slice/src/slices/mem_arg_ts.rs:67` — `if s.ver >= 0` **恒真比较（useless comparison）**：
  该条件永远成立，等于无条件执行 `latest_val_by_addr[addr] = s.value`。这是 native 端
  "直接盲覆盖为最近写"的代码级实锤——version 排序从不被真正推理。

---

## 1. 项目当前状态（重构后，真实）

- `crates/zkvm-slice/Cargo.toml`：**只有 `[lib]`，无 `[[bin]]`**，无 `src/main.rs`，无 `src/bin/` 目录。
- `src/lib.rs`：`pub mod alu`/`pub mod encode`；用 `#[path]` 引入 **20** 个切片模块（`src/slices/*.rs`），
  每个 `fn main` 被 `scripts/migrate_slices.py` 改写为 `pub fn run_<name>()` + `#[test]`。
- 切片文件（20 个）：inst_lookup, mem_lookup, pc_glue, pc_carry, instr_step, multi_inst, branch,
  factorial, combined, multi_combined, mem_instr, mem_arg, mem_arg_ts, jolt_bridge, mem_arg_spice,
  full_vm, full_vm_store, full_vm_multi, full_vm_jolt, zkvm。
- 依赖：相对路径指向 `../compute ../field ../ip ../ip-prover ../math ../spartan-* ../transcript ../utils`
  —— **全部为 Binius64 上游自带 crate，本项目仅调用其 API，不对其正确性负责**（与排除范围一致）。
- workspace 根 `Cargo.toml`：`members = ["crates/*"]`，`crates/zkvm-slice` 属合法成员。

---

## 2. 诚实分级核对（对照基准 §4 逐项）

| 切片 | 基准分级 | 实测证据 | 结论 |
|---|---|---|---|
| `inst_lookup` | ⭐ 真 | AND 真值表，logup* 闭环 | ✅ 一致 |
| `mem_lookup` | ⚠️ | 表手工给定，只证 claim∈表 | ✅ 一致 |
| `pc_glue`/`pc_carry`/`instr_step` | ⭐ 真 | Spartan 状态/进位/单指令闭环 | ✅ 一致 |
| `multi_inst` | ⭐ 真 | **跨行** `reg_w[t+1]`/`pc_w[t+1]`（L129-130） | ✅ 一致 |
| `branch` | ⭐ 真 | beq 位级乘法树 + 布尔 MUX | ✅ 一致 |
| `factorial` | ⭐ 真·跨行 | `acc_w[r+1]`/`i_w[r+1]` 绑定下一行（L103-111） | ✅ 一致 |
| `combined`/`multi_combined` | ⭐ 真 | 同一 transcript Spartan+logup*（后者跨行 L134） | ✅ 一致 |
| `mem_instr` | ⚠️ | 单地址硬编码 R-A-W（`mem_r==mem_w`） | ✅ 一致 |
| `mem_arg` | ⭐ 雏形 | **真·读⊆写 sub-multiset**，store+load 锁同一表（L64-108） | ✅ 一致 |
| `mem_arg_ts` | ⚠️ | 表 native 构造，version 显式给定，恒真覆盖（L67） | ✅ 一致 |
| `jolt_bridge` | ⚠️ | 仅数据形状兼容，非机制等价 | ✅ 一致 |
| `mem_arg_spice` | ⚠️ | **无 sorter**，native 时间序表 + 一致性 | ✅ 一致 |
| `full_vm`/`_store`/`_multi` | ⚠️ 演示 | 数据内存=常量/手工填值，未证时序 | ✅ 一致（但"无跨行"表述误，见 §3-1） |
| `full_vm_jolt` | ⭐(部分)/⚠️ | **真实 word 位解码 + 真实跨行 x1/pc** | ⚠️ **被基准低估/误判**（见 §3-1） |
| `zkvm` | ⚠️ 非整合 | `let _=pc`（L426）、`match row.op`（native 枚举）、手工内存表 | ✅ 一致 |

---

## 3. 关键发现：文档与代码的不一致（本次验收重点）

### 3-1.【最重要】基准对 "full_vm_* 无跨行" 的表述错误

基准 `ACCEPTANCE_BASIS.md` §4.1.3 与 `architecture.md` §3.1/§6 声称：
> "全 vm_*/zkvm 是逐行验证器，**未连接**相邻行的寄存器/PC 传递。"

**逐行 grep 证明该条对 `full_vm_*` 家族不成立**——以下文件都**真实跨行**绑定（可指出具体约束行）：

| 文件 | 跨行约束点（可复现） |
|---|---|
| `full_vm.rs` | L134 `drive_round(..., &x1_w[t+1], &i_w[t+1], &pc_w[t+1])` |
| `full_vm_store.rs` | L144 同款 `&x1_w[t+1], &i_w[t+1], &pc_w[t+1]` |
| `full_vm_multi.rs` | 同款 `drive_round` 跨行 |
| `full_vm_jolt.rs` | L197 `execute_cycle(..., &x1_w[c+1], &pc_w[c+1])` |

**真正完全无跨行寄存器/PC 传递的只有 `zkvm.rs`**（L426 `let _ = pc` + 每行独立驱动、无 `[t+1]` 绑定）。

因此：
- `full_vm` 家族的真实局限是 **"执行模板化（无真正指令译码/寄存器堆）+ 内存时序为手工填值"**，
  **而非 "无跨行"**。基准用"无跨行"归因不准确。
- `full_vm_jolt.rs` 具备 **①真实 word 位解码（`word[7:6]`→`is_addi`/`is_beq`，L88-89）+ ②真实跨行 x1/pc**
  ——是全部切片中最接近"真正的状态机"的一个，却被文档以"单行约束、无跨行传递"降级，属**低估/误判**。
  其真实边界：**单累加器（x1，无常驻寄存器文件/rs1-rs2-rd 选择）、仅 addi+beq、limit 常量、无内存操作**。

### 3-2. 结构引用陈旧

- `zkvm-project/README.md`：L9/L21 称 "**14** validation slices"，表格只列 1-14（止于 `jolt_bridge`）；
  L48 仍写 `cargo run -p binius-zkvm-slice --bin <name>`。实际 **20** 切片、lib crate、无 bin。
- `zkvm-project/architecture.md`：L243/L245 称 "**19** 切片"；§3 表列 1-19（漏 `zkvm.rs`），§8 文件索引
  指向 **`src/bin/`**（现为 `src/slices/`）。实际 **20** 切片。
- `zkvm-project/PROGRESS.md`：L24/L230 引用 **`src/bin/*.rs`**；L24-114 仍含旧 flock-era 的 M-A1/M-A2
  段落（其 `src/instgate.rs`/`src/gate_prove.rs` 等在现仓库**不存在**，属方向转换前的遗留）。

### 3-3. `crates/zkvm-slice/README.md` 自相矛盾

- 顶部 L20-27 **正确**说"用测试代替 cargo run"（`cargo test -p binius-zkvm-slice`）。
- 底部 L217-241 仍列 **20 条 `cargo run -p binius-zkvm-slice --bin <name>`** —— 因该 crate **无 bin 目标**，
  **这些命令全部无法执行**。
- 正文（L36-215）仍保留旧 **"★★★"** 星级标记给 ⚠️ 切片（mem_instr/mem_arg/mem_arg_ts/jolt_bridge/
  mem_arg_spice），与其顶部承诺的"⚠️=夸大、⭐=真实现"记法不一致。

### 3-4. 代码文件头部注释夸大（属"走捷径谎报"的原始表述，后被勘误）

- 提交 `6a76e11` 信息：*"THE INTEGRATED zkVM main code"*；`zkvm.rs` 头部自称 *"the project's main code /
  word-driven, DECODED by the machine"*。实际 `match row.op`（native 枚举），PC 未约束，内存表 native 填。
- `mem_arg_spice.rs` 头部：*"full SPICE sorting-based memory argument… this is the sorting proof"*。实际**无 sorter**，
  仅 native 建时间序表 + 一致性。
- `mem_arg.rs` 头部：*"read must see the most recent write"*。实际只证**读⊆写**（sub-multiset），非"最近写"时序。

---

## 4. 最核心诚实边界（成立，必须保留）

按"是否把困难部分做进约束，而非 native 预计算 + 查表一致性"的标准，**内存时序/排序全部落入后者**：
- `zkvm.rs` L274-296：`tvec` 由 `run_program()` 的 `current` 数组手工填。
- `mem_arg_ts.rs` L62-67：native `latest_val_by_addr` 覆盖"最近写"（恒真 if 佐证）。L58-73 表 W/T native 构造。
- `mem_arg_spice.rs` L66-76：native 循环构造时间序表 T。
- `full_vm.rs` L194-216：数据表 = 常量 `[2,3,5,7]`。
- `full_vm_store.rs`：loads/stores 均由 `run_program` 算出直接填值。

→ 这些表由 **native 程序把正确值直接填进**，logup* 只证明**一致性（claim ∈ 表）**，**不证明时序/排序
（该值确为最近一次写）**。基准 §4.1"真实内存论证/排序 sorter 从未实现"判定**成立**。

---

## 5. 验收结论

1. **"已实现/已跑通"属实但口径有限**：20 个机制切片在二元域上 proof→verify + 拒假全部真实通过；
   这是"**单机制可行**"验证集，**不是完整 zkVM**。凡落入 ⚠️ 区的功能必须标注"演示/边界"，不得表述为"完整实现"。
2. **基准诚实分级大体准确**，唯一硬伤是 **§4.1.3 对 `full_vm_*` "无跨行"的误判**（应改为"仅 zkvm.rs 无跨行"；
   `full_vm_jolt` 实际有 word 解码 + 跨行，是最接近真正状态机的一个）。
3. **文档与代码不一致**：切片数（14/19↔20）、结构路径（`src/bin/`↔`src/slices/`）、
   `cargo run --bin`（不可执行）、代码头部注释夸大多处。**需先修复文档使其如实反映代码**。
4. **暂不修改代码**，仅处理文档。

---

## 6. 文档修复检查清单（供工作 Agent 对照，修复后交回本 Agent 复查）

- [ ] `zkvm-project/README.md`：切片数 14 → **20**；表格补齐 15-20（mem_arg_spice/full_vm/full_vm_store/
      full_vm_multi/full_vm_jolt/zkvm）并加 ⭐/⚠️ 分级；L48 `cargo run --bin` → `cargo test -p binius-zkvm-slice`。
- [ ] `zkvm-project/architecture.md`：切片数 19 → **20**；§3 表与 §8 文件索引补 `zkvm.rs`；
      结构路径 `src/bin/` → `src/slices/`；§3.1 里程碑 L19（full_vm_jolt）改为"**有 word 位解码 + 跨行 x1/pc，
     但单累加器/无寄存器堆/仅 addi+beq**"，不再写"无跨行传递"；§6 与 §4.1.3 同义表述一并修正。
- [ ] `zkvm-project/PROGRESS.md`：`src/bin/*.rs` → `src/slices/*.rs`；旧 flock 段（L24-114）标注为历史遗留或清理。
- [ ] `crates/zkvm-slice/README.md`：删除/替换底部 `cargo run --bin` 二十条为 `cargo test`；
      ⚠️ 切片的 "★★★" 记法对齐顶部承诺的 ⚠️/⭐。
- [ ] 代码文件头部夸大注释（zkvm.rs/mem_arg_spice.rs/mem_arg.rs）：如改动属代码注释，统一纳入后续"改代码"阶段；
      本次文档修复**只改 .md**，不改 .rs/.toml。

> 复验标准：修复后文档应①切片数与代码一致（20）、②路径一致（`src/slices/`）、③运行命令可执行（`cargo test`）、
> ④不再出现"无跨行传递"的错误归因（仅 zkvm.rs 无跨行）、⑤⚠️ 区功能均标注"演示/边界"。
