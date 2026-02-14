#!/usr/bin/env bash
# ============================================================
# CJWasm 系统测试
#
# 编译并运行 examples/ 下所有 .cj 示例文件，
# 自动从源文件中提取 "// 预期输出: <value>" 注释，
# 与 wasmtime 实际运行的返回值进行比较验证。
#
# 用法:
#   ./scripts/system_test.sh              # 运行所有示例测试
#   ./scripts/system_test.sh hello.cj     # 仅测试指定文件
#   ./scripts/system_test.sh --verbose    # 显示详细输出
#   ./scripts/system_test.sh --no-build   # 跳过编译器构建
# ============================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
EXAMPLES_DIR="$PROJECT_DIR/examples"
OUT_DIR="$PROJECT_DIR/target/examples"

# ── 颜色 ──
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
DIM='\033[2m'
NC='\033[0m'

# ── 参数 ──
VERBOSE=false
NO_BUILD=false
FILTER=""

for arg in "$@"; do
  case "$arg" in
    --verbose|-v)  VERBOSE=true ;;
    --no-build)    NO_BUILD=true ;;
    -h|--help)
      echo "用法: $0 [选项] [文件名...]"
      echo ""
      echo "系统测试：编译运行 .cj 文件并验证返回值与预期是否一致。"
      echo "预期值从源文件中 '// 预期输出: <value>' 注释提取。"
      echo ""
      echo "选项:"
      echo "  --verbose, -v  显示详细输出（编译器输出、WASM 运行输出等）"
      echo "  --no-build     跳过编译器构建（假定已构建）"
      echo "  -h, --help     显示帮助"
      echo ""
      echo "示例:"
      echo "  $0                    # 运行所有示例测试"
      echo "  $0 hello.cj math.cj  # 仅测试指定文件"
      echo "  $0 --verbose          # 显示详细信息"
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

# ── 准备 ──

echo -e "${BOLD}${CYAN}═══ CJWasm 系统测试 ═══${NC}"
echo ""

# 构建编译器
if ! $NO_BUILD; then
  echo -e "${CYAN}[1/3] 构建编译器 (release)...${NC}"
  build_output=$(cargo build --release --manifest-path "$PROJECT_DIR/Cargo.toml" 2>&1) || {
    echo -e "${RED}编译器构建失败！${NC}"
    echo "$build_output"
    exit 1
  }
  if $VERBOSE && [[ -n "$build_output" ]]; then
    echo -e "${DIM}$build_output${NC}"
  fi
else
  echo -e "${CYAN}[1/3] 跳过编译器构建 (--no-build)${NC}"
fi

CJWASM="$PROJECT_DIR/target/release/cjwasm"
if [[ ! -x "$CJWASM" ]]; then
  echo -e "${RED}错误: 编译器不存在: $CJWASM${NC}" >&2
  exit 1
fi

# 检查 wasmtime
if ! command -v wasmtime &>/dev/null; then
  echo -e "${RED}错误: 未安装 wasmtime，无法运行系统测试${NC}" >&2
  echo -e "${YELLOW}  安装: brew install wasmtime${NC}" >&2
  exit 1
fi

mkdir -p "$OUT_DIR"

# ── 收集文件 ──

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

# ── 运行测试 ──

echo -e "${CYAN}[3/3] 编译、运行并验证...${NC}"
echo ""

PASS=0
FAIL=0
SKIP=0
ERRORS=()

# 表头
printf "${BOLD}%-30s %-10s %-14s %-14s %s${NC}\n" "文件" "编译" "预期" "实际" "结果"
printf "%-30s %-10s %-14s %-14s %s\n" \
  "──────────────────────────────" "──────────" "──────────────" "──────────────" "──────"

for filepath in "${FILES[@]}"; do
  filename=$(basename "$filepath")
  name="${filename%.cj}"
  wasm_file="$OUT_DIR/${name}.wasm"

  # ── 提取预期输出 ──
  expected=""
  if grep -q '预期输出' "$filepath"; then
    expected=$(grep '预期输出' "$filepath" | tail -1 | grep -oE '[-]?[0-9]+' | tail -1)
  fi

  # ── 编译 ──
  compile_output=""
  compile_ok=false
  if compile_output=$("$CJWASM" "$filepath" -o "$wasm_file" 2>&1); then
    compile_ok=true
  fi

  if ! $compile_ok; then
    ((FAIL++)) || true
    ERRORS+=("$filename: 编译失败")
    printf "%-30s ${RED}✗ 失败${NC}     %-14s %-14s ${RED}✗ FAIL${NC}\n" "$filename" "${expected:-?}" "-"
    if $VERBOSE && [[ -n "$compile_output" ]]; then
      echo -e "  ${DIM}${RED}$compile_output${NC}"
    fi
    continue
  fi

  if $VERBOSE && [[ -n "$compile_output" ]]; then
    echo -e "  ${DIM}$compile_output${NC}"
  fi

  # ── 运行 ──
  run_output=""
  exit_code=0
  run_output=$(wasmtime run -W timeout=10s --invoke main "$wasm_file" 2>&1) || exit_code=$?

  # 提取返回值（最后一行）
  actual=""
  is_error=false

  if echo "$run_output" | grep -qi "error\|trap\|unreachable\|timed out\|failed to"; then
    is_error=true
    if echo "$run_output" | grep -qi "timed out"; then
      actual="TIMEOUT"
    elif echo "$run_output" | grep -qi "unreachable"; then
      actual="UNREACHABLE"
    elif echo "$run_output" | grep -qi "trap"; then
      actual="TRAP"
    else
      actual="ERROR"
    fi
  else
    # 从输出中提取最后一行作为返回值
    actual=$(echo "$run_output" | tail -1 | tr -d '[:space:]')
  fi

  # ── 验证 ──
  if $is_error; then
    ((FAIL++)) || true
    ERRORS+=("$filename: 运行错误 ($actual)")
    printf "%-30s ${GREEN}✓${NC}          %-14s ${RED}%-14s${NC} ${RED}✗ FAIL${NC}\n" "$filename" "${expected:-?}" "$actual"
    if $VERBOSE; then
      echo "$run_output" | head -3 | while IFS= read -r line; do
        echo -e "  ${DIM}${RED}$line${NC}"
      done
    fi
  elif [[ -z "$expected" ]]; then
    # 没有预期值，跳过验证
    ((SKIP++)) || true
    printf "%-30s ${GREEN}✓${NC}          ${DIM}%-14s${NC} %-14s ${YELLOW}○ SKIP${NC}\n" "$filename" "无预期值" "$actual"
  elif [[ "$actual" == "$expected" ]]; then
    ((PASS++)) || true
    printf "%-30s ${GREEN}✓${NC}          %-14s %-14s ${GREEN}✓ PASS${NC}\n" "$filename" "$expected" "$actual"
  else
    ((FAIL++)) || true
    ERRORS+=("$filename: 预期=$expected 实际=$actual")
    printf "%-30s ${GREEN}✓${NC}          %-14s ${RED}%-14s${NC} ${RED}✗ FAIL${NC}\n" "$filename" "$expected" "$actual"
  fi
done

# ── 多文件测试 ──

MULTIFILE_DIR="$EXAMPLES_DIR/multifile"
if [[ -d "$MULTIFILE_DIR" && -z "$FILTER" ]]; then
  echo ""
  echo -e "${CYAN}[附加] 多文件编译测试: examples/multifile/${NC}"

  main_file="$MULTIFILE_DIR/module_main.cj"
  if [[ -f "$main_file" ]]; then
    wasm_file="$OUT_DIR/multifile.wasm"
    multi_files=$(find "$MULTIFILE_DIR" -name '*.cj' | sort | tr '\n' ' ')

    if compile_output=$($CJWASM $multi_files -o "$wasm_file" 2>&1); then
      run_output=$(wasmtime run -W timeout=10s --invoke main "$wasm_file" 2>&1) || true
      actual=$(echo "$run_output" | tail -1 | tr -d '[:space:]')

      # 检查 multifile 目录的预期值
      multi_expected=""
      if grep -q '预期输出' "$main_file"; then
        multi_expected=$(grep '预期输出' "$main_file" | tail -1 | grep -oE '[-]?[0-9]+' | tail -1)
      fi

      if [[ -n "$multi_expected" && "$actual" == "$multi_expected" ]]; then
        printf "  %-28s ${GREEN}✓${NC}          %-14s %-14s ${GREEN}✓ PASS${NC}\n" "multifile/" "$multi_expected" "$actual"
        ((PASS++)) || true
      elif [[ -n "$multi_expected" ]]; then
        printf "  %-28s ${GREEN}✓${NC}          %-14s ${RED}%-14s${NC} ${RED}✗ FAIL${NC}\n" "multifile/" "$multi_expected" "$actual"
        ((FAIL++)) || true
        ERRORS+=("multifile/: 预期=$multi_expected 实际=$actual")
      else
        printf "  %-28s ${GREEN}✓${NC}          ${DIM}%-14s${NC} %-14s ${YELLOW}○ SKIP${NC}\n" "multifile/" "无预期值" "$actual"
        ((SKIP++)) || true
      fi
    else
      printf "  %-28s ${RED}✗ 编译失败${NC}\n" "multifile/"
      ((FAIL++)) || true
      ERRORS+=("multifile/ (编译失败)")
    fi
  fi
fi

# ── cjpm 工程测试 (examples/project/) ────────────────────────

PROJECT_DIR="$EXAMPLES_DIR/project"
if [[ -d "$PROJECT_DIR" && -f "$PROJECT_DIR/cjpm.toml" && -z "$FILTER" ]]; then
  echo ""
  echo -e "${CYAN}[附加] cjpm 工程测试: examples/project/${NC}"

  wasm_file="$OUT_DIR/project_demo.wasm"
  if compile_output=$("$CJWASM" build -p "$PROJECT_DIR" -o "$wasm_file" 2>&1); then
    run_output=$(wasmtime run -W timeout=10s --invoke main "$wasm_file" 2>&1) || true
    actual=$(echo "$run_output" | tail -1 | tr -d '[:space:]')

    # 从 src/main.cj 提取预期值
    proj_main="$PROJECT_DIR/src/main.cj"
    proj_expected=""
    if [[ -f "$proj_main" ]] && grep -q '预期输出' "$proj_main"; then
      proj_expected=$(grep '预期输出' "$proj_main" | tail -1 | grep -oE '[-]?[0-9]+' | tail -1)
    fi

    if [[ -n "$proj_expected" && "$actual" == "$proj_expected" ]]; then
      printf "  %-28s ${GREEN}✓${NC}          %-14s %-14s ${GREEN}✓ PASS${NC}\n" "project/" "$proj_expected" "$actual"
      ((PASS++)) || true
    elif [[ -n "$proj_expected" ]]; then
      printf "  %-28s ${GREEN}✓${NC}          %-14s ${RED}%-14s${NC} ${RED}✗ FAIL${NC}\n" "project/" "$proj_expected" "$actual"
      ((FAIL++)) || true
      ERRORS+=("project/: 预期=$proj_expected 实际=$actual")
    else
      printf "  %-28s ${GREEN}✓${NC}          ${DIM}%-14s${NC} %-14s ${YELLOW}○ SKIP${NC}\n" "project/" "无预期值" "$actual"
      ((SKIP++)) || true
    fi
  else
    printf "  %-28s ${RED}✗ 编译失败${NC}\n" "project/"
    ((FAIL++)) || true
    ERRORS+=("project/ (编译失败)")
    if $VERBOSE; then
      echo -e "  ${DIM}${RED}$compile_output${NC}"
    fi
  fi
fi

# ── 汇总 ──

echo ""
echo -e "${BOLD}${CYAN}═══ 测试结果汇总 ═══${NC}"
echo -e "  ${GREEN}通过 (PASS): $PASS${NC}"
echo -e "  ${RED}失败 (FAIL): $FAIL${NC}"
if [[ $SKIP -gt 0 ]]; then
  echo -e "  ${YELLOW}跳过 (SKIP): $SKIP${NC}"
fi
echo -e "  总计: $((PASS + FAIL + SKIP))"
echo ""

if [[ $FAIL -gt 0 ]]; then
  echo -e "${RED}失败详情:${NC}"
  for err in "${ERRORS[@]}"; do
    echo -e "  ${RED}• $err${NC}"
  done
  echo ""
  echo -e "${RED}${BOLD}系统测试未通过！${NC}"
  exit 1
else
  echo -e "${GREEN}${BOLD}所有系统测试通过！${NC}"
  exit 0
fi
