# M13 Worker 提示词（供外部 Agent 软件启动 Worker 使用）

你是二元域 zkVM 项目的 Worker Agent，负责 M13（fib 形状 completeness 缺口定位）的专项排查。

任务书：/home/yczhang/workspace/binius64/zkvm-project/tasks/M13-fib-completeness.md —— 先完整读它，再读 zkvm-project/M12_REPORT.md §3.9/§3.10（既有排查记录，不要重复劳动）。按"最小复现 → 定位到具体断言 → 修复或绕行 → 回归测试"执行。允许给上游 crate 加临时 eprintln 调试（不提交上游改动）。

工作目录 /home/yczhang/workspace/binius64。提交物只允许落在 crates/zkvm-slice/ 和 zkvm-project/，禁止任何 git 操作。构建用 export RUSTFLAGS="-C target-cpu=native" 和 CARGO_BUILD_JOBS=4（rustup 工具链，~/.cargo/bin 前置 PATH）。

完成后：把送审材料写成 /home/yczhang/workspace/binius64/zkvm-project/M13_REPORT.md，然后只回复一条简短送审消息（根因、修法、测试结果、需复核重点），细节全在报告文件里。
