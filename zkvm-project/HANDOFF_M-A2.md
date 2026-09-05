# M-A2 Handoff: 指令门 R1CS + Ligerito prove/verify

## 一句话目标
在 flock (BinarySpartan/Ligerito) 之上实现 RV32I 指令门的端到端 zkSNARK 证明。

## 当前状态
- M-A1（native RISC-V 参考）✓ 完成
- M-A2（指令门 R1CS + prove/verify）**代码可编译可运行**，但验证失败于 `Wiring(Gkr(ProductMismatch))`

## 阻塞问题
证明生成成功，但 `verify_ligerito_union_circuit` 报 `Wiring(Gkr(ProductMismatch))`。

**根因**：`CircuitBuilder` 的 wiring（io_schema word-column → committed polynomial position）和 `generate_witness()` 的 bit packing 没有对齐。

**具体表现**：
- Lincheck 通过 ✓（R1CS 约束满足）
- Wiring GKR 失败（σ 置换 product 不一致）
- Builder 把 io_schema 的 word 0/1/2/3 映射到 committed polynomial 的特定 F128 位置
- 我们的 witness 按 R1CS bit layout 填数据，没匹配 builder 的映射

## 已确认的 flock 架构事实

### CircuitBuilder 路径（anoncred.rs 模式）
1. `CircuitBuilder::new(nu)` → 创建 builder
2. `b.slot(gate)` → 注册 gate slot，gate.table() 返回 TableType（含 R1CS）
3. `b.value(actual_val)` → 创建 Wire 绑定真实值
4. `b.gate(slot, &[inputs])` → 连接 Wire 到 gate，builder 用实际值求值
5. `b.publish(out)` → 公开输出
6. `b.finish()` → 构建 `BuiltCircuit`（含 shape + witness）
7. `built.rows::<Gate>(slot)` → 读取 builder 收集的 Row 记录
8. Witness 从 rows 生成（DeferredToRows 模式）

### R1CS/Lincheck 路径
- gate.table() 返回 `TableType::from_block_r1cs(&r1cs).with_io_schema(io_schema)`
- `from_block_r1cs` **克隆** A_0/B_0/C_0 到 TableType
- Registry 存储 TableType（含克隆的矩阵）
- Lincheck circuit 来自 `BlockR1cs::csc_lincheck_circuit()`
- Prover 需要：Circuit（wiring）+ LincheckCircuit（R1CS 矩阵）+ witness (z/a/b/z_lincheck)

### Witness 格式
- z/a/b: `Vec<F128>`，BatchMajor 格式（或 RowMajor？需要确认）
- z_lincheck: `Vec<u8>`，byte-stripe 格式（8 blocks 的 u64 transpose）
- 矩阵维度：n_total × (2^K_LOG / 128) 个 F128 元素

### Ligerito floor
- MIN_DENSE_M = 22 → m_total ≥ 22 → n_blocks_log ≥ 12 for K_LOG=10
- `committed_words()` 被 `packed_len()` 截断，所以 floor 不自动生效
- 需要 n_blocks_log ≥ 12 使 packed_len ≥ 2^15

## 下一步（修复 wiring 对齐）

### 方案 A：搞清楚 builder 的 wiring 映射
需要研究：
1. `CircuitBuilder::finish()` → `CircuitShape` → `Circuit` 如何构建 wiring
2. `Registry::new()` 如何根据 TableType 的 k_log/io_schema 分配 slot offset
3. `CellSpace` 的 wiring permutation σ 如何把 cell 映射到 committed position
4. 然后让 `generate_witness()` 精确按这个映射填数据

关键文件：`flock-core/src/circuit/builder.rs`, `flock-core/src/schedule.rs`, `flock-core/src/circuit.rs`

### 方案 B：绕过 CircuitBuilder（更底层）
直接构建：
1. `BlockR1cs`（已有）
2. `Registry::new(vec![table_type], nu)`（用 gate 的 TableType）
3. `Circuit::new(registry, counts, ...)`（如果能手动构建）
4. `UnionInstance::new(&registry, counts)`
5. Witness 直接按 R1CS bit layout 填充

难点：`Circuit` 的构造函数可能是 private 的，需要检查。

### 方案 C：用 drive_witness_batch_major_partial（hash modules 的路径）
参考 `blake3::generate_witness_batch_major_partial` 和 `common::drive_witness_batch_major_partial`：
- 它们用 `BmRow = [u64; 8]` 格式
- per_group closure 写 z/a/b 到 BatchMajor 布局
- 这个函数是 `pub(crate)`，不能从外部 crate 调用
- 但可以复制其逻辑

## 构建命令
```bash
cd ~/workspace/binaryfield-zkvm
RUSTFLAGS="-C target-cpu=native" cargo run --release
```

## 依赖
- flock: `~/workspace/flock/crates/{flock-core,flock-prover}`
- features: `unsound-challenger`
- 新增依赖: `rayon`

## 参考会话
- M-A1 实现: binaryfield-zkvm 初始会话
- M-A2 实现: 当前会话（2026-09-01）
