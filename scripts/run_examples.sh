#!/usr/bin/env bash
# ============================================================
# 编译并运行 examples/ 下所有 .cj 示例文件
#
# 用法:
#   ./scripts/run_examples.sh            # 编译并运行所有示例
#   ./scripts/run_examples.sh --compile  # 仅编译，不运行
#   ./scripts/run_examples.sh --verbose  # 显示详细输出
#   ./scripts/run_examples.sh hello.cj   # 仅编译运行指定文件
# ============================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
EXAMPLES_DIR="$PROJECT_DIR/examples"
OUT_DIR="$PROJECT_DIR/target/examples"

# 颜色
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

# 默认参数
COMPILE_ONLY=false
VERBOSE=false
FILTER=""

# 解析参数
for arg in "$@"; do
  case "$arg" in
    --compile)  COMPILE_ONLY=true ;;
    --verbose)  VERBOSE=true ;;
    -h|--help)
      echo "用法: $0 [选项] [文件名...]"
      echo ""
      echo "选项:"
      echo "  --compile   仅编译，不运行 WASM"
      echo "  --verbose   显示详细输出（编译器警告、WASM 返回值等）"
      echo "  -h, --help  显示帮助"
      echo ""
      echo "示例:"
      echo "  $0                    # 编译并运行所有示例"
      echo "  $0 hello.cj math.cj  # 仅编译运行指定文件"
      echo "  $0 --compile          # 仅编译所有示例"
      exit 0
      ;;
    *)
      if [[ "$arg" == *.cj ]]; then
        FILTER="$FILTER $arg"
      else
        echo -e "${RED}未知参数: $arg${NC}" >&2
        exit 1
      fi
      ;;
  esac
done

# ── 准备 ──────────────────────────────────────────────────────

# 构建编译器（release 模式）
echo -e "${BOLD}${CYAN}═══ CJWasm 示例编译运行器 ═══${NC}"
echo ""

echo -e "${CYAN}[1/3] 构建编译器...${NC}"
cargo build --release --manifest-path "$PROJECT_DIR/Cargo.toml" 2>&1 | {
  if $VERBOSE; then cat; else cat > /dev/null; fi
}
CJWASM="$PROJECT_DIR/target/release/cjwasm"

if [[ ! -x "$CJWASM" ]]; then
  echo -e "${RED}错误: 编译器构建失败，$CJWASM 不存在${NC}" >&2
  exit 1
fi

# 检查 wasmtime
HAS_WASMTIME=false
if command -v wasmtime &>/dev/null; then
  HAS_WASMTIME=true
fi

if ! $COMPILE_ONLY && ! $HAS_WASMTIME; then
  echo -e "${YELLOW}警告: 未安装 wasmtime，将仅编译不运行${NC}"
  echo -e "${YELLOW}  安装: brew install wasmtime${NC}"
  COMPILE_ONLY=true
fi

# 创建输出目录
mkdir -p "$OUT_DIR"

# ── 收集文件 ──────────────────────────────────────────────────

if [[ -n "$FILTER" ]]; then
  FILES=()
  for f in $FILTER; do
    full="$EXAMPLES_DIR/$f"
    if [[ -f "$full" ]]; then
      FILES+=("$full")
    else
      echo -e "${RED}文件不存在: $full${NC}" >&2
      exit 1
    fi
  done
else
  # 收集所有 .cj 文件（不含子目录中的文件，multifile 需特殊处理）
  FILES=()
  while IFS= read -r f; do
    FILES+=("$f")
  done < <(find "$EXAMPLES_DIR" -maxdepth 1 -name '*.cj' | sort)
fi

TOTAL=${#FILES[@]}
if [[ $TOTAL -eq 0 ]]; then
  echo -e "${YELLOW}未找到 .cj 示例文件${NC}"
  exit 0
fi

echo -e "${CYAN}[2/3] 找到 ${TOTAL} 个示例文件${NC}"
echo ""

# ── 编译并运行 ────────────────────────────────────────────────

PASS=0
FAIL=0
SKIP=0
ERRORS=()

printf "${BOLD}%-30s %-12s %-12s %s${NC}\n" "文件" "编译" "运行" "返回值"
printf "%-30s %-12s %-12s %s\n"   "──────────────────────────────" "────────────" "────────────" "──────"

for filepath in "${FILES[@]}"; do
  filename=$(basename "$filepath")
  name="${filename%.cj}"
  wasm_file="$OUT_DIR/${name}.wasm"

  # ── 编译 ──
  compile_output=""
  compile_ok=false
  if compile_output=$("$CJWASM" "$filepath" -o "$wasm_file" 2>&1); then
    compile_ok=true
  fi

  if $compile_ok; then
    compile_status="${GREEN}✓ 成功${NC}"
  else
    compile_status="${RED}✗ 失败${NC}"
    ((FAIL++)) || true
    ERRORS+=("$filename (编译失败)")
    printf "%-30s ${RED}✗ 编译失败${NC}\n" "$filename"
    if $VERBOSE && [[ -n "$compile_output" ]]; then
      echo -e "  ${RED}$compile_output${NC}"
    fi
    continue
  fi

  # ── 运行 ──
  if $COMPILE_ONLY; then
    printf "%-30s ${GREEN}✓ 编译成功${NC}   ${YELLOW}— 跳过${NC}\n" "$filename"
    ((PASS++)) || true
    continue
  fi

  run_output=""
  exit_code=0
  # wasmtime 运行，--invoke main 调用入口函数（10 秒超时防止死循环）
  run_output=$(wasmtime run -W timeout=10s --invoke main "$wasm_file" 2>&1) || exit_code=$?

  if [[ $exit_code -eq 0 ]]; then
    run_status="${GREEN}✓ 成功${NC}"
    return_val="${run_output##*$'\n'}"  # 最后一行为返回值
    printf "%-30s ${GREEN}✓ 编译${NC}       ${GREEN}✓ 运行${NC}       %s\n" "$filename" "$return_val"
    ((PASS++)) || true
  else
    # wasmtime 返回非零可能是正常的（main 返回非零），也可能是真错误
    # 检查是否有 trap 或 error
    if echo "$run_output" | grep -qi "error\|trap\|unreachable\|timed out\|failed to"; then
      # 提取简短错误描述
      short_err=""
      if echo "$run_output" | grep -qi "timed out"; then
        short_err="超时"
      elif echo "$run_output" | grep -qi "failed to compile\|translation error\|type mismatch"; then
        short_err="WASM 无效"
      elif echo "$run_output" | grep -qi "unreachable"; then
        short_err="unreachable"
      elif echo "$run_output" | grep -qi "trap"; then
        short_err="trap"
      else
        short_err="exit=$exit_code"
      fi
      ((FAIL++)) || true
      ERRORS+=("$filename ($short_err)")
      printf "%-30s ${GREEN}✓ 编译${NC}       ${RED}✗ %-10s${NC}\n" "$filename" "$short_err"
      if $VERBOSE; then
        # 仅显示前 3 行错误
        echo "$run_output" | head -3 | while IFS= read -r line; do
          echo -e "  ${RED}$line${NC}"
        done
      fi
    else
      # 非零返回值但无错误，视为正常（main 返回非零数值）
      return_val="${run_output##*$'\n'}"
      printf "%-30s ${GREEN}✓ 编译${NC}       ${GREEN}✓ 运行${NC}       %s\n" "$filename" "$return_val"
      ((PASS++)) || true
    fi
  fi

  if $VERBOSE && [[ -n "$compile_output" ]]; then
    echo -e "  ${YELLOW}编译器输出: $compile_output${NC}"
  fi
done

# ── 多文件编译（multifile 目录）──────────────────────────────

MULTIFILE_DIR="$EXAMPLES_DIR/multifile"
if [[ -d "$MULTIFILE_DIR" && -z "$FILTER" ]]; then
  echo ""
  echo -e "${CYAN}[附加] 多文件编译: examples/multifile/${NC}"

  main_file="$MULTIFILE_DIR/module_main.cj"
  if [[ -f "$main_file" ]]; then
    wasm_file="$OUT_DIR/multifile.wasm"
    # 收集 multifile 目录下所有 .cj 文件
    multi_files=$(find "$MULTIFILE_DIR" -name '*.cj' | sort | tr '\n' ' ')

    if compile_output=$($CJWASM $multi_files -o "$wasm_file" 2>&1); then
      printf "%-30s ${GREEN}✓ 编译${NC}" "multifile/"
      if ! $COMPILE_ONLY && $HAS_WASMTIME; then
        run_output=$(wasmtime run -W timeout=10s --invoke main "$wasm_file" 2>&1) || true
        return_val="${run_output##*$'\n'}"
        printf "       ${GREEN}✓ 运行${NC}       %s\n" "$return_val"
      else
        printf "       ${YELLOW}— 跳过${NC}\n"
      fi
      ((PASS++)) || true
    else
      printf "%-30s ${RED}✗ 编译失败${NC}\n" "multifile/"
      ((FAIL++)) || true
      ERRORS+=("multifile/ (编译失败)")
      if $VERBOSE; then
        echo -e "  ${RED}$compile_output${NC}"
      fi
    fi
  fi
fi

# ── 汇总 ──────────────────────────────────────────────────────

echo ""
echo -e "${BOLD}${CYAN}═══ 结果汇总 ═══${NC}"
echo -e "  ${GREEN}通过: $PASS${NC}"
if [[ $FAIL -gt 0 ]]; then
  echo -e "  ${RED}失败: $FAIL${NC}"
  echo ""
  echo -e "${RED}失败详情:${NC}"
  for err in "${ERRORS[@]}"; do
    echo -e "  ${RED}• $err${NC}"
  done
else
  echo -e "  ${RED}失败: 0${NC}"
fi
echo ""

if [[ $FAIL -gt 0 ]]; then
  echo -e "${RED}${BOLD}部分示例编译或运行失败！${NC}"
  exit 1
else
  echo -e "${GREEN}${BOLD}所有示例编译运行成功！${NC}"
  if ! $COMPILE_ONLY; then
    echo -e "WASM 文件输出: ${CYAN}$OUT_DIR/${NC}"
  fi
  exit 0
fi
