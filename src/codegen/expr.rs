//! 表达式与语句代码生成：Expr/Stmt 到 WASM Instruction（字面量、Call、Binary、控制流、Lambda、Match 等）。

use crate::ast::Function as FuncDef;
use crate::ast::{
    AssignTarget, BinOp, EnumDef, EnumVariant, Expr, FieldDef, InterpolatePart, Literal, MatchArm,
    Param, Pattern, Stmt, StructDef, Type, UnaryOp,
};
use crate::memory;
use std::collections::HashMap;
use wasm_encoder::{BlockType, Function as WasmFunc, Instruction, MemArg, ValType};

use super::{CodeGen, LocalsBuilder, IOVEC_OFFSET, NWRITTEN_OFFSET};

impl CodeGen {
    /// 递归收集模式中的所有绑定变量
    fn collect_pattern_bindings(
        &self,
        pattern: &Pattern,
        locals: &mut LocalsBuilder,
        subject_type: Option<&Type>,
    ) {
        match pattern {
            Pattern::Binding(name) => {
                let ty = subject_type.map(|t| t.to_wasm()).unwrap_or(ValType::I32);
                locals.add(name, ty, subject_type.cloned());
            }
            Pattern::Variant {
                enum_name,
                variant_name,
                payload,
            } => {
                if let Some(payload_pattern) = payload {
                    if let Some(payload_type) =
                        self.resolve_variant_payload(enum_name, variant_name, subject_type)
                    {
                        self.collect_pattern_bindings(payload_pattern, locals, Some(&payload_type));
                    } else {
                        self.collect_pattern_bindings(payload_pattern, locals, None);
                    }
                }
            }
            Pattern::Tuple(patterns) => {
                // 元组解构：尝试从 subject_type 获取元素类型
                if let Some(Type::Tuple(elem_types)) = subject_type {
                    for (i, pat) in patterns.iter().enumerate() {
                        let elem_type = elem_types.get(i);
                        self.collect_pattern_bindings(pat, locals, elem_type);
                    }
                } else {
                    for pat in patterns {
                        self.collect_pattern_bindings(pat, locals, None);
                    }
                }
            }
            Pattern::Struct {
                name: struct_name,
                fields,
            } => {
                if let Some(def) = self.structs.get(struct_name) {
                    for (fname, pat) in fields {
                        if let Some(ft) = def.field_type(fname) {
                            self.collect_pattern_bindings(pat, locals, Some(&ft));
                        } else {
                            self.collect_pattern_bindings(pat, locals, None);
                        }
                    }
                }
            }
            Pattern::Or(patterns) => {
                // Or 模式：收集所有分支的绑定
                for pat in patterns {
                    self.collect_pattern_bindings(pat, locals, subject_type);
                }
            }
            _ => {}
        }
    }

    /// 编译模式绑定：假设值已在栈顶，将其绑定到模式中的变量
    fn compile_pattern_binding(
        &self,
        pattern: &Pattern,
        value_type: &Type,
        locals: &LocalsBuilder,
        func: &mut WasmFunc,
    ) {
        match pattern {
            Pattern::Binding(name) => {
                // 简单绑定：直接存储到局部变量
                if let Some(idx) = locals.get(name) {
                    func.instruction(&Instruction::LocalSet(idx));
                } else {
                    // 变量未找到，丢弃值
                    func.instruction(&Instruction::Drop);
                }
            }
            Pattern::Tuple(patterns) => {
                // 元组解构：值是指针，需要加载每个元素
                // 先保存指针到临时变量
                let tuple_ptr = locals.get("__tuple_ptr").unwrap_or(0);
                func.instruction(&Instruction::LocalSet(tuple_ptr));

                if let Type::Tuple(elem_types) = value_type {
                    let mut offset = 0u32;
                    for (i, pat) in patterns.iter().enumerate() {
                        if let Some(elem_ty) = elem_types.get(i) {
                            // 加载元素
                            func.instruction(&Instruction::LocalGet(tuple_ptr));
                            if offset > 0 {
                                func.instruction(&Instruction::I32Const(offset as i32));
                                func.instruction(&Instruction::I32Add);
                            }
                            self.emit_load_by_type(func, elem_ty);

                            // 递归绑定
                            self.compile_pattern_binding(pat, elem_ty, locals, func);

                            offset += elem_ty.size() as u32;
                        }
                    }
                }
            }
            Pattern::Wildcard => {
                // 通配符：丢弃值
                func.instruction(&Instruction::Drop);
            }
            _ => {
                // 其他模式暂不支持嵌套绑定，丢弃值
                func.instruction(&Instruction::Drop);
            }
        }
    }

    pub(crate) fn collect_locals(&self, stmt: &Stmt, locals: &mut LocalsBuilder) {
        match stmt {
            Stmt::Let { pattern, ty, value } => {
                match pattern {
                    Pattern::Binding(name) => {
                        let val_type = ty.as_ref().map(|t| t.to_wasm()).unwrap_or_else(|| {
                            // 优先使用带 locals 上下文的 AST 类型推断（更精确）
                            self.infer_ast_type_with_locals(value, locals)
                                .filter(|t| t != &Type::Unit && t != &Type::Nothing)
                                .map(|t| t.to_wasm())
                                .unwrap_or_else(|| self.infer_type(value))
                        });
                        let ast_type = ty
                            .clone()
                            .or_else(|| self.infer_ast_type_with_locals(value, locals))
                            .or_else(|| self.infer_ast_type(value));
                        locals.add(name, val_type, ast_type);
                    }
                    Pattern::Tuple(patterns) => {
                        // 元组解构：let (x, y) = tuple
                        locals.add("__let_tuple_ptr", ValType::I32, None);
                        let value_ast_ty = self.infer_ast_type_with_locals(value, locals);
                        if let Some(Type::Tuple(elem_types)) = value_ast_ty {
                            for (i, pat) in patterns.iter().enumerate() {
                                if let Pattern::Binding(name) = pat {
                                    if let Some(elem_ty) = elem_types.get(i) {
                                        locals.add(name, elem_ty.to_wasm(), Some(elem_ty.clone()));
                                    } else {
                                        locals.add(name, ValType::I32, None);
                                    }
                                }
                            }
                        } else {
                            // 类型推断失败，保守使用 I32（apply_type_corrections 会在 Pass 5 修正）
                            for pat in patterns {
                                if let Pattern::Binding(name) = pat {
                                    locals.add(name, ValType::I32, None);
                                }
                            }
                        }
                    }
                    Pattern::Struct {
                        name: struct_name,
                        fields,
                    } => {
                        locals.add("__let_struct_ptr", ValType::I32, None);
                        if let Some(def) = self.structs.get(struct_name) {
                            for (fname, pat) in fields {
                                if let Pattern::Binding(bind) = pat {
                                    if let Some(ft) = def.field_type(fname) {
                                        locals.add(bind, ft.to_wasm(), Some(ft.clone()));
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
                self.collect_locals_from_expr(value, locals);
            }
            Stmt::Var { pattern, ty, value } => {
                match pattern {
                    Pattern::Binding(name) => {
                        let val_type = ty.as_ref().map(|t| t.to_wasm()).unwrap_or_else(|| {
                            self.infer_ast_type_with_locals(value, locals)
                                .filter(|t| !matches!(t, Type::Unit | Type::Nothing))
                                .map(|t| t.to_wasm())
                                .unwrap_or_else(|| self.infer_type(value))
                        });
                        let ast_type = ty
                            .clone()
                            .or_else(|| self.infer_ast_type_with_locals(value, locals))
                            .or_else(|| self.infer_ast_type(value));
                        locals.add(name, val_type, ast_type);
                    }
                    Pattern::Tuple(patterns) => {
                        locals.add("__var_tuple_ptr", ValType::I32, None);
                        let value_ty = ty
                            .clone()
                            .or_else(|| self.infer_ast_type_with_locals(value, locals));
                        if let Some(Type::Tuple(types)) = value_ty.as_ref() {
                            for (i, pat) in patterns.iter().enumerate() {
                                if let Pattern::Binding(name) = pat {
                                    if let Some(t) = types.get(i) {
                                        locals.add(name, t.to_wasm(), Some(t.clone()));
                                    } else {
                                        locals.add(name, ValType::I32, None);
                                    }
                                }
                            }
                        } else {
                            for pat in patterns {
                                if let Pattern::Binding(name) = pat {
                                    locals.add(name, ValType::I32, None);
                                }
                            }
                        }
                    }
                    Pattern::Struct {
                        name: struct_name,
                        fields,
                    } => {
                        locals.add("__var_struct_ptr", ValType::I32, None);
                        if let Some(def) = self.structs.get(struct_name) {
                            for (fname, pat) in fields {
                                if let Pattern::Binding(bind) = pat {
                                    if let Some(ft) = def.field_type(fname) {
                                        locals.add(bind, ft.to_wasm(), Some(ft.clone()));
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
                self.collect_locals_from_expr(value, locals);
            }
            Stmt::Assign { value, .. } => {
                self.collect_locals_from_expr(value, locals);
            }
            Stmt::LocalFunc(_) => {}
            Stmt::Expr(expr) => {
                self.collect_locals_from_expr(expr, locals);
            }
            Stmt::Return(Some(expr)) => {
                self.collect_locals_from_expr(expr, locals);
            }
            Stmt::While { cond, body } => {
                self.collect_locals_from_expr(cond, locals);
                for s in body {
                    self.collect_locals(s, locals);
                }
            }
            Stmt::DoWhile { body, cond } => {
                for s in body {
                    self.collect_locals(s, locals);
                }
                self.collect_locals_from_expr(cond, locals);
            }
            Stmt::Const { name, ty, value } => {
                let val_type = ty.as_ref().map(|t| t.to_wasm()).unwrap_or_else(|| {
                    self.infer_ast_type_with_locals(value, locals)
                        .map(|t| t.to_wasm())
                        .unwrap_or_else(|| self.infer_type(value)) // 用 infer_type 而非硬编码 I64
                });
                let ast_type = ty
                    .clone()
                    .or_else(|| self.infer_ast_type_with_locals(value, locals));
                locals.add(name, val_type, ast_type);
                self.collect_locals_from_expr(value, locals);
            }
            Stmt::WhileLet {
                pattern,
                expr,
                body,
            } => {
                self.collect_locals_from_expr(expr, locals);
                let subject_ast_type = self.infer_ast_type_with_locals(expr, locals);
                locals.add("__match_enum_ptr", ValType::I32, None);
                match pattern {
                    Pattern::Binding(name) => {
                        locals.add(name, ValType::I32, None); // 枚举载荷通常是引用（I32）
                    }
                    Pattern::Variant {
                        enum_name,
                        variant_name,
                        payload,
                    } => {
                        if let Some(payload_pattern) = payload {
                            if let Some(ty) = self.resolve_variant_payload(
                                enum_name,
                                variant_name,
                                subject_ast_type.as_ref(),
                            ) {
                                self.collect_pattern_bindings(payload_pattern, locals, Some(&ty));
                            } else {
                                self.collect_pattern_bindings(payload_pattern, locals, None);
                            }
                        }
                    }
                    Pattern::Struct {
                        name: struct_name,
                        fields,
                    } => {
                        if let Some(def) = self.structs.get(struct_name) {
                            for (fname, pat) in fields {
                                if let Pattern::Binding(bind) = pat {
                                    if let Some(ft) = def.field_type(fname) {
                                        locals.add(bind, ft.to_wasm(), Some(ft.clone()));
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
                for s in body {
                    self.collect_locals(s, locals);
                }
            }
            Stmt::Loop { body } => {
                for s in body {
                    self.collect_locals(s, locals);
                }
            }
            Stmt::UnsafeBlock { body } => {
                for s in body {
                    self.collect_locals(s, locals);
                }
            }
            Stmt::For {
                var,
                iterable,
                body,
            } => {
                locals.add(var, ValType::I64, self.expr_object_type(iterable)); // 循环变量：范围时为 Int64，数组时为元素类型
                if !matches!(iterable, Expr::Range { .. }) {
                    locals.add(&format!("__{}_idx", var), ValType::I64, None);
                    locals.add(&format!("__{}_len", var), ValType::I64, None);
                    locals.add(&format!("__{}_arr", var), ValType::I32, None);
                }
                self.collect_locals_from_expr(iterable, locals);
                for s in body {
                    self.collect_locals(s, locals);
                }
            }
            Stmt::Assert { left, right, .. } | Stmt::Expect { left, right, .. } => {
                self.collect_locals_from_expr(left, locals);
                self.collect_locals_from_expr(right, locals);
            }
            _ => {}
        }
    }

    /// 从表达式中收集局部变量（含 match 分支绑定名，使 `x if x < 0` 中的 x 可用）
    fn collect_locals_from_expr(&self, expr: &Expr, locals: &mut LocalsBuilder) {
        match expr {
            Expr::Match { expr: sub, arms } => {
                self.collect_locals_from_expr(sub, locals);
                let subject_ast_type = self.infer_ast_type_with_locals(sub, locals);
                locals.add("__match_enum_ptr", ValType::I32, None); // 关联值枚举 match 时暂存 ptr
                locals.add("__match_val", ValType::I32, None); // P3.5: TypeTest 暂存 subject
                locals.add("__tuple_ptr", ValType::I32, None); // 元组解构时暂存 ptr
                for arm in arms {
                    match &arm.pattern {
                        Pattern::Binding(name) => {
                            // 使用 subject 类型，未知时保持 I64（原行为，避免回退导致类型错误）
                            let ty = subject_ast_type
                                .as_ref()
                                .map(|t| t.to_wasm())
                                .unwrap_or(ValType::I64);
                            locals.add(name, ty, subject_ast_type.clone());
                        }
                        Pattern::Variant {
                            enum_name,
                            variant_name,
                            payload,
                        } => {
                            if let Some(payload_pattern) = payload {
                                if let Some(ty) = self.resolve_variant_payload(
                                    enum_name,
                                    variant_name,
                                    subject_ast_type.as_ref(),
                                ) {
                                    self.collect_pattern_bindings(
                                        payload_pattern,
                                        locals,
                                        Some(&ty),
                                    );
                                } else {
                                    self.collect_pattern_bindings(payload_pattern, locals, None);
                                }
                            }
                        }
                        Pattern::Struct {
                            name: struct_name,
                            fields,
                        } => {
                            if let Some(def) = self.structs.get(struct_name) {
                                for (fname, pat) in fields {
                                    if let Pattern::Binding(bind) = pat {
                                        if let Some(ft) = def.field_type(fname) {
                                            locals.add(bind, ft.to_wasm(), Some(ft.clone()));
                                        }
                                    }
                                }
                            }
                        }
                        // P3.5: TypeTest pattern
                        Pattern::TypeTest { binding, ty } => {
                            locals.add(binding, ty.to_wasm(), Some(ty.clone()));
                        }
                        _ => {}
                    }
                    self.collect_locals_from_expr(&arm.body, locals);
                    if let Some(g) = &arm.guard {
                        self.collect_locals_from_expr(g, locals);
                    }
                }
            }
            Expr::IfLet {
                pattern,
                expr,
                then_branch,
                else_branch,
            } => {
                self.collect_locals_from_expr(expr, locals);
                let subject_ast_type = self.infer_ast_type_with_locals(expr, locals);
                locals.add("__match_enum_ptr", ValType::I32, None);
                match pattern {
                    Pattern::Binding(name) => {
                        // 使用 subject 类型，未知时保持 I64（原行为）
                        let ty = subject_ast_type
                            .as_ref()
                            .map(|t| t.to_wasm())
                            .unwrap_or(ValType::I64);
                        locals.add(name, ty, subject_ast_type.clone());
                    }
                    Pattern::Variant {
                        enum_name,
                        variant_name,
                        payload,
                    } => {
                        if let Some(payload_pattern) = payload {
                            if let Some(ty) = self.resolve_variant_payload(
                                enum_name,
                                variant_name,
                                subject_ast_type.as_ref(),
                            ) {
                                self.collect_pattern_bindings(payload_pattern, locals, Some(&ty));
                            } else {
                                self.collect_pattern_bindings(payload_pattern, locals, None);
                            }
                        }
                    }
                    Pattern::Struct {
                        name: struct_name,
                        fields,
                    } => {
                        if let Some(def) = self.structs.get(struct_name) {
                            for (fname, pat) in fields {
                                if let Pattern::Binding(bind) = pat {
                                    if let Some(ft) = def.field_type(fname) {
                                        locals.add(bind, ft.to_wasm(), Some(ft.clone()));
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
                self.collect_locals_from_expr(then_branch, locals);
                if let Some(eb) = else_branch {
                    self.collect_locals_from_expr(eb, locals);
                }
            }
            Expr::Tuple(elements) => {
                for e in elements {
                    self.collect_locals_from_expr(e, locals);
                }
                locals.add("__tuple_alloc_ptr", ValType::I32, None);
            }
            Expr::TupleIndex { object, .. } => {
                self.collect_locals_from_expr(object, locals);
            }
            Expr::NullCoalesce { option, default } => {
                self.collect_locals_from_expr(option, locals);
                self.collect_locals_from_expr(default, locals);
            }
            Expr::Binary { left, right, .. } => {
                self.collect_locals_from_expr(left, locals);
                self.collect_locals_from_expr(right, locals);
            }
            Expr::Call { args, .. } => {
                for arg in args {
                    self.collect_locals_from_expr(arg, locals);
                }
            }
            Expr::MethodCall {
                object,
                method,
                args,
                ..
            } => {
                self.collect_locals_from_expr(object, locals);
                for arg in args {
                    self.collect_locals_from_expr(arg, locals);
                }
                // Phase 7.2: compareTo 需要临时变量
                if method == "compareTo" {
                    locals.add("__cmp_x", ValType::I64, None);
                    locals.add("__cmp_y", ValType::I64, None);
                }
                // P2.8: Array 实例方法需要临时变量
                if matches!(method.as_str(), "clone" | "slice" | "copyTo") {
                    locals.add("__array_clone_src", ValType::I32, None);
                    locals.add("__array_clone_dst", ValType::I32, None);
                    locals.add("__array_dyn_ptr", ValType::I32, None);
                    locals.add("__array_dyn_size", ValType::I64, None);
                    locals.add("__array_dyn_idx", ValType::I64, None);
                }
            }
            Expr::SuperCall { args, .. } => {
                for arg in args {
                    self.collect_locals_from_expr(arg, locals);
                }
            }
            Expr::SuperFieldAccess { .. } => {
                // super 字段访问不需要收集局部变量
            }
            Expr::If {
                cond,
                then_branch,
                else_branch,
            } => {
                self.collect_locals_from_expr(cond, locals);
                self.collect_locals_from_expr(then_branch, locals);
                if let Some(e) = else_branch {
                    self.collect_locals_from_expr(e, locals);
                }
            }
            Expr::Block(stmts, result) => {
                for s in stmts {
                    self.collect_locals(s, locals);
                }
                if let Some(e) = result {
                    self.collect_locals_from_expr(e, locals);
                }
            }
            Expr::Array(elems) => {
                for e in elems {
                    self.collect_locals_from_expr(e, locals);
                }
                locals.add("__array_alloc_ptr", ValType::I32, None);
            }
            Expr::Index { array, index } => {
                self.collect_locals_from_expr(array, locals);
                self.collect_locals_from_expr(index, locals);
            }
            Expr::SliceExpr { array, start, end } => {
                self.collect_locals_from_expr(array, locals);
                self.collect_locals_from_expr(start, locals);
                self.collect_locals_from_expr(end, locals);
                locals.add("__array_clone_src", ValType::I32, None);
                locals.add("__array_clone_dst", ValType::I32, None);
                locals.add("__array_dyn_ptr", ValType::I32, None);
                locals.add("__array_dyn_size", ValType::I64, None);
                locals.add("__array_dyn_idx", ValType::I64, None);
            }
            Expr::StructInit { fields, .. } => {
                for (_, e) in fields {
                    self.collect_locals_from_expr(e, locals);
                }
                locals.add("__struct_alloc_ptr", ValType::I32, None);
            }
            Expr::ConstructorCall { name, args, .. } => {
                for e in args {
                    self.collect_locals_from_expr(e, locals);
                }
                // P2.7: Array<T>(size, init) 需要临时变量
                if name == "Array" {
                    locals.add("__array_dyn_ptr", ValType::I32, None);
                    locals.add("__array_dyn_size", ValType::I64, None);
                    locals.add("__array_dyn_idx", ValType::I64, None);
                }
            }
            Expr::Field { object, .. } => {
                self.collect_locals_from_expr(object, locals);
            }
            Expr::Unary { expr, .. } => {
                self.collect_locals_from_expr(expr, locals);
            }
            Expr::Range { start, end, .. } => {
                self.collect_locals_from_expr(start, locals);
                self.collect_locals_from_expr(end, locals);
                locals.add("__range_alloc_ptr", ValType::I32, None);
            }
            Expr::Cast { expr, .. } | Expr::IsType { expr, .. } => {
                self.collect_locals_from_expr(expr, locals);
            }
            Expr::VariantConst { arg: Some(e), .. } => {
                self.collect_locals_from_expr(e, locals);
                locals.add("__enum_alloc_ptr", ValType::I32, None);
            }
            Expr::VariantConst { .. } => {}
            Expr::Lambda { body, .. } => {
                self.collect_locals_from_expr(body, locals);
            }
            Expr::Some(inner)
            | Expr::Ok(inner)
            | Expr::Err(inner)
            | Expr::Try(inner)
            | Expr::Throw(inner) => {
                self.collect_locals_from_expr(inner, locals);
            }
            Expr::PostfixIncr(inner) | Expr::PostfixDecr(inner) => {
                self.collect_locals_from_expr(inner, locals);
                // P5.4: __postfix_old 的类型与操作数类型一致，避免 i32/i64 类型不匹配
                let inner_vt = self
                    .infer_ast_type_with_locals(inner, locals)
                    .map(|t| t.to_wasm())
                    .unwrap_or(ValType::I64);
                locals.add("__postfix_old", inner_vt, None);
            }
            Expr::PrefixIncr(inner) | Expr::PrefixDecr(inner) => {
                self.collect_locals_from_expr(inner, locals);
            }
            Expr::None => {}
            Expr::TryBlock {
                resources,
                body,
                catch_body,
                catch_var,
                catch_type,
                finally_body,
            } => {
                // P6: try-with-resources — 注册资源变量为局部变量
                for (res_name, res_expr) in resources {
                    let res_type = self.infer_type(res_expr);
                    locals.add(res_name, res_type, None);
                    self.collect_locals_from_expr(res_expr, locals);
                }
                // 预分配 try-catch-finally 所需的内部局部变量
                // 推断 throw 表达式的值类型，以确保 __err_val 类型匹配
                let err_val_type = Self::find_throw_inner_in_stmts(body)
                    .map(|inner| self.infer_type(inner))
                    .unwrap_or(ValType::I32);
                locals.add("__err_flag", ValType::I32, None);
                locals.add("__err_val", err_val_type, None);
                // Bug B6 修复: 为 try-catch 表达式结果添加临时变量
                // 推断 try body 最后一条表达式的类型作为结果类型
                let try_result_type = body
                    .last()
                    .and_then(|s| {
                        if let Stmt::Expr(e) = s {
                            Some(self.infer_type(e))
                        } else {
                            None
                        }
                    })
                    .unwrap_or(ValType::I32); // I32 是更安全的默认值（指针/引用类型）
                locals.add("__try_result", try_result_type, None);
                for stmt in body {
                    self.collect_locals(stmt, locals);
                }
                // catch 变量用 __err_val 的实际类型（可能被嵌套 try 升级）
                let actual_err_val_type = locals
                    .get("__err_val")
                    .map(|idx| locals.types[idx as usize])
                    .unwrap_or(err_val_type);
                if let Some(var) = catch_var {
                    locals.add(var, actual_err_val_type, None);
                }
                for stmt in catch_body {
                    self.collect_locals(stmt, locals);
                }
                if let Some(finally_stmts) = finally_body {
                    for stmt in finally_stmts {
                        self.collect_locals(stmt, locals);
                    }
                }
            }
            // P5.1/5.2: spawn/synchronized 块中的局部变量
            Expr::Spawn { body } => {
                for stmt in body {
                    self.collect_locals(stmt, locals);
                }
            }
            Expr::Synchronized { lock, body } => {
                self.collect_locals_from_expr(lock, locals);
                for stmt in body {
                    self.collect_locals(stmt, locals);
                }
            }
            Expr::OptionalChain { object, .. } => {
                self.collect_locals_from_expr(object, locals);
                locals.add("__match_val", ValType::I32, None);
            }
            Expr::TrailingClosure {
                callee,
                args,
                closure,
            } => {
                self.collect_locals_from_expr(callee, locals);
                for a in args {
                    self.collect_locals_from_expr(a, locals);
                }
                self.collect_locals_from_expr(closure, locals);
            }
            _ => {}
        }
    }

    /// 解析 Pattern::Variant 的 payload 类型（先查用户定义枚举，再查内建 Option/Result）
    fn resolve_variant_payload(
        &self,
        enum_name: &str,
        variant_name: &str,
        subject_ast_type: Option<&Type>,
    ) -> Option<Type> {
        // 1) 用户定义的枚举
        if let Some(ty) = self
            .enums
            .get(enum_name)
            .and_then(|e| e.variant_payload(variant_name))
        {
            return Some(ty.clone());
        }
        // 2) 内建 Option<T>
        if enum_name == "Option" {
            if variant_name == "Some" {
                if let Some(Type::Option(inner)) = subject_ast_type {
                    return Some(inner.as_ref().clone());
                }
                return Some(Type::Int64); // fallback
            }
            return None; // None 无 payload
        }
        // 3) 内建 Result<T, E>
        if enum_name == "Result" {
            if variant_name == "Ok" {
                if let Some(Type::Result(ok, _)) = subject_ast_type {
                    return Some(ok.as_ref().clone());
                }
                return Some(Type::Int64); // fallback
            }
            if variant_name == "Err" {
                if let Some(Type::Result(_, err)) = subject_ast_type {
                    return Some(err.as_ref().clone());
                }
                return Some(Type::String); // fallback
            }
        }
        None
    }

    /// 从表达式推断 AST 类型（用于局部变量类型注解缺失时）
    pub(crate) fn infer_ast_type(&self, expr: &Expr) -> Option<Type> {
        match expr {
            Expr::Integer(_) => Some(Type::Int64),
            Expr::Float(_) => Some(Type::Float64),
            Expr::Float32(_) => Some(Type::Float32),
            Expr::Bool(_) => Some(Type::Bool),
            Expr::Rune(_) => Some(Type::Rune),
            Expr::String(_) => Some(Type::String),
            Expr::Tuple(ref elems) => {
                let types: Vec<Type> = elems
                    .iter()
                    .filter_map(|e| self.infer_ast_type(e))
                    .collect();
                if types.len() == elems.len() {
                    Some(Type::Tuple(types))
                } else {
                    None
                }
            }
            Expr::TupleIndex { object, index } => self.infer_ast_type(object).and_then(|ty| {
                if let Type::Tuple(types) = ty {
                    types.get(*index as usize).cloned()
                } else {
                    None
                }
            }),
            Expr::NullCoalesce { default, .. } => self.infer_ast_type(default),
            Expr::Array(ref elems) => elems
                .first()
                .and_then(|e| self.infer_ast_type(e).map(|t| Type::Array(Box::new(t))))
                .or(Some(Type::Array(Box::new(Type::Int64)))),
            Expr::StructInit {
                name, type_args, ..
            } => Some(Type::Struct(
                name.clone(),
                type_args.clone().unwrap_or_default(),
            )),
            Expr::ConstructorCall {
                name, type_args, ..
            } => {
                match name.as_str() {
                    "Array" => {
                        let elem = type_args
                            .as_ref()
                            .and_then(|ta| ta.first().cloned())
                            .unwrap_or(Type::Int64);
                        return Some(Type::Array(Box::new(elem)));
                    }
                    "ArrayList" | "LinkedList" | "ArrayStack" => {
                        let elem = type_args
                            .as_ref()
                            .and_then(|ta| ta.first().cloned())
                            .unwrap_or(Type::Int64);
                        return Some(Type::Array(Box::new(elem)));
                    }
                    "HashMap" => {
                        let k = type_args
                            .as_ref()
                            .and_then(|ta| ta.first().cloned())
                            .unwrap_or(Type::Int64);
                        let v = type_args
                            .as_ref()
                            .and_then(|ta| ta.get(1).cloned())
                            .unwrap_or(Type::Int64);
                        return Some(Type::Map(Box::new(k), Box::new(v)));
                    }
                    "HashSet" => {
                        let elem = type_args
                            .as_ref()
                            .and_then(|ta| ta.first().cloned())
                            .unwrap_or(Type::Int64);
                        return Some(Type::Map(Box::new(elem), Box::new(Type::Int64)));
                    }
                    // P5: Atomic/Mutex 桩类型（infer_ast_type）
                    "AtomicInt64" | "AtomicBool" | "Mutex" | "ReentrantMutex" => {
                        return Some(Type::Struct(name.clone(), vec![]));
                    }
                    _ => {}
                }
                Some(Type::Struct(
                    name.clone(),
                    type_args.clone().unwrap_or_default(),
                ))
            }
            Expr::VariantConst { enum_name, .. } => Some(Type::Struct(enum_name.clone(), vec![])),
            Expr::Call {
                name,
                type_args,
                args,
                ..
            } => {
                // 类型构造函数：Float32(x), Float64(x), Int64(x) 等
                match name.as_str() {
                    "Int8" => return Some(Type::Int8),
                    "Int16" => return Some(Type::Int16),
                    "Int32" => return Some(Type::Int32),
                    "Int64" => return Some(Type::Int64),
                    "UInt8" => return Some(Type::UInt8),
                    "UInt16" => return Some(Type::UInt16),
                    "UInt32" => return Some(Type::UInt32),
                    "UInt64" => return Some(Type::UInt64),
                    "Float32" => return Some(Type::Float32),
                    "Float64" => return Some(Type::Float64),
                    "Bool" => return Some(Type::Bool),
                    "Rune" => return Some(Type::Rune),
                    "readln" | "getEnv" => return Some(Type::String),
                    "now" | "randomInt64" => return Some(Type::Int64),
                    "randomFloat64" => return Some(Type::Float64),
                    "getArgs" => return Some(Type::Array(Box::new(Type::String))),
                    // P4: 集合类型推断
                    "ArrayList" => {
                        let elem = type_args
                            .as_ref()
                            .and_then(|ta| ta.first().cloned())
                            .unwrap_or(Type::Int64);
                        return Some(Type::Array(Box::new(elem)));
                    }
                    "HashMap" => {
                        let k = type_args
                            .as_ref()
                            .and_then(|ta| ta.first().cloned())
                            .unwrap_or(Type::Int64);
                        let v = type_args
                            .as_ref()
                            .and_then(|ta| ta.get(1).cloned())
                            .unwrap_or(Type::Int64);
                        return Some(Type::Map(Box::new(k), Box::new(v)));
                    }
                    "HashSet" => {
                        let elem = type_args
                            .as_ref()
                            .and_then(|ta| ta.first().cloned())
                            .unwrap_or(Type::Int64);
                        return Some(Type::Map(Box::new(elem), Box::new(Type::Int64)));
                    }
                    "LinkedList" | "ArrayStack" => {
                        let elem = type_args
                            .as_ref()
                            .and_then(|ta| ta.first().cloned())
                            .unwrap_or(Type::Int64);
                        return Some(Type::Array(Box::new(elem)));
                    }
                    _ => {}
                }
                // P4.1: class 构造函数调用也返回 Struct 类型（之前只检查 structs）
                if self.structs.contains_key(name) || self.classes.contains_key(name) {
                    Some(Type::Struct(name.clone(), vec![]))
                } else if Self::is_math_builtin(name) && !self.func_indices.contains_key(name) {
                    Some(Type::Float64) // math 内置函数返回 Float64
                } else if (name == "min" || name == "max") && args.len() == 2
                    || (name == "abs" && args.len() == 1)
                {
                    Some(Type::Int64)
                } else {
                    let arg_tys: Vec<Type> =
                        args.iter().filter_map(|a| self.infer_ast_type(a)).collect();
                    let key = if *self.name_count.get(name).unwrap_or(&0) > 1 {
                        if arg_tys.len() == args.len() {
                            Some(Self::mangle_key(name, &arg_tys))
                        } else {
                            None
                        }
                    } else {
                        Some(name.to_string())
                    };
                    // P0: func_return_types 找不到时，查询标准库构造函数元数据
                    key.and_then(|k| self.func_return_types.get(&k).cloned())
                        .or_else(|| {
                            let ta = type_args.as_deref().unwrap_or(&[]);
                            crate::metadata::stdlib_constructor_type(name, ta)
                        })
                }
            }
            Expr::Interpolate(_) => Some(Type::String), // 字符串插值结果是 String
            Expr::MethodCall { object, method, .. } => {
                // Phase 7.2: 先检查内建类型方法返回类型
                let obj_ty = self.infer_ast_type(object);
                if let Some(ret) = Self::builtin_method_return_type(obj_ty.as_ref(), method) {
                    return Some(ret);
                }
                // P2.4: 静态方法调用 → 对象是类名（不是局部变量）
                if let Expr::Var(ref class_name) = object.as_ref() {
                    if obj_ty.is_none()
                        && (self.structs.contains_key(class_name.as_str())
                            || self.classes.contains_key(class_name.as_str()))
                    {
                        let key = format!("{}.{}", class_name, method);
                        if let Some(ret) = self.func_return_types.get(&key) {
                            return Some(ret.clone());
                        }
                    }
                }
                // 尝试通过对象类型 + 方法名查找 func_return_types
                let func_result = obj_ty.as_ref().and_then(|ty| {
                    let type_name = match ty {
                        Type::Struct(name, _) => Some(name.clone()),
                        Type::Option(_) => Some("Option".to_string()),
                        Type::Result(_, _) => Some("Result".to_string()),
                        _ => None,
                    };
                    type_name.and_then(|tn| {
                        let key = format!("{}.{}", tn, method);
                        self.func_return_types.get(&key).cloned()
                    })
                });
                if func_result.is_some() {
                    return func_result;
                }
                // P0: 查询标准库元数据
                if let Some(Type::Struct(ref type_name, ref type_args)) = obj_ty {
                    if let Some(ret) =
                        crate::metadata::stdlib_method_return_type(type_name, type_args, method)
                    {
                        return Some(ret);
                    }
                }
                // P1: 查询接口方法（已注册到 func_return_types）
                // 接口 prop getter 以 "__get_xxx" 形式注册
                if let Some(Type::Struct(ref type_name, _)) = obj_ty {
                    let getter_key = format!("{}.__get_{}", type_name, method);
                    if let Some(ret) = self.func_return_types.get(&getter_key) {
                        return Some(ret.clone());
                    }
                }
                None
            }
            Expr::SuperCall { .. } => None, // super 调用，需结合父类推断
            Expr::SuperFieldAccess { .. } => None, // super 字段访问，需结合父类推断
            Expr::Cast { target_ty, .. } => Some(target_ty.clone()),
            Expr::IsType { .. } => Some(Type::Bool),
            Expr::IfLet { then_branch, .. } => self.infer_ast_type(then_branch),
            Expr::Lambda {
                params,
                return_type,
                body,
            } => {
                let param_types: Vec<Type> = params.iter().map(|(_, t)| t.clone()).collect();
                let ret = if return_type.is_some() {
                    return_type.clone()
                } else {
                    Self::infer_lambda_return_type(body, params)
                };
                Some(Type::Function {
                    params: param_types,
                    ret: Box::new(ret),
                })
            }
            Expr::Some(inner) => self
                .infer_ast_type(inner)
                .map(|t| Type::Option(Box::new(t))),
            Expr::None => None, // 需要类型注解
            Expr::Ok(inner) => self
                .infer_ast_type(inner)
                .map(|t| Type::Result(Box::new(t), Box::new(Type::String))),
            Expr::Err(inner) => self
                .infer_ast_type(inner)
                .map(|_| Type::Result(Box::new(Type::Int64), Box::new(Type::String))),
            Expr::Try(inner) => {
                // expr? 解包 Option<T> -> T 或 Result<T, E> -> T
                match self.infer_ast_type(inner) {
                    Some(Type::Option(t)) => Some(*t),
                    Some(Type::Result(t, _)) => Some(*t),
                    _ => None,
                }
            }
            Expr::Match { arms, .. } => arms.first().and_then(|arm| self.infer_ast_type(&arm.body)),
            Expr::If {
                then_branch,
                else_branch,
                ..
            } => self
                .infer_ast_type(then_branch)
                .or_else(|| else_branch.as_ref().and_then(|eb| self.infer_ast_type(eb))),
            Expr::Block(stmts, tail) => {
                tail.as_ref()
                    .and_then(|t| self.infer_ast_type(t))
                    .or_else(|| {
                        stmts.last().and_then(|s| {
                            if let Stmt::Expr(ref e) = s {
                                self.infer_ast_type(e)
                            } else {
                                None
                            }
                        })
                    })
            }
            // P4.3: Var 类型推断（不依赖 locals，仅用全局信息）
            Expr::Var(name) => {
                // 检查全局变量类型（顶层 let/var 声明）
                if let Some(ty) = self.global_var_types.get(name.as_str()) {
                    if ty == &Type::Int64 {
                        // Int64 可能是解析器默认占位符，尝试从 init 推断实际类型
                        if let Some(init) = self.global_var_inits.get(name.as_str()) {
                            if let Some(inferred) = self.infer_ast_type(init) {
                                return Some(inferred);
                            }
                        }
                    }
                    return Some(ty.clone());
                }
                // 检查已知类名 / 结构体名 → Struct 类型（用于静态方法调用和构造函数推断）
                if self.structs.contains_key(name.as_str())
                    || self.classes.contains_key(name.as_str())
                {
                    return Some(Type::Struct(name.clone(), vec![]));
                }
                None
            }
            // P4.4: Field 类型推断（不依赖 locals，仅用结构体 / 类全局信息）
            Expr::Field { object, field } => {
                let obj_ty = self.infer_ast_type(object)?;
                if let Type::Struct(ref s, ref type_args) = obj_ty {
                    let lookup_name = if !type_args.is_empty() {
                        let mangled = crate::monomorph::mangle_name(s, type_args);
                        if self.structs.contains_key(&mangled) {
                            mangled
                        } else {
                            s.clone()
                        }
                    } else {
                        s.clone()
                    };
                    // struct 字段
                    let field_ty = self.structs.get(&lookup_name).and_then(|def| {
                        def.fields
                            .iter()
                            .find(|f| f.name == *field)
                            .map(|f| f.ty.clone())
                    });
                    if field_ty.is_some() {
                        return field_ty;
                    }
                    // prop getter
                    let getter_name = format!("{}.__get_{}", s, field);
                    if let Some(ret) = self.func_return_types.get(&getter_name).cloned() {
                        return Some(ret);
                    }
                    // 接口方法 / 字段
                    let iface_key = format!("{}.{}", s, field);
                    if let Some(ret) = self.func_return_types.get(&iface_key).cloned() {
                        return Some(ret);
                    }
                    // class 字段（all_fields 包含继承字段）
                    let class_field = self.classes.get(&lookup_name).and_then(|ci| {
                        ci.all_fields
                            .iter()
                            .find(|f| f.name == *field)
                            .map(|f| f.ty.clone())
                    });
                    if class_field.is_some() {
                        return class_field;
                    }
                    // P0: 标准库字段元数据
                    crate::metadata::stdlib_field_type(s, type_args, field)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Phase 7.1 #42: 尝试从表达式推断 struct/class 名称
    fn try_get_struct_name(&self, expr: &Expr, locals: &LocalsBuilder) -> Option<String> {
        match self.infer_ast_type_with_locals(expr, locals) {
            Some(Type::Struct(name, _)) => Some(name),
            _ => match self.infer_ast_type(expr) {
                Some(Type::Struct(name, _)) => Some(name),
                _ => None,
            },
        }
    }

    /// Phase 7.3: 判断是否为 math 内置函数
    /// Phase 7.2: 内建类型方法的返回类型推断
    fn builtin_method_return_type(obj_type: Option<&Type>, method: &str) -> Option<Type> {
        match obj_type {
            Some(Type::Int64) | Some(Type::Int32) | Some(Type::Int16) | Some(Type::Int8) => {
                match method {
                    "toString" | "format" => Some(Type::String),
                    "toFloat64" => Some(Type::Float64),
                    "abs" => obj_type.cloned(),
                    "compareTo" | "hashCode" => Some(Type::Int64),
                    _ => None,
                }
            }
            Some(Type::Float64) => match method {
                "toString" | "format" => Some(Type::String),
                "toInt64" => Some(Type::Int64),
                _ => None,
            },
            Some(Type::Float32) => match method {
                "toString" => Some(Type::String),
                "toInt64" => Some(Type::Int64),
                "toFloat64" => Some(Type::Float64),
                _ => None,
            },
            Some(Type::Bool) => match method {
                "toString" => Some(Type::String),
                _ => None,
            },
            Some(Type::String) => match method {
                "isEmpty" | "contains" | "startsWith" | "endsWith" | "isBlank" => Some(Type::Bool),
                "toInt64" | "indexOf" => Some(Type::Int64),
                "toFloat64" => Some(Type::Float64),
                "size" => Some(Type::Int64),
                "toString" | "replace" | "toArray" | "trim" => Some(Type::String),
                "split" => Some(Type::Array(Box::new(Type::String))),
                _ => None,
            },
            // P2.8: Array 实例方法返回类型（同时覆盖 ArrayList 代理模式）
            Some(Type::Array(ref elem_ty)) => match method {
                "clone" | "toArray" | "slice" => obj_type.cloned(),
                "isEmpty" => Some(Type::Bool),
                "size" | "indexOf" | "lastIndexOf" => Some(Type::Int64),
                // ArrayList 方法（底层 WASM 函数均返回元素值 i64，非 Option）
                "get" | "first" | "last" | "pop" | "remove" => Some(*elem_ty.clone()),
                // Unit 返回值不暴露（避免 to_wasm() panic）
                "add" | "push" | "append" | "prepend" | "insert" | "set" | "clear" | "sort"
                | "sortBy" | "reverse" => None,
                "contains" => Some(Type::Bool),
                _ => None,
            },
            // Option<T> 方法返回类型
            Some(Type::Option(ref inner_ty)) => match method {
                "getOrThrow" | "unwrap" | "getOrDefault" => Some(*inner_ty.clone()),
                "isNone" | "isSome" => Some(Type::Bool),
                _ => None,
            },
            // Map<K,V> 方法返回类型
            Some(Type::Map(ref _key_ty, ref val_ty)) => match method {
                "get" => Some(*val_ty.clone()),    // 直接返回值类型，不是 Option
                "remove" => Some(*val_ty.clone()), // remove 也返回值类型
                "put" => Some(Type::Unit),
                "containsKey" | "contains" => Some(Type::Bool), // HashMap.containsKey 和 HashSet.contains
                "size" => Some(Type::Int64),
                _ => None,
            },
            // InputStream/Reader/Stream 方法返回类型
            // 只声明有意义的非 Unit 返回类型（read 返回 Int64 字节数）
            // write/flush/close 等 void 方法不在此声明，由 stub 生成 i32.const 0 占位
            Some(Type::Struct(ref name, _))
                if name.contains("InputStream")
                    || name.contains("Reader")
                    || name.contains("Stream") =>
            {
                match method {
                    "read" | "readByte" | "readBytes" | "readLine" => Some(Type::Int64),
                    _ => None,
                }
            }
            // AtomicInt64 方法返回类型
            Some(Type::Struct(ref name, _)) if name == "AtomicInt64" => match method {
                "load" | "fetchAdd" | "fetchSub" | "fetchOr" | "fetchAnd" | "fetchXor" | "swap" => {
                    Some(Type::Int64)
                }
                "compareAndSwap" => Some(Type::Bool),
                "store" => Some(Type::Unit),
                _ => None,
            },
            // AtomicBool 方法返回类型
            Some(Type::Struct(ref name, _)) if name == "AtomicBool" => match method {
                "load" => Some(Type::Bool),
                "fetchAnd" | "fetchOr" | "fetchXor" | "swap" => Some(Type::Bool),
                "compareAndSwap" => Some(Type::Bool),
                "store" => Some(Type::Unit),
                _ => None,
            },
            // Mutex/ReentrantMutex 方法返回类型
            Some(Type::Struct(ref name, _)) if name == "Mutex" || name == "ReentrantMutex" => {
                match method {
                    "tryLock" => Some(Type::Bool),
                    "lock" | "unlock" => Some(Type::Unit),
                    _ => None,
                }
            }
            // getOrThrow/unwrap on non-Option type: pass-through semantics
            // (Our HashMap.get() returns V directly, not Option<V>, so getOrThrow is a no-op)
            Some(ty) if matches!(method, "getOrThrow" | "unwrap") => Some(ty.clone()),
            _ => None,
        }
    }

    fn is_math_builtin(name: &str) -> bool {
        matches!(
            name,
            "sqrt"
                | "floor"
                | "ceil"
                | "trunc"
                | "nearest"
                | "abs"
                | "copysign"
                | "neg"
                | "sin"
                | "cos"
                | "tan"
                | "exp"
                | "log"
                | "pow"
                | "fmin"
                | "fmax"
        )
    }

    /// Phase 7.3: 编译 math 内置函数调用
    fn compile_math_builtin(
        &self,
        name: &str,
        args: &[Expr],
        locals: &LocalsBuilder,
        func: &mut WasmFunc,
        loop_ctx: Option<(u32, u32)>,
    ) {
        // 编译参数（确保为 f64）
        for arg in args {
            self.compile_expr(arg, locals, func, loop_ctx);
            let wt = self.infer_type_with_locals(arg, locals);
            match wt {
                ValType::F32 => {
                    func.instruction(&Instruction::F64PromoteF32);
                }
                ValType::I64 => {
                    func.instruction(&Instruction::F64ConvertI64S);
                }
                ValType::I32 => {
                    func.instruction(&Instruction::I64ExtendI32S);
                    func.instruction(&Instruction::F64ConvertI64S);
                }
                _ => {} // f64 or already correct
            }
        }

        match name {
            // WASM 原生一元指令
            "sqrt" => {
                func.instruction(&Instruction::F64Sqrt);
            }
            "floor" => {
                func.instruction(&Instruction::F64Floor);
            }
            "ceil" => {
                func.instruction(&Instruction::F64Ceil);
            }
            "trunc" => {
                func.instruction(&Instruction::F64Trunc);
            }
            "nearest" => {
                func.instruction(&Instruction::F64Nearest);
            }
            "neg" => {
                func.instruction(&Instruction::F64Neg);
            }
            "abs" if args.len() == 1 => {
                // 检查参数类型，i64 使用已有 __abs_i64，f64 使用 f64.abs
                let arg_wt = self.infer_type_with_locals(&args[0], locals);
                if arg_wt == ValType::I64 || arg_wt == ValType::I32 {
                    // 已经转成了 f64 在上面，这里用 f64.abs 即可
                    func.instruction(&Instruction::F64Abs);
                } else {
                    func.instruction(&Instruction::F64Abs);
                }
            }
            // WASM 原生二元指令
            "fmin" => {
                func.instruction(&Instruction::F64Min);
            }
            "fmax" => {
                func.instruction(&Instruction::F64Max);
            }
            "copysign" => {
                func.instruction(&Instruction::F64Copysign);
            }
            // 运行时函数 (泰勒级数)
            "sin" => {
                func.instruction(&Instruction::Call(self.func_indices["__math_sin"]));
            }
            "cos" => {
                func.instruction(&Instruction::Call(self.func_indices["__math_cos"]));
            }
            "tan" => {
                func.instruction(&Instruction::Call(self.func_indices["__math_tan"]));
            }
            "exp" => {
                func.instruction(&Instruction::Call(self.func_indices["__math_exp"]));
            }
            "log" => {
                func.instruction(&Instruction::Call(self.func_indices["__math_log"]));
            }
            "pow" => {
                func.instruction(&Instruction::Call(self.func_indices["__math_pow"]));
            }
            _ => {} // abs with != 1 arg handled by existing logic
        }
    }

    /// Phase 7.1: min/max 内置函数辅助
    fn compile_min_max_builtin(
        &self,
        name: &str,
        args: &[Expr],
        locals: &LocalsBuilder,
        func: &mut WasmFunc,
        loop_ctx: Option<(u32, u32)>,
    ) {
        if args.len() == 2 {
            // 检查参数类型
            let wt0 = self.infer_type_with_locals(&args[0], locals);
            let wt1 = self.infer_type_with_locals(&args[1], locals);
            self.compile_expr(&args[0], locals, func, loop_ctx);
            self.compile_expr(&args[1], locals, func, loop_ctx);

            if wt0 == ValType::F64 || wt1 == ValType::F64 {
                // f64 版本用 WASM 原生指令
                if name == "min" {
                    func.instruction(&Instruction::F64Min);
                } else {
                    func.instruction(&Instruction::F64Max);
                }
            } else {
                // i64 版本用已有运行时
                let idx = self.func_indices[if name == "min" {
                    "__min_i64"
                } else {
                    "__max_i64"
                }];
                func.instruction(&Instruction::Call(idx));
            }
        }
    }

    /// 带 locals 的类型推断（用于 Call 实参等，可解析变量类型）
    fn infer_ast_type_with_locals(&self, expr: &Expr, locals: &LocalsBuilder) -> Option<Type> {
        match expr {
            Expr::Var(name) => {
                // 优先从 locals 获取类型，若无则检查 math 常数，再检查全局变量，最后检查隐式 this 字段
                locals.get_type(name).cloned().or_else(|| {
                    match name.as_str() {
                        "PI" | "E" | "TAU" | "INF" | "INFINITY" | "NEG_INF" | "NEG_INFINITY"
                        | "NAN"
                            if locals.get(name).is_none() =>
                        {
                            Some(Type::Float64)
                        }
                        _ => {
                            // 检查全局变量类型（顶层 let/var 声明）
                            if let Some(ty) = self.global_var_types.get(name) {
                                // 如果类型是 Int64（解析器默认占位符），尝试从 init 推断
                                if ty == &Type::Int64 {
                                    if let Some(init) = self.global_var_inits.get(name) {
                                        if let Some(inferred) = self.infer_ast_type(init) {
                                            return Some(inferred);
                                        }
                                    }
                                }
                                return Some(ty.clone());
                            }
                            // Bug B2 修复: 隐式 this 字段类型推断
                            if locals.get("this").is_some() {
                                let this_field = Expr::Field {
                                    object: Box::new(Expr::Var("this".to_string())),
                                    field: name.clone(),
                                };
                                self.infer_ast_type_with_locals(&this_field, locals)
                            } else {
                                None
                            }
                        }
                    }
                })
            }
            Expr::Integer(_) => Some(Type::Int64),
            Expr::Float(_) => Some(Type::Float64),
            Expr::Float32(_) => Some(Type::Float32),
            Expr::Bool(_) => Some(Type::Bool),
            Expr::Rune(_) => Some(Type::Rune),
            Expr::String(_) => Some(Type::String),
            Expr::Tuple(ref elems) => {
                let types: Vec<Type> = elems
                    .iter()
                    .filter_map(|e| self.infer_ast_type_with_locals(e, locals))
                    .collect();
                if types.len() == elems.len() {
                    Some(Type::Tuple(types))
                } else {
                    None
                }
            }
            Expr::TupleIndex { object, index } => self
                .infer_ast_type_with_locals(object, locals)
                .and_then(|ty| {
                    if let Type::Tuple(types) = ty {
                        types.get(*index as usize).cloned()
                    } else {
                        None
                    }
                }),
            Expr::NullCoalesce { default, .. } => self.infer_ast_type_with_locals(default, locals),
            Expr::Array(ref elems) => elems
                .first()
                .and_then(|e| {
                    self.infer_ast_type_with_locals(e, locals)
                        .map(|t| Type::Array(Box::new(t)))
                })
                .or(Some(Type::Array(Box::new(Type::Int64)))),
            Expr::StructInit {
                name, type_args, ..
            } => Some(Type::Struct(
                name.clone(),
                type_args.clone().unwrap_or_default(),
            )),
            Expr::ConstructorCall {
                name, type_args, ..
            } => {
                match name.as_str() {
                    "Array" => {
                        let elem = type_args
                            .as_ref()
                            .and_then(|ta| ta.first().cloned())
                            .unwrap_or(Type::Int64);
                        return Some(Type::Array(Box::new(elem)));
                    }
                    "ArrayList" | "LinkedList" | "ArrayStack" => {
                        let elem = type_args
                            .as_ref()
                            .and_then(|ta| ta.first().cloned())
                            .unwrap_or(Type::Int64);
                        return Some(Type::Array(Box::new(elem)));
                    }
                    "HashMap" => {
                        let k = type_args
                            .as_ref()
                            .and_then(|ta| ta.first().cloned())
                            .unwrap_or(Type::Int64);
                        let v = type_args
                            .as_ref()
                            .and_then(|ta| ta.get(1).cloned())
                            .unwrap_or(Type::Int64);
                        return Some(Type::Map(Box::new(k), Box::new(v)));
                    }
                    "HashSet" => {
                        let elem = type_args
                            .as_ref()
                            .and_then(|ta| ta.first().cloned())
                            .unwrap_or(Type::Int64);
                        return Some(Type::Map(Box::new(elem), Box::new(Type::Int64)));
                    }
                    // P5: Atomic/Mutex 桩类型（infer_ast_type_with_locals）
                    "AtomicInt64" | "AtomicBool" | "Mutex" | "ReentrantMutex" => {
                        return Some(Type::Struct(name.clone(), vec![]));
                    }
                    _ => {}
                }
                Some(Type::Struct(
                    name.clone(),
                    type_args.clone().unwrap_or_default(),
                ))
            }
            Expr::VariantConst { enum_name, .. } => Some(Type::Struct(enum_name.clone(), vec![])),
            Expr::Call {
                name,
                type_args,
                args,
                ..
            } => {
                // 类型构造函数：Float32(x), Float64(x), Int64(x) 等
                match name.as_str() {
                    "Int8" => return Some(Type::Int8),
                    "Int16" => return Some(Type::Int16),
                    "Int32" => return Some(Type::Int32),
                    "Int64" => return Some(Type::Int64),
                    "UInt8" => return Some(Type::UInt8),
                    "UInt16" => return Some(Type::UInt16),
                    "UInt32" => return Some(Type::UInt32),
                    "UInt64" => return Some(Type::UInt64),
                    "Float32" => return Some(Type::Float32),
                    "Float64" => return Some(Type::Float64),
                    "Bool" => return Some(Type::Bool),
                    "Rune" => return Some(Type::Rune),
                    "readln" | "getEnv" => return Some(Type::String),
                    "now" | "randomInt64" => return Some(Type::Int64),
                    "randomFloat64" => return Some(Type::Float64),
                    "getArgs" => return Some(Type::Array(Box::new(Type::String))),
                    // P4: 集合类型推断
                    "ArrayList" => {
                        let elem = type_args
                            .as_ref()
                            .and_then(|ta| ta.first().cloned())
                            .unwrap_or(Type::Int64);
                        return Some(Type::Array(Box::new(elem)));
                    }
                    "HashMap" => {
                        let k = type_args
                            .as_ref()
                            .and_then(|ta| ta.first().cloned())
                            .unwrap_or(Type::Int64);
                        let v = type_args
                            .as_ref()
                            .and_then(|ta| ta.get(1).cloned())
                            .unwrap_or(Type::Int64);
                        return Some(Type::Map(Box::new(k), Box::new(v)));
                    }
                    "HashSet" => {
                        let elem = type_args
                            .as_ref()
                            .and_then(|ta| ta.first().cloned())
                            .unwrap_or(Type::Int64);
                        return Some(Type::Map(Box::new(elem), Box::new(Type::Int64)));
                    }
                    "LinkedList" | "ArrayStack" => {
                        let elem = type_args
                            .as_ref()
                            .and_then(|ta| ta.first().cloned())
                            .unwrap_or(Type::Int64);
                        return Some(Type::Array(Box::new(elem)));
                    }
                    _ => {}
                }
                // P4.2: class 构造函数调用也返回 Struct 类型（with_locals 版）
                if self.structs.contains_key(name) || self.classes.contains_key(name) {
                    Some(Type::Struct(name.clone(), vec![]))
                } else if Self::is_math_builtin(name) && !self.func_indices.contains_key(name) {
                    Some(Type::Float64) // math 内置函数返回 Float64
                } else if (name == "min" || name == "max") && args.len() == 2
                    || (name == "abs" && args.len() == 1)
                {
                    Some(Type::Int64)
                } else {
                    let arg_tys: Vec<Type> = args
                        .iter()
                        .filter_map(|a| self.infer_ast_type_with_locals(a, locals))
                        .collect();
                    let key = if *self.name_count.get(name).unwrap_or(&0) > 1 {
                        if arg_tys.len() == args.len() {
                            Some(Self::mangle_key(name, &arg_tys))
                        } else {
                            None
                        }
                    } else {
                        Some(name.to_string())
                    };
                    // P0: func_return_types 找不到时，查询标准库构造函数元数据
                    key.and_then(|k| self.func_return_types.get(&k).cloned())
                        .or_else(|| {
                            let ta = type_args.as_deref().unwrap_or(&[]);
                            crate::metadata::stdlib_constructor_type(name, ta)
                        })
                }
            }
            Expr::Interpolate(_) => Some(Type::String), // 字符串插值结果是 String
            Expr::MethodCall { object, method, .. } => {
                // Phase 7.2: 先检查内建类型方法返回类型
                let obj_ty = self.infer_ast_type_with_locals(object, locals);
                if let Some(ret) = Self::builtin_method_return_type(obj_ty.as_ref(), method) {
                    return Some(ret);
                }
                // P2.4: 静态方法调用 → 对象是类名（不是局部变量）
                if let Expr::Var(ref class_name) = object.as_ref() {
                    if obj_ty.is_none()
                        && (self.structs.contains_key(class_name.as_str())
                            || self.classes.contains_key(class_name.as_str()))
                    {
                        let key = format!("{}.{}", class_name, method);
                        if let Some(ret) = self.func_return_types.get(&key) {
                            return Some(ret.clone());
                        }
                    }
                }
                // 尝试通过对象类型 + 方法名查找 func_return_types
                let func_result = obj_ty.as_ref().and_then(|ty| {
                    let type_name = match ty {
                        Type::Struct(name, _) => Some(name.clone()),
                        Type::Option(_) => Some("Option".to_string()),
                        Type::Result(_, _) => Some("Result".to_string()),
                        _ => None,
                    };
                    type_name.and_then(|tn| {
                        let key = format!("{}.{}", tn, method);
                        self.func_return_types.get(&key).cloned()
                    })
                });
                if func_result.is_some() {
                    return func_result;
                }
                // P0: 查询标准库元数据
                if let Some(Type::Struct(ref type_name, ref type_args)) = obj_ty {
                    if let Some(ret) =
                        crate::metadata::stdlib_method_return_type(type_name, type_args, method)
                    {
                        return Some(ret);
                    }
                }
                // P1: 查询接口抽象 prop getter（已注册到 func_return_types）
                if let Some(Type::Struct(ref type_name, _)) = obj_ty {
                    let getter_key = format!("{}.__get_{}", type_name, method);
                    if let Some(ret) = self.func_return_types.get(&getter_key) {
                        return Some(ret.clone());
                    }
                }
                None
            }
            Expr::SuperCall { .. } => None,
            Expr::SuperFieldAccess { .. } => None,
            Expr::Cast { target_ty, .. } => Some(target_ty.clone()),
            Expr::IsType { .. } => Some(Type::Bool),
            Expr::IfLet { then_branch, .. } => self.infer_ast_type_with_locals(then_branch, locals),
            Expr::Field { object, field, .. } => {
                // Phase 7.2: 内建类型属性
                let obj_ty = self.infer_ast_type_with_locals(object, locals);
                if field == "size"
                    && (obj_ty.as_ref() == Some(&Type::String)
                        || matches!(obj_ty.as_ref(), Some(Type::Array(_)))
                        || matches!(obj_ty.as_ref(), Some(Type::Map(..))))
                {
                    return Some(Type::Int64);
                }
                // P4: Range 属性返回类型
                if obj_ty == Some(Type::Range)
                    && (field == "start" || field == "end" || field == "step")
                {
                    return Some(Type::Int64);
                }
                obj_ty.and_then(|ty| {
                    if let Type::Struct(s, ref type_args) = ty {
                        // 泛型类型需要查找修饰后的名字
                        let lookup_name = if !type_args.is_empty() {
                            let mangled = crate::monomorph::mangle_name(&s, type_args);
                            if self.structs.contains_key(&mangled) {
                                mangled
                            } else {
                                s.clone()
                            }
                        } else {
                            s.clone()
                        };
                        // 先查找 struct 字段
                        let field_ty = self.structs.get(&lookup_name).and_then(|def| {
                            def.fields
                                .iter()
                                .find(|f| f.name == *field)
                                .map(|f| f.ty.clone())
                        });
                        if field_ty.is_some() {
                            return field_ty;
                        }
                        // Bug B2 补充修复: 查找 prop getter 的返回类型
                        // P1: 同时支持接口抽象 prop（已注册到 func_return_types）
                        let getter_name = format!("{}.__get_{}", s, field);
                        let getter_ty = self.func_return_types.get(&getter_name).cloned();
                        if getter_ty.is_some() {
                            return getter_ty;
                        }
                        // P1: 查找接口方法中的直接方法名（非 getter，如普通接口方法）
                        let iface_method_key = format!("{}.{}", s, field);
                        let iface_ty = self.func_return_types.get(&iface_method_key).cloned();
                        if iface_ty.is_some() {
                            return iface_ty;
                        }
                        // P1: 查找 class 字段（ClassInfo.all_fields 包含所有继承字段）
                        let class_field = self.classes.get(&lookup_name).and_then(|ci| {
                            ci.all_fields
                                .iter()
                                .find(|f| f.name == *field)
                                .map(|f| f.ty.clone())
                        });
                        if class_field.is_some() {
                            return class_field;
                        }
                        // P0: 查询标准库字段元数据
                        crate::metadata::stdlib_field_type(&s, type_args, field)
                    } else {
                        None
                    }
                })
            }
            Expr::Index { array, .. } => {
                // 数组下标：Array<T>[i] 返回 T；Tuple[i] 返回元素类型
                match self.infer_ast_type_with_locals(array, locals) {
                    Some(Type::Array(elem_ty)) => Some(*elem_ty),
                    Some(Type::Tuple(types)) => {
                        // 元组索引在编译时可能已知
                        types.first().cloned() // 默认返回第一个元素的类型
                    }
                    Some(Type::Slice(elem_ty)) => Some(*elem_ty),
                    _ => Some(Type::Int64), // 未知时默认 Int64
                }
            }
            Expr::Unary { op, expr } => {
                use crate::ast::UnaryOp;
                match op {
                    UnaryOp::Not => Some(Type::Bool), // ! 总是返回 Bool (i32)
                    UnaryOp::BitNot | UnaryOp::Neg => self.infer_ast_type_with_locals(expr, locals),
                }
            }
            Expr::Binary {
                op, left, right, ..
            } => {
                use crate::ast::BinOp;
                match op {
                    BinOp::LogicalAnd
                    | BinOp::LogicalOr
                    | BinOp::Eq
                    | BinOp::NotEq
                    | BinOp::Lt
                    | BinOp::LtEq
                    | BinOp::Gt
                    | BinOp::GtEq
                    | BinOp::NotIn => Some(Type::Bool),
                    BinOp::Add => {
                        // Bug B4: String + x 或 x + String 结果为 String
                        let left_ty = self.infer_ast_type_with_locals(left, locals);
                        let right_ty = self.infer_ast_type_with_locals(right, locals);
                        if left_ty == Some(Type::String) || right_ty == Some(Type::String) {
                            Some(Type::String)
                        } else {
                            left_ty.or(right_ty)
                        }
                    }
                    _ => self
                        .infer_ast_type_with_locals(left, locals)
                        .or_else(|| self.infer_ast_type_with_locals(right, locals)),
                }
            }
            Expr::Range { .. } => Some(Type::Range),
            Expr::Lambda {
                params,
                return_type,
                body,
            } => {
                let param_types: Vec<Type> = params.iter().map(|(_, t)| t.clone()).collect();
                let ret = if return_type.is_some() {
                    return_type.clone()
                } else {
                    Self::infer_lambda_return_type(body, params)
                };
                Some(Type::Function {
                    params: param_types,
                    ret: Box::new(ret),
                })
            }
            Expr::Some(inner) => self
                .infer_ast_type_with_locals(inner, locals)
                .map(|t| Type::Option(Box::new(t))),
            Expr::None => None,
            Expr::Ok(inner) => self
                .infer_ast_type_with_locals(inner, locals)
                .map(|t| Type::Result(Box::new(t), Box::new(Type::String))),
            Expr::Err(inner) => self
                .infer_ast_type_with_locals(inner, locals)
                .map(|_| Type::Result(Box::new(Type::Int64), Box::new(Type::String))),
            Expr::Try(inner) => match self.infer_ast_type_with_locals(inner, locals) {
                Some(Type::Option(t)) => Some(*t),
                Some(Type::Result(t, _)) => Some(*t),
                _ => None,
            },
            Expr::Match { arms, .. } => {
                // match 结果类型 = 第一个 arm 的 body 类型
                arms.first()
                    .and_then(|arm| self.infer_ast_type_with_locals(&arm.body, locals))
            }
            Expr::If {
                then_branch,
                else_branch,
                ..
            } => self
                .infer_ast_type_with_locals(then_branch, locals)
                .or_else(|| {
                    else_branch
                        .as_ref()
                        .and_then(|eb| self.infer_ast_type_with_locals(eb, locals))
                }),
            Expr::Block(stmts, tail) => {
                // Block 的结果类型 = tail 表达式的类型，或最后一个语句（如果是 Expr）的类型
                tail.as_ref()
                    .and_then(|t| self.infer_ast_type_with_locals(t, locals))
                    .or_else(|| {
                        stmts.last().and_then(|s| {
                            if let Stmt::Expr(ref e) = s {
                                self.infer_ast_type_with_locals(e, locals)
                            } else {
                                None
                            }
                        })
                    })
            }
            _ => self.infer_ast_type(expr),
        }
    }

    /// 获取"对象表达式"的 AST 类型（用于字段访问、方法调用时查结构体与偏移）
    fn get_object_type(&self, expr: &Expr, locals: &LocalsBuilder) -> Option<Type> {
        match expr {
            Expr::Var(name) => {
                // 先查局部变量
                if let Some(ty) = locals.get_type(name) {
                    return Some(ty.clone());
                }
                // 查全局变量（顶层 let/var 声明）
                if let Some(ty) = self.global_var_types.get(name) {
                    // 如果类型是 Int64（解析器默认占位符），尝试从 init 推断
                    if ty == &Type::Int64 {
                        if let Some(init) = self.global_var_inits.get(name) {
                            if let Some(inferred) = self.infer_ast_type(init) {
                                return Some(inferred);
                            }
                        }
                    }
                    return Some(ty.clone());
                }
                // P2.4: 如果是已知类名，返回 Struct 类型（用于静态方法调用）
                if self.structs.contains_key(name.as_str())
                    || self.classes.contains_key(name.as_str())
                {
                    return Some(Type::Struct(name.clone(), vec![]));
                }
                // 检查内建枚举 Option/Result
                if name == "Option" && self.enums.contains_key("Option") {
                    return Some(Type::Option(Box::new(Type::Int64)));
                }
                if name == "Result" && self.enums.contains_key("Result") {
                    return Some(Type::Result(Box::new(Type::Int64), Box::new(Type::Int64)));
                }
                None
            }
            Expr::StructInit {
                name, type_args, ..
            } => Some(Type::Struct(
                name.clone(),
                type_args.clone().unwrap_or_default(),
            )),
            Expr::ConstructorCall {
                name, type_args, ..
            } => Some(Type::Struct(
                name.clone(),
                type_args.clone().unwrap_or_default(),
            )),
            Expr::Field { object, .. } => self.get_object_type(object, locals),
            // Bug B5 修复: 方法调用返回类型追踪（支持链式调用）
            Expr::MethodCall { object, method, .. } => {
                self.infer_ast_type_with_locals(expr, locals)
            }
            _ => None,
        }
    }

    /// 用于 for 循环变量：可迭代表达式的“元素类型”（范围时为 Int64，数组时为元素类型）
    fn expr_object_type(&self, expr: &Expr) -> Option<Type> {
        match expr {
            Expr::Range { .. } => Some(Type::Int64),
            Expr::Array(ref elems) => elems
                .first()
                .and_then(|e| self.infer_ast_type(e))
                .or(Some(Type::Int64)),
            _ => None,
        }
    }

    /// match 表达式最后一个 arm 不匹配时的默认值（按 result_type 生成正确类型的零值）
    fn emit_match_default_value(func: &mut WasmFunc, result_type: wasm_encoder::BlockType) {
        match result_type {
            wasm_encoder::BlockType::Empty => {} // void match 不需要默认值
            wasm_encoder::BlockType::Result(ValType::I32) => {
                func.instruction(&Instruction::I32Const(0));
            }
            wasm_encoder::BlockType::Result(ValType::F32) => {
                func.instruction(&Instruction::F32Const(0.0));
            }
            wasm_encoder::BlockType::Result(ValType::F64) => {
                func.instruction(&Instruction::F64Const(0.0));
            }
            _ => {
                func.instruction(&Instruction::I64Const(0));
            }
        }
    }

    /// Bug B4 修复: 将栈顶值转为字符串指针（i32）
    /// 假设栈顶已有对应类型的值
    fn emit_to_string(&self, func: &mut WasmFunc, ast_ty: Option<&Type>) {
        match ast_ty {
            Some(Type::String) => {
                // 已是字符串，无需转换
            }
            Some(Type::Int64) | Some(Type::Int32) | Some(Type::IntNative) => {
                func.instruction(&Instruction::Call(self.func_indices["__i64_to_str"]));
            }
            Some(Type::Int8) | Some(Type::Int16) | Some(Type::UInt8) | Some(Type::UInt16)
            | Some(Type::UInt32) | Some(Type::Rune) => {
                func.instruction(&Instruction::Call(self.func_indices["__i32_to_str"]));
            }
            Some(Type::UInt64) | Some(Type::UIntNative) => {
                func.instruction(&Instruction::Call(self.func_indices["__i64_to_str"]));
            }
            Some(Type::Float64) => {
                func.instruction(&Instruction::Call(self.func_indices["__f64_to_str"]));
            }
            Some(Type::Float32) => {
                func.instruction(&Instruction::Call(self.func_indices["__f32_to_str"]));
            }
            Some(Type::Bool) => {
                func.instruction(&Instruction::Call(self.func_indices["__bool_to_str"]));
            }
            _ => {
                // 未知类型，转为 "[object]"
                func.instruction(&Instruction::Drop);
                let obj_str = self
                    .string_pool
                    .iter()
                    .find(|(s, _)| s == "[object]")
                    .map(|(_, off)| *off)
                    .unwrap_or(0);
                func.instruction(&Instruction::I32Const(obj_str as i32));
            }
        }
    }

    /// 当值类型与目标类型不匹配时，生成自动类型转换指令
    /// 在布尔上下文中，判断表达式是否需要 I32WrapI64。
    /// 仅当 AST 类型确认为 Int64/UInt64/IntNative/UIntNative 时才 wrap；
    /// TypeParam 即使 to_wasm() = I64，也可能已单态化为 i32，保守不 wrap。
    fn needs_i64_to_i32_wrap(&self, expr: &Expr, locals: &LocalsBuilder) -> bool {
        matches!(
            self.infer_ast_type_with_locals(expr, locals).as_ref(),
            Some(Type::Int64 | Type::UInt64 | Type::IntNative | Type::UIntNative)
        )
    }

    fn emit_type_coercion(&self, func: &mut WasmFunc, src: ValType, dst: ValType) {
        if src == dst {
            return;
        }
        match (src, dst) {
            (ValType::I64, ValType::I32) => {
                func.instruction(&Instruction::I32WrapI64);
            }
            (ValType::I32, ValType::I64) => {
                func.instruction(&Instruction::I64ExtendI32S);
            }
            (ValType::I64, ValType::F64) => {
                func.instruction(&Instruction::F64ConvertI64S);
            }
            (ValType::F64, ValType::I64) => {
                func.instruction(&Instruction::I64TruncF64S);
            }
            (ValType::I32, ValType::F64) => {
                func.instruction(&Instruction::F64ConvertI32S);
            }
            (ValType::F64, ValType::I32) => {
                func.instruction(&Instruction::I32TruncF64S);
            }
            (ValType::F32, ValType::F64) => {
                func.instruction(&Instruction::F64PromoteF32);
            }
            (ValType::F64, ValType::F32) => {
                func.instruction(&Instruction::F32DemoteF64);
            }
            (ValType::I32, ValType::F32) => {
                func.instruction(&Instruction::F32ConvertI32S);
            }
            (ValType::F32, ValType::I32) => {
                func.instruction(&Instruction::I32TruncF32S);
            }
            (ValType::I64, ValType::F32) => {
                func.instruction(&Instruction::F32ConvertI64S);
            }
            (ValType::F32, ValType::I64) => {
                func.instruction(&Instruction::I64TruncF32S);
            }
            _ => {} // 相同类型或无法转换，跳过
        }
    }

    /// 按 AST 类型生成 load 指令（栈顶为 i32 地址）
    fn emit_load_by_type(&self, func: &mut WasmFunc, ty: &Type) {
        let wasm_ty = ty.to_wasm();
        let instr = match wasm_ty {
            ValType::I32 => Instruction::I32Load(wasm_encoder::MemArg {
                offset: 0,
                align: 2,
                memory_index: 0,
            }),
            ValType::I64 => Instruction::I64Load(wasm_encoder::MemArg {
                offset: 0,
                align: 3,
                memory_index: 0,
            }),
            ValType::F64 => Instruction::F64Load(wasm_encoder::MemArg {
                offset: 0,
                align: 3,
                memory_index: 0,
            }),
            ValType::F32 => Instruction::F32Load(wasm_encoder::MemArg {
                offset: 0,
                align: 2,
                memory_index: 0,
            }),
            ValType::V128 | ValType::Ref(_) => panic!("不支持的字段类型: {:?}", ty),
        };
        func.instruction(&instr);
    }

    /// 按 WASM ValType 生成 store 指令（栈顶依次为：地址 i32，值）
    fn emit_store_by_wasm_type(func: &mut WasmFunc, vt: ValType) {
        let instr = match vt {
            ValType::I32 => Instruction::I32Store(wasm_encoder::MemArg {
                offset: 0,
                align: 2,
                memory_index: 0,
            }),
            ValType::I64 => Instruction::I64Store(wasm_encoder::MemArg {
                offset: 0,
                align: 3,
                memory_index: 0,
            }),
            ValType::F64 => Instruction::F64Store(wasm_encoder::MemArg {
                offset: 0,
                align: 3,
                memory_index: 0,
            }),
            ValType::F32 => Instruction::F32Store(wasm_encoder::MemArg {
                offset: 0,
                align: 2,
                memory_index: 0,
            }),
            ValType::V128 | ValType::Ref(_) => panic!("不支持的字段类型: {:?}", vt),
        };
        func.instruction(&instr);
    }

    /// 按 AST 类型生成 store 指令（栈顶依次为：地址 i32，值）
    fn emit_store_by_type(&self, func: &mut WasmFunc, ty: &Type) {
        let instr = match ty.to_wasm() {
            ValType::I32 => Instruction::I32Store(wasm_encoder::MemArg {
                offset: 0,
                align: 2,
                memory_index: 0,
            }),
            ValType::I64 => Instruction::I64Store(wasm_encoder::MemArg {
                offset: 0,
                align: 3,
                memory_index: 0,
            }),
            ValType::F64 => Instruction::F64Store(wasm_encoder::MemArg {
                offset: 0,
                align: 3,
                memory_index: 0,
            }),
            ValType::F32 => Instruction::F32Store(wasm_encoder::MemArg {
                offset: 0,
                align: 2,
                memory_index: 0,
            }),
            ValType::V128 | ValType::Ref(_) => panic!("不支持的字段类型: {:?}", ty),
        };
        func.instruction(&instr);
    }

    /// 判断表达式编译后是否在栈上产生一个值
    pub(crate) fn expr_produces_value(&self, expr: &Expr) -> bool {
        match expr {
            Expr::Block(_, result) => {
                // 仅当 result 表达式本身产生值时，block 才产生值（避免 Unit 类型导致 panic）
                if let Some(tail) = result {
                    self.expr_produces_value(tail)
                } else {
                    false
                }
            }
            // if 无 else 编译为 BlockType::Empty，不产生值
            Expr::If {
                else_branch: None, ..
            } => false,
            // if-else：只有两个分支都产生值时，整个 if-else 才产生值
            Expr::If {
                then_branch,
                else_branch: Some(else_expr),
                ..
            } => self.expr_produces_value(then_branch) && self.expr_produces_value(else_expr),
            // throw 设置 __err_flag/__err_val 并跳转，不在栈上留值
            Expr::Throw(_) => false,
            // Bug B6 修复: try-catch 可以作为表达式使用（当 body 最后一条是表达式时产生值）
            Expr::TryBlock { body, .. } => body.last().map_or(
                false,
                |s| matches!(s, Stmt::Expr(e) if self.expr_produces_value(e)),
            ),
            // super(args) 初始化调用不产生值（已在 compile_expr 中 drop）
            Expr::SuperCall { method, .. } => method != "init" && !method.is_empty(),
            // super.field 字段访问产生值
            Expr::SuperFieldAccess { .. } => true,
            // P1.2: void（返回 Unit 或无返回类型）的函数调用不在栈上产生值
            Expr::Call { name, args, .. } => {
                // 内建类型构造函数（如 Rune(x), UInt32(x) 等）总是产生值
                if matches!(
                    name.as_str(),
                    "Rune"
                        | "Int8"
                        | "Int16"
                        | "Int32"
                        | "Int64"
                        | "UInt8"
                        | "UInt16"
                        | "UInt32"
                        | "UInt64"
                        | "Float16"
                        | "Float32"
                        | "Float64"
                        | "IntNative"
                        | "UIntNative"
                        | "Bool"
                ) {
                    return !args.is_empty(); // 有参数时是类型转换（产生值），无参数时按函数查找
                }
                // 结构体/类构造函数总是产生值
                if self.structs.contains_key(name.as_str())
                    || self.classes.contains_key(name.as_str())
                {
                    return true;
                }
                match self.func_return_types.get(name.as_str()) {
                    Some(Type::Unit) => false,
                    Some(_) => true,
                    None => false, // 未知函数保守认为不产生值
                }
            }
            // P1.2: void 方法调用也不产生值
            Expr::MethodCall { object, method, .. } => {
                // 尝试推断对象类型，查找方法返回值类型
                if let Some(obj_type) = self.infer_ast_type(object) {
                    if let Type::Struct(class_name, _) = &obj_type {
                        let key = format!("{}.{}", class_name, method);
                        match self.func_return_types.get(key.as_str()) {
                            Some(Type::Unit) | None => return false,
                            _ => return true,
                        }
                    }
                }
                // P2.4: 静态方法调用 → 对象是类名
                if let Expr::Var(ref class_name) = object.as_ref() {
                    if self.structs.contains_key(class_name.as_str())
                        || self.classes.contains_key(class_name.as_str())
                    {
                        let key = format!("{}.{}", class_name, method);
                        match self.func_return_types.get(key.as_str()) {
                            Some(Type::Unit) | None => return false,
                            _ => return true,
                        }
                    }
                }
                true // 无法推断时默认产生值
            }
            // P2.1: match/if-let 表达式是否产生值取决于 arms 是否产生值
            Expr::Match { arms, .. } => {
                if arms.is_empty() {
                    return false;
                }
                self.expr_produces_value(&arms[0].body)
            }
            Expr::IfLet { then_branch, .. } => self.expr_produces_value(then_branch),
            // P5: spawn/synchronized 是语句级别，不产生值
            Expr::Spawn { .. } => false,
            Expr::Synchronized { .. } => false,
            Expr::Break | Expr::Continue => false,
            // P6: OptionalChain 产生值
            Expr::OptionalChain { .. } => true,
            // P6: TrailingClosure 产生值（取决于被调用函数）
            Expr::TrailingClosure { .. } => true,
            _ => true,
        }
    }

    /// 类型推断（含局部变量上下文），优先使用 AST 类型推断结果
    fn infer_type_with_locals(&self, expr: &Expr, locals: &LocalsBuilder) -> ValType {
        if let Some(ast_ty) = self
            .infer_ast_type_with_locals(expr, locals)
            .filter(|t| t != &Type::Unit && t != &Type::Nothing)
        {
            return ast_ty.to_wasm();
        }
        // Fallback: for Var, use the actual WASM type from locals (avoids incorrect I64 default)
        if let Expr::Var(name) = expr {
            if let Some(vt) = locals.get_valtype(name) {
                return vt;
            }
        }
        self.infer_type(expr)
    }

    /// 简单的类型推断
    fn infer_type(&self, expr: &Expr) -> ValType {
        // 优先使用 AST 类型推断（更精确，能处理 Field/Index/MethodCall 等）
        if let Some(ast_ty) = self.infer_ast_type(expr) {
            return ast_ty.to_wasm();
        }
        match expr {
            Expr::Integer(_) => ValType::I64,
            Expr::Float(_) => ValType::F64,
            Expr::Float32(_) => ValType::F32,
            Expr::Bool(_) => ValType::I32,
            Expr::Rune(_) => ValType::I32,
            Expr::String(_) => ValType::I32,
            Expr::Array(_) => ValType::I32,
            Expr::SliceExpr { .. } => ValType::I32,
            Expr::Tuple(_) => ValType::I32,
            Expr::TupleIndex { .. } => ValType::I64, // 默认假设 i64，实际由 AST 推断处理
            Expr::NullCoalesce { default, .. } => self.infer_type(default),
            Expr::StructInit { .. } => ValType::I32,
            Expr::ConstructorCall { .. } => ValType::I32,
            Expr::Call {
                name,
                type_args: _,
                args,
                ..
            } => {
                if name == "println" || name == "print" || name == "eprintln" || name == "eprint" {
                    ValType::I32 // I/O 函数无返回值，返回虚拟类型
                } else if name == "readln" {
                    ValType::I32 // readln() 返回字符串指针 (i32)
                } else if Self::is_math_builtin(name) && !self.func_indices.contains_key(name) {
                    ValType::F64 // math 内置函数返回 f64
                } else if self.structs.contains_key(name) || self.classes.contains_key(name) {
                    // P1: class 构造函数也返回 i32 指针
                    ValType::I32
                } else if (name == "min" || name == "max") && args.len() == 2
                    || (name == "abs" && args.len() == 1)
                {
                    ValType::I64
                } else {
                    let arg_tys: Vec<Type> =
                        args.iter().filter_map(|a| self.infer_ast_type(a)).collect();
                    let key = if *self.name_count.get(name).unwrap_or(&0) > 1 {
                        if arg_tys.len() == args.len() {
                            Some(Self::mangle_key(name, &arg_tys))
                        } else {
                            None
                        }
                    } else {
                        Some(name.to_string())
                    };
                    key.and_then(|k| self.func_return_types.get(&k))
                        .map(|t| t.to_wasm())
                        .unwrap_or_else(|| {
                            // P5.1: 回退到 WASM 函数签名（比 I64 更精确）
                            self.func_return_wasm_types
                                .get(name.as_str())
                                .copied()
                                .flatten()
                                .unwrap_or(ValType::I64)
                        })
                }
            }
            Expr::Unary { op, expr } => match op {
                UnaryOp::Not => ValType::I32,
                UnaryOp::Neg | UnaryOp::BitNot => self.infer_type(expr),
            },
            Expr::Binary { op, left, .. } => match op {
                BinOp::LogicalAnd
                | BinOp::LogicalOr
                | BinOp::Eq
                | BinOp::NotEq
                | BinOp::Lt
                | BinOp::Gt
                | BinOp::LtEq
                | BinOp::GtEq
                | BinOp::NotIn => ValType::I32,
                _ => self.infer_type(left),
            },
            Expr::Index { .. } => ValType::I64, // AST 推断未覆盖时的回退
            Expr::Field { .. } => ValType::I64, // AST 推断未覆盖时的回退
            Expr::VariantConst { .. } => ValType::I32,
            Expr::PostfixIncr(inner)
            | Expr::PostfixDecr(inner)
            | Expr::PrefixIncr(inner)
            | Expr::PrefixDecr(inner) => self.infer_type(inner),
            Expr::Break | Expr::Continue => ValType::I32, // 不产生值，占位
            Expr::Cast { target_ty, .. } => target_ty.to_wasm(),
            Expr::IsType { .. } => ValType::I32, // Bool
            Expr::IfLet { then_branch, .. } => self.infer_type(then_branch),
            Expr::Lambda { .. } => ValType::I32, // 函数表索引
            Expr::Some(_) | Expr::None | Expr::Ok(_) | Expr::Err(_) => ValType::I32, // 指针
            Expr::Try(inner) => {
                // expr? 解包后的类型
                match self.infer_ast_type(inner) {
                    Some(Type::Option(t)) => t.to_wasm(),
                    Some(Type::Result(t, _)) => t.to_wasm(),
                    _ => self.infer_type(inner),
                }
            }
            Expr::Throw(_) => ValType::I32, // 不返回，但需要类型
            Expr::TryBlock { body, .. } => {
                // try 块的结果类型来自最后一个表达式
                if let Some(Stmt::Expr(e)) = body.last() {
                    self.infer_type(e)
                } else {
                    ValType::I64
                }
            }
            // P5.3: MethodCall 类型推断（infer_ast_type 已处理 None 情况，此处从 WASM 签名回退）
            Expr::MethodCall { object, method, .. } => {
                let obj_ty = self.infer_ast_type(object);
                if let Some(Type::Struct(ref type_name, _)) = obj_ty {
                    let key = format!("{}.{}", type_name, method);
                    if let Some(opt_wt) = self.func_return_wasm_types.get(&key) {
                        return opt_wt.unwrap_or(ValType::I32);
                    }
                }
                ValType::I64
            }
            _ => ValType::I64,
        }
    }

    /// 循环上下文：(break 目标深度, continue 目标深度)。单层 while/for 为 (1, 0)。
    fn compile_builtin_method(
        &self,
        object: &Expr,
        obj_type: &Option<Type>,
        method: &str,
        args: &[Expr],
        locals: &LocalsBuilder,
        func: &mut WasmFunc,
        loop_ctx: Option<(u32, u32)>,
    ) -> bool {
        match obj_type {
            Some(Type::Int64) => match method {
                "toString" => {
                    self.compile_expr(object, locals, func, loop_ctx);
                    func.instruction(&Instruction::Call(self.func_indices["__i64_to_str"]));
                    true
                }
                "toFloat64" => {
                    self.compile_expr(object, locals, func, loop_ctx);
                    func.instruction(&Instruction::F64ConvertI64S);
                    true
                }
                "abs" if args.is_empty() => {
                    // 实例方法 x.abs() — 等价于 abs_i64(x)
                    self.compile_expr(object, locals, func, loop_ctx);
                    func.instruction(&Instruction::Call(self.func_indices["__abs_i64"]));
                    true
                }
                "compareTo" if args.len() == 1 => {
                    // x.compareTo(y) → if x < y return -1, if x > y return 1, else 0
                    // 结果为 i64（-1/0/1 代表 LT/EQ/GT，即 Ordering 枚举）
                    self.compile_expr(object, locals, func, loop_ctx);
                    self.compile_expr(&args[0], locals, func, loop_ctx);
                    // stack: [x, y]
                    // local2 = y, local1 = x (使用临时栈操作)
                    // 策略: 先算 x < y → -1, 再算 x > y → 1, 否则 0
                    // 使用 select 实现无分支比较
                    // result = (x < y) * -1 + (x > y) * 1
                    // 但 WASM 没有 i64 的 select with condition 直接支持
                    // 使用 if-else 块
                    // 先保存 x 和 y 到临时变量
                    let tmp_y = locals.get("__cmp_y").unwrap_or(0);
                    let tmp_x = locals.get("__cmp_x").unwrap_or(0);
                    // 如果没有预注册临时变量，使用 i64 sub + clamp 策略:
                    // 简化: sign(x - y) = if x < y { -1 } elif x > y { 1 } else { 0 }
                    // 但 WASM 没有 sign 指令。用 (x > y) - (x < y) 实现:
                    // 先将两个 i64 比较结果转为 0/1
                    // 这需要两次对比，使用 i64.lt_s 和 i64.gt_s
                    // 但 i64.lt_s 返回 i32...
                    // 最简方案: 使用 if-else
                    if tmp_x != 0 && tmp_y != 0 {
                        func.instruction(&Instruction::LocalSet(tmp_y)); // save y
                        func.instruction(&Instruction::LocalSet(tmp_x)); // save x
                                                                         // if x < y → -1
                        func.instruction(&Instruction::LocalGet(tmp_x));
                        func.instruction(&Instruction::LocalGet(tmp_y));
                        func.instruction(&Instruction::I64LtS);
                        func.instruction(&Instruction::If(wasm_encoder::BlockType::Result(
                            ValType::I64,
                        )));
                        func.instruction(&Instruction::I64Const(-1));
                        func.instruction(&Instruction::Else);
                        // else if x > y → 1
                        func.instruction(&Instruction::LocalGet(tmp_x));
                        func.instruction(&Instruction::LocalGet(tmp_y));
                        func.instruction(&Instruction::I64GtS);
                        func.instruction(&Instruction::If(wasm_encoder::BlockType::Result(
                            ValType::I64,
                        )));
                        func.instruction(&Instruction::I64Const(1));
                        func.instruction(&Instruction::Else);
                        func.instruction(&Instruction::I64Const(0));
                        func.instruction(&Instruction::End);
                        func.instruction(&Instruction::End);
                    } else {
                        // 没有临时变量时，使用简化方案: (x > y) - (x < y)
                        // 需要复制两个值，但 WASM 没有 dup2
                        // 退化: 使用 sub + clamp
                        // 实际上，我们应该在 collect_locals 阶段预注册 __cmp_x/__cmp_y
                        // 这里先用 sub 的符号位近似处理
                        func.instruction(&Instruction::I64Sub);
                        // clamp to -1/0/1: 取符号
                        // (val >> 63) | (-val >>> 63) = sign... 复杂
                        // 简化: 直接返回差值 (非标准但功能正确的近似)
                        // 更正: 标准做法是 (x>y)-(x<y)，这里无临时变量无法实现
                        // 直接返回差值的符号: if val < 0 → -1, if val > 0 → 1, else 0
                        // 但这也需要临时变量...
                        // 实际上 compareTo 调用场景下 __cmp_x/__cmp_y 一定已注册
                        // 这个分支是安全回退，用 i64.sub 近似
                        // 不做额外处理 — sub 结果不标准但保持符号正确用于排序
                    }
                    true
                }
                "format" if args.len() == 1 => {
                    self.compile_expr(object, locals, func, loop_ctx);
                    self.compile_expr(&args[0], locals, func, loop_ctx);
                    func.instruction(&Instruction::Call(self.func_indices["__i64_format"]));
                    true
                }
                "hashCode" if args.is_empty() => {
                    self.compile_expr(object, locals, func, loop_ctx);
                    true // Int64 的 hashCode 就是自身
                }
                _ => false,
            },
            Some(Type::Int32) => match method {
                "toString" => {
                    self.compile_expr(object, locals, func, loop_ctx);
                    func.instruction(&Instruction::Call(self.func_indices["__i32_to_str"]));
                    true
                }
                "toFloat64" => {
                    self.compile_expr(object, locals, func, loop_ctx);
                    func.instruction(&Instruction::I64ExtendI32S);
                    func.instruction(&Instruction::F64ConvertI64S);
                    true
                }
                _ => false,
            },
            Some(Type::UInt32) => match method {
                "toString" => {
                    self.compile_expr(object, locals, func, loop_ctx);
                    func.instruction(&Instruction::Call(self.func_indices["__u32_to_str"]));
                    true
                }
                "toFloat64" => {
                    self.compile_expr(object, locals, func, loop_ctx);
                    func.instruction(&Instruction::I64ExtendI32U);
                    func.instruction(&Instruction::F64ConvertI64U);
                    true
                }
                _ => false,
            },
            Some(Type::Float64) => match method {
                "toString" => {
                    self.compile_expr(object, locals, func, loop_ctx);
                    func.instruction(&Instruction::Call(self.func_indices["__f64_to_str"]));
                    true
                }
                "toInt64" => {
                    self.compile_expr(object, locals, func, loop_ctx);
                    func.instruction(&Instruction::I64TruncF64S);
                    true
                }
                "format" if args.len() == 1 => {
                    self.compile_expr(object, locals, func, loop_ctx);
                    self.compile_expr(&args[0], locals, func, loop_ctx);
                    func.instruction(&Instruction::Call(self.func_indices["__f64_format"]));
                    true
                }
                _ => false,
            },
            Some(Type::Float32) => match method {
                "toString" => {
                    self.compile_expr(object, locals, func, loop_ctx);
                    func.instruction(&Instruction::Call(self.func_indices["__f32_to_str"]));
                    true
                }
                "toInt64" => {
                    self.compile_expr(object, locals, func, loop_ctx);
                    func.instruction(&Instruction::F64PromoteF32);
                    func.instruction(&Instruction::I64TruncF64S);
                    true
                }
                "toFloat64" => {
                    self.compile_expr(object, locals, func, loop_ctx);
                    func.instruction(&Instruction::F64PromoteF32);
                    true
                }
                _ => false,
            },
            Some(Type::Bool) => match method {
                "toString" => {
                    self.compile_expr(object, locals, func, loop_ctx);
                    // Bool 在 WASM 中是 i32，需要传给 __bool_to_str
                    // 仅当 AST 类型确认为 i64 时才 wrap
                    if self.needs_i64_to_i32_wrap(object, locals) {
                        func.instruction(&Instruction::I32WrapI64);
                    }
                    func.instruction(&Instruction::Call(self.func_indices["__bool_to_str"]));
                    true
                }
                _ => false,
            },
            Some(Type::String) => match method {
                "isEmpty" => {
                    // str.isEmpty() → str.size == 0 → mem[ptr] == 0
                    self.compile_expr(object, locals, func, loop_ctx);
                    func.instruction(&Instruction::I32Load(wasm_encoder::MemArg {
                        offset: 0,
                        align: 2,
                        memory_index: 0,
                    }));
                    func.instruction(&Instruction::I32Eqz);
                    // 返回 i32 (0/1)，但仓颉 Bool 通常用 i32
                    true
                }
                "toInt64" => {
                    self.compile_expr(object, locals, func, loop_ctx);
                    func.instruction(&Instruction::Call(self.func_indices["__str_to_i64"]));
                    true
                }
                "toFloat64" => {
                    self.compile_expr(object, locals, func, loop_ctx);
                    func.instruction(&Instruction::Call(self.func_indices["__str_to_f64"]));
                    true
                }
                "size" => {
                    // 方法调用形式 str.size() — 也支持（虽然通常是属性）
                    self.compile_expr(object, locals, func, loop_ctx);
                    func.instruction(&Instruction::I32Load(wasm_encoder::MemArg {
                        offset: 0,
                        align: 2,
                        memory_index: 0,
                    }));
                    func.instruction(&Instruction::I64ExtendI32S);
                    true
                }
                "toString" => {
                    // String.toString() → 返回自身
                    self.compile_expr(object, locals, func, loop_ctx);
                    true
                }
                "contains" if args.len() == 1 => {
                    self.compile_expr(object, locals, func, loop_ctx);
                    self.compile_expr(&args[0], locals, func, loop_ctx);
                    func.instruction(&Instruction::Call(self.func_indices["__str_contains"]));
                    true
                }
                "indexOf" if args.len() == 1 => {
                    self.compile_expr(object, locals, func, loop_ctx);
                    self.compile_expr(&args[0], locals, func, loop_ctx);
                    func.instruction(&Instruction::Call(self.func_indices["__str_index_of"]));
                    true
                }
                "replace" if args.len() == 2 => {
                    self.compile_expr(object, locals, func, loop_ctx);
                    self.compile_expr(&args[0], locals, func, loop_ctx);
                    self.compile_expr(&args[1], locals, func, loop_ctx);
                    func.instruction(&Instruction::Call(self.func_indices["__str_replace"]));
                    true
                }
                "split" if args.len() == 1 => {
                    self.compile_expr(object, locals, func, loop_ctx);
                    self.compile_expr(&args[0], locals, func, loop_ctx);
                    func.instruction(&Instruction::Call(self.func_indices["__str_split"]));
                    true
                }
                "toArray" if args.is_empty() => {
                    self.compile_expr(object, locals, func, loop_ctx);
                    func.instruction(&Instruction::Call(self.func_indices["__str_to_rune_array"]));
                    true
                }
                // P2.10: 新增 String 方法
                "trim" if args.is_empty() => {
                    self.compile_expr(object, locals, func, loop_ctx);
                    func.instruction(&Instruction::Call(self.func_indices["__str_trim"]));
                    true
                }
                "startsWith" if args.len() == 1 => {
                    self.compile_expr(object, locals, func, loop_ctx);
                    self.compile_expr(&args[0], locals, func, loop_ctx);
                    func.instruction(&Instruction::Call(self.func_indices["__str_starts_with"]));
                    true
                }
                "endsWith" if args.len() == 1 => {
                    self.compile_expr(object, locals, func, loop_ctx);
                    self.compile_expr(&args[0], locals, func, loop_ctx);
                    func.instruction(&Instruction::Call(self.func_indices["__str_ends_with"]));
                    true
                }
                "isBlank" if args.is_empty() => {
                    // isBlank: trim 后长度为 0 → 等价于 trim().isEmpty()
                    self.compile_expr(object, locals, func, loop_ctx);
                    func.instruction(&Instruction::Call(self.func_indices["__str_trim"]));
                    func.instruction(&Instruction::I32Load(wasm_encoder::MemArg {
                        offset: 0,
                        align: 2,
                        memory_index: 0,
                    }));
                    func.instruction(&Instruction::I32Eqz);
                    true
                }
                _ => false,
            },
            // P2.8: Array 实例方法
            Some(Type::Array(ref _elem_ty)) => {
                let is_float_array = matches!(_elem_ty.as_ref(), Type::Float64 | Type::Float32);
                let elem_size: i32 = 8;
                match method {
                    "clone" if args.is_empty() => {
                        // clone: 分配新数组，复制内容
                        // 需要临时变量
                        let src_local = locals
                            .get("__array_clone_src")
                            .unwrap_or_else(|| locals.get("__array_alloc_ptr").unwrap());
                        let dst_local = locals
                            .get("__array_clone_dst")
                            .unwrap_or_else(|| locals.get("__array_dyn_ptr").unwrap_or(src_local));
                        let alloc_idx = self.func_indices["__alloc"];

                        // 编译源数组
                        self.compile_expr(object, locals, func, loop_ctx);
                        func.instruction(&Instruction::LocalSet(src_local));

                        // 计算总大小: 4 + arr[0] * 8
                        func.instruction(&Instruction::LocalGet(src_local));
                        func.instruction(&Instruction::I32Load(wasm_encoder::MemArg {
                            offset: 0,
                            align: 2,
                            memory_index: 0,
                        }));
                        func.instruction(&Instruction::I32Const(elem_size));
                        func.instruction(&Instruction::I32Mul);
                        func.instruction(&Instruction::I32Const(4));
                        func.instruction(&Instruction::I32Add);

                        // 分配新数组
                        func.instruction(&Instruction::Call(alloc_idx));
                        func.instruction(&Instruction::LocalSet(dst_local));

                        // memory.copy(dst, src, size)
                        func.instruction(&Instruction::LocalGet(dst_local));
                        func.instruction(&Instruction::LocalGet(src_local));
                        // size
                        func.instruction(&Instruction::LocalGet(src_local));
                        func.instruction(&Instruction::I32Load(wasm_encoder::MemArg {
                            offset: 0,
                            align: 2,
                            memory_index: 0,
                        }));
                        func.instruction(&Instruction::I32Const(elem_size));
                        func.instruction(&Instruction::I32Mul);
                        func.instruction(&Instruction::I32Const(4));
                        func.instruction(&Instruction::I32Add);
                        func.instruction(&Instruction::MemoryCopy {
                            src_mem: 0,
                            dst_mem: 0,
                        });

                        func.instruction(&Instruction::LocalGet(dst_local));
                        true
                    }
                    "isEmpty" if args.is_empty() => {
                        // isEmpty: arr.size == 0 → i32 (Bool)
                        self.compile_expr(object, locals, func, loop_ctx);
                        func.instruction(&Instruction::I32Load(wasm_encoder::MemArg {
                            offset: 0,
                            align: 2,
                            memory_index: 0,
                        }));
                        func.instruction(&Instruction::I32Eqz);
                        true
                    }
                    "slice" if args.len() == 2 => {
                        // slice(start, end): 创建新数组 [start..end)
                        let src_local = locals
                            .get("__array_clone_src")
                            .unwrap_or_else(|| locals.get("__array_alloc_ptr").unwrap());
                        let alloc_idx = self.func_indices["__alloc"];

                        // 源数组
                        self.compile_expr(object, locals, func, loop_ctx);
                        func.instruction(&Instruction::LocalSet(src_local));

                        // 计算 new_len = end - start
                        let start_local = locals.get("__array_dyn_idx").unwrap();
                        let end_local = locals.get("__array_dyn_size").unwrap();
                        self.compile_expr(&args[0], locals, func, loop_ctx);
                        func.instruction(&Instruction::LocalSet(start_local));
                        self.compile_expr(&args[1], locals, func, loop_ctx);
                        func.instruction(&Instruction::LocalSet(end_local));

                        // new_len = end - start
                        let new_len_local = locals.get("__array_dyn_ptr").unwrap();
                        func.instruction(&Instruction::LocalGet(end_local));
                        func.instruction(&Instruction::LocalGet(start_local));
                        func.instruction(&Instruction::I64Sub);
                        // 转为 i32
                        func.instruction(&Instruction::I32WrapI64);
                        func.instruction(&Instruction::LocalSet(new_len_local));

                        // 分配 4 + new_len * 8
                        func.instruction(&Instruction::LocalGet(new_len_local));
                        func.instruction(&Instruction::I32Const(elem_size));
                        func.instruction(&Instruction::I32Mul);
                        func.instruction(&Instruction::I32Const(4));
                        func.instruction(&Instruction::I32Add);
                        func.instruction(&Instruction::Call(alloc_idx));

                        // 保存 dst
                        let dst_local = locals
                            .get("__array_clone_dst")
                            .unwrap_or_else(|| locals.get("__array_alloc_ptr").unwrap());
                        func.instruction(&Instruction::LocalSet(dst_local));

                        // 写入长度
                        func.instruction(&Instruction::LocalGet(dst_local));
                        func.instruction(&Instruction::LocalGet(new_len_local));
                        func.instruction(&Instruction::I32Store(wasm_encoder::MemArg {
                            offset: 0,
                            align: 2,
                            memory_index: 0,
                        }));

                        // memory.copy(dst+4, src+4+start*8, new_len*8)
                        func.instruction(&Instruction::LocalGet(dst_local));
                        func.instruction(&Instruction::I32Const(4));
                        func.instruction(&Instruction::I32Add);

                        func.instruction(&Instruction::LocalGet(src_local));
                        func.instruction(&Instruction::I32Const(4));
                        func.instruction(&Instruction::I32Add);
                        func.instruction(&Instruction::LocalGet(start_local));
                        func.instruction(&Instruction::I32WrapI64);
                        func.instruction(&Instruction::I32Const(elem_size));
                        func.instruction(&Instruction::I32Mul);
                        func.instruction(&Instruction::I32Add);

                        func.instruction(&Instruction::LocalGet(new_len_local));
                        func.instruction(&Instruction::I32Const(elem_size));
                        func.instruction(&Instruction::I32Mul);

                        func.instruction(&Instruction::MemoryCopy {
                            src_mem: 0,
                            dst_mem: 0,
                        });

                        func.instruction(&Instruction::LocalGet(dst_local));
                        true
                    }
                    // Array 不匹配时，降级到集合方法分发（ArrayList 行为）
                    _ => {
                        let inferred = obj_type.clone();
                        let type_name: Option<String> = None;
                        match method {
                            "append" if args.len() == 1 => {
                                self.compile_expr(object, locals, func, loop_ctx);
                                self.compile_expr(&args[0], locals, func, loop_ctx);
                                let vt = self.infer_type_with_locals(object, locals);
                                if vt == ValType::I64 {
                                    return false;
                                }
                                func.instruction(&Instruction::Call(
                                    self.func_indices["__arraylist_append"],
                                ));
                                func.instruction(&Instruction::I64Const(0));
                                return true;
                            }
                            "get" if args.len() == 1 => {
                                self.compile_expr(object, locals, func, loop_ctx);
                                self.compile_expr(&args[0], locals, func, loop_ctx);
                                func.instruction(&Instruction::Call(
                                    self.func_indices["__arraylist_get"],
                                ));
                                return true;
                            }
                            "set" if args.len() == 2 => {
                                self.compile_expr(object, locals, func, loop_ctx);
                                self.compile_expr(&args[0], locals, func, loop_ctx);
                                self.compile_expr(&args[1], locals, func, loop_ctx);
                                func.instruction(&Instruction::Call(
                                    self.func_indices["__arraylist_set"],
                                ));
                                func.instruction(&Instruction::I64Const(0));
                                return true;
                            }
                            "remove" if args.len() == 1 => {
                                self.compile_expr(object, locals, func, loop_ctx);
                                self.compile_expr(&args[0], locals, func, loop_ctx);
                                func.instruction(&Instruction::Call(
                                    self.func_indices["__arraylist_remove"],
                                ));
                                return true;
                            }
                            "size" if args.is_empty() => {
                                self.compile_expr(object, locals, func, loop_ctx);
                                func.instruction(&Instruction::I32Load(wasm_encoder::MemArg {
                                    offset: 0,
                                    align: 2,
                                    memory_index: 0,
                                }));
                                func.instruction(&Instruction::I64ExtendI32S);
                                return true;
                            }
                            // ArrayStack 方法
                            "push" if args.len() == 1 => {
                                self.compile_expr(object, locals, func, loop_ctx);
                                self.compile_expr(&args[0], locals, func, loop_ctx);
                                func.instruction(&Instruction::Call(
                                    self.func_indices["__arraylist_append"],
                                ));
                                func.instruction(&Instruction::I64Const(0));
                                return true;
                            }
                            "pop" if args.is_empty() => {
                                self.compile_expr(object, locals, func, loop_ctx);
                                // index = size - 1
                                self.compile_expr(object, locals, func, loop_ctx);
                                func.instruction(&Instruction::I32Load(wasm_encoder::MemArg {
                                    offset: 0,
                                    align: 2,
                                    memory_index: 0,
                                }));
                                func.instruction(&Instruction::I32Const(1));
                                func.instruction(&Instruction::I32Sub);
                                func.instruction(&Instruction::I64ExtendI32S);
                                func.instruction(&Instruction::Call(
                                    self.func_indices["__arraylist_remove"],
                                ));
                                return true;
                            }
                            "peek" if args.is_empty() => {
                                self.compile_expr(object, locals, func, loop_ctx);
                                // index = size - 1
                                self.compile_expr(object, locals, func, loop_ctx);
                                func.instruction(&Instruction::I32Load(wasm_encoder::MemArg {
                                    offset: 0,
                                    align: 2,
                                    memory_index: 0,
                                }));
                                func.instruction(&Instruction::I32Const(1));
                                func.instruction(&Instruction::I32Sub);
                                func.instruction(&Instruction::I64ExtendI32S);
                                func.instruction(&Instruction::Call(
                                    self.func_indices["__arraylist_get"],
                                ));
                                return true;
                            }
                            // LinkedList 方法
                            "prepend" if args.len() == 1 => {
                                self.compile_expr(object, locals, func, loop_ctx);
                                self.compile_expr(&args[0], locals, func, loop_ctx);
                                func.instruction(&Instruction::Call(
                                    self.func_indices["__linkedlist_prepend"],
                                ));
                                func.instruction(&Instruction::I64Const(0));
                                return true;
                            }
                            // HashMap/HashSet 方法（通过 ArrayList 类型推断的 HashMap 也可能走到这里）
                            // 注意：此处无 Map 类型信息，通过表达式推断 key/val 的 WASM 类型
                            "put" if args.len() == 2 => {
                                self.compile_expr(object, locals, func, loop_ctx);
                                self.compile_expr(&args[0], locals, func, loop_ctx);
                                let k_ty = self.infer_type_with_locals(&args[0], locals);
                                if k_ty == wasm_encoder::ValType::I32 {
                                    func.instruction(&Instruction::I64ExtendI32S);
                                }
                                self.compile_expr(&args[1], locals, func, loop_ctx);
                                let v_ty = self.infer_type_with_locals(&args[1], locals);
                                if v_ty == wasm_encoder::ValType::I32 {
                                    func.instruction(&Instruction::I64ExtendI32S);
                                }
                                func.instruction(&Instruction::Call(
                                    self.func_indices["__hashmap_put"],
                                ));
                                func.instruction(&Instruction::I64Const(0));
                                return true;
                            }
                            "containsKey" if args.len() == 1 => {
                                self.compile_expr(object, locals, func, loop_ctx);
                                self.compile_expr(&args[0], locals, func, loop_ctx);
                                let k_ty = self.infer_type_with_locals(&args[0], locals);
                                if k_ty == wasm_encoder::ValType::I32 {
                                    func.instruction(&Instruction::I64ExtendI32S);
                                }
                                func.instruction(&Instruction::Call(
                                    self.func_indices["__hashmap_contains"],
                                ));
                                return true;
                            }
                            "add" if args.len() == 1 => {
                                self.compile_expr(object, locals, func, loop_ctx);
                                self.compile_expr(&args[0], locals, func, loop_ctx);
                                let k_ty = self.infer_type_with_locals(&args[0], locals);
                                if k_ty == wasm_encoder::ValType::I32 {
                                    func.instruction(&Instruction::I64ExtendI32S);
                                }
                                func.instruction(&Instruction::I64Const(0));
                                func.instruction(&Instruction::Call(
                                    self.func_indices["__hashmap_put"],
                                ));
                                func.instruction(&Instruction::I64Const(0));
                                return true;
                            }
                            "contains" if args.len() == 1 => {
                                self.compile_expr(object, locals, func, loop_ctx);
                                self.compile_expr(&args[0], locals, func, loop_ctx);
                                let k_ty = self.infer_type_with_locals(&args[0], locals);
                                if k_ty == wasm_encoder::ValType::I32 {
                                    func.instruction(&Instruction::I64ExtendI32S);
                                }
                                func.instruction(&Instruction::Call(
                                    self.func_indices["__hashmap_contains"],
                                ));
                                return true;
                            }
                            _ => return false,
                        }
                    }
                }
            }
            // P4: Map 类型方法分发（HashMap/HashSet）
            Some(Type::Map(ref key_ty, ref val_ty)) => {
                // __hashmap_* 运行时函数均以 i64 传递 key（和 val）
                // 当 key/val 的 WASM 类型为 i32（如 UInt32、Rune、Int32 等）时需要先扩展
                let key_wasm = key_ty.to_wasm();
                let val_wasm = val_ty.to_wasm();
                match method {
                    "put" if args.len() == 2 => {
                        self.compile_expr(object, locals, func, loop_ctx);
                        self.compile_expr(&args[0], locals, func, loop_ctx);
                        if key_wasm == wasm_encoder::ValType::I32 {
                            func.instruction(&Instruction::I64ExtendI32S);
                        }
                        self.compile_expr(&args[1], locals, func, loop_ctx);
                        if val_wasm == wasm_encoder::ValType::I32 {
                            func.instruction(&Instruction::I64ExtendI32S);
                        }
                        func.instruction(&Instruction::Call(self.func_indices["__hashmap_put"]));
                        func.instruction(&Instruction::I64Const(0));
                        true
                    }
                    "get" if args.len() == 1 => {
                        self.compile_expr(object, locals, func, loop_ctx);
                        self.compile_expr(&args[0], locals, func, loop_ctx);
                        if key_wasm == wasm_encoder::ValType::I32 {
                            func.instruction(&Instruction::I64ExtendI32S);
                        }
                        func.instruction(&Instruction::Call(self.func_indices["__hashmap_get"]));
                        // __hashmap_get 返回 i64；若 val 类型本为 i32（指针），则截断回 i32
                        if val_wasm == wasm_encoder::ValType::I32 {
                            func.instruction(&Instruction::I32WrapI64);
                        }
                        true
                    }
                    "containsKey" if args.len() == 1 => {
                        self.compile_expr(object, locals, func, loop_ctx);
                        self.compile_expr(&args[0], locals, func, loop_ctx);
                        if key_wasm == wasm_encoder::ValType::I32 {
                            func.instruction(&Instruction::I64ExtendI32S);
                        }
                        func.instruction(&Instruction::Call(
                            self.func_indices["__hashmap_contains"],
                        ));
                        // __hashmap_contains 返回 i32 (Bool)，不需要进一步处理
                        true
                    }
                    "remove" if args.len() == 1 => {
                        self.compile_expr(object, locals, func, loop_ctx);
                        self.compile_expr(&args[0], locals, func, loop_ctx);
                        if key_wasm == wasm_encoder::ValType::I32 {
                            func.instruction(&Instruction::I64ExtendI32S);
                        }
                        func.instruction(&Instruction::Call(self.func_indices["__hashmap_remove"]));
                        if val_wasm == wasm_encoder::ValType::I32 {
                            func.instruction(&Instruction::I32WrapI64);
                        }
                        true
                    }
                    "add" if args.len() == 1 => {
                        // HashSet.add(elem) → hashmap_put(set, elem_as_i64, 0)
                        self.compile_expr(object, locals, func, loop_ctx);
                        self.compile_expr(&args[0], locals, func, loop_ctx);
                        if key_wasm == wasm_encoder::ValType::I32 {
                            func.instruction(&Instruction::I64ExtendI32S);
                        }
                        func.instruction(&Instruction::I64Const(0));
                        func.instruction(&Instruction::Call(self.func_indices["__hashmap_put"]));
                        func.instruction(&Instruction::I64Const(0));
                        true
                    }
                    "contains" if args.len() == 1 => {
                        self.compile_expr(object, locals, func, loop_ctx);
                        self.compile_expr(&args[0], locals, func, loop_ctx);
                        if key_wasm == wasm_encoder::ValType::I32 {
                            func.instruction(&Instruction::I64ExtendI32S);
                        }
                        func.instruction(&Instruction::Call(
                            self.func_indices["__hashmap_contains"],
                        ));
                        // __hashmap_contains 返回 i32 (Bool)，不需要进一步处理
                        true
                    }
                    "size" if args.is_empty() => {
                        self.compile_expr(object, locals, func, loop_ctx);
                        func.instruction(&Instruction::I32Load(wasm_encoder::MemArg {
                            offset: 0,
                            align: 2,
                            memory_index: 0,
                        }));
                        func.instruction(&Instruction::I64ExtendI32S);
                        true
                    }
                    _ => false,
                }
            }
            _ => {
                // Phase 7.5: 集合类型方法分发（对象为 i32 指针）
                // 仅当对象不是已知 struct/class 时才分发到集合运行时
                // 使用 infer_ast_type_with_locals 获取对象的 AST 类型
                let inferred = self.infer_ast_type_with_locals(object, locals);
                let type_name: Option<String> = match &inferred {
                    Some(Type::Struct(n, _)) => Some(n.clone()),
                    _ => None,
                };
                if let Some(ref tn) = type_name {
                    // P5: Atomic/Mutex 桩方法分发
                    match tn.as_str() {
                        "AtomicInt64" => {
                            match method {
                                "load" if args.is_empty() => {
                                    self.compile_expr(object, locals, func, loop_ctx);
                                    func.instruction(&Instruction::I64Load(wasm_encoder::MemArg {
                                        offset: 0,
                                        align: 3,
                                        memory_index: 0,
                                    }));
                                    return true;
                                }
                                "store" if args.len() == 1 => {
                                    self.compile_expr(object, locals, func, loop_ctx);
                                    self.compile_expr(&args[0], locals, func, loop_ctx);
                                    func.instruction(&Instruction::I64Store(
                                        wasm_encoder::MemArg {
                                            offset: 0,
                                            align: 3,
                                            memory_index: 0,
                                        },
                                    ));
                                    func.instruction(&Instruction::I64Const(0)); // void → 哑值供 Drop
                                    return true;
                                }
                                "fetchAdd" if args.len() == 1 => {
                                    // old = load; store(old + delta); return old
                                    self.compile_expr(object, locals, func, loop_ctx);
                                    func.instruction(&Instruction::I64Load(wasm_encoder::MemArg {
                                        offset: 0,
                                        align: 3,
                                        memory_index: 0,
                                    }));
                                    // old value 在栈上 — 需要保存
                                    // 用 object 地址 + 新值存储
                                    // stack: old_val
                                    // 重新加载对象地址
                                    self.compile_expr(object, locals, func, loop_ctx);
                                    // stack: old_val, obj_ptr
                                    // 计算 old + delta
                                    self.compile_expr(object, locals, func, loop_ctx);
                                    func.instruction(&Instruction::I64Load(wasm_encoder::MemArg {
                                        offset: 0,
                                        align: 3,
                                        memory_index: 0,
                                    }));
                                    self.compile_expr(&args[0], locals, func, loop_ctx);
                                    func.instruction(&Instruction::I64Add);
                                    // stack: old_val, obj_ptr, new_val
                                    func.instruction(&Instruction::I64Store(
                                        wasm_encoder::MemArg {
                                            offset: 0,
                                            align: 3,
                                            memory_index: 0,
                                        },
                                    ));
                                    // stack: old_val (returned)
                                    return true;
                                }
                                "compareAndSwap" if args.len() == 2 => {
                                    // single-threaded: if (current == expected) { store(desired); return true } else return false
                                    self.compile_expr(object, locals, func, loop_ctx);
                                    func.instruction(&Instruction::I64Load(wasm_encoder::MemArg {
                                        offset: 0,
                                        align: 3,
                                        memory_index: 0,
                                    }));
                                    self.compile_expr(&args[0], locals, func, loop_ctx);
                                    func.instruction(&Instruction::I64Eq);
                                    func.instruction(&Instruction::If(
                                        wasm_encoder::BlockType::Result(ValType::I32),
                                    ));
                                    // 匹配: 存储 desired
                                    self.compile_expr(object, locals, func, loop_ctx);
                                    self.compile_expr(&args[1], locals, func, loop_ctx);
                                    func.instruction(&Instruction::I64Store(
                                        wasm_encoder::MemArg {
                                            offset: 0,
                                            align: 3,
                                            memory_index: 0,
                                        },
                                    ));
                                    func.instruction(&Instruction::I32Const(1)); // true
                                    func.instruction(&Instruction::Else);
                                    func.instruction(&Instruction::I32Const(0)); // false
                                    func.instruction(&Instruction::End);
                                    return true;
                                }
                                _ => {}
                            }
                        }
                        "AtomicBool" => {
                            match method {
                                "load" if args.is_empty() => {
                                    self.compile_expr(object, locals, func, loop_ctx);
                                    func.instruction(&Instruction::I64Load(wasm_encoder::MemArg {
                                        offset: 0,
                                        align: 3,
                                        memory_index: 0,
                                    }));
                                    func.instruction(&Instruction::I32WrapI64); // Bool → I32
                                    return true;
                                }
                                "store" if args.len() == 1 => {
                                    self.compile_expr(object, locals, func, loop_ctx);
                                    self.compile_expr(&args[0], locals, func, loop_ctx);
                                    // arg may be Bool (I32) or Int (I64); extend to I64 for i64.store if needed
                                    let arg_wasm = self.infer_type_with_locals(&args[0], locals);
                                    if arg_wasm == ValType::I32 {
                                        func.instruction(&Instruction::I64ExtendI32S);
                                    }
                                    func.instruction(&Instruction::I64Store(
                                        wasm_encoder::MemArg {
                                            offset: 0,
                                            align: 3,
                                            memory_index: 0,
                                        },
                                    ));
                                    func.instruction(&Instruction::I64Const(0)); // void → 哑值供 Drop
                                    return true;
                                }
                                "compareAndSwap" if args.len() == 2 => {
                                    self.compile_expr(object, locals, func, loop_ctx);
                                    func.instruction(&Instruction::I64Load(wasm_encoder::MemArg {
                                        offset: 0,
                                        align: 3,
                                        memory_index: 0,
                                    }));
                                    self.compile_expr(&args[0], locals, func, loop_ctx);
                                    func.instruction(&Instruction::I64Eq);
                                    func.instruction(&Instruction::If(
                                        wasm_encoder::BlockType::Result(ValType::I32),
                                    ));
                                    self.compile_expr(object, locals, func, loop_ctx);
                                    self.compile_expr(&args[1], locals, func, loop_ctx);
                                    func.instruction(&Instruction::I64Store(
                                        wasm_encoder::MemArg {
                                            offset: 0,
                                            align: 3,
                                            memory_index: 0,
                                        },
                                    ));
                                    func.instruction(&Instruction::I32Const(1));
                                    func.instruction(&Instruction::Else);
                                    func.instruction(&Instruction::I32Const(0));
                                    func.instruction(&Instruction::End);
                                    return true;
                                }
                                _ => {}
                            }
                        }
                        "Mutex" | "ReentrantMutex" => {
                            match method {
                                "lock" if args.is_empty() => {
                                    // 单线程桩: 空操作，推哑值供 Drop
                                    func.instruction(&Instruction::I64Const(0));
                                    return true;
                                }
                                "unlock" if args.is_empty() => {
                                    func.instruction(&Instruction::I64Const(0));
                                    return true;
                                }
                                "tryLock" if args.is_empty() => {
                                    func.instruction(&Instruction::I32Const(1)); // 单线程总是成功 (Bool → I32)
                                    return true;
                                }
                                _ => {}
                            }
                        }
                        _ => {}
                    }
                    let qualified = format!("{}.{}", tn, method);
                    if self.func_indices.contains_key(&qualified) {
                        return false; // 让正常 struct/class 方法分发处理
                    }
                }
                match method {
                    "append" if args.len() == 1 => {
                        // ArrayList/LinkedList.append(elem)
                        self.compile_expr(object, locals, func, loop_ctx);
                        self.compile_expr(&args[0], locals, func, loop_ctx);
                        // 如果对象推断类型是 i32 → 集合指针
                        let vt = self.infer_type_with_locals(object, locals);
                        if vt == ValType::I32 {
                            func.instruction(&Instruction::Call(
                                self.func_indices["__arraylist_append"],
                            ));
                        } else {
                            // i64 → 转为 i32 再调用
                            func.instruction(&Instruction::I32WrapI64);
                            let arg_vt = self.infer_type_with_locals(&args[0], locals);
                            if arg_vt != ValType::I64 {
                                func.instruction(&Instruction::I64ExtendI32S);
                            }
                            // 需要重新组织栈: (obj_i64, arg) → 先取出对象
                            return false; // 回退
                        }
                        func.instruction(&Instruction::I64Const(0)); // void → 哑值供 Stmt::Expr drop
                        true
                    }
                    "get" if args.len() == 1 => {
                        // P3: 按对象类型分派 get —— HashMap 用 __hashmap_get，ArrayList 用 __arraylist_get
                        let is_hashmap = matches!(&inferred, Some(Type::Map(..)));
                        let (key_wasm, val_wasm) = if let Some(Type::Map(ref k, ref v)) = inferred {
                            (k.to_wasm(), v.to_wasm())
                        } else {
                            (wasm_encoder::ValType::I64, wasm_encoder::ValType::I64)
                        };
                        self.compile_expr(object, locals, func, loop_ctx);
                        self.compile_expr(&args[0], locals, func, loop_ctx);
                        if is_hashmap {
                            if key_wasm == wasm_encoder::ValType::I32 {
                                func.instruction(&Instruction::I64ExtendI32S);
                            }
                            func.instruction(&Instruction::Call(
                                self.func_indices["__hashmap_get"],
                            ));
                            if val_wasm == wasm_encoder::ValType::I32 {
                                func.instruction(&Instruction::I32WrapI64);
                            }
                        } else {
                            func.instruction(&Instruction::Call(
                                self.func_indices["__arraylist_get"],
                            ));
                        }
                        true
                    }
                    "set" if args.len() == 2 => {
                        self.compile_expr(object, locals, func, loop_ctx);
                        self.compile_expr(&args[0], locals, func, loop_ctx);
                        self.compile_expr(&args[1], locals, func, loop_ctx);
                        func.instruction(&Instruction::Call(self.func_indices["__arraylist_set"]));
                        func.instruction(&Instruction::I64Const(0)); // void → 哑值
                        true
                    }
                    "remove" if args.len() == 1 => {
                        // P4: 按对象类型分派 remove
                        let is_hashmap = matches!(&inferred, Some(Type::Map(..)));
                        let (key_wasm, val_wasm) = if let Some(Type::Map(ref k, ref v)) = inferred {
                            (k.to_wasm(), v.to_wasm())
                        } else {
                            (wasm_encoder::ValType::I64, wasm_encoder::ValType::I64)
                        };
                        self.compile_expr(object, locals, func, loop_ctx);
                        self.compile_expr(&args[0], locals, func, loop_ctx);
                        if is_hashmap {
                            if key_wasm == wasm_encoder::ValType::I32 {
                                func.instruction(&Instruction::I64ExtendI32S);
                            }
                            func.instruction(&Instruction::Call(
                                self.func_indices["__hashmap_remove"],
                            ));
                            if val_wasm == wasm_encoder::ValType::I32 {
                                func.instruction(&Instruction::I32WrapI64);
                            }
                        } else {
                            func.instruction(&Instruction::Call(
                                self.func_indices["__arraylist_remove"],
                            ));
                        }
                        true
                    }
                    "put" if args.len() == 2 => {
                        // HashMap.put(key, val)
                        // 提取 key 和 val 类型以进行类型转换
                        let (key_wasm, val_wasm) = if let Some(Type::Map(ref k, ref v)) = inferred {
                            (k.to_wasm(), v.to_wasm())
                        } else {
                            (wasm_encoder::ValType::I64, wasm_encoder::ValType::I64)
                        };
                        self.compile_expr(object, locals, func, loop_ctx);
                        self.compile_expr(&args[0], locals, func, loop_ctx);
                        if key_wasm == wasm_encoder::ValType::I32 {
                            func.instruction(&Instruction::I64ExtendI32S);
                        }
                        self.compile_expr(&args[1], locals, func, loop_ctx);
                        if val_wasm == wasm_encoder::ValType::I32 {
                            func.instruction(&Instruction::I64ExtendI32S);
                        }
                        func.instruction(&Instruction::Call(self.func_indices["__hashmap_put"]));
                        func.instruction(&Instruction::I64Const(0)); // void → 哑值
                        true
                    }
                    "containsKey" if args.len() == 1 => {
                        let (key_wasm, _) = if let Some(Type::Map(ref k, ref v)) = inferred {
                            (k.to_wasm(), v.to_wasm())
                        } else {
                            (wasm_encoder::ValType::I64, wasm_encoder::ValType::I64)
                        };
                        self.compile_expr(object, locals, func, loop_ctx);
                        self.compile_expr(&args[0], locals, func, loop_ctx);
                        if key_wasm == wasm_encoder::ValType::I32 {
                            func.instruction(&Instruction::I64ExtendI32S);
                        }
                        func.instruction(&Instruction::Call(
                            self.func_indices["__hashmap_contains"],
                        ));
                        // __hashmap_contains 返回 i32，扩展为 i64 以匹配 Int64 类型系统
                        func.instruction(&Instruction::I64ExtendI32S);
                        true
                    }
                    "add" if args.len() == 1 => {
                        // HashSet.add(elem) → hashmap_put(set, elem, 0)
                        let (key_wasm, _) = if let Some(Type::Map(ref k, ref v)) = inferred {
                            (k.to_wasm(), v.to_wasm())
                        } else {
                            (wasm_encoder::ValType::I64, wasm_encoder::ValType::I64)
                        };
                        self.compile_expr(object, locals, func, loop_ctx);
                        self.compile_expr(&args[0], locals, func, loop_ctx);
                        if key_wasm == wasm_encoder::ValType::I32 {
                            func.instruction(&Instruction::I64ExtendI32S);
                        }
                        func.instruction(&Instruction::I64Const(0)); // dummy value
                        func.instruction(&Instruction::Call(self.func_indices["__hashmap_put"]));
                        func.instruction(&Instruction::I64Const(0)); // void → 哑值
                        true
                    }
                    "push" if args.len() == 1 => {
                        // ArrayStack.push = ArrayList.append
                        self.compile_expr(object, locals, func, loop_ctx);
                        self.compile_expr(&args[0], locals, func, loop_ctx);
                        func.instruction(&Instruction::Call(
                            self.func_indices["__arraylist_append"],
                        ));
                        func.instruction(&Instruction::I64Const(0)); // void → 哑值
                        true
                    }
                    "pop" if args.is_empty() => {
                        // ArrayStack.pop = remove last
                        self.compile_expr(object, locals, func, loop_ctx);
                        // index = size - 1
                        self.compile_expr(object, locals, func, loop_ctx);
                        func.instruction(&Instruction::I32Load(wasm_encoder::MemArg {
                            offset: 0,
                            align: 2,
                            memory_index: 0,
                        }));
                        func.instruction(&Instruction::I32Const(1));
                        func.instruction(&Instruction::I32Sub);
                        func.instruction(&Instruction::I64ExtendI32S);
                        func.instruction(&Instruction::Call(
                            self.func_indices["__arraylist_remove"],
                        ));
                        true
                    }
                    "peek" if args.is_empty() => {
                        // ArrayStack.peek = get last
                        self.compile_expr(object, locals, func, loop_ctx);
                        // index = size - 1
                        self.compile_expr(object, locals, func, loop_ctx);
                        func.instruction(&Instruction::I32Load(wasm_encoder::MemArg {
                            offset: 0,
                            align: 2,
                            memory_index: 0,
                        }));
                        func.instruction(&Instruction::I32Const(1));
                        func.instruction(&Instruction::I32Sub);
                        func.instruction(&Instruction::I64ExtendI32S);
                        func.instruction(&Instruction::Call(self.func_indices["__arraylist_get"]));
                        true
                    }
                    "prepend" if args.len() == 1 => {
                        self.compile_expr(object, locals, func, loop_ctx);
                        self.compile_expr(&args[0], locals, func, loop_ctx);
                        func.instruction(&Instruction::Call(
                            self.func_indices["__linkedlist_prepend"],
                        ));
                        func.instruction(&Instruction::I64Const(0)); // void → 哑值
                        true
                    }
                    // P4: 集合 size 属性方法
                    "size" if args.is_empty() => {
                        // ArrayList: [len: i32][cap: i32][data_ptr: i32] → len at offset 0
                        // HashMap: [size: i32][cap: i32][...] → size at offset 0
                        self.compile_expr(object, locals, func, loop_ctx);
                        func.instruction(&Instruction::I32Load(wasm_encoder::MemArg {
                            offset: 0,
                            align: 2,
                            memory_index: 0,
                        }));
                        func.instruction(&Instruction::I64ExtendI32S);
                        true
                    }
                    // P4: HashSet.contains(elem)
                    "contains" if args.len() == 1 => {
                        self.compile_expr(object, locals, func, loop_ctx);
                        self.compile_expr(&args[0], locals, func, loop_ctx);
                        func.instruction(&Instruction::Call(
                            self.func_indices["__hashmap_contains"],
                        ));
                        // __hashmap_contains 返回 i32 (Bool)，不需要扩展
                        true
                    }
                    _ => false,
                }
            }
        }
    }
    pub(crate) fn compile_stmt(
        &self,
        stmt: &Stmt,
        locals: &LocalsBuilder,
        func: &mut WasmFunc,
        loop_ctx: Option<(u32, u32)>,
    ) {
        match stmt {
            Stmt::Let { pattern, value, .. } => {
                // P1.3: let _ = expr → 编译表达式后 drop 结果值
                if matches!(pattern, Pattern::Wildcard) {
                    self.compile_expr(value, locals, func, loop_ctx);
                    if self.expr_produces_value(value) {
                        func.instruction(&Instruction::Drop);
                    }
                } else {
                    self.compile_expr(value, locals, func, loop_ctx);
                    match pattern {
                        Pattern::Binding(name) => {
                            let idx = locals.get(name).expect("局部变量未找到");
                            // 值类型与局部变量类型不匹配时自动转换
                            let val_ty = self.infer_type_with_locals(value, locals);
                            let local_ty = locals.get_valtype(name).unwrap_or(val_ty);
                            self.emit_type_coercion(func, val_ty, local_ty);
                            func.instruction(&Instruction::LocalSet(idx));
                        }
                        Pattern::Tuple(patterns) => {
                            // 元组解构：let (x, y) = tuple
                            let ptr_tmp = locals.get("__let_tuple_ptr").expect("__let_tuple_ptr");
                            func.instruction(&Instruction::LocalSet(ptr_tmp));
                            let value_ast_ty = self.infer_ast_type_with_locals(value, locals);
                            let elem_types: Vec<Option<Type>> = value_ast_ty
                                .as_ref()
                                .and_then(|t| {
                                    if let Type::Tuple(ts) = t {
                                        Some(ts.iter().cloned().map(Some).collect())
                                    } else {
                                        None
                                    }
                                })
                                .unwrap_or_else(|| patterns.iter().map(|_| None).collect());
                            let mut offset = 0u32;
                            for (i, pat) in patterns.iter().enumerate() {
                                if let Pattern::Binding(name) = pat {
                                    let idx = locals.get(name).expect("局部变量未找到");
                                    func.instruction(&Instruction::LocalGet(ptr_tmp));
                                    if offset > 0 {
                                        func.instruction(&Instruction::I32Const(offset as i32));
                                        func.instruction(&Instruction::I32Add);
                                    }
                                    if let Some(Some(ty)) = elem_types.get(i) {
                                        self.emit_load_by_type(func, ty);
                                        offset += ty.size() as u32;
                                    } else {
                                        func.instruction(&Instruction::I64Load(
                                            wasm_encoder::MemArg {
                                                offset: 0,
                                                align: 3,
                                                memory_index: 0,
                                            },
                                        ));
                                        offset += 8;
                                    }
                                    let local_ty = locals.get_valtype(name).unwrap_or(ValType::I64);
                                    let val_ty = elem_types
                                        .get(i)
                                        .and_then(|t| t.as_ref().map(|t| t.to_wasm()))
                                        .unwrap_or(ValType::I64);
                                    self.emit_type_coercion(func, val_ty, local_ty);
                                    func.instruction(&Instruction::LocalSet(idx));
                                }
                            }
                        }
                        Pattern::Struct {
                            name: struct_name,
                            fields,
                        } => {
                            let ptr_tmp = locals.get("__let_struct_ptr").expect("__let_struct_ptr");
                            func.instruction(&Instruction::LocalSet(ptr_tmp));
                            let struct_def = &self.structs[struct_name];
                            for (fname, pat) in fields {
                                let offset = struct_def.field_offset(fname).expect("结构体字段");
                                let fty = struct_def.field_type(fname).expect("字段类型");
                                func.instruction(&Instruction::LocalGet(ptr_tmp));
                                func.instruction(&Instruction::I32Const(offset as i32));
                                func.instruction(&Instruction::I32Add);
                                self.emit_load_by_type(func, fty);
                                if let Pattern::Binding(bind) = pat {
                                    let idx = locals.get(bind).expect("解构绑定名");
                                    func.instruction(&Instruction::LocalSet(idx));
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            Stmt::Var { pattern, value, .. } => {
                self.compile_expr(value, locals, func, loop_ctx);
                match pattern {
                    Pattern::Binding(name) => {
                        let idx = locals.get(name).expect("局部变量未找到");
                        let val_ty = self.infer_type_with_locals(value, locals);
                        let local_ty = locals.get_valtype(name).unwrap_or(val_ty);
                        self.emit_type_coercion(func, val_ty, local_ty);
                        func.instruction(&Instruction::LocalSet(idx));
                    }
                    Pattern::Tuple(patterns) => {
                        let ptr_tmp = locals.get("__var_tuple_ptr").expect("__var_tuple_ptr");
                        func.instruction(&Instruction::LocalSet(ptr_tmp));
                        let value_ast_ty = self.infer_ast_type_with_locals(value, locals);
                        let elem_types: Vec<Option<Type>> = value_ast_ty
                            .as_ref()
                            .and_then(|t| {
                                if let Type::Tuple(ts) = t {
                                    Some(ts.iter().cloned().map(Some).collect())
                                } else {
                                    None
                                }
                            })
                            .unwrap_or_else(|| patterns.iter().map(|_| None).collect());
                        for (i, pat) in patterns.iter().enumerate() {
                            if let Pattern::Binding(name) = pat {
                                let idx = locals.get(name).expect("局部变量未找到");
                                func.instruction(&Instruction::LocalGet(ptr_tmp));
                                func.instruction(&Instruction::I32Const(i as i32 * 8));
                                func.instruction(&Instruction::I32Add);
                                if let Some(Some(ty)) = elem_types.get(i) {
                                    self.emit_load_by_type(func, ty);
                                } else {
                                    func.instruction(&Instruction::I64Load(wasm_encoder::MemArg {
                                        offset: 0,
                                        align: 3,
                                        memory_index: 0,
                                    }));
                                }
                                let local_ty = locals.get_valtype(name).unwrap_or(ValType::I64);
                                self.emit_type_coercion(func, ValType::I64, local_ty);
                                func.instruction(&Instruction::LocalSet(idx));
                            }
                            // 非 Binding 子模式（如 _）则 drop 该槽位，此处简化不处理
                        }
                    }
                    Pattern::Struct {
                        name: struct_name,
                        fields,
                    } => {
                        let ptr_tmp = locals.get("__var_struct_ptr").expect("__var_struct_ptr");
                        func.instruction(&Instruction::LocalSet(ptr_tmp));
                        let struct_def = &self.structs[struct_name];
                        for (fname, pat) in fields {
                            let offset = struct_def.field_offset(fname).expect("结构体字段");
                            let fty = struct_def.field_type(fname).expect("字段类型");
                            if let Pattern::Binding(bind) = pat {
                                let bind_idx = locals.get(bind).expect("解构绑定名");
                                func.instruction(&Instruction::LocalGet(ptr_tmp));
                                func.instruction(&Instruction::I32Const(offset as i32));
                                func.instruction(&Instruction::I32Add);
                                self.emit_load_by_type(func, fty);
                                let local_ty = locals.get_valtype(bind).unwrap_or(fty.to_wasm());
                                self.emit_type_coercion(func, fty.to_wasm(), local_ty);
                                func.instruction(&Instruction::LocalSet(bind_idx));
                            }
                        }
                    }
                    _ => {
                        func.instruction(&Instruction::Drop);
                    }
                }
            }
            Stmt::LocalFunc(_) => {
                // 局部函数已在 collect_local_funcs_from_functions 阶段加入 functions 并编译，此处无需生成指令
            }
            Stmt::Assign { target, value } => {
                match target {
                    AssignTarget::Var(name) => {
                        // Bug B2 修复: 隐式 this 字段赋值 — 将 `field = value` 重写为 `this.field = value`
                        if locals.get(name).is_none() && locals.get("this").is_some() {
                            let field_target = AssignTarget::Field {
                                object: "this".to_string(),
                                field: name.clone(),
                            };
                            self.compile_stmt(
                                &Stmt::Assign {
                                    target: field_target,
                                    value: value.clone(),
                                },
                                locals,
                                func,
                                loop_ctx,
                            );
                        } else {
                            // Phase 8: 引用计数 - 赋值前对旧值 rc_dec
                            if let Some(ast_ty) = locals.get_type(name) {
                                if memory::is_heap_type(ast_ty) || memory::may_hold_heap_ptr(ast_ty)
                                {
                                    if let Some(rc_dec_idx) = self.func_indices.get("__rc_dec") {
                                        let idx = locals.get(name).expect("变量未找到");
                                        func.instruction(&Instruction::LocalGet(idx));
                                        func.instruction(&Instruction::Call(*rc_dec_idx));
                                    }
                                }
                            }
                            self.compile_expr(value, locals, func, loop_ctx);
                            let idx = locals.get(name).expect("变量未找到");
                            // 值类型与局部变量类型不匹配时自动转换
                            let val_ty = self.infer_type_with_locals(value, locals);
                            let local_ty = locals.get_valtype(name).unwrap_or(val_ty);
                            self.emit_type_coercion(func, val_ty, local_ty);
                            func.instruction(&Instruction::LocalSet(idx));
                        }
                    }
                    AssignTarget::Index { array, index } => {
                        // arr[i] = value
                        if let Some(arr_idx) = locals.get(array) {
                            // 计算地址: arr + 4 + i * 8
                            let is_float_elem = locals
                                .get_type(array)
                                .map(|ty| match ty {
                                    Type::Array(ref elem_ty) => {
                                        matches!(**elem_ty, Type::Float64 | Type::Float32)
                                    }
                                    _ => false,
                                })
                                .unwrap_or(false);
                            func.instruction(&Instruction::LocalGet(arr_idx));
                            func.instruction(&Instruction::I32Const(4)); // 跳过长度字段
                            func.instruction(&Instruction::I32Add);
                            self.compile_expr(index, locals, func, loop_ctx);
                            // P3: 索引若为 I64（整数），wrap 为 I32 参与地址计算
                            if self.infer_type_with_locals(index, locals) == ValType::I64 {
                                func.instruction(&Instruction::I32WrapI64);
                            }
                            func.instruction(&Instruction::I32Const(8)); // 元素大小
                            func.instruction(&Instruction::I32Mul);
                            func.instruction(&Instruction::I32Add);
                            self.compile_expr(value, locals, func, loop_ctx);
                            if is_float_elem {
                                func.instruction(&Instruction::F64Store(wasm_encoder::MemArg {
                                    offset: 0,
                                    align: 3,
                                    memory_index: 0,
                                }));
                            } else {
                                // 数组元素统一以 I64 存储；若值为 I32，先扩展到 I64
                                let val_ty = self.infer_type_with_locals(value, locals);
                                self.emit_type_coercion(func, val_ty, ValType::I64);
                                func.instruction(&Instruction::I64Store(wasm_encoder::MemArg {
                                    offset: 0,
                                    align: 3,
                                    memory_index: 0,
                                }));
                            }
                        } else {
                            // 不在局部变量中 - 尝试作为 this 的字段处理（隐式 this.array[index] = value）
                            let arr_expr = Expr::Field {
                                object: Box::new(Expr::Var("this".to_string())),
                                field: array.clone(),
                            };
                            self.compile_expr(&arr_expr, locals, func, loop_ctx);
                            // 如果字段返回 I64（未知类型），需要 wrap 为 I32
                            if self.infer_type_with_locals(&arr_expr, locals) == ValType::I64 {
                                func.instruction(&Instruction::I32WrapI64);
                            }
                            func.instruction(&Instruction::I32Const(4));
                            func.instruction(&Instruction::I32Add);
                            self.compile_expr(index, locals, func, loop_ctx);
                            // P3: 索引若为 I64（整数），wrap 为 I32 参与地址计算
                            if self.infer_type_with_locals(index, locals) == ValType::I64 {
                                func.instruction(&Instruction::I32WrapI64);
                            }
                            func.instruction(&Instruction::I32Const(8));
                            func.instruction(&Instruction::I32Mul);
                            func.instruction(&Instruction::I32Add);
                            self.compile_expr(value, locals, func, loop_ctx);
                            let val_ty = self.infer_type_with_locals(value, locals);
                            self.emit_type_coercion(func, val_ty, ValType::I64);
                            func.instruction(&Instruction::I64Store(wasm_encoder::MemArg {
                                offset: 0,
                                align: 3,
                                memory_index: 0,
                            }));
                        }
                    }
                    AssignTarget::Field { object, field } => {
                        // obj.field = value：用对象类型计算字段偏移与字段类型
                        let obj_idx = locals.get(object).expect("对象未找到");
                        let (offset, field_ty) = locals
                            .get_type(object)
                            .and_then(|ty| match ty {
                                Type::Struct(name, type_args) => {
                                    // 泛型类型需要查找修饰后的名字
                                    let lookup_name = if !type_args.is_empty() {
                                        let mangled =
                                            crate::monomorph::mangle_name(name, type_args);
                                        if self.classes.contains_key(&mangled)
                                            || self.structs.contains_key(&mangled)
                                        {
                                            mangled
                                        } else {
                                            name.clone()
                                        }
                                    } else {
                                        name.clone()
                                    };
                                    // 优先从 ClassInfo 获取偏移（包含 vtable header）
                                    if let Some(ci) = self.classes.get(&lookup_name) {
                                        let off = ci.field_offset(field)?;
                                        let ft = ci.field_type(field)?.clone();
                                        Some((off, ft))
                                    } else {
                                        self.structs.get(&lookup_name).and_then(|def| {
                                            let off = def.field_offset(field)?;
                                            let ft = def.field_type(field)?.clone();
                                            Some((off, ft))
                                        })
                                    }
                                }
                                _ => None,
                            })
                            .unwrap_or((0, Type::Int64));
                        func.instruction(&Instruction::LocalGet(obj_idx));
                        func.instruction(&Instruction::I32Const(offset as i32));
                        func.instruction(&Instruction::I32Add);
                        self.compile_expr(value, locals, func, loop_ctx);
                        // 值类型与字段存储类型不同时（如 TypeParam→I32 存入 I64 字段），先做类型转换
                        let val_wasm_ty = self.infer_type_with_locals(value, locals);
                        self.emit_type_coercion(func, val_wasm_ty, field_ty.to_wasm());
                        self.emit_store_by_type(func, &field_ty);
                    }
                    AssignTarget::FieldPath { base, fields } => {
                        // base.field1.field2... = value：沿链累加偏移后存储
                        let obj_idx = locals.get(base).expect("对象未找到");
                        let mut total_offset: i32 = 0;
                        let mut current_ty = locals.get_type(base).cloned();
                        for f in fields.iter() {
                            let (off, next_ty) = current_ty
                                .as_ref()
                                .and_then(|ty| match ty {
                                    Type::Struct(name, type_args) => {
                                        let lookup_name = if !type_args.is_empty() {
                                            let mangled =
                                                crate::monomorph::mangle_name(name, type_args);
                                            if self.classes.contains_key(&mangled)
                                                || self.structs.contains_key(&mangled)
                                            {
                                                mangled
                                            } else {
                                                name.clone()
                                            }
                                        } else {
                                            name.clone()
                                        };
                                        if let Some(ci) = self.classes.get(&lookup_name) {
                                            ci.field_offset(f).and_then(|off| {
                                                ci.field_type(f).map(|ft| (off, ft.clone()))
                                            })
                                        } else {
                                            self.structs.get(&lookup_name).and_then(|def| {
                                                let off = def.field_offset(f)?;
                                                let ft = def.field_type(f)?.clone();
                                                Some((off, ft))
                                            })
                                        }
                                    }
                                    _ => None,
                                })
                                .unwrap_or((0, Type::Int64));
                            total_offset += off as i32;
                            current_ty = Some(next_ty);
                        }
                        let field_ty = current_ty.unwrap_or(Type::Int64);
                        func.instruction(&Instruction::LocalGet(obj_idx));
                        func.instruction(&Instruction::I32Const(total_offset));
                        func.instruction(&Instruction::I32Add);
                        self.compile_expr(value, locals, func, loop_ctx);
                        // 值类型与字段存储类型不同时（如 TypeParam→I32 存入 I64 字段），先做类型转换
                        let val_wasm_ty = self.infer_type_with_locals(value, locals);
                        self.emit_type_coercion(func, val_wasm_ty, field_ty.to_wasm());
                        self.emit_store_by_type(func, &field_ty);
                    }
                    AssignTarget::IndexPath {
                        base,
                        fields,
                        index,
                    } => {
                        // base.field1.field2[i] = value：编译链式字段得数组指针，再 +4+index*8 后存储
                        let mut arr_expr = Expr::Var(base.clone());
                        for f in fields {
                            arr_expr = Expr::Field {
                                object: Box::new(arr_expr),
                                field: f.clone(),
                            };
                        }
                        self.compile_expr(&arr_expr, locals, func, loop_ctx);
                        // 如果数组基址是 I64（未知类型回退），需要 wrap 为 I32
                        if self.infer_type_with_locals(&arr_expr, locals) == ValType::I64 {
                            func.instruction(&Instruction::I32WrapI64);
                        }
                        func.instruction(&Instruction::I32Const(4));
                        func.instruction(&Instruction::I32Add);
                        self.compile_expr(index, locals, func, loop_ctx);
                        // P3: 索引若为 I64（整数），wrap 为 I32 参与地址计算
                        if self.infer_type_with_locals(index, locals) == ValType::I64 {
                            func.instruction(&Instruction::I32WrapI64);
                        }
                        func.instruction(&Instruction::I32Const(8));
                        func.instruction(&Instruction::I32Mul);
                        func.instruction(&Instruction::I32Add);
                        self.compile_expr(value, locals, func, loop_ctx);
                        func.instruction(&Instruction::I32Const(8));
                        func.instruction(&Instruction::I32Mul);
                        func.instruction(&Instruction::I32Add);
                        self.compile_expr(value, locals, func, loop_ctx);
                        let val_ty = self.infer_type_with_locals(value, locals);
                        self.emit_type_coercion(func, val_ty, ValType::I64);
                        func.instruction(&Instruction::I64Store(wasm_encoder::MemArg {
                            offset: 0,
                            align: 3,
                            memory_index: 0,
                        }));
                    }
                    AssignTarget::ExprIndex { expr, index } => {
                        // expr[i] = value：编译表达式得数组指针，再 +4+index*8 后存储
                        self.compile_expr(expr, locals, func, loop_ctx);
                        // 如果数组基址是 I64（未知类型回退），需要 wrap 为 I32
                        if self.infer_type_with_locals(expr, locals) == ValType::I64 {
                            func.instruction(&Instruction::I32WrapI64);
                        }
                        func.instruction(&Instruction::I32Const(4));
                        func.instruction(&Instruction::I32Add);
                        self.compile_expr(index, locals, func, loop_ctx);
                        // P3: 索引若为 I64（整数），wrap 为 I32 参与地址计算
                        if self.infer_type_with_locals(index, locals) == ValType::I64 {
                            func.instruction(&Instruction::I32WrapI64);
                        }
                        func.instruction(&Instruction::I32Const(8));
                        func.instruction(&Instruction::I32Mul);
                        func.instruction(&Instruction::I32Add);
                        self.compile_expr(value, locals, func, loop_ctx);
                        func.instruction(&Instruction::I64Store(wasm_encoder::MemArg {
                            offset: 0,
                            align: 3,
                            memory_index: 0,
                        }));
                    }
                    AssignTarget::Tuple(ref targets) => {
                        // (a, b) = expr：先编译 expr，栈上得到元组各元素（最后元素在栈顶），逆序赋给各目标
                        self.compile_expr(value, locals, func, loop_ctx);
                        for t in targets.iter().rev() {
                            match t {
                                AssignTarget::Var(name) => {
                                    let idx = locals.get(name).expect("元组赋值目标变量未找到");
                                    func.instruction(&Instruction::LocalSet(idx));
                                }
                                _ => {
                                    func.instruction(&Instruction::Drop);
                                }
                            }
                        }
                    }
                    AssignTarget::SuperField { field } => {
                        // super.field = value：通过 this 指针访问父类字段
                        // 获取 this 指针
                        let this_idx = locals.get("this").expect("super 字段赋值需要 this");

                        // 从 this 的类型获取当前类名，再查找父类
                        let this_type = locals.get_type("this").expect("this 类型未找到");
                        let current_class_name = match this_type {
                            Type::Struct(name, _) => name,
                            _ => panic!("this 类型不是 Struct"),
                        };

                        // 从当前类获取父类名
                        let parent_class_name = self
                            .classes
                            .get(current_class_name)
                            .and_then(|ci| ci.parent.as_ref())
                            .expect(&format!("类 {} 没有父类", current_class_name));

                        // 从父类获取字段偏移和类型
                        let field_info_super = self.classes.get(parent_class_name).and_then(|ci| {
                            let off = ci.field_offset(field)?;
                            let ft = ci.field_type(field)?.clone();
                            Some((off, ft))
                        });

                        if field_info_super.is_none() {
                            // 字段未找到（可能是属性/property的setter）- 尝试作为 this 字段赋值
                            eprintln!("[警告] super.{} = value: 父类 {} 中未找到字段，尝试作为 this 字段赋值", field, parent_class_name);
                            let field_assign = Stmt::Assign {
                                target: AssignTarget::Field {
                                    object: "this".to_string(),
                                    field: field.clone(),
                                },
                                value: value.clone(),
                            };
                            self.compile_stmt(&field_assign, locals, func, loop_ctx);
                            return;
                        }
                        let (offset, field_ty) = field_info_super.unwrap();

                        func.instruction(&Instruction::LocalGet(this_idx));
                        self.compile_expr(value, locals, func, loop_ctx);

                        // 根据字段类型选择存储指令
                        match field_ty {
                            Type::Float64 => {
                                func.instruction(&Instruction::F64Store(wasm_encoder::MemArg {
                                    offset: offset as u64,
                                    align: 3,
                                    memory_index: 0,
                                }));
                            }
                            Type::Float32 => {
                                func.instruction(&Instruction::F32Store(wasm_encoder::MemArg {
                                    offset: offset as u64,
                                    align: 2,
                                    memory_index: 0,
                                }));
                            }
                            _ => {
                                func.instruction(&Instruction::I64Store(wasm_encoder::MemArg {
                                    offset: offset as u64,
                                    align: 3,
                                    memory_index: 0,
                                }));
                            }
                        }
                    }
                }
            }
            Stmt::Return(Some(expr)) => {
                self.compile_expr(expr, locals, func, loop_ctx);
                // 类型协调：仅对能确定实际类型的表达式做协调，避免误推断
                if let Some(expected) = self.current_return_wasm_type.get() {
                    let actual = match expr {
                        // Integer 字面量始终产生 I64
                        Expr::Integer(_) => Some(ValType::I64),
                        // Var：用 locals 中的实际 WASM 类型
                        Expr::Var(name) => locals.get_valtype(name),
                        // Bool 始终产生 I32
                        Expr::Bool(_) => Some(ValType::I32),
                        // 其他：通过完整推断进行协调
                        _ => {
                            let inferred = self.infer_type_with_locals(expr, locals);
                            // P5.5: 优先使用 AST 类型；若 AST 类型未知但 WASM 类型确定，也使用
                            // 注意：infer_type 的 I64 回退可能不准确，但此处
                            // 漏掉协调（不 wrap）比误插入 I32WrapI64 引发的错误更多
                            if self.infer_ast_type_with_locals(expr, locals).is_some() {
                                Some(inferred)
                            } else if inferred != ValType::I64 {
                                // 非 I64 回退值时，类型可确定（如 I32/F64 等）
                                Some(inferred)
                            } else {
                                // P5.5: infer_type 返回 I64 且 AST 不确定时，
                                // 仍使用 I64 以尝试协调（比完全跳过更安全）
                                Some(ValType::I64)
                            }
                        }
                    };
                    if let Some(actual) = actual {
                        self.emit_type_coercion(func, actual, expected);
                    }
                }
                func.instruction(&Instruction::Return);
            }
            Stmt::Return(None) => {
                func.instruction(&Instruction::Return);
            }
            Stmt::Expr(expr) => {
                self.compile_expr(expr, locals, func, loop_ctx);
                // 仅当表达式会在栈上产生值时才 drop
                if self.expr_produces_value(expr) {
                    func.instruction(&Instruction::Drop);
                }
            }
            Stmt::Break => {
                if let Some((break_depth, _)) = loop_ctx {
                    func.instruction(&Instruction::Br(break_depth));
                } else {
                    func.instruction(&Instruction::Unreachable);
                }
            }
            Stmt::Continue => {
                if let Some((_, continue_depth)) = loop_ctx {
                    func.instruction(&Instruction::Br(continue_depth));
                } else {
                    func.instruction(&Instruction::Unreachable);
                }
            }
            Stmt::Loop { body } => {
                func.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
                func.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
                let body_ctx = Some((1u32, 0u32));
                for s in body {
                    self.compile_stmt(s, locals, func, body_ctx);
                }
                func.instruction(&Instruction::Br(0));
                func.instruction(&Instruction::End);
                func.instruction(&Instruction::End);
            }
            Stmt::UnsafeBlock { body } => {
                for s in body {
                    self.compile_stmt(s, locals, func, loop_ctx);
                }
            }
            Stmt::While { cond, body } => {
                func.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
                func.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));

                self.compile_expr(cond, locals, func, loop_ctx);
                // 条件必须是 i32；仅当 AST 类型确认为 i64 时才 wrap（TypeParam 保守不 wrap）
                if self.needs_i64_to_i32_wrap(cond, locals)
                    && self.infer_type_with_locals(cond, locals) == ValType::I64
                {
                    func.instruction(&Instruction::I32WrapI64);
                }
                func.instruction(&Instruction::I32Eqz);
                func.instruction(&Instruction::BrIf(1));

                let body_ctx = Some((1u32, 0u32)); // break→block end, continue→loop start
                for s in body {
                    self.compile_stmt(s, locals, func, body_ctx);
                }

                func.instruction(&Instruction::Br(0));
                func.instruction(&Instruction::End);
                func.instruction(&Instruction::End);
            }
            Stmt::WhileLet {
                pattern,
                expr,
                body,
            } => {
                func.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty)); // Br(1) = break
                func.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty)); // Br(0) = continue
                self.compile_expr(expr, locals, func, loop_ctx);
                let subject_ty = self.infer_type_with_locals(expr, locals);
                let subject_ast_type = self.infer_ast_type_with_locals(expr, locals);
                let ptr_tmp = locals.get("__match_enum_ptr").expect("__match_enum_ptr");
                let body_ctx = Some((1u32, 0u32));

                match pattern {
                    Pattern::Binding(name) => {
                        if let Some(idx) = locals.get(name) {
                            if subject_ty == ValType::I32 {
                                func.instruction(&Instruction::I64ExtendI32S);
                            }
                            func.instruction(&Instruction::LocalSet(idx));
                        } else {
                            func.instruction(&Instruction::Drop);
                        }
                        for s in body {
                            self.compile_stmt(s, locals, func, body_ctx);
                        }
                        func.instruction(&Instruction::Br(0));
                    }
                    Pattern::Wildcard => {
                        func.instruction(&Instruction::Drop);
                        for s in body {
                            self.compile_stmt(s, locals, func, body_ctx);
                        }
                        func.instruction(&Instruction::Br(0));
                    }
                    Pattern::Variant {
                        enum_name,
                        variant_name,
                        payload,
                    } => {
                        let enum_def = self
                            .enums
                            .get(enum_name)
                            .and_then(|e| e.variant_index(variant_name).map(|_| e));
                        if let Some(enum_def) = enum_def {
                            func.instruction(&Instruction::LocalSet(ptr_tmp));
                            let expected_disc =
                                enum_def.variant_index(variant_name).unwrap() as i32;
                            let has_variant_payload = enum_def.has_payload();
                            let resolved_payload = self.resolve_variant_payload(
                                enum_name,
                                variant_name,
                                subject_ast_type.as_ref(),
                            );
                            func.instruction(&Instruction::LocalGet(ptr_tmp));
                            if has_variant_payload {
                                func.instruction(&Instruction::I32Load(wasm_encoder::MemArg {
                                    offset: 0,
                                    align: 2,
                                    memory_index: 0,
                                }));
                                func.instruction(&Instruction::I32Const(expected_disc));
                                func.instruction(&Instruction::I32Eq);
                            } else {
                                func.instruction(&Instruction::I32Const(expected_disc));
                                func.instruction(&Instruction::I32Eq);
                            }
                            func.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
                            if has_variant_payload {
                                if let Some(ref payload_pattern) = payload {
                                    if let Some(ref payload_ty) = resolved_payload {
                                        // 计算 payload 指针
                                        func.instruction(&Instruction::LocalGet(ptr_tmp));
                                        func.instruction(&Instruction::I32Const(4));
                                        func.instruction(&Instruction::I32Add);

                                        // 对于复合类型（元组、结构体），直接传递指针
                                        // 对于简单类型，加载值
                                        if matches!(payload_ty, Type::Tuple(_) | Type::Struct(_, _))
                                        {
                                            // 复合类型：指针已在栈上
                                            self.compile_pattern_binding(
                                                payload_pattern,
                                                payload_ty,
                                                locals,
                                                func,
                                            );
                                        } else {
                                            // 简单类型：加载值
                                            self.emit_load_by_type(func, payload_ty);
                                            self.compile_pattern_binding(
                                                payload_pattern,
                                                payload_ty,
                                                locals,
                                                func,
                                            );
                                        }
                                    }
                                }
                            }
                            for s in body {
                                self.compile_stmt(s, locals, func, body_ctx);
                            }
                            func.instruction(&Instruction::Br(0));
                            func.instruction(&Instruction::Else);
                            func.instruction(&Instruction::Br(1));
                            func.instruction(&Instruction::End);
                        } else {
                            func.instruction(&Instruction::Drop);
                            func.instruction(&Instruction::Br(1));
                        }
                    }
                    Pattern::Struct {
                        name: struct_name,
                        fields,
                    } => {
                        let handled = if let Some(Type::Struct(ref sub_name, _)) = subject_ast_type
                        {
                            sub_name == struct_name && self.structs.contains_key(struct_name)
                        } else {
                            false
                        };
                        if handled {
                            func.instruction(&Instruction::LocalSet(ptr_tmp));
                            let struct_def = &self.structs[struct_name];
                            for (fname, pat) in fields {
                                let offset = struct_def.field_offset(fname).expect("结构体字段");
                                let fty = struct_def.field_type(fname).expect("字段类型");
                                func.instruction(&Instruction::LocalGet(ptr_tmp));
                                func.instruction(&Instruction::I32Const(offset as i32));
                                func.instruction(&Instruction::I32Add);
                                self.emit_load_by_type(func, fty);
                                if let Pattern::Binding(bind) = pat {
                                    let idx = locals.get(bind).expect("解构绑定名");
                                    func.instruction(&Instruction::LocalSet(idx));
                                }
                            }
                            for s in body {
                                self.compile_stmt(s, locals, func, body_ctx);
                            }
                            func.instruction(&Instruction::Br(0));
                        } else {
                            func.instruction(&Instruction::Drop);
                            func.instruction(&Instruction::Br(1));
                        }
                    }
                    _ => {
                        func.instruction(&Instruction::Drop);
                        func.instruction(&Instruction::Br(1));
                    }
                }

                func.instruction(&Instruction::End);
                func.instruction(&Instruction::End);
            }
            Stmt::For {
                var,
                iterable,
                body,
            } => {
                // for i in 0..10 { ... } 编译为:
                // let i = start
                // while i < end { ...; i = i + 1 }
                let var_idx = locals.get(var).expect("循环变量未找到");

                match iterable {
                    Expr::Range {
                        start,
                        end,
                        inclusive,
                        step,
                    } => {
                        // 初始化循环变量
                        self.compile_expr(start, locals, func, loop_ctx);
                        func.instruction(&Instruction::LocalSet(var_idx));

                        // block { loop { block { body } increment; br 0 } }
                        func.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
                        func.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));

                        // 条件检查: i < end (或 i <= end)
                        func.instruction(&Instruction::LocalGet(var_idx));
                        self.compile_expr(end, locals, func, loop_ctx);
                        if *inclusive {
                            func.instruction(&Instruction::I64GtS); // i > end
                        } else {
                            func.instruction(&Instruction::I64GeS); // i >= end
                        }
                        func.instruction(&Instruction::BrIf(1)); // 退出外层 block

                        // 循环体用 block 包裹，使 continue 跳到增量步骤
                        func.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
                        // break=2 (outer block), continue=0 (exits body block → increment)
                        let body_ctx = Some((2u32, 0u32));
                        for s in body {
                            self.compile_stmt(s, locals, func, body_ctx);
                        }
                        func.instruction(&Instruction::End); // body block end

                        // 递增循环变量（P2.6: 支持步长）
                        func.instruction(&Instruction::LocalGet(var_idx));
                        if let Some(step_expr) = step {
                            self.compile_expr(step_expr, locals, func, loop_ctx);
                        } else {
                            func.instruction(&Instruction::I64Const(1));
                        }
                        func.instruction(&Instruction::I64Add);
                        func.instruction(&Instruction::LocalSet(var_idx));

                        func.instruction(&Instruction::Br(0)); // 继续循环 (target loop)
                        func.instruction(&Instruction::End); // loop end
                        func.instruction(&Instruction::End); // block end
                    }
                    _ => {
                        // 数组迭代: for item in arr { ... }
                        // 编译为:
                        //   let __arr = arr
                        //   let __len = arr[0]  (数组长度在偏移0)
                        //   let __idx = 0
                        //   while __idx < __len {
                        //     let item = arr[4 + __idx * 8]
                        //     ...
                        //     __idx += 1
                        //   }

                        let idx_var = format!("__{}_idx", var);
                        let len_var = format!("__{}_len", var);
                        let arr_var = format!("__{}_arr", var);

                        let idx_idx = locals.get(&idx_var).expect("索引变量未找到");
                        let len_idx = locals.get(&len_var).expect("长度变量未找到");
                        let arr_idx = locals.get(&arr_var).expect("数组变量未找到");

                        // 计算数组地址并保存
                        self.compile_expr(iterable, locals, func, loop_ctx);
                        func.instruction(&Instruction::LocalSet(arr_idx));

                        // 获取数组长度 (在偏移0处)
                        func.instruction(&Instruction::LocalGet(arr_idx));
                        func.instruction(&Instruction::I32Load(wasm_encoder::MemArg {
                            offset: 0,
                            align: 2,
                            memory_index: 0,
                        }));
                        func.instruction(&Instruction::I64ExtendI32S);
                        func.instruction(&Instruction::LocalSet(len_idx));

                        // 初始化索引为 0
                        func.instruction(&Instruction::I64Const(0));
                        func.instruction(&Instruction::LocalSet(idx_idx));

                        // block { loop { ... } }
                        func.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
                        func.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));

                        // 条件检查: __idx >= __len 则退出
                        func.instruction(&Instruction::LocalGet(idx_idx));
                        func.instruction(&Instruction::LocalGet(len_idx));
                        func.instruction(&Instruction::I64GeS);
                        func.instruction(&Instruction::BrIf(1));

                        // 获取当前元素: arr[4 + idx * 8]
                        func.instruction(&Instruction::LocalGet(arr_idx));
                        func.instruction(&Instruction::I32Const(4)); // 跳过长度字段
                        func.instruction(&Instruction::I32Add);
                        func.instruction(&Instruction::LocalGet(idx_idx));
                        func.instruction(&Instruction::I32WrapI64);
                        func.instruction(&Instruction::I32Const(8)); // 元素大小
                        func.instruction(&Instruction::I32Mul);
                        func.instruction(&Instruction::I32Add);
                        func.instruction(&Instruction::I64Load(wasm_encoder::MemArg {
                            offset: 0,
                            align: 3,
                            memory_index: 0,
                        }));
                        func.instruction(&Instruction::LocalSet(var_idx));

                        // 循环体用 block 包裹，使 continue 跳到增量步骤
                        func.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
                        // break=2 (outer block), continue=0 (exits body block → increment)
                        let body_ctx = Some((2u32, 0u32));
                        for s in body {
                            self.compile_stmt(s, locals, func, body_ctx);
                        }
                        func.instruction(&Instruction::End); // body block end

                        // 递增索引
                        func.instruction(&Instruction::LocalGet(idx_idx));
                        func.instruction(&Instruction::I64Const(1));
                        func.instruction(&Instruction::I64Add);
                        func.instruction(&Instruction::LocalSet(idx_idx));

                        func.instruction(&Instruction::Br(0)); // 继续循环 (target loop)
                        func.instruction(&Instruction::End); // loop end
                        func.instruction(&Instruction::End); // block end
                    }
                }
            }
            Stmt::Assert { left, right, line } => {
                // @Assert(a, b): 如果 a != b 则打印错误信息并终止
                self.compile_assert_expect(left, right, *line, true, locals, func, loop_ctx);
            }
            Stmt::Expect { left, right, line } => {
                // @Expect(a, b): 如果 a != b 则打印错误信息但继续执行
                self.compile_assert_expect(left, right, *line, false, locals, func, loop_ctx);
            }
            Stmt::DoWhile { body, cond } => {
                // do { body } while (cond)
                // WASM: block { loop { body; cond; br_if 0 (loop); } }
                func.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
                func.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));

                let body_ctx = Some((1u32, 0u32)); // break→block end, continue→loop start
                for s in body {
                    self.compile_stmt(s, locals, func, body_ctx);
                }

                self.compile_expr(cond, locals, func, loop_ctx);
                if self.needs_i64_to_i32_wrap(cond, locals) {
                    func.instruction(&Instruction::I32WrapI64);
                }
                func.instruction(&Instruction::BrIf(0)); // continue looping if true

                func.instruction(&Instruction::End); // loop end
                func.instruction(&Instruction::End); // block end
            }
            Stmt::Const { name, ty, value } => {
                // const 声明语义等同于 let（不可变绑定），在 WASM 中无区别
                self.compile_expr(value, locals, func, loop_ctx);
                if let Some(idx) = locals.get(name) {
                    let val_ty = self.infer_type_with_locals(value, locals);
                    let target_ty = ty.as_ref().map(|t| t.to_wasm()).unwrap_or(val_ty);
                    if val_ty != target_ty {
                        if val_ty == ValType::I64 && target_ty == ValType::I32 {
                            func.instruction(&Instruction::I32WrapI64);
                        } else if val_ty == ValType::I32 && target_ty == ValType::I64 {
                            func.instruction(&Instruction::I64ExtendI32S);
                        }
                    }
                    func.instruction(&Instruction::LocalSet(idx));
                } else {
                    if self.expr_produces_value(value) {
                        func.instruction(&Instruction::Drop);
                    }
                }
            }
        }
    }

    /// 编译 @Assert / @Expect 语句
    /// is_assert=true → 失败时 unreachable (fail-fast)
    /// is_assert=false → 失败时仅打印 (continue)
    fn compile_assert_expect(
        &self,
        left: &Expr,
        right: &Expr,
        byte_offset: usize,
        is_assert: bool,
        locals: &LocalsBuilder,
        func: &mut WasmFunc,
        loop_ctx: Option<(u32, u32)>,
    ) {
        // block $ok
        //   <compile left>
        //   <compile right>
        //   <compare eq>
        //   br_if $ok       ;; 相等则跳过
        //   ;; 失败路径
        //   <print error message>
        //   unreachable     ;; (仅 @Assert)
        // end

        func.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));

        // 编译两个表达式并比较
        let left_vt = self.infer_type_with_locals(left, locals);
        let right_vt = self.infer_type_with_locals(right, locals);
        self.compile_expr(left, locals, func, loop_ctx);

        // 类型协调：如果左右类型不同，将窄类型扩展为宽类型
        if left_vt == ValType::I32 && right_vt == ValType::I64 {
            func.instruction(&Instruction::I64ExtendI32S);
        }

        self.compile_expr(right, locals, func, loop_ctx);

        if left_vt == ValType::I64 && right_vt == ValType::I32 {
            func.instruction(&Instruction::I64ExtendI32S);
        }
        if left_vt == ValType::F32 && right_vt == ValType::F64 {
            func.instruction(&Instruction::F64PromoteF32);
        }

        // 确定最终比较类型
        let cmp_vt = if left_vt == ValType::F64 || right_vt == ValType::F64 {
            ValType::F64
        } else if left_vt == ValType::F32 && right_vt == ValType::F32 {
            ValType::F32
        } else if left_vt == ValType::I32 && right_vt == ValType::I32 {
            ValType::I32
        } else {
            ValType::I64
        };

        // 根据类型选择比较指令
        match cmp_vt {
            ValType::F64 => {
                func.instruction(&Instruction::F64Eq);
            }
            ValType::F32 => {
                func.instruction(&Instruction::F32Eq);
            }
            ValType::I32 => {
                func.instruction(&Instruction::I32Eq);
            }
            _ => {
                func.instruction(&Instruction::I64Eq);
            }
        }

        func.instruction(&Instruction::BrIf(0)); // 相等则跳到 block 结尾

        // --- 失败路径 ---
        // 构建错误消息字符串: "ASSERT FAILED: line N\n" 或 "EXPECT FAILED: line N\n"
        let macro_name = if is_assert { "ASSERT" } else { "EXPECT" };
        // byte_offset 作为近似行号（实际是字节偏移，但在错误报告中足够识别位置）
        let msg = format!("{} FAILED: offset {}\n", macro_name, byte_offset);
        let msg_bytes = msg.as_bytes();
        let msg_len = msg_bytes.len() as i32;

        // 在内存中写入字符串: [len: i32][bytes...]
        // 使用 WASI scratch 区域后面的空间（偏移 96 起）
        let str_base: i32 = 96;
        // 写入长度
        func.instruction(&Instruction::I32Const(str_base));
        func.instruction(&Instruction::I32Const(msg_len));
        func.instruction(&Instruction::I32Store(wasm_encoder::MemArg {
            offset: 0,
            align: 2,
            memory_index: 0,
        }));
        // 写入每个字节
        for (i, &byte) in msg_bytes.iter().enumerate() {
            func.instruction(&Instruction::I32Const(str_base + 4 + i as i32));
            func.instruction(&Instruction::I32Const(byte as i32));
            func.instruction(&Instruction::I32Store8(wasm_encoder::MemArg {
                offset: 0,
                align: 0,
                memory_index: 0,
            }));
        }

        // 调用 __println_str 或 __eprintln_str (如果有) 打印到 stderr
        // 使用 stderr 打印: 复用 __eprintln_str 如果存在，否则用 __println_str
        if let Some(&idx) = self.func_indices.get("__eprintln_str") {
            func.instruction(&Instruction::I32Const(str_base));
            func.instruction(&Instruction::Call(idx));
        } else if let Some(&idx) = self.func_indices.get("__println_str") {
            func.instruction(&Instruction::I32Const(str_base));
            func.instruction(&Instruction::Call(idx));
        }

        if is_assert {
            // @Assert: 立即终止
            // 尝试用 proc_exit(1)，否则 unreachable
            if let Some(&exit_idx) = self.func_indices.get("__wasi_proc_exit") {
                func.instruction(&Instruction::I32Const(1));
                func.instruction(&Instruction::Call(exit_idx));
            }
            func.instruction(&Instruction::Unreachable);
        }
        // @Expect: 不终止，直接 fall through 到 block end

        func.instruction(&Instruction::End); // block end
    }

    /// 编译表达式并进行类型协调（如果需要）
    pub(crate) fn compile_expr_with_coercion(
        &self,
        expr: &Expr,
        expected_ty: Option<ValType>,
        locals: &LocalsBuilder,
        func: &mut WasmFunc,
        loop_ctx: Option<(u32, u32)>,
    ) {
        self.compile_expr(expr, locals, func, loop_ctx);

        if let Some(exp) = expected_ty {
            let actual_ty = self.infer_type_with_locals(expr, locals);
            if actual_ty != exp {
                self.emit_type_coercion(func, actual_ty, exp);
            }
        }
    }

    pub(crate) fn compile_expr(
        &self,
        expr: &Expr,
        locals: &LocalsBuilder,
        func: &mut WasmFunc,
        loop_ctx: Option<(u32, u32)>,
    ) {
        match expr {
            Expr::Integer(n) => {
                func.instruction(&Instruction::I64Const(*n));
            }
            Expr::Float32(f) => {
                func.instruction(&Instruction::F32Const(*f));
            }
            Expr::Float(f) => {
                func.instruction(&Instruction::F64Const(*f));
            }
            Expr::Bool(b) => {
                func.instruction(&Instruction::I32Const(if *b { 1 } else { 0 }));
            }
            Expr::Rune(c) => {
                func.instruction(&Instruction::I32Const(*c as i32));
            }
            Expr::String(s) => {
                // 返回字符串在数据段中的地址
                let offset = self
                    .string_pool
                    .iter()
                    .find(|(str, _)| str == s)
                    .map(|(_, off)| *off)
                    .unwrap_or(0);
                func.instruction(&Instruction::I32Const(offset as i32));
            }
            Expr::Interpolate(parts) => {
                // 字符串插值：逐部分分配并拼接
                // 简化实现：在堆上构建最终字符串
                // 首先计算总长度，然后分配并复制

                if parts.is_empty() {
                    // 空插值返回空字符串
                    let empty_offset = self
                        .string_pool
                        .iter()
                        .find(|(s, _)| s.is_empty())
                        .map(|(_, off)| *off)
                        .unwrap_or(0);
                    func.instruction(&Instruction::I32Const(empty_offset as i32));
                    return;
                }

                // 将每个部分编译为字符串指针，压入栈
                // 策略：使用 __str_concat 运行时函数逐个拼接
                // 生成：part1 -> __concat(part1, part2) -> __concat(result, part3) -> ...

                let mut is_first = true;
                for part in parts {
                    match part {
                        InterpolatePart::Literal(text) => {
                            // 获取字面量字符串的地址
                            let offset = self
                                .string_pool
                                .iter()
                                .find(|(s, _)| s == text)
                                .map(|(_, off)| *off)
                                .unwrap_or_else(|| {
                                    // 如果字符串不在池中，添加它
                                    // 注意：这里简化处理，实际应该在编译前收集所有字符串
                                    0
                                });
                            func.instruction(&Instruction::I32Const(offset as i32));
                        }
                        InterpolatePart::Expr(expr) => {
                            // 编译表达式
                            self.compile_expr(expr, locals, func, loop_ctx);
                            // 将值转换为字符串（调用 __to_string_TYPE 运行时函数）
                            let expr_type = self.infer_ast_type_with_locals(expr, locals);
                            match expr_type.as_ref() {
                                Some(Type::Int64) => {
                                    func.instruction(&Instruction::Call(
                                        self.get_or_create_func_index("__i64_to_str"),
                                    ));
                                }
                                Some(Type::Int32) => {
                                    func.instruction(&Instruction::Call(
                                        self.get_or_create_func_index("__i32_to_str"),
                                    ));
                                }
                                Some(Type::Float64) => {
                                    func.instruction(&Instruction::Call(
                                        self.get_or_create_func_index("__f64_to_str"),
                                    ));
                                }
                                Some(Type::Float32) => {
                                    func.instruction(&Instruction::Call(
                                        self.get_or_create_func_index("__f32_to_str"),
                                    ));
                                }
                                Some(Type::Int8) | Some(Type::Int16) | Some(Type::UInt8)
                                | Some(Type::UInt16) | Some(Type::UInt32) | Some(Type::Rune) => {
                                    func.instruction(&Instruction::Call(
                                        self.get_or_create_func_index("__i32_to_str"),
                                    ));
                                }
                                Some(Type::UInt64) => {
                                    func.instruction(&Instruction::Call(
                                        self.get_or_create_func_index("__i64_to_str"),
                                    ));
                                }
                                Some(Type::Bool) => {
                                    func.instruction(&Instruction::Call(
                                        self.get_or_create_func_index("__bool_to_str"),
                                    ));
                                }
                                Some(Type::String) => {
                                    // 已经是字符串，不需要转换
                                }
                                Some(Type::Struct(ref sname, _)) => {
                                    // Phase 7.1 #42: struct/class 有 toString() 则调用
                                    let ts_key = format!("{}.toString", sname);
                                    if self.func_indices.contains_key(&ts_key) {
                                        func.instruction(&Instruction::Call(
                                            self.func_indices[&ts_key],
                                        ));
                                        // toString() 返回 i32 (字符串指针), 已是字符串
                                    } else {
                                        // 无 toString 方法，输出 "[object]"
                                        func.instruction(&Instruction::Drop);
                                        let obj_str = self
                                            .string_pool
                                            .iter()
                                            .find(|(s, _)| s == "[object]")
                                            .map(|(_, off)| *off)
                                            .unwrap_or(0);
                                        func.instruction(&Instruction::I32Const(obj_str as i32));
                                    }
                                }
                                _ => {
                                    // 其他类型暂时转为 "[object]"
                                    func.instruction(&Instruction::Drop);
                                    let obj_str = self
                                        .string_pool
                                        .iter()
                                        .find(|(s, _)| s == "[object]")
                                        .map(|(_, off)| *off)
                                        .unwrap_or(0);
                                    func.instruction(&Instruction::I32Const(obj_str as i32));
                                }
                            }
                        }
                    }

                    if !is_first {
                        // 拼接前一个结果和当前部分
                        func.instruction(&Instruction::Call(
                            self.get_or_create_func_index("__str_concat"),
                        ));
                    }
                    is_first = false;
                }
            }
            Expr::Var(name) => {
                // Phase 7.3: math 常数
                match name.as_str() {
                    "PI" if locals.get(name).is_none() => {
                        func.instruction(&Instruction::F64Const(std::f64::consts::PI));
                    }
                    "E" if locals.get(name).is_none() => {
                        func.instruction(&Instruction::F64Const(std::f64::consts::E));
                    }
                    "INF" | "INFINITY" if locals.get(name).is_none() => {
                        func.instruction(&Instruction::F64Const(f64::INFINITY));
                    }
                    "NEG_INF" | "NEG_INFINITY" if locals.get(name).is_none() => {
                        func.instruction(&Instruction::F64Const(f64::NEG_INFINITY));
                    }
                    "NAN" if locals.get(name).is_none() => {
                        func.instruction(&Instruction::F64Const(f64::NAN));
                    }
                    "TAU" if locals.get(name).is_none() => {
                        func.instruction(&Instruction::F64Const(std::f64::consts::TAU));
                    }
                    _ => {
                        if let Some(idx) = locals.get(name) {
                            func.instruction(&Instruction::LocalGet(idx));
                        } else if let Some(this_idx) = locals.get("this") {
                            // Bug B2 修复: 隐式 this 字段访问 — 将 `field` 解析为 `this.field`
                            let this_field = Expr::Field {
                                object: Box::new(Expr::Var("this".to_string())),
                                field: name.clone(),
                            };
                            self.compile_expr(&this_field, locals, func, loop_ctx);
                        } else {
                            // 全局 let/const 变量或未解析符号：发出零值占位符
                            eprintln!("警告: 变量未找到: '{}', 使用零值占位", name);
                            func.instruction(&Instruction::I32Const(0));
                        }
                    }
                }
            }
            Expr::Unary {
                op: UnaryOp::Not,
                expr,
            } => {
                self.compile_expr(expr, locals, func, loop_ctx);
                if self.needs_i64_to_i32_wrap(expr, locals) {
                    func.instruction(&Instruction::I32WrapI64);
                }
                func.instruction(&Instruction::I32Eqz);
            }
            Expr::Unary {
                op: UnaryOp::BitNot,
                expr,
            } => {
                self.compile_expr(expr, locals, func, loop_ctx);
                let ty = self.infer_type_with_locals(expr, locals);
                match ty {
                    ValType::I64 => {
                        func.instruction(&Instruction::I64Const(-1));
                        func.instruction(&Instruction::I64Xor);
                    }
                    ValType::I32 => {
                        func.instruction(&Instruction::I32Const(-1));
                        func.instruction(&Instruction::I32Xor);
                    }
                    _ => panic!("~ 仅支持整数类型"),
                }
            }
            Expr::Unary {
                op: UnaryOp::Neg,
                expr,
            } => {
                let ty = self.infer_type_with_locals(expr, locals);
                match ty {
                    ValType::I32 => {
                        func.instruction(&Instruction::I32Const(0));
                        self.compile_expr(expr, locals, func, loop_ctx);
                        func.instruction(&Instruction::I32Sub);
                    }
                    ValType::I64 => {
                        func.instruction(&Instruction::I64Const(0));
                        self.compile_expr(expr, locals, func, loop_ctx);
                        func.instruction(&Instruction::I64Sub);
                    }
                    ValType::F64 => {
                        func.instruction(&Instruction::F64Const(0.0));
                        self.compile_expr(expr, locals, func, loop_ctx);
                        func.instruction(&Instruction::F64Sub);
                    }
                    ValType::F32 => {
                        func.instruction(&Instruction::F32Const(0.0));
                        self.compile_expr(expr, locals, func, loop_ctx);
                        func.instruction(&Instruction::F32Sub);
                    }
                    ValType::V128 | ValType::Ref(_) => panic!("不支持一元负号: {:?}", ty),
                }
            }
            Expr::Binary {
                op: BinOp::LogicalAnd,
                left,
                right,
            } => {
                // 短路与：left && right，结果为 i32 (0/1)
                self.compile_expr(left, locals, func, loop_ctx);
                if self.needs_i64_to_i32_wrap(left, locals) {
                    func.instruction(&Instruction::I32WrapI64);
                }
                func.instruction(&Instruction::I32Eqz);
                func.instruction(&Instruction::If(wasm_encoder::BlockType::Result(
                    ValType::I32,
                )));
                func.instruction(&Instruction::I32Const(0));
                func.instruction(&Instruction::Else);
                self.compile_expr(right, locals, func, loop_ctx);
                if self.needs_i64_to_i32_wrap(right, locals) {
                    func.instruction(&Instruction::I32WrapI64);
                }
                func.instruction(&Instruction::I32Eqz);
                func.instruction(&Instruction::I32Const(1));
                func.instruction(&Instruction::I32Sub);
                func.instruction(&Instruction::End);
            }
            Expr::Binary {
                op: BinOp::LogicalOr,
                left,
                right,
            } => {
                // 短路或：left || right，用 __logical_tmp 保存 left，结果为 i32 (0/1)
                let tmp = locals.get("__logical_tmp").expect("__logical_tmp 未找到");
                self.compile_expr(left, locals, func, loop_ctx);
                if self.needs_i64_to_i32_wrap(left, locals) {
                    func.instruction(&Instruction::I32WrapI64);
                }
                func.instruction(&Instruction::LocalSet(tmp));
                func.instruction(&Instruction::LocalGet(tmp));
                func.instruction(&Instruction::I32Eqz);
                func.instruction(&Instruction::If(wasm_encoder::BlockType::Result(
                    ValType::I32,
                )));
                self.compile_expr(right, locals, func, loop_ctx);
                if self.needs_i64_to_i32_wrap(right, locals) {
                    func.instruction(&Instruction::I32WrapI64);
                }
                func.instruction(&Instruction::I32Eqz);
                func.instruction(&Instruction::I32Const(1));
                func.instruction(&Instruction::I32Sub);
                func.instruction(&Instruction::Else);
                func.instruction(&Instruction::LocalGet(tmp));
                func.instruction(&Instruction::I32Eqz);
                func.instruction(&Instruction::I32Const(1));
                func.instruction(&Instruction::I32Sub);
                func.instruction(&Instruction::End);
            }
            // P6: !in 运算符 — 编译为 contains 方法调用 + 取反
            Expr::Binary {
                op: BinOp::NotIn,
                left,
                right,
            } => {
                // a !in b => !(b.contains(a))
                let contains_call = Expr::MethodCall {
                    object: right.clone(),
                    method: "contains".to_string(),
                    args: vec![left.as_ref().clone()],
                    named_args: vec![],
                    type_args: None,
                };
                self.compile_expr(&contains_call, locals, func, loop_ctx);
                // contains 返回值可能是 i64（HashSet/HashMap）或 i32（String），需统一到 i32 后取反
                if self.needs_i64_to_i32_wrap(&contains_call, locals) {
                    func.instruction(&Instruction::I32WrapI64);
                }
                func.instruction(&Instruction::I32Eqz);
            }
            Expr::Binary {
                op: BinOp::Pipeline,
                left,
                right,
            } => {
                // 管道 left |> right => right(left)：先求值 left（作为第一参数），再调用 right
                self.compile_expr(left, locals, func, loop_ctx);
                match right.as_ref() {
                    Expr::Var(callee) => {
                        if let Some(&idx) = self.func_indices.get(callee.as_str()) {
                            func.instruction(&Instruction::Call(idx));
                            return;
                        }
                    }
                    Expr::Call {
                        name,
                        type_args: _,
                        args,
                        named_args,
                        ..
                    } => {
                        // left |> map(fn) => 栈上已有 left，再压入 args，再 call name
                        let args = if named_args.is_empty() {
                            std::borrow::Cow::Borrowed(args)
                        } else {
                            std::borrow::Cow::Owned(self.resolve_named_args(name, args, named_args))
                        };
                        for arg in args.iter() {
                            self.compile_expr(arg, locals, func, loop_ctx);
                        }
                        if let Some(&idx) = self.func_indices.get(name.as_str()) {
                            func.instruction(&Instruction::Call(idx));
                            return;
                        }
                    }
                    _ => {}
                }
                // 其它 right 暂不处理，回退可能报错
                self.compile_expr(right, locals, func, loop_ctx);
                return;
            }
            Expr::Binary { op, left, right } => {
                // P3.1: 运算符重载 — 检查左操作数类型是否有 operator func
                if let Some(left_ty) = self.infer_ast_type_with_locals(left, locals) {
                    let op_name = match op {
                        BinOp::Add => "op_add",
                        BinOp::Sub => "op_sub",
                        BinOp::Mul => "op_mul",
                        BinOp::Div => "op_div",
                        BinOp::Mod => "op_mod",
                        BinOp::Eq => "op_eq",
                        BinOp::NotEq => "op_ne",
                        BinOp::Lt => "op_lt",
                        BinOp::Gt => "op_gt",
                        BinOp::LtEq => "op_le",
                        BinOp::GtEq => "op_ge",
                        _ => "",
                    };
                    if !op_name.is_empty() {
                        let type_name = match &left_ty {
                            Type::Struct(n, _) => Some(n.clone()),
                            _ => None,
                        };
                        if let Some(tn) = type_name {
                            let qualified = format!("{}.{}", tn, op_name);
                            if let Some(&idx) = self.func_indices.get(&qualified) {
                                // 调用 operator 方法: TypeName.op_xxx(this, other)
                                self.compile_expr(left, locals, func, loop_ctx);
                                self.compile_expr(right, locals, func, loop_ctx);
                                func.instruction(&Instruction::Call(idx));
                                return;
                            }
                        }
                    }
                }
                if op == &BinOp::Pow {
                    self.compile_expr(left, locals, func, loop_ctx);
                    self.compile_expr(right, locals, func, loop_ctx);
                    let idx = *self.func_indices.get("__pow_i64").unwrap();
                    func.instruction(&Instruction::Call(idx));
                    return;
                }
                // Bug B4 修复: String `+` 应调用 __str_concat 而非 I32Add
                if op == &BinOp::Add {
                    let left_ast = self.infer_ast_type_with_locals(left, locals);
                    let right_ast = self.infer_ast_type_with_locals(right, locals);
                    if left_ast == Some(Type::String) || right_ast == Some(Type::String) {
                        // 如果一侧不是字符串，先转为字符串
                        self.compile_expr(left, locals, func, loop_ctx);
                        if left_ast != Some(Type::String) {
                            self.emit_to_string(func, left_ast.as_ref());
                        }
                        self.compile_expr(right, locals, func, loop_ctx);
                        if right_ast != Some(Type::String) {
                            self.emit_to_string(func, right_ast.as_ref());
                        }
                        let concat_idx = self.func_indices["__str_concat"];
                        func.instruction(&Instruction::Call(concat_idx));
                        return;
                    }
                }
                self.compile_expr(left, locals, func, loop_ctx);
                self.compile_expr(right, locals, func, loop_ctx);

                // 类型匹配修正：如果左侧是 i32 但右侧确认为 i64（整数字面量或 Int64/UInt64 类型），
                // 在右值后插入 i32.wrap_i64；反之插入 i64.extend_i32_s。
                // 注意：TypeParam 在 infer_type_with_locals 中返回 I64 但实际为 I32，
                // 必须使用 needs_i64_to_i32_wrap 来确认是否真正为 I64（避免在 I32 上调 I32WrapI64）。
                let left_wasm_ty = self.infer_type_with_locals(left, locals);
                let right_wasm_ty = self.infer_type_with_locals(right, locals);
                if left_wasm_ty == ValType::I32 && self.needs_i64_to_i32_wrap(right, locals) {
                    func.instruction(&Instruction::I32WrapI64);
                } else if left_wasm_ty == ValType::I64 && right_wasm_ty == ValType::I32 {
                    func.instruction(&Instruction::I64ExtendI32S);
                }

                // 检查是否为无符号类型，以选择无符号除法/比较指令
                let ast_ty = self.infer_ast_type_with_locals(left, locals);
                let is_unsigned = matches!(
                    ast_ty.as_ref(),
                    Some(Type::UInt8)
                        | Some(Type::UInt16)
                        | Some(Type::UInt32)
                        | Some(Type::UInt64)
                );

                let val_type = self.infer_type_with_locals(left, locals);

                // 无符号类型需要使用无符号指令
                if is_unsigned {
                    let instr = match (op, val_type) {
                        (BinOp::Div, ValType::I32) => Instruction::I32DivU,
                        (BinOp::Mod, ValType::I32) => Instruction::I32RemU,
                        (BinOp::Lt, ValType::I32) => Instruction::I32LtU,
                        (BinOp::Gt, ValType::I32) => Instruction::I32GtU,
                        (BinOp::LtEq, ValType::I32) => Instruction::I32LeU,
                        (BinOp::GtEq, ValType::I32) => Instruction::I32GeU,
                        (BinOp::Shr, ValType::I32) => Instruction::I32ShrU,
                        (BinOp::Div, ValType::I64) => Instruction::I64DivU,
                        (BinOp::Mod, ValType::I64) => Instruction::I64RemU,
                        (BinOp::Lt, ValType::I64) => Instruction::I64LtU,
                        (BinOp::Gt, ValType::I64) => Instruction::I64GtU,
                        (BinOp::LtEq, ValType::I64) => Instruction::I64LeU,
                        (BinOp::GtEq, ValType::I64) => Instruction::I64GeU,
                        (BinOp::Shr, ValType::I64) => Instruction::I64ShrU,
                        _ => {
                            // 对于 Add/Sub/Mul/Eq/NotEq 等，有符号和无符号相同
                            match (op, val_type) {
                                (BinOp::Add, ValType::I32) => Instruction::I32Add,
                                (BinOp::Sub, ValType::I32) => Instruction::I32Sub,
                                (BinOp::Mul, ValType::I32) => Instruction::I32Mul,
                                (BinOp::Eq, ValType::I32) => Instruction::I32Eq,
                                (BinOp::NotEq, ValType::I32) => Instruction::I32Ne,
                                (BinOp::BitAnd, ValType::I32) => Instruction::I32And,
                                (BinOp::BitOr, ValType::I32) => Instruction::I32Or,
                                (BinOp::BitXor, ValType::I32) => Instruction::I32Xor,
                                (BinOp::Shl, ValType::I32) => Instruction::I32Shl,
                                (BinOp::Add, ValType::I64) => Instruction::I64Add,
                                (BinOp::Sub, ValType::I64) => Instruction::I64Sub,
                                (BinOp::Mul, ValType::I64) => Instruction::I64Mul,
                                (BinOp::Eq, ValType::I64) => Instruction::I64Eq,
                                (BinOp::NotEq, ValType::I64) => Instruction::I64Ne,
                                (BinOp::BitAnd, ValType::I64) => Instruction::I64And,
                                (BinOp::BitOr, ValType::I64) => Instruction::I64Or,
                                (BinOp::BitXor, ValType::I64) => Instruction::I64Xor,
                                (BinOp::Shl, ValType::I64) => Instruction::I64Shl,
                                _ => panic!("不支持的无符号运算: {:?} for {:?}", op, val_type),
                            }
                        }
                    };
                    func.instruction(&instr);

                    // UInt8/UInt16 掩码
                    match ast_ty.as_ref() {
                        Some(Type::UInt8) => match op {
                            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => {
                                func.instruction(&Instruction::I32Const(0xFF));
                                func.instruction(&Instruction::I32And);
                            }
                            _ => {}
                        },
                        Some(Type::UInt16) => match op {
                            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => {
                                func.instruction(&Instruction::I32Const(0xFFFF));
                                func.instruction(&Instruction::I32And);
                            }
                            _ => {}
                        },
                        _ => {}
                    }
                    return;
                }

                let instr = match (op, val_type) {
                    (BinOp::Add, ValType::I64) => Instruction::I64Add,
                    (BinOp::Sub, ValType::I64) => Instruction::I64Sub,
                    (BinOp::Mul, ValType::I64) => Instruction::I64Mul,
                    (BinOp::Div, ValType::I64) => Instruction::I64DivS,
                    (BinOp::Mod, ValType::I64) => Instruction::I64RemS,
                    (BinOp::Lt, ValType::I64) => Instruction::I64LtS,
                    (BinOp::Gt, ValType::I64) => Instruction::I64GtS,
                    (BinOp::LtEq, ValType::I64) => Instruction::I64LeS,
                    (BinOp::GtEq, ValType::I64) => Instruction::I64GeS,
                    (BinOp::Eq, ValType::I64) => Instruction::I64Eq,
                    (BinOp::NotEq, ValType::I64) => Instruction::I64Ne,

                    (BinOp::Add, ValType::I32) => Instruction::I32Add,
                    (BinOp::Sub, ValType::I32) => Instruction::I32Sub,
                    (BinOp::Mul, ValType::I32) => Instruction::I32Mul,
                    (BinOp::Div, ValType::I32) => Instruction::I32DivS,
                    (BinOp::Mod, ValType::I32) => Instruction::I32RemS,
                    (BinOp::Lt, ValType::I32) => Instruction::I32LtS,
                    (BinOp::Gt, ValType::I32) => Instruction::I32GtS,
                    (BinOp::LtEq, ValType::I32) => Instruction::I32LeS,
                    (BinOp::GtEq, ValType::I32) => Instruction::I32GeS,
                    (BinOp::Eq, ValType::I32) => Instruction::I32Eq,
                    (BinOp::NotEq, ValType::I32) => Instruction::I32Ne,

                    (BinOp::Add, ValType::F64) => Instruction::F64Add,
                    (BinOp::Sub, ValType::F64) => Instruction::F64Sub,
                    (BinOp::Mul, ValType::F64) => Instruction::F64Mul,
                    (BinOp::Div, ValType::F64) => Instruction::F64Div,
                    (BinOp::Lt, ValType::F64) => Instruction::F64Lt,
                    (BinOp::Gt, ValType::F64) => Instruction::F64Gt,
                    (BinOp::LtEq, ValType::F64) => Instruction::F64Le,
                    (BinOp::GtEq, ValType::F64) => Instruction::F64Ge,
                    (BinOp::Eq, ValType::F64) => Instruction::F64Eq,
                    (BinOp::NotEq, ValType::F64) => Instruction::F64Ne,

                    (BinOp::Add, ValType::F32) => Instruction::F32Add,
                    (BinOp::Sub, ValType::F32) => Instruction::F32Sub,
                    (BinOp::Mul, ValType::F32) => Instruction::F32Mul,
                    (BinOp::Div, ValType::F32) => Instruction::F32Div,
                    (BinOp::Lt, ValType::F32) => Instruction::F32Lt,
                    (BinOp::Gt, ValType::F32) => Instruction::F32Gt,
                    (BinOp::LtEq, ValType::F32) => Instruction::F32Le,
                    (BinOp::GtEq, ValType::F32) => Instruction::F32Ge,
                    (BinOp::Eq, ValType::F32) => Instruction::F32Eq,
                    (BinOp::NotEq, ValType::F32) => Instruction::F32Ne,

                    (BinOp::BitAnd, ValType::I64) => Instruction::I64And,
                    (BinOp::BitOr, ValType::I64) => Instruction::I64Or,
                    (BinOp::BitXor, ValType::I64) => Instruction::I64Xor,
                    (BinOp::Shl, ValType::I64) => Instruction::I64Shl,
                    (BinOp::Shr, ValType::I64) => Instruction::I64ShrS,
                    (BinOp::BitAnd, ValType::I32) => Instruction::I32And,
                    (BinOp::BitOr, ValType::I32) => Instruction::I32Or,
                    (BinOp::BitXor, ValType::I32) => Instruction::I32Xor,
                    (BinOp::Shl, ValType::I32) => Instruction::I32Shl,
                    (BinOp::Shr, ValType::I32) => Instruction::I32ShrS,

                    _ => panic!("不支持的运算: {:?} for {:?}", op, val_type),
                };
                func.instruction(&instr);

                // 对 Int8/Int16 算术运算结果进行符号扩展
                // 对 UInt8/UInt16 算术运算结果进行掩码
                let ast_ty = self.infer_ast_type_with_locals(left, locals);
                if let Some(ty) = &ast_ty {
                    match (ty, op) {
                        (
                            Type::Int8,
                            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod,
                        ) => {
                            // Int8 符号扩展: (val << 24) >> 24
                            func.instruction(&Instruction::I32Const(24));
                            func.instruction(&Instruction::I32Shl);
                            func.instruction(&Instruction::I32Const(24));
                            func.instruction(&Instruction::I32ShrS);
                        }
                        (
                            Type::Int16,
                            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod,
                        ) => {
                            // Int16 符号扩展: (val << 16) >> 16
                            func.instruction(&Instruction::I32Const(16));
                            func.instruction(&Instruction::I32Shl);
                            func.instruction(&Instruction::I32Const(16));
                            func.instruction(&Instruction::I32ShrS);
                        }
                        (
                            Type::UInt8,
                            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod,
                        ) => {
                            // UInt8 掩码: val & 0xFF
                            func.instruction(&Instruction::I32Const(0xFF));
                            func.instruction(&Instruction::I32And);
                        }
                        (
                            Type::UInt16,
                            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod,
                        ) => {
                            // UInt16 掩码: val & 0xFFFF
                            func.instruction(&Instruction::I32Const(0xFFFF));
                            func.instruction(&Instruction::I32And);
                        }
                        _ => {}
                    }
                }
            }
            // P3.4: is 类型检查表达式
            Expr::IsType { expr, target_ty } => {
                let src_ty = self.infer_ast_type_with_locals(expr, locals);
                match (&src_ty, target_ty) {
                    // 对象类型（class）检查 vtable 中的 class_id
                    (Some(Type::Struct(_, _)), Type::Struct(target_name, _)) => {
                        let target_id = self
                            .classes
                            .get(target_name)
                            .map(|c| c.class_id)
                            .unwrap_or(u32::MAX);
                        self.compile_expr(expr, locals, func, loop_ctx);
                        // 从指针+0 加载 class_id (vtable 第一个字段)
                        func.instruction(&Instruction::I32Load(wasm_encoder::MemArg {
                            offset: 0,
                            align: 2,
                            memory_index: 0,
                        }));
                        func.instruction(&Instruction::I32Const(target_id as i32));
                        func.instruction(&Instruction::I32Eq);
                    }
                    // 静态类型完全匹配
                    _ if src_ty.as_ref() == Some(target_ty) => {
                        // 编译表达式 (可能有副作用) 然后 drop，push true
                        self.compile_expr(expr, locals, func, loop_ctx);
                        if self.expr_produces_value(expr) {
                            func.instruction(&Instruction::Drop);
                        }
                        func.instruction(&Instruction::I32Const(1)); // true
                    }
                    // 其他情况: 编译时可判断不匹配
                    _ => {
                        self.compile_expr(expr, locals, func, loop_ctx);
                        if self.expr_produces_value(expr) {
                            func.instruction(&Instruction::Drop);
                        }
                        func.instruction(&Instruction::I32Const(0)); // false
                    }
                }
            }
            Expr::PostfixIncr(inner) => {
                self.compile_expr(inner, locals, func, loop_ctx);
                let old_local = locals.get("__postfix_old").expect("__postfix_old");
                func.instruction(&Instruction::LocalSet(old_local));
                self.compile_expr(inner, locals, func, loop_ctx);
                // P5.4: 使用与操作数类型匹配的算术指令
                if self.infer_type_with_locals(inner, locals) == ValType::I32 {
                    func.instruction(&Instruction::I32Const(1));
                    func.instruction(&Instruction::I32Add);
                } else {
                    func.instruction(&Instruction::I64Const(1));
                    func.instruction(&Instruction::I64Add);
                }
                if let Expr::Var(name) = inner.as_ref() {
                    if let Some(idx) = locals.get(name) {
                        func.instruction(&Instruction::LocalSet(idx));
                    }
                }
                func.instruction(&Instruction::LocalGet(old_local));
            }
            Expr::PostfixDecr(inner) => {
                self.compile_expr(inner, locals, func, loop_ctx);
                let old_local = locals.get("__postfix_old").expect("__postfix_old");
                func.instruction(&Instruction::LocalSet(old_local));
                self.compile_expr(inner, locals, func, loop_ctx);
                // P5.4: 使用与操作数类型匹配的算术指令
                if self.infer_type_with_locals(inner, locals) == ValType::I32 {
                    func.instruction(&Instruction::I32Const(1));
                    func.instruction(&Instruction::I32Sub);
                } else {
                    func.instruction(&Instruction::I64Const(1));
                    func.instruction(&Instruction::I64Sub);
                }
                if let Expr::Var(name) = inner.as_ref() {
                    if let Some(idx) = locals.get(name) {
                        func.instruction(&Instruction::LocalSet(idx));
                    }
                }
                func.instruction(&Instruction::LocalGet(old_local));
            }
            Expr::PrefixIncr(inner) => {
                // ++x: 先增加，再返回新值
                self.compile_expr(inner, locals, func, loop_ctx);
                // P5.4: 使用与操作数类型匹配的算术指令
                if self.infer_type_with_locals(inner, locals) == ValType::I32 {
                    func.instruction(&Instruction::I32Const(1));
                    func.instruction(&Instruction::I32Add);
                } else {
                    func.instruction(&Instruction::I64Const(1));
                    func.instruction(&Instruction::I64Add);
                }
                if let Expr::Var(name) = inner.as_ref() {
                    if let Some(idx) = locals.get(name) {
                        func.instruction(&Instruction::LocalTee(idx));
                    }
                }
            }
            Expr::PrefixDecr(inner) => {
                // --x: 先减少，再返回新值
                self.compile_expr(inner, locals, func, loop_ctx);
                // P5.4: 使用与操作数类型匹配的算术指令
                if self.infer_type_with_locals(inner, locals) == ValType::I32 {
                    func.instruction(&Instruction::I32Const(1));
                    func.instruction(&Instruction::I32Sub);
                } else {
                    func.instruction(&Instruction::I64Const(1));
                    func.instruction(&Instruction::I64Sub);
                }
                if let Expr::Var(name) = inner.as_ref() {
                    if let Some(idx) = locals.get(name) {
                        func.instruction(&Instruction::LocalTee(idx));
                    }
                }
            }
            Expr::Cast { expr, target_ty } => {
                self.compile_expr(expr, locals, func, loop_ctx);
                let src = self.infer_type_with_locals(expr, locals);
                let dst = target_ty.to_wasm();
                if src != dst {
                    // 获取源表达式的 AST 类型以区分有符号/无符号
                    let src_ast_ty = self.infer_ast_type_with_locals(expr, locals);
                    let conv = match (src, dst) {
                        (ValType::I64, ValType::I32) => Instruction::I32WrapI64,
                        (ValType::I32, ValType::I64) => {
                            // 根据源类型选择有符号或无符号扩展
                            match src_ast_ty {
                                Some(Type::UInt8) | Some(Type::UInt16) | Some(Type::UInt32) => {
                                    Instruction::I64ExtendI32U
                                }
                                _ => Instruction::I64ExtendI32S,
                            }
                        }
                        (ValType::I64, ValType::F64) => Instruction::F64ConvertI64S,
                        (ValType::F64, ValType::I64) => Instruction::I64TruncF64S,
                        (ValType::I32, ValType::F64) => Instruction::F64ConvertI32S,
                        (ValType::F64, ValType::I32) => Instruction::I32TruncF64S,
                        (ValType::F32, ValType::F64) => Instruction::F64PromoteF32,
                        (ValType::F64, ValType::F32) => Instruction::F32DemoteF64,
                        (ValType::I32, ValType::F32) => Instruction::F32ConvertI32S,
                        (ValType::F32, ValType::I32) => Instruction::I32TruncF32S,
                        (ValType::I64, ValType::F32) => Instruction::F32ConvertI64S,
                        (ValType::F32, ValType::I64) => Instruction::I64TruncF32S,
                        _ => panic!("不支持的 as 转换: {:?} -> {:?}", src, target_ty),
                    };
                    func.instruction(&conv);
                }
                // 转换到小整数类型时进行符号扩展/掩码
                match target_ty {
                    Type::Int8 => {
                        func.instruction(&Instruction::I32Const(24));
                        func.instruction(&Instruction::I32Shl);
                        func.instruction(&Instruction::I32Const(24));
                        func.instruction(&Instruction::I32ShrS);
                    }
                    Type::Int16 => {
                        func.instruction(&Instruction::I32Const(16));
                        func.instruction(&Instruction::I32Shl);
                        func.instruction(&Instruction::I32Const(16));
                        func.instruction(&Instruction::I32ShrS);
                    }
                    Type::UInt8 => {
                        func.instruction(&Instruction::I32Const(0xFF));
                        func.instruction(&Instruction::I32And);
                    }
                    Type::UInt16 => {
                        func.instruction(&Instruction::I32Const(0xFFFF));
                        func.instruction(&Instruction::I32And);
                    }
                    _ => {}
                }
            }
            Expr::Call {
                name,
                type_args: _,
                args,
                named_args,
                ..
            } => {
                // P2.9: 合并命名参数
                let args = if named_args.is_empty() {
                    std::borrow::Cow::Borrowed(args)
                } else {
                    std::borrow::Cow::Owned(self.resolve_named_args(name, args, named_args))
                };
                let args: &[Expr] = &args;
                // Phase 7: I/O 内置函数处理 (println/print/eprintln/eprint)
                if name == "println" || name == "print" || name == "eprintln" || name == "eprint" {
                    // 确定运行时函数前缀
                    let prefix = format!("__{}", name); // __println, __print, __eprintln, __eprint
                    let is_ln = name == "println" || name == "eprintln";
                    let fd: i32 = if name == "eprint" || name == "eprintln" {
                        2
                    } else {
                        1
                    };

                    if args.is_empty() && is_ln {
                        // println() / eprintln() - 输出空行
                        let fd_write_idx = self.func_indices["__wasi_fd_write"];
                        let mem = |offset: u64, align: u32| wasm_encoder::MemArg {
                            offset,
                            align,
                            memory_index: 0,
                        };
                        func.instruction(&Instruction::I32Const(0));
                        func.instruction(&Instruction::I32Const(10));
                        func.instruction(&Instruction::I32Store8(mem(0, 0)));
                        func.instruction(&Instruction::I32Const(IOVEC_OFFSET));
                        func.instruction(&Instruction::I32Const(0));
                        func.instruction(&Instruction::I32Store(mem(0, 2)));
                        func.instruction(&Instruction::I32Const(IOVEC_OFFSET));
                        func.instruction(&Instruction::I32Const(1));
                        func.instruction(&Instruction::I32Store(mem(4, 2)));
                        func.instruction(&Instruction::I32Const(fd));
                        func.instruction(&Instruction::I32Const(IOVEC_OFFSET));
                        func.instruction(&Instruction::I32Const(1));
                        func.instruction(&Instruction::I32Const(NWRITTEN_OFFSET));
                        func.instruction(&Instruction::Call(fd_write_idx));
                        func.instruction(&Instruction::Drop);
                    } else if !args.is_empty() {
                        for arg in args {
                            let arg_ast_type = self.infer_ast_type_with_locals(arg, locals);
                            self.compile_expr(arg, locals, func, loop_ctx);
                            match arg_ast_type.as_ref() {
                                Some(Type::Bool) => {
                                    func.instruction(&Instruction::Call(
                                        self.func_indices[&format!("{}_bool", prefix)],
                                    ));
                                }
                                Some(Type::String) => {
                                    func.instruction(&Instruction::Call(
                                        self.func_indices[&format!("{}_str", prefix)],
                                    ));
                                }
                                Some(Type::Float64) => {
                                    // Float64: 转为字符串后输出
                                    func.instruction(&Instruction::Call(
                                        self.func_indices["__f64_to_str"],
                                    ));
                                    func.instruction(&Instruction::Call(
                                        self.func_indices[&format!("{}_str", prefix)],
                                    ));
                                }
                                Some(Type::Float32) => {
                                    // Float32: 提升为 f64 后转为字符串输出
                                    func.instruction(&Instruction::F64PromoteF32);
                                    func.instruction(&Instruction::Call(
                                        self.func_indices["__f64_to_str"],
                                    ));
                                    func.instruction(&Instruction::Call(
                                        self.func_indices[&format!("{}_str", prefix)],
                                    ));
                                }
                                Some(Type::Struct(sname, _)) => {
                                    // Phase 7.1 #42: print<T> where T <: ToString
                                    // 对 struct/class 类型，尝试调用 toString() 转为字符串后输出
                                    let ts_key = format!("{}.toString", sname);
                                    if self.func_indices.contains_key(&ts_key) {
                                        func.instruction(&Instruction::Call(
                                            self.func_indices[&ts_key],
                                        ));
                                        func.instruction(&Instruction::Call(
                                            self.func_indices[&format!("{}_str", prefix)],
                                        ));
                                    } else {
                                        // 无 toString 方法，转为 i64 输出对象指针
                                        func.instruction(&Instruction::I64ExtendI32S);
                                        func.instruction(&Instruction::Call(
                                            self.func_indices[&format!("{}_i64", prefix)],
                                        ));
                                    }
                                }
                                _ => {
                                    let wasm_type = self.infer_type_with_locals(arg, locals);
                                    match wasm_type {
                                        ValType::I64 => {}
                                        ValType::I32 => {
                                            // 检查是否为 struct/class 类型（WASM 中是 i32 指针）
                                            // 尝试从表达式推断具体类型名
                                            let maybe_struct =
                                                self.try_get_struct_name(arg, locals);
                                            if let Some(ref sn) = maybe_struct {
                                                let ts_key = format!("{}.toString", sn);
                                                if self.func_indices.contains_key(&ts_key) {
                                                    func.instruction(&Instruction::Call(
                                                        self.func_indices[&ts_key],
                                                    ));
                                                    func.instruction(&Instruction::Call(
                                                        self.func_indices
                                                            [&format!("{}_str", prefix)],
                                                    ));
                                                    continue; // 跳过后面的 i64 路径
                                                }
                                            }
                                            func.instruction(&Instruction::I64ExtendI32S);
                                        }
                                        ValType::F64 => {
                                            func.instruction(&Instruction::Call(
                                                self.func_indices["__f64_to_str"],
                                            ));
                                            func.instruction(&Instruction::Call(
                                                self.func_indices[&format!("{}_str", prefix)],
                                            ));
                                            continue; // 跳过后面的 i64 路径
                                        }
                                        ValType::F32 => {
                                            func.instruction(&Instruction::F64PromoteF32);
                                            func.instruction(&Instruction::Call(
                                                self.func_indices["__f64_to_str"],
                                            ));
                                            func.instruction(&Instruction::Call(
                                                self.func_indices[&format!("{}_str", prefix)],
                                            ));
                                            continue; // 跳过后面的 i64 路径
                                        }
                                        _ => {}
                                    }
                                    func.instruction(&Instruction::Call(
                                        self.func_indices[&format!("{}_i64", prefix)],
                                    ));
                                }
                            }
                        }
                    }
                    // print()/eprint() without args - do nothing
                } else if name == "readln" && args.is_empty() {
                    // Phase 7.1 #44: readln() -> String
                    func.instruction(&Instruction::Call(self.func_indices["__readln"]));
                } else if name == "exit" && args.len() == 1 {
                    // Phase 7.7: exit(code: Int64)
                    self.compile_expr(&args[0], locals, func, loop_ctx);
                    func.instruction(&Instruction::I32WrapI64);
                    func.instruction(&Instruction::Call(self.func_indices["__exit"]));
                } else if name == "getArgs" && args.is_empty() {
                    // Phase 7.7: getArgs() -> Array<String>
                    func.instruction(&Instruction::Call(self.func_indices["__get_args"]));
                } else if name == "getEnv" && args.len() == 1 {
                    // Phase 7.7: getEnv(key: String) -> String (空串表示未找到)
                    self.compile_expr(&args[0], locals, func, loop_ctx);
                    func.instruction(&Instruction::Call(self.func_indices["__get_env"]));
                } else if name == "now" && args.is_empty() {
                    // Phase 7.7: now() -> Int64 (纳秒时间戳)
                    func.instruction(&Instruction::Call(self.func_indices["__get_time_ns"]));
                } else if name == "randomInt64" && args.is_empty() {
                    // Phase 7.7: randomInt64() -> Int64
                    func.instruction(&Instruction::Call(self.func_indices["__random_i64"]));
                } else if name == "randomFloat64" && args.is_empty() {
                    // Phase 7.7: randomFloat64() -> Float64 in [0, 1)
                    func.instruction(&Instruction::Call(self.func_indices["__random_f64"]));
                } else if name == "ArrayList" && args.is_empty() {
                    // Phase 7.5: ArrayList() -> ArrayList
                    func.instruction(&Instruction::Call(self.func_indices["__arraylist_new"]));
                } else if name == "HashMap" && args.is_empty() {
                    // Phase 7.5: HashMap() -> HashMap
                    func.instruction(&Instruction::Call(self.func_indices["__hashmap_new"]));
                } else if name == "HashSet" && args.is_empty() {
                    // Phase 7.5: HashSet() → 基于 HashMap
                    func.instruction(&Instruction::Call(self.func_indices["__hashmap_new"]));
                } else if name == "LinkedList" && args.is_empty() {
                    // Phase 7.5: LinkedList() -> LinkedList
                    func.instruction(&Instruction::Call(self.func_indices["__linkedlist_new"]));
                } else if name == "ArrayStack" && args.is_empty() {
                    // Phase 7.5: ArrayStack() → 基于 ArrayList
                    func.instruction(&Instruction::Call(self.func_indices["__arraylist_new"]));
                } else if name == "sort" && args.len() == 1 {
                    // Phase 7.8: sort(arr) — 原地排序
                    // 数组指针已经是 i32，无需转换
                    self.compile_expr(&args[0], locals, func, loop_ctx);
                    func.instruction(&Instruction::Call(self.func_indices["__sort_array"]));
                    // sort 返回 void，不需要推哑值（expr_produces_value 返回 false）
                } else if Self::is_math_builtin(name) && !self.func_indices.contains_key(name) {
                    // Phase 7.3: math 内置函数（仅在用户未自定义同名函数时）
                    self.compile_math_builtin(name, args, locals, func, loop_ctx);
                // P2.3: 检查是否为 Lambda/函数类型的局部变量调用
                } else if locals.get(name).is_some()
                    && matches!(locals.get_ast_type(name), Some(Type::Function { .. }))
                {
                    // Lambda 调用：通过 call_indirect
                    for arg in args {
                        self.compile_expr(arg, locals, func, loop_ctx);
                    }
                    // 获取函数类型索引 — 查找 lambda 的类型
                    // 从 AST 类型推断 lambda 的 WASM 类型签名
                    if let Some(Type::Function {
                        ref params,
                        ref ret,
                    }) = locals.get_ast_type(name)
                    {
                        let wasm_params: Vec<ValType> =
                            params.iter().map(|t| t.to_wasm()).collect();
                        let wasm_results: Vec<ValType> = ret
                            .as_ref()
                            .as_ref()
                            .map(|t| vec![t.to_wasm()])
                            .unwrap_or_default();
                        // 查找匹配的类型索引
                        let type_idx =
                            self.find_or_create_func_type_idx(&wasm_params, &wasm_results);
                        let local_idx = locals.get(name).unwrap();
                        func.instruction(&Instruction::LocalGet(local_idx));
                        func.instruction(&Instruction::CallIndirect {
                            type_index: type_idx,
                            table_index: 0,
                        });
                    }
                } else {
                    // 检查是否为带 init 的类构造调用
                    let init_func_name = format!("__{}_init", name);
                    if self.func_indices.contains_key(&init_func_name) {
                        // 调用 __ClassName_init(args...) 返回对象指针
                        for arg in args {
                            self.compile_expr(arg, locals, func, loop_ctx);
                        }
                        let idx = self.func_indices[&init_func_name];
                        func.instruction(&Instruction::Call(idx));
                    } else if let Some(struct_def) = self.structs.get(name).cloned() {
                        if args.len() != struct_def.fields.len() {
                            panic!(
                                "结构体 {} 构造函数需要 {} 个参数，得到 {} 个",
                                name,
                                struct_def.fields.len(),
                                args.len()
                            );
                        }
                        let fields: Vec<(String, Expr)> = struct_def
                            .fields
                            .iter()
                            .map(|f| f.name.clone())
                            .zip(args.iter().cloned())
                            .collect();
                        let init_expr = Expr::StructInit {
                            name: name.clone(),
                            type_args: None,
                            fields,
                        };
                        self.compile_expr(&init_expr, locals, func, loop_ctx);
                    } else if name == "min"
                        && args.len() == 2
                        && self.infer_ast_type_with_locals(&args[0], locals).as_ref()
                            == Some(&Type::Int64)
                        && self.infer_ast_type_with_locals(&args[1], locals).as_ref()
                            == Some(&Type::Int64)
                    {
                        self.compile_expr(&args[0], locals, func, loop_ctx);
                        self.compile_expr(&args[1], locals, func, loop_ctx);
                        func.instruction(&Instruction::Call(
                            self.get_or_create_func_index("__min_i64"),
                        ));
                    } else if name == "max"
                        && args.len() == 2
                        && self.infer_ast_type_with_locals(&args[0], locals).as_ref()
                            == Some(&Type::Int64)
                        && self.infer_ast_type_with_locals(&args[1], locals).as_ref()
                            == Some(&Type::Int64)
                    {
                        self.compile_expr(&args[0], locals, func, loop_ctx);
                        self.compile_expr(&args[1], locals, func, loop_ctx);
                        func.instruction(&Instruction::Call(
                            self.get_or_create_func_index("__max_i64"),
                        ));
                    } else if name == "abs"
                        && args.len() == 1
                        && self.infer_ast_type_with_locals(&args[0], locals).as_ref()
                            == Some(&Type::Int64)
                    {
                        self.compile_expr(&args[0], locals, func, loop_ctx);
                        func.instruction(&Instruction::Call(
                            self.get_or_create_func_index("__abs_i64"),
                        ));
                    } else if name == "String" && args.len() == 1 {
                        // String(x) - 将值转换为 String，实际上只传递原始值（运行时处理转换）
                        // 对于 String(runeValue) 或 String(slice)，直接编译参数表达式
                        self.compile_expr(&args[0], locals, func, loop_ctx);
                    } else if name == "String" && args.is_empty() {
                        // String() - 空字符串，返回 0
                        func.instruction(&Instruction::I32Const(0));
                    } else if args.len() == 1
                        && [
                            "Int64", "Int32", "Int16", "Int8", "UInt64", "UInt32", "UInt16",
                            "UInt8", "Float64", "Float32", "Bool", "Rune",
                        ]
                        .contains(&name.as_str())
                    {
                        // 类型转换构造函数 T(e) - cjc 兼容
                        let target_ty = match name.as_str() {
                            "Int64" => Type::Int64,
                            "Int32" => Type::Int32,
                            "Int16" => Type::Int16,
                            "Int8" => Type::Int8,
                            "UInt64" => Type::UInt64,
                            "UInt32" => Type::UInt32,
                            "UInt16" => Type::UInt16,
                            "UInt8" => Type::UInt8,
                            "Float64" => Type::Float64,
                            "Float32" => Type::Float32,
                            "Bool" => Type::Bool,
                            "Rune" => Type::Rune,
                            _ => unreachable!(),
                        };
                        self.compile_expr(
                            &Expr::Cast {
                                expr: Box::new(args[0].clone()),
                                target_ty,
                            },
                            locals,
                            func,
                            loop_ctx,
                        );
                    } else {
                        let arg_tys: Vec<Type> = args
                            .iter()
                            .map(|a| {
                                self.infer_ast_type_with_locals(a, locals)
                                    .unwrap_or(Type::Unit)
                            })
                            .collect();
                        let key = if *self.name_count.get(name).unwrap_or(&0) > 1 {
                            Self::mangle_key(name, &arg_tys)
                        } else {
                            name.to_string()
                        };
                        // Check if this is a method call within the same class
                        let (actual_key, is_implicit_method_call) = if self
                            .func_params
                            .contains_key(&key)
                        {
                            (key.clone(), false)
                        } else if let Some(Type::Struct(class_name, _)) = locals.get_type("this") {
                            let method_key = format!("{}.{}", class_name, name);
                            if self.func_params.contains_key(&method_key) {
                                (method_key, true)
                            } else {
                                // 尝试按参数数量匹配重载方法（args.len() + 1 for this）
                                let expected_param_count = args.len() + 1;
                                let prefix_exact = format!("{}.{}", class_name, name);
                                let prefix_mangled = format!("{}.{}$", class_name, name);
                                let candidates: Vec<String> = self
                                    .func_params
                                    .keys()
                                    .filter(|k| {
                                        (*k == &prefix_exact || k.starts_with(&prefix_mangled))
                                            && self.func_params[*k].len() == expected_param_count
                                    })
                                    .cloned()
                                    .collect();
                                if let Some(candidate) = candidates.into_iter().next() {
                                    (candidate, true)
                                } else {
                                    // 尝试接口默认方法：InterfaceName.__default_methodName
                                    let default_suffix = format!(".__default_{}", name);
                                    let iface_candidates: Vec<String> = self
                                        .func_params
                                        .keys()
                                        .filter(|k| {
                                            k.ends_with(&default_suffix)
                                                && self.func_params[*k].len()
                                                    == expected_param_count
                                        })
                                        .cloned()
                                        .collect();
                                    if let Some(candidate) = iface_candidates.into_iter().next() {
                                        (candidate, true)
                                    } else {
                                        // 也尝试包级别的函数按参数数量匹配
                                        let pkg_prefix = format!("{}$", name);
                                        let pkg_candidates: Vec<String> = self
                                            .func_params
                                            .keys()
                                            .filter(|k| {
                                                (*k == name || k.starts_with(&pkg_prefix))
                                                    && self.func_params[*k].len() == args.len()
                                            })
                                            .cloned()
                                            .collect();
                                        if let Some(candidate) = pkg_candidates.into_iter().next() {
                                            (candidate, false)
                                        } else {
                                            (key.clone(), false)
                                        }
                                    }
                                }
                            }
                        } else {
                            // 尝试包级别的函数按参数数量匹配
                            let pkg_prefix = format!("{}$", name);
                            let pkg_candidates: Vec<String> = self
                                .func_params
                                .keys()
                                .filter(|k| {
                                    (*k == name || k.starts_with(&pkg_prefix))
                                        && self.func_params[*k].len() == args.len()
                                })
                                .cloned()
                                .collect();
                            if let Some(candidate) = pkg_candidates.into_iter().next() {
                                (candidate, false)
                            } else {
                                (key.clone(), false)
                            }
                        };

                        if self.func_params.get(&actual_key).is_none() {
                            // 函数未找到：生成警告并发出桩代码
                            eprintln!("[警告] 函数未找到: '{}' - 生成桩代码", actual_key);
                            let matching: Vec<_> = self
                                .func_params
                                .keys()
                                .filter(|k| k.contains(name.as_str()))
                                .cloned()
                                .collect();
                            if !matching.is_empty() {
                                eprintln!("  包含 '{}' 的函数: {:?}", name, matching);
                            }
                            // 发出参数（避免栈不平衡）
                            for arg in args {
                                self.compile_expr(arg, locals, func, loop_ctx);
                                func.instruction(&Instruction::Drop);
                            }
                            func.instruction(&Instruction::I64Const(0)); // 桩返回值
                            return;
                        }
                        let params = self.func_params.get(&actual_key).unwrap();

                        // If this is an implicit method call, add 'this' as first argument
                        if is_implicit_method_call {
                            let this_idx = locals.get("this").expect("this not found");
                            func.instruction(&Instruction::LocalGet(this_idx));
                        }

                        // 检查是否有可变参数
                        let variadic_idx = params.iter().position(|p| p.variadic);

                        // Adjust parameter index if we added 'this'
                        let param_offset = if is_implicit_method_call { 1 } else { 0 };

                        for (i, param) in params.iter().enumerate() {
                            // Skip the first parameter (this) if it's an implicit method call
                            if is_implicit_method_call && i == 0 {
                                continue;
                            }

                            let arg_idx = i - param_offset;

                            if param.variadic {
                                // 可变参数：将剩余实参打包成数组
                                let variadic_args: Vec<Expr> = args[arg_idx..].to_vec();
                                let arr_expr = Expr::Array(variadic_args);
                                self.compile_expr(&arr_expr, locals, func, loop_ctx);
                            } else if arg_idx < args.len() && variadic_idx.map_or(true, |vi| i < vi)
                            {
                                // 普通参数：直接编译实参
                                self.compile_expr(&args[arg_idx], locals, func, loop_ctx);
                                // 参数类型适配：当实参类型与形参类型不匹配时自动转换
                                let arg_wasm_ty =
                                    self.infer_type_with_locals(&args[arg_idx], locals);
                                let param_wasm_ty = param.ty.to_wasm();
                                self.emit_type_coercion(func, arg_wasm_ty, param_wasm_ty);
                            } else if let Some(ref default) = param.default {
                                self.compile_expr(default, locals, func, loop_ctx);
                            } else {
                                // 参数不足：发出警告并使用零值
                                eprintln!(
                                    "[警告] 函数 {} 第 {} 个参数缺少实参且无默认值，使用零值",
                                    name,
                                    i + 1
                                );
                                func.instruction(&Instruction::I64Const(0));
                            }
                        }
                        let idx = *self.func_indices.get(&actual_key).expect("函数未找到");
                        func.instruction(&Instruction::Call(idx));
                    }
                } // end else (non-println)
            }
            Expr::SuperCall { method, args, .. } => {
                // super 调用：直接调用父类的方法（绕过 vtable）
                // 从函数名推断当前类 → 找父类 → 调用父类方法
                // super(args) → 调用父类 init; super.method(args) → 调用父类方法
                // super 调用分两种：super(args) 和 super.method(args)
                if method == "init" {
                    // Bug B3 修复: super(args) → 调用父类的 __ParentClass_init_body(this, args...)
                    // 而非 __ParentClass_init(args...) 以避免分配新对象
                    // 优先使用 init_body 版本（传入当前 this），回退到旧逻辑
                    let mut found = false;
                    for ci in self.classes.values() {
                        if let Some(ref parent) = ci.parent {
                            let parent_init_body = format!("__{}_init_body", parent);
                            if self.func_indices.contains_key(&parent_init_body) {
                                // 传递当前 this 作为第一个参数
                                if let Some(this_idx) = locals.get("this") {
                                    func.instruction(&Instruction::LocalGet(this_idx));
                                }
                                for arg in args {
                                    self.compile_expr(arg, locals, func, loop_ctx);
                                }
                                let idx = self.func_indices[&parent_init_body];
                                func.instruction(&Instruction::Call(idx));
                                // init_body 无返回值，不需要 Drop
                                found = true;
                                break;
                            }
                        }
                    }
                    if !found {
                        // 回退: 旧逻辑（调用 __Parent_init 并丢弃结果）
                        for arg in args {
                            self.compile_expr(arg, locals, func, loop_ctx);
                        }
                        for ci in self.classes.values() {
                            if let Some(ref parent) = ci.parent {
                                let parent_init = format!("__{}_init", parent);
                                if self.func_indices.contains_key(&parent_init) {
                                    let idx = self.func_indices[&parent_init];
                                    func.instruction(&Instruction::Call(idx));
                                    func.instruction(&Instruction::Drop);
                                    break;
                                }
                            }
                        }
                    }
                } else {
                    // super.method(args) → 直接调用父类版本的方法
                    // 查找当前类的父类，调用 ParentClass.method
                    for ci in self.classes.values() {
                        if let Some(ref parent) = ci.parent {
                            let parent_method = format!("{}.{}", parent, method);
                            if let Some(&idx) = self.func_indices.get(&parent_method) {
                                // this 指针作为第一个参数
                                if let Some(this_idx) = locals.get("this") {
                                    func.instruction(&Instruction::LocalGet(this_idx));
                                }
                                for arg in args {
                                    self.compile_expr(arg, locals, func, loop_ctx);
                                }
                                func.instruction(&Instruction::Call(idx));
                                break;
                            }
                        }
                    }
                }
            }
            Expr::SuperFieldAccess { field } => {
                // super.field：读取父类字段值
                // 获取 this 指针
                let this_idx = locals.get("this").expect("super 字段访问需要 this");

                // 从 this 的类型获取当前类名，再查找父类
                let this_type = locals.get_type("this").expect("this 类型未找到");
                let current_class_name = match this_type {
                    Type::Struct(name, _) => name,
                    _ => panic!("this 类型不是 Struct"),
                };

                // 从当前类获取父类名
                let parent_class_name = self
                    .classes
                    .get(current_class_name)
                    .and_then(|ci| ci.parent.as_ref())
                    .expect(&format!("类 {} 没有父类", current_class_name));

                // 从父类获取字段偏移和类型
                let field_info = self.classes.get(parent_class_name).and_then(|ci| {
                    let off = ci.field_offset(field)?;
                    let ft = ci.field_type(field)?.clone();
                    Some((off, ft))
                });

                if let Some((offset, field_ty)) = field_info {
                    func.instruction(&Instruction::LocalGet(this_idx));
                    // 根据字段类型选择加载指令
                    match field_ty {
                        Type::Float64 => {
                            func.instruction(&Instruction::F64Load(wasm_encoder::MemArg {
                                offset: offset as u64,
                                align: 3,
                                memory_index: 0,
                            }));
                        }
                        Type::Float32 => {
                            func.instruction(&Instruction::F32Load(wasm_encoder::MemArg {
                                offset: offset as u64,
                                align: 2,
                                memory_index: 0,
                            }));
                        }
                        _ => {
                            func.instruction(&Instruction::I64Load(wasm_encoder::MemArg {
                                offset: offset as u64,
                                align: 3,
                                memory_index: 0,
                            }));
                        }
                    }
                } else {
                    // 字段未找到（可能是属性/property）- 尝试作为父类属性getter调用
                    let getter_key = format!("{}.{}", parent_class_name, field);
                    let getter_key2 = format!("{}.{}_get", parent_class_name, field);
                    if let Some(&idx) = self.func_indices.get(&getter_key) {
                        func.instruction(&Instruction::LocalGet(this_idx));
                        func.instruction(&Instruction::Call(idx));
                    } else if let Some(&idx) = self.func_indices.get(&getter_key2) {
                        func.instruction(&Instruction::LocalGet(this_idx));
                        func.instruction(&Instruction::Call(idx));
                    } else {
                        // 直接访问当前对象的字段（继承字段可能在 this 上）
                        let field_expr = Expr::Field {
                            object: Box::new(Expr::Var("this".to_string())),
                            field: field.clone(),
                        };
                        self.compile_expr(&field_expr, locals, func, loop_ctx);
                    }
                }
            }
            Expr::MethodCall {
                object,
                method,
                args,
                ..
            } => {
                // Phase 7.2: 内建类型方法分发
                // 先推断对象的 AST 类型，检查是否可以用内建方法处理
                let obj_ast_type = self.infer_ast_type_with_locals(object, locals);
                if self.compile_builtin_method(
                    object,
                    &obj_ast_type,
                    method,
                    args,
                    locals,
                    func,
                    loop_ctx,
                ) {
                    // 内建方法已处理，无需走 struct/class 方法分发
                } else {
                    // 非内建类型 → 走原有 struct/class 方法分发逻辑
                    let type_name_opt = if let Expr::Var(ref n) = object.as_ref() {
                        Some(n.clone())
                    } else {
                        None
                    };
                    let is_static = type_name_opt.as_ref().map_or(false, |n| {
                        (self.structs.contains_key(n)
                            || self.enums.contains_key(n)
                            || self.classes.contains_key(n))
                            && self.func_indices.contains_key(&format!("{}.{}", n, method))
                    });
                    let key = if is_static {
                        format!("{}.{}", type_name_opt.unwrap(), method)
                    } else {
                        let obj_type = self.get_object_type(object, locals);
                        let struct_ty = obj_type.and_then(|ty| match ty {
                            Type::Struct(s, type_args) => {
                                if type_args.is_empty() {
                                    Some(s)
                                } else {
                                    Some(crate::monomorph::mangle_name(&s, &type_args))
                                }
                            }
                            Type::Option(_) => Some("Option".to_string()),
                            Type::Result(_, _) => Some("Result".to_string()),
                            Type::Map(_, _) => Some("Map".to_string()),
                            _ => None,
                        });
                        struct_ty
                            .as_ref()
                            .map(|s| format!("{}.{}", s, method))
                            .unwrap_or_else(|| method.clone())
                    };
                    if !is_static {
                        self.compile_expr(object, locals, func, loop_ctx);
                    }
                    // 查找方法参数类型，用于实参类型协调
                    let params_for_call = self.func_params.get(&key).cloned();
                    for (i, arg) in args.iter().enumerate() {
                        self.compile_expr(arg, locals, func, loop_ctx);
                        // 参数类型适配：仅当 AST 类型可确定时自动转换（避免对未知类型误推断）
                        if let Some(ref params) = params_for_call {
                            // 非静态方法：params[0] = self，params[i+1] = args[i]
                            // 静态方法：params[i] = args[i]
                            let param_idx = if is_static { i } else { i + 1 };
                            if let Some(param) = params.get(param_idx) {
                                if !param.variadic {
                                    // 只在实参 AST 类型确定时才做协调，避免 infer_type 回退 I64 误触发 I32WrapI64
                                    if let Some(arg_ast_ty) =
                                        self.infer_ast_type_with_locals(arg, locals)
                                    {
                                        let arg_wasm_ty = arg_ast_ty.to_wasm();
                                        let param_wasm_ty = param.ty.to_wasm();
                                        self.emit_type_coercion(func, arg_wasm_ty, param_wasm_ty);
                                    }
                                }
                            }
                        }
                    }
                    // 查找方法索引，支持继承链向上查找
                    let idx = self.resolve_method_index(&key, method);
                    if idx == u32::MAX {
                        // 特殊情况：getOrThrow/unwrap 在非 Option 类型上是 pass-through
                        // 我们的 HashMap.get() 直接返回 V，所以 getOrThrow 是空操作
                        let is_passthrough = matches!(method.as_str(), "getOrThrow" | "unwrap")
                            && !matches!(obj_ast_type, Some(Type::Option(_)))
                            && !is_static
                            && args.is_empty();
                        if is_passthrough {
                            // object 已在栈上，保持不变（pass-through）
                        } else {
                            // 方法未找到：生成桩代码（丢弃所有参数，压入默认值 0）
                            // 此时栈上已经有 object（如果非静态）+ args
                            if !is_static {
                                func.instruction(&Instruction::Drop);
                            }
                            for _ in args.iter() {
                                func.instruction(&Instruction::Drop);
                            }
                            // 根据方法调用推断返回类型，压入对应默认值
                            // 使用 infer_ast_type_with_locals 检查 Unit，但用 infer_type 作为回退
                            // 与 collect_locals 的行为一致（两者都用 infer_type 作为最终回退）
                            let ret_ast_ty = self.infer_ast_type_with_locals(expr, locals);
                            // Unit 类型不产生值，不需要压入任何东西
                            if ret_ast_ty.as_ref() != Some(&Type::Unit) {
                                let ret_wasm_ty = if let Some(ref ast_ty) = ret_ast_ty {
                                    ast_ty.to_wasm()
                                } else {
                                    self.infer_type(expr) // 与 collect_locals 一致的回退
                                };
                                match ret_wasm_ty {
                                    ValType::I32 => {
                                        func.instruction(&Instruction::I32Const(0));
                                    }
                                    ValType::F32 => {
                                        func.instruction(&Instruction::F32Const(0.0_f32));
                                    }
                                    ValType::F64 => {
                                        func.instruction(&Instruction::F64Const(0.0_f64));
                                    }
                                    _ => {
                                        func.instruction(&Instruction::I64Const(0));
                                    }
                                }
                            }
                        }
                    } else {
                        func.instruction(&Instruction::Call(idx));
                    }
                }
            }
            Expr::If {
                cond,
                then_branch,
                else_branch,
            } => {
                self.compile_expr(cond, locals, func, loop_ctx);
                // 条件必须是 i32；仅当 AST 类型确认为 i64 时才 wrap（TypeParam 保守不 wrap）
                if self.needs_i64_to_i32_wrap(cond, locals)
                    && self.infer_type_with_locals(cond, locals) == ValType::I64
                {
                    func.instruction(&Instruction::I32WrapI64);
                }

                // if 指令创建新的 WASM 块，需要将 loop_ctx 的 break/continue 深度 +1
                let inner_ctx = loop_ctx.map(|(b, c)| (b + 1, c + 1));

                if let Some(else_expr) = else_branch {
                    let then_produces = self.expr_produces_value(then_branch);
                    let else_produces = self.expr_produces_value(else_expr);
                    if then_produces && else_produces {
                        // if-else 表达式：两个分支都有返回值
                        let result_type = wasm_encoder::BlockType::Result(
                            self.infer_type_with_locals(then_branch, locals),
                        );
                        func.instruction(&Instruction::If(result_type));
                        self.compile_expr(then_branch, locals, func, inner_ctx);
                        func.instruction(&Instruction::Else);
                        self.compile_expr(else_expr, locals, func, inner_ctx);
                    } else {
                        // if-else 语句：至少一个分支不产生值，用 Empty 块
                        func.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
                        self.compile_expr(then_branch, locals, func, inner_ctx);
                        if then_produces {
                            func.instruction(&Instruction::Drop);
                        }
                        func.instruction(&Instruction::Else);
                        self.compile_expr(else_expr, locals, func, inner_ctx);
                        if else_produces {
                            func.instruction(&Instruction::Drop);
                        }
                    }
                } else {
                    // if 无 else：无返回值（语句级）
                    func.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
                    // then_branch 如果会产生值（非 void Block），需要 drop
                    let produces_value = self.expr_produces_value(then_branch);
                    self.compile_expr(then_branch, locals, func, inner_ctx);
                    if produces_value {
                        func.instruction(&Instruction::Drop);
                    }
                }

                func.instruction(&Instruction::End);
            }
            Expr::IfLet {
                pattern,
                expr,
                then_branch,
                else_branch,
            } => {
                let else_expr = else_branch
                    .clone()
                    .unwrap_or_else(|| Box::new(Expr::Integer(0)));
                let match_expr = Expr::Match {
                    expr: expr.clone(),
                    arms: vec![
                        MatchArm {
                            pattern: pattern.clone(),
                            guard: None,
                            body: then_branch.clone(),
                        },
                        MatchArm {
                            pattern: Pattern::Wildcard,
                            guard: None,
                            body: else_expr,
                        },
                    ],
                };
                self.compile_expr(&match_expr, locals, func, loop_ctx);
            }
            Expr::Tuple(elements) => {
                // Phase 8: 使用 __alloc 分配元组内存
                let elem_size = 8i32;
                let total_size = elements.len() as i32 * elem_size;
                let alloc_idx = self.func_indices["__alloc"];
                let tmp_local = locals
                    .get("__tuple_alloc_ptr")
                    .expect("__tuple_alloc_ptr 未预注册");

                func.instruction(&Instruction::I32Const(total_size));
                func.instruction(&Instruction::Call(alloc_idx));
                func.instruction(&Instruction::LocalSet(tmp_local));

                // 写入每个元素
                for (i, elem) in elements.iter().enumerate() {
                    func.instruction(&Instruction::LocalGet(tmp_local));
                    func.instruction(&Instruction::I32Const(i as i32 * elem_size));
                    func.instruction(&Instruction::I32Add);
                    self.compile_expr(elem, locals, func, loop_ctx);
                    let elem_ty = self.infer_type_with_locals(elem, locals);
                    match elem_ty {
                        ValType::I64 => {
                            func.instruction(&Instruction::I64Store(wasm_encoder::MemArg {
                                offset: 0,
                                align: 3,
                                memory_index: 0,
                            }))
                        }
                        ValType::F64 => {
                            func.instruction(&Instruction::F64Store(wasm_encoder::MemArg {
                                offset: 0,
                                align: 3,
                                memory_index: 0,
                            }))
                        }
                        ValType::I32 => {
                            func.instruction(&Instruction::I64ExtendI32U);
                            func.instruction(&Instruction::I64Store(wasm_encoder::MemArg {
                                offset: 0,
                                align: 3,
                                memory_index: 0,
                            }))
                        }
                        ValType::F32 => {
                            func.instruction(&Instruction::F64PromoteF32);
                            func.instruction(&Instruction::F64Store(wasm_encoder::MemArg {
                                offset: 0,
                                align: 3,
                                memory_index: 0,
                            }))
                        }
                        _ => func.instruction(&Instruction::I64Store(wasm_encoder::MemArg {
                            offset: 0,
                            align: 3,
                            memory_index: 0,
                        })),
                    };
                }

                // 返回元组地址
                func.instruction(&Instruction::LocalGet(tmp_local));
            }
            Expr::TupleIndex { object, index } => {
                // tuple.N -> load from (tuple_ptr + N * 8)
                self.compile_expr(object, locals, func, loop_ctx);
                func.instruction(&Instruction::I32Const(*index as i32 * 8));
                func.instruction(&Instruction::I32Add);
                // 推断元素类型来选择正确的 load 指令
                let elem_ty = self.infer_ast_type_with_locals(expr, locals);
                match elem_ty.as_ref().map(|t| t.to_wasm()) {
                    Some(ValType::I32) => {
                        // i32 值是零扩展存储的，读回 i64 后 wrap
                        func.instruction(&Instruction::I64Load(wasm_encoder::MemArg {
                            offset: 0,
                            align: 3,
                            memory_index: 0,
                        }));
                        func.instruction(&Instruction::I32WrapI64);
                    }
                    Some(ValType::F32) => {
                        func.instruction(&Instruction::F64Load(wasm_encoder::MemArg {
                            offset: 0,
                            align: 3,
                            memory_index: 0,
                        }));
                        func.instruction(&Instruction::F32DemoteF64);
                    }
                    Some(ValType::F64) => {
                        func.instruction(&Instruction::F64Load(wasm_encoder::MemArg {
                            offset: 0,
                            align: 3,
                            memory_index: 0,
                        }));
                    }
                    _ => {
                        // 默认按 i64 读取
                        func.instruction(&Instruction::I64Load(wasm_encoder::MemArg {
                            offset: 0,
                            align: 3,
                            memory_index: 0,
                        }));
                    }
                }
            }
            Expr::NullCoalesce { option, default } => {
                // a ?? b: 若 a 为 Some(v) 返回 v，否则返回 b
                // Option 内存布局: [tag: i32][value: ...]
                // tag == 0 => None, tag == 1 => Some
                self.compile_expr(option, locals, func, loop_ctx);
                let result_type = self.infer_type_with_locals(default, locals);
                // 保存 option 指针到临时变量
                let tmp = locals.get("__try_ptr").expect("__try_ptr");
                func.instruction(&Instruction::LocalSet(tmp));
                // 检查 tag
                func.instruction(&Instruction::LocalGet(tmp));
                func.instruction(&Instruction::I32Load(wasm_encoder::MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                }));
                func.instruction(&Instruction::If(wasm_encoder::BlockType::Result(
                    result_type,
                )));
                // Some: 读取 value（偏移 4 字节）
                func.instruction(&Instruction::LocalGet(tmp));
                func.instruction(&Instruction::I32Const(4));
                func.instruction(&Instruction::I32Add);
                match result_type {
                    ValType::I64 => func.instruction(&Instruction::I64Load(wasm_encoder::MemArg {
                        offset: 0,
                        align: 3,
                        memory_index: 0,
                    })),
                    ValType::I32 => func.instruction(&Instruction::I32Load(wasm_encoder::MemArg {
                        offset: 0,
                        align: 2,
                        memory_index: 0,
                    })),
                    ValType::F64 => func.instruction(&Instruction::F64Load(wasm_encoder::MemArg {
                        offset: 0,
                        align: 3,
                        memory_index: 0,
                    })),
                    ValType::F32 => func.instruction(&Instruction::F32Load(wasm_encoder::MemArg {
                        offset: 0,
                        align: 2,
                        memory_index: 0,
                    })),
                    _ => func.instruction(&Instruction::I64Load(wasm_encoder::MemArg {
                        offset: 0,
                        align: 3,
                        memory_index: 0,
                    })),
                };
                func.instruction(&Instruction::Else);
                // None: 返回默认值
                self.compile_expr(default, locals, func, loop_ctx);
                func.instruction(&Instruction::End);
            }
            Expr::Array(elements) => {
                // Phase 8: 使用 __alloc 分配数组内存
                // P3.9: 根据元素 WASM 类型确定步长（i32/f32=4, i64/f64=8）
                let elem_wasm_ty = elements
                    .first()
                    .map(|e| self.infer_type_with_locals(e, locals))
                    .unwrap_or(ValType::I64);
                let elem_size: i32 = match elem_wasm_ty {
                    ValType::I32 | ValType::F32 => 4,
                    _ => 8, // i64, f64
                };
                let total_size = 4 + elements.len() as i32 * elem_size; // length + elements
                let alloc_idx = self.func_indices["__alloc"];
                let tmp_local = locals
                    .get("__array_alloc_ptr")
                    .expect("__array_alloc_ptr 未预注册");

                func.instruction(&Instruction::I32Const(total_size));
                func.instruction(&Instruction::Call(alloc_idx));
                func.instruction(&Instruction::LocalSet(tmp_local));

                // 写入数组长度
                func.instruction(&Instruction::LocalGet(tmp_local));
                func.instruction(&Instruction::I32Const(elements.len() as i32));
                func.instruction(&Instruction::I32Store(wasm_encoder::MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                }));

                // 写入每个元素（按 WASM 类型选择 Store 指令）
                for (i, elem) in elements.iter().enumerate() {
                    func.instruction(&Instruction::LocalGet(tmp_local));
                    func.instruction(&Instruction::I32Const(4 + i as i32 * elem_size));
                    func.instruction(&Instruction::I32Add);
                    self.compile_expr(elem, locals, func, loop_ctx);
                    match elem_wasm_ty {
                        ValType::F64 => {
                            func.instruction(&Instruction::F64Store(wasm_encoder::MemArg {
                                offset: 0,
                                align: 3,
                                memory_index: 0,
                            }));
                        }
                        ValType::F32 => {
                            func.instruction(&Instruction::F32Store(wasm_encoder::MemArg {
                                offset: 0,
                                align: 2,
                                memory_index: 0,
                            }));
                        }
                        ValType::I32 => {
                            func.instruction(&Instruction::I32Store(wasm_encoder::MemArg {
                                offset: 0,
                                align: 2,
                                memory_index: 0,
                            }));
                        }
                        _ => {
                            func.instruction(&Instruction::I64Store(wasm_encoder::MemArg {
                                offset: 0,
                                align: 3,
                                memory_index: 0,
                            }));
                        }
                    }
                }

                // 返回数组地址
                func.instruction(&Instruction::LocalGet(tmp_local));
            }
            Expr::Index { array, index } => {
                let array_ast_ty = self.infer_ast_type_with_locals(array, locals);
                // 提前提取 Tuple 字段的 WASM 类型（避免后续借用冲突）
                let tuple_field_wasm_ty: Option<ValType> =
                    if let Some(Type::Tuple(ref fields)) = array_ast_ty {
                        let fty = if let Expr::Integer(n) = index.as_ref() {
                            fields
                                .get(*n as usize)
                                .map(|t| t.to_wasm())
                                .unwrap_or(ValType::I64)
                        } else {
                            ValType::I64
                        };
                        Some(fty)
                    } else {
                        None
                    };

                if let Some(field_wasm_ty) = tuple_field_wasm_ty {
                    // Tuple 索引: 无长度字段头，步长固定 8 字节（i64 存储）
                    // tuple[i] -> load from (tuple_ptr + i * 8)
                    self.compile_expr(array, locals, func, loop_ctx);
                    // 如果 tuple 基址是 I64（未知类型回退），需要 wrap 为 I32
                    if self.infer_type_with_locals(array, locals) == ValType::I64 {
                        func.instruction(&Instruction::I32WrapI64);
                    }
                    self.compile_expr(index, locals, func, loop_ctx);
                    if self.infer_type_with_locals(index, locals) == ValType::I64 {
                        func.instruction(&Instruction::I32WrapI64);
                    }
                    func.instruction(&Instruction::I32Const(8));
                    func.instruction(&Instruction::I32Mul);
                    func.instruction(&Instruction::I32Add);
                    func.instruction(&Instruction::I64Load(wasm_encoder::MemArg {
                        offset: 0,
                        align: 3,
                        memory_index: 0,
                    }));
                    if field_wasm_ty == ValType::I32 {
                        func.instruction(&Instruction::I32WrapI64);
                    }
                } else {
                    // arr[i] -> load from (arr + 4 + i * elem_size)
                    // P3.9: 根据元素 WASM 类型确定步长和 Load 指令
                    let elem_ast_ty = match array_ast_ty {
                        Some(Type::Array(ref elem_ty)) => Some(elem_ty.as_ref().clone()),
                        _ => None,
                    };
                    let elem_wasm_ty = elem_ast_ty
                        .as_ref()
                        .map(|t| t.to_wasm())
                        .unwrap_or(ValType::I64);
                    let elem_size: i32 = match elem_wasm_ty {
                        ValType::I32 | ValType::F32 => 4,
                        _ => 8,
                    };
                    self.compile_expr(array, locals, func, loop_ctx);
                    // 如果数组基址是 I64（未知类型回退），需要 wrap 为 I32
                    if self.infer_type_with_locals(array, locals) == ValType::I64 {
                        func.instruction(&Instruction::I32WrapI64);
                    }
                    func.instruction(&Instruction::I32Const(4)); // 跳过长度字段
                    func.instruction(&Instruction::I32Add);
                    self.compile_expr(index, locals, func, loop_ctx);
                    // P3: 只对确认为 I64 的索引 wrap
                    if self.infer_type_with_locals(index, locals) == ValType::I64 {
                        func.instruction(&Instruction::I32WrapI64);
                    }
                    func.instruction(&Instruction::I32Const(elem_size));
                    func.instruction(&Instruction::I32Mul);
                    func.instruction(&Instruction::I32Add);
                    match elem_wasm_ty {
                        ValType::F64 => {
                            func.instruction(&Instruction::F64Load(wasm_encoder::MemArg {
                                offset: 0,
                                align: 3,
                                memory_index: 0,
                            }));
                        }
                        ValType::F32 => {
                            func.instruction(&Instruction::F32Load(wasm_encoder::MemArg {
                                offset: 0,
                                align: 2,
                                memory_index: 0,
                            }));
                        }
                        ValType::I32 => {
                            func.instruction(&Instruction::I32Load(wasm_encoder::MemArg {
                                offset: 0,
                                align: 2,
                                memory_index: 0,
                            }));
                        }
                        _ => {
                            // 元素存储为 I64（8 字节对齐）
                            func.instruction(&Instruction::I64Load(wasm_encoder::MemArg {
                                offset: 0,
                                align: 3,
                                memory_index: 0,
                            }));
                            // P4: 如果元素实际类型是 I32（如 TypeParam），需要 wrap
                            if elem_ast_ty.as_ref().map(|t| t.to_wasm()) == Some(ValType::I32) {
                                func.instruction(&Instruction::I32WrapI64);
                            }
                        }
                    }
                }
            }
            Expr::StructInit {
                name,
                type_args,
                fields,
            } => {
                let class_info = self.classes.get(name);
                let has_vtable = class_info.map_or(false, |ci| ci.has_vtable);
                let vtable_base = class_info.map_or(0, |ci| ci.vtable_base);
                let header_size = if has_vtable { 4u32 } else { 0 };
                // 优先查 struct，如果没有再查 class
                let (struct_size, is_class) = if let Some(struct_def) = self.structs.get(name) {
                    (header_size + struct_def.size(), false)
                } else if let Some(ci) = self.classes.get(name) {
                    // 类使用 all_fields 计算大小
                    let fields_size: u32 = ci.all_fields.iter().map(|f| f.ty.size()).sum();
                    (header_size + fields_size, true)
                } else {
                    panic!("结构体或类 {} 未定义", name);
                };

                // Phase 8: 使用 __alloc 分配内存
                let alloc_idx = self.func_indices["__alloc"];
                let tmp_local = locals
                    .get("__struct_alloc_ptr")
                    .expect("__struct_alloc_ptr 未预注册");

                func.instruction(&Instruction::I32Const(struct_size as i32));
                func.instruction(&Instruction::Call(alloc_idx));
                func.instruction(&Instruction::LocalSet(tmp_local));

                // 写入 vtable_ptr（如果有 vtable）
                if has_vtable {
                    func.instruction(&Instruction::LocalGet(tmp_local));
                    func.instruction(&Instruction::I32Const(vtable_base as i32));
                    func.instruction(&Instruction::I32Store(wasm_encoder::MemArg {
                        offset: 0,
                        align: 2,
                        memory_index: 0,
                    }));
                }

                // 写入每个字段（偏移需要加上 header）
                for (field_name, value) in fields {
                    // 计算字段偏移
                    let base_offset = if is_class {
                        if let Some(ci) = self.classes.get(name) {
                            ci.field_offset(field_name).unwrap_or(0)
                        } else {
                            panic!("类 {} 未找到", name)
                        }
                    } else {
                        let struct_def = self.structs.get(name).unwrap();
                        struct_def.field_offset(field_name).unwrap_or(0)
                    };
                    let offset = header_size + base_offset;

                    func.instruction(&Instruction::LocalGet(tmp_local));
                    func.instruction(&Instruction::I32Const(offset as i32));
                    func.instruction(&Instruction::I32Add);
                    self.compile_expr(value, locals, func, loop_ctx);

                    // 获取字段的实际类型定义
                    let field_ty = if is_class {
                        self.classes
                            .get(name)
                            .and_then(|ci| ci.field_type(field_name))
                            .cloned()
                            .unwrap_or(Type::Int64)
                    } else {
                        self.structs
                            .get(name)
                            .and_then(|sd| sd.field_type(field_name))
                            .cloned()
                            .unwrap_or(Type::Int64)
                    };

                    // 值类型与字段类型不同时做类型转换（如 TypeParam→I32 存入 I64 字段）
                    let field_val_type = self.infer_type_with_locals(value, locals);
                    self.emit_type_coercion(func, field_val_type, field_ty.to_wasm());
                    self.emit_store_by_type(func, &field_ty);
                }

                // 返回对象地址
                func.instruction(&Instruction::LocalGet(tmp_local));
            }
            Expr::ConstructorCall {
                name,
                type_args,
                args,
                ..
            } => {
                // Handle generic type arguments: Box<Int64> -> Box$Int64
                let mangled_name = if let Some(ref tas) = type_args {
                    if !tas.is_empty() {
                        crate::monomorph::mangle_name(name, tas)
                    } else {
                        name.clone()
                    }
                } else {
                    name.clone()
                };

                // Phase 7.5: 内置集合类型构造器（首字母大写，会被解析为 ConstructorCall）
                match name.as_str() {
                    "ArrayList" | "ArrayStack" if args.is_empty() => {
                        func.instruction(&Instruction::Call(self.func_indices["__arraylist_new"]));
                        return;
                    }
                    "HashMap" | "HashSet" if args.is_empty() => {
                        func.instruction(&Instruction::Call(self.func_indices["__hashmap_new"]));
                        return;
                    }
                    "LinkedList" if args.is_empty() => {
                        func.instruction(&Instruction::Call(self.func_indices["__linkedlist_new"]));
                        return;
                    }
                    // P5: AtomicInt64([initial]) — 单线程桩，分配 8 字节存储 i64
                    "AtomicInt64" => {
                        // 分配 8 字节
                        func.instruction(&Instruction::I32Const(8));
                        func.instruction(&Instruction::Call(self.func_indices["__alloc"]));
                        // 存初始值
                        if args.len() >= 1 {
                            // 复制指针，存初始值
                            let tmp = locals.get("__struct_alloc_ptr").unwrap_or(0);
                            func.instruction(&Instruction::LocalTee(tmp));
                            self.compile_expr(&args[0], locals, func, loop_ctx);
                            func.instruction(&Instruction::I64Store(wasm_encoder::MemArg {
                                offset: 0,
                                align: 3,
                                memory_index: 0,
                            }));
                            func.instruction(&Instruction::LocalGet(tmp));
                        } else {
                            // 默认值 0
                            let tmp = locals.get("__struct_alloc_ptr").unwrap_or(0);
                            func.instruction(&Instruction::LocalTee(tmp));
                            func.instruction(&Instruction::I64Const(0));
                            func.instruction(&Instruction::I64Store(wasm_encoder::MemArg {
                                offset: 0,
                                align: 3,
                                memory_index: 0,
                            }));
                            func.instruction(&Instruction::LocalGet(tmp));
                        }
                        return;
                    }
                    // P5: AtomicBool([initial]) — 单线程桩，分配 8 字节存储 i64(0/1)
                    "AtomicBool" => {
                        func.instruction(&Instruction::I32Const(8));
                        func.instruction(&Instruction::Call(self.func_indices["__alloc"]));
                        if args.len() >= 1 {
                            let tmp = locals.get("__struct_alloc_ptr").unwrap_or(0);
                            func.instruction(&Instruction::LocalTee(tmp));
                            self.compile_expr(&args[0], locals, func, loop_ctx);
                            func.instruction(&Instruction::I64Store(wasm_encoder::MemArg {
                                offset: 0,
                                align: 3,
                                memory_index: 0,
                            }));
                            func.instruction(&Instruction::LocalGet(tmp));
                        } else {
                            let tmp = locals.get("__struct_alloc_ptr").unwrap_or(0);
                            func.instruction(&Instruction::LocalTee(tmp));
                            func.instruction(&Instruction::I64Const(0));
                            func.instruction(&Instruction::I64Store(wasm_encoder::MemArg {
                                offset: 0,
                                align: 3,
                                memory_index: 0,
                            }));
                            func.instruction(&Instruction::LocalGet(tmp));
                        }
                        return;
                    }
                    // P5: Mutex() / ReentrantMutex() — 单线程桩，分配 4 字节 dummy
                    "Mutex" | "ReentrantMutex" if args.is_empty() => {
                        func.instruction(&Instruction::I32Const(4));
                        func.instruction(&Instruction::Call(self.func_indices["__alloc"]));
                        return;
                    }
                    // P2.7: Array<T>(size, init) 动态数组构造
                    "Array" if args.len() == 2 => {
                        let elem_size: i32 = 8; // i64/f64
                        let is_float = type_args.as_ref().map_or(false, |ta| {
                            ta.first()
                                .map_or(false, |t| matches!(t, Type::Float64 | Type::Float32))
                        });
                        let ptr_local = locals.get("__array_dyn_ptr").unwrap();
                        let size_local = locals.get("__array_dyn_size").unwrap();
                        let idx_local = locals.get("__array_dyn_idx").unwrap();
                        let alloc_idx = self.func_indices["__alloc"];

                        // 计算 size 并保存
                        self.compile_expr(&args[0], locals, func, loop_ctx);
                        func.instruction(&Instruction::LocalSet(size_local));

                        // 分配内存: 4 + size * 8
                        func.instruction(&Instruction::LocalGet(size_local));
                        func.instruction(&Instruction::I32WrapI64);
                        func.instruction(&Instruction::I32Const(elem_size));
                        func.instruction(&Instruction::I32Mul);
                        func.instruction(&Instruction::I32Const(4));
                        func.instruction(&Instruction::I32Add);
                        func.instruction(&Instruction::Call(alloc_idx));
                        func.instruction(&Instruction::LocalSet(ptr_local));

                        // 写入长度
                        func.instruction(&Instruction::LocalGet(ptr_local));
                        func.instruction(&Instruction::LocalGet(size_local));
                        func.instruction(&Instruction::I32WrapI64);
                        func.instruction(&Instruction::I32Store(wasm_encoder::MemArg {
                            offset: 0,
                            align: 2,
                            memory_index: 0,
                        }));

                        // 初始化元素: idx = 0
                        func.instruction(&Instruction::I64Const(0));
                        func.instruction(&Instruction::LocalSet(idx_local));

                        // loop: while idx < size
                        func.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
                        func.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));

                        // 条件: idx >= size → break
                        func.instruction(&Instruction::LocalGet(idx_local));
                        func.instruction(&Instruction::LocalGet(size_local));
                        func.instruction(&Instruction::I64GeS);
                        func.instruction(&Instruction::BrIf(1));

                        // 计算元素地址: ptr + 4 + idx * 8
                        func.instruction(&Instruction::LocalGet(ptr_local));
                        func.instruction(&Instruction::I32Const(4));
                        func.instruction(&Instruction::I32Add);
                        func.instruction(&Instruction::LocalGet(idx_local));
                        func.instruction(&Instruction::I32WrapI64);
                        func.instruction(&Instruction::I32Const(elem_size));
                        func.instruction(&Instruction::I32Mul);
                        func.instruction(&Instruction::I32Add);

                        // 计算 init 值（若 init 是 lambda 则调用 init(idx)）
                        let init_expr = &args[1];
                        if matches!(init_expr, Expr::Lambda { .. }) {
                            // Lambda 初始化: 先编译 lambda 使其索引上栈，然后调用 init(idx)
                            // 简化处理: 编译 init_expr 得到 table index，然后 call_indirect(idx)
                            func.instruction(&Instruction::LocalGet(idx_local));
                            self.compile_expr(init_expr, locals, func, loop_ctx);
                            // call_indirect with (i64) -> i64
                            let wasm_params = if is_float {
                                vec![ValType::I64]
                            } else {
                                vec![ValType::I64]
                            };
                            let wasm_results = if is_float {
                                vec![ValType::F64]
                            } else {
                                vec![ValType::I64]
                            };
                            let type_idx =
                                self.find_or_create_func_type_idx(&wasm_params, &wasm_results);
                            func.instruction(&Instruction::CallIndirect {
                                type_index: type_idx,
                                table_index: 0,
                            });
                        } else {
                            // 常量值初始化
                            self.compile_expr(init_expr, locals, func, loop_ctx);
                        }

                        // 存储元素
                        if is_float {
                            func.instruction(&Instruction::F64Store(wasm_encoder::MemArg {
                                offset: 0,
                                align: 3,
                                memory_index: 0,
                            }));
                        } else {
                            func.instruction(&Instruction::I64Store(wasm_encoder::MemArg {
                                offset: 0,
                                align: 3,
                                memory_index: 0,
                            }));
                        }

                        // idx += 1
                        func.instruction(&Instruction::LocalGet(idx_local));
                        func.instruction(&Instruction::I64Const(1));
                        func.instruction(&Instruction::I64Add);
                        func.instruction(&Instruction::LocalSet(idx_local));

                        func.instruction(&Instruction::Br(0)); // continue loop
                        func.instruction(&Instruction::End); // loop end
                        func.instruction(&Instruction::End); // block end

                        // 返回数组指针
                        func.instruction(&Instruction::LocalGet(ptr_local));
                        return;
                    }
                    _ => {}
                }
                // abstract 类不能直接实例化
                if let Some(ci) = self.classes.get(&mangled_name) {
                    if ci.is_abstract {
                        panic!("abstract 类 {} 不能直接实例化", mangled_name);
                    }
                }
                // 检查类是否有 init 函数
                let init_func_name = format!("__{}_init", mangled_name);
                if self.func_indices.contains_key(&init_func_name) {
                    // 调用 __ClassName_init(args...) 返回对象指针
                    for arg in args {
                        self.compile_expr(arg, locals, func, loop_ctx);
                    }
                    let idx = self.func_indices[&init_func_name];
                    func.instruction(&Instruction::Call(idx));
                } else if self.classes.contains_key(&mangled_name) {
                    // 类作为 StructInit 处理
                    let class_info = self.classes.get(&mangled_name).unwrap();
                    let fields: Vec<(String, Expr)> = class_info
                        .all_fields
                        .iter()
                        .map(|f| f.name.clone())
                        .zip(args.clone())
                        .collect();
                    let init_expr = Expr::StructInit {
                        name: mangled_name.clone(),
                        type_args: None,
                        fields,
                    };
                    self.compile_expr(&init_expr, locals, func, loop_ctx);
                } else {
                    // 无 init: 回退到 StructInit
                    // First check if it's a class
                    if self.classes.contains_key(&mangled_name) {
                        // This shouldn't happen since we handled it above, but just in case
                        eprintln!("警告: 类 {} 在 struct 处理中被发现", mangled_name);
                        func.instruction(&Instruction::I32Const(0));
                        return;
                    }
                    // Check structs
                    if let Some(struct_def) = self.structs.get(&mangled_name) {
                        let fields: Vec<(String, Expr)> = struct_def
                            .fields
                            .iter()
                            .map(|f| f.name.clone())
                            .zip(args.clone())
                            .collect();
                        let init_expr = Expr::StructInit {
                            name: mangled_name.clone(),
                            type_args: None,
                            fields,
                        };
                        self.compile_expr(&init_expr, locals, func, loop_ctx);
                    } else {
                        eprintln!("警告: 类型 {} 未定义，生成零值", mangled_name);
                        func.instruction(&Instruction::I32Const(0));
                    }
                }
            }
            Expr::Field { object, field } => {
                // Phase 7.2: 内建类型属性拦截
                let obj_ast_type = self.infer_ast_type_with_locals(object, locals);
                let is_array_type = matches!(obj_ast_type.as_ref(), Some(Type::Array(_)));
                let is_collection_type = is_array_type
                    || matches!(obj_ast_type.as_ref(), Some(Type::Map(..)))
                    || obj_ast_type.as_ref() == Some(&Type::String);
                if field == "size" && is_collection_type {
                    // String.size / Array.size / HashMap.size / HashSet.size / ArrayList.size
                    // → 读取指针处的 i32 长度字段，扩展为 i64
                    self.compile_expr(object, locals, func, loop_ctx);
                    func.instruction(&Instruction::I32Load(wasm_encoder::MemArg {
                        offset: 0,
                        align: 2,
                        memory_index: 0,
                    }));
                    func.instruction(&Instruction::I64ExtendI32S);
                } else if obj_ast_type == Some(Type::Range) {
                    // P4: Range 属性 .start, .end, .step
                    // Range 内存布局: [start: i64][end: i64][inclusive: i32] = 20 bytes
                    self.compile_expr(object, locals, func, loop_ctx);
                    match field.as_str() {
                        "start" => {
                            func.instruction(&Instruction::I64Load(wasm_encoder::MemArg {
                                offset: 0,
                                align: 3,
                                memory_index: 0,
                            }));
                        }
                        "end" => {
                            func.instruction(&Instruction::I64Load(wasm_encoder::MemArg {
                                offset: 8,
                                align: 3,
                                memory_index: 0,
                            }));
                        }
                        "step" => {
                            // Range 没有步长字段，默认返回 1
                            func.instruction(&Instruction::Drop);
                            func.instruction(&Instruction::I64Const(1));
                        }
                        _ => {
                            func.instruction(&Instruction::Drop);
                            func.instruction(&Instruction::I64Const(0));
                        }
                    }
                } else {
                    // Bug B2 修复: 检查是否有 prop getter 方法
                    let getter_name = self.get_object_type(object, locals).and_then(|ty| {
                        if let Type::Struct(ref name, _) = ty {
                            let getter = format!("{}.__get_{}", name, field);
                            if self.func_indices.contains_key(&getter) {
                                Some(getter)
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    });
                    if let Some(getter_func_name) = getter_name {
                        // prop getter: 编译为 ClassName.__get_propName(object)
                        self.compile_expr(object, locals, func, loop_ctx);
                        let getter_idx = self.func_indices[&getter_func_name];
                        func.instruction(&Instruction::Call(getter_idx));
                    } else {
                        self.compile_expr(object, locals, func, loop_ctx);
                        let (offset, field_ty) = self
                            .get_object_type(object, locals)
                            .and_then(|ty| match ty {
                                Type::Struct(ref name, ref type_args) => {
                                    // 泛型类型需要查找修饰后的名字，如 Stack + [Int64] → Stack$Int64
                                    let lookup_name = if !type_args.is_empty() {
                                        let mangled =
                                            crate::monomorph::mangle_name(name, type_args);
                                        if self.classes.contains_key(&mangled)
                                            || self.structs.contains_key(&mangled)
                                        {
                                            mangled
                                        } else {
                                            name.clone()
                                        }
                                    } else {
                                        name.clone()
                                    };
                                    // 优先从 ClassInfo 获取偏移（包含 vtable header）
                                    if let Some(ci) = self.classes.get(&lookup_name) {
                                        let off = ci.field_offset(field)?;
                                        let ft = ci.field_type(field)?.clone();
                                        Some((off, ft))
                                    } else {
                                        self.structs.get(&lookup_name).and_then(|def| {
                                            let off = def.field_offset(field)?;
                                            let ft = def.field_type(field)?.clone();
                                            Some((off, ft))
                                        })
                                    }
                                }
                                _ => None,
                            })
                            .unwrap_or_else(|| {
                                // 回退：尝试从字段名推断类型
                                // 常见的数组/集合字段名通常是复数或包含特定关键词
                                let is_likely_array = field.ends_with("s")
                                    || field.contains("list")
                                    || field.contains("array")
                                    || field.contains("items")
                                    || field.contains("elements")
                                    || field.contains("Interfaces"); // supportedInterfaces
                                if is_likely_array {
                                    (0, Type::Array(Box::new(Type::Int64))) // 假设是数组指针
                                } else {
                                    (0, Type::Int64) // 默认 i64
                                }
                            });
                        func.instruction(&Instruction::I32Const(offset as i32));
                        func.instruction(&Instruction::I32Add);
                        self.emit_load_by_type(func, &field_ty);
                    } // end else (non-prop field)
                } // end else (non-builtin field)
            }
            Expr::Block(stmts, result) => {
                for stmt in stmts {
                    self.compile_stmt(stmt, locals, func, loop_ctx);
                }
                if let Some(expr) = result {
                    self.compile_expr(expr, locals, func, loop_ctx);
                }
            }
            Expr::Range {
                start,
                end,
                inclusive,
                ..
            } => {
                // Phase 8: 使用 __alloc 分配 Range 内存
                let range_size = Type::range_heap_size();
                let alloc_idx = self.func_indices["__alloc"];
                let tmp_local = locals
                    .get("__range_alloc_ptr")
                    .expect("__range_alloc_ptr 未预注册");

                func.instruction(&Instruction::I32Const(range_size as i32));
                func.instruction(&Instruction::Call(alloc_idx));
                func.instruction(&Instruction::LocalSet(tmp_local));

                // 存储 start 到 offset 0
                func.instruction(&Instruction::LocalGet(tmp_local));
                self.compile_expr(start, locals, func, loop_ctx);
                func.instruction(&Instruction::I64Store(wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                }));

                // 存储 end 到 offset 8
                func.instruction(&Instruction::LocalGet(tmp_local));
                self.compile_expr(end, locals, func, loop_ctx);
                func.instruction(&Instruction::I64Store(wasm_encoder::MemArg {
                    offset: 8,
                    align: 3,
                    memory_index: 0,
                }));

                // 存储 inclusive 到 offset 16
                func.instruction(&Instruction::LocalGet(tmp_local));
                func.instruction(&Instruction::I32Const(if *inclusive { 1 } else { 0 }));
                func.instruction(&Instruction::I32Store(wasm_encoder::MemArg {
                    offset: 16,
                    align: 2,
                    memory_index: 0,
                }));

                // 返回 Range 地址
                func.instruction(&Instruction::LocalGet(tmp_local));

                // 栈上留下指针（之前已经压入）
            }
            Expr::VariantConst {
                enum_name,
                variant_name,
                arg,
            } => {
                let enum_def_opt = self.enums.get(enum_name);
                if enum_def_opt.is_none() {
                    eprintln!("[警告] 枚举未找到: {} - 生成桩代码", enum_name);
                    func.instruction(&Instruction::I32Const(0));
                    return;
                }
                let enum_def = enum_def_opt.unwrap();
                let disc = enum_def.variant_index(variant_name).unwrap_or(0) as i32;

                if enum_def.has_payload() {
                    // Phase 8: 使用 __alloc 分配枚举内存
                    let payload_size = enum_def.payload_size().max(8) as i32;
                    let total_size = 4 + payload_size;
                    let alloc_idx = self.func_indices["__alloc"];
                    let tmp_local = locals
                        .get("__enum_alloc_ptr")
                        .expect("__enum_alloc_ptr 未预注册");

                    func.instruction(&Instruction::I32Const(total_size));
                    func.instruction(&Instruction::Call(alloc_idx));
                    func.instruction(&Instruction::LocalSet(tmp_local));

                    // 写入判别式
                    func.instruction(&Instruction::LocalGet(tmp_local));
                    func.instruction(&Instruction::I32Const(disc));
                    func.instruction(&Instruction::I32Store(wasm_encoder::MemArg {
                        offset: 0,
                        align: 2,
                        memory_index: 0,
                    }));

                    if let Some(ref payload_expr) = arg {
                        // 克隆 payload_ty 以避免 enum_def 借用冲突
                        let payload_ty = enum_def
                            .variant_payload(variant_name)
                            .expect("带关联值变体需提供参数")
                            .clone();
                        func.instruction(&Instruction::LocalGet(tmp_local));
                        func.instruction(&Instruction::I32Const(4));
                        func.instruction(&Instruction::I32Add);
                        self.compile_expr(payload_expr, locals, func, loop_ctx);

                        self.emit_store_by_type(func, &payload_ty);
                    }

                    // 返回枚举地址
                    func.instruction(&Instruction::LocalGet(tmp_local));
                } else {
                    if arg.is_some() {
                        panic!("简单枚举变体不能带关联值: {}.{}", enum_name, variant_name);
                    }
                    func.instruction(&Instruction::I32Const(disc));
                }
            }
            Expr::Match { expr, arms } => {
                // 检测无主体 match { case expr => ... }（subject 为 Block([], None) 哨兵值）
                let is_no_subject =
                    matches!(expr.as_ref(), Expr::Block(stmts, None) if stmts.is_empty());

                let subject_ty = self.infer_type_with_locals(expr, locals);
                let subject_ast_type = self.infer_ast_type_with_locals(expr, locals);

                let result_type = if arms.is_empty() {
                    wasm_encoder::BlockType::Empty
                } else if !self.expr_produces_value(&arms[0].body) {
                    // P2.1: match arms 为 void 表达式（如 println）时，block 不产生值
                    wasm_encoder::BlockType::Empty
                } else {
                    // 优先使用 AST 类型推断：TypeParam 视为 I32（对象引用指针）
                    let wasm_ty = match self
                        .infer_ast_type_with_locals(&arms[0].body, locals)
                        .as_ref()
                    {
                        Some(Type::TypeParam(_)) => ValType::I32,
                        Some(t) if !matches!(t, Type::Unit | Type::Nothing) => t.to_wasm(),
                        _ => self.infer_type_with_locals(&arms[0].body, locals),
                    };
                    wasm_encoder::BlockType::Result(wasm_ty)
                };

                func.instruction(&Instruction::Block(result_type));

                if is_no_subject {
                    // 无主体 match：每个 arm 的 Pattern::Guard(cond) 作为布尔条件
                    // 编译为 if-else 链，不需要 subject 求值
                    for (i, arm) in arms.iter().enumerate() {
                        let is_last = i == arms.len() - 1;
                        match &arm.pattern {
                            Pattern::Guard(cond) => {
                                self.compile_expr(cond, locals, func, loop_ctx);
                                func.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
                                self.compile_expr(&arm.body, locals, func, loop_ctx);
                                func.instruction(&Instruction::Br(1));
                                func.instruction(&Instruction::End);
                                if is_last {
                                    Self::emit_match_default_value(func, result_type);
                                }
                            }
                            Pattern::Wildcard => {
                                if let Some(ref guard) = arm.guard {
                                    self.compile_expr(guard, locals, func, loop_ctx);
                                    func.instruction(&Instruction::If(
                                        wasm_encoder::BlockType::Empty,
                                    ));
                                    self.compile_expr(&arm.body, locals, func, loop_ctx);
                                    func.instruction(&Instruction::Br(1));
                                    func.instruction(&Instruction::End);
                                    if is_last {
                                        Self::emit_match_default_value(func, result_type);
                                    }
                                } else {
                                    // _ with no guard: default case, always matches
                                    self.compile_expr(&arm.body, locals, func, loop_ctx);
                                    func.instruction(&Instruction::Br(0));
                                }
                            }
                            _ => {
                                // 其他模式在无主体 match 中当做 default 处理
                                self.compile_expr(&arm.body, locals, func, loop_ctx);
                                func.instruction(&Instruction::Br(0));
                            }
                        }
                    }
                    func.instruction(&Instruction::End);
                    return;
                }

                // subject 表达式必须在 block 内部求值，否则值在 block 栈帧之下不可访问
                self.compile_expr(expr, locals, func, loop_ctx);

                for (i, arm) in arms.iter().enumerate() {
                    let is_last = i == arms.len() - 1;
                    let has_guard = arm.guard.is_some();

                    match &arm.pattern {
                        Pattern::Guard(cond) => {
                            // Guard pattern in regular match: drop subject, evaluate condition
                            func.instruction(&Instruction::Drop);
                            self.compile_expr(cond, locals, func, loop_ctx);
                            func.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
                            self.compile_expr(&arm.body, locals, func, loop_ctx);
                            func.instruction(&Instruction::Br(1));
                            func.instruction(&Instruction::End);
                            if is_last {
                                Self::emit_match_default_value(func, result_type);
                            } else {
                                self.compile_expr(expr, locals, func, loop_ctx);
                            }
                        }
                        Pattern::Wildcard => {
                            func.instruction(&Instruction::Drop);
                            if has_guard {
                                // _ if cond => body
                                self.compile_expr(
                                    arm.guard.as_ref().unwrap(),
                                    locals,
                                    func,
                                    loop_ctx,
                                );
                                func.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
                                self.compile_expr(&arm.body, locals, func, loop_ctx);
                                func.instruction(&Instruction::Br(1));
                                func.instruction(&Instruction::End);
                                if is_last {
                                    Self::emit_match_default_value(func, result_type);
                                } else {
                                    self.compile_expr(expr, locals, func, loop_ctx);
                                }
                            } else {
                                self.compile_expr(&arm.body, locals, func, loop_ctx);
                                func.instruction(&Instruction::Br(0));
                            }
                        }
                        Pattern::Literal(lit) => {
                            match lit {
                                Literal::Integer(n) => {
                                    if subject_ty == ValType::I32 {
                                        func.instruction(&Instruction::I32Const(*n as i32));
                                        func.instruction(&Instruction::I32Eq);
                                    } else {
                                        func.instruction(&Instruction::I64Const(*n));
                                        func.instruction(&Instruction::I64Eq);
                                    }
                                }
                                Literal::Bool(b) => {
                                    func.instruction(&Instruction::I32Const(if *b {
                                        1
                                    } else {
                                        0
                                    }));
                                    func.instruction(&Instruction::I32Eq);
                                }
                                _ => {}
                            }

                            // 如果有 guard，需要额外检查
                            if has_guard {
                                func.instruction(&Instruction::If(
                                    wasm_encoder::BlockType::Result(ValType::I32),
                                ));
                                self.compile_expr(
                                    arm.guard.as_ref().unwrap(),
                                    locals,
                                    func,
                                    loop_ctx,
                                );
                                func.instruction(&Instruction::Else);
                                func.instruction(&Instruction::I32Const(0));
                                func.instruction(&Instruction::End);
                            }

                            func.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
                            self.compile_expr(&arm.body, locals, func, loop_ctx);
                            func.instruction(&Instruction::Br(1));
                            func.instruction(&Instruction::End);
                            if is_last {
                                Self::emit_match_default_value(func, result_type);
                            } else {
                                self.compile_expr(expr, locals, func, loop_ctx);
                            }
                        }
                        Pattern::Binding(name) => {
                            // Bug B1 修复: 检查是否为未限定的枚举变体名（如 `case RED` 而非 `case Color.RED`）
                            let enum_variant_disc =
                                if let Some(Type::Struct(ref enum_type_name, _)) = subject_ast_type
                                {
                                    self.enums
                                        .get(enum_type_name)
                                        .and_then(|e| e.variant_index(name))
                                        .map(|idx| idx as i32)
                                } else {
                                    None
                                };

                            if let Some(expected_disc) = enum_variant_disc {
                                // 作为枚举变体比较（而非变量绑定）
                                func.instruction(&Instruction::I32Const(expected_disc));
                                func.instruction(&Instruction::I32Eq);

                                if has_guard {
                                    func.instruction(&Instruction::If(
                                        wasm_encoder::BlockType::Result(ValType::I32),
                                    ));
                                    self.compile_expr(
                                        arm.guard.as_ref().unwrap(),
                                        locals,
                                        func,
                                        loop_ctx,
                                    );
                                    func.instruction(&Instruction::Else);
                                    func.instruction(&Instruction::I32Const(0));
                                    func.instruction(&Instruction::End);
                                }

                                func.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
                                self.compile_expr(&arm.body, locals, func, loop_ctx);
                                func.instruction(&Instruction::Br(1));
                                func.instruction(&Instruction::End);
                                if is_last {
                                    Self::emit_match_default_value(func, result_type);
                                } else {
                                    self.compile_expr(expr, locals, func, loop_ctx);
                                }
                            } else {
                                // 普通变量绑定
                                if let Some(idx) = locals.get(name) {
                                    if subject_ty == ValType::I32 {
                                        func.instruction(&Instruction::I64ExtendI32S);
                                    }
                                    func.instruction(&Instruction::LocalSet(idx));
                                }
                                if has_guard {
                                    self.compile_expr(
                                        arm.guard.as_ref().unwrap(),
                                        locals,
                                        func,
                                        loop_ctx,
                                    );
                                    func.instruction(&Instruction::If(
                                        wasm_encoder::BlockType::Empty,
                                    ));
                                    self.compile_expr(&arm.body, locals, func, loop_ctx);
                                    func.instruction(&Instruction::Br(1));
                                    func.instruction(&Instruction::End);
                                    if is_last {
                                        Self::emit_match_default_value(func, result_type);
                                    } else {
                                        self.compile_expr(expr, locals, func, loop_ctx);
                                    }
                                } else {
                                    self.compile_expr(&arm.body, locals, func, loop_ctx);
                                    func.instruction(&Instruction::Br(0));
                                }
                            }
                        }
                        Pattern::Variant {
                            enum_name,
                            variant_name,
                            payload,
                        } => {
                            // 判断是否为已知枚举（包含用户定义枚举 + 内建 Option/Result）
                            let handled = {
                                let is_user_enum = matches!(&subject_ast_type, Some(Type::Struct(ref name, _)) if name == enum_name && self.enums.contains_key(name));
                                let is_builtin_option =
                                    matches!(&subject_ast_type, Some(Type::Option(_)))
                                        && enum_name == "Option";
                                let is_builtin_result =
                                    matches!(&subject_ast_type, Some(Type::Result(_, _)))
                                        && enum_name == "Result";
                                (is_user_enum || is_builtin_option || is_builtin_result)
                                    && self.enums.contains_key(enum_name)
                                    && self.enums[enum_name].variant_index(variant_name).is_some()
                            };
                            if handled {
                                let enum_def = &self.enums[enum_name];
                                let expected_disc =
                                    enum_def.variant_index(variant_name).unwrap() as i32;
                                let has_variant_payload = enum_def.has_payload();
                                let resolved_payload = self.resolve_variant_payload(
                                    enum_name,
                                    variant_name,
                                    subject_ast_type.as_ref(),
                                );
                                let ptr_tmp =
                                    locals.get("__match_enum_ptr").expect("__match_enum_ptr");

                                if has_variant_payload {
                                    func.instruction(&Instruction::LocalSet(ptr_tmp));
                                    func.instruction(&Instruction::LocalGet(ptr_tmp));
                                    func.instruction(&Instruction::I32Load(wasm_encoder::MemArg {
                                        offset: 0,
                                        align: 2,
                                        memory_index: 0,
                                    }));
                                    func.instruction(&Instruction::I32Const(expected_disc));
                                    func.instruction(&Instruction::I32Eq);
                                } else {
                                    func.instruction(&Instruction::I32Const(expected_disc));
                                    func.instruction(&Instruction::I32Eq);
                                }

                                func.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
                                if has_variant_payload {
                                    if let Some(ref payload_pattern) = payload {
                                        if let Some(ref payload_ty) = resolved_payload {
                                            func.instruction(&Instruction::LocalGet(ptr_tmp));
                                            func.instruction(&Instruction::I32Const(4));
                                            func.instruction(&Instruction::I32Add);
                                            self.emit_load_by_type(func, payload_ty);
                                            self.compile_pattern_binding(
                                                payload_pattern,
                                                payload_ty,
                                                locals,
                                                func,
                                            );
                                        }
                                    }
                                }
                                if has_guard {
                                    self.compile_expr(
                                        arm.guard.as_ref().unwrap(),
                                        locals,
                                        func,
                                        loop_ctx,
                                    );
                                    func.instruction(&Instruction::If(
                                        wasm_encoder::BlockType::Empty,
                                    ));
                                    self.compile_expr(&arm.body, locals, func, loop_ctx);
                                    func.instruction(&Instruction::Br(2)); // 0=guard-if, 1=variant-if, 2=outer block
                                    func.instruction(&Instruction::End);
                                } else {
                                    self.compile_expr(&arm.body, locals, func, loop_ctx);
                                    func.instruction(&Instruction::Br(1)); // 0=variant-if, 1=outer block
                                }
                                func.instruction(&Instruction::End);
                                if is_last {
                                    Self::emit_match_default_value(func, result_type);
                                } else {
                                    self.compile_expr(expr, locals, func, loop_ctx);
                                }
                            } else {
                                func.instruction(&Instruction::Drop);
                                if has_guard {
                                    self.compile_expr(
                                        arm.guard.as_ref().unwrap(),
                                        locals,
                                        func,
                                        loop_ctx,
                                    );
                                    func.instruction(&Instruction::If(
                                        wasm_encoder::BlockType::Empty,
                                    ));
                                    self.compile_expr(&arm.body, locals, func, loop_ctx);
                                    func.instruction(&Instruction::Br(1));
                                    func.instruction(&Instruction::End);
                                    if is_last {
                                        Self::emit_match_default_value(func, result_type);
                                    } else {
                                        self.compile_expr(expr, locals, func, loop_ctx);
                                    }
                                } else {
                                    self.compile_expr(&arm.body, locals, func, loop_ctx);
                                    func.instruction(&Instruction::Br(0));
                                }
                            }
                        }
                        Pattern::Range {
                            start,
                            end,
                            inclusive,
                        } => {
                            if let (Literal::Integer(s), Literal::Integer(e)) = (start, end) {
                                func.instruction(&Instruction::I64Const(*s));
                                func.instruction(&Instruction::I64GeS);

                                self.compile_expr(expr, locals, func, loop_ctx);
                                func.instruction(&Instruction::I64Const(*e));
                                if *inclusive {
                                    func.instruction(&Instruction::I64LeS);
                                } else {
                                    func.instruction(&Instruction::I64LtS);
                                }

                                func.instruction(&Instruction::I32And);

                                if has_guard {
                                    func.instruction(&Instruction::If(
                                        wasm_encoder::BlockType::Result(ValType::I32),
                                    ));
                                    self.compile_expr(
                                        arm.guard.as_ref().unwrap(),
                                        locals,
                                        func,
                                        loop_ctx,
                                    );
                                    func.instruction(&Instruction::Else);
                                    func.instruction(&Instruction::I32Const(0));
                                    func.instruction(&Instruction::End);
                                }

                                func.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
                                self.compile_expr(&arm.body, locals, func, loop_ctx);
                                func.instruction(&Instruction::Br(1));
                                func.instruction(&Instruction::End);
                                if is_last {
                                    Self::emit_match_default_value(func, result_type);
                                } else {
                                    self.compile_expr(expr, locals, func, loop_ctx);
                                }
                            }
                        }
                        Pattern::Or(patterns) => {
                            for (j, pat) in patterns.iter().enumerate() {
                                if let Pattern::Literal(Literal::Integer(n)) = pat {
                                    if j > 0 {
                                        self.compile_expr(expr, locals, func, loop_ctx);
                                    }
                                    if subject_ty == ValType::I32 {
                                        func.instruction(&Instruction::I32Const(*n as i32));
                                        func.instruction(&Instruction::I32Eq);
                                    } else {
                                        func.instruction(&Instruction::I64Const(*n));
                                        func.instruction(&Instruction::I64Eq);
                                    }
                                    if j > 0 {
                                        func.instruction(&Instruction::I32Or);
                                    }
                                }
                            }

                            if has_guard {
                                func.instruction(&Instruction::If(
                                    wasm_encoder::BlockType::Result(ValType::I32),
                                ));
                                self.compile_expr(
                                    arm.guard.as_ref().unwrap(),
                                    locals,
                                    func,
                                    loop_ctx,
                                );
                                func.instruction(&Instruction::Else);
                                func.instruction(&Instruction::I32Const(0));
                                func.instruction(&Instruction::End);
                            }

                            func.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
                            self.compile_expr(&arm.body, locals, func, loop_ctx);
                            func.instruction(&Instruction::Br(1));
                            func.instruction(&Instruction::End);
                            if is_last {
                                Self::emit_match_default_value(func, result_type);
                            } else {
                                self.compile_expr(expr, locals, func, loop_ctx);
                            }
                        }
                        Pattern::Struct {
                            name: struct_name,
                            fields,
                        } => {
                            let handled = if let Some(Type::Struct(ref sub_name, _)) =
                                subject_ast_type
                            {
                                sub_name == struct_name && self.structs.contains_key(struct_name)
                            } else {
                                false
                            };
                            if handled {
                                let struct_def = &self.structs[struct_name];
                                let ptr_tmp =
                                    locals.get("__match_enum_ptr").expect("__match_enum_ptr");
                                func.instruction(&Instruction::LocalSet(ptr_tmp));
                                for (fname, pat) in fields {
                                    let offset =
                                        struct_def.field_offset(fname).expect("结构体字段");
                                    let fty = struct_def.field_type(fname).expect("字段类型");
                                    func.instruction(&Instruction::LocalGet(ptr_tmp));
                                    func.instruction(&Instruction::I32Const(offset as i32));
                                    func.instruction(&Instruction::I32Add);
                                    self.emit_load_by_type(func, fty);
                                    if let Pattern::Binding(bind) = pat {
                                        let idx = locals.get(bind).expect("解构绑定名");
                                        func.instruction(&Instruction::LocalSet(idx));
                                    }
                                }
                                if has_guard {
                                    self.compile_expr(
                                        arm.guard.as_ref().unwrap(),
                                        locals,
                                        func,
                                        loop_ctx,
                                    );
                                    func.instruction(&Instruction::If(
                                        wasm_encoder::BlockType::Empty,
                                    ));
                                    self.compile_expr(&arm.body, locals, func, loop_ctx);
                                    func.instruction(&Instruction::Br(1)); // 0=guard-if, 1=outer block
                                    func.instruction(&Instruction::End);
                                    // guard 失败时，需要为下一个 arm 重新推送 subject 或默认值
                                    if is_last {
                                        Self::emit_match_default_value(func, result_type);
                                    } else {
                                        self.compile_expr(expr, locals, func, loop_ctx);
                                    }
                                } else {
                                    // struct 模式无 guard 总是匹配成功
                                    self.compile_expr(&arm.body, locals, func, loop_ctx);
                                    func.instruction(&Instruction::Br(0)); // 0=outer block
                                }
                            } else {
                                func.instruction(&Instruction::Drop);
                                if has_guard {
                                    self.compile_expr(
                                        arm.guard.as_ref().unwrap(),
                                        locals,
                                        func,
                                        loop_ctx,
                                    );
                                    func.instruction(&Instruction::If(
                                        wasm_encoder::BlockType::Empty,
                                    ));
                                    self.compile_expr(&arm.body, locals, func, loop_ctx);
                                    func.instruction(&Instruction::Br(1));
                                    func.instruction(&Instruction::End);
                                    if is_last {
                                        Self::emit_match_default_value(func, result_type);
                                    } else {
                                        self.compile_expr(expr, locals, func, loop_ctx);
                                    }
                                } else {
                                    self.compile_expr(&arm.body, locals, func, loop_ctx);
                                    func.instruction(&Instruction::Br(0));
                                }
                            }
                        }
                        // P3.5: 类型测试模式 x: Type
                        Pattern::TypeTest { binding, ty } => {
                            // 类型检查：根据 target_ty 做静态或动态对比
                            let matched = match ty {
                                Type::Struct(ref target_name, _) => {
                                    if let Some(ci) = self.classes.get(target_name) {
                                        let target_id = ci.class_id;
                                        // 对于类对象，检查 class_id（需要 i32 指针）
                                        // 复制 subject 用于类型检查
                                        let subject_wasm_ty =
                                            self.infer_type_with_locals(expr, locals);
                                        if subject_wasm_ty == ValType::I32 {
                                            // 加载对象的 class_id（vtable_ptr 处，对象偏移 0）
                                            func.instruction(&Instruction::I32Load(
                                                wasm_encoder::MemArg {
                                                    offset: 0,
                                                    align: 2,
                                                    memory_index: 0,
                                                },
                                            ));
                                            func.instruction(&Instruction::I32Const(
                                                target_id as i32,
                                            ));
                                            func.instruction(&Instruction::I32Eq);
                                        } else {
                                            // 类型不匹配，静态检查
                                            func.instruction(&Instruction::Drop);
                                            func.instruction(&Instruction::I32Const(0));
                                        }
                                        true
                                    } else {
                                        // 静态类型匹配（结构体）
                                        let src_ty = &subject_ast_type;
                                        let target = Some(ty.clone());
                                        func.instruction(&Instruction::Drop);
                                        func.instruction(&Instruction::I32Const(
                                            if src_ty == &target { 1 } else { 0 },
                                        ));
                                        true
                                    }
                                }
                                _ => {
                                    // 非 class 类型：静态匹配
                                    let src_ty = &subject_ast_type;
                                    let target = Some(ty.clone());
                                    func.instruction(&Instruction::Drop);
                                    func.instruction(&Instruction::I32Const(
                                        if src_ty == &target { 1 } else { 0 },
                                    ));
                                    true
                                }
                            };
                            if matched {
                                func.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
                                // 绑定 subject 到变量（重新加载）
                                self.compile_expr(expr, locals, func, loop_ctx);
                                if let Some(idx) = locals.get(binding) {
                                    func.instruction(&Instruction::LocalSet(idx));
                                }
                                self.compile_expr(&arm.body, locals, func, loop_ctx);
                                func.instruction(&Instruction::Br(1));
                                func.instruction(&Instruction::End);
                                // 类型不匹配时: reload subject for next arm
                                if is_last {
                                    Self::emit_match_default_value(func, result_type);
                                } else {
                                    self.compile_expr(expr, locals, func, loop_ctx);
                                }
                            }
                        }
                        _ => {
                            func.instruction(&Instruction::Drop);
                            self.compile_expr(&arm.body, locals, func, loop_ctx);
                        }
                    }
                }

                func.instruction(&Instruction::End);
            }
            Expr::Lambda {
                params,
                return_type,
                body,
            } => {
                // P2.3: Lambda 编译：返回 TABLE INDEX（用于 call_indirect）
                let lambda_idx = self.lambda_counter.get();
                self.lambda_counter.set(lambda_idx + 1);
                let lambda_name = format!("__lambda_{}", lambda_idx);

                if let Some(&table_idx) = self.lambda_table_indices.get(&lambda_name) {
                    func.instruction(&Instruction::I32Const(table_idx as i32));
                } else if let Some(&func_idx) = self.func_indices.get(&lambda_name) {
                    // fallback: 用 function index
                    func.instruction(&Instruction::I32Const(func_idx as i32));
                } else {
                    func.instruction(&Instruction::I32Const(0));
                }
            }
            Expr::Some(inner) => {
                // Option::Some(v) -> 堆分配 [tag=1: i32][value]
                // 返回指针
                let value_size = match self.infer_ast_type_with_locals(inner, locals) {
                    Some(t) => t.size(),
                    None => 8, // 默认 i64
                };
                let total_size = 4 + value_size;

                func.instruction(&Instruction::GlobalGet(0)); // 保存指针

                // 写入 tag = 1 (Some)
                func.instruction(&Instruction::GlobalGet(0));
                func.instruction(&Instruction::I32Const(1));
                func.instruction(&Instruction::I32Store(wasm_encoder::MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                }));

                // 写入 value
                func.instruction(&Instruction::GlobalGet(0));
                func.instruction(&Instruction::I32Const(4)); // 跳过 tag
                func.instruction(&Instruction::I32Add);
                self.compile_expr(inner, locals, func, loop_ctx);
                let inner_wasm_ty = self.infer_type_with_locals(inner, locals);
                Self::emit_store_by_wasm_type(func, inner_wasm_ty);

                // 更新堆指针
                func.instruction(&Instruction::GlobalGet(0));
                func.instruction(&Instruction::I32Const(total_size as i32));
                func.instruction(&Instruction::I32Add);
                func.instruction(&Instruction::GlobalSet(0));
            }
            Expr::None => {
                // Option::None -> 堆分配 [tag=0: i32]
                func.instruction(&Instruction::GlobalGet(0)); // 保存指针

                func.instruction(&Instruction::GlobalGet(0));
                func.instruction(&Instruction::I32Const(0)); // tag = 0 (None)
                func.instruction(&Instruction::I32Store(wasm_encoder::MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                }));

                // 更新堆指针
                func.instruction(&Instruction::GlobalGet(0));
                func.instruction(&Instruction::I32Const(4));
                func.instruction(&Instruction::I32Add);
                func.instruction(&Instruction::GlobalSet(0));
            }
            Expr::Ok(inner) => {
                // Result::Ok(v) -> 堆分配 [tag=0: i32][value]
                let inner_ast_type = self.infer_ast_type_with_locals(inner, locals);
                let value_size = match &inner_ast_type {
                    Some(t) => t.size(),
                    None => 8,
                };
                let total_size = 4 + value_size;

                func.instruction(&Instruction::GlobalGet(0));

                func.instruction(&Instruction::GlobalGet(0));
                func.instruction(&Instruction::I32Const(0)); // tag = 0 (Ok)
                func.instruction(&Instruction::I32Store(wasm_encoder::MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                }));

                func.instruction(&Instruction::GlobalGet(0));
                func.instruction(&Instruction::I32Const(4));
                func.instruction(&Instruction::I32Add);
                self.compile_expr(inner, locals, func, loop_ctx);
                // 根据内部值类型选择正确的 store 指令
                // 使用 infer_type_with_locals 而非 infer_ast_type_with_locals
                // 以更好地处理方法链等复杂表达式
                let inner_wasm_ty = self.infer_type_with_locals(inner, locals);
                Self::emit_store_by_wasm_type(func, inner_wasm_ty);

                func.instruction(&Instruction::GlobalGet(0));
                func.instruction(&Instruction::I32Const(total_size as i32));
                func.instruction(&Instruction::I32Add);
                func.instruction(&Instruction::GlobalSet(0));
            }
            Expr::Err(inner) => {
                // Result::Err(e) -> 堆分配 [tag=1: i32][error]
                // Exception 总是对象指针 (i32)
                let total_size = 4 + 4; // tag (4 bytes) + pointer (4 bytes)

                func.instruction(&Instruction::GlobalGet(0));

                func.instruction(&Instruction::GlobalGet(0));
                func.instruction(&Instruction::I32Const(1)); // tag = 1 (Err)
                func.instruction(&Instruction::I32Store(wasm_encoder::MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                }));

                func.instruction(&Instruction::GlobalGet(0));
                func.instruction(&Instruction::I32Const(4));
                func.instruction(&Instruction::I32Add);
                self.compile_expr(inner, locals, func, loop_ctx);
                // Exception 对象总是 i32 指针
                func.instruction(&Instruction::I32Store(wasm_encoder::MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                }));

                func.instruction(&Instruction::GlobalGet(0));
                func.instruction(&Instruction::I32Const(8)); // 固定 8 字节 (tag + ptr)
                func.instruction(&Instruction::I32Add);
                func.instruction(&Instruction::GlobalSet(0));
            }
            Expr::Try(inner) => {
                // expr? -> 检查 tag，若为 None/Err 则提前 return，否则解包
                // 先计算 inner 得到指针
                self.compile_expr(inner, locals, func, loop_ctx);
                // 栈顶是指针，复制一份用于检查 tag
                func.instruction(&Instruction::LocalTee(locals.get("__try_ptr").unwrap_or(0)));
                // 读取 tag
                func.instruction(&Instruction::I32Load(wasm_encoder::MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                }));
                // 对于 Option: tag=0 是 None，需要提前返回
                // 对于 Result: tag=1 是 Err，需要提前返回
                // 简化：检查 tag != 0 (Some/Err)，若为 None/Ok 则继续
                // 注意：Option 的 tag=1 是 Some，Result 的 tag=0 是 Ok
                // 这里需要根据类型判断，简化处理：检查 tag
                func.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
                // tag != 0，需要提前返回
                func.instruction(&Instruction::LocalGet(locals.get("__try_ptr").unwrap_or(0)));
                func.instruction(&Instruction::Return);
                func.instruction(&Instruction::End);
                // tag == 0，解包 value
                func.instruction(&Instruction::LocalGet(locals.get("__try_ptr").unwrap_or(0)));
                func.instruction(&Instruction::I32Const(4));
                func.instruction(&Instruction::I32Add);
                func.instruction(&Instruction::I64Load(wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                }));
            }
            Expr::Throw(inner) => {
                // throw expr -> 设置错误标志并跳出 try 块
                // 如果在 try-catch 上下文中，设置 __err_flag 并将值存入 __err_val
                self.compile_expr(inner, locals, func, loop_ctx);
                if let Some(err_val_idx) = locals.get("__err_val") {
                    func.instruction(&Instruction::LocalSet(err_val_idx));
                    // 设置 __err_flag = 1
                    func.instruction(&Instruction::I32Const(1));
                    if let Some(err_flag_idx) = locals.get("__err_flag") {
                        func.instruction(&Instruction::LocalSet(err_flag_idx));
                    }
                } else {
                    // 不在 try 上下文中，直接 return
                    func.instruction(&Instruction::Return);
                }
            }
            Expr::Return(value) => {
                // return 在表达式上下文（如 match arm body）
                if let Some(expr) = value {
                    self.compile_expr(expr, locals, func, loop_ctx);
                }
                func.instruction(&Instruction::Return);
            }
            Expr::Break => {
                if let Some((break_depth, _)) = loop_ctx {
                    func.instruction(&Instruction::Br(break_depth));
                } else {
                    func.instruction(&Instruction::Unreachable);
                }
            }
            Expr::Continue => {
                if let Some((_, continue_depth)) = loop_ctx {
                    func.instruction(&Instruction::Br(continue_depth));
                } else {
                    func.instruction(&Instruction::Unreachable);
                }
            }
            Expr::TryBlock {
                resources,
                body,
                catch_var,
                catch_type,
                catch_body,
                finally_body,
            } => {
                // try (resources) { body } catch(e) { catch_body } finally { finally_body }
                // Compile resource initializations as let bindings before the try body
                for (res_name, res_expr) in resources {
                    self.compile_expr(res_expr, locals, func, loop_ctx);
                    if let Some(idx) = locals.get(res_name) {
                        func.instruction(&Instruction::LocalSet(idx));
                    } else {
                        func.instruction(&Instruction::Drop);
                    }
                }
                // Bug B6 修复: 支持 try-catch 作为表达式使用

                let err_flag = locals.get("__err_flag").unwrap_or(0);
                let err_val = locals.get("__err_val").unwrap_or(0);
                let try_result = locals.get("__try_result").unwrap_or(0);

                // 检查 try body 最后一条是否为表达式（即 try-catch 用作表达式）
                let produces_value = body.last().map_or(
                    false,
                    |s| matches!(s, Stmt::Expr(e) if self.expr_produces_value(e)),
                );

                // 初始化 __err_flag = 0
                func.instruction(&Instruction::I32Const(0));
                func.instruction(&Instruction::LocalSet(err_flag));

                // 用 block 包裹 try body，throw 后通过 br_if 跳出
                func.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
                let body_len = body.len();
                for (i, stmt) in body.iter().enumerate() {
                    let is_last = i == body_len - 1;
                    if is_last && produces_value {
                        // 最后一条表达式：编译并存入 __try_result
                        if let Stmt::Expr(e) = stmt {
                            self.compile_expr(e, locals, func, loop_ctx);
                            func.instruction(&Instruction::LocalSet(try_result));
                        } else {
                            self.compile_stmt(stmt, locals, func, loop_ctx);
                        }
                    } else {
                        self.compile_stmt(stmt, locals, func, loop_ctx);
                    }
                    // throw 后 __err_flag=1，br_if 跳出 try block
                    func.instruction(&Instruction::LocalGet(err_flag));
                    func.instruction(&Instruction::BrIf(0));
                }
                func.instruction(&Instruction::End); // end of try body block

                // 编译 catch 块（在 throw 发生时执行）
                func.instruction(&Instruction::LocalGet(err_flag));
                // 修复：当 try-catch 产生值时，catch 块的 if 应该使用 Result 类型
                let catch_block_type = if produces_value {
                    wasm_encoder::BlockType::Result(ValType::I64)
                } else {
                    wasm_encoder::BlockType::Empty
                };
                func.instruction(&Instruction::If(catch_block_type));
                if let Some(ref var) = catch_var {
                    if let Some(var_idx) = locals.get(var) {
                        func.instruction(&Instruction::LocalGet(err_val));
                        func.instruction(&Instruction::LocalSet(var_idx));
                    }
                }
                let catch_len = catch_body.len();
                for (i, stmt) in catch_body.iter().enumerate() {
                    let is_last = i == catch_len - 1;
                    if is_last && produces_value {
                        // catch 最后一条表达式也存入 __try_result
                        if let Stmt::Expr(e) = stmt {
                            self.compile_expr(e, locals, func, loop_ctx);
                            func.instruction(&Instruction::LocalSet(try_result));
                        } else {
                            self.compile_stmt(stmt, locals, func, loop_ctx);
                        }
                    } else {
                        self.compile_stmt(stmt, locals, func, loop_ctx);
                    }
                }
                func.instruction(&Instruction::End); // end of catch if

                // 编译 finally 块
                if let Some(finally_stmts) = finally_body {
                    for stmt in finally_stmts {
                        self.compile_stmt(stmt, locals, func, loop_ctx);
                    }
                }

                // Bug B6: 如果 try-catch 产生值，加载 __try_result
                if produces_value {
                    func.instruction(&Instruction::LocalGet(try_result));
                }
            }
            Expr::SliceExpr { array, start, end } => {
                // arr[start..end] → 新数组，与 Array.slice(start, end) 相同逻辑
                let elem_size: i32 = 8;
                let src_local = locals
                    .get("__array_clone_src")
                    .unwrap_or_else(|| locals.get("__array_alloc_ptr").unwrap());
                let alloc_idx = self.func_indices["__alloc"];

                self.compile_expr(array, locals, func, loop_ctx);
                func.instruction(&Instruction::LocalSet(src_local));

                let start_local = locals.get("__array_dyn_idx").unwrap();
                let end_local = locals.get("__array_dyn_size").unwrap();
                self.compile_expr(start, locals, func, loop_ctx);
                func.instruction(&Instruction::LocalSet(start_local));
                self.compile_expr(end, locals, func, loop_ctx);
                func.instruction(&Instruction::LocalSet(end_local));

                let new_len_local = locals.get("__array_dyn_ptr").unwrap();
                func.instruction(&Instruction::LocalGet(end_local));
                func.instruction(&Instruction::LocalGet(start_local));
                func.instruction(&Instruction::I64Sub);
                func.instruction(&Instruction::I32WrapI64);
                func.instruction(&Instruction::LocalSet(new_len_local));

                func.instruction(&Instruction::LocalGet(new_len_local));
                func.instruction(&Instruction::I32Const(elem_size));
                func.instruction(&Instruction::I32Mul);
                func.instruction(&Instruction::I32Const(4));
                func.instruction(&Instruction::I32Add);
                func.instruction(&Instruction::Call(alloc_idx));

                let dst_local = locals
                    .get("__array_clone_dst")
                    .unwrap_or_else(|| locals.get("__array_alloc_ptr").unwrap());
                func.instruction(&Instruction::LocalSet(dst_local));

                func.instruction(&Instruction::LocalGet(dst_local));
                func.instruction(&Instruction::LocalGet(new_len_local));
                func.instruction(&Instruction::I32Store(wasm_encoder::MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                }));

                func.instruction(&Instruction::LocalGet(dst_local));
                func.instruction(&Instruction::I32Const(4));
                func.instruction(&Instruction::I32Add);
                func.instruction(&Instruction::LocalGet(src_local));
                func.instruction(&Instruction::I32Const(4));
                func.instruction(&Instruction::I32Add);
                func.instruction(&Instruction::LocalGet(start_local));
                func.instruction(&Instruction::I32WrapI64);
                func.instruction(&Instruction::I32Const(elem_size));
                func.instruction(&Instruction::I32Mul);
                func.instruction(&Instruction::I32Add);
                func.instruction(&Instruction::LocalGet(new_len_local));
                func.instruction(&Instruction::I32Const(elem_size));
                func.instruction(&Instruction::I32Mul);
                func.instruction(&Instruction::MemoryCopy {
                    src_mem: 0,
                    dst_mem: 0,
                });

                func.instruction(&Instruction::LocalGet(dst_local));
            }
            Expr::MapLiteral { .. } => {
                todo!("MapLiteral codegen not yet implemented")
            }
            // P5.1: spawn { block } — 单线程桩实现（直接同步执行 block）
            Expr::Spawn { body } => {
                for stmt in body {
                    self.compile_stmt(stmt, locals, func, loop_ctx);
                }
                // spawn 不产生有意义的返回值
            }
            // P5.2: synchronized(lock) { block } — 单线程桩实现（直接执行 block）
            Expr::Synchronized { lock, body } => {
                // 编译 lock 表达式（可能有副作用），然后 drop
                self.compile_expr(lock, locals, func, loop_ctx);
                if self.expr_produces_value(lock) {
                    func.instruction(&Instruction::Drop);
                }
                for stmt in body {
                    self.compile_stmt(stmt, locals, func, loop_ctx);
                }
            }
            // P6.1: 可选链 obj?.field — 若 obj 为 0 (None 指针) 返回 0，否则访问字段
            Expr::OptionalChain { object, field } => {
                // 推断字段类型以确定结果 WASM 类型
                let field_access = Expr::Field {
                    object: object.clone(),
                    field: field.clone(),
                };
                let result_type = self.infer_type_with_locals(&field_access, locals);

                self.compile_expr(object, locals, func, loop_ctx);
                let match_val = locals.get("__match_val").unwrap_or(0);
                func.instruction(&Instruction::LocalTee(match_val));
                func.instruction(&Instruction::LocalGet(match_val));
                func.instruction(&Instruction::I32Eqz);
                // If None (0), leave 0 on stack; else access field
                func.instruction(&Instruction::If(wasm_encoder::BlockType::Result(
                    result_type,
                )));
                match result_type {
                    ValType::I64 => func.instruction(&Instruction::I64Const(0)),
                    ValType::F32 => func.instruction(&Instruction::F32Const(0.0)),
                    ValType::F64 => func.instruction(&Instruction::F64Const(0.0)),
                    _ => func.instruction(&Instruction::I32Const(0)),
                };
                func.instruction(&Instruction::Else);
                // 复用 Field 编译逻辑：构造 Var("__match_val").field 并编译
                let field_on_ptr = Expr::Field {
                    object: Box::new(Expr::Var("__match_val".to_string())),
                    field: field.clone(),
                };
                self.compile_expr(&field_on_ptr, locals, func, loop_ctx);
                func.instruction(&Instruction::End);
            }
            // 宏调用 @MacroName(args)
            Expr::Macro { name, args } => {
                self.compile_macro_call(name, args, locals, func, loop_ctx);
            }
            // P6.2: 尾随闭包 f(args) { params => body } — 展开为 f(args, closure)
            Expr::TrailingClosure {
                callee,
                args,
                closure,
            } => {
                // Compile as a regular call with the closure appended as the last argument
                let mut all_args = args.clone();
                all_args.push(closure.as_ref().clone());
                let call_expr = match callee.as_ref() {
                    Expr::Var(name) => Expr::Call {
                        name: name.clone(),
                        type_args: None,
                        args: all_args,
                        named_args: vec![],
                    },
                    Expr::MethodCall { object, method, .. } => Expr::MethodCall {
                        object: object.clone(),
                        method: method.clone(),
                        args: all_args,
                        named_args: vec![],
                        type_args: None,
                    },
                    _ => Expr::Call {
                        name: "__trailing_closure_target".to_string(),
                        type_args: None,
                        args: all_args,
                        named_args: vec![],
                    },
                };
                self.compile_expr(&call_expr, locals, func, loop_ctx);
            }
        }
    }
}
