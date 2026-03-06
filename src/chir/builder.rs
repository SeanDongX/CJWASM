//! CHIR 构建器 - 辅助构建 CHIR 结构

use super::types::*;
use crate::ast::{BinOp, UnaryOp, Type};
use wasm_encoder::ValType;

/// CHIR 构建器
pub struct CHIRBuilder {
    next_local: u32,
}

impl CHIRBuilder {
    /// 创建新的构建器
    pub fn new() -> Self {
        CHIRBuilder { next_local: 0 }
    }

    /// 分配新的局部变量索引
    pub fn alloc_local(&mut self) -> u32 {
        let idx = self.next_local;
        self.next_local += 1;
        idx
    }

    /// 重置局部变量计数器
    pub fn reset_locals(&mut self) {
        self.next_local = 0;
    }

    // === 表达式构建 ===

    /// 整数常量
    pub fn int_const(&self, value: i64, ty: Type) -> CHIRExpr {
        CHIRExpr::int_const(value, ty)
    }

    /// 布尔常量
    pub fn bool_const(&self, value: bool) -> CHIRExpr {
        CHIRExpr::bool_const(value)
    }

    /// 字符串常量
    pub fn string_const(&self, value: String) -> CHIRExpr {
        CHIRExpr::new(
            CHIRExprKind::String(value),
            Type::String,
            ValType::I32,
        )
    }

    /// 局部变量引用
    pub fn local_get(&self, idx: u32, ty: Type) -> CHIRExpr {
        let wasm_ty = ty.to_wasm();
        CHIRExpr::new(CHIRExprKind::Local(idx), ty, wasm_ty)
    }

    /// 二元运算
    pub fn binary(&self, op: BinOp, left: CHIRExpr, right: CHIRExpr, result_ty: Type) -> CHIRExpr {
        let wasm_ty = result_ty.to_wasm();
        CHIRExpr::new(
            CHIRExprKind::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            },
            result_ty,
            wasm_ty,
        )
    }

    /// 一元运算
    pub fn unary(&self, op: UnaryOp, expr: CHIRExpr, result_ty: Type) -> CHIRExpr {
        let wasm_ty = result_ty.to_wasm();
        CHIRExpr::new(
            CHIRExprKind::Unary {
                op,
                expr: Box::new(expr),
            },
            result_ty,
            wasm_ty,
        )
    }

    /// 函数调用
    pub fn call(&self, func_idx: u32, args: Vec<CHIRExpr>, return_ty: Type) -> CHIRExpr {
        let wasm_ty = return_ty.to_wasm();
        CHIRExpr::new(
            CHIRExprKind::Call { func_idx, args },
            return_ty,
            wasm_ty,
        )
    }

    /// 类型转换
    pub fn cast(&self, expr: CHIRExpr, to_ty: Type) -> CHIRExpr {
        let from_wasm_ty = expr.wasm_ty;
        let to_wasm_ty = to_ty.to_wasm();

        // 如果类型相同，不需要转换
        if from_wasm_ty == to_wasm_ty {
            return expr;
        }

        CHIRExpr::new(
            CHIRExprKind::Cast {
                expr: Box::new(expr),
                from_ty: from_wasm_ty,
                to_ty: to_wasm_ty,
            },
            to_ty.clone(),
            to_wasm_ty,
        )
    }

    /// If 表达式
    pub fn if_expr(
        &self,
        cond: CHIRExpr,
        then_block: CHIRBlock,
        else_block: Option<CHIRBlock>,
        result_ty: Type,
    ) -> CHIRExpr {
        let wasm_ty = result_ty.to_wasm();
        CHIRExpr::new(
            CHIRExprKind::If {
                cond: Box::new(cond),
                then_block,
                else_block,
            },
            result_ty,
            wasm_ty,
        )
    }

    /// 块表达式
    pub fn block(&self, block: CHIRBlock, result_ty: Type) -> CHIRExpr {
        let wasm_ty = result_ty.to_wasm();
        CHIRExpr::new(CHIRExprKind::Block(block), result_ty, wasm_ty)
    }

    // === 语句构建 ===

    /// Let 语句
    pub fn let_stmt(&mut self, value: CHIRExpr) -> (u32, CHIRStmt) {
        let local_idx = self.alloc_local();
        let stmt = CHIRStmt::Let { local_idx, value };
        (local_idx, stmt)
    }

    /// 赋值语句
    pub fn assign(&self, target: CHIRLValue, value: CHIRExpr) -> CHIRStmt {
        CHIRStmt::Assign { target, value }
    }

    /// 表达式语句
    pub fn expr_stmt(&self, expr: CHIRExpr) -> CHIRStmt {
        CHIRStmt::Expr(expr)
    }

    /// Return 语句
    pub fn return_stmt(&self, value: Option<CHIRExpr>) -> CHIRStmt {
        CHIRStmt::Return(value)
    }

    // === 块构建 ===

    /// 创建空块
    pub fn empty_block(&self) -> CHIRBlock {
        CHIRBlock::empty()
    }

    /// 从语句列表创建块
    pub fn block_from_stmts(&self, stmts: Vec<CHIRStmt>) -> CHIRBlock {
        CHIRBlock {
            stmts,
            result: None,
        }
    }

    /// 从语句列表和结果表达式创建块
    pub fn block_from_stmts_and_result(
        &self,
        stmts: Vec<CHIRStmt>,
        result: CHIRExpr,
    ) -> CHIRBlock {
        CHIRBlock {
            stmts,
            result: Some(Box::new(result)),
        }
    }

    // === 函数构建 ===

    /// 创建函数
    pub fn function(
        &mut self,
        name: String,
        params: Vec<CHIRParam>,
        return_ty: Type,
        body: CHIRBlock,
    ) -> CHIRFunction {
        let return_wasm_ty = return_ty.to_wasm();
        CHIRFunction {
            name,
            params,
            return_ty,
            return_wasm_ty,
            locals: Vec::new(),
            body,
        }
    }
}

impl Default for CHIRBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builder_basic() {
        let builder = CHIRBuilder::new();

        // 创建简单表达式: 42
        let expr = builder.int_const(42, Type::Int64);
        assert!(matches!(expr.kind, CHIRExprKind::Integer(42)));
        assert_eq!(expr.wasm_ty, ValType::I64);
    }

    #[test]
    fn test_builder_binary() {
        let builder = CHIRBuilder::new();

        // 创建: 1 + 2
        let left = builder.int_const(1, Type::Int64);
        let right = builder.int_const(2, Type::Int64);
        let expr = builder.binary(BinOp::Add, left, right, Type::Int64);

        assert!(matches!(expr.kind, CHIRExprKind::Binary { .. }));
        assert_eq!(expr.wasm_ty, ValType::I64);
    }

    #[test]
    fn test_builder_cast() {
        let builder = CHIRBuilder::new();

        // 创建: (Int32)42
        let expr = builder.int_const(42, Type::Int64);
        let casted = builder.cast(expr, Type::Int32);

        assert!(matches!(casted.kind, CHIRExprKind::Cast { .. }));
        assert_eq!(casted.wasm_ty, ValType::I32);
    }

    #[test]
    fn test_builder_function() {
        let mut builder = CHIRBuilder::new();

        // 创建函数: func test(): Int64 { return 42; }
        let expr = builder.int_const(42, Type::Int64);
        let stmt = builder.return_stmt(Some(expr));
        let block = builder.block_from_stmts(vec![stmt]);

        let func = builder.function(
            "test".to_string(),
            vec![],
            Type::Int64,
            block,
        );

        assert_eq!(func.name, "test");
        assert_eq!(func.return_wasm_ty, ValType::I64);
        assert_eq!(func.body.stmts.len(), 1);
    }
}
