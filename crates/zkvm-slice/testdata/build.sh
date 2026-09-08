#!/usr/bin/env bash
# M8-C T1: build the C test programs to RV32 ELF (reproducible artifacts).
# Toolchain: riscv64-unknown-elf-gcc 13.2.0 (multilib front-end, -march=rv32im).
set -euo pipefail
cd "$(dirname "$0")"
CFLAGS="-march=rv32im -mabi=ilp32 -nostdlib -nostartfiles -ffreestanding \
  -fno-builtin -O1 -Wl,-Ttext=0x0,--build-id=none -e _start"
for src in bubble16.c fib.c; do
  [ -f "$src" ] || continue
  out="${src%.c}.elf"
  riscv64-unknown-elf-gcc $CFLAGS -o "$out" "$src"
  riscv64-unknown-elf-objdump -d "$out" > "${src%.c}.dis" 2>/dev/null || true
  echo "built $out"
done
