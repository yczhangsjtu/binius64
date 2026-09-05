#!/usr/bin/env python3
"""Batch-migrate slice binaries to crate modules.

For each src/bin/<stem>.rs:
  1. `fn main()`  -> `pub fn run_<stem>()`
  2. remove local copies of to_bits/fa/add_constant/inc8/mul8/leq8/native_xor
  3. add `use crate::alu::*;` after the last use block
  4. append a `#[cfg(test)] mod tests` with `#[test] fn <stem>` calling run_<stem>
"""
import re, os, sys, glob

SRC = "src/bin"
STOP_FNS = {"to_bits", "fa", "add_constant", "inc8", "mul8", "leq8", "native_xor", "native_xori"}

def parse_fn_sig_end(s, start):
    """Given `fn x` appearing at s.index('fn', start), find the end of the fn
    signature (the opening brace) and then the matching closing brace."""
    i = s.index("fn", start)
    # find first '{' that starts the body (not inside type generics — we scan for brace depth from '{')
    brace = s.index("{", i)
    depth = 0
    for j in range(brace, len(s)):
        if s[j] == "{":
            depth += 1
        elif s[j] == "}":
            depth -= 1
            if depth == 0:
                return j + 1  # position after closing brace
    raise ValueError("no matching brace")

def strip_fns(text, exempt=()):
    """Remove STOP_FNS function definitions that start at line start (fn ...).
    `exempt` is a set of stems whose file-local helpers are kept (e.g. pc_carry)."""
    # A def is a line that (after optional leading ws) starts with `fn <stop> <...>` or `fn <stop><...>`
    pattern = re.compile(r"^fn\s+(" + "|".join(STOP_FNS) + r")\b")
    lines = text.split("\n")
    out = []
    i = 0
    while i < len(lines):
        stripped = lines[i].lstrip()
        m = pattern.match(stripped)
        if m:
            # this line starts a fn to remove; find its end by reconstructing till matching brace
            # join from this line onward, parse, and skip.
            seg = "\n".join(lines[i:])
            end = parse_fn_sig_end(seg, 0)
            # count lines consumed = end position in seg
            consumed = seg[:end].count("\n")
            # account: if end lands mid-line, include that line
            i += consumed + (0 if seg[:end].endswith("\n") else 1)
            continue
        out.append(lines[i])
        i += 1
    return "\n".join(out)

def add_use_crate_alu(text):
    """Insert `use crate::alu::*;` at top (after any leading doc comments)."""
    lines = text.split("\n")
    # find first non-comment, non-empty code line
    idx = 0
    while idx < len(lines) and (lines[idx].strip().startswith("//") or not lines[idx].strip()):
        idx += 1
    lines.insert(idx, "use crate::alu::*;\n")
    return "\n".join(lines)

def main():
    files = sorted(glob.glob(os.path.join(SRC, "*.rs")))
    for f in files:
        stem = os.path.splitext(os.path.basename(f))[0]
        seg = open(f).read()
        # guard: skip if already migrated
        if f"fn run_{stem}" in seg:
            print(f"skip {stem} (already migrated)")
            continue
        # pc_carry uses array to_bits([B128;BITS]) + full_adder (not `fa`); keep its
        # local helpers and do NOT inject crate::alu::* (name clash with Vec to_bits).
        keep_local = (stem == "pc_carry")
        # 1. fn main() -> pub fn run_<stem>()
        new = seg.replace("fn main()", f"pub fn run_{stem}()")
        # 2. strip local duplicate helpers
        new = strip_fns(new, exempt=([stem] if keep_local else ()))
        # 3. add use crate::alu::*;
        if not keep_local:
            new = add_use_crate_alu(new)
        # 4. append test module
        new = new.rstrip() + "\n\n#[cfg(test)]\nmod tests {\n\tsuper::*;\n\t#[test]\n\tfn " + stem + "() {\n\t\trun_" + stem + "();\n\t}\n}\n"
        open(f, "w").write(new)
        print(f"migrated {stem}")

if __name__ == "__main__":
    main()
