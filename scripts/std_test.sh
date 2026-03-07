#!/usr/bin/env bash
# ============================================================
# CJWasm std 兼容性测试
#
# 基于 cangjie_test 官方测试集 (third_party/cangjie_test/testsuites/LLT/API/std)
# 测试 CJWasm 对标准库相关 .cj 文件的编译能力（解析 + 代码生成）。
#
# 用法:
#   ./scripts/std_test.sh                # 测试所有支持的 std 模块
#   ./scripts/std_test.sh math           # 仅测试 math 模块
#   ./scripts/std_test.sh --validate     # 额外做 wasm-validate 验证
#   ./scripts/std_test.sh --verbose      # 显示详细编译输出
#   ./scripts/std_test.sh --list         # 仅列出待测文件
# ============================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
TEST_SRC="$PROJECT_DIR/third_party/cangjie_test/testsuites/LLT/API/std"
OUT_DIR="$PROJECT_DIR/target/std_test"
STATS_DIR=""

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
DIM='\033[2m'
NC='\033[0m'

# L1 模块：pipeline.rs L1_STD_TOP 中声明的模块（vendor 解析已支持）
L1_MODULES=(
  io
  binary
  console
  overflow
  crypto
  deriving
  argOpt
  sort
  unicode
)

# L2 模块：vendor std 中存在但未纳入 L1 的模块（仅测试编译能力）
L2_MODULES=(
  math
  collection
  convert
  core
  option
  time
  random
  sync
  unittest
)

# 不支持的模块（WASM 沙箱限制或依赖过重，默认跳过）：
# fs, net, database, process, env, posix, reflect, regex, runtime, objectpool, ref, ast

SUPPORTED_MODULES=("${L1_MODULES[@]}" "${L2_MODULES[@]}")

# 已知不支持的测试文件（宏系统 F6 等架构限制）
KNOWN_SKIP=(
  # deriving: 依赖 @Test/@TestCase/quote() 宏系统
  "deriving/annotated_test.cj"
  "deriving/api-2.cj"
  "deriving/api.cj"
  "deriving/arbitrary_field_exclude.cj"
  "deriving/arbitrary_test.cj"
  "deriving/diagnostics-illegal-target.cj"
  "deriving/diagnostics-multi.cj"
  "deriving/diagnostics-order.cj"
  "deriving/diagnostics.cj"
  "deriving/equatable_test.cj"
  "deriving/primary.cj"
  "deriving/shrink_field_exclude.cj"
  "deriving/shrink_test.cj"
  "deriving/statics-ok.cj"
  "deriving/statics.cj"
  "deriving/tostring_test.cj"
  # argOpt: 依赖 @UnittestOption 自定义宏注解
  "argOpt/test_argopt.cj"
)

is_known_skip() {
  local rel="$1"
  for s in "${KNOWN_SKIP[@]}"; do
    [[ "$rel" == "$s" ]] && return 0
  done
  return 1
}

VALIDATE=false
VERBOSE=false
LIST_ONLY=false
FILTER_MODULES=()

for arg in "$@"; do
  case "$arg" in
    --validate)  VALIDATE=true ;;
    --verbose)   VERBOSE=true ;;
    --list)      LIST_ONLY=true ;;
    -h|--help)
      echo "用法: $0 [选项] [模块名...]"
      echo ""
      echo "选项:"
      echo "  --validate   对生成的 WASM 做 wasm-validate 验证"
      echo "  --verbose    显示编译器详细输出（含错误信息）"
      echo "  --list       仅列出待测文件，不编译"
      echo "  -h, --help   显示帮助"
      echo ""
      echo "L1 模块 (vendor 解析): ${L1_MODULES[*]}"
      echo "L2 模块 (编译测试):    ${L2_MODULES[*]}"
      echo ""
      echo "示例:"
      echo "  $0                   # 测试所有模块"
      echo "  $0 math collection   # 仅测试 math 和 collection"
      echo "  $0 --validate math   # 测试 math 并做 WASM 验证"
      exit 0
      ;;
    *)
      FILTER_MODULES+=("$arg")
      ;;
  esac
done

if [[ ${#FILTER_MODULES[@]} -gt 0 ]]; then
  MODULES=("${FILTER_MODULES[@]}")
else
  MODULES=("${L1_MODULES[@]}")
fi

# ── 模块统计辅助（兼容 bash 3.x，用临时文件代替关联数组）────

init_stats() {
  STATS_DIR=$(mktemp -d)
  trap 'rm -rf "$STATS_DIR"' EXIT
}

inc_stat() {
  local mod="$1" key="$2"
  local f="$STATS_DIR/${mod}_${key}"
  local v=0
  [[ -f "$f" ]] && v=$(cat "$f")
  echo $((v + 1)) > "$f"
}

get_stat() {
  local mod="$1" key="$2"
  local f="$STATS_DIR/${mod}_${key}"
  if [[ -f "$f" ]]; then cat "$f"; else echo 0; fi
}

# ── 检查前置条件 ────────────────────────────────────────────

if [[ ! -d "$TEST_SRC" ]]; then
  echo -e "${RED}错误: 测试源目录不存在: $TEST_SRC${NC}" >&2
  echo -e "${YELLOW}请先初始化 submodule:${NC}"
  echo "  git submodule update --init third_party/cangjie_test"
  exit 1
fi

# ── 构建编译器 ──────────────────────────────────────────────

echo -e "${BOLD}${CYAN}═══ CJWasm std 兼容性测试 ═══${NC}"
echo ""

if ! $LIST_ONLY; then
  echo -e "${CYAN}[1/3] 构建编译器 (release)...${NC}"
  cargo build --release --manifest-path "$PROJECT_DIR/Cargo.toml" 2>&1 | {
    if $VERBOSE; then cat; else cat > /dev/null; fi
  }
  CJWASM="$PROJECT_DIR/target/release/cjwasm"

  if [[ ! -x "$CJWASM" ]]; then
    echo -e "${RED}错误: 编译器构建失败${NC}" >&2
    exit 1
  fi

  HAS_WASM_VALIDATE=false
  if $VALIDATE && command -v wasm-validate &>/dev/null; then
    HAS_WASM_VALIDATE=true
  elif $VALIDATE; then
    echo -e "${YELLOW}警告: 未安装 wasm-validate，跳过 WASM 验证${NC}"
    echo -e "${YELLOW}  安装: brew install wabt${NC}"
    VALIDATE=false
  fi

  mkdir -p "$OUT_DIR"
  init_stats
fi

# ── 收集文件 ────────────────────────────────────────────────

echo -e "${CYAN}[2/3] 收集测试文件...${NC}"

ALL_FILES=()
TESTED_MODULES=()
MOD_COUNT=0

for mod in "${MODULES[@]}"; do
  mod_dir="$TEST_SRC/$mod"
  if [[ ! -d "$mod_dir" ]]; then
    echo -e "${YELLOW}  跳过: $mod/ (目录不存在)${NC}"
    continue
  fi

  count=0
  while IFS= read -r f; do
    ALL_FILES+=("$mod|$f")
    count=$((count + 1))
  done < <(find "$mod_dir" -name '*.cj' -type f | sort)

  if [[ $count -gt 0 ]]; then
    TESTED_MODULES+=("$mod")
    MOD_COUNT=$((MOD_COUNT + 1))
  fi
done

TOTAL=${#ALL_FILES[@]}

if [[ $TOTAL -eq 0 ]]; then
  echo -e "${YELLOW}未找到 .cj 测试文件${NC}"
  exit 0
fi

echo -e "  共 ${BOLD}${TOTAL}${NC} 个测试文件，覆盖 ${MOD_COUNT} 个模块"
echo ""

if $LIST_ONLY; then
  for entry in "${ALL_FILES[@]}"; do
    mod="${entry%%|*}"
    file="${entry#*|}"
    rel="${file#$TEST_SRC/}"
    echo "  [$mod] $rel"
  done
  echo ""
  echo "共 $TOTAL 个文件"
  exit 0
fi

# ── 编译测试 ────────────────────────────────────────────────

echo -e "${CYAN}[3/3] 编译测试...${NC}"
echo ""

TOTAL_PASS=0
TOTAL_FAIL=0
TOTAL_SKIP=0
TOTAL_VPASS=0
TOTAL_VFAIL=0
FAIL_LIST=()

current_mod=""

for entry in "${ALL_FILES[@]}"; do
  mod="${entry%%|*}"
  file="${entry#*|}"
  rel="${file#$TEST_SRC/}"
  basename_cj="$(basename "$file" .cj)"

  if [[ "$mod" != "$current_mod" ]]; then
    if [[ -n "$current_mod" ]]; then
      mp=$(get_stat "$current_mod" pass)
      mf=$(get_stat "$current_mod" fail)
      ms=$(get_stat "$current_mod" skip)
      mt=$((mp + mf))
      skip_info=""
      [[ $ms -gt 0 ]] && skip_info=" (+${ms} 跳过)"
      echo -e "  ${DIM}── $current_mod: $mp/$mt 通过${skip_info}${NC}"
      echo ""
    fi
    current_mod="$mod"
    echo -e "${BOLD}  [$mod]${NC}"
  fi

  # 跳过已知不支持的文件
  if is_known_skip "$rel"; then
    TOTAL_SKIP=$((TOTAL_SKIP + 1))
    inc_stat "$mod" skip
    $VERBOSE && echo -e "    ${DIM}⊘ $rel (已知不支持)${NC}"
    continue
  fi

  out_wasm="$OUT_DIR/${mod}_${basename_cj}.wasm"

  compile_out=$("$CJWASM" "$file" -o "$out_wasm" 2>&1) && compile_ok=true || compile_ok=false

  if $compile_ok; then
    TOTAL_PASS=$((TOTAL_PASS + 1))
    inc_stat "$mod" pass

    if $VALIDATE && $HAS_WASM_VALIDATE && [[ -f "$out_wasm" ]]; then
      validate_out=$(wasm-validate "$out_wasm" 2>&1) && validate_ok=true || validate_ok=false
      if $validate_ok; then
        TOTAL_VPASS=$((TOTAL_VPASS + 1))
        inc_stat "$mod" vpass
        $VERBOSE && echo -e "    ${GREEN}✓${NC} $rel ${GREEN}(WASM ✓)${NC}"
      else
        TOTAL_VFAIL=$((TOTAL_VFAIL + 1))
        inc_stat "$mod" vfail
        $VERBOSE && echo -e "    ${GREEN}✓${NC} $rel ${YELLOW}(WASM ✗)${NC}"
      fi
    else
      $VERBOSE && echo -e "    ${GREEN}✓${NC} $rel"
    fi
  else
    TOTAL_FAIL=$((TOTAL_FAIL + 1))
    inc_stat "$mod" fail
    FAIL_LIST+=("$rel")

    if $VERBOSE; then
      echo -e "    ${RED}✗${NC} $rel"
      echo "$compile_out" | head -3 | sed 's/^/      /'
    fi
  fi
done

if [[ -n "$current_mod" ]]; then
  mp=$(get_stat "$current_mod" pass)
  mf=$(get_stat "$current_mod" fail)
  ms=$(get_stat "$current_mod" skip)
  mt=$((mp + mf))
  skip_info=""
  [[ $ms -gt 0 ]] && skip_info=" (+${ms} 跳过)"
  echo -e "  ${DIM}── $current_mod: $mp/$mt 通过${skip_info}${NC}"
fi

# ── 汇总报告 ────────────────────────────────────────────────

echo ""
echo -e "${BOLD}${CYAN}═══ 测试报告 ═══${NC}"
echo ""

GRAND_TOTAL=$((TOTAL_PASS + TOTAL_FAIL))
PASS_PCT=0
if [[ $GRAND_TOTAL -gt 0 ]]; then
  PASS_PCT=$((TOTAL_PASS * 100 / GRAND_TOTAL))
fi

echo -e "  ${BOLD}总计${NC}: $GRAND_TOTAL 个文件"
echo -e "  ${GREEN}编译通过${NC}: $TOTAL_PASS ($PASS_PCT%)"
echo -e "  ${RED}编译失败${NC}: $TOTAL_FAIL"
if [[ $TOTAL_SKIP -gt 0 ]]; then
  echo -e "  ${DIM}已知跳过${NC}: $TOTAL_SKIP (宏系统等架构限制)"
fi

if $VALIDATE; then
  VTOTAL=$((TOTAL_VPASS + TOTAL_VFAIL))
  echo ""
  echo -e "  ${BOLD}WASM 验证${NC} (仅编译通过的 $VTOTAL 个):"
  echo -e "  ${GREEN}验证通过${NC}: $TOTAL_VPASS"
  echo -e "  ${YELLOW}验证失败${NC}: $TOTAL_VFAIL"
fi

is_l1_module() {
  local m="$1"
  for l in "${L1_MODULES[@]}"; do
    [[ "$l" == "$m" ]] && return 0
  done
  return 1
}

echo ""
echo -e "${BOLD}  各模块明细:${NC}"
echo ""
printf "  %-14s %-4s %6s %6s %6s %6s\n" "模块" "层级" "通过" "失败" "跳过" "通过率"
printf "  %-14s %-4s %6s %6s %6s %6s\n" "──────────────" "────" "──────" "──────" "──────" "──────"

for mod in "${TESTED_MODULES[@]}"; do
  mp=$(get_stat "$mod" pass)
  mf=$(get_stat "$mod" fail)
  ms=$(get_stat "$mod" skip)
  mt=$((mp + mf))
  if [[ $mt -eq 0 && $ms -eq 0 ]]; then continue; fi

  if is_l1_module "$mod"; then
    tier="L1"
  else
    tier="L2"
  fi

  if [[ $mt -eq 0 ]]; then
    printf "  %-14s %-4s %6d %6d %6d ${DIM}    ⊘${NC}\n" "$mod" "$tier" "$mp" "$mf" "$ms"
    continue
  fi

  pct=$((mp * 100 / mt))

  if [[ $mf -eq 0 ]]; then
    color="$GREEN"
  elif [[ $pct -ge 50 ]]; then
    color="$YELLOW"
  else
    color="$RED"
  fi

  printf "  %-14s %-4s %6d %6d %6d ${color}%5d%%${NC}\n" "$mod" "$tier" "$mp" "$mf" "$ms" "$pct"

  if $VALIDATE; then
    mvp=$(get_stat "$mod" vpass)
    mvf=$(get_stat "$mod" vfail)
    mvt=$((mvp + mvf))
    if [[ $mvt -gt 0 ]]; then
      printf "  ${DIM}  └─ wasm-validate: %d/%d 通过${NC}\n" "$mvp" "$mvt"
    fi
  fi
done

if [[ ${#FAIL_LIST[@]} -gt 0 ]] && [[ ${#FAIL_LIST[@]} -le 30 ]]; then
  echo ""
  echo -e "${BOLD}  编译失败文件:${NC}"
  for f in "${FAIL_LIST[@]}"; do
    echo -e "    ${RED}✗${NC} $f"
  done
elif [[ ${#FAIL_LIST[@]} -gt 30 ]]; then
  echo ""
  echo -e "${BOLD}  编译失败文件 (前 30 个):${NC}"
  for f in "${FAIL_LIST[@]:0:30}"; do
    echo -e "    ${RED}✗${NC} $f"
  done
  echo -e "    ${DIM}... 共 ${#FAIL_LIST[@]} 个失败文件${NC}"
fi

echo ""

if [[ $TOTAL_FAIL -eq 0 ]]; then
  echo -e "${GREEN}${BOLD}全部通过！${NC}"
  if [[ $TOTAL_SKIP -gt 0 ]]; then
    echo -e "${DIM}($TOTAL_SKIP 个已知不支持的文件已跳过)${NC}"
  fi
else
  echo -e "${YELLOW}编译兼容率: ${PASS_PCT}% ($TOTAL_PASS/$GRAND_TOTAL)${NC}"
  if [[ $TOTAL_SKIP -gt 0 ]]; then
    echo -e "${DIM}($TOTAL_SKIP 个已知不支持的文件已跳过)${NC}"
  fi
fi

echo -e "${DIM}WASM 输出: $OUT_DIR/${NC}"
echo ""

exit 0
