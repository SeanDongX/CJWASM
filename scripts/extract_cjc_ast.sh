#!/bin/bash
# 从 cjc 头文件提取所有 AST 节点定义

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
CJC_AST_HEADER="$PROJECT_ROOT/third_party/cangjie_compiler/include/cangjie/AST/Node.h"
OUTPUT_FILE="$PROJECT_ROOT/docs/cjc_ast_nodes_list.txt"

if [ ! -f "$CJC_AST_HEADER" ]; then
    echo "Error: CJC AST header not found at $CJC_AST_HEADER"
    exit 1
fi

echo "Extracting AST nodes from cjc..."

# 提取所有 Decl/Expr/Type/Pattern/Stmt 相关的类和结构体
grep -E "^\s*(class|struct|enum)\s+\w+(Decl|Expr|Type|Pattern|Stmt|Lit)" "$CJC_AST_HEADER" \
    | sed 's/class //g; s/struct //g; s/enum //g; s/ :.*//g; s/^\s*//g' \
    | sort -u > "$OUTPUT_FILE"

echo "Extracted $(wc -l < "$OUTPUT_FILE") AST node types"
echo "Output: $OUTPUT_FILE"
echo ""
echo "Top 10 nodes:"
head -10 "$OUTPUT_FILE"
