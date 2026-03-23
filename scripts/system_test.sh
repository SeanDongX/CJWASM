#!/usr/bin/env bash
# ============================================================
# CJWasm 系统测试
#
# 编译并运行 tests/examples/ 下所有 .cj 示例文件，
# 自动从源文件中提取 "// 预期输出: <value>" 注释，
# 与 wasmtime 实际运行的返回值进行比较验证。
# 同时使用 wasm-validate 检查生成 WASM 的合法性。
#
# 用法:
#   ./scripts/system_test.sh              # 运行所有示例测试
#   ./scripts/system_test.sh hello.cj     # 仅测试指定文件
#   ./scripts/system_test.sh --verbose    # 显示详细输出
#   ./scripts/system_test.sh --compile    # 仅编译+验证，不运行
#   ./scripts/system_test.sh --no-build   # 跳过编译器构建
# ============================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
EXAMPLES_DIR="$PROJECT_ROOT/tests/examples"
OUT_DIR="$PROJECT_ROOT/target/examples"

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
COMPILE_ONLY=false
NO_STD_TEST=false
FILTER=""

for arg in "$@"; do
  case "$arg" in
    --verbose|-v)  VERBOSE=true ;;
    --no-build)    NO_BUILD=true ;;
    --compile)     COMPILE_ONLY=true ;;
    --no-std-test) NO_STD_TEST=true ;;
    -h|--help)
      echo "用法: $0 [选项] [文件名...]"
      echo ""
      echo "系统测试：编译运行 .cj 文件并验证返回值与预期是否一致。"
      echo "预期值从源文件中 '// 预期输出: <value>' 注释提取。"
      echo ""
      echo "选项:"
      echo "  --verbose, -v  显示详细输出（编译器输出、WASM 验证错误等）"
      echo "  --compile      仅编译和 WASM 验证，不运行"
      echo "  --no-build     跳过编译器构建（假定已构建）"
      echo "  --no-std-test  跳过 std_test.sh 兼容性测试"
      echo "  -h, --help     显示帮助"
      echo ""
      echo "示例:"
      echo "  $0                    # 运行所有示例测试"
      echo "  $0 hello.cj math.cj  # 仅测试指定文件"
      echo "  $0 --compile          # 仅编译 + WASM 验证"
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
  build_output=$(cargo build --release --manifest-path "$PROJECT_ROOT/Cargo.toml" 2>&1) || {
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

CJWASM="$PROJECT_ROOT/target/release/cjwasm"
if [[ ! -x "$CJWASM" ]]; then
  echo -e "${RED}错误: 编译器不存在: $CJWASM${NC}" >&2
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

# 检查 wasm-validate
HAS_WASM_VALIDATE=false
if command -v wasm-validate &>/dev/null; then
  HAS_WASM_VALIDATE=true
fi

mkdir -p "$OUT_DIR"

# WASM 验证错误汇总日志
VALIDATE_LOG=$(mktemp "${TMPDIR:-/tmp}/cjwasm_validate.XXXXXX")
trap 'rm -f "$VALIDATE_LOG"' EXIT
VALIDATE_ERRORS_TOTAL=0
VALIDATE_FILES_INVALID=0

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

# ── 公共函数 ──

# 从源文件提取 "// 预期输出: <value>"
extract_expected() {
  local filepath="$1"
  local expected=""
  if grep -q '预期输出' "$filepath"; then
    expected=$(grep '预期输出' "$filepath" | tail -1 | grep -oE '[-]?[0-9]+' | tail -1 || true)
    if [[ -z "$expected" ]]; then
      expected=$(grep '预期输出' "$filepath" | tail -1 | sed 's/.*预期输出[：:][[:space:]]*//' | sed 's/[[:space:]]*$//')
    fi
  fi
  echo "$expected"
}

# 运行 wasm-validate 并记录结果；设置 _validate_status 和 _validate_err_count
run_wasm_validate() {
  local wasm_file="$1" label="$2"
  _validate_status=""
  _validate_err_count=0
  if $HAS_WASM_VALIDATE; then
    local validate_output
    validate_output=$(wasm-validate "$wasm_file" 2>&1) || true
    _validate_err_count=$(echo "$validate_output" | (grep "error:" || true) | wc -l | tr -d ' ')
    if [[ $_validate_err_count -eq 0 ]]; then
      _validate_status="${GREEN}✓${NC}"
    else
      _validate_status="${RED}✗${_validate_err_count}err${NC}"
      ((VALIDATE_ERRORS_TOTAL += _validate_err_count)) || true
      ((VALIDATE_FILES_INVALID++)) || true
      echo "$validate_output" | (grep "error:" || true) | \
        sed "s|^.*error: |${label}\t|" >> "$VALIDATE_LOG"
    fi
    if $VERBOSE && [[ $_validate_err_count -gt 0 ]]; then
      echo "$validate_output" | (grep "error:" || true) | head -5 | while IFS= read -r line; do
        echo -e "  ${DIM}${RED}$line${NC}"
      done
      if [[ $_validate_err_count -gt 5 ]]; then
        echo -e "  ${DIM}${RED}... 还有 $((_validate_err_count - 5)) 个错误${NC}"
      fi
    fi
  else
    _validate_status="${DIM}—${NC}"
  fi
}

# 运行 wasmtime 并设置 _actual 和 _is_error
run_wasmtime() {
  local wasm_file="$1"
  _actual=""
  _is_error=false

  local run_output="" exit_code=0 stderr_file
  stderr_file=$(mktemp)
  run_output=$(wasmtime run -W timeout=10s --invoke main "$wasm_file" 2>"$stderr_file") || exit_code=$?
  local run_stderr
  run_stderr=$(cat "$stderr_file")
  rm -f "$stderr_file"

  if [[ $exit_code -ne 0 ]] || echo "$run_stderr" | grep -qi "trap\|unreachable\|timed out\|failed to"; then
    _is_error=true
    if echo "$run_stderr" | grep -qi "timed out"; then
      _actual="TIMEOUT"
    elif echo "$run_stderr" | grep -qi "failed to compile\|translation error\|type mismatch"; then
      _actual="WASM无效"
    elif echo "$run_stderr" | grep -qi "unreachable"; then
      _actual="UNREACHABLE"
    elif echo "$run_stderr" | grep -qi "trap"; then
      _actual="TRAP"
    else
      _actual="ERROR"
    fi
    if $VERBOSE; then
      echo "$run_output" | head -3 | while IFS= read -r line; do
        echo -e "  ${DIM}${RED}$line${NC}"
      done
    fi
  else
    _actual=$(echo "$run_output" | tail -1 | tr -d '[:space:]')
  fi
}

# ── 运行测试 ──

echo -e "${CYAN}[3/3] 编译、运行并验证...${NC}"
echo ""

PASS=0
FAIL=0
SKIP=0
ERRORS=()

# 表头（验证列仅在 wasm-validate 可用时显示）
if $HAS_WASM_VALIDATE; then
  printf "${BOLD}%-30s %-10s %-6s %-14s %-14s %s${NC}\n" "文件" "编译" "验证" "预期" "实际" "结果"
  printf "%-30s %-10s %-6s %-14s %-14s %s\n" \
    "──────────────────────────────" "──────────" "──────" "──────────────" "──────────────" "──────"
else
  printf "${BOLD}%-30s %-10s %-14s %-14s %s${NC}\n" "文件" "编译" "预期" "实际" "结果"
  printf "%-30s %-10s %-14s %-14s %s\n" \
    "──────────────────────────────" "──────────" "──────────────" "──────────────" "──────"
fi

for filepath in "${FILES[@]}"; do
  filename=$(basename "$filepath")
  name="${filename%.cj}"
  wasm_file="$OUT_DIR/${name}.wasm"

  # ── 提取预期输出 ──
  expected=$(extract_expected "$filepath")

  # ── 编译 ──
  compile_output=""
  compile_ok=false
  if compile_output=$("$CJWASM" "$filepath" -o "$wasm_file" 2>&1); then
    compile_ok=true
  fi

  if ! $compile_ok; then
    ((FAIL++)) || true
    ERRORS+=("$filename: 编译失败")
    if $HAS_WASM_VALIDATE; then
      printf "%-30s ${RED}✗ 失败${NC}     %-6s %-14s %-14s ${RED}✗ FAIL${NC}\n" "$filename" " " "${expected:-?}" "-"
    else
      printf "%-30s ${RED}✗ 失败${NC}     %-14s %-14s ${RED}✗ FAIL${NC}\n" "$filename" "${expected:-?}" "-"
    fi
    if $VERBOSE && [[ -n "$compile_output" ]]; then
      echo -e "  ${DIM}${RED}$compile_output${NC}"
    fi
    continue
  fi

  if $VERBOSE && [[ -n "$compile_output" ]]; then
    echo -e "  ${DIM}$compile_output${NC}"
  fi

  # ── WASM 验证 ──
  run_wasm_validate "$wasm_file" "$filename"
  validate_col="$_validate_status"

  # ── 运行 ──
  if $COMPILE_ONLY; then
    if $HAS_WASM_VALIDATE; then
      printf "%-30s ${GREEN}✓${NC}          %-10b ${DIM}%-14s${NC} ${YELLOW}— 跳过${NC}       ${YELLOW}○ SKIP${NC}\n" \
        "$filename" "$validate_col" "${expected:-—}"
    else
      printf "%-30s ${GREEN}✓${NC}          ${DIM}%-14s${NC} ${YELLOW}— 跳过${NC}       ${YELLOW}○ SKIP${NC}\n" \
        "$filename" "${expected:-—}"
    fi
    ((SKIP++)) || true
    continue
  fi

  run_wasmtime "$wasm_file"
  actual="$_actual"
  is_error="$_is_error"

  # ── 验证 ──
  result=""
  if $is_error; then
    ((FAIL++)) || true
    ERRORS+=("$filename: 运行错误 ($actual)")
    result="${RED}✗ FAIL${NC}"
    actual_col="${RED}${actual}${NC}"
  elif [[ -z "$expected" ]]; then
    ((SKIP++)) || true
    result="${YELLOW}○ SKIP${NC}"
    actual_col="$actual"
  elif [[ "$actual" == "$expected" ]]; then
    ((PASS++)) || true
    result="${GREEN}✓ PASS${NC}"
    actual_col="$actual"
  else
    ((FAIL++)) || true
    ERRORS+=("$filename: 预期=$expected 实际=$actual")
    result="${RED}✗ FAIL${NC}"
    actual_col="${RED}${actual}${NC}"
  fi

  expected_display="${expected:-无预期值}"

  if $HAS_WASM_VALIDATE; then
    printf "%-30s ${GREEN}✓${NC}          %-10b %-14s %-18b %b\n" \
      "$filename" "$validate_col" "$expected_display" "$actual_col" "$result"
  else
    printf "%-30s ${GREEN}✓${NC}          %-14s %-18b %b\n" \
      "$filename" "$expected_display" "$actual_col" "$result"
  fi
done

# ── 多文件测试 ──

MULTIFILE_DIR="$EXAMPLES_DIR/multifile"
if [[ -d "$MULTIFILE_DIR" && -z "$FILTER" ]]; then
  echo ""
  echo -e "${CYAN}[附加] 多文件编译测试: tests/examples/multifile/${NC}"

  main_file="$MULTIFILE_DIR/module_main.cj"
  if [[ -f "$main_file" ]]; then
    wasm_file="$OUT_DIR/multifile.wasm"
    multi_files=$(find "$MULTIFILE_DIR" -name '*.cj' | sort | tr '\n' ' ')

    if compile_output=$($CJWASM $multi_files -o "$wasm_file" 2>&1); then
      run_wasm_validate "$wasm_file" "multifile/"

      multi_expected=$(extract_expected "$main_file")

      if $COMPILE_ONLY; then
        printf "  %-28s ${GREEN}✓${NC}          ${YELLOW}— 跳过${NC}\n" "multifile/"
        ((SKIP++)) || true
      else
        run_wasmtime "$wasm_file"

        if [[ -n "$multi_expected" && "$_actual" == "$multi_expected" ]]; then
          printf "  %-28s ${GREEN}✓${NC}          %-14s %-14s ${GREEN}✓ PASS${NC}\n" "multifile/" "$multi_expected" "$_actual"
          ((PASS++)) || true
        elif [[ -n "$multi_expected" ]]; then
          printf "  %-28s ${GREEN}✓${NC}          %-14s ${RED}%-14s${NC} ${RED}✗ FAIL${NC}\n" "multifile/" "$multi_expected" "$_actual"
          ((FAIL++)) || true
          ERRORS+=("multifile/: 预期=$multi_expected 实际=$_actual")
        else
          printf "  %-28s ${GREEN}✓${NC}          ${DIM}%-14s${NC} %-14s ${YELLOW}○ SKIP${NC}\n" "multifile/" "无预期值" "$_actual"
          ((SKIP++)) || true
        fi
      fi
    else
      printf "  %-28s ${RED}✗ 编译失败${NC}\n" "multifile/"
      ((FAIL++)) || true
      ERRORS+=("multifile/ (编译失败)")
      if $VERBOSE; then
        echo -e "  ${DIM}${RED}$compile_output${NC}"
      fi
    fi
  fi
fi

# ── cjpm 工程测试 (tests/examples/project/) ──

PROJ_DIR="$EXAMPLES_DIR/project"
if [[ -d "$PROJ_DIR" && -f "$PROJ_DIR/cjpm.toml" && -z "$FILTER" ]]; then
  echo ""
  echo -e "${CYAN}[附加] cjpm 工程测试: tests/examples/project/${NC}"

  wasm_file="$OUT_DIR/project_demo.wasm"
  if compile_output=$("$CJWASM" build -p "$PROJ_DIR" -o "$wasm_file" 2>&1); then
    run_wasm_validate "$wasm_file" "project/"

    proj_expected=$(extract_expected "$PROJ_DIR/src/main.cj")

    if $COMPILE_ONLY; then
      printf "  %-28s ${GREEN}✓${NC}          ${YELLOW}— 跳过${NC}\n" "project/"
      ((SKIP++)) || true
    else
      run_wasmtime "$wasm_file"

      if [[ -n "$proj_expected" && "$_actual" == "$proj_expected" ]]; then
        printf "  %-28s ${GREEN}✓${NC}          %-14s %-14s ${GREEN}✓ PASS${NC}\n" "project/" "$proj_expected" "$_actual"
        ((PASS++)) || true
      elif [[ -n "$proj_expected" ]]; then
        printf "  %-28s ${GREEN}✓${NC}          %-14s ${RED}%-14s${NC} ${RED}✗ FAIL${NC}\n" "project/" "$proj_expected" "$_actual"
        ((FAIL++)) || true
        ERRORS+=("project/: 预期=$proj_expected 实际=$_actual")
      else
        printf "  %-28s ${GREEN}✓${NC}          ${DIM}%-14s${NC} %-14s ${YELLOW}○ SKIP${NC}\n" "project/" "无预期值" "$_actual"
        ((SKIP++)) || true
      fi
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

# ── L1 标准库工程 (tests/examples/std/) ──

STD_DIR="$EXAMPLES_DIR/std"
if [[ -d "$STD_DIR" && -f "$STD_DIR/cjpm.toml" && -z "$FILTER" ]]; then
  echo ""
  echo -e "${CYAN}[附加] L1 标准库示例: tests/examples/std/${NC}"

  if compile_output=$(cd "$PROJECT_ROOT" && "$CJWASM" build -p tests/examples/std 2>&1); then
    std_wasm="$STD_DIR/target/wasm/std_examples.wasm"
    if [[ -f "$std_wasm" ]]; then
      run_wasm_validate "$std_wasm" "std/"
    fi

    if $COMPILE_ONLY; then
      printf "  %-28s ${GREEN}✓${NC}          ${YELLOW}— 跳过${NC}\n" "std/"
      ((SKIP++)) || true
    elif [[ -f "$std_wasm" ]] && $HAS_WASMTIME; then
      run_wasmtime "$std_wasm"
      printf "  %-28s ${GREEN}✓${NC}          %-14s ${GREEN}✓ 运行${NC}\n" "std/" "$_actual"
      ((PASS++)) || true
    else
      printf "  %-28s ${GREEN}✓${NC}          ${YELLOW}— 跳过${NC}\n" "std/"
      ((PASS++)) || true
    fi
  else
    printf "  %-28s ${RED}✗ 编译失败${NC}\n" "std/"
    ((FAIL++)) || true
    ERRORS+=("std/ (编译失败)")
    if $VERBOSE; then
      echo -e "  ${DIM}${RED}$compile_output${NC}"
    fi
  fi
fi

# ── std 兼容性测试 (third_party/cangjie_test) ──

STD_TEST_SCRIPT="$SCRIPT_DIR/std_test.sh"
if [[ -x "$STD_TEST_SCRIPT" && -z "$FILTER" && $NO_STD_TEST == false ]]; then
  echo ""
  echo -e "${CYAN}[附加] std 兼容性测试: third_party/cangjie_test${NC}"

  std_test_args=()
  if $HAS_WASM_VALIDATE; then
    std_test_args+=("--validate")
  fi
  if $VERBOSE; then
    std_test_args+=("--verbose")
  fi

  if "$STD_TEST_SCRIPT" "${std_test_args[@]}"; then
    printf "  %-28s ${GREEN}✓${NC}\n" "std_test.sh"
    ((PASS++)) || true
  else
    printf "  %-28s ${RED}✗ 失败${NC}\n" "std_test.sh"
    ((FAIL++)) || true
    ERRORS+=("std_test.sh (失败)")
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

# WASM 验证错误汇总
if $HAS_WASM_VALIDATE; then
  if [[ $VALIDATE_ERRORS_TOTAL -eq 0 ]]; then
    echo -e "  ${GREEN}WASM 验证错误: 0${NC}"
  else
    echo -e "  ${RED}WASM 验证错误: ${VALIDATE_ERRORS_TOTAL} 条（${VALIDATE_FILES_INVALID} 个文件）${NC}"

    echo ""
    echo -e "${BOLD}${CYAN}═══ WASM 验证错误分类 ═══${NC}"

    echo -e "${BOLD}  按错误类型（Top 20）:${NC}"
    awk -F'\t' 'NF>=2{print $2}' "$VALIDATE_LOG" \
      | sed 's/, expected.*//; s/: expected.*//' \
      | sort | uniq -c | sort -rn \
      | head -20 \
      | awk '{
          count=$1; $1=""; msg=substr($0,2)
          printf "  %6d  %s\n", count, msg
        }'

    HAS_TYPE_MISMATCH=$(awk -F'\t' 'NF>=2 && $2~/type mismatch/' "$VALIDATE_LOG" | wc -l | tr -d ' ')
    if [[ $HAS_TYPE_MISMATCH -gt 0 ]]; then
      echo ""
      echo -e "${BOLD}  类型不匹配详细组合（Top 15）:${NC}"
      awk -F'\t' 'NF>=2 && $2~/type mismatch/{print $2}' "$VALIDATE_LOG" \
        | grep -oE 'expected \[[^]]*\] but got \[[^]]*\]|expected [^ ,]+ but [^,]+' \
        | sort | uniq -c | sort -rn \
        | head -15 \
        | awk '{
            count=$1; $1=""; msg=substr($0,2)
            printf "  %6d  %s\n", count, msg
          }'
    fi

    echo ""
    echo -e "${BOLD}  按来源文件:${NC}"
    awk -F'\t' 'NF>=2{print $1}' "$VALIDATE_LOG" \
      | sort | uniq -c | sort -rn \
      | awk '{
          count=$1; $1=""; fname=substr($0,2)
          printf "  %6d  %s\n", count, fname
        }'
  fi
fi

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
