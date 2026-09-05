# Jolt ↔ Binius64 内存论证转译映射（后端替换可行性 · 第一手源码对照）

日期: 2026-09-05 | 来源: `workspace/jolt` (a16z, clone)，对照 `binius64/crates/zkvm-slice`

## 0. 结论先行
Jolt 的 RAM memory-checking **不是全局排序器**，而是
**one-hot addressing + write-increment + sumcheck 关系式**。它在代数语义上
与我们在二元域 logup* 上的内存论证（`mem_arg`/`mem_arg_ts`）**同构**——
这正是"后端替换 Jolt"在 memory-checking 这一环**语义等价、接口不同**的核心证据。

---

## 1. Jolt 的 RAM read/write-checking（模块化权威定义）

### 1.1 关系式（`jolt-claims/.../relations/ram/read_write_checking.rs`）
```
input:   ram_read_value + γ · ram_write_value          (来自 SpartanOuter)
output:  eq_cycle · ra · ( val + γ·(val + inc) )
          = eq_cycle · ra · val + γ · eq_cycle · ra · val + γ · eq_cycle · ra · inc
```
测试断言: `input = read + gamma*write`; `output = eq * ra * (val + gamma*(val+inc))`。degree=3。

### 1.1b 寄存器 read/write-checking（超集，更简单：K=128 固定寄存器数）
`relations/registers/read_write_checking.rs`：
```
input:  rd_write_value + γ·rs1_value + γ²·rs2_value
output: eq_cycle · [ rd_wa·(inc + val) + γ·rs1_ra·val + γ²·rs2_ra·val ]
```
- 涉及 3 个源寄存器（rs1/rs2）+ 目的寄存器（rd）：rs1_ra/rs2_ra（one-hot 读），rd_wa（写）。
- **与 RAM 同构**：`rd_inc`=目的寄存器增量，`registers_val`=写前值。
- `RegistersCycleMajorEntry`: `col`=寄存器索引(0..128)，`ra_coeff = γ·rs1_ra + γ²·rs2_ra`。

### 1.1c Jolt 前端 trace 契约（对接接口，全部域无关 u64）
`specs/proof-trace-row-layout.md` 定义 `JoltTraceRow` 访问器：
```
rs1_value / rs2_value / rd_pre_value / rd_write_value / ram_address
/ ram_read_value / ram_write_value
```
- 这些是 **u64 值**（域无关）——Jolt tracer 产出，可直接复用为二元域论证的输入。
- 内存行别名：LD 行 `ram_address=rs2, ram_read=rd_post`；SD 行 `ram_write=rs2, ram_read=rd_pre`。

### 1.2 语义（`geometry/ram.rs`）
| 标识 | 含义 | 类型 |
|---|---|---|
| `ram_ra(k,j)` | 地址 k 在周期 j 被访问 =1，否则 =0 | 虚拟多项式 (one-hot, K×T 稀疏矩阵) |
| `ram_val(k,j)` | 周期 j 之前地址 k 的值 | 虚拟多项式 |
| `ram_inc` | 该周期写入的**增量** (write-increment) | **已提交**多项式 (committed) |
| `ram_read_value` | Spartan outer 里声明的读值 | 开口 (opening) |
| `ram_write_value` | Spartan outer 里声明的写值 | 开口 |
| `eq_cycle` | cycle 变量 eq 多项式 (绑定周期) | derived |

### 1.3 数据结构（`subprotocols/read_write_matrix/{ram,address_major,cycle_major}.rs`）
- **`RamCycleMajorEntry`**: `row=cycle_index`(时间序), `col=remap_addr`(地址), 携带
  `prev_val`(写前值)/`next_val`(写后值)/`val_coeff`/`ra_coeff`。
- `RAMAccess::Write{pre_value,post_value,address}`；`Read{value,address}`。
- `AddressMajor`: 按 (col,row) 排序 → **每地址的访问序列**。
- 绑定: `bind_entries`(even/odd checkpoint) + `compute_evals`(inc_eval,eq_eval,gamma)。

---

## 2. 我们的二元域内存论证（`binius64/crates/zkvm-slice`）

| 切片 | 机制 | 证明的命题 |
|---|---|---|
| `mem_lookup` | logup* 查表 T[addr]=val | 单点读值正确（表手工给） |
| `mem_instr` | Spartan R1CS: `mem_r==mem_w` | 单地址 R-A-W |
| `mem_arg` ★ | logup* 多表: store+load **同锁一表 T** | **读⊆写** (sub-multiset on (addr,val)) |
| `mem_arg_ts` ★ | logup* **两表**: 写日志 W[(addr,ver)] + 读状态 T[addr]=最近写 | **同址多写·最近写判别** |

### 2.1 二元域实现的内存论证关系式（logup*）
```
∀ looker i:  (I*T)[index_i] = T[index_i] = claim_i
   store looker: T[addr] = store_value     （写进内存表）
   load  looker: T[addr] = load_value      （从内存表读）
   ⇒ load_value = T[addr] = store_value    （读 = 最近写）
```
logup* `verify_reduction` 在 transcript 的挑战 γ 上做**多重集合等式**验证。

---

## 3. 转译映射（Jolt 元素 → 二元域元素）

| Jolt (BN254 素域) | Binius64 (二元域 GF(2^128)) | 对应切片 | 状态 |
|---|---|---|---|
| `ram_ra` (one-hot) | logup* looker index (地址) | `mem_arg` | ✅ 已在二元域 |
| `ram_val` (写前值) | 内存表 T 的 store 值 | `mem_arg` | ✅ |
| `ram_inc` (写增量) | 版本序号 `ver` / R-A-W 门 | `mem_arg_ts` | ✅ |
| `ram_read_value`/`write_value` (Spartan outer) | load/store looker claim | `mem_arg`/`mem_arg_ts` | ✅ |
| sumcheck 关系式 (γ-fold) | logup* 挑战 γ 的多重集合等式 | (机制等价) | ✅ 语义同构 |
| `eq_cycle` 绑定 | 表 index 位置编码 | (机制等价) | ✅ |
| **sumcheck over sparse K×T matrix** | **logup* 通用查表** | — | ⚠️ **接口不同** |
| **commit (Dory)** | **Binius PCS** | — | ⚠️ 后端替换点 |

### 3.1 关键结论
- **语义上**：Jolt 的 `ra·(val + γ·(val+inc))` 与我们 logup* 的
  `store_claim + γ·load_claim ∈ T` 证明的是**同一个命题**：
  "load 读到的值 == 该地址最近一次 store 的值（读⊆写 + 最近写）"。
- **接口上**：Jolt 用**稀疏矩阵 sumcheck**（`bind_entries`/`compute_evals`），
  我们用 **logup* 查表**。两者是不同的"查找论证"后端，但都可表达该命题。

---

## 4. 后端替换的真正困难点（现在能量化）

1. **域**（最根本）：Jolt 全程 BN254 素域 + Dory PCS + 素域 sumcheck。切二元域 =
   **整层证明协议重写为 char-2**（sumcheck/PCS/multiset 全部二元化）。
2. **查找后端接口**：Jolt = sparse-matrix sumcheck / Shout(one-hot)；Binius64 = logup*。
   需一个**转译层**（把 Jolt 的 lookup 实例 → logup* TableLookup）。
3. **增量进位**：Jolt `inc` 素域原生；二元域需位分解+全加器链（`pc_carry`/`factory` 已验证）。
4. **前端可复用**：Jolt 的 executor/tracer + `Cycle` + RAMAccess 是**域无关**的，可保留。

### 4.1 复用 vs 重写
| 层 | 能否复用 Jolt | 处置 |
|---|---|---|
| executor / tracer / `Cycle` | ✅ 域无关 | **保留** |
| `RAMAccess` / `MemoryLayout` / `remap_address` | ✅ 域无关 | **保留** |
| memory-checking 数据组织 (address_major) | ✅ 值域无关 | **保留/翻译** |
| **证明后端 (sumcheck/PCS/multiset)** | ❌ 素域耦合 | **替换为二元域** (用 logup*+我们的论证) |

---

## 5. 行动建议
- **优先**：把 Jolt 的 `read_write_checking` 关系式作为**参考规范**，对照验证
  `mem_arg_ts` 语义等价（已基本确认同构）。
- **其次**：将 Jolt 前端 (executor+Cycle+RAMAccess) 接上二元域 logup* 内存论证，
  产出第一个"Jolt 前端 + Binius64 后端"的内存一致性证明。
- **最后**：评估 sumcheck 层二元化（Jolt sparse-matrix sumcheck vs Binius logup*），
  这是最大工程，但 memory-checking 的语义已对口。
