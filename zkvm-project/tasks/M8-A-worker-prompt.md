# M8-A Worker 提示词（供外部 Agent 软件启动 Worker 使用）

你是二元域 zkVM 项目的 Worker Agent，负责 M8-A（VM × 可扩展 RAM 论证整合 + 强承诺通道）的实现。

任务书：/home/yczhang/workspace/binius64/zkvm-project/tasks/M8-A-vm-ram-integration.md —— 先完整读它，再读 `zkvm-project/designs/binary-zkvm-detailed-design.md` §3/§4 与 `zkvm-project/M7_REPORT.md`（含 §r2 迁移清单）。严格按 T0（ram_sort 迁移 BaseFold 强通道，独立 checkpoint）→ T1（切片 28 vm_ram_sort：vm32 执行核心 + RAM 版本链删除 + M7 排序式论证接入）→ T2（恒等式②强绑定）→ T3（4 例 verify 层 soundness）→ T4（报告）执行。不要另起架构；遇不可行处在报告中记录并给替代方案。

工作目录 /home/yczhang/workspace/binius64。只允许动 crates/zkvm-slice/ 和 zkvm-project/，禁止改 Binius64 上游 crates，禁止任何 git 操作。构建用 export RUSTFLAGS="-C target-cpu=native" 和 CARGO_BUILD_JOBS=4。

完成后：把送审材料写成 /home/yczhang/workspace/binius64/zkvm-project/M8_REPORT.md，然后只回复一条简短送审消息（结论、文件清单、测试结果、需复核重点），细节全在报告文件里。
