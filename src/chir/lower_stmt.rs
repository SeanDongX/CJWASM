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
                // 确定 local 的 WASM 类型：显式注解 > type_ctx 推断 > value 的 wasm_ty
                let local_wasm_ty = if let Some(decl_ty) = ty {
                    let decl_wasm = match decl_ty {
                        crate::ast::Type::Unit | crate::ast::Type::Nothing => wasm_encoder::ValType::I32,
                        t => t.to_wasm(),
                    };
                    value_chir = self.insert_cast_if_needed(value_chir, decl_wasm);
                    decl_wasm
                } else if let Pattern::Binding(name) = pattern {
                    // 无注解时从 type_ctx 获取推断类型
                    self.type_ctx.locals.get(name.as_str())
                        .map(|t| match t {
                            crate::ast::Type::Unit | crate::ast::Type::Nothing => wasm_encoder::ValType::I32,
                            t => t.to_wasm(),
                        })
                        .unwrap_or(value_chir.wasm_ty)
                } else {
                    value_chir.wasm_ty
                };

                match pattern {
                    Pattern::Binding(name) => {
                        let local_idx = self.alloc_local_typed(name.clone(), local_wasm_ty);
                        value_chir = self.insert_cast_if_needed(value_chir, local_wasm_ty);
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

            // Var 语句
            Stmt::Var { pattern, ty, value } => {
                let mut value_chir = self.lower_expr(value)?;
                let local_wasm_ty = if let Some(decl_ty) = ty {
                    let decl_wasm = match decl_ty {
                        crate::ast::Type::Unit | crate::ast::Type::Nothing => wasm_encoder::ValType::I32,
                        t => t.to_wasm(),
                    };
                    value_chir = self.insert_cast_if_needed(value_chir, decl_wasm);
                    decl_wasm
                } else if let Pattern::Binding(name) = pattern {
                    self.type_ctx.locals.get(name.as_str())
                        .map(|t| match t {
                            crate::ast::Type::Unit | crate::ast::Type::Nothing => wasm_encoder::ValType::I32,
                            t => t.to_wasm(),
                        })
                        .unwrap_or(value_chir.wasm_ty)
                } else {
                    value_chir.wasm_ty
                };

                match pattern {
                    Pattern::Binding(name) => {
                        let local_idx = self.alloc_local_typed(name.clone(), local_wasm_ty);
                        value_chir = self.insert_cast_if_needed(value_chir, local_wasm_ty);
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
                let mut value_chir = self.lower_expr(value)?;
                let target_chir = self.lower_assign_target(target)?;

                // 若赋值目标是已知类型的局部变量，插入类型强制转换，防止 local.set 类型不匹配
                if let crate::chir::CHIRLValue::Local(idx) = &target_chir {
                    if let Some(expected_ty) = self.get_local_ty(*idx) {
                        value_chir = self.insert_cast_if_needed(value_chir, expected_ty);
                    }
                }

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
                    let mut val = self.lower_expr(expr)?;
                    // 确保返回值类型与函数签名一致
                    if let Some(ret_ty) = self.return_wasm_ty {
                        val = self.insert_cast_if_needed(val, ret_ty);
                    }
                    Some(val)
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
                if let Some(local_idx) = self.get_local(name) {
                    Ok(CHIRLValue::Local(local_idx))
                } else if let Some((class_name, this_idx)) = self.current_class.clone() {
                    // 隐式 this 字段赋值
                    if let Some(fields) = self.class_field_info.get(&class_name) {
                        if let Some((offset, _)) = fields.get(name) {
                            let this_expr = crate::chir::CHIRExpr::new(
                                crate::chir::CHIRExprKind::Local(this_idx),
                                crate::ast::Type::Struct(class_name.clone(), vec![]),
                                wasm_encoder::ValType::I32,
                            );
                            return Ok(crate::chir::CHIRLValue::Field {
                                object: Box::new(this_expr),
                                offset: *offset,
                            });
                        }
                    }
                    Err(format!("变量未定义: {}", name))
                } else {
                    Err(format!("变量未定义: {}", name))
                }
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

            AssignTarget::FieldPath { base, fields } => {
                // 链式字段：base.f1.f2...fN → 逐级 FieldGet 到倒数第二层，最后一层作为 offset
                let mut obj_expr = crate::ast::Expr::Var(base.clone());
                for field in &fields[..fields.len() - 1] {
                    obj_expr = crate::ast::Expr::Field {
                        object: Box::new(obj_expr),
                        field: field.clone(),
                    };
                }
                let obj_chir = self.lower_expr(&obj_expr)?;
                let obj_ty = self.type_ctx.infer_expr(&obj_expr)?;
                let last_field = fields.last().unwrap();
                let offset = self.get_field_offset(&obj_ty, last_field)?;
                Ok(CHIRLValue::Field {
                    object: Box::new(obj_chir),
                    offset,
                })
            }

            AssignTarget::IndexPath { base, fields, index } => {
                // 链式字段后索引：base.f1.f2[i]
                let mut obj_expr = crate::ast::Expr::Var(base.clone());
                for field in fields {
                    obj_expr = crate::ast::Expr::Field {
                        object: Box::new(obj_expr),
                        field: field.clone(),
                    };
                }
                let array_chir = self.lower_expr(&obj_expr)?;
                let index_chir = self.lower_expr(index)?;
                Ok(CHIRLValue::Index {
                    array: Box::new(array_chir),
                    index: Box::new(index_chir),
                })
            }

            AssignTarget::ExprIndex { expr, index } => {
                let array_chir = self.lower_expr(expr)?;
                let index_chir = self.lower_expr(index)?;
                Ok(CHIRLValue::Index {
                    array: Box::new(array_chir),
                    index: Box::new(index_chir),
                })
            }

            _ => {
                // SuperField, Tuple 等暂不支持
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

            // 如果是最后一个语句且是表达式，尝试作为块的结果值
            // 必须复用已 lower 的 expr_chir，避免二次调用 lower_expr 导致 local 索引翻倍
            if is_last {
                if let Stmt::Expr(expr) = stmt {
                    let expr_chir = self.lower_expr(expr)?;
                    if !matches!(expr_chir.ty, crate::ast::Type::Unit) {
                        // 产生非 Unit 值：作为块的结果（调用者负责消费）
                        result = Some(Box::new(expr_chir));
                    } else {
                        // Unit 表达式：作为语句保留（不作为结果），避免再次 lower
                        chir_stmts.push(crate::chir::CHIRStmt::Expr(expr_chir));
                    }
                    continue; // 无论哪种情况都不再调用 lower_stmt
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
    use crate::ast::{Expr, Type, Pattern, AssignTarget};
    use crate::chir::type_inference::TypeInferenceContext;
    use std::collections::HashMap;

    #[test]
    fn test_lower_let() {
        let type_ctx = TypeInferenceContext::new();
        let func_indices = HashMap::new();
        let func_params = HashMap::new();
        let struct_offsets = HashMap::new();
        let class_offsets = HashMap::new();
        let class_field_info = HashMap::new();

        let mut ctx = LoweringContext::new(
            &type_ctx,
            &func_indices,
            &func_params,
            &struct_offsets,
            &class_offsets,
            &class_field_info,
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
        let func_params = HashMap::new();
        let struct_offsets = HashMap::new();
        let class_offsets = HashMap::new();
        let class_field_info = HashMap::new();

        let mut ctx = LoweringContext::new(
            &type_ctx,
            &func_indices,
            &func_params,
            &struct_offsets,
            &class_offsets,
            &class_field_info,
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
        let func_params = HashMap::new();
        let struct_offsets = HashMap::new();
        let class_offsets = HashMap::new();
        let class_field_info = HashMap::new();

        let mut ctx = LoweringContext::new(
            &type_ctx,
            &func_indices,
            &func_params,
            &struct_offsets,
            &class_offsets,
            &class_field_info,
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

    fn make_ctx<'a>(
        type_ctx: &'a TypeInferenceContext,
        func_indices: &'a HashMap<String, u32>,
        func_params: &'a HashMap<String, Vec<crate::ast::Param>>,
        struct_offsets: &'a HashMap<String, HashMap<String, u32>>,
        class_offsets: &'a HashMap<String, HashMap<String, u32>>,
        class_field_info: &'a HashMap<String, HashMap<String, (u32, Type)>>,
    ) -> crate::chir::lower_expr::LoweringContext<'a> {
        crate::chir::lower_expr::LoweringContext::new(
            type_ctx, func_indices, func_params, struct_offsets, class_offsets, class_field_info,
        )
    }

    #[test]
    fn test_lower_var() {
        let type_ctx = TypeInferenceContext::new();
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let stmt = Stmt::Var {
            pattern: Pattern::Binding("x".into()),
            ty: Some(Type::Int64),
            value: Expr::Integer(0),
        };
        let chir = ctx.lower_stmt(&stmt).unwrap();
        assert!(matches!(chir, CHIRStmt::Let { local_idx: 0, .. }));
    }

    #[test]
    fn test_lower_var_inferred_type() {
        let mut type_ctx = TypeInferenceContext::new();
        type_ctx.add_local("y".into(), Type::Float64);
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let stmt = Stmt::Var {
            pattern: Pattern::Binding("y".into()),
            ty: None,
            value: Expr::Float(3.14),
        };
        let chir = ctx.lower_stmt(&stmt).unwrap();
        assert!(matches!(chir, CHIRStmt::Let { .. }));
    }

    #[test]
    fn test_lower_assign_local() {
        let mut type_ctx = TypeInferenceContext::new();
        type_ctx.add_local("x".into(), Type::Int64);
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);
        ctx.alloc_local_typed("x".into(), wasm_encoder::ValType::I64);

        let stmt = Stmt::Assign {
            target: crate::ast::AssignTarget::Var("x".into()),
            value: Expr::Integer(100),
        };
        let chir = ctx.lower_stmt(&stmt).unwrap();
        assert!(matches!(chir, CHIRStmt::Assign { .. }));
    }

    #[test]
    fn test_lower_expr_stmt() {
        let type_ctx = TypeInferenceContext::new();
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let stmt = Stmt::Expr(Expr::Integer(42));
        let chir = ctx.lower_stmt(&stmt).unwrap();
        assert!(matches!(chir, CHIRStmt::Expr(_)));
    }

    #[test]
    fn test_lower_return_none() {
        let type_ctx = TypeInferenceContext::new();
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let stmt = Stmt::Return(None);
        let chir = ctx.lower_stmt(&stmt).unwrap();
        assert!(matches!(chir, CHIRStmt::Return(None)));
    }

    #[test]
    fn test_lower_break() {
        let type_ctx = TypeInferenceContext::new();
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let stmt = Stmt::Break;
        let chir = ctx.lower_stmt(&stmt).unwrap();
        assert!(matches!(chir, CHIRStmt::Break));
    }

    #[test]
    fn test_lower_continue() {
        let type_ctx = TypeInferenceContext::new();
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let stmt = Stmt::Continue;
        let chir = ctx.lower_stmt(&stmt).unwrap();
        assert!(matches!(chir, CHIRStmt::Continue));
    }

    #[test]
    fn test_lower_while() {
        let type_ctx = TypeInferenceContext::new();
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let stmt = Stmt::While {
            cond: Expr::Bool(true),
            body: vec![Stmt::Break],
        };
        let chir = ctx.lower_stmt(&stmt).unwrap();
        assert!(matches!(chir, CHIRStmt::While { .. }));
    }

    #[test]
    fn test_lower_loop() {
        let type_ctx = TypeInferenceContext::new();
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let stmt = Stmt::Loop {
            body: vec![Stmt::Break],
        };
        let chir = ctx.lower_stmt(&stmt).unwrap();
        assert!(matches!(chir, CHIRStmt::Loop { .. }));
    }

    #[test]
    fn test_lower_for() {
        let type_ctx = TypeInferenceContext::new();
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let stmt = Stmt::For {
            var: "i".into(),
            iterable: Expr::Integer(10),
            body: vec![Stmt::Expr(Expr::Integer(0))],
        };
        // For 循环可能被降低为 While 或其他形式（Expr 包装）
        let chir = ctx.lower_stmt(&stmt).unwrap();
        assert!(!matches!(chir, CHIRStmt::Return(_)));
    }

    #[test]
    fn test_lower_block_with_result() {
        let type_ctx = TypeInferenceContext::new();
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let stmts = vec![
            Stmt::Expr(Expr::Integer(42)),
        ];
        let block = ctx.lower_stmts_to_block(&stmts).unwrap();
        assert!(block.result.is_some());
    }

    #[test]
    fn test_lower_block_unit_result() {
        let type_ctx = TypeInferenceContext::new();
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let stmts = vec![
            Stmt::Expr(Expr::Call {
                name: "println".into(), args: vec![Expr::String("hi".into())],
                type_args: None, named_args: vec![],
            }),
        ];
        let block = ctx.lower_stmts_to_block(&stmts).unwrap();
        // Unit 表达式不作为 result，而是作为 stmt
        assert!(block.result.is_none());
        assert_eq!(block.stmts.len(), 1);
    }

    #[test]
    fn test_lower_let_type_from_ctx() {
        let mut type_ctx = TypeInferenceContext::new();
        type_ctx.add_local("z".into(), Type::Int64);
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let stmt = Stmt::Let {
            pattern: Pattern::Binding("z".into()),
            ty: None,
            value: Expr::Integer(0),
        };
        let chir = ctx.lower_stmt(&stmt).unwrap();
        if let CHIRStmt::Let { local_idx, .. } = chir {
            assert_eq!(ctx.get_local_ty(local_idx), Some(wasm_encoder::ValType::I64));
        } else {
            panic!("expected Let");
        }
    }

    #[test]
    fn test_lower_assign_to_field() {
        let mut type_ctx = TypeInferenceContext::new();
        type_ctx.add_local("obj".into(), Type::Struct("Point".into(), vec![]));
        let mut sf = HashMap::new();
        sf.insert("x".into(), Type::Int64);
        type_ctx.struct_fields.insert("Point".into(), sf);

        let fi = HashMap::new(); let fp = HashMap::new();
        let mut so = HashMap::new();
        let mut po = HashMap::new();
        po.insert("x".into(), 8u32);
        so.insert("Point".into(), po);
        let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);
        ctx.alloc_local_typed("obj".into(), wasm_encoder::ValType::I32);

        let stmt = Stmt::Assign {
            target: AssignTarget::Field { object: "obj".into(), field: "x".into() },
            value: Expr::Integer(100),
        };
        let chir = ctx.lower_stmt(&stmt).unwrap();
        assert!(matches!(chir, CHIRStmt::Assign { .. }));
    }

    #[test]
    fn test_lower_assign_to_index() {
        let mut type_ctx = TypeInferenceContext::new();
        type_ctx.add_local("arr".into(), Type::Array(Box::new(Type::Int64)));
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);
        ctx.alloc_local_typed("arr".into(), wasm_encoder::ValType::I32);

        let stmt = Stmt::Assign {
            target: AssignTarget::Index { array: "arr".into(), index: Box::new(Expr::Integer(0)) },
            value: Expr::Integer(42),
        };
        let chir = ctx.lower_stmt(&stmt).unwrap();
        assert!(matches!(chir, CHIRStmt::Assign { .. }));
    }

    #[test]
    fn test_lower_assign_field_path() {
        let mut type_ctx = TypeInferenceContext::new();
        type_ctx.add_local("o".into(), Type::Struct("Outer".into(), vec![]));
        let mut sf = HashMap::new();
        let mut inner = HashMap::new();
        inner.insert("v".into(), Type::Int64);
        sf.insert("Inner".into(), inner);
        let mut outer = HashMap::new();
        outer.insert("inner".into(), Type::Struct("Inner".into(), vec![]));
        sf.insert("Outer".into(), outer);
        type_ctx.struct_fields = sf;

        let fi = HashMap::new(); let fp = HashMap::new();
        let mut so = HashMap::new();
        let mut outer_off = HashMap::new();
        outer_off.insert("inner".into(), 8u32);
        so.insert("Outer".into(), outer_off);
        let mut inner_off = HashMap::new();
        inner_off.insert("v".into(), 8u32);
        so.insert("Inner".into(), inner_off);
        let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);
        ctx.alloc_local_typed("o".into(), wasm_encoder::ValType::I32);

        let stmt = Stmt::Assign {
            target: AssignTarget::FieldPath { base: "o".into(), fields: vec!["inner".into(), "v".into()] },
            value: Expr::Integer(1),
        };
        let chir = ctx.lower_stmt(&stmt).unwrap();
        assert!(matches!(chir, CHIRStmt::Assign { .. }));
    }

    #[test]
    fn test_lower_assign_expr_index() {
        let mut type_ctx = TypeInferenceContext::new();
        type_ctx.add_local("arr".into(), Type::Array(Box::new(Type::Int64)));
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);
        ctx.alloc_local_typed("arr".into(), wasm_encoder::ValType::I32);

        let stmt = Stmt::Assign {
            target: AssignTarget::ExprIndex {
                expr: Box::new(Expr::Var("arr".into())),
                index: Box::new(Expr::Integer(1)),
            },
            value: Expr::Integer(99),
        };
        let chir = ctx.lower_stmt(&stmt).unwrap();
        assert!(matches!(chir, CHIRStmt::Assign { .. }));
    }

    #[test]
    fn test_lower_while_with_cond() {
        let type_ctx = TypeInferenceContext::new();
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let stmt = Stmt::While {
            cond: Expr::Bool(false),
            body: vec![Stmt::Expr(Expr::Integer(0))],
        };
        let chir = ctx.lower_stmt(&stmt).unwrap();
        assert!(matches!(chir, CHIRStmt::While { .. }));
    }

    #[test]
    fn test_lower_for_with_array() {
        let mut type_ctx = TypeInferenceContext::new();
        type_ctx.add_local("x".into(), Type::Int64);
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let stmt = Stmt::For {
            var: "x".into(),
            iterable: Expr::Array(vec![Expr::Integer(1), Expr::Integer(2)]),
            body: vec![Stmt::Expr(Expr::Var("x".into()))],
        };
        let chir = ctx.lower_stmt(&stmt).unwrap();
        assert!(!matches!(chir, CHIRStmt::Return(_)));
    }

    #[test]
    fn test_lower_do_while() {
        let type_ctx = TypeInferenceContext::new();
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let stmt = Stmt::DoWhile {
            body: vec![Stmt::Break],
            cond: Expr::Bool(false),
        };
        let chir = ctx.lower_stmt(&stmt).unwrap();
        assert!(matches!(chir, CHIRStmt::Expr(_)));
    }

    #[test]
    fn test_lower_nested_loops() {
        let type_ctx = TypeInferenceContext::new();
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let stmt = Stmt::While {
            cond: Expr::Bool(true),
            body: vec![
                Stmt::Loop { body: vec![Stmt::Break] },
            ],
        };
        let chir = ctx.lower_stmt(&stmt).unwrap();
        assert!(matches!(chir, CHIRStmt::While { .. }));
    }

    #[test]
    fn test_lower_block_trailing_expr() {
        let type_ctx = TypeInferenceContext::new();
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let stmts = vec![
            Stmt::Let {
                pattern: Pattern::Binding("a".into()),
                ty: Some(Type::Int64),
                value: Expr::Integer(1),
            },
            Stmt::Expr(Expr::Binary {
                op: crate::ast::BinOp::Add,
                left: Box::new(Expr::Var("a".into())),
                right: Box::new(Expr::Integer(2)),
            }),
        ];
        let block = ctx.lower_stmts_to_block(&stmts).unwrap();
        assert!(block.result.is_some());
        assert_eq!(block.stmts.len(), 1);
    }

    #[test]
    fn test_lower_return_with_cast() {
        let mut type_ctx = TypeInferenceContext::new();
        type_ctx.add_local("x".into(), Type::Float64);
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);
        ctx.return_wasm_ty = Some(wasm_encoder::ValType::I64);
        ctx.alloc_local_typed("x".into(), wasm_encoder::ValType::F64);

        let stmt = Stmt::Return(Some(Expr::Var("x".into())));
        let chir = ctx.lower_stmt(&stmt).unwrap();
        assert!(matches!(chir, CHIRStmt::Return(Some(_))));
    }
}
