# M6 T3：每指令成本基准（thesis 定量证据）

> 日期：2026-09-07。构建 profile：**debug**（`RUSTFLAGS="-C target-cpu=native"`）。机器：i5-12400F（AVX2），32GB。
> 方法：每条 RV32I 指令构造 N=16 次重复的微程序（固定操作数，非平凡位型：rs1=0x80000000、
> rs2=0x7FFFFFFF、shamt=9、imm=0x1FF、分支全部配置为 not-taken 保持线性、jal 链目标 +4、
> jalr 用 `addi x1,x1,8 / jalr x3,x1,0` 对实现线性间接跳转链），经 `vm32::run_machine_full`
> 全门级 prove + 三表 logup* + verify（全部 `c_ok=true l_ok=true`），记录 CircuitStat 与耗时。
> 复现：`cargo test -p binius-zkvm-slice --lib -- --ignored --nocapture bench_instruction`。

## M9：release 缩放曲线（thesis 最终定量证据，2026-09-08）

> `--release` + RUSTFLAGS=native，串行单进程。复现：`cargo test --release -p binius-zkvm-slice
> --lib vm_ram_sort -- --include-ignored --nocapture --test-threads=1`。

| N | T（周期） | gates | prove 时间 | g/cyc | 峰值内存 |
|---|---|---|---|---|---|
| 16 | 1,801 | 905,168 | 1.1s | 502.6 | — |
| 32 | 6,973 | 3,502,632 | 4.5s | 502.3 | — |
| 64 | 27,343 | 13,730,612 | 18.1s | 502.2 | 8.88 GB |

- **gates 严格线性**（502.3±0.2 门/周期，方差 0.04%）；prove_time 拟合指数 ≈1.03（近线性）。
- 每指令成本 release 版：31 条指令 **1113-1115 g/cyc 统一**（M6 的 973 + M8-B/M9 的
  M 族断言与 sb/sh 每周期固定门）——成本与指令类型无关在 39 条 ISA 下成立。
- 阶段 profile（release N=32）：build_circuit 2.88s（64%）＞ frontend_prove 0.50s（11%）
  ＞ BaseFold/logup/fracadd 72ms ＞ witness_fill 12ms——瓶颈在电路构建（详见 M9_REPORT T3）。

## M8-B 增补（2026-09-08）：新增 ISA 行 + fetch/桥的公开列开销

> 复现：`cargo test -p binius-zkvm-slice --lib word_vm32_m8b -- --nocapture`。
> 新指令（mul/div/divu/rem/remu/lb/lbu/lh/lhu）暂未进 per-inst 基准循环；当前端到端证据：
> **M8-B ISA 微程序（9 指令混合，21 cyc）：gates=23,368（imul=63, and=8,604），c_ok/l_ok 全绿**。
> M 族断言的 imul 门为**每周期固定成本**（div 族验证序列 2 个 imul/周期 + mul 1 个/周期），
> 与指令是否出现无关（全展开 mux 结构，同 M5 惯例）。
> vm_ram_sort（N=16）在 T0/T1 公开化后：inout 1 词 → ~18.5k 词（inst/pc/排序流 8 列），
> gates 905,168（+fetch logup/oracle relation 开销 <5%），honest prove 1.9s。

## 每指令成本表（真实运行输出，2026-09-07）

```
inst      cyc    gates     zero    and      imul   bmul     g/cyc    t(us)
add        cyc=23   gates=22388     zero=542     and=8221     imul=0      bmul=10411    g/cyc=973     t=142268us
sub        cyc=23   gates=22388     zero=542     and=8221     imul=0      bmul=10411    g/cyc=973     t=130490us
sll        cyc=23   gates=22388     zero=542     and=8221     imul=0      bmul=10411    g/cyc=973     t=130727us
slt        cyc=23   gates=22388     zero=542     and=8221     imul=0      bmul=10411    g/cyc=973     t=131919us
sltu       cyc=23   gates=22388     zero=542     and=8221     imul=0      bmul=10411    g/cyc=973     t=129480us
xor        cyc=23   gates=22388     zero=542     and=8221     imul=0      bmul=10411    g/cyc=973     t=130953us
srl        cyc=23   gates=22388     zero=542     and=8221     imul=0      bmul=10411    g/cyc=973     t=132052us
sra        cyc=23   gates=22388     zero=542     and=8221     imul=0      bmul=10411    g/cyc=973     t=131843us
or         cyc=23   gates=22388     zero=542     and=8221     imul=0      bmul=10411    g/cyc=973     t=133104us
and        cyc=23   gates=22388     zero=542     and=8221     imul=0      bmul=10411    g/cyc=973     t=138609us
addi       cyc=23   gates=22388     zero=542     and=8221     imul=0      bmul=10411    g/cyc=973     t=175119us
slti       cyc=23   gates=22388     zero=542     and=8221     imul=0      bmul=10411    g/cyc=973     t=229759us
sltiu      cyc=23   gates=22388     zero=542     and=8221     imul=0      bmul=10411    g/cyc=973     t=189737us
xori       cyc=23   gates=22388     zero=542     and=8221     imul=0      bmul=10411    g/cyc=973     t=215706us
ori        cyc=23   gates=22388     zero=542     and=8221     imul=0      bmul=10411    g/cyc=973     t=146941us
andi       cyc=23   gates=22388     zero=542     and=8221     imul=0      bmul=10411    g/cyc=973     t=136954us
slli       cyc=23   gates=22388     zero=542     and=8221     imul=0      bmul=10411    g/cyc=973     t=146259us
srli       cyc=23   gates=22388     zero=542     and=8221     imul=0      bmul=10411    g/cyc=973     t=138397us
srai       cyc=23   gates=22388     zero=542     and=8221     imul=0      bmul=10411    g/cyc=973     t=141948us
lui        cyc=23   gates=22388     zero=542     and=8221     imul=0      bmul=10411    g/cyc=973     t=144108us
auipc      cyc=23   gates=22388     zero=542     and=8221     imul=0      bmul=10411    g/cyc=973     t=138084us
jal        cyc=23   gates=22388     zero=542     and=8221     imul=0      bmul=10411    g/cyc=973     t=137949us
jalr       cyc=39   gates=38052     zero=830     and=14029    imul=0      bmul=17835    g/cyc=975     t=166845us
beq        cyc=23   gates=22388     zero=542     and=8221     imul=0      bmul=10411    g/cyc=973     t=133162us
bne        cyc=23   gates=22388     zero=542     and=8221     imul=0      bmul=10411    g/cyc=973     t=138737us
blt        cyc=23   gates=22388     zero=542     and=8221     imul=0      bmul=10411    g/cyc=973     t=132430us
bge        cyc=23   gates=22388     zero=542     and=8221     imul=0      bmul=10411    g/cyc=973     t=133842us
bltu       cyc=23   gates=22388     zero=542     and=8221     imul=0      bmul=10411    g/cyc=973     t=139040us
bgeu       cyc=23   gates=22388     zero=542     and=8221     imul=0      bmul=10411    g/cyc=973     t=132343us
lw         cyc=23   gates=22388     zero=542     and=8221     imul=0      bmul=10411    g/cyc=973     t=143665us
sw         cyc=23   gates=22388     zero=542     and=8221     imul=0      bmul=10411    g/cyc=973     t=133672us
```

## Thesis 结论（定量）

- **30/31 条指令逐数字完全相同**：`gates=22388, zero=542, AND=8221, IMUL=0, BMUL=10411, g/cyc=973`。
  覆盖 ALU（R/I）、移位、比较（signed/unsigned）、立即数、lui/auipc、jal、6 类分支、lw/sw——
  **每周期约束成本与指令类型无关**（thesis"成本∝指令数、与指令类型无关"的直接证据）。
- **IMUL=0**：32 位版本链与值链全部落在 binfield 门（AND/BMUL），无整数乘法约束。
- **jalr (975 g/cyc)**：微程序为 `addi x1,x1,8 / jalr x3,x1,0` 对（寄存器间接跳转无法用静态
  立即数构线性链——目标寄存器无关），jalr 周期成本本身与其它指令同量级；表中 jalr 行含 addi 混合，
  口径已在代码注释与本节标注。
- **耗时**：debug profile 单微程序证明 130-230ms（addi/slti 等稍高为机器噪声），30 条微程序总
  prove 时间 ≈ 4.5s。

## 分量口径（链底噪 vs 译码+执行增量）

- 每周期 973 门的结构分量（不可在线分离，口径说明）：
  1. **版本链底噪**（占绝对主项）：32 寄存器 ×（值+版本）mux/递增 + 64 字 RAM 版本链 + 事件钉扎
     —— 由 VER_MAX=128（M5-R5 放大）与 O((32+64)·T) 版本链设计决定，与指令类型无关；
  2. **译码+执行增量**：解码器（funct3 mux8、立即数符号扩展、is_sub/is_sra 等标记）与 ALU 主体。
- **可分离证据**：30 条指令同周期同门数 ⇒ 增量项对全指令族恒等（版本链主导，译码增量在 973 门中的
  差异 <1 门——即每条不同指令的"额外"门数 ≤ 采样分辨率）。要严格分离需对照"空载周期"（如纯补丁
  nop）微程序，留作 M7 可选工作，报告如实标注。
- prove/verify 耗时与 gates 数线性相关（t(us) 与指令类型无关的直观佐证）。

## 参照系

- torture（50 周期混合程序）：48732 gates（ZERO=1028 AND=18022 IMUL=0 BMUL=22939，含
  M5-R5 store 地址修复后的 imm_s/select 开销；M5 v2 报 44,784 为 VER_MAX=16 时代旧值）。
- bubblesort（391 周期）：382,660 gates，g/cyc≈979（与微程序 973 同量级——torture 的
  lw/sw+分支混合也几乎相等，进一步佐证与指令类型无关）。