//! AST → CHIR 表达式转换

use crate::ast::{Expr, Type};
use crate::chir::{CHIRExpr, CHIRExprKind, CHIRBlock, CHIRMatchArm, CHIRPattern};
use crate::chir::type_inference::TypeInferenceContext;
use std::collections::HashMap;
use wasm_encoder::ValType;

/// 类型转换构造函数 → WASM ValType（如 Float32(x), Int64(x)）
fn type_cast_wasm(name: &str) -> Option<ValType> {
    match name {
        "Float32" => Some(ValType::F32),
        "Float64" => Some(ValType::F64),
        "Int32" | "UInt32" => Some(ValType::I32),
        "Int64" | "UInt64" => Some(ValType::I64),
        _ => None,
    }
}

/// 降低（Lowering）上下文
pub struct LoweringContext<'a> {
    /// 类型推断上下文
    pub type_ctx: &'a TypeInferenceContext,

    /// 变量名 → 局部变量索引
    local_map: HashMap<String, u32>,

    /// 函数名 → 函数索引
    func_indices: &'a HashMap<String, u32>,

    /// 结构体字段偏移
    struct_field_offsets: &'a HashMap<String, HashMap<String, u32>>,

    /// 类字段偏移
    #[allow(dead_code)]
    class_field_offsets: &'a HashMap<String, HashMap<String, u32>>,

    /// 下一个可用的局部变量索引
    next_local: u32,

    /// 当前函数返回值的 WASM 类型（用于 Return 语句的类型强制转换）
    pub return_wasm_ty: Option<ValType>,
}

impl<'a> LoweringContext<'a> {
    /// 创建新的降低上下文
    pub fn new(
        type_ctx: &'a TypeInferenceContext,
        func_indices: &'a HashMap<String, u32>,
        struct_field_offsets: &'a HashMap<String, HashMap<String, u32>>,
        class_field_offsets: &'a HashMap<String, HashMap<String, u32>>,
    ) -> Self {
        LoweringContext {
            type_ctx,
            local_map: HashMap::new(),
            func_indices,
            struct_field_offsets,
            class_field_offsets,
            next_local: 0,
            return_wasm_ty: None,
        }
    }

    /// 分配新的局部变量索引
    pub fn alloc_local(&mut self, name: String) -> u32 {
        let idx = self.next_local;
        self.next_local += 1;
        self.local_map.insert(name, idx);
        idx
    }

    /// 获取局部变量索引
    pub fn get_local(&self, name: &str) -> Option<u32> {
        self.local_map.get(name).copied()
    }

    /// 降低表达式
    pub fn lower_expr(&mut self, expr: &Expr) -> Result<CHIRExpr, String> {
        // 先推断类型
        let ty = self.type_ctx.infer_expr(expr)?;
        // Unit/Nothing 不映射到 WASM 值类型，用 I32 占位（不会被实际使用）
        let wasm_ty = match &ty {
            crate::ast::Type::Unit | crate::ast::Type::Nothing => wasm_encoder::ValType::I32,
            t => t.to_wasm(),
        };

        let kind = match expr {
            // 字面量
            Expr::Integer(n) => CHIRExprKind::Integer(*n),
            Expr::Float(f) => CHIRExprKind::Float(*f),
            Expr::Float32(f) => CHIRExprKind::Float32(*f),
            Expr::Bool(b) => CHIRExprKind::Bool(*b),
            Expr::String(s) => CHIRExprKind::String(s.clone()),
            Expr::Rune(c) => CHIRExprKind::Rune(*c),

            // 变量
            Expr::Var(name) => {
                if let Some(local_idx) = self.local_map.get(name) {
                    CHIRExprKind::Local(*local_idx)
                } else {
                    // 全局变量或未定义
                    CHIRExprKind::Global(name.clone())
                }
            }

            // 二元运算
            Expr::Binary { op, left, right } => {
                let left_chir = self.lower_expr(left)?;
                let right_chir = self.lower_expr(right)?;

                // 插入类型转换（如果需要）
                let left_chir = self.insert_cast_if_needed(left_chir, wasm_ty);
                let right_chir = self.insert_cast_if_needed(right_chir, wasm_ty);

                CHIRExprKind::Binary {
                    op: op.clone(),
                    left: Box::new(left_chir),
                    right: Box::new(right_chir),
                }
            }

            // 一元运算
            Expr::Unary { op, expr: inner } => {
                let inner_chir = self.lower_expr(inner)?;
                CHIRExprKind::Unary {
                    op: op.clone(),
                    expr: Box::new(inner_chir),
                }
            }

            // 函数调用
            Expr::Call { name, args, .. } => {
                // 类型转换构造函数：Float32(x), Float64(x), Int32(x), Int64(x)
                if let Some(to_wasm) = type_cast_wasm(name) {
                    if let Some(arg) = args.first() {
                        let inner = self.lower_expr(arg)?;
                        let from_ty = inner.wasm_ty;
                        let to_ty = to_wasm;
                        if from_ty == to_ty {
                            return Ok(inner);
                        }
                        let ast_ty = ty.clone();
                        return Ok(CHIRExpr {
                            kind: CHIRExprKind::Cast { expr: Box::new(inner), from_ty, to_ty },
                            ty: ast_ty,
                            wasm_ty: to_ty,
                            span: None,
                        });
                    }
                }

                let func_idx = self.func_indices.get(name.as_str())
                    .copied()
                    .unwrap_or(0);

                // 查询函数签名以获取参数类型，用于插入必要的类型转换
                let param_tys: Vec<ValType> = self.type_ctx.functions
                    .get(name.as_str())
                    .map(|sig| sig.params.iter().map(|p| match p {
                        crate::ast::Type::Unit | crate::ast::Type::Nothing => ValType::I32,
                        t => t.to_wasm(),
                    }).collect())
                    .unwrap_or_default();

                let args_chir: Result<Vec<CHIRExpr>, String> = args.iter()
                    .enumerate()
                    .map(|(i, a)| {
                        let mut arg_chir = self.lower_expr(a)?;
                        // 若有已知的参数类型，插入强制转换
                        if let Some(&target_wasm) = param_tys.get(i) {
                            arg_chir = self.insert_cast_if_needed(arg_chir, target_wasm);
                        }
                        Ok(arg_chir)
                    })
                    .collect();

                CHIRExprKind::Call {
                    func_idx,
                    args: args_chir?,
                }
            }

            // 方法调用
            Expr::MethodCall { object, method: _, args, .. } => {
                let receiver = self.lower_expr(object)?;
                let args_chir: Result<Vec<_>, _> = args.iter()
                    .map(|a| self.lower_expr(a))
                    .collect();

                // 简化：暂不解析 vtable 偏移
                CHIRExprKind::MethodCall {
                    vtable_offset: None,
                    func_idx: None,
                    receiver: Box::new(receiver),
                    args: args_chir?,
                }
            }

            // 字段访问
            Expr::Field { object, field } => {
                let obj_chir = self.lower_expr(object)?;
                let obj_ty = self.type_ctx.infer_expr(object)?;

                // 获取字段偏移
                let offset = self.get_field_offset(&obj_ty, field)?;
                let field_ty = self.type_ctx.infer_field_type(&obj_ty, field)?;

                CHIRExprKind::FieldGet {
                    object: Box::new(obj_chir),
                    field_offset: offset,
                    field_ty,
                }
            }

            // 数组
            Expr::Array(elems) => {
                // 简化：转换为 ArrayNew
                let len = CHIRExpr::new(
                    CHIRExprKind::Integer(elems.len() as i64),
                    Type::Int64,
                    ValType::I64,
                );

                // 默认初始化为 0
                let init = CHIRExpr::new(
                    CHIRExprKind::Integer(0),
                    Type::Int64,
                    ValType::I64,
                );

                CHIRExprKind::ArrayNew {
                    len: Box::new(len),
                    init: Box::new(init),
                }
            }

            // 数组索引
            Expr::Index { array, index } => {
                let array_chir = self.lower_expr(array)?;
                let index_chir = self.lower_expr(index)?;

                CHIRExprKind::ArrayGet {
                    array: Box::new(array_chir),
                    index: Box::new(index_chir),
                }
            }

            // 元组
            Expr::Tuple(elems) => {
                let elems_chir: Result<Vec<_>, _> = elems.iter()
                    .map(|e| self.lower_expr(e))
                    .collect();

                CHIRExprKind::TupleNew {
                    elements: elems_chir?,
                }
            }

            // 元组索引
            Expr::TupleIndex { object, index } => {
                let tuple_chir = self.lower_expr(object)?;

                CHIRExprKind::TupleGet {
                    tuple: Box::new(tuple_chir),
                    index: *index as usize,
                }
            }

            // 结构体初始化
            Expr::StructInit { name, fields, .. } => {
                let fields_chir: Result<Vec<(String, CHIRExpr)>, String> = fields.iter()
                    .map(|(fname, fexpr)| {
                        let fchir = self.lower_expr(fexpr)?;
                        Ok((fname.clone(), fchir))
                    })
                    .collect();

                CHIRExprKind::StructNew {
                    struct_name: name.clone(),
                    fields: fields_chir?,
                }
            }

            // 构造函数调用
            Expr::ConstructorCall { name, args, .. } => {
                // 类型转换构造函数
                if let Some(to_wasm) = type_cast_wasm(name) {
                    if let Some(arg) = args.first() {
                        let inner = self.lower_expr(arg)?;
                        let from_ty = inner.wasm_ty;
                        let to_ty = to_wasm;
                        if from_ty == to_ty {
                            return Ok(inner);
                        }
                        let ast_ty = ty.clone();
                        return Ok(CHIRExpr {
                            kind: CHIRExprKind::Cast { expr: Box::new(inner), from_ty, to_ty },
                            ty: ast_ty,
                            wasm_ty: to_ty,
                            span: None,
                        });
                    }
                }
                let func_idx = self.func_indices.get(name.as_str())
                    .copied()
                    .unwrap_or(0);

                let param_tys: Vec<ValType> = self.type_ctx.functions
                    .get(name.as_str())
                    .map(|sig| sig.params.iter().map(|p| match p {
                        crate::ast::Type::Unit | crate::ast::Type::Nothing => ValType::I32,
                        t => t.to_wasm(),
                    }).collect())
                    .unwrap_or_default();

                let args_chir: Result<Vec<CHIRExpr>, String> = args.iter()
                    .enumerate()
                    .map(|(i, a)| {
                        let mut arg_chir = self.lower_expr(a)?;
                        if let Some(&target_wasm) = param_tys.get(i) {
                            arg_chir = self.insert_cast_if_needed(arg_chir, target_wasm);
                        }
                        Ok(arg_chir)
                    })
                    .collect();

                CHIRExprKind::Call {
                    func_idx,
                    args: args_chir?,
                }
            }

            // If 表达式
            Expr::If { cond, then_branch, else_branch } => {
                let cond_chir = self.lower_expr(cond)?;
                let then_block = self.lower_expr_to_block(then_branch)?;
                let else_block = if let Some(else_expr) = else_branch {
                    Some(self.lower_expr_to_block(else_expr)?)
                } else {
                    None
                };

                CHIRExprKind::If {
                    cond: Box::new(cond_chir),
                    then_block,
                    else_block,
                }
            }

            // Match 表达式
            Expr::Match { expr: subject, arms } => {
                let subject_chir = self.lower_expr(subject)?;
                let arms_chir: Result<Vec<_>, _> = arms.iter()
                    .map(|arm| self.lower_match_arm(arm))
                    .collect();

                CHIRExprKind::Match {
                    subject: Box::new(subject_chir),
                    arms: arms_chir?,
                }
            }

            // 其他表达式暂时返回 Nop
            _ => CHIRExprKind::Nop,
        };

        Ok(CHIRExpr {
            kind,
            ty,
            wasm_ty,
            span: None,
        })
    }

    /// 插入类型转换（如果需要）
    pub fn insert_cast_if_needed(&self, expr: CHIRExpr, target_ty: ValType) -> CHIRExpr {
        if expr.wasm_ty == target_ty {
            return expr;
        }

        let from_ty = expr.wasm_ty;
        let ty = expr.ty.clone();

        CHIRExpr {
            kind: CHIRExprKind::Cast {
                expr: Box::new(expr),
                from_ty,
                to_ty: target_ty,
            },
            ty,
            wasm_ty: target_ty,
            span: None,
        }
    }

    /// 获取字段偏移（对未知结构体或未知字段均返回 0，避免 lowering 中断）
    pub fn get_field_offset(&self, obj_ty: &Type, field: &str) -> Result<u32, String> {
        match obj_ty {
            Type::Struct(name, _) => {
                let offset = self.struct_field_offsets
                    .get(name.as_str())
                    .and_then(|fields| fields.get(field).copied())
                    .unwrap_or(0);
                Ok(offset)
            }
            _ => Ok(0),
        }
    }

    /// 将表达式转换为块
    pub fn lower_expr_to_block(&mut self, expr: &Expr) -> Result<CHIRBlock, String> {
        // Block 表达式直接转换为 CHIRBlock（保留语句）
        if let Expr::Block(stmts, block_result) = expr {
            let mut block = self.lower_stmts_to_block(stmts)?;
            if let Some(result_expr) = block_result {
                if block.result.is_none() {
                    block.result = Some(Box::new(self.lower_expr(result_expr)?));
                }
            }
            return Ok(block);
        }
        let chir_expr = self.lower_expr(expr)?;
        Ok(CHIRBlock {
            stmts: Vec::new(),
            result: Some(Box::new(chir_expr)),
        })
    }

    /// 降低 Match 分支
    fn lower_match_arm(&mut self, _arm: &crate::ast::MatchArm) -> Result<CHIRMatchArm, String> {
        // 简化：返回通配符模式
        Ok(CHIRMatchArm {
            pattern: CHIRPattern::Wildcard,
            guard: None,
            body: CHIRBlock::empty(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Program, BinOp};

    #[test]
    fn test_lower_integer() {
        let type_ctx = TypeInferenceContext::new();
        let func_indices = HashMap::new();
        let struct_offsets = HashMap::new();
        let class_offsets = HashMap::new();

        let mut ctx = LoweringContext::new(
            &type_ctx,
            &func_indices,
            &struct_offsets,
            &class_offsets,
        );

        let expr = Expr::Integer(42);
        let chir = ctx.lower_expr(&expr).unwrap();

        assert!(matches!(chir.kind, CHIRExprKind::Integer(42)));
        assert_eq!(chir.wasm_ty, ValType::I64);
    }

    #[test]
    fn test_lower_binary() {
        let type_ctx = TypeInferenceContext::new();
        let func_indices = HashMap::new();
        let struct_offsets = HashMap::new();
        let class_offsets = HashMap::new();

        let mut ctx = LoweringContext::new(
            &type_ctx,
            &func_indices,
            &struct_offsets,
            &class_offsets,
        );

        let expr = Expr::Binary {
            op: BinOp::Add,
            left: Box::new(Expr::Integer(1)),
            right: Box::new(Expr::Integer(2)),
        };

        let chir = ctx.lower_expr(&expr).unwrap();

        assert!(matches!(chir.kind, CHIRExprKind::Binary { .. }));
        assert_eq!(chir.wasm_ty, ValType::I64);
    }

    #[test]
    fn test_lower_with_cast() {
        let type_ctx = TypeInferenceContext::new();
        let func_indices = HashMap::new();
        let struct_offsets = HashMap::new();
        let class_offsets = HashMap::new();

        let mut ctx = LoweringContext::new(
            &type_ctx,
            &func_indices,
            &struct_offsets,
            &class_offsets,
        );

        // 创建一个需要类型转换的表达式
        let expr = Expr::Integer(42);
        let chir = ctx.lower_expr(&expr).unwrap();

        // 插入转换
        let casted = ctx.insert_cast_if_needed(chir, ValType::I32);

        assert!(matches!(casted.kind, CHIRExprKind::Cast { .. }));
        assert_eq!(casted.wasm_ty, ValType::I32);
    }
}
