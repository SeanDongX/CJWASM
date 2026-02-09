#!/usr/bin/env bash
# 运行测试覆盖率。若未设置 LLVM_COV/LLVM_PROFDATA，则尝试从 rustup 工具链中查找。

set -e
cd "$(dirname "$0")/.."

if [[ -z "$LLVM_COV" || -z "$LLVM_PROFDATA" ]]; then
  BIN_DIR=$(find "$HOME/.rustup/toolchains" -path '*/bin/llvm-cov' 2>/dev/null | head -1 | xargs dirname 2>/dev/null)
  if [[ -n "$BIN_DIR" ]]; then
    export LLVM_COV="$BIN_DIR/llvm-cov"
    export LLVM_PROFDATA="$BIN_DIR/llvm-profdata"
  fi
fi

if [[ "$1" == "--html" ]]; then
  cargo llvm-cov --all-features --html
  echo "HTML 报告: target/llvm-cov/html/index.html"
else
  cargo llvm-cov --all-features "$@"
fi
