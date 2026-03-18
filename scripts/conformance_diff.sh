#!/usr/bin/env bash
# Run Cangjie Conformance harness with cjc and cjwasm, then diff reports.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
HARNESS_DIR="$PROJECT_DIR/third_party/cangjie_test/Conformance/Compiler/harness"
TEST_ROOT="$PROJECT_DIR/third_party/cangjie_test/Conformance/Compiler/testsuite"
SHIM="$PROJECT_DIR/scripts/cjwasm_cjc_shim.sh"

CJC_BIN="${CJC_BIN:-cjc}"
CJWASM_BIN="${CJWASM_BIN:-$PROJECT_DIR/target/release/cjwasm}"
REPORT_ROOT="${REPORT_ROOT:-$PROJECT_DIR/target/conformance}"
LOG_MODE="${LOG_MODE:-short}"
COMP_THREADS="${COMP_THREADS:-4}"
BASE_TIMEOUT="${BASE_TIMEOUT:-30}"
BUILD_CJWASM=true
RUN_CJC=true
RUN_CJWASM=true

declare -a TESTS_FILTER=()
LEVEL=""

usage() {
  cat <<'EOF'
Usage: ./scripts/conformance_diff.sh [options]

Options:
  --tests <path>          Add a harness --tests filter path (repeatable)
  --level <n>             Pass harness --level
  --cjc <path>            cjc binary path (default: cjc in PATH)
  --cjwasm <path>         cjwasm binary path (default: target/release/cjwasm)
  --report-root <dir>     Output directory root (default: target/conformance)
  --log-mode <mode>       Harness log mode: progress|short|detailed|verbose
  --comp-threads <n>      Harness compilation thread count (default: 4)
  --base-timeout <sec>    Harness base timeout in seconds (default: 30)
  --no-build              Skip cargo build --release
  --skip-cjc              Skip baseline run with cjc
  --skip-cjwasm           Skip candidate run with cjwasm shim
  -h, --help              Show this help

Examples:
  ./scripts/conformance_diff.sh
  ./scripts/conformance_diff.sh --tests ../testsuite/src/tests/01_lexical_structure
  ./scripts/conformance_diff.sh --level 1 --comp-threads 8
EOF
}

require_cmd() {
  local cmd="$1"
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "missing required command: $cmd" >&2
    exit 127
  fi
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --tests)
      [[ $# -ge 2 ]] || { echo "missing arg for --tests" >&2; exit 2; }
      TESTS_FILTER+=("$2")
      shift 2
      ;;
    --level)
      [[ $# -ge 2 ]] || { echo "missing arg for --level" >&2; exit 2; }
      LEVEL="$2"
      shift 2
      ;;
    --cjc)
      [[ $# -ge 2 ]] || { echo "missing arg for --cjc" >&2; exit 2; }
      CJC_BIN="$2"
      shift 2
      ;;
    --cjwasm)
      [[ $# -ge 2 ]] || { echo "missing arg for --cjwasm" >&2; exit 2; }
      CJWASM_BIN="$2"
      shift 2
      ;;
    --report-root)
      [[ $# -ge 2 ]] || { echo "missing arg for --report-root" >&2; exit 2; }
      REPORT_ROOT="$2"
      shift 2
      ;;
    --log-mode)
      [[ $# -ge 2 ]] || { echo "missing arg for --log-mode" >&2; exit 2; }
      LOG_MODE="$2"
      shift 2
      ;;
    --comp-threads)
      [[ $# -ge 2 ]] || { echo "missing arg for --comp-threads" >&2; exit 2; }
      COMP_THREADS="$2"
      shift 2
      ;;
    --base-timeout)
      [[ $# -ge 2 ]] || { echo "missing arg for --base-timeout" >&2; exit 2; }
      BASE_TIMEOUT="$2"
      shift 2
      ;;
    --no-build)
      BUILD_CJWASM=false
      shift
      ;;
    --skip-cjc)
      RUN_CJC=false
      shift
      ;;
    --skip-cjwasm)
      RUN_CJWASM=false
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown option: $1" >&2
      usage
      exit 2
      ;;
  esac
done

require_cmd python3
[[ -d "$HARNESS_DIR" ]] || { echo "harness dir not found: $HARNESS_DIR" >&2; exit 1; }
[[ -d "$TEST_ROOT" ]] || { echo "testsuite dir not found: $TEST_ROOT" >&2; exit 1; }
[[ -x "$SHIM" ]] || { echo "shim not executable: $SHIM" >&2; exit 1; }

if [[ "$RUN_CJC" == true ]]; then
  if [[ "$CJC_BIN" == */* ]]; then
    [[ -x "$CJC_BIN" ]] || { echo "cjc not executable: $CJC_BIN" >&2; exit 127; }
  else
    require_cmd "$CJC_BIN"
  fi
fi

if [[ "$BUILD_CJWASM" == true ]]; then
  echo "[1/5] building cjwasm (release)..."
  cargo build --release --manifest-path "$PROJECT_DIR/Cargo.toml"
fi

if [[ ! -x "$CJWASM_BIN" ]]; then
  echo "cjwasm not executable: $CJWASM_BIN" >&2
  exit 127
fi

ts="$(date +%Y%m%d_%H%M%S)"
RUN_DIR="$REPORT_ROOT/$ts"
mkdir -p "$RUN_DIR"

cjc_log="$RUN_DIR/cjc.log"
cjwasm_log="$RUN_DIR/cjwasm.log"
cjc_json="$cjc_log.json"
cjwasm_json="$cjwasm_log.json"
diff_txt="$RUN_DIR/diff.txt"

declare -a common_args
common_args=(
  --run-mode compile
  --test-root "$TEST_ROOT"
  --log-mode "$LOG_MODE"
  --comp-threads "$COMP_THREADS"
  --base-timeout "$BASE_TIMEOUT"
)

if [[ -n "$LEVEL" ]]; then
  common_args+=(--level "$LEVEL")
fi

if [[ ${#TESTS_FILTER[@]} -gt 0 ]]; then
  common_args+=(--tests "${TESTS_FILTER[@]}")
fi

if [[ "$RUN_CJC" == true ]]; then
  echo "[2/5] running harness with cjc..."
  (
    cd "$HARNESS_DIR"
    python3 ./harness.py \
      "${common_args[@]}" \
      --cjc "$CJC_BIN" \
      --work-dir "$RUN_DIR/work_cjc" \
      --bin-output "$RUN_DIR/bin_cjc" \
      --test-output "$RUN_DIR/test_res_cjc" \
      --log-file "$cjc_log"
  )
fi

if [[ "$RUN_CJWASM" == true ]]; then
  echo "[3/5] running harness with cjwasm shim..."
  (
    cd "$HARNESS_DIR"
    CJWASM_BIN="$CJWASM_BIN" python3 ./harness.py \
      "${common_args[@]}" \
      --cjc "$SHIM" \
      --work-dir "$RUN_DIR/work_cjwasm" \
      --bin-output "$RUN_DIR/bin_cjwasm" \
      --test-output "$RUN_DIR/test_res_cjwasm" \
      --log-file "$cjwasm_log"
  )
fi

if [[ "$RUN_CJC" == true && "$RUN_CJWASM" == true ]]; then
  [[ -f "$cjc_json" ]] || { echo "missing report: $cjc_json" >&2; exit 1; }
  [[ -f "$cjwasm_json" ]] || { echo "missing report: $cjwasm_json" >&2; exit 1; }
  echo "[4/5] diffing harness reports..."
  (
    cd "$HARNESS_DIR"
    python3 ./run_diff.py "$cjc_json" "$cjwasm_json" -t plain -o "$diff_txt" --no-color
  )
fi

echo "[5/5] done"
echo "output dir: $RUN_DIR"
if [[ -f "$cjc_json" ]]; then
  echo "cjc report: $cjc_json"
fi
if [[ -f "$cjwasm_json" ]]; then
  echo "cjwasm report: $cjwasm_json"
fi
if [[ -f "$diff_txt" ]]; then
  echo "diff report: $diff_txt"
fi
