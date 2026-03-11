#!/usr/bin/env bash
# ============================================================
# CJWasm 测试运行器
#
# 交互式菜单，选择运行不同级别的测试。
#
# 用法:
#   ./scripts/run_test.sh          # 交互式菜单
#   ./scripts/run_test.sh 1        # 直接运行 cargo test
#   ./scripts/run_test.sh 2        # 直接运行 system test (含 std L1)
#   ./scripts/run_test.sh 3        # 直接运行 performance test
#   ./scripts/run_test.sh 4        # cargo test + system test
#   ./scripts/run_test.sh 5        # 全部运行 (1 + 2 + 3)
# ============================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

# ── 颜色 ──
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
DIM='\033[2m'
NC='\033[0m'

# ── 结果追踪 ──
TOTAL_PASS=0
TOTAL_FAIL=0
RESULTS=()

# ── 测试函数 ──

run_cargo_test() {
    echo ""
    echo -e "${BOLD}${CYAN}━━━ [1] Cargo Test (单元测试 + 集成测试) ━━━${NC}"
    echo ""
    if cargo test --manifest-path "$PROJECT_DIR/Cargo.toml" 2>&1; then
        ((TOTAL_PASS++)) || true
        RESULTS+=("${GREEN}✓ Cargo Test${NC}")
    else
        ((TOTAL_FAIL++)) || true
        RESULTS+=("${RED}✗ Cargo Test${NC}")
    fi
}

run_system_test() {
    echo ""
    echo -e "${BOLD}${CYAN}━━━ [2] System Test (编译运行 .cj 示例 + std L1) ━━━${NC}"
    echo ""
    if bash "$SCRIPT_DIR/system_test.sh" --no-build; then
        ((TOTAL_PASS++)) || true
        RESULTS+=("${GREEN}✓ System Test${NC}")
    else
        ((TOTAL_FAIL++)) || true
        RESULTS+=("${RED}✗ System Test${NC}")
    fi
}

run_performance_test() {
    echo ""
    echo -e "${BOLD}${CYAN}━━━ [3] Performance Test (性能基准测试) ━━━${NC}"
    echo ""
    if [[ -f "$SCRIPT_DIR/benchmark.sh" ]]; then
        if bash "$SCRIPT_DIR/benchmark.sh" --quick; then
            ((TOTAL_PASS++)) || true
            RESULTS+=("${GREEN}✓ Performance Test${NC}")
        else
            ((TOTAL_FAIL++)) || true
            RESULTS+=("${RED}✗ Performance Test${NC}")
        fi
    else
        echo -e "${YELLOW}⚠ benchmark.sh 不存在，跳过${NC}"
        RESULTS+=("${YELLOW}○ Performance Test (跳过)${NC}")
    fi
}

# ── 构建编译器（共享） ──

build_compiler() {
    echo -e "${CYAN}构建编译器 (release)...${NC}"
    if cargo build --release --manifest-path "$PROJECT_DIR/Cargo.toml" 2>&1 | tail -1; then
        echo -e "${GREEN}构建完成${NC}"
    else
        echo -e "${RED}构建失败！${NC}"
        exit 1
    fi
    echo ""
}

# ── 汇总 ──

print_summary() {
    echo ""
    echo -e "${BOLD}${CYAN}━━━ 测试汇总 ━━━${NC}"
    for r in "${RESULTS[@]}"; do
        echo -e "  $r"
    done
    echo ""
    echo -e "  通过: ${GREEN}${TOTAL_PASS}${NC}  失败: ${RED}${TOTAL_FAIL}${NC}"
    echo ""
    if [[ $TOTAL_FAIL -eq 0 ]]; then
        echo -e "${GREEN}${BOLD}所有测试通过！${NC}"
    else
        echo -e "${RED}${BOLD}存在失败的测试！${NC}"
        exit 1
    fi
}

# ── 菜单 ──

show_menu() {
    echo -e "${BOLD}${CYAN}═══ CJWasm 测试运行器 ═══${NC}"
    echo ""
    echo -e "  ${BOLD}1${NC}  Cargo Test          ${DIM}单元测试 + 集成测试${NC}"
    echo -e "  ${BOLD}2${NC}  System Test         ${DIM}编译运行所有 .cj 示例 (含 std L1)${NC}"
    echo -e "  ${BOLD}3${NC}  Performance Test    ${DIM}性能基准测试${NC}"
    echo -e "  ${BOLD}4${NC}  Cargo + System      ${DIM}运行 1 + 2${NC}"
    echo -e "  ${BOLD}5${NC}  All                 ${DIM}运行 1 + 2 + 3${NC}"
    echo ""
    echo -ne "  请选择 [1-5]: "
}

# ── 主逻辑 ──

CHOICE="${1:-}"

if [[ -z "$CHOICE" ]]; then
    show_menu
    read -r CHOICE
fi

case "$CHOICE" in
    1)
        run_cargo_test
        ;;
    2)
        build_compiler
        run_system_test
        ;;
    3)
        build_compiler
        run_performance_test
        ;;
    4)
        run_cargo_test
        build_compiler
        run_system_test
        ;;
    5)
        run_cargo_test
        build_compiler
        run_system_test
        run_performance_test
        ;;
    *)
        echo -e "${RED}无效选项: $CHOICE${NC}" >&2
        echo "用法: $0 [1|2|3|4|5]" >&2
        exit 1
        ;;
esac

print_summary
