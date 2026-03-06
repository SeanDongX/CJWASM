//! AST → CHIR 语句转换

use crate::ast::{Stmt, Pattern, AssignTarget};
use crate::chir::{CHIRStmt, CHIRLValue, CHIRBlock};
use super::lower_expr::LoweringContext;

impl<'a> LoweringContext<'a> {
    /// 降低语句
    pub fn lower_stmt(&mut self, stmt: &Stmt) -> Result<CHIRStmt, String> {
        match stmt {
            // Let 语句
            Stmt::Let { pattern, ty, value } => {
                let mut value_chir = self.lower_expr(value)?;
                // 如果有显式类型注解，确保值类型匹配（插入类型转换）
                if let Some(decl_ty) = ty {
                    let decl_wasm = match decl_ty {
                        crate::ast::Type::Unit | crate::ast::Type::Nothing => wasm_encoder::ValType::I32,
                        t => t.to_wasm(),
                    };
                    value_chir = self.insert_cast_if_needed(value_chir, decl_wasm);
                }

                match pattern {
                    Pattern::Binding(name) => {
                        let local_idx = self.alloc_local(name.clone());
                        Ok(CHIRStmt::Let {
                            local_idx,
                            value: value_chir,
                        })
                    }
                    _ => {
                        // 其他模式暂不支持
                        Ok(CHIRStmt::Expr(value_chir))
                    }
                }
            }

            // Var 语句
            Stmt::Var { pattern, ty, value } => {
                let mut value_chir = self.lower_expr(value)?;
                // 如果有显式类型注解，确保值类型匹配（插入类型转换）
                if let Some(decl_ty) = ty {
                    let decl_wasm = match decl_ty {
                        crate::ast::Type::Unit | crate::ast::Type::Nothing => wasm_encoder::ValType::I32,
                        t => t.to_wasm(),
                    };
                    value_chir = self.insert_cast_if_needed(value_chir, decl_wasm);
                }

                match pattern {
                    Pattern::Binding(name) => {
                        let local_idx = self.alloc_local(name.clone());
                        Ok(CHIRStmt::Let {
                            local_idx,
                            value: value_chir,
                        })
                    }
                    _ => {
                        Ok(CHIRStmt::Expr(value_chir))
                    }
                }
            }

            // 赋值语句
            Stmt::Assign { target, value } => {
                let value_chir = self.lower_expr(value)?;
                let target_chir = self.lower_assign_target(target)?;

                Ok(CHIRStmt::Assign {
                    target: target_chir,
                    value: value_chir,
                })
            }

            // 表达式语句
            Stmt::Expr(expr) => {
                let expr_chir = self.lower_expr(expr)?;
                Ok(CHIRStmt::Expr(expr_chir))
            }

            // Return 语句
            Stmt::Return(expr_opt) => {
                let chir_opt = if let Some(expr) = expr_opt {
                    Some(self.lower_expr(expr)?)
                } else {
                    None
                };
                Ok(CHIRStmt::Return(chir_opt))
            }

            // Break 语句
            Stmt::Break => Ok(CHIRStmt::Break),

            // Continue 语句
            Stmt::Continue => Ok(CHIRStmt::Continue),

            // While 语句
            Stmt::While { cond, body } => {
                let cond_chir = self.lower_expr(cond)?;
                let body_block = self.lower_stmts_to_block(body)?;

                Ok(CHIRStmt::While {
                    cond: cond_chir,
                    body: body_block,
                })
            }

            // Loop 语句
            Stmt::Loop { body } => {
                let body_block = self.lower_stmts_to_block(body)?;

                Ok(CHIRStmt::Loop {
                    body: body_block,
                })
            }

            // 其他语句暂时转换为 Nop
            _ => Ok(CHIRStmt::Expr(crate::chir::CHIRExpr::new(
                crate::chir::CHIRExprKind::Nop,
                crate::ast::Type::Unit,
                wasm_encoder::ValType::I32,
            ))),
        }
    }

    /// 降低赋值目标
    fn lower_assign_target(&mut self, target: &AssignTarget) -> Result<CHIRLValue, String> {
        match target {
            AssignTarget::Var(name) => {
                let local_idx = self.get_local(name)
                    .ok_or_else(|| format!("变量未定义: {}", name))?;
                Ok(CHIRLValue::Local(local_idx))
            }

            AssignTarget::Field { object, field } => {
                let obj_expr = crate::ast::Expr::Var(object.clone());
                let obj_chir = self.lower_expr(&obj_expr)?;
                let obj_ty = self.type_ctx.infer_expr(&obj_expr)?;
                let offset = self.get_field_offset(&obj_ty, field)?;

                Ok(CHIRLValue::Field {
                    object: Box::new(obj_chir),
                    offset,
                })
            }

            AssignTarget::Index { array, index } => {
                let array_expr = crate::ast::Expr::Var(array.clone());
                let array_chir = self.lower_expr(&array_expr)?;
                let index_chir = self.lower_expr(index)?;

                Ok(CHIRLValue::Index {
                    array: Box::new(array_chir),
                    index: Box::new(index_chir),
                })
            }

            _ => {
                // 其他赋值目标暂不支持
                Err("不支持的赋值目标".to_string())
            }
        }
    }

    /// 将语句列表转换为块（完整实现）
    pub fn lower_stmts_to_block(&mut self, stmts: &[Stmt]) -> Result<CHIRBlock, String> {
        let mut chir_stmts = Vec::new();
        let mut result = None;

        for (i, stmt) in stmts.iter().enumerate() {
            let is_last = i == stmts.len() - 1;

            // 如果是最后一个语句且是表达式，作为块的结果
            if is_last {
                if let Stmt::Expr(expr) = stmt {
                    let expr_chir = self.lower_expr(expr)?;
                    // 检查是否产生值
                    if !matches!(expr_chir.ty, crate::ast::Type::Unit) {
                        result = Some(Box::new(expr_chir));
                        continue;
                    }
                }
            }

            let chir_stmt = self.lower_stmt(stmt)?;
            chir_stmts.push(chir_stmt);
        }

        Ok(CHIRBlock {
            stmts: chir_stmts,
            result,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Expr, Type};
    use crate::chir::type_inference::TypeInferenceContext;
    use std::collections::HashMap;

    #[test]
    fn test_lower_let() {
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

        let stmt = Stmt::Let {
            pattern: Pattern::Binding("x".to_string()),
            ty: Some(Type::Int64),
            value: Expr::Integer(42),
        };

        let chir = ctx.lower_stmt(&stmt).unwrap();

        assert!(matches!(chir, CHIRStmt::Let { .. }));
    }

    #[test]
    fn test_lower_return() {
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

        let stmt = Stmt::Return(Some(Expr::Integer(42)));
        let chir = ctx.lower_stmt(&stmt).unwrap();

        assert!(matches!(chir, CHIRStmt::Return(Some(_))));
    }

    #[test]
    fn test_lower_block() {
        let mut type_ctx = TypeInferenceContext::new();
        // 预先注册局部变量类型，以便 infer_expr 能识别
        type_ctx.add_local("x".to_string(), Type::Int64);
        let func_indices = HashMap::new();
        let struct_offsets = HashMap::new();
        let class_offsets = HashMap::new();

        let mut ctx = LoweringContext::new(
            &type_ctx,
            &func_indices,
            &struct_offsets,
            &class_offsets,
        );

        let stmts = vec![
            Stmt::Let {
                pattern: Pattern::Binding("x".to_string()),
                ty: Some(Type::Int64),
                value: Expr::Integer(42),
            },
            Stmt::Return(Some(Expr::Var("x".to_string()))),
        ];

        let block = ctx.lower_stmts_to_block(&stmts).unwrap();

        assert_eq!(block.stmts.len(), 2);
    }
}
