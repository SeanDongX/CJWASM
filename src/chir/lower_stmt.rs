//! AST → CHIR 语句转换

use super::lower_expr::LoweringContext;
use crate::ast::{AssignTarget, Pattern, Stmt};
use crate::chir::{CHIRBlock, CHIRLValue, CHIRStmt};

impl<'a> LoweringContext<'a> {
    /// 降低语句
    pub fn lower_stmt(&mut self, stmt: &Stmt) -> Result<CHIRStmt, String> {
        match stmt {
            // Let 语句
            Stmt::Let { pattern, ty, value } => {
                let mut value_chir = self.lower_expr(value)?;
                let local_wasm_ty = if let Some(decl_ty) = ty {
                    let decl_wasm = match decl_ty {
                        crate::ast::Type::Unit | crate::ast::Type::Nothing => {
                            wasm_encoder::ValType::I32
                        }
                        t => t.to_wasm(),
                    };
                    value_chir = self.insert_cast_if_needed(value_chir, decl_wasm);
                    decl_wasm
                } else if let Pattern::Binding(name) = pattern {
                    self.type_ctx
                        .locals
                        .get(name.as_str())
                        .map(|t| match t {
                            crate::ast::Type::Unit | crate::ast::Type::Nothing => {
                                wasm_encoder::ValType::I32
                            }
                            t => t.to_wasm(),
                        })
                        .unwrap_or(value_chir.wasm_ty)
                } else {
                    value_chir.wasm_ty
                };

                match pattern {
                    Pattern::Binding(name) => {
                        let ast_ty = if let Some(decl_ty) = ty {
                            decl_ty.clone()
                        } else {
                            value_chir.ty.clone()
                        };
                        self.local_ast_types.insert(name.clone(), ast_ty);
                        let local_idx = self.alloc_local_typed(name.clone(), local_wasm_ty);
                        value_chir = self.insert_cast_if_needed(value_chir, local_wasm_ty);
                        Ok(CHIRStmt::Let {
                            local_idx,
                            value: value_chir,
                        })
                    }
                    Pattern::Struct {
                        name: struct_name,
                        fields,
                    } => {
                        let multi =
                            self.lower_struct_deconstruction(struct_name, fields, value_chir)?;
                        // Return the first statement; remaining will be handled by lower_stmts_to_block
                        Ok(multi.into_iter().next().unwrap_or(CHIRStmt::Expr(
                            crate::chir::CHIRExpr::new(
                                crate::chir::CHIRExprKind::Nop,
                                crate::ast::Type::Unit,
                                wasm_encoder::ValType::I32,
                            ),
                        )))
                    }
                    _ => Ok(CHIRStmt::Expr(value_chir)),
                }
            }

            // Var 语句
            Stmt::Var { pattern, ty, value } => {
                let mut value_chir = self.lower_expr(value)?;
                let local_wasm_ty = if let Some(decl_ty) = ty {
                    let decl_wasm = match decl_ty {
                        crate::ast::Type::Unit | crate::ast::Type::Nothing => {
                            wasm_encoder::ValType::I32
                        }
                        t => t.to_wasm(),
                    };
                    value_chir = self.insert_cast_if_needed(value_chir, decl_wasm);
                    decl_wasm
                } else if let Pattern::Binding(name) = pattern {
                    self.type_ctx
                        .locals
                        .get(name.as_str())
                        .map(|t| match t {
                            crate::ast::Type::Unit | crate::ast::Type::Nothing => {
                                wasm_encoder::ValType::I32
                            }
                            t => t.to_wasm(),
                        })
                        .unwrap_or(value_chir.wasm_ty)
                } else {
                    value_chir.wasm_ty
                };

                match pattern {
                    Pattern::Binding(name) => {
                        let ast_ty = if let Some(decl_ty) = ty {
                            decl_ty.clone()
                        } else {
                            value_chir.ty.clone()
                        };
                        self.local_ast_types.insert(name.clone(), ast_ty);
                        let local_idx = self.alloc_local_typed(name.clone(), local_wasm_ty);
                        value_chir = self.insert_cast_if_needed(value_chir, local_wasm_ty);
                        Ok(CHIRStmt::Let {
                            local_idx,
                            value: value_chir,
                        })
                    }
                    _ => Ok(CHIRStmt::Expr(value_chir)),
                }
            }

            // 赋值语句
            Stmt::Assign { target, value } => {
                // P1-1: 赋值语义检查（不可变变量 + 类型不匹配）
                if let AssignTarget::Var(name) = target {
                    if self.get_local(name).is_some() {
                        if !self
                            .type_ctx
                            .local_mutability
                            .get(name)
                            .copied()
                            .unwrap_or(true)
                        {
                            return Err(
                                "semantic error: cannot assign to immutable value".to_string()
                            );
                        }
                        if let Some(target_ty) = self
                            .local_ast_types
                            .get(name)
                            .cloned()
                            .or_else(|| self.type_ctx.locals.get(name).cloned())
                        {
                            let value_ty = self.type_ctx.infer_expr(value)?;
                            if !self.type_ctx.is_assignable_type(&target_ty, &value_ty) {
                                return Err("semantic error: mismatched types".to_string());
                            }
                        }
                    }
                }

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

                Ok(CHIRStmt::Loop { body: body_block })
            }

            // For 循环：降低为 While
            Stmt::For {
                var,
                iterable,
                body,
            } => {
                fn replace_continue_in_block(
                    block: &mut crate::chir::CHIRBlock,
                    make_inc: &dyn Fn() -> CHIRStmt,
                ) {
                    let mut new_stmts = Vec::new();
                    for stmt in block.stmts.drain(..) {
                        match stmt {
                            CHIRStmt::Continue => {
                                new_stmts.push(make_inc());
                                new_stmts.push(CHIRStmt::Continue);
                            }
                            CHIRStmt::While { cond, body: inner } => {
                                new_stmts.push(CHIRStmt::While { cond, body: inner });
                            }
                            CHIRStmt::Loop { body: inner } => {
                                new_stmts.push(CHIRStmt::Loop { body: inner });
                            }
                            CHIRStmt::Expr(expr) => {
                                let expr = replace_continue_in_expr(expr, make_inc);
                                new_stmts.push(CHIRStmt::Expr(expr));
                            }
                            other => new_stmts.push(other),
                        }
                    }
                    block.stmts = new_stmts;
                }
                fn replace_continue_in_expr(
                    expr: crate::chir::CHIRExpr,
                    make_inc: &dyn Fn() -> CHIRStmt,
                ) -> crate::chir::CHIRExpr {
                    match expr.kind {
                        crate::chir::CHIRExprKind::If {
                            cond,
                            mut then_block,
                            else_block,
                        } => {
                            replace_continue_in_block(&mut then_block, make_inc);
                            let else_block = else_block.map(|mut b| {
                                replace_continue_in_block(&mut b, make_inc);
                                b
                            });
                            crate::chir::CHIRExpr {
                                kind: crate::chir::CHIRExprKind::If {
                                    cond,
                                    then_block,
                                    else_block,
                                },
                                ..expr
                            }
                        }
                        crate::chir::CHIRExprKind::Block(mut b) => {
                            replace_continue_in_block(&mut b, make_inc);
                            crate::chir::CHIRExpr {
                                kind: crate::chir::CHIRExprKind::Block(b),
                                ..expr
                            }
                        }
                        _ => expr,
                    }
                }

                if let crate::ast::Expr::Range {
                    start,
                    end,
                    inclusive,
                    step,
                } = iterable
                {
                    let start_chir = self.lower_expr(start)?;
                    let end_chir = self.lower_expr(end)?;
                    let step_val = if let Some(s) = step {
                        self.lower_expr(s)?
                    } else {
                        crate::chir::CHIRExpr::new(
                            crate::chir::CHIRExprKind::Integer(1),
                            crate::ast::Type::Int64,
                            wasm_encoder::ValType::I64,
                        )
                    };
                    let loop_var_idx = self.alloc_local_typed(var.clone(), start_chir.wasm_ty);
                    // 保存 end 值到临时 local
                    let end_local_idx =
                        self.alloc_local_typed(format!("__for_end_{}", var), end_chir.wasm_ty);
                    // 构建条件：var < end (或 var <= end)
                    let cmp_op = if *inclusive {
                        crate::chir::CHIRExprKind::Binary {
                            op: crate::ast::BinOp::LtEq,
                            left: Box::new(crate::chir::CHIRExpr::new(
                                crate::chir::CHIRExprKind::Local(loop_var_idx),
                                start_chir.ty.clone(),
                                start_chir.wasm_ty,
                            )),
                            right: Box::new(crate::chir::CHIRExpr::new(
                                crate::chir::CHIRExprKind::Local(end_local_idx),
                                end_chir.ty.clone(),
                                end_chir.wasm_ty,
                            )),
                        }
                    } else {
                        crate::chir::CHIRExprKind::Binary {
                            op: crate::ast::BinOp::Lt,
                            left: Box::new(crate::chir::CHIRExpr::new(
                                crate::chir::CHIRExprKind::Local(loop_var_idx),
                                start_chir.ty.clone(),
                                start_chir.wasm_ty,
                            )),
                            right: Box::new(crate::chir::CHIRExpr::new(
                                crate::chir::CHIRExprKind::Local(end_local_idx),
                                end_chir.ty.clone(),
                                end_chir.wasm_ty,
                            )),
                        }
                    };
                    let cond_expr = crate::chir::CHIRExpr::new(
                        cmp_op,
                        crate::ast::Type::Bool,
                        wasm_encoder::ValType::I32,
                    );
                    // 降低循环体
                    let mut body_block = self.lower_stmts_to_block(body)?;
                    // 构建增量表达式（用于末尾和 continue 替换）
                    let make_increment = || -> CHIRStmt {
                        CHIRStmt::Assign {
                            target: CHIRLValue::Local(loop_var_idx),
                            value: crate::chir::CHIRExpr::new(
                                crate::chir::CHIRExprKind::Binary {
                                    op: crate::ast::BinOp::Add,
                                    left: Box::new(crate::chir::CHIRExpr::new(
                                        crate::chir::CHIRExprKind::Local(loop_var_idx),
                                        start_chir.ty.clone(),
                                        start_chir.wasm_ty,
                                    )),
                                    right: Box::new(step_val.clone()),
                                },
                                start_chir.ty.clone(),
                                start_chir.wasm_ty,
                            ),
                        }
                    };
                    replace_continue_in_block(&mut body_block, &make_increment);
                    // 在循环体末尾追加 var = var + step
                    body_block.stmts.push(make_increment());
                    // 构造完整语句序列：let i = start; let __end = end; while (cond) { body }
                    let init_stmt = CHIRStmt::Let {
                        local_idx: loop_var_idx,
                        value: start_chir,
                    };
                    let end_init_stmt = CHIRStmt::Let {
                        local_idx: end_local_idx,
                        value: end_chir,
                    };
                    let while_stmt = CHIRStmt::While {
                        cond: cond_expr,
                        body: body_block,
                    };
                    // 用一个包裹块把三条语句打包
                    let wrapper_block = crate::chir::CHIRBlock {
                        stmts: vec![init_stmt, end_init_stmt, while_stmt],
                        result: None,
                    };
                    Ok(CHIRStmt::Expr(crate::chir::CHIRExpr::new(
                        crate::chir::CHIRExprKind::Block(wrapper_block),
                        crate::ast::Type::Unit,
                        wasm_encoder::ValType::I32,
                    )))
                } else {
                    // for (elem in arr) → 数组迭代
                    // let __arr = arr; let __idx = 0; let __len = arr.length;
                    // while (__idx < __len) { let elem = arr[__idx]; body; __idx++ }
                    let arr_chir = self.lower_expr(iterable)?;
                    let arr_local = self.alloc_local_typed(
                        format!("__for_arr_{}", var),
                        wasm_encoder::ValType::I32,
                    );
                    let idx_local = self.alloc_local_typed(
                        format!("__for_idx_{}", var),
                        wasm_encoder::ValType::I64,
                    );
                    let len_local = self.alloc_local_typed(
                        format!("__for_len_{}", var),
                        wasm_encoder::ValType::I64,
                    );
                    let elem_local =
                        self.alloc_local_typed(var.clone(), wasm_encoder::ValType::I64);

                    let arr_init = CHIRStmt::Let {
                        local_idx: arr_local,
                        value: self.insert_cast_if_needed(arr_chir, wasm_encoder::ValType::I32),
                    };
                    let idx_init = CHIRStmt::Let {
                        local_idx: idx_local,
                        value: crate::chir::CHIRExpr::int_const(0, crate::ast::Type::Int64),
                    };
                    // len = i64.load(arr_ptr) → actually i32.load then extend
                    let len_expr = crate::chir::CHIRExpr::new(
                        crate::chir::CHIRExprKind::Cast {
                            expr: Box::new(crate::chir::CHIRExpr::new(
                                crate::chir::CHIRExprKind::FieldGet {
                                    object: Box::new(crate::chir::CHIRExpr::new(
                                        crate::chir::CHIRExprKind::Local(arr_local),
                                        crate::ast::Type::Int32,
                                        wasm_encoder::ValType::I32,
                                    )),
                                    field_offset: 0,
                                    field_ty: crate::ast::Type::Int32,
                                },
                                crate::ast::Type::Int32,
                                wasm_encoder::ValType::I32,
                            )),
                            from_ty: wasm_encoder::ValType::I32,
                            to_ty: wasm_encoder::ValType::I64,
                        },
                        crate::ast::Type::Int64,
                        wasm_encoder::ValType::I64,
                    );
                    let len_init = CHIRStmt::Let {
                        local_idx: len_local,
                        value: len_expr,
                    };
                    // cond: __idx < __len
                    let cond_expr = crate::chir::CHIRExpr::new(
                        crate::chir::CHIRExprKind::Binary {
                            op: crate::ast::BinOp::Lt,
                            left: Box::new(crate::chir::CHIRExpr::new(
                                crate::chir::CHIRExprKind::Local(idx_local),
                                crate::ast::Type::Int64,
                                wasm_encoder::ValType::I64,
                            )),
                            right: Box::new(crate::chir::CHIRExpr::new(
                                crate::chir::CHIRExprKind::Local(len_local),
                                crate::ast::Type::Int64,
                                wasm_encoder::ValType::I64,
                            )),
                        },
                        crate::ast::Type::Bool,
                        wasm_encoder::ValType::I32,
                    );
                    // elem = arr[__idx]
                    let elem_expr = crate::chir::CHIRExpr::new(
                        crate::chir::CHIRExprKind::ArrayGet {
                            array: Box::new(crate::chir::CHIRExpr::new(
                                crate::chir::CHIRExprKind::Local(arr_local),
                                crate::ast::Type::Int32,
                                wasm_encoder::ValType::I32,
                            )),
                            index: Box::new(crate::chir::CHIRExpr::new(
                                crate::chir::CHIRExprKind::Local(idx_local),
                                crate::ast::Type::Int64,
                                wasm_encoder::ValType::I64,
                            )),
                        },
                        crate::ast::Type::Int64,
                        wasm_encoder::ValType::I64,
                    );
                    let elem_assign = CHIRStmt::Let {
                        local_idx: elem_local,
                        value: elem_expr,
                    };
                    // lower body
                    let mut body_block = self.lower_stmts_to_block(body)?;
                    // prepend elem assignment
                    body_block.stmts.insert(0, elem_assign);
                    // increment: __idx = __idx + 1
                    let make_increment = || -> CHIRStmt {
                        CHIRStmt::Assign {
                            target: CHIRLValue::Local(idx_local),
                            value: crate::chir::CHIRExpr::new(
                                crate::chir::CHIRExprKind::Binary {
                                    op: crate::ast::BinOp::Add,
                                    left: Box::new(crate::chir::CHIRExpr::new(
                                        crate::chir::CHIRExprKind::Local(idx_local),
                                        crate::ast::Type::Int64,
                                        wasm_encoder::ValType::I64,
                                    )),
                                    right: Box::new(crate::chir::CHIRExpr::int_const(
                                        1,
                                        crate::ast::Type::Int64,
                                    )),
                                },
                                crate::ast::Type::Int64,
                                wasm_encoder::ValType::I64,
                            ),
                        }
                    };
                    replace_continue_in_block(&mut body_block, &make_increment);
                    body_block.stmts.push(make_increment());
                    let while_stmt = CHIRStmt::While {
                        cond: cond_expr,
                        body: body_block,
                    };
                    let wrapper_block = crate::chir::CHIRBlock {
                        stmts: vec![arr_init, idx_init, len_init, while_stmt],
                        result: None,
                    };
                    Ok(CHIRStmt::Expr(crate::chir::CHIRExpr::new(
                        crate::chir::CHIRExprKind::Block(wrapper_block),
                        crate::ast::Type::Unit,
                        wasm_encoder::ValType::I32,
                    )))
                }
            }

            // DoWhile 循环：降为 loop { body; if !cond break }
            Stmt::DoWhile { body, cond } => {
                let cond_chir = self.lower_expr(cond)?;
                let mut body_block = self.lower_stmts_to_block(body)?;
                // if (!cond) break
                let not_cond = crate::chir::CHIRExpr::new(
                    crate::chir::CHIRExprKind::Unary {
                        op: crate::ast::UnaryOp::Not,
                        expr: Box::new(cond_chir),
                    },
                    crate::ast::Type::Bool,
                    wasm_encoder::ValType::I32,
                );
                let break_block = crate::chir::CHIRBlock {
                    stmts: vec![CHIRStmt::Break],
                    result: None,
                };
                let empty_block = crate::chir::CHIRBlock {
                    stmts: vec![],
                    result: None,
                };
                let if_break = crate::chir::CHIRExpr::new(
                    crate::chir::CHIRExprKind::If {
                        cond: Box::new(not_cond),
                        then_block: break_block,
                        else_block: Some(empty_block),
                    },
                    crate::ast::Type::Unit,
                    wasm_encoder::ValType::I32,
                );
                body_block.stmts.push(CHIRStmt::Expr(if_break));
                Ok(CHIRStmt::Loop { body: body_block })
            }

            // Assert 语句
            Stmt::Assert { left, right, .. } => {
                let left_chir = self.lower_expr(left)?;
                let right_chir = self.lower_expr(right)?;
                // 如果 left != right → unreachable (trap)
                let ne_expr = crate::chir::CHIRExpr::new(
                    crate::chir::CHIRExprKind::Binary {
                        op: crate::ast::BinOp::NotEq,
                        left: Box::new(left_chir),
                        right: Box::new(right_chir),
                    },
                    crate::ast::Type::Bool,
                    wasm_encoder::ValType::I32,
                );
                let trap_block = crate::chir::CHIRBlock {
                    stmts: vec![CHIRStmt::Expr(crate::chir::CHIRExpr::new(
                        crate::chir::CHIRExprKind::Unreachable,
                        crate::ast::Type::Nothing,
                        wasm_encoder::ValType::I32,
                    ))],
                    result: None,
                };
                let empty_block = crate::chir::CHIRBlock {
                    stmts: vec![],
                    result: None,
                };
                let if_trap = crate::chir::CHIRExpr::new(
                    crate::chir::CHIRExprKind::If {
                        cond: Box::new(ne_expr),
                        then_block: trap_block,
                        else_block: Some(empty_block),
                    },
                    crate::ast::Type::Unit,
                    wasm_encoder::ValType::I32,
                );
                Ok(CHIRStmt::Expr(if_trap))
            }

            // Const 语句：等价于 Let
            Stmt::Const { name, ty, value } => {
                let mut value_chir = self.lower_expr(value)?;
                let local_wasm_ty = if let Some(decl_ty) = ty {
                    let decl_wasm = match decl_ty {
                        crate::ast::Type::Unit | crate::ast::Type::Nothing => {
                            wasm_encoder::ValType::I32
                        }
                        t => t.to_wasm(),
                    };
                    value_chir = self.insert_cast_if_needed(value_chir, decl_wasm);
                    decl_wasm
                } else {
                    value_chir.wasm_ty
                };
                let local_idx = self.alloc_local_typed(name.clone(), local_wasm_ty);
                value_chir = self.insert_cast_if_needed(value_chir, local_wasm_ty);
                Ok(CHIRStmt::Let {
                    local_idx,
                    value: value_chir,
                })
            }

            // while-let: while (let Some(n) <- current) { body }
            // → loop { if current.tag != 1 { break }; let n = current.value; body }
            Stmt::WhileLet {
                pattern,
                expr,
                body,
            } => {
                use crate::ast::Pattern;
                let expr_chir = self.lower_expr(expr)?;
                let ptr_local =
                    self.alloc_local_typed("__wl_ptr".into(), wasm_encoder::ValType::I32);

                let mut loop_stmts: Vec<crate::chir::CHIRStmt> = Vec::new();

                // Evaluate expr and save to ptr_local each iteration
                loop_stmts.push(crate::chir::CHIRStmt::Assign {
                    target: crate::chir::CHIRLValue::Local(ptr_local),
                    value: expr_chir.clone(),
                });

                // Load tag at offset 0
                let tag_load = crate::chir::CHIRExpr::new(
                    crate::chir::CHIRExprKind::Load {
                        ptr: Box::new(crate::chir::CHIRExpr::new(
                            crate::chir::CHIRExprKind::Local(ptr_local),
                            crate::ast::Type::Int32,
                            wasm_encoder::ValType::I32,
                        )),
                        offset: 0,
                        align: 2,
                    },
                    crate::ast::Type::Int32,
                    wasm_encoder::ValType::I32,
                );
                // if tag != 1 { break }
                let cond_ne = crate::chir::CHIRExpr::new(
                    crate::chir::CHIRExprKind::Binary {
                        op: crate::ast::BinOp::NotEq,
                        left: Box::new(tag_load),
                        right: Box::new(crate::chir::CHIRExpr::new(
                            crate::chir::CHIRExprKind::Integer(1),
                            crate::ast::Type::Int32,
                            wasm_encoder::ValType::I32,
                        )),
                    },
                    crate::ast::Type::Bool,
                    wasm_encoder::ValType::I32,
                );
                let break_block = crate::chir::CHIRBlock {
                    stmts: vec![crate::chir::CHIRStmt::Break],
                    result: None,
                };
                loop_stmts.push(crate::chir::CHIRStmt::Expr(crate::chir::CHIRExpr::new(
                    crate::chir::CHIRExprKind::If {
                        cond: Box::new(cond_ne),
                        then_block: break_block,
                        else_block: None,
                    },
                    crate::ast::Type::Unit,
                    wasm_encoder::ValType::I32,
                )));

                // Bind payload variable from pattern
                if let Pattern::Variant {
                    payload: Some(ref payload_pat),
                    ..
                } = pattern
                {
                    if let Pattern::Binding(ref name) = **payload_pat {
                        let bind_local =
                            self.alloc_local_typed(name.clone(), wasm_encoder::ValType::I64);
                        let val_load = crate::chir::CHIRExpr::new(
                            crate::chir::CHIRExprKind::Load {
                                ptr: Box::new(crate::chir::CHIRExpr::new(
                                    crate::chir::CHIRExprKind::Local(ptr_local),
                                    crate::ast::Type::Int32,
                                    wasm_encoder::ValType::I32,
                                )),
                                offset: 4,
                                align: 3,
                            },
                            crate::ast::Type::Int64,
                            wasm_encoder::ValType::I64,
                        );
                        loop_stmts.push(crate::chir::CHIRStmt::Let {
                            local_idx: bind_local,
                            value: val_load,
                        });
                        self.local_ast_types
                            .insert(name.clone(), crate::ast::Type::Int64);
                    }
                } else if let Pattern::Binding(ref name) = pattern {
                    let bind_local =
                        self.alloc_local_typed(name.clone(), wasm_encoder::ValType::I64);
                    let val_load = crate::chir::CHIRExpr::new(
                        crate::chir::CHIRExprKind::Load {
                            ptr: Box::new(crate::chir::CHIRExpr::new(
                                crate::chir::CHIRExprKind::Local(ptr_local),
                                crate::ast::Type::Int32,
                                wasm_encoder::ValType::I32,
                            )),
                            offset: 4,
                            align: 3,
                        },
                        crate::ast::Type::Int64,
                        wasm_encoder::ValType::I64,
                    );
                    loop_stmts.push(crate::chir::CHIRStmt::Let {
                        local_idx: bind_local,
                        value: val_load,
                    });
                    self.local_ast_types
                        .insert(name.clone(), crate::ast::Type::Int64);
                }

                // Lower body statements
                for s in body {
                    if let Ok(chir_s) = self.lower_stmt(s) {
                        loop_stmts.push(chir_s);
                    }
                }

                let loop_body = crate::chir::CHIRBlock {
                    stmts: loop_stmts,
                    result: None,
                };
                Ok(CHIRStmt::Loop { body: loop_body })
            }

            // 其他语句暂时转换为 Nop
            _ => Ok(CHIRStmt::Expr(crate::chir::CHIRExpr::new(
                crate::chir::CHIRExprKind::Nop,
                crate::ast::Type::Unit,
                wasm_encoder::ValType::I32,
            ))),
        }
    }

    /// Struct deconstruction: let Point { x, y } = expr
    /// Returns multiple statements that need to be emitted in sequence
    fn lower_struct_deconstruction(
        &mut self,
        struct_name: &str,
        fields: &[(String, crate::ast::Pattern)],
        value_chir: crate::chir::CHIRExpr,
    ) -> Result<Vec<CHIRStmt>, String> {
        let struct_ty = crate::ast::Type::Struct(struct_name.to_string(), vec![]);
        let ptr_local = self.alloc_local_typed(
            format!("__destruct_{}", struct_name),
            wasm_encoder::ValType::I32,
        );
        let ptr_val = self.insert_cast_if_needed(value_chir, wasm_encoder::ValType::I32);
        let mut stmts = vec![CHIRStmt::Let {
            local_idx: ptr_local,
            value: ptr_val,
        }];
        for (field_name, sub_pat) in fields {
            if let crate::ast::Pattern::Binding(bind_name) = sub_pat {
                let offset = self.get_field_offset(&struct_ty, field_name)?;
                let field_ty = self
                    .type_ctx
                    .infer_field_type(&struct_ty, field_name)
                    .unwrap_or(crate::ast::Type::Int64);
                let field_wasm = match &field_ty {
                    crate::ast::Type::Unit | crate::ast::Type::Nothing => {
                        wasm_encoder::ValType::I32
                    }
                    t => t.to_wasm(),
                };
                self.local_ast_types
                    .insert(bind_name.clone(), field_ty.clone());
                let bind_local = self.alloc_local_typed(bind_name.clone(), field_wasm);
                let load_expr = crate::chir::CHIRExpr::new(
                    crate::chir::CHIRExprKind::FieldGet {
                        object: Box::new(crate::chir::CHIRExpr::new(
                            crate::chir::CHIRExprKind::Local(ptr_local),
                            struct_ty.clone(),
                            wasm_encoder::ValType::I32,
                        )),
                        field_offset: offset,
                        field_ty: field_ty,
                    },
                    crate::ast::Type::Int64,
                    field_wasm,
                );
                stmts.push(CHIRStmt::Let {
                    local_idx: bind_local,
                    value: load_expr,
                });
            }
        }
        Ok(stmts)
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

            AssignTarget::IndexPath {
                base,
                fields,
                index,
            } => {
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

            // Struct deconstruction expands to multiple statements
            if let Stmt::Let {
                pattern: Pattern::Struct { name, fields },
                value,
                ..
            } = stmt
            {
                let value_chir = self.lower_expr(value)?;
                let multi = self.lower_struct_deconstruction(name, fields, value_chir)?;
                chir_stmts.extend(multi);
                continue;
            }
            if let Stmt::Var {
                pattern: Pattern::Struct { name, fields },
                value,
                ..
            } = stmt
            {
                let value_chir = self.lower_expr(value)?;
                let multi = self.lower_struct_deconstruction(name, fields, value_chir)?;
                chir_stmts.extend(multi);
                continue;
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
    use crate::ast::{AssignTarget, Expr, Pattern, Type};
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
            type_ctx,
            func_indices,
            func_params,
            struct_offsets,
            class_offsets,
            class_field_info,
        )
    }

    #[test]
    fn test_lower_var() {
        let type_ctx = TypeInferenceContext::new();
        let fi = HashMap::new();
        let fp = HashMap::new();
        let so = HashMap::new();
        let co = HashMap::new();
        let ci = HashMap::new();
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
        let fi = HashMap::new();
        let fp = HashMap::new();
        let so = HashMap::new();
        let co = HashMap::new();
        let ci = HashMap::new();
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
        let fi = HashMap::new();
        let fp = HashMap::new();
        let so = HashMap::new();
        let co = HashMap::new();
        let ci = HashMap::new();
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
        let fi = HashMap::new();
        let fp = HashMap::new();
        let so = HashMap::new();
        let co = HashMap::new();
        let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let stmt = Stmt::Expr(Expr::Integer(42));
        let chir = ctx.lower_stmt(&stmt).unwrap();
        assert!(matches!(chir, CHIRStmt::Expr(_)));
    }

    #[test]
    fn test_lower_return_none() {
        let type_ctx = TypeInferenceContext::new();
        let fi = HashMap::new();
        let fp = HashMap::new();
        let so = HashMap::new();
        let co = HashMap::new();
        let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let stmt = Stmt::Return(None);
        let chir = ctx.lower_stmt(&stmt).unwrap();
        assert!(matches!(chir, CHIRStmt::Return(None)));
    }

    #[test]
    fn test_lower_break() {
        let type_ctx = TypeInferenceContext::new();
        let fi = HashMap::new();
        let fp = HashMap::new();
        let so = HashMap::new();
        let co = HashMap::new();
        let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let stmt = Stmt::Break;
        let chir = ctx.lower_stmt(&stmt).unwrap();
        assert!(matches!(chir, CHIRStmt::Break));
    }

    #[test]
    fn test_lower_continue() {
        let type_ctx = TypeInferenceContext::new();
        let fi = HashMap::new();
        let fp = HashMap::new();
        let so = HashMap::new();
        let co = HashMap::new();
        let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let stmt = Stmt::Continue;
        let chir = ctx.lower_stmt(&stmt).unwrap();
        assert!(matches!(chir, CHIRStmt::Continue));
    }

    #[test]
    fn test_lower_while() {
        let type_ctx = TypeInferenceContext::new();
        let fi = HashMap::new();
        let fp = HashMap::new();
        let so = HashMap::new();
        let co = HashMap::new();
        let ci = HashMap::new();
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
        let fi = HashMap::new();
        let fp = HashMap::new();
        let so = HashMap::new();
        let co = HashMap::new();
        let ci = HashMap::new();
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
        let fi = HashMap::new();
        let fp = HashMap::new();
        let so = HashMap::new();
        let co = HashMap::new();
        let ci = HashMap::new();
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
        let fi = HashMap::new();
        let fp = HashMap::new();
        let so = HashMap::new();
        let co = HashMap::new();
        let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let stmts = vec![Stmt::Expr(Expr::Integer(42))];
        let block = ctx.lower_stmts_to_block(&stmts).unwrap();
        assert!(block.result.is_some());
    }

    #[test]
    fn test_lower_block_unit_result() {
        let type_ctx = TypeInferenceContext::new();
        let fi = HashMap::new();
        let fp = HashMap::new();
        let so = HashMap::new();
        let co = HashMap::new();
        let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let stmts = vec![Stmt::Expr(Expr::Call {
            name: "println".into(),
            args: vec![Expr::String("hi".into())],
            type_args: None,
            named_args: vec![],
        })];
        let block = ctx.lower_stmts_to_block(&stmts).unwrap();
        // Unit 表达式不作为 result，而是作为 stmt
        assert!(block.result.is_none());
        assert_eq!(block.stmts.len(), 1);
    }

    #[test]
    fn test_lower_let_type_from_ctx() {
        let mut type_ctx = TypeInferenceContext::new();
        type_ctx.add_local("z".into(), Type::Int64);
        let fi = HashMap::new();
        let fp = HashMap::new();
        let so = HashMap::new();
        let co = HashMap::new();
        let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let stmt = Stmt::Let {
            pattern: Pattern::Binding("z".into()),
            ty: None,
            value: Expr::Integer(0),
        };
        let chir = ctx.lower_stmt(&stmt).unwrap();
        if let CHIRStmt::Let { local_idx, .. } = chir {
            assert_eq!(
                ctx.get_local_ty(local_idx),
                Some(wasm_encoder::ValType::I64)
            );
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

        let fi = HashMap::new();
        let fp = HashMap::new();
        let mut so = HashMap::new();
        let mut po = HashMap::new();
        po.insert("x".into(), 8u32);
        so.insert("Point".into(), po);
        let co = HashMap::new();
        let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);
        ctx.alloc_local_typed("obj".into(), wasm_encoder::ValType::I32);

        let stmt = Stmt::Assign {
            target: AssignTarget::Field {
                object: "obj".into(),
                field: "x".into(),
            },
            value: Expr::Integer(100),
        };
        let chir = ctx.lower_stmt(&stmt).unwrap();
        assert!(matches!(chir, CHIRStmt::Assign { .. }));
    }

    #[test]
    fn test_lower_assign_to_index() {
        let mut type_ctx = TypeInferenceContext::new();
        type_ctx.add_local("arr".into(), Type::Array(Box::new(Type::Int64)));
        let fi = HashMap::new();
        let fp = HashMap::new();
        let so = HashMap::new();
        let co = HashMap::new();
        let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);
        ctx.alloc_local_typed("arr".into(), wasm_encoder::ValType::I32);

        let stmt = Stmt::Assign {
            target: AssignTarget::Index {
                array: "arr".into(),
                index: Box::new(Expr::Integer(0)),
            },
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

        let fi = HashMap::new();
        let fp = HashMap::new();
        let mut so = HashMap::new();
        let mut outer_off = HashMap::new();
        outer_off.insert("inner".into(), 8u32);
        so.insert("Outer".into(), outer_off);
        let mut inner_off = HashMap::new();
        inner_off.insert("v".into(), 8u32);
        so.insert("Inner".into(), inner_off);
        let co = HashMap::new();
        let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);
        ctx.alloc_local_typed("o".into(), wasm_encoder::ValType::I32);

        let stmt = Stmt::Assign {
            target: AssignTarget::FieldPath {
                base: "o".into(),
                fields: vec!["inner".into(), "v".into()],
            },
            value: Expr::Integer(1),
        };
        let chir = ctx.lower_stmt(&stmt).unwrap();
        assert!(matches!(chir, CHIRStmt::Assign { .. }));
    }

    #[test]
    fn test_lower_assign_expr_index() {
        let mut type_ctx = TypeInferenceContext::new();
        type_ctx.add_local("arr".into(), Type::Array(Box::new(Type::Int64)));
        let fi = HashMap::new();
        let fp = HashMap::new();
        let so = HashMap::new();
        let co = HashMap::new();
        let ci = HashMap::new();
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
        let fi = HashMap::new();
        let fp = HashMap::new();
        let so = HashMap::new();
        let co = HashMap::new();
        let ci = HashMap::new();
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
        let fi = HashMap::new();
        let fp = HashMap::new();
        let so = HashMap::new();
        let co = HashMap::new();
        let ci = HashMap::new();
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
        let fi = HashMap::new();
        let fp = HashMap::new();
        let so = HashMap::new();
        let co = HashMap::new();
        let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let stmt = Stmt::DoWhile {
            body: vec![Stmt::Break],
            cond: Expr::Bool(false),
        };
        let chir = ctx.lower_stmt(&stmt).unwrap();
        assert!(matches!(chir, CHIRStmt::Loop { .. }));
    }

    #[test]
    fn test_lower_nested_loops() {
        let type_ctx = TypeInferenceContext::new();
        let fi = HashMap::new();
        let fp = HashMap::new();
        let so = HashMap::new();
        let co = HashMap::new();
        let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let stmt = Stmt::While {
            cond: Expr::Bool(true),
            body: vec![Stmt::Loop {
                body: vec![Stmt::Break],
            }],
        };
        let chir = ctx.lower_stmt(&stmt).unwrap();
        assert!(matches!(chir, CHIRStmt::While { .. }));
    }

    #[test]
    fn test_lower_block_trailing_expr() {
        let type_ctx = TypeInferenceContext::new();
        let fi = HashMap::new();
        let fp = HashMap::new();
        let so = HashMap::new();
        let co = HashMap::new();
        let ci = HashMap::new();
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
        let fi = HashMap::new();
        let fp = HashMap::new();
        let so = HashMap::new();
        let co = HashMap::new();
        let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);
        ctx.return_wasm_ty = Some(wasm_encoder::ValType::I64);
        ctx.alloc_local_typed("x".into(), wasm_encoder::ValType::F64);

        let stmt = Stmt::Return(Some(Expr::Var("x".into())));
        let chir = ctx.lower_stmt(&stmt).unwrap();
        assert!(matches!(chir, CHIRStmt::Return(Some(_))));
    }
}
