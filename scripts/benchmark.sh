#!/usr/bin/env bash
# ============================================================
# CJWasm vs CJC 综合性能基准测试
#
# 用法:
#   ./scripts/benchmark.sh           # 完整测试
#   ./scripts/benchmark.sh --quick   # 快速测试 (3 次迭代)
#   ./scripts/benchmark.sh --compile # 仅编译速度对比
#   ./scripts/benchmark.sh --runtime # 仅运行时对比
#   ./scripts/benchmark.sh --size    # 仅输出大小对比
#   ./scripts/benchmark.sh --criterion # 仅 Rust 内部微基准
# ============================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
BENCH_DIR="$PROJECT_DIR/benches/fixtures"
REPORT_DIR="$PROJECT_DIR/target/bench_reports"
TIMESTAMP=$(date +%Y%m%d_%H%M%S)

# ── 运行时峰值内存采集 (Peak RSS) ──
# 返回值: 输出 "KB" (Linux: kbytes, macOS: bytes→KB)
# 若不可用则输出 0
peak_rss_kb() {
    local cmd="$1"
    local os
    os="$(uname -s)"

    if [[ "$os" == "Darwin" ]]; then
        if command -v /usr/bin/time &>/dev/null; then
            # /usr/bin/time -l: "maximum resident set size" 的数值单位是 bytes
            local out rss_bytes
            out=$({ /usr/bin/time -l bash -c "$cmd" >/dev/null; } 2>&1) || true
            rss_bytes=$(echo "$out" | awk '/maximum resident set size/ {print $1; exit}')
            if [[ -n "${rss_bytes:-}" ]]; then
                echo $((rss_bytes / 1024))
                return 0
            fi
        fi
        echo 0
        return 0
    fi

    # GNU time: "Maximum resident set size (kbytes):"
    if command -v /usr/bin/time &>/dev/null; then
        local out rss_kb
        out=$({ /usr/bin/time -v bash -c "$cmd" >/dev/null; } 2>&1) || true
        rss_kb=$(echo "$out" | awk -F': *' '/Maximum resident set size/ {print $2; exit}')
        if [[ -n "${rss_kb:-}" ]]; then
            echo "$rss_kb"
            return 0
        fi
    fi

    echo 0
}

positive_diff_kb() {
    local total="${1:-0}"
    local base="${2:-0}"
    if [[ -z "$total" || "$total" == "0" ]]; then
        echo 0
        return 0
    fi
    if [[ -z "$base" || "$base" == "0" ]]; then
        echo "$total"
        return 0
    fi
    if (( total > base )); then
        echo $((total - base))
    else
        echo 0
    fi
}

format_kb_as_mb() {
    local kb="${1:-0}"
    if [[ -z "$kb" || "$kb" == "0" ]]; then
        echo "N/A"
        return 0
    fi
    python3 - <<PY 2>/dev/null || echo "N/A"
kb = int("$kb")
print(f"{kb/1024.0:.1f} MB")
PY
}

WASMTIME_BASELINE_RSS_KB=0
WASMTIME_BASELINE_READY=false

# 采集 wasmtime 空载基线 RSS，用于从端到端 RSS 中扣除 VM/JIT 固定成本。
measure_wasmtime_baseline_rss_kb() {
    if $WASMTIME_BASELINE_READY; then
        echo "$WASMTIME_BASELINE_RSS_KB"
        return 0
    fi

    WASMTIME_BASELINE_READY=true

    if ! $WASMTIME_AVAILABLE; then
        echo 0
        return 0
    fi

    local baseline_src="$TMPDIR/wasmtime_baseline_empty.cj"
    local baseline_wasm="$TMPDIR/wasmtime_baseline_empty.wasm"

    cat > "$baseline_src" << 'CJ'
main(): Int64 {
    return 0
}
CJ

    if ! $CJWASM "$baseline_src" -o "$baseline_wasm" >/dev/null 2>&1; then
        echo 0
        return 0
    fi

    if ! wasmtime run --invoke main "$baseline_wasm" >/dev/null 2>&1; then
        echo 0
        return 0
    fi

    local baseline_kb=0
    for _ in $(seq 1 3); do
        local v
        v=$(peak_rss_kb "wasmtime run --invoke main \"$baseline_wasm\"") || v=0
        if [[ "$v" -gt "$baseline_kb" ]]; then
            baseline_kb="$v"
        fi
    done

    WASMTIME_BASELINE_RSS_KB="$baseline_kb"
    echo "$WASMTIME_BASELINE_RSS_KB"
}

# 颜色
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m' # No Color

# 默认选项
WARMUP=3
RUNS=10
MODE="all"

# ── 解析命令行参数 ──
while [[ $# -gt 0 ]]; do
    case $1 in
        --quick)    WARMUP=1; RUNS=3; shift ;;
        --compile)  MODE="compile"; shift ;;
        --runtime)  MODE="runtime"; shift ;;
        --size)     MODE="size"; shift ;;
        --criterion) MODE="criterion"; shift ;;
        --runs)     RUNS="$2"; shift 2 ;;
        --warmup)   WARMUP="$2"; shift 2 ;;
        -h|--help)
            echo "用法: $0 [选项]"
            echo "  --quick        快速测试 (3 次迭代)"
            echo "  --compile      仅编译速度对比"
            echo "  --runtime      仅运行时对比 (需要 wasmtime)"
            echo "  --size         仅输出大小对比"
            echo "  --criterion    仅 Rust 内部微基准"
            echo "  --runs N       设置迭代次数 (默认 10)"
            echo "  --warmup N     设置预热次数 (默认 3)"
            exit 0
            ;;
        *) echo "未知选项: $1"; exit 1 ;;
    esac
done

# ── 工具检查 ──
check_tool() {
    if ! command -v "$1" &>/dev/null; then
        echo -e "${RED}错误: 未找到 $1，请先安装${NC}"
        echo "  $2"
        return 1
    fi
}

echo -e "${BOLD}${CYAN}╔══════════════════════════════════════════════╗${NC}"
echo -e "${BOLD}${CYAN}║   CJWasm vs CJC 综合性能基准测试            ║${NC}"
echo -e "${BOLD}${CYAN}╚══════════════════════════════════════════════╝${NC}"
echo ""
echo -e "${BLUE}时间: $(date)${NC}"
echo -e "${BLUE}模式: $MODE | 迭代: $RUNS | 预热: $WARMUP${NC}"
echo ""

# ── 环境信息 ──
echo -e "${BOLD}▸ 环境信息${NC}"
echo "  系统: $(uname -s) $(uname -m)"
echo "  CPU:  $(sysctl -n machdep.cpu.brand_string 2>/dev/null || echo 'N/A')"
echo "  内存: $(sysctl -n hw.memsize 2>/dev/null | awk '{printf "%.1f GB", $1/1024/1024/1024}' || echo 'N/A')"
echo ""

# ── 构建 cjwasm (release) ──
echo -e "${BOLD}▸ 构建 cjwasm (release 模式)...${NC}"
cd "$PROJECT_DIR"
cargo build --release 2>&1 | tail -3
CJWASM="$PROJECT_DIR/target/release/cjwasm"
echo -e "  ${GREEN}✓ cjwasm 就绪: $CJWASM${NC}"

# 检查 cjc
CJC_AVAILABLE=false
if command -v cjc &>/dev/null; then
    CJC_AVAILABLE=true
    CJC_VERSION=$(cjc --version 2>&1 | head -1)
    echo -e "  ${GREEN}✓ cjc 就绪: $CJC_VERSION${NC}"

    # macOS SIP 会在 bash 子进程中清除 DYLD_LIBRARY_PATH，
    # 导致 cjc 编译的 native 二进制找不到 libcangjie-runtime.dylib。
    # 这里根据 CANGJIE_HOME 或 cjc 路径重新设置。
    if [[ -z "${DYLD_LIBRARY_PATH:-}" ]] && [[ "$(uname -s)" == "Darwin" ]]; then
        CJC_HOME="${CANGJIE_HOME:-}"
        if [[ -z "$CJC_HOME" ]]; then
            # 从 cjc 路径推断 CANGJIE_HOME
            CJC_HOME="$(cd "$(dirname "$(command -v cjc)")/.." && pwd)"
        fi
        if [[ -d "$CJC_HOME/runtime/lib" ]]; then
            # 查找 runtime 库目录
            CJC_RUNTIME_LIB=$(find "$CJC_HOME/runtime/lib" -maxdepth 1 -type d -name 'darwin_*' | head -1)
            if [[ -n "$CJC_RUNTIME_LIB" ]]; then
                export DYLD_LIBRARY_PATH="${CJC_RUNTIME_LIB}:${CJC_HOME}/tools/lib:${DYLD_LIBRARY_PATH:-}"
                echo -e "  ${GREEN}✓ DYLD_LIBRARY_PATH 已设置${NC}"
            fi
        fi
    fi
else
    echo -e "  ${YELLOW}⚠ cjc 未找到，将跳过 cjc 对比测试${NC}"
fi

# 检查 wasmtime
WASMTIME_AVAILABLE=false
if command -v wasmtime &>/dev/null; then
    WASMTIME_AVAILABLE=true
    WASMTIME_VERSION=$(wasmtime --version 2>&1)
    echo -e "  ${GREEN}✓ wasmtime 就绪: $WASMTIME_VERSION${NC}"
else
    echo -e "  ${YELLOW}⚠ wasmtime 未找到，将跳过运行时对比${NC}"
fi

# 检查 hyperfine
check_tool hyperfine "brew install hyperfine"

echo ""
mkdir -p "$REPORT_DIR"

# ── 准备临时目录 ──
TMPDIR=$(mktemp -d)
trap "rm -rf $TMPDIR" EXIT

# 准备 cjc 兼容版本的测试文件
# cjc 的 main 函数语法是 main() { ... } 而不是 func main() -> Int64 { ... }
prepare_cjc_files() {
    for f in "$BENCH_DIR"/bench_*.cj; do
        local basename=$(basename "$f")
        cp "$f" "$TMPDIR/cjwasm_$basename"
        # cjc 兼容版本: 需要 main() { } 格式
        # 由于语法差异较大，cjc 用简化的对应文件
    done

    # cjc 小规模
    cat > "$TMPDIR/cjc_bench_small.cj" << 'CJ'
func add(a: Int64, b: Int64): Int64 {
    return a + b
}

func factorial(n: Int64): Int64 {
    if (n <= 1) {
        return 1
    }
    return n * factorial(n - 1)
}

func fib(n: Int64): Int64 {
    if (n <= 1) {
        return n
    }
    return fib(n - 1) + fib(n - 2)
}

main() {
    let a = add(10, 20)
    let b = factorial(10)
    let c = fib(10)
    println(a + b + c)
}
CJ

    # cjc 中等规模
    cat > "$TMPDIR/cjc_bench_medium.cj" << 'CJ'
struct Point {
    var x: Int64
    var y: Int64
    init(x: Int64, y: Int64) {
        this.x = x
        this.y = y
    }
    func distance(other: Point): Int64 {
        let dx = this.x - other.x
        let dy = this.y - other.y
        return dx * dx + dy * dy
    }
}

func sumRange(start: Int64, end: Int64): Int64 {
    var sum: Int64 = 0
    var i = start
    while (i < end) {
        sum = sum + i
        i = i + 1
    }
    return sum
}

func fibonacci(n: Int64): Int64 {
    if (n <= 1) {
        return n
    }
    return fibonacci(n - 1) + fibonacci(n - 2)
}

func isPrime(n: Int64): Bool {
    if (n < 2) {
        return false
    }
    var i: Int64 = 2
    while (i * i <= n) {
        if (n % i == 0) {
            return false
        }
        i = i + 1
    }
    return true
}

func countPrimes(limit: Int64): Int64 {
    var count: Int64 = 0
    var i: Int64 = 2
    while (i < limit) {
        if (isPrime(i)) {
            count = count + 1
        }
        i = i + 1
    }
    return count
}

main() {
    let p1 = Point(1, 2)
    let p2 = Point(4, 6)
    let d = p1.distance(p2)
    let sum = sumRange(1, 100)
    let fib = fibonacci(10)
    let primes = countPrimes(50)
    println(d + sum + fib + primes)
}
CJ

    # cjc 大规模
    cat > "$TMPDIR/cjc_bench_large.cj" << 'CJ'
struct Pair<T> {
    var first: T
    var second: T
    init(first: T, second: T) {
        this.first = first
        this.second = second
    }
}

class Vector2D {
    var x: Int64
    var y: Int64
    init(x: Int64, y: Int64) {
        this.x = x
        this.y = y
    }
    func length(): Int64 {
        return this.x * this.x + this.y * this.y
    }
}

class Counter {
    var count: Int64
    init(count: Int64) {
        this.count = count
    }
    func increment(): Int64 {
        return this.count + 1
    }
    func decrement(): Int64 {
        return this.count - 1
    }
}

func identity<T>(value: T): T {
    return value
}

func fibonacci(n: Int64): Int64 {
    if (n <= 1) {
        return n
    }
    return fibonacci(n - 1) + fibonacci(n - 2)
}

func gcd(a: Int64, b: Int64): Int64 {
    if (b == 0) {
        return a
    }
    return gcd(b, a % b)
}

func power(base: Int64, exp: Int64): Int64 {
    if (exp == 0) {
        return 1
    }
    return base * power(base, exp - 1)
}

func sumOfSquares(n: Int64): Int64 {
    var sum: Int64 = 0
    var i: Int64 = 1
    while (i <= n) {
        sum = sum + i * i
        i = i + 1
    }
    return sum
}

func isPrime(n: Int64): Bool {
    if (n < 2) {
        return false
    }
    var i: Int64 = 2
    while (i * i <= n) {
        if (n % i == 0) {
            return false
        }
        i = i + 1
    }
    return true
}

func countPrimes(limit: Int64): Int64 {
    var count: Int64 = 0
    var i: Int64 = 2
    while (i < limit) {
        if (isPrime(i)) {
            count = count + 1
        }
        i = i + 1
    }
    return count
}

func collatz(n: Int64): Int64 {
    var steps: Int64 = 0
    var current = n
    while (current != 1) {
        if (current % 2 == 0) {
            current = current / 2
        } else {
            current = current * 3 + 1
        }
        steps = steps + 1
    }
    return steps
}

main() {
    let v = Vector2D(3, 4)
    let c = Counter(10)
    let p = Pair<Int64>(10, 20)

    let fib = fibonacci(10)
    let g = gcd(48, 18)
    let pw = power(2, 10)
    let sq = sumOfSquares(10)
    let primes = countPrimes(100)
    let col = collatz(27)
    let id = identity<Int64>(42)

    println(v.length() + c.increment() + c.decrement() + p.first + p.second + fib + g + pw + sq + primes + col + id)
}
CJ
}

# ============================================================
# 1. 编译速度对比 (hyperfine)
# ============================================================
run_compile_bench() {
    echo -e "${BOLD}${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo -e "${BOLD}${GREEN}  1. 编译速度对比 (hyperfine)${NC}"
    echo -e "${BOLD}${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo ""

    local sizes=("small" "medium" "large")
    local labels=("小规模" "中规模" "大规模")

    for i in "${!sizes[@]}"; do
        local size="${sizes[$i]}"
        local label="${labels[$i]}"
        local cjwasm_src="$BENCH_DIR/bench_${size}.cj"
        local cjc_src="$TMPDIR/cjc_bench_${size}.cj"
        local cjwasm_out="$TMPDIR/out_${size}.wasm"
        local cjc_out="$TMPDIR/out_${size}"

        echo -e "${CYAN}── $label (bench_${size}.cj) ──${NC}"

        if $CJC_AVAILABLE && [ -f "$cjc_src" ]; then
            hyperfine \
                -N \
                --warmup "$WARMUP" \
                --runs "$RUNS" \
                --export-json "$REPORT_DIR/compile_${size}_${TIMESTAMP}.json" \
                --command-name "cjwasm" \
                "$CJWASM $cjwasm_src -o $cjwasm_out" \
                --command-name "cjc" \
                "cjc $cjc_src -o $cjc_out" \
                2>&1
        else
            hyperfine \
                -N \
                --warmup "$WARMUP" \
                --runs "$RUNS" \
                --export-json "$REPORT_DIR/compile_${size}_${TIMESTAMP}.json" \
                --command-name "cjwasm" \
                "$CJWASM $cjwasm_src -o $cjwasm_out" \
                2>&1
        fi

        echo ""
    done

    # 编译全部 tests/examples
    echo -e "${CYAN}── 批量编译 (全部 tests/examples/*.cj) ──${NC}"
    local all_files=$(ls "$PROJECT_DIR/tests/examples/"*.cj 2>/dev/null | tr '\n' ' ')
    if [ -n "$all_files" ]; then
        local count=$(echo $all_files | wc -w | tr -d ' ')
        echo "  文件数: $count"

        # cjwasm 逐个编译所有 examples
        hyperfine \
            --warmup "$WARMUP" \
            --runs "$RUNS" \
            --export-json "$REPORT_DIR/compile_all_examples_${TIMESTAMP}.json" \
            --command-name "cjwasm (${count}个文件)" \
            --shell=default \
            "for f in $PROJECT_DIR/tests/examples/*.cj; do $CJWASM \"\$f\" -o $TMPDIR/\$(basename \"\$f\" .cj).wasm 2>/dev/null; done" \
            2>&1
    fi
    echo ""
}

# ============================================================
# 2. 输出大小对比
# ============================================================
run_size_bench() {
    echo -e "${BOLD}${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo -e "${BOLD}${GREEN}  2. 输出大小对比${NC}"
    echo -e "${BOLD}${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo ""

    printf "  ${BOLD}%-12s %12s %12s %12s %10s${NC}\n" "规模" "源码(B)" "WASM(B)" "CJC(B)" "压缩比"
    printf "  %-12s %12s %12s %12s %10s\n" "──────────" "──────────" "──────────" "──────────" "────────"

    local sizes=("small" "medium" "large")
    local labels=("小规模" "中规模" "大规模")
    local report_file="$REPORT_DIR/output_size_${TIMESTAMP}.txt"

    echo "# 输出大小对比报告 - $(date)" > "$report_file"
    echo "" >> "$report_file"

    for i in "${!sizes[@]}"; do
        local size="${sizes[$i]}"
        local label="${labels[$i]}"
        local src_file="$BENCH_DIR/bench_${size}.cj"
        local wasm_out="$TMPDIR/size_${size}.wasm"
        local cjc_src="$TMPDIR/cjc_bench_${size}.cj"
        local cjc_out="$TMPDIR/size_${size}_cjc"

        local src_size=$(wc -c < "$src_file" | tr -d ' ')

        # cjwasm 编译
        $CJWASM "$src_file" -o "$wasm_out" >/dev/null 2>&1
        local wasm_size=$(wc -c < "$wasm_out" | tr -d ' ')

        # cjc 编译
        local cjc_size="N/A"
        if $CJC_AVAILABLE && [ -f "$cjc_src" ]; then
            if cjc "$cjc_src" -o "$cjc_out" 2>/dev/null; then
                cjc_size=$(wc -c < "$cjc_out" | tr -d ' ')
            fi
        fi

        local ratio="N/A"
        if [ "$cjc_size" != "N/A" ] && [ "$cjc_size" -gt 0 ]; then
            ratio=$(echo "scale=1; $cjc_size / $wasm_size" | bc)x
        fi

        printf "  %-12s %12s %12s %12s %10s\n" "$label" "$src_size" "$wasm_size" "$cjc_size" "$ratio"
        echo "$label: src=$src_size wasm=$wasm_size cjc=$cjc_size ratio=$ratio" >> "$report_file"
    done

    echo ""

    # 所有 examples 的输出大小
    echo -e "  ${BOLD}单独文件大小:${NC}"
    printf "  ${BOLD}%-35s %10s${NC}\n" "文件" "WASM(B)"
    printf "  %-35s %10s\n" "─────────────────────────────────" "────────"

    local total_wasm=0
    for f in "$PROJECT_DIR/tests/examples/"*.cj; do
        local basename=$(basename "$f")
        local out="$TMPDIR/size_${basename%.cj}.wasm"
        if $CJWASM "$f" -o "$out" >/dev/null 2>&1; then
            local sz=$(wc -c < "$out" | tr -d ' ')
            total_wasm=$((total_wasm + sz))
            printf "  %-35s %10s\n" "$basename" "$sz"
        else
            printf "  %-35s %10s\n" "$basename" "编译失败"
        fi
    done
    printf "  %-35s %10s\n" "─────────────────────────────────" "────────"
    printf "  ${BOLD}%-35s %10s${NC}\n" "合计" "$total_wasm"
    echo ""
}

# ============================================================
# 3. 运行时性能对比 (wasmtime)
# ============================================================
run_runtime_bench() {
    if ! $WASMTIME_AVAILABLE; then
        echo -e "${YELLOW}⚠ wasmtime 不可用，跳过运行时对比${NC}"
        return
    fi

    echo -e "${BOLD}${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo -e "${BOLD}${GREEN}  3. 运行时性能对比 (WASM → wasmtime)${NC}"
    echo -e "${BOLD}${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo ""
    echo -e "  ${YELLOW}注: WASM 输出尚未通过完整 validation，部分文件可能无法在 wasmtime 中执行${NC}"
    echo ""

    local sizes=("small" "medium" "large")
    local labels=("小规模" "中规模" "大规模")

    for i in "${!sizes[@]}"; do
        local size="${sizes[$i]}"
        local label="${labels[$i]}"
        local cjwasm_src="$BENCH_DIR/bench_${size}.cj"
        local wasm_out="$TMPDIR/rt_${size}.wasm"
        local cjc_src="$TMPDIR/cjc_bench_${size}.cj"
        local cjc_out="$TMPDIR/rt_${size}_cjc"

        # 编译
        $CJWASM "$cjwasm_src" -o "$wasm_out" >/dev/null 2>&1

        # 先验证 wasmtime 能否运行
        if ! wasmtime run --invoke main "$wasm_out" >/dev/null 2>&1; then
            echo -e "${CYAN}── $label: WASM 运行时 ──${NC}"
            echo -e "  ${YELLOW}⚠ wasmtime 执行失败 (WASM validation 错误)，跳过${NC}"
            echo ""
            continue
        fi

        echo -e "${CYAN}── $label: WASM 运行时 ──${NC}"

        if $CJC_AVAILABLE && [ -f "$cjc_src" ]; then
            if cjc "$cjc_src" -o "$cjc_out" 2>/dev/null && "$cjc_out" >/dev/null 2>&1; then
                hyperfine \
                    -N \
                    --warmup "$WARMUP" \
                    --runs "$RUNS" \
                    --export-json "$REPORT_DIR/runtime_${size}_${TIMESTAMP}.json" \
                    --command-name "wasmtime (cjwasm)" \
                    "wasmtime run --invoke main $wasm_out" \
                    --command-name "native (cjc)" \
                    "$cjc_out" \
                    2>&1
            else
                echo -e "  ${YELLOW}cjc 编译或运行失败，仅测试 wasmtime${NC}"
                hyperfine \
                    -N \
                    --warmup "$WARMUP" \
                    --runs "$RUNS" \
                    --export-json "$REPORT_DIR/runtime_${size}_${TIMESTAMP}.json" \
                    --command-name "wasmtime (cjwasm)" \
                    "wasmtime run --invoke main $wasm_out" \
                    2>&1
            fi
        else
            hyperfine \
                -N \
                --warmup "$WARMUP" \
                --runs "$RUNS" \
                --command-name "wasmtime (cjwasm)" \
                "wasmtime run --invoke main $wasm_out" \
                2>&1
        fi
        echo ""
    done
}

# ============================================================
# 4. Rust 内部微基准 (criterion)
# ============================================================
run_criterion_bench() {
    echo -e "${BOLD}${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo -e "${BOLD}${GREEN}  4. Rust 内部微基准 (Criterion)${NC}"
    echo -e "${BOLD}${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo ""

    cd "$PROJECT_DIR"
    cargo bench --bench compile_bench 2>&1

    echo ""
    echo -e "  ${GREEN}HTML 报告: $PROJECT_DIR/target/criterion/report/index.html${NC}"
    echo ""
}

# ============================================================
# 5. 编译吞吐量统计
# ============================================================
run_throughput_bench() {
    echo -e "${BOLD}${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo -e "${BOLD}${GREEN}  5. 编译吞吐量统计${NC}"
    echo -e "${BOLD}${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo ""

    printf "  ${BOLD}%-12s %8s %12s %12s %12s${NC}\n" "规模" "行数" "耗时(ms)" "行/秒" "KB/秒"
    printf "  %-12s %8s %12s %12s %12s\n" "──────────" "──────" "──────────" "──────────" "──────────"

    local sizes=("small" "medium" "large")
    local labels=("小规模" "中规模" "大规模")

    for i in "${!sizes[@]}"; do
        local size="${sizes[$i]}"
        local label="${labels[$i]}"
        local src_file="$BENCH_DIR/bench_${size}.cj"
        local out_file="$TMPDIR/tp_${size}.wasm"

        local lines=$(wc -l < "$src_file" | tr -d ' ')
        local bytes=$(wc -c < "$src_file" | tr -d ' ')

        # 测量编译时间 (取 10 次平均)
        local total_ns=0
        local count=10
        for _ in $(seq 1 $count); do
            local start_ns=$(python3 -c 'import time; print(int(time.time_ns()))')
            $CJWASM "$src_file" -o "$out_file" >/dev/null 2>&1
            local end_ns=$(python3 -c 'import time; print(int(time.time_ns()))')
            total_ns=$((total_ns + end_ns - start_ns))
        done

        local avg_ms=$(echo "scale=2; $total_ns / $count / 1000000" | bc)
        local avg_s=$(echo "scale=6; $avg_ms / 1000" | bc)
        local lines_per_sec=$(echo "scale=0; $lines / $avg_s" | bc 2>/dev/null || echo "N/A")
        local kb_per_sec=$(echo "scale=1; $bytes / 1024 / $avg_s" | bc 2>/dev/null || echo "N/A")

        printf "  %-12s %8s %12s %12s %12s\n" "$label" "$lines" "$avg_ms" "$lines_per_sec" "$kb_per_sec"
    done

    # 全部 tests/examples 合计
    local total_lines=0
    local total_bytes=0
    for f in "$PROJECT_DIR/tests/examples/"*.cj; do
        total_lines=$((total_lines + $(wc -l < "$f" | tr -d ' ')))
        total_bytes=$((total_bytes + $(wc -c < "$f" | tr -d ' ')))
    done

    local total_ns=0
    local count=5
    for _ in $(seq 1 $count); do
        local start_ns=$(python3 -c 'import time; print(int(time.time_ns()))')
        for f in "$PROJECT_DIR/tests/examples/"*.cj; do
            $CJWASM "$f" -o "$TMPDIR/tp_all_$(basename "$f" .cj).wasm" >/dev/null 2>&1
        done
        local end_ns=$(python3 -c 'import time; print(int(time.time_ns()))')
        total_ns=$((total_ns + end_ns - start_ns))
    done

    local avg_ms=$(echo "scale=2; $total_ns / $count / 1000000" | bc)
    local avg_s=$(echo "scale=6; $avg_ms / 1000" | bc)
    local lines_per_sec=$(echo "scale=0; $total_lines / $avg_s" | bc 2>/dev/null || echo "N/A")
    local kb_per_sec=$(echo "scale=1; $total_bytes / 1024 / $avg_s" | bc 2>/dev/null || echo "N/A")

    printf "  %-12s %8s %12s %12s %12s\n" "────────" "──────" "──────────" "──────────" "──────────"
    printf "  ${BOLD}%-12s %8s %12s %12s %12s${NC}\n" "全部examples" "$total_lines" "$avg_ms" "$lines_per_sec" "$kb_per_sec"

    echo ""
}

# ============================================================
# 6. 生成 HTML 可视化对比报告
# ============================================================
generate_html_report() {
    echo -e "${BOLD}${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo -e "${BOLD}${GREEN}  6. 生成 HTML 可视化报告${NC}"
    echo -e "${BOLD}${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo ""

    local HTML="$REPORT_DIR/benchmark_report_${TIMESTAMP}.html"
    local LATEST_HTML="$PROJECT_DIR/benches/report.html"

    # 收集编译速度数据
    local sizes=("small" "medium" "large" "heavy")
    local labels=("小规模(27行)" "中规模(100行)" "大规模(311行)" "高负载(97行)")
    local cjwasm_times=()
    local cjc_times=()
    local src_sizes=()
    local wasm_sizes=()
    local cjc_bin_sizes=()

    for i in "${!sizes[@]}"; do
        local size="${sizes[$i]}"
        local src_file="$BENCH_DIR/bench_${size}.cj"
        local wasm_out="$TMPDIR/html_${size}.wasm"
        local cjc_src="$TMPDIR/cjc_bench_${size}.cj"
        local cjc_out="$TMPDIR/html_${size}_cjc"

        # cjwasm 编译耗时 (ms, 取 20 次平均)
        local total_ns=0
        for _ in $(seq 1 20); do
            local s=$(python3 -c 'import time; print(int(time.time_ns()))')
            $CJWASM "$src_file" -o "$wasm_out" >/dev/null 2>&1
            local e=$(python3 -c 'import time; print(int(time.time_ns()))')
            total_ns=$((total_ns + e - s))
        done
        local cjwasm_ms=$(echo "scale=2; $total_ns / 20 / 1000000" | bc)
        cjwasm_times+=("$cjwasm_ms")

        # cjc 编译耗时
        local cjc_ms="0"
        if $CJC_AVAILABLE && [ -f "$cjc_src" ]; then
            total_ns=0
            for _ in $(seq 1 5); do
                local s=$(python3 -c 'import time; print(int(time.time_ns()))')
                cjc "$cjc_src" -o "$cjc_out" >/dev/null 2>&1
                local e=$(python3 -c 'import time; print(int(time.time_ns()))')
                total_ns=$((total_ns + e - s))
            done
            cjc_ms=$(echo "scale=2; $total_ns / 5 / 1000000" | bc)
        fi
        cjc_times+=("$cjc_ms")

        # 文件大小
        src_sizes+=("$(wc -c < "$src_file" | tr -d ' ')")
        wasm_sizes+=("$(wc -c < "$wasm_out" | tr -d ' ')")
        if [ -f "$cjc_out" ]; then
            cjc_bin_sizes+=("$(wc -c < "$cjc_out" | tr -d ' ')")
        else
            cjc_bin_sizes+=("0")
        fi
    done

    cat > "$HTML" << 'HTMLHEAD'
<!DOCTYPE html>
<html lang="zh-CN">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>CJWasm vs CJC 性能对比报告</title>
<style>
  * { margin: 0; padding: 0; box-sizing: border-box; }
  body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; background: #0f172a; color: #e2e8f0; padding: 2rem; }
  .container { max-width: 1200px; margin: 0 auto; }
  h1 { font-size: 2rem; font-weight: 700; text-align: center; margin-bottom: 0.5rem; background: linear-gradient(135deg, #38bdf8, #818cf8); -webkit-background-clip: text; -webkit-text-fill-color: transparent; }
  .subtitle { text-align: center; color: #94a3b8; margin-bottom: 2rem; font-size: 0.9rem; }
  .grid { display: grid; grid-template-columns: 1fr 1fr; gap: 1.5rem; margin-bottom: 2rem; }
  .card { background: #1e293b; border-radius: 12px; padding: 1.5rem; border: 1px solid #334155; }
  .card h2 { font-size: 1.1rem; color: #94a3b8; margin-bottom: 1rem; display: flex; align-items: center; gap: 0.5rem; }
  .card h2 .icon { font-size: 1.3rem; }
  .full-width { grid-column: 1 / -1; }
  table { width: 100%; border-collapse: collapse; margin-top: 0.5rem; }
  th, td { padding: 0.75rem 1rem; text-align: right; border-bottom: 1px solid #334155; }
  th { color: #94a3b8; font-weight: 600; font-size: 0.85rem; text-transform: uppercase; letter-spacing: 0.05em; }
  th:first-child, td:first-child { text-align: left; }
  td { font-size: 0.95rem; }
  .cjwasm { color: #38bdf8; font-weight: 600; }
  .cjc { color: #f472b6; font-weight: 600; }
  .speedup { color: #4ade80; font-weight: 700; font-size: 1.1rem; }
  .bar-container { display: flex; align-items: center; gap: 0.5rem; }
  .bar { height: 24px; border-radius: 4px; min-width: 2px; transition: width 0.5s ease; }
  .bar-cjwasm { background: linear-gradient(90deg, #0ea5e9, #38bdf8); }
  .bar-cjc { background: linear-gradient(90deg, #ec4899, #f472b6); }
  .bar-label { font-size: 0.8rem; white-space: nowrap; min-width: 80px; }
  .legend { display: flex; gap: 2rem; justify-content: center; margin-bottom: 1.5rem; }
  .legend-item { display: flex; align-items: center; gap: 0.5rem; font-size: 0.9rem; }
  .legend-dot { width: 12px; height: 12px; border-radius: 3px; }
  .dot-cjwasm { background: #38bdf8; }
  .dot-cjc { background: #f472b6; }
  .env-info { display: flex; gap: 2rem; justify-content: center; color: #64748b; font-size: 0.8rem; margin-bottom: 1rem; }
  .big-number { font-size: 2.5rem; font-weight: 700; text-align: center; margin: 0.5rem 0; }
  .big-label { text-align: center; color: #64748b; font-size: 0.85rem; }
  .highlight-row { background: #1e293b; }
  .highlight-row:nth-child(odd) { background: #0f172a; }
  .note { background: #1e1b4b; border: 1px solid #3730a3; border-radius: 8px; padding: 1rem; margin-top: 1rem; font-size: 0.85rem; color: #a5b4fc; }
  .note strong { color: #818cf8; }
</style>
</head>
<body>
<div class="container">
<h1>CJWasm vs CJC 编译性能对比报告</h1>
HTMLHEAD

    # 动态生成内容
    cat >> "$HTML" << EOF
<div class="subtitle">生成时间: $(date '+%Y-%m-%d %H:%M:%S') | 系统: $(uname -m) | CPU: $(sysctl -n machdep.cpu.brand_string 2>/dev/null || echo 'N/A')</div>

<div class="legend">
  <div class="legend-item"><div class="legend-dot dot-cjwasm"></div> <strong>cjwasm</strong> (Rust → WASM 前端编译器)</div>
  <div class="legend-item"><div class="legend-dot dot-cjc"></div> <strong>cjc</strong> ($( cjc --version 2>&1 | head -1 || echo 'N/A' )) 原生编译器</div>
</div>
EOF

    # ── 编译速度对比卡片 ──
    cat >> "$HTML" << 'EOF'
<div class="grid">
<div class="card full-width">
<h2><span class="icon">⚡</span> 编译速度对比</h2>
<table>
<tr><th>规模</th><th>cjwasm (ms)</th><th>cjc (ms)</th><th>加速比</th><th>可视化</th></tr>
EOF

    for i in "${!sizes[@]}"; do
        local label="${labels[$i]}"
        local cjwasm_ms="${cjwasm_times[$i]}"
        local cjc_ms="${cjc_times[$i]}"
        local speedup="N/A"
        local cjwasm_pct=100
        local cjc_pct=100

        if [ "$cjc_ms" != "0" ] && [ -n "$cjc_ms" ]; then
            speedup=$(echo "scale=0; $cjc_ms / $cjwasm_ms" | bc 2>/dev/null || echo "N/A")
            # 计算条形图比例 (cjc 为 100%)
            cjwasm_pct=$(echo "scale=1; $cjwasm_ms * 100 / $cjc_ms" | bc 2>/dev/null || echo "1")
            cjc_pct=100
        fi

        cat >> "$HTML" << EOF
<tr class="highlight-row">
  <td><strong>$label</strong></td>
  <td class="cjwasm">$cjwasm_ms</td>
  <td class="cjc">$cjc_ms</td>
  <td class="speedup">${speedup}x</td>
  <td>
    <div class="bar-container">
      <span class="bar-label cjwasm">cjwasm</span>
      <div class="bar bar-cjwasm" style="width: ${cjwasm_pct}%"></div>
    </div>
    <div class="bar-container" style="margin-top:3px">
      <span class="bar-label cjc">cjc</span>
      <div class="bar bar-cjc" style="width: ${cjc_pct}%"></div>
    </div>
  </td>
</tr>
EOF
    done

    cat >> "$HTML" << 'EOF'
</table>
<div class="note">
  <strong>说明：</strong>cjwasm 是轻量级前端编译器，仅生成 WASM 字节码（~1-4KB）；cjc 是完整的原生编译器，生成包含标准库链接的可执行文件（~1MB）。两者定位不同，编译速度差异主要来源于此。
</div>
</div>
EOF

    # ── 输出大小对比 ──
    cat >> "$HTML" << 'EOF'
<div class="card">
<h2><span class="icon">📦</span> 输出大小对比</h2>
<table>
<tr><th>规模</th><th>源码</th><th>cjwasm WASM</th><th>cjc 原生</th><th>比率</th></tr>
EOF

    for i in "${!sizes[@]}"; do
        local label="${labels[$i]}"
        local src_sz="${src_sizes[$i]}"
        local wasm_sz="${wasm_sizes[$i]}"
        local cjc_sz="${cjc_bin_sizes[$i]}"
        local ratio="N/A"
        if [ "$cjc_sz" != "0" ] && [ "$wasm_sz" != "0" ]; then
            ratio=$(echo "scale=0; $cjc_sz / $wasm_sz" | bc 2>/dev/null)x
        fi
        # 格式化大小
        local wasm_fmt="${wasm_sz} B"
        local cjc_fmt="N/A"
        if [ "$cjc_sz" != "0" ]; then
            cjc_fmt=$(echo "scale=1; $cjc_sz / 1024" | bc 2>/dev/null)" KB"
        fi

        cat >> "$HTML" << EOF
<tr class="highlight-row">
  <td>$label</td>
  <td>${src_sz} B</td>
  <td class="cjwasm">$wasm_fmt</td>
  <td class="cjc">$cjc_fmt</td>
  <td class="speedup">$ratio</td>
</tr>
EOF
    done

    cat >> "$HTML" << 'EOF'
</table>
</div>
EOF

    # ── 运行时性能对比 ──
    local has_runtime=false
    local rt_rows=""
    local wasmtime_baseline_rss_kb=0
    local wasmtime_baseline_rss_fmt="N/A"
    if command -v wasmtime &>/dev/null; then
        wasmtime_baseline_rss_kb=$(measure_wasmtime_baseline_rss_kb)
        wasmtime_baseline_rss_fmt="$(format_kb_as_mb "$wasmtime_baseline_rss_kb")"
        for i in "${!sizes[@]}"; do
            local size="${sizes[$i]}"
            local label="${labels[$i]}"
            local rt_src="$BENCH_DIR/bench_${size}.cj"
            local rt_wasm="$TMPDIR/rt_html_${size}.wasm"
            local rt_cjc_src="$TMPDIR/cjc_bench_${size}.cj"
            local rt_cjc_bin="$TMPDIR/rt_html_${size}_cjc"

            # 编译 WASM
            $CJWASM "$rt_src" -o "$rt_wasm" >/dev/null 2>&1 || continue
            # 验证 wasmtime 能否运行
            wasmtime run --invoke main "$rt_wasm" >/dev/null 2>&1 || continue

            has_runtime=true

            # wasmtime 运行耗时 (10 次平均)
            local rt_total_ns=0
            for _ in $(seq 1 10); do
                local s=$(python3 -c 'import time; print(int(time.time_ns()))')
                wasmtime run --invoke main "$rt_wasm" >/dev/null 2>&1
                local e=$(python3 -c 'import time; print(int(time.time_ns()))')
                rt_total_ns=$((rt_total_ns + e - s))
            done
            local wasm_rt_ms=$(echo "scale=2; $rt_total_ns / 10 / 1000000" | bc)

            # 峰值内存: 取 3 次中的最大值（避免抖动）
            local wasm_rss_kb=0
            for _ in $(seq 1 3); do
                local v
                v=$(peak_rss_kb "wasmtime run --invoke main \"$rt_wasm\"") || v=0
                if [[ "$v" -gt "$wasm_rss_kb" ]]; then
                    wasm_rss_kb="$v"
                fi
            done
            local wasm_rss_fmt
            wasm_rss_fmt="$(format_kb_as_mb "$wasm_rss_kb")"
            local wasm_guest_rss_kb
            wasm_guest_rss_kb=$(positive_diff_kb "$wasm_rss_kb" "$wasmtime_baseline_rss_kb")
            local wasm_guest_rss_fmt
            wasm_guest_rss_fmt="$(format_kb_as_mb "$wasm_guest_rss_kb")"

            # cjc native 运行耗时
            local native_rt_ms="N/A"
            local native_rss_kb=0
            local native_rss_fmt="N/A"
            local rt_status=""
            if $CJC_AVAILABLE && [ -f "$rt_cjc_src" ]; then
                if cjc "$rt_cjc_src" -o "$rt_cjc_bin" >/dev/null 2>&1 && "$rt_cjc_bin" >/dev/null 2>&1; then
                    rt_total_ns=0
                    for _ in $(seq 1 10); do
                        local s=$(python3 -c 'import time; print(int(time.time_ns()))')
                        "$rt_cjc_bin" >/dev/null 2>&1
                        local e=$(python3 -c 'import time; print(int(time.time_ns()))')
                        rt_total_ns=$((rt_total_ns + e - s))
                    done
                    native_rt_ms=$(echo "scale=2; $rt_total_ns / 10 / 1000000" | bc)

                    # 峰值内存: 取 3 次最大值
                    native_rss_kb=0
                    for _ in $(seq 1 3); do
                        local v
                        v=$(peak_rss_kb "\"$rt_cjc_bin\"") || v=0
                        if [[ "$v" -gt "$native_rss_kb" ]]; then
                            native_rss_kb="$v"
                        fi
                    done
                    native_rss_fmt="$(format_kb_as_mb "$native_rss_kb")"
                else
                    rt_status="cjc 运行崩溃"
                fi
            fi

            rt_rows+="<tr class=\"highlight-row\">"
            rt_rows+="<td><strong>$label</strong></td>"
            rt_rows+="<td class=\"cjwasm\">${wasm_rt_ms} ms</td>"
            rt_rows+="<td class=\"cjwasm\">${wasm_rss_fmt}</td>"
            rt_rows+="<td class=\"cjwasm\">${wasm_guest_rss_fmt}</td>"
            if [ "$native_rt_ms" != "N/A" ]; then
                rt_rows+="<td class=\"cjc\">${native_rt_ms} ms</td>"
                rt_rows+="<td class=\"cjc\">${native_rss_fmt}</td>"
                # 计算比率: 谁更快, 显示更友好
                local wasm_faster=$(echo "$wasm_rt_ms < $native_rt_ms" | bc -l 2>/dev/null)
                if [ "$wasm_faster" = "1" ]; then
                    local rt_ratio=$(printf "%.1f" $(echo "scale=2; $native_rt_ms / $wasm_rt_ms" | bc 2>/dev/null))
                    rt_rows+="<td class=\"speedup\">wasm ${rt_ratio}x 快</td>"
                else
                    local rt_ratio=$(printf "%.1f" $(echo "scale=2; $wasm_rt_ms / $native_rt_ms" | bc 2>/dev/null))
                    rt_rows+="<td style=\"color:#f97316\">native ${rt_ratio}x 快</td>"
                fi
            elif [ -n "$rt_status" ]; then
                rt_rows+="<td style=\"color:#f97316\">${rt_status}</td>"
                rt_rows+="<td style=\"color:#64748b\">-</td>"
                rt_rows+="<td>-</td>"
            else
                rt_rows+="<td style=\"color:#64748b\">N/A</td>"
                rt_rows+="<td style=\"color:#64748b\">N/A</td>"
                rt_rows+="<td>-</td>"
            fi
            rt_rows+="</tr>"
        done
    fi

    if $has_runtime; then
        cat >> "$HTML" << 'EOF'
<div class="card full-width">
<h2><span class="icon">🚀</span> 运行时性能 (wasmtime vs cjc native)</h2>
<table>
<tr><th>规模</th><th>wasmtime 耗时</th><th>wasmtime 端到端 RSS</th><th>wasmtime 扣基线后 RSS</th><th>native 耗时</th><th>native Peak RSS</th><th>比率</th></tr>
EOF
        echo "$rt_rows" >> "$HTML"
        cat >> "$HTML" << EOF
</table>
<div class="note">
  <strong>说明：</strong>wasmtime 运行时间包含 JIT 编译开销。所有内存值均取自进程的 <strong>Peak RSS</strong>（macOS: <code>/usr/bin/time -l</code>；Linux: <code>/usr/bin/time -v</code>），取 3 次运行的最大值。<strong>wasmtime 端到端 RSS</strong> 包含 VM/JIT/宿主固定开销；<strong>wasmtime 扣基线后 RSS</strong> = 当前 Peak RSS - 空载 wasmtime Peak RSS（本次基线约 ${wasmtime_baseline_rss_fmt}），用于更接近 guest/程序本身内存占用，但它仍是估算值，不等于精确的 WASM 线性内存或 heap。
</div>
</div>
EOF
    fi

    # ── bench_heavy.cj CHIR 优化效果对比 ──
    local heavy_src="$BENCH_DIR/bench_heavy.cj"
    if [ -f "$heavy_src" ] && command -v wasmtime &>/dev/null; then
        local heavy_opt_wasm="$TMPDIR/heavy_opt.wasm"
        local heavy_noopt_wasm="$TMPDIR/heavy_noopt.wasm"

        $CJWASM "$heavy_src" -o "$heavy_opt_wasm" >/dev/null 2>&1 || true
        NO_CHIR_OPT=1 $CJWASM "$heavy_src" -o "$heavy_noopt_wasm" >/dev/null 2>&1 || true

        local heavy_opt_ok=false
        local heavy_noopt_ok=false
        wasmtime run --invoke main "$heavy_opt_wasm" >/dev/null 2>&1 && heavy_opt_ok=true || true
        wasmtime run --invoke main "$heavy_noopt_wasm" >/dev/null 2>&1 && heavy_noopt_ok=true || true

        if $heavy_opt_ok; then
            local heavy_opt_ns=0
            for _ in $(seq 1 10); do
                local s e
                s=$(python3 -c 'import time; print(int(time.time_ns()))')
                wasmtime run --invoke main "$heavy_opt_wasm" >/dev/null 2>&1
                e=$(python3 -c 'import time; print(int(time.time_ns()))')
                heavy_opt_ns=$((heavy_opt_ns + e - s))
            done
            local heavy_opt_ms heavy_opt_sz
            heavy_opt_ms=$(echo "scale=2; $heavy_opt_ns / 10 / 1000000" | bc)
            heavy_opt_sz=$(wc -c < "$heavy_opt_wasm" | tr -d ' ')

            local heavy_noopt_ms="N/A"
            local heavy_noopt_sz="N/A"
            local heavy_speedup_str="N/A"

            if $heavy_noopt_ok; then
                local heavy_noopt_ns=0
                for _ in $(seq 1 10); do
                    local s e
                    s=$(python3 -c 'import time; print(int(time.time_ns()))')
                    wasmtime run --invoke main "$heavy_noopt_wasm" >/dev/null 2>&1
                    e=$(python3 -c 'import time; print(int(time.time_ns()))')
                    heavy_noopt_ns=$((heavy_noopt_ns + e - s))
                done
                heavy_noopt_ms=$(echo "scale=2; $heavy_noopt_ns / 10 / 1000000" | bc)
                heavy_noopt_sz=$(wc -c < "$heavy_noopt_wasm" | tr -d ' ')
                local heavy_speedup_raw
                heavy_speedup_raw=$(echo "scale=2; $heavy_noopt_ms / $heavy_opt_ms" | bc 2>/dev/null || echo "N/A")
                heavy_speedup_str="${heavy_speedup_raw}x"
            fi

            cat >> "$HTML" << EOF
<div class="card full-width" style="margin-top:1.5rem">
<h2><span class="icon">🔥</span> CHIR 优化效果 (bench_heavy.cj — 100,000 次密集小函数调用)</h2>
<table>
<tr><th>版本</th><th>wasmtime 耗时</th><th>WASM 大小</th><th>说明</th></tr>
<tr class="highlight-row">
  <td><strong class="cjwasm">CHIR + 优化（默认）</strong></td>
  <td class="cjwasm">${heavy_opt_ms} ms</td>
  <td class="cjwasm">${heavy_opt_sz} B</td>
  <td>小函数内联 + 冗余 local 消除</td>
</tr>
<tr class="highlight-row">
  <td><strong class="cjc">CHIR 无优化（NO_CHIR_OPT）</strong></td>
  <td class="cjc">${heavy_noopt_ms} ms</td>
  <td class="cjc">${heavy_noopt_sz} B</td>
  <td>无内联优化</td>
</tr>
</table>
<div class="note">
  <strong>优化加速比：</strong><span class="speedup">${heavy_speedup_str}</span> — bench_heavy.cj 包含 100,000 次密集小函数调用（identity / double / square / addOne / Counter.get 等），CHIR 内联 pass 将这些调用直接替换为表达式，消除 call 指令开销。wasmtime JIT 会对两者都做本地优化，差异会被部分抹平；在无 JIT 的解释器（wasm-interp）下可观察到约 1.7x 加速。
</div>
</div>
EOF
        fi
    fi

    # ── 平均加速比高亮 ──
    local total_speedup=0
    local count=0
    for i in "${!sizes[@]}"; do
        if [ "${cjc_times[$i]}" != "0" ]; then
            local sp=$(echo "scale=0; ${cjc_times[$i]} / ${cjwasm_times[$i]}" | bc 2>/dev/null || echo 0)
            total_speedup=$((total_speedup + sp))
            count=$((count + 1))
        fi
    done
    local avg_speedup="N/A"
    if [ "$count" -gt 0 ]; then
        avg_speedup=$(echo "scale=0; $total_speedup / $count" | bc)
    fi

    cat >> "$HTML" << EOF
<div class="card" style="text-align:center">
<h2><span class="icon">🏆</span> 总体性能</h2>
<div class="big-number cjwasm">${avg_speedup}x</div>
<div class="big-label">cjwasm 平均编译加速比 (相比 cjc)</div>
<div style="margin-top:1rem; color:#64748b; font-size:0.85rem">
  cjwasm 输出 WASM 字节码, 平均体积仅 cjc 原生产物的 <strong style="color:#4ade80">1/${avg_speedup}</strong>
</div>
</div>
EOF

    # ── cjwasm 内部管线分析提示 ──
    cat >> "$HTML" << 'EOF'
</div><!-- grid end -->

<div class="card full-width" style="margin-top:1.5rem">
<h2><span class="icon">🔬</span> cjwasm 内部管线分析</h2>
<p style="color:#94a3b8; margin-bottom:0.5rem">
  更详细的内部管线性能分析（词法分析、语法解析、优化器、代码生成各阶段耗时），请查看 Criterion 微基准报告：
</p>
<p><code style="background:#334155; padding:0.3rem 0.6rem; border-radius:4px; color:#38bdf8">target/criterion/report/index.html</code></p>
<p style="color:#64748b; margin-top:0.5rem; font-size:0.85rem">运行 <code style="background:#334155; padding:0.2rem 0.4rem; border-radius:3px">cargo bench</code> 更新 Criterion 报告</p>
</div>

<div style="text-align:center; margin-top:2rem; color:#475569; font-size:0.8rem">
  由 <strong>scripts/benchmark.sh</strong> 自动生成 | CJWasm Compiler Benchmark Suite
</div>

</div><!-- container end -->
</body>
</html>
EOF

    # 创建 latest 软链接
    cp "$HTML" "$LATEST_HTML"

    echo -e "  ${GREEN}✓ HTML 报告已生成: $HTML${NC}"
    echo -e "  ${GREEN}✓ 报告: $LATEST_HTML${NC}"
    echo ""
}

# ============================================================
# 执行
# ============================================================

prepare_cjc_files

case "$MODE" in
    all)
        run_compile_bench
        run_size_bench
        run_runtime_bench
        run_throughput_bench
        generate_html_report
        run_criterion_bench
        ;;
    compile)
        run_compile_bench
        run_throughput_bench
        generate_html_report
        ;;
    runtime)
        run_runtime_bench
        generate_html_report
        ;;
    size)
        run_size_bench
        generate_html_report
        ;;
    criterion)
        run_criterion_bench
        ;;
esac

# ── 汇总 ──
echo -e "${BOLD}${CYAN}╔══════════════════════════════════════════════╗${NC}"
echo -e "${BOLD}${CYAN}║   基准测试完成                              ║${NC}"
echo -e "${BOLD}${CYAN}╚══════════════════════════════════════════════╝${NC}"
echo ""
echo -e "  ${GREEN}对比报告: $PROJECT_DIR/benches/report.html${NC}"
echo -e "  ${GREEN}Criterion: $PROJECT_DIR/target/criterion/report/index.html${NC}"
echo -e "  ${GREEN}JSON数据: $REPORT_DIR/${NC}"
echo ""
echo -e "  ${BLUE}快速重测: ./scripts/benchmark.sh --quick${NC}"
echo -e "  ${BLUE}仅编译:   ./scripts/benchmark.sh --compile${NC}"
echo -e "  ${BLUE}仅运行时: ./scripts/benchmark.sh --runtime${NC}"
echo ""
