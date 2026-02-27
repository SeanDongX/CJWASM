#!/bin/bash
# 提取所有 Parse* 方法签名

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
CJC_PARSER_HEADER="$PROJECT_ROOT/third_party/cangjie_compiler/include/cangjie/Parse/Parser.h"
OUTPUT_FILE="$PROJECT_ROOT/docs/cjc_parser_methods.txt"

if [ ! -f "$CJC_PARSER_HEADER" ]; then
    echo "Error: CJC Parser header not found at $CJC_PARSER_HEADER"
    exit 1
fi

echo "Extracting Parser methods from cjc..."

# 提取所有 Parse* 方法
grep -E "^\s*(OwnedPtr<.*>|bool|void|std::.*)\s+Parse\w+" "$CJC_PARSER_HEADER" \
    | sed 's/^\s*//g' > "$OUTPUT_FILE"

echo "Extracted $(wc -l < "$OUTPUT_FILE") parser methods"
echo "Output: $OUTPUT_FILE"
echo ""
echo "Sample methods:"
head -10 "$OUTPUT_FILE"
