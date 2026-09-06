# 汇报：M1 寄存器读-写矩阵（ReadWriteChecking）实现

> 汇报 Agent（Hermes）→ 验收 Agent | 日期：2026-09-06 | 基准：`ACCEPTANCE_BASIS.md`
> 范围：新增 `crates/zkvm-slice/src/slices/reg_rw.rs`（代码）+ 文档更新
> 结论先行：**M1 真实实现并跑通**（prove→verify + 拒绝过期读值），`cargo test` 21 全过。
> 它把 zkvm.rs 之前"操作数值独立注入、未绑定到写"的弱点，用**寄存器读-写矩阵**补上了。

---

## 1. 做了什么

新增一个切片 `reg_rw`（`crates/zkvm-slice/src/slices/reg_rw.rs`，259 行），用
**logup\***（sub-multiset 论证）证明：**每条指令读到的寄存器值 = 最近一次写该寄存器的值**。

这是朝 Jolt 风格 zkVM 的第一步——Jolt 的 `registers/read_write_checking.rs` 正是用
读-写矩阵做寄存器一致性，我们把同样的命题用 Binius64 的 logup* 承载。

## 2. 模型（对照 Jolt ReadWriteChecking，但在二元域上）

- **时间排序寄存器状态表** `T[ts * NREG + reg]`（复用 mem_arg_spice 的模板，但表是
  **寄存器文件**而非内存地址）。
- **写**（store 到 rd）= 更新该寄存器当前值；**读**（load 自 rs）= 看该 ts 时刻寄存器值。
- **logup\*** 把所有读/写作为 lookers 绑定到同一张表 → 读值必须是最近写值。

## 3. 程序与真实输出（可复现）

程序：`addi x1,5; addi x2,3; add x1,x1,x2; addi x2,7; add x1,x1,x2; addi x5,x1,1`

真实运行（`cargo test -p binius-zkvm-slice --lib reg_rw -- --nocapture`）：
```
== M1: Register read-write matrix (logup* sub-multiset, binary field) ==
   program: addi x1,5; addi x2,3; add x1,x1,x2; addi x2,7; add x1,x1,x2; addi x5,x1,1
   final regs: x1=15 x2=7 x5=16 (cross-check native) ✓
     ts=0 reg=x1 val=5 WRITE
     ts=1 reg=x2 val=3 WRITE
     ts=2 reg=x1 val=5 READ      <- 第一次 add：读 x1=5（初值）
     ts=4 reg=x1 val=8 WRITE     <- add 后 x1=5+3=8
     ts=5 reg=x2 val=7 WRITE     <- addi x2,7 覆盖 3
     ts=6 reg=x1 val=8 READ      <- 第二次 add：读 x1=8（非过期 5）
     ts=7 reg=x2 val=7 READ      <- 第二次 add：读 x2=7（非过期 3）
     ts=8 reg=x1 val=15 WRITE
     ts=10 reg=x5 val=16 WRITE
   logup*: time-ordered register table T[ts*8 + reg], 11 events
   ✅ register read-write matrix verified (reads see most-recent writes)
   cross-check: native x1=15 x2=7 x5=16 ✓
   soundness: verifier REJECTED a stale register read (x1=5 after x1=15) ✓
```

**读见最近写两项关键证据**：
- 第 3 条指令 `add x1,x1,x2`（第二次）读 x1=**8**（第 1 次 add 写过），而非过期值 5；
- 同条读 x2=**7**（addi x2,7 覆盖后），而非过期值 3。

## 4. 完整测试状态（全部 21 个）

```bash
cd /home/yczhang/workspace/binius64 && export RUSTFLAGS="-C target-cpu=native" && CARGO_BUILD_JOBS=4 cargo test -p binius-zkvm-slice
```
→ `running 21 tests ... ok; 21 passed; 0 failed`（原 20 切片 + 新增 reg_rw）。

## 5. 边界与诚实声明

- ⭐ **真实实现**：寄存器读-写一致性由 logup* sub-multiset 论证强制（非 native 手工填值）。
- ⚠️ **边界（须如实承认）**：
  1. **仅寄存器读-写一致性**——尚未接入指令执行/ALU（`reg_rw` 是独立的寄存器论证切片，
     尚未与真实状态机 drive 融合）。
  2. **表长/时间戳上限**：`TS_MAX=16`、`NREG=8`，表 `16*8=128` 格（`m=7`）——教学规模，
     程序访问数不能超过 ts 上限（当前 11 个事件）。
  3. **仍是"读⊆写+时间序"层面的论证**——未能证明"该读时刻确实经过了正确的写序列"
     （即时间戳排序的合法性本身未在证明系统内强制，与 mem_arg_spice 同类边界）。
  4. **未做寄存器值→ALU 结果的绑定**（本条读 rs、ALU 算 res、写 rd 的自洽电路约束仍缺失）。

## 6. 变更文件

- `crates/zkvm-slice/src/slices/reg_rw.rs`（新增，259 行，M1 主体）
- `crates/zkvm-slice/src/lib.rs`（`#[path]` 引入 reg_rw 模块）
- `zkvm-project/PROGRESS.md`（加 M1 条目）
- `zkvm-project/architecture.md`（切片数 20→21，§3 表加 reg_rw 行）
- `zkvm-project/research/zkvm-gap-analysis-jolt.md`（新增，M1 的 Jolt 差距分析依据）

## 7. 提交

- `e7bee30`（feat(M1)：reg_rw + 文档）→ 已推送 `yczhangsjtu/binius64`

## 8. 后续

- **M2**：指令查表化执行（`LookupQuery` + `CircuitFlags`，把硬编码加法器改为 logup* 查表），
  并让寄存器读-写矩阵与真实状态机融合。
- **M3**：内存时序（increment 而非手工填表）。
