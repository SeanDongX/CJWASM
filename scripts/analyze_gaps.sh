#!/bin/bash
# 分析 cjwasm 相对于 cjc 的功能缺口

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

cd "$PROJECT_ROOT"

echo "=========================================="
echo "CJWasm vs CJC Feature Gap Analysis"
echo "=========================================="
echo ""

# 1. 提取 cjc AST 节点
if [ ! -f "docs/cjc_ast_nodes_list.txt" ]; then
    echo "Running extract_cjc_ast.sh first..."
    bash scripts/extract_cjc_ast.sh
fi

# 2. 提取 cjwasm AST 节点
echo "Extracting cjwasm AST nodes..."
grep -rh "pub enum\|pub struct" src/ast/ \
    | grep -E "(Decl|Expr|Type|Pattern|Stmt)" \
    | awk '{print $3}' \
    | sed 's/<.*//g; s/{.*//g' \
    | sort -u > docs/cjwasm_ast_nodes_list.txt

echo "CJC AST nodes: $(wc -l < docs/cjc_ast_nodes_list.txt)"
echo "CJWasm AST nodes: $(wc -l < docs/cjwasm_ast_nodes_list.txt)"
echo ""

# 3. 对比差异
echo "=== Missing AST Nodes in CJWasm ==="
comm -23 docs/cjc_ast_nodes_list.txt docs/cjwasm_ast_nodes_list.txt | head -20
MISSING_COUNT=$(comm -23 docs/cjc_ast_nodes_list.txt docs/cjwasm_ast_nodes_list.txt | wc -l)
echo "... ($MISSING_COUNT total missing)"
echo ""

# 4. 提取 cjc parser 方法
if [ ! -f "docs/cjc_parser_methods.txt" ]; then
    echo "Running extract_cjc_parser_methods.sh first..."
    bash scripts/extract_cjc_parser_methods.sh
fi

# 5. 提取 cjwasm parser 方法
echo "Extracting cjwasm parser methods..."
grep -rh "fn parse_" src/parser/ \
    | awk -F'fn ' '{print $2}' \
    | awk '{print $1}' \
    | sed 's/(.*//g' \
    | sort -u > docs/cjwasm_parser_methods_list.txt

# 6. 对比 parser 方法
echo "=== Parser Method Coverage ==="
CJC_METHODS=$(grep -c "Parse" docs/cjc_parser_methods.txt || echo 0)
CJWASM_METHODS=$(wc -l < docs/cjwasm_parser_methods_list.txt)
echo "CJC parser methods: $CJC_METHODS"
echo "CJWasm parser methods: $CJWASM_METHODS"
echo ""

# 7. 统计代码行数
echo "=== Code Size Comparison ==="
CJC_PARSE_LINES=$(find third_party/cangjie_compiler/src/Parse -name "*.cpp" -exec wc -l {} + 2>/dev/null | tail -1 | awk '{print $1}')
CJC_AST_LINES=$(find third_party/cangjie_compiler/src/AST -name "*.cpp" -exec wc -l {} + 2>/dev/null | tail -1 | awk '{print $1}')
CJWASM_PARSER_LINES=$(find src/parser -name "*.rs" -exec wc -l {} + 2>/dev/null | tail -1 | awk '{print $1}')
CJWASM_AST_LINES=$(find src/ast -name "*.rs" -exec wc -l {} + 2>/dev/null | tail -1 | awk '{print $1}')

echo "CJC Parse: $CJC_PARSE_LINES lines"
echo "CJC AST: $CJC_AST_LINES lines"
echo "CJWasm Parser: $CJWASM_PARSER_LINES lines"
echo "CJWasm AST: $CJWASM_AST_LINES lines"
echo ""

# 8. 计算完成度
COMPLETION_PERCENT=$(echo "scale=1; $CJWASM_AST_LINES * 100 / ($CJC_AST_LINES + $CJC_PARSE_LINES)" | bc)
echo "=== Estimated Completion ==="
echo "Code coverage: ~${COMPLETION_PERCENT}%"
echo ""

echo "=========================================="
echo "Analysis complete. Check docs/ for details."
echo "=========================================="
