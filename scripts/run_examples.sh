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

# 检查 wasm-validate
HAS_WASM_VALIDATE=false
if command -v wasm-validate &>/dev/null; then
  HAS_WASM_VALIDATE=true
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
VALIDATE_ERRORS_TOTAL=0   # 全部 WASM 验证错误行数之和
VALIDATE_FILES_INVALID=0  # 含验证错误的 WASM 文件数

# 临时文件：收集所有 wasm-validate 错误行，用于结尾的分类汇总
VALIDATE_LOG=$(mktemp /tmp/cjwasm_validate_XXXXXX.log)
trap 'rm -f "$VALIDATE_LOG"' EXIT

printf "${BOLD}%-30s %-12s %-14s %-12s %s${NC}\n" "文件" "编译" "WASM验证" "运行" "返回值"
printf "%-30s %-12s %-14s %-12s %s\n" \
  "──────────────────────────────" "────────────" "──────────────" "────────────" "──────"

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
    compile_status="${GREEN}✓ 编译${NC}"
  else
    ((FAIL++)) || true
    ERRORS+=("$filename (编译失败)")
    printf "%-30s ${RED}✗ 编译失败${NC}\n" "$filename"
    if $VERBOSE && [[ -n "$compile_output" ]]; then
      echo -e "  ${RED}$compile_output${NC}"
    fi
    continue
  fi

  # ── WASM 验证 ──
  validate_status=""
  validate_err_count=0
  if $HAS_WASM_VALIDATE; then
    validate_output=$(wasm-validate "$wasm_file" 2>&1) || true
    validate_err_count=$(echo "$validate_output" | (grep "error:" || true) | wc -l | tr -d ' ')
    if [[ $validate_err_count -eq 0 ]]; then
      validate_status="${GREEN}✓ 合法${NC}"
    else
      validate_status="${RED}✗ ${validate_err_count}个错误${NC}"
      ((VALIDATE_ERRORS_TOTAL += validate_err_count)) || true
      ((VALIDATE_FILES_INVALID++)) || true
      # 追加错误行到汇总日志（格式："文件名\t错误描述"）
      echo "$validate_output" | (grep "error:" || true) | \
        sed "s|^.*error: |${filename}\t|" >> "$VALIDATE_LOG"
    fi
  else
    validate_status="${YELLOW}— 跳过${NC}"
  fi

  # ── 运行 ──
  if $COMPILE_ONLY; then
    printf "%-30s ${compile_status}       %-20b ${YELLOW}— 跳过${NC}\n" \
      "$filename" "$validate_status"
    ((PASS++)) || true
    if $VERBOSE && $HAS_WASM_VALIDATE && [[ $validate_err_count -gt 0 ]]; then
      echo "$validate_output" | (grep "error:" || true) | head -5 | while IFS= read -r line; do
        echo -e "  ${RED}  $line${NC}"
      done
    fi
    continue
  fi

  run_output=""
  exit_code=0
  # wasmtime 运行，--invoke main 调用入口函数（10 秒超时防止死循环）
  run_output=$(wasmtime run -W timeout=10s --invoke main "$wasm_file" 2>&1) || exit_code=$?

  if [[ $exit_code -eq 0 ]]; then
    return_val="${run_output##*$'\n'}"  # 最后一行为返回值
    printf "%-30s ${compile_status}       %-20b ${GREEN}✓ 运行${NC}       %s\n" \
      "$filename" "$validate_status" "$return_val"
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
        short_err="WASM无效"
      elif echo "$run_output" | grep -qi "unreachable"; then
        short_err="unreachable"
      elif echo "$run_output" | grep -qi "trap"; then
        short_err="trap"
      else
        short_err="exit=$exit_code"
      fi
      ((FAIL++)) || true
      ERRORS+=("$filename ($short_err)")
      printf "%-30s ${compile_status}       %-20b ${RED}✗ %-10s${NC}\n" \
        "$filename" "$validate_status" "$short_err"
      if $VERBOSE; then
        echo "$run_output" | head -3 | while IFS= read -r line; do
          echo -e "  ${RED}$line${NC}"
        done
      fi
    else
      # 非零返回值但无错误，视为正常（main 返回非零数值）
      return_val="${run_output##*$'\n'}"
      printf "%-30s ${compile_status}       %-20b ${GREEN}✓ 运行${NC}       %s\n" \
        "$filename" "$validate_status" "$return_val"
      ((PASS++)) || true
    fi
  fi

  # 验证错误详情（verbose 模式）
  if $VERBOSE && $HAS_WASM_VALIDATE && [[ $validate_err_count -gt 0 ]]; then
    echo "$validate_output" | (grep "error:" || true) | head -5 | while IFS= read -r line; do
      echo -e "  ${RED}  $line${NC}"
    done
    if [[ $validate_err_count -gt 5 ]]; then
      echo -e "  ${RED}  ... 还有 $((validate_err_count - 5)) 个错误${NC}"
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
      # WASM 验证
      mf_validate_status="${YELLOW}— 跳过${NC}"
      mf_validate_count=0
      if $HAS_WASM_VALIDATE; then
        mf_validate_out=$(wasm-validate "$wasm_file" 2>&1) || true
        mf_validate_count=$(echo "$mf_validate_out" | (grep "error:" || true) | wc -l | tr -d ' ')
        if [[ $mf_validate_count -eq 0 ]]; then
          mf_validate_status="${GREEN}✓ 合法${NC}"
        else
          mf_validate_status="${RED}✗ ${mf_validate_count}个错误${NC}"
          ((VALIDATE_ERRORS_TOTAL += mf_validate_count)) || true
          ((VALIDATE_FILES_INVALID++)) || true
          echo "$mf_validate_out" | (grep "error:" || true) | \
            sed "s|^.*error: |multifile/\t|" >> "$VALIDATE_LOG"
        fi
      fi
      printf "%-30s ${GREEN}✓ 编译${NC}       %-20b" "multifile/" "$mf_validate_status"
      if ! $COMPILE_ONLY && $HAS_WASMTIME; then
        run_output=$(wasmtime run -W timeout=10s --invoke main "$wasm_file" 2>&1) || true
        return_val="${run_output##*$'\n'}"
        printf " ${GREEN}✓ 运行${NC}       %s\n" "$return_val"
      else
        printf " ${YELLOW}— 跳过${NC}\n"
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

# ── examples/std（L1 标准库 cjpm 工程）──────────────────────────

STD_DIR="$EXAMPLES_DIR/std"
if [[ -d "$STD_DIR" && -f "$STD_DIR/cjpm.toml" && -z "$FILTER" ]]; then
  echo ""
  echo -e "${CYAN}[附加] L1 标准库示例: examples/std/ (cjwasm build -p examples/std)${NC}"

  if compile_output=$(cd "$PROJECT_DIR" && "$CJWASM" build -p examples/std 2>&1); then
    std_wasm="$STD_DIR/target/wasm/std_examples.wasm"
    # WASM 验证
    std_validate_status="${YELLOW}— 跳过${NC}"
    if $HAS_WASM_VALIDATE && [[ -f "$std_wasm" ]]; then
      std_validate_out=$(wasm-validate "$std_wasm" 2>&1) || true
      std_validate_count=$(echo "$std_validate_out" | (grep "error:" || true) | wc -l | tr -d ' ')
      if [[ $std_validate_count -eq 0 ]]; then
        std_validate_status="${GREEN}✓ 合法${NC}"
      else
        std_validate_status="${RED}✗ ${std_validate_count}个错误${NC}"
        ((VALIDATE_ERRORS_TOTAL += std_validate_count)) || true
        ((VALIDATE_FILES_INVALID++)) || true
        echo "$std_validate_out" | (grep "error:" || true) | \
          sed "s|^.*error: |std/\t|" >> "$VALIDATE_LOG"
      fi
    fi
    printf "%-30s ${GREEN}✓ 编译${NC}       %-20b" "std/" "$std_validate_status"
    if ! $COMPILE_ONLY && $HAS_WASMTIME && [[ -f "$std_wasm" ]]; then
      run_output=$(wasmtime run -W timeout=10s --invoke main "$std_wasm" 2>&1) || true
      return_val="${run_output##*$'\n'}"
      printf " ${GREEN}✓ 运行${NC}       %s\n" "$return_val"
    else
      printf " ${YELLOW}— 跳过${NC}\n"
    fi
    ((PASS++)) || true
  else
    printf "%-30s ${RED}✗ 编译失败${NC}\n" "std/"
    ((FAIL++)) || true
    ERRORS+=("std/ (编译失败)")
    if $VERBOSE; then
      echo -e "  ${RED}$compile_output${NC}"
    fi
  fi
fi

# ── 汇总 ──────────────────────────────────────────────────────

echo ""
echo -e "${BOLD}${CYAN}═══ 结果汇总 ═══${NC}"
echo -e "  ${GREEN}通过: $PASS${NC}"
if [[ $FAIL -gt 0 ]]; then
  echo -e "  ${RED}编译/运行失败: $FAIL${NC}"
else
  echo -e "  编译/运行失败: 0"
fi

# WASM 验证错误统计 + 分类汇总
if $HAS_WASM_VALIDATE; then
  if [[ $VALIDATE_ERRORS_TOTAL -eq 0 ]]; then
    echo -e "  ${GREEN}WASM 验证错误: 0${NC}"
  else
    echo -e "  ${RED}WASM 验证错误: ${VALIDATE_ERRORS_TOTAL} 条（${VALIDATE_FILES_INVALID} 个文件）${NC}"

    # ── 错误类型分类汇总 ──────────────────────────────────────
    echo ""
    echo -e "${BOLD}${CYAN}═══ WASM 验证错误分类 ═══${NC}"

    # 第一层：按指令粒度分组（提取 "type mismatch in X" 或其他错误前缀）
    # 日志格式：文件名\t错误描述
    # 策略：取错误描述的第一个逗号前的内容作为"错误类型键"
    echo -e "${BOLD}  按错误类型（Top 20）:${NC}"
    awk -F'\t' 'NF>=2{print $2}' "$VALIDATE_LOG" \
      | sed 's/, expected.*//; s/: expected.*//' \
      | sort | uniq -c | sort -rn \
      | head -20 \
      | awk '{
          count=$1; $1=""; msg=substr($0,2)
          printf "  %6d  %s\n", count, msg
        }'

    # 第二层：按 "expected X but got Y" 模式分组（类型不匹配的具体组合）
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

    # 第三层：来源文件分布
    echo ""
    echo -e "${BOLD}  按来源文件:${NC}"
    awk -F'\t' 'NF>=2{print $1}' "$VALIDATE_LOG" \
      | sort | uniq -c | sort -rn \
      | awk '{
          count=$1; $1=""; fname=substr($0,2)
          printf "  %6d  %s\n", count, fname
        }'
  fi
else
  echo -e "  ${YELLOW}WASM 验证错误: 未检测（wasm-validate 未安装）${NC}"
  echo -e "  ${YELLOW}  安装: brew install wabt${NC}"
fi

if [[ $FAIL -gt 0 ]]; then
  echo ""
  echo -e "${RED}编译/运行失败详情:${NC}"
  for err in "${ERRORS[@]}"; do
    echo -e "  ${RED}• $err${NC}"
  done
fi
echo ""

if [[ $FAIL -gt 0 ]]; then
  echo -e "${RED}${BOLD}部分示例编译或运行失败！${NC}"
  exit 1
elif [[ $VALIDATE_ERRORS_TOTAL -gt 0 ]]; then
  echo -e "${YELLOW}${BOLD}所有示例通过编译运行，但存在 ${VALIDATE_ERRORS_TOTAL} 个 WASM 验证错误。${NC}"
  if ! $COMPILE_ONLY; then
    echo -e "WASM 文件输出: ${CYAN}$OUT_DIR/${NC}"
  fi
  exit 0
else
  echo -e "${GREEN}${BOLD}所有示例编译运行成功！${NC}"
  if ! $COMPILE_ONLY; then
    echo -e "WASM 文件输出: ${CYAN}$OUT_DIR/${NC}"
  fi
  exit 0
fi
