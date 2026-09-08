#!/usr/bin/env bash
# M10 T2：zkvm-slice 测试入口（CI 可复跑）。
# 用法：
#   tools/run_zkvm_tests.sh            # 默认：debug 全量（快测试）+ 大测试串行段
#   tools/run_zkvm_tests.sh release    # release 全量（缩放/基准口径）
#   QUICK=1 tools/run_zkvm_tests.sh    # 只跑快测试（跳过 #[ignore] 大测试）
# 内存护栏：N>=64 的测试（scale64）峰值 ~9GB——串行段用 --test-threads=1，且编译与
# 测试运行不并发（先 build 再 test）；内存 <12GB 的机器请跳过大测试段（QUICK=1）。
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"          # rustup 工具链（系统 cargo 1.75 不支持 edition2024）
export RUSTFLAGS="-C target-cpu=native"
export CARGO_BUILD_JOBS=4                     # 防 OOM（12 核/32GB 实测约束）
PROFILE="${1:-debug}"
FLAG=""; [ "$PROFILE" = "release" ] && FLAG="--release"

echo "== [1/3] build ($PROFILE) =="
cargo build -p binius-zkvm-slice $FLAG --tests

echo "== [2/3] quick tests =="
cargo test -p binius-zkvm-slice $FLAG --lib -- --test-threads=1

if [ "${QUICK:-0}" = "1" ]; then
  echo "== [3/3] large tests skipped (QUICK=1) =="
else
  echo "== [3/3] large tests (serial; N=64 peaks ~9GB) =="
  cargo test -p binius-zkvm-slice $FLAG --lib -- --ignored --nocapture --test-threads=1
fi
echo "== ALL GREEN =="
