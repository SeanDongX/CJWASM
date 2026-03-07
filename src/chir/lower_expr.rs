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

    /// 局部变量索引 → WASM 类型（用于赋值时的类型强制转换）
    pub local_wasm_tys: HashMap<u32, ValType>,

    /// 函数名 → 函数索引
    func_indices: &'a HashMap<String, u32>,

    /// 函数名（含修饰名）→ 参数列表（含默认值），用于命名参数 default 补全
    func_params: &'a HashMap<String, Vec<crate::ast::Param>>,

    /// 结构体字段偏移
    struct_field_offsets: &'a HashMap<String, HashMap<String, u32>>,

    /// 类字段偏移
    class_field_offsets: &'a HashMap<String, HashMap<String, u32>>,

    /// 类字段完整信息（字段名 → 偏移 + 类型），用于隐式 this 字段访问
    /// class_name → field_name → (offset, field_type)
    pub class_field_info: &'a HashMap<String, HashMap<String, (u32, crate::ast::Type)>>,

    /// 当前类上下文（仅在类实例方法中非 None）：类名 + `this` 局部变量索引
    pub current_class: Option<(String, u32)>,

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
        func_params: &'a HashMap<String, Vec<crate::ast::Param>>,
        struct_field_offsets: &'a HashMap<String, HashMap<String, u32>>,
        class_field_offsets: &'a HashMap<String, HashMap<String, u32>>,
        class_field_info: &'a HashMap<String, HashMap<String, (u32, crate::ast::Type)>>,
    ) -> Self {
        LoweringContext {
            type_ctx,
            local_map: HashMap::new(),
            local_wasm_tys: HashMap::new(),
            func_indices,
            func_params,
            struct_field_offsets,
            class_field_offsets,
            class_field_info,
            current_class: None,
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

    /// 分配局部变量并记录类型（供赋值时进行类型强制转换）
    pub fn alloc_local_typed(&mut self, name: String, wasm_ty: ValType) -> u32 {
        let idx = self.alloc_local(name);
        self.local_wasm_tys.insert(idx, wasm_ty);
        idx
    }

    /// 获取局部变量的声明 WASM 类型
    pub fn get_local_ty(&self, idx: u32) -> Option<ValType> {
        self.local_wasm_tys.get(&idx).copied()
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
                } else if let Some((class_name, this_idx)) = self.current_class.clone() {
                    // 隐式 this 字段访问：类方法内直接引用字段名（等价于 this.field）
                    if let Some(fields) = self.class_field_info.get(&class_name) {
                        if let Some((offset, field_ty)) = fields.get(name) {
                            let this_expr = CHIRExpr::new(
                                CHIRExprKind::Local(this_idx),
                                crate::ast::Type::Struct(class_name.clone(), vec![]),
                                ValType::I32,
                            );
                            return Ok(CHIRExpr::new(
                                CHIRExprKind::FieldGet {
                                    object: Box::new(this_expr),
                                    field_offset: *offset,
                                    field_ty: field_ty.clone(),
                                },
                                field_ty.clone(),
                                field_ty.to_wasm(),
                            ));
                        }
                    }
                    // 不是类字段，视为全局
                    CHIRExprKind::Global(name.clone())
                } else {
                    // 全局变量或未定义
                    CHIRExprKind::Global(name.clone())
                }
            }

            // 管道操作符 a |> f：语义等同 f(a)，但涉及 iterator/closure 等尚不支持的特性
            // 不展开 left/right 以避免孤值堆积在栈上，直接返回结果类型的 Nop 零值占位
            Expr::Binary { op: crate::ast::BinOp::Pipeline, .. } => {
                CHIRExprKind::Nop
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
                        // 若 inner 是 Unit/Nothing Nop，直接返回目标类型的零值，
                        // 避免 Cast 包住空栈 Nop 导致 i64.extend_i32_s 空栈错误
                        if matches!(inner.ty, crate::ast::Type::Unit | crate::ast::Type::Nothing) {
                            let sub_ty = match to_wasm {
                                ValType::I64 => crate::ast::Type::Int64,
                                ValType::F32 => crate::ast::Type::Float32,
                                ValType::F64 => crate::ast::Type::Float64,
                                _ => crate::ast::Type::Int32,
                            };
                            return Ok(CHIRExpr::new(CHIRExprKind::Nop, sub_ty, to_wasm));
                        }
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

                // 内置 I/O 函数：生成 Print 节点，由 chir_codegen 负责发射 WASM fd_write 调用
                match name.as_str() {
                    "println" | "print" | "eprintln" | "eprint" => {
                        let newline = matches!(name.as_str(), "println" | "eprintln");
                        let fd = if matches!(name.as_str(), "eprint" | "eprintln") { 2 } else { 1 };
                        let arg = if args.is_empty() {
                            None
                        } else {
                            Some(Box::new(self.lower_expr(&args[0])?))
                        };
                        return Ok(CHIRExpr::new(
                            CHIRExprKind::Print { arg, newline, fd },
                            crate::ast::Type::Unit,
                            ValType::I32,
                        ));
                    }
                    "exit" | "panic" | "abort" => {
                        return Ok(CHIRExpr::new(
                            CHIRExprKind::Unreachable,
                            crate::ast::Type::Nothing,
                            ValType::I32,
                        ));
                    }
                    "readln" => {
                        // 返回 I32（字符串指针），用零值占位
                        return Ok(CHIRExpr::new(
                            CHIRExprKind::Nop,
                            crate::ast::Type::String,
                            ValType::I32,
                        ));
                    }
                    _ => {}
                }

                // 函数查找：优先按 "name$arity" 修饰名（支持重载），fallback 原名
                // 对 "ClassName.method" 形式，也尝试 arity+1（含 this 参数）
                let mangled = format!("{}${}", name, args.len());
                let mangled_plus_this = format!("{}${}", name, args.len() + 1);
                let (func_idx_opt, needs_this) = if let Some(&idx) = self.func_indices.get(mangled.as_str()) {
                    (Some(idx), false)
                } else if name.contains('.') {
                    if let Some(&idx) = self.func_indices.get(mangled_plus_this.as_str()) {
                        (Some(idx), true)
                    } else if let Some(&idx) = self.func_indices.get(name.as_str()) {
                        (Some(idx), false)
                    } else {
                        (None, false)
                    }
                } else {
                    (self.func_indices.get(name.as_str()).copied(), false)
                };
                let func_idx = match func_idx_opt {
                    Some(idx) => idx,
                    None => {
                        // 未知函数：生成与返回类型匹配的零值占位，避免误用 fd_write
                        return Ok(CHIRExpr::new(
                            CHIRExprKind::Nop,
                            ty,
                            wasm_ty,
                        ));
                    }
                };

                // 查询函数签名以获取参数类型，用于插入必要的类型转换
                // 优先用与实际 arity 匹配的签名（含 this 时 arity+1）
                let actual_arity = if needs_this { args.len() + 1 } else { args.len() };
                let param_tys: Vec<ValType> = {
                    let mangled_key = format!("{}${}", name, actual_arity);
                    self.type_ctx.functions
                        .get(mangled_key.as_str())
                        .or_else(|| self.type_ctx.functions.get(name.as_str()))
                        .map(|sig| sig.params.iter().map(|p| match p {
                            crate::ast::Type::Unit | crate::ast::Type::Nothing => ValType::I32,
                            t => t.to_wasm(),
                        }).collect())
                        .unwrap_or_default()
                };

                // 如果需要隐式 this 参数（类方法调用通过 Expr::Call），前插 this
                let mut args_chir: Vec<CHIRExpr> = Vec::new();
                if needs_this {
                    if let Some((ref class_name, this_idx)) = self.current_class {
                        let this_expr = CHIRExpr::new(
                            CHIRExprKind::Local(this_idx),
                            crate::ast::Type::Struct(class_name.clone(), vec![]),
                            ValType::I32,
                        );
                        args_chir.push(this_expr);
                    } else {
                        args_chir.push(CHIRExpr::new(CHIRExprKind::Nop, crate::ast::Type::Int32, ValType::I32));
                    }
                }
                let this_offset = args_chir.len();

                // lower 位置参数
                let positional_args: Vec<CHIRExpr> = args.iter()
                    .enumerate()
                    .map(|(i, a)| {
                        let param_idx = this_offset + i;
                        let mut arg_chir = self.lower_expr(a)?;
                        if let Some(&target_wasm) = param_tys.get(param_idx) {
                            if matches!(arg_chir.ty, crate::ast::Type::Unit | crate::ast::Type::Nothing) {
                                let sub_ty = match target_wasm {
                                    ValType::I64 => crate::ast::Type::Int64,
                                    ValType::F32 => crate::ast::Type::Float32,
                                    ValType::F64 => crate::ast::Type::Float64,
                                    _ => crate::ast::Type::Int32,
                                };
                                arg_chir = CHIRExpr::new(CHIRExprKind::Nop, sub_ty, target_wasm);
                            } else {
                                arg_chir = self.insert_cast_if_needed(arg_chir, target_wasm);
                            }
                        } else if matches!(arg_chir.ty, crate::ast::Type::Unit | crate::ast::Type::Nothing) {
                            arg_chir = CHIRExpr::new(CHIRExprKind::Nop, crate::ast::Type::Int32, ValType::I32);
                        }
                        Ok(arg_chir)
                    })
                    .collect::<Result<Vec<CHIRExpr>, String>>()?;
                args_chir.extend(positional_args);

                // 补充缺失的命名参数（有默认值的参数）
                // 先处理 named_args（按名称匹配），再为完全缺失的参数补零值
                let Expr::Call { named_args, .. } = expr else { unreachable!() };
                if args_chir.len() < param_tys.len() {
                    // 查找函数定义中的参数（含默认值）
                    let mangled_key = format!("{}${}", name, param_tys.len());
                    let func_param_defs = self.func_params
                        .get(mangled_key.as_str())
                        .or_else(|| self.func_params.get(name.as_str()));
                    if let Some(param_defs) = func_param_defs {
                        for param_def in param_defs.iter().skip(args_chir.len()) {
                            // 先查 named_args 中是否有此参数的值
                            let named_val = named_args.iter()
                                .find(|(n, _)| n == &param_def.name)
                                .map(|(_, v)| v);
                            let arg_chir = if let Some(val_expr) = named_val {
                                self.lower_expr(val_expr)?
                            } else if let Some(default_expr) = &param_def.default {
                                // 使用参数的默认值
                                self.lower_expr(default_expr)?
                            } else {
                                // 无默认值：生成对应类型零值
                                let wt = param_def.ty.to_wasm();
                                let sub_ty = match wt {
                                    ValType::I64 => crate::ast::Type::Int64,
                                    ValType::F32 => crate::ast::Type::Float32,
                                    ValType::F64 => crate::ast::Type::Float64,
                                    _ => crate::ast::Type::Int32,
                                };
                                CHIRExpr::new(CHIRExprKind::Nop, sub_ty, wt)
                            };
                            let target_wasm = param_tys[args_chir.len()];
                            let arg_chir = self.insert_cast_if_needed(arg_chir, target_wasm);
                            args_chir.push(arg_chir);
                        }
                    } else {
                        // 函数定义未找到：用零值填充缺失参数
                        for &target_wasm in &param_tys[args_chir.len()..] {
                            let sub_ty = match target_wasm {
                                ValType::I64 => crate::ast::Type::Int64,
                                ValType::F32 => crate::ast::Type::Float32,
                                ValType::F64 => crate::ast::Type::Float64,
                                _ => crate::ast::Type::Int32,
                            };
                            args_chir.push(CHIRExpr::new(CHIRExprKind::Nop, sub_ty, target_wasm));
                        }
                    }
                }

                CHIRExprKind::Call {
                    func_idx,
                    args: args_chir,
                }
            }

            // 方法调用：解析为 ClassName.methodName 的直接调用
            Expr::MethodCall { object, method, args, named_args, .. } => {
                // 推断 receiver 类型以确定类名
                let obj_ty = self.type_ctx.infer_expr(object)?;
                let class_name = match &obj_ty {
                    crate::ast::Type::Struct(name, _) => Some(name.clone()),
                    crate::ast::Type::Qualified(parts) => parts.last().cloned(),
                    _ => None,
                };

                if let Some(cls) = class_name {
                    let mangled_method = format!("{}.{}", cls, method);
                    // 尝试精确匹配（arity = 1 + args.len()，含 this）
                    let arity = 1 + args.len() + named_args.len();
                    let mangled_with_arity = format!("{}${}", mangled_method, arity);
                    let func_idx = self.func_indices.get(&mangled_with_arity)
                        .or_else(|| self.func_indices.get(&mangled_method))
                        .copied();

                    if let Some(func_idx) = func_idx {
                        // 查询方法签名以获取参数 WASM 类型（含 this）
                        let param_tys: Vec<ValType> = {
                            let mangled_key = format!("{}${}", mangled_method, arity);
                            self.type_ctx.functions
                                .get(mangled_key.as_str())
                                .or_else(|| self.type_ctx.functions.get(mangled_method.as_str()))
                                .map(|sig| sig.params.iter().map(|p| match p {
                                    crate::ast::Type::Unit | crate::ast::Type::Nothing => ValType::I32,
                                    t => t.to_wasm(),
                                }).collect())
                                .unwrap_or_default()
                        };

                        // receiver 作为第一个参数（this）
                        let receiver_chir = self.lower_expr(object)?;
                        let receiver_chir = self.insert_cast_if_needed(receiver_chir, ValType::I32);
                        let mut call_args = vec![receiver_chir];

                        // 普通参数（带类型对齐）
                        for (i, arg) in args.iter().enumerate() {
                            let mut arg_chir = self.lower_expr(arg)?;
                            // param_tys[0] 是 this，所以偏移 1
                            if let Some(&target_wasm) = param_tys.get(i + 1) {
                                arg_chir = self.insert_cast_if_needed(arg_chir, target_wasm);
                            }
                            call_args.push(arg_chir);
                        }
                        // 命名参数
                        for (j, (_, val)) in named_args.iter().enumerate() {
                            let mut val_chir = self.lower_expr(val)?;
                            let param_idx = 1 + args.len() + j;
                            if let Some(&target_wasm) = param_tys.get(param_idx) {
                                val_chir = self.insert_cast_if_needed(val_chir, target_wasm);
                            }
                            call_args.push(val_chir);
                        }

                        return Ok(CHIRExpr::new(
                            CHIRExprKind::Call { func_idx, args: call_args },
                            ty.clone(),
                            wasm_ty,
                        ));
                    }
                }

                // 未能解析：返回 Nop（方法不在已知类中，或 vtable 调用等待后续实现）
                CHIRExprKind::Nop
            }

            // 字段访问
            Expr::Field { object, field } => {
                let obj_chir = self.lower_expr(object)?;
                let obj_ty = self.type_ctx.infer_expr(object)?;

                // 获取字段偏移
                let offset = self.get_field_offset(&obj_ty, field)?;
                let field_ty = self.type_ctx.infer_field_type(&obj_ty, field)?;

                // 对象指针必须是 I32，否则 I32Add 会类型不匹配
                let obj_chir = self.insert_cast_if_needed(obj_chir, ValType::I32);

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
                // 数组指针必须是 I32
                let array_chir_raw = self.lower_expr(array)?;
                let array_chir = self.insert_cast_if_needed(array_chir_raw, ValType::I32);
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
                        if matches!(inner.ty, crate::ast::Type::Unit | crate::ast::Type::Nothing) {
                            let sub_ty = match to_wasm {
                                ValType::I64 => crate::ast::Type::Int64,
                                ValType::F32 => crate::ast::Type::Float32,
                                ValType::F64 => crate::ast::Type::Float64,
                                _ => crate::ast::Type::Int32,
                            };
                            return Ok(CHIRExpr::new(CHIRExprKind::Nop, sub_ty, to_wasm));
                        }
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
                // 未知构造函数：返回 Nop（避免 func_idx 回退为 0 即 fd_write）
                let func_idx = match self.func_indices.get(name.as_str()).copied() {
                    Some(idx) => idx,
                    None => {
                        return Ok(CHIRExpr::new(CHIRExprKind::Nop, ty, wasm_ty));
                    }
                };

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
                            if matches!(arg_chir.ty, crate::ast::Type::Unit | crate::ast::Type::Nothing) {
                                let sub_ty = match target_wasm {
                                    ValType::I64 => crate::ast::Type::Int64,
                                    ValType::F32 => crate::ast::Type::Float32,
                                    ValType::F64 => crate::ast::Type::Float64,
                                    _ => crate::ast::Type::Int32,
                                };
                                arg_chir = CHIRExpr::new(CHIRExprKind::Nop, sub_ty, target_wasm);
                            } else {
                                arg_chir = self.insert_cast_if_needed(arg_chir, target_wasm);
                            }
                        } else if matches!(arg_chir.ty, crate::ast::Type::Unit | crate::ast::Type::Nothing) {
                            arg_chir = CHIRExpr::new(CHIRExprKind::Nop, crate::ast::Type::Int32, ValType::I32);
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
                let cond_chir_raw = self.lower_expr(cond)?;
                // WASM if 指令期望 I32 条件，若类型不符则插入截断
                let cond_chir = self.insert_cast_if_needed(cond_chir_raw, ValType::I32);
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

            // try-catch-finally：
            // - 有 catch block 时不执行 try body（避免 throw Nop 后继续执行引发 trap）
            //   仅执行 finally body（如有）
            // - 无 catch block（纯 try-finally）时，顺序执行 try body + finally body
            Expr::TryBlock { body, catch_body, catch_var, finally_body, resources, .. } => {
                let has_catch = catch_var.is_some() || !catch_body.is_empty() || !resources.is_empty();
                let mut stmts: Vec<crate::chir::CHIRStmt> = Vec::new();

                if !has_catch {
                    // 纯 try-finally：执行 try body
                    for stmt in body {
                        if let Ok(s) = self.lower_stmt(stmt) {
                            stmts.push(s);
                        }
                    }
                }
                // finally body（无论如何都执行）
                if let Some(fin_stmts) = finally_body {
                    for stmt in fin_stmts {
                        if let Ok(s) = self.lower_stmt(stmt) {
                            stmts.push(s);
                        }
                    }
                }
                if stmts.is_empty() {
                    // 没有任何语句需要执行：返回 Nop
                    CHIRExprKind::Nop
                } else {
                    return Ok(CHIRExpr::new(
                        CHIRExprKind::Block(crate::chir::CHIRBlock { stmts, result: None }),
                        ty.clone(),
                        wasm_ty,
                    ));
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
        // Unit/Nothing 表达式不产生值，必须先替换为对应类型的零值 Nop，
        // 否则 insert_cast_if_needed 返回原始 Unit 表达式，emit 时产生空栈
        if matches!(expr.ty, crate::ast::Type::Unit | crate::ast::Type::Nothing) {
            let sub_ty = match target_ty {
                ValType::I64 => crate::ast::Type::Int64,
                ValType::F32 => crate::ast::Type::Float32,
                ValType::F64 => crate::ast::Type::Float64,
                _ => crate::ast::Type::Int32,
            };
            return CHIRExpr::new(CHIRExprKind::Nop, sub_ty, target_ty);
        }

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
    use crate::ast::{Program, BinOp, Stmt, Type};

    #[test]
    fn test_lower_integer() {
        let type_ctx = TypeInferenceContext::new();
        let func_indices = HashMap::new();
        let struct_offsets = HashMap::new();
        let class_offsets = HashMap::new();
        let class_field_info = HashMap::new();

        let func_params = HashMap::new();
        let mut ctx = LoweringContext::new(
            &type_ctx,
            &func_indices,
            &func_params,
            &struct_offsets,
            &class_offsets,
            &class_field_info,
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
        let class_field_info = HashMap::new();

        let func_params = HashMap::new();
        let mut ctx = LoweringContext::new(
            &type_ctx,
            &func_indices,
            &func_params,
            &struct_offsets,
            &class_offsets,
            &class_field_info,
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
        let class_field_info = HashMap::new();

        let func_params = HashMap::new();
        let mut ctx = LoweringContext::new(
            &type_ctx,
            &func_indices,
            &func_params,
            &struct_offsets,
            &class_offsets,
            &class_field_info,
        );

        let expr = Expr::Integer(42);
        let chir = ctx.lower_expr(&expr).unwrap();
        let casted = ctx.insert_cast_if_needed(chir, ValType::I32);

        assert!(matches!(casted.kind, CHIRExprKind::Cast { .. }));
        assert_eq!(casted.wasm_ty, ValType::I32);
    }

    fn make_ctx<'a>(
        type_ctx: &'a TypeInferenceContext,
        func_indices: &'a HashMap<String, u32>,
        func_params: &'a HashMap<String, Vec<crate::ast::Param>>,
        struct_offsets: &'a HashMap<String, HashMap<String, u32>>,
        class_offsets: &'a HashMap<String, HashMap<String, u32>>,
        class_field_info: &'a HashMap<String, HashMap<String, (u32, crate::ast::Type)>>,
    ) -> LoweringContext<'a> {
        LoweringContext::new(type_ctx, func_indices, func_params, struct_offsets, class_offsets, class_field_info)
    }

    #[test]
    fn test_lower_float() {
        let type_ctx = TypeInferenceContext::new();
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let chir = ctx.lower_expr(&Expr::Float(3.14)).unwrap();
        assert!(matches!(chir.kind, CHIRExprKind::Float(_)));
        assert_eq!(chir.wasm_ty, ValType::F64);
    }

    #[test]
    fn test_lower_float32() {
        let type_ctx = TypeInferenceContext::new();
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let chir = ctx.lower_expr(&Expr::Float32(1.5)).unwrap();
        assert!(matches!(chir.kind, CHIRExprKind::Float32(_)));
        assert_eq!(chir.wasm_ty, ValType::F32);
    }

    #[test]
    fn test_lower_bool() {
        let type_ctx = TypeInferenceContext::new();
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let chir = ctx.lower_expr(&Expr::Bool(true)).unwrap();
        assert!(matches!(chir.kind, CHIRExprKind::Bool(true)));
        assert_eq!(chir.wasm_ty, ValType::I32);
    }

    #[test]
    fn test_lower_string() {
        let type_ctx = TypeInferenceContext::new();
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let chir = ctx.lower_expr(&Expr::String("hello".into())).unwrap();
        assert!(matches!(chir.kind, CHIRExprKind::String(_)));
        assert_eq!(chir.wasm_ty, ValType::I32);
    }

    #[test]
    fn test_lower_rune() {
        let type_ctx = TypeInferenceContext::new();
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let chir = ctx.lower_expr(&Expr::Rune('A')).unwrap();
        assert!(matches!(chir.kind, CHIRExprKind::Rune('A')));
    }

    #[test]
    fn test_lower_var_local() {
        let mut type_ctx = TypeInferenceContext::new();
        type_ctx.add_local("x".into(), Type::Float64);
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        ctx.alloc_local_typed("x".into(), ValType::F64);
        let chir = ctx.lower_expr(&Expr::Var("x".into())).unwrap();
        assert!(matches!(chir.kind, CHIRExprKind::Local(0)));
        assert_eq!(chir.wasm_ty, ValType::F64);
    }

    #[test]
    fn test_lower_var_global() {
        let type_ctx = TypeInferenceContext::new();
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let chir = ctx.lower_expr(&Expr::Var("unknown_global".into())).unwrap();
        assert!(matches!(chir.kind, CHIRExprKind::Global(_)));
    }

    #[test]
    fn test_lower_unary() {
        let type_ctx = TypeInferenceContext::new();
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let expr = Expr::Unary {
            op: crate::ast::UnaryOp::Neg,
            expr: Box::new(Expr::Integer(42)),
        };
        let chir = ctx.lower_expr(&expr).unwrap();
        assert!(matches!(chir.kind, CHIRExprKind::Unary { .. }));
        assert_eq!(chir.wasm_ty, ValType::I64);
    }

    #[test]
    fn test_lower_call_known() {
        let type_ctx = TypeInferenceContext::new();
        let mut fi = HashMap::new();
        fi.insert("myFunc".into(), 5u32);
        fi.insert("myFunc$1".into(), 5u32);
        let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let expr = Expr::Call {
            name: "myFunc".into(), args: vec![Expr::Integer(1)],
            type_args: None, named_args: vec![],
        };
        let chir = ctx.lower_expr(&expr).unwrap();
        assert!(matches!(chir.kind, CHIRExprKind::Call { func_idx: 5, .. }));
    }

    #[test]
    fn test_lower_call_println() {
        let type_ctx = TypeInferenceContext::new();
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let expr = Expr::Call {
            name: "println".into(), args: vec![Expr::String("hello".into())],
            type_args: None, named_args: vec![],
        };
        let chir = ctx.lower_expr(&expr).unwrap();
        assert!(matches!(chir.kind, CHIRExprKind::Print { .. }));
    }

    #[test]
    fn test_lower_call_exit() {
        let type_ctx = TypeInferenceContext::new();
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let expr = Expr::Call {
            name: "exit".into(), args: vec![Expr::Integer(1)],
            type_args: None, named_args: vec![],
        };
        let chir = ctx.lower_expr(&expr).unwrap();
        assert!(matches!(chir.kind, CHIRExprKind::Unreachable));
    }

    #[test]
    fn test_lower_call_unknown() {
        let type_ctx = TypeInferenceContext::new();
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let expr = Expr::Call {
            name: "unknownFunc".into(), args: vec![],
            type_args: None, named_args: vec![],
        };
        let chir = ctx.lower_expr(&expr).unwrap();
        assert!(matches!(chir.kind, CHIRExprKind::Nop));
    }

    #[test]
    fn test_lower_if_expr() {
        let type_ctx = TypeInferenceContext::new();
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let expr = Expr::If {
            cond: Box::new(Expr::Bool(true)),
            then_branch: Box::new(Expr::Integer(1)),
            else_branch: Some(Box::new(Expr::Integer(2))),
        };
        let chir = ctx.lower_expr(&expr).unwrap();
        assert!(matches!(chir.kind, CHIRExprKind::If { .. }));
    }

    #[test]
    fn test_lower_if_without_else() {
        let type_ctx = TypeInferenceContext::new();
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let expr = Expr::If {
            cond: Box::new(Expr::Bool(true)),
            then_branch: Box::new(Expr::Integer(1)),
            else_branch: None,
        };
        let chir = ctx.lower_expr(&expr).unwrap();
        assert!(matches!(chir.kind, CHIRExprKind::If { .. }));
    }

    #[test]
    fn test_lower_array() {
        let type_ctx = TypeInferenceContext::new();
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let expr = Expr::Array(vec![Expr::Integer(1), Expr::Integer(2)]);
        let chir = ctx.lower_expr(&expr).unwrap();
        assert!(matches!(chir.kind, CHIRExprKind::ArrayNew { .. }));
    }

    #[test]
    fn test_lower_tuple() {
        let type_ctx = TypeInferenceContext::new();
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let expr = Expr::Tuple(vec![Expr::Integer(1), Expr::Bool(true)]);
        let chir = ctx.lower_expr(&expr).unwrap();
        assert!(matches!(chir.kind, CHIRExprKind::TupleNew { .. }));
    }

    #[test]
    fn test_lower_index() {
        let type_ctx = TypeInferenceContext::new();
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let expr = Expr::Index {
            array: Box::new(Expr::Array(vec![Expr::Integer(10)])),
            index: Box::new(Expr::Integer(0)),
        };
        let chir = ctx.lower_expr(&expr).unwrap();
        assert!(matches!(chir.kind, CHIRExprKind::ArrayGet { .. }));
    }

    #[test]
    fn test_lower_field_get() {
        let mut type_ctx = TypeInferenceContext::new();
        type_ctx.add_local("p".into(), Type::Struct("Point".into(), vec![]));
        let mut struct_fields = HashMap::new();
        struct_fields.insert("x".into(), Type::Float64);
        type_ctx.struct_fields.insert("Point".into(), struct_fields);

        let fi = HashMap::new(); let fp = HashMap::new();
        let mut so = HashMap::new();
        let mut point_offsets = HashMap::new();
        point_offsets.insert("x".into(), 8u32);
        so.insert("Point".into(), point_offsets);
        let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);
        ctx.alloc_local_typed("p".into(), ValType::I32);

        let expr = Expr::Field {
            object: Box::new(Expr::Var("p".into())),
            field: "x".into(),
        };
        let chir = ctx.lower_expr(&expr).unwrap();
        assert!(matches!(chir.kind, CHIRExprKind::FieldGet { field_offset: 8, .. }));
        assert_eq!(chir.wasm_ty, ValType::F64);
    }

    #[test]
    fn test_lower_pipeline_nop() {
        let type_ctx = TypeInferenceContext::new();
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let expr = Expr::Binary {
            op: crate::ast::BinOp::Pipeline,
            left: Box::new(Expr::Integer(1)),
            right: Box::new(Expr::Var("f".into())),
        };
        let chir = ctx.lower_expr(&expr).unwrap();
        assert!(matches!(chir.kind, CHIRExprKind::Nop));
    }

    #[test]
    fn test_lower_struct_init() {
        let mut type_ctx = TypeInferenceContext::new();
        let mut sf = HashMap::new();
        sf.insert("x".into(), Type::Float64);
        type_ctx.struct_fields.insert("Point".into(), sf);

        let fi = HashMap::new(); let fp = HashMap::new();
        let mut so = HashMap::new();
        let mut po = HashMap::new();
        po.insert("x".into(), 8u32);
        so.insert("Point".into(), po);
        let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let expr = Expr::StructInit {
            name: "Point".into(),
            fields: vec![("x".into(), Expr::Float(1.0))],
            type_args: None,
        };
        let chir = ctx.lower_expr(&expr).unwrap();
        assert!(matches!(chir.kind, CHIRExprKind::StructNew { .. }));
    }

    #[test]
    fn test_lower_block_expr() {
        let type_ctx = TypeInferenceContext::new();
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let expr = Expr::Block(
            vec![Stmt::Expr(Expr::Integer(1))],
            Some(Box::new(Expr::Integer(42))),
        );
        let chir = ctx.lower_expr(&expr).unwrap();
        // Block lowering 成功即可
        assert!(!matches!(chir.kind, CHIRExprKind::Unreachable));
    }

    #[test]
    fn test_insert_cast_same_type() {
        let type_ctx = TypeInferenceContext::new();
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let expr = Expr::Integer(42);
        let chir = ctx.lower_expr(&expr).unwrap();
        let same = ctx.insert_cast_if_needed(chir, ValType::I64);
        assert!(matches!(same.kind, CHIRExprKind::Integer(42)));
    }

    #[test]
    fn test_alloc_local_typed() {
        let type_ctx = TypeInferenceContext::new();
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let idx = ctx.alloc_local_typed("x".into(), ValType::F64);
        assert_eq!(idx, 0);
        assert_eq!(ctx.get_local_ty(idx), Some(ValType::F64));
        assert_eq!(ctx.get_local("x"), Some(0));
    }

    #[test]
    fn test_lower_method_call() {
        let type_ctx = TypeInferenceContext::new();
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let expr = Expr::MethodCall {
            object: Box::new(Expr::Var("obj".into())),
            method: "doSomething".into(),
            args: vec![], type_args: None, named_args: vec![],
        };
        let chir = ctx.lower_expr(&expr).unwrap();
        // MethodCall without known func_idx falls back
        assert!(!matches!(chir.kind, CHIRExprKind::Unreachable));
    }

    #[test]
    fn test_lower_match_multi_arms() {
        let type_ctx = TypeInferenceContext::new();
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let expr = Expr::Match {
            expr: Box::new(Expr::Integer(1)),
            arms: vec![
                crate::ast::MatchArm {
                    pattern: crate::ast::Pattern::Literal(crate::ast::Literal::Integer(0)),
                    guard: None,
                    body: Box::new(Expr::Integer(10)),
                },
                crate::ast::MatchArm {
                    pattern: crate::ast::Pattern::Literal(crate::ast::Literal::Integer(1)),
                    guard: None,
                    body: Box::new(Expr::Integer(20)),
                },
                crate::ast::MatchArm {
                    pattern: crate::ast::Pattern::Wildcard,
                    guard: None,
                    body: Box::new(Expr::Integer(0)),
                },
            ],
        };
        let chir = ctx.lower_expr(&expr).unwrap();
        assert!(matches!(chir.kind, CHIRExprKind::Match { .. }));
        assert_eq!(chir.wasm_ty, ValType::I64);
    }

    #[test]
    fn test_lower_tuple_index() {
        let mut type_ctx = TypeInferenceContext::new();
        type_ctx.add_local("t".into(), Type::Tuple(vec![Type::Int64, Type::Float64]));
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);
        ctx.alloc_local_typed("t".into(), ValType::I64);

        let expr = Expr::TupleIndex {
            object: Box::new(Expr::Var("t".into())),
            index: 1,
        };
        let chir = ctx.lower_expr(&expr).unwrap();
        assert!(matches!(chir.kind, CHIRExprKind::TupleGet { index: 1, .. }));
    }

    #[test]
    fn test_lower_constructor_call_type_cast() {
        let type_ctx = TypeInferenceContext::new();
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let expr = Expr::ConstructorCall {
            name: "Int64".into(),
            type_args: None,
            args: vec![Expr::Float(3.14)],
            named_args: vec![],
        };
        let chir = ctx.lower_expr(&expr).unwrap();
        assert!(matches!(chir.kind, CHIRExprKind::Cast { .. }));
        assert_eq!(chir.wasm_ty, ValType::I64);
    }

    #[test]
    fn test_lower_constructor_call_same_type() {
        let type_ctx = TypeInferenceContext::new();
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let expr = Expr::ConstructorCall {
            name: "Int64".into(),
            type_args: None,
            args: vec![Expr::Integer(42)],
            named_args: vec![],
        };
        let chir = ctx.lower_expr(&expr).unwrap();
        assert!(matches!(chir.kind, CHIRExprKind::Integer(42)));
    }

    #[test]
    fn test_lower_method_call_known_struct() {
        let mut type_ctx = TypeInferenceContext::new();
        type_ctx.add_local("p".into(), Type::Struct("Point".into(), vec![]));
        let mut sf = HashMap::new();
        sf.insert("x".into(), Type::Float64);
        type_ctx.struct_fields.insert("Point".into(), sf);

        let mut fi = HashMap::new();
        fi.insert("Point.mag$1".into(), 10u32);
        let fp = HashMap::new();
        let mut so = HashMap::new();
        let mut point_offsets = HashMap::new();
        point_offsets.insert("x".into(), 8u32);
        so.insert("Point".into(), point_offsets);
        let co = HashMap::new();
        let mut ci = HashMap::new();
        let mut point_fields = HashMap::new();
        point_fields.insert("x".into(), (8u32, Type::Float64));
        ci.insert("Point".into(), point_fields);

        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);
        ctx.alloc_local_typed("p".into(), ValType::I32);

        let expr = Expr::MethodCall {
            object: Box::new(Expr::Var("p".into())),
            method: "mag".into(),
            args: vec![],
            type_args: None,
            named_args: vec![],
        };
        let chir = ctx.lower_expr(&expr).unwrap();
        assert!(matches!(chir.kind, CHIRExprKind::Call { func_idx: 10, .. }));
    }

    #[test]
    fn test_lower_array_mixed_elems() {
        let type_ctx = TypeInferenceContext::new();
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let expr = Expr::Array(vec![
            Expr::Integer(1),
            Expr::Integer(2),
            Expr::Float(3.0),
        ]);
        let chir = ctx.lower_expr(&expr).unwrap();
        assert!(matches!(chir.kind, CHIRExprKind::ArrayNew { .. }));
    }

    #[test]
    fn test_lower_call_readln() {
        let type_ctx = TypeInferenceContext::new();
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let expr = Expr::Call {
            name: "readln".into(),
            args: vec![],
            type_args: None,
            named_args: vec![],
        };
        let chir = ctx.lower_expr(&expr).unwrap();
        assert!(matches!(chir.kind, CHIRExprKind::Nop));
        assert_eq!(chir.ty, Type::String);
    }

    #[test]
    fn test_lower_call_panic() {
        let type_ctx = TypeInferenceContext::new();
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let expr = Expr::Call {
            name: "panic".into(),
            args: vec![Expr::String("err".into())],
            type_args: None,
            named_args: vec![],
        };
        let chir = ctx.lower_expr(&expr).unwrap();
        assert!(matches!(chir.kind, CHIRExprKind::Unreachable));
    }

    #[test]
    fn test_lower_binary_complex() {
        let type_ctx = TypeInferenceContext::new();
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let expr = Expr::Binary {
            op: BinOp::LogicalAnd,
            left: Box::new(Expr::Bool(true)),
            right: Box::new(Expr::Bool(false)),
        };
        let chir = ctx.lower_expr(&expr).unwrap();
        assert!(matches!(chir.kind, CHIRExprKind::Binary { .. }));
    }

    #[test]
    fn test_lower_tuple_get_index_0() {
        let mut type_ctx = TypeInferenceContext::new();
        type_ctx.add_local("t".into(), Type::Tuple(vec![Type::Int64, Type::Bool]));
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);
        ctx.alloc_local_typed("t".into(), ValType::I64);

        let expr = Expr::TupleIndex {
            object: Box::new(Expr::Var("t".into())),
            index: 0,
        };
        let chir = ctx.lower_expr(&expr).unwrap();
        assert!(matches!(chir.kind, CHIRExprKind::TupleGet { index: 0, .. }));
    }

    #[test]
    fn test_lower_get_field_offset_unknown() {
        let type_ctx = TypeInferenceContext::new();
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let offset = ctx.get_field_offset(&Type::Struct("Unknown".into(), vec![]), "x");
        assert!(offset.is_ok());
        assert_eq!(offset.unwrap(), 0);
    }

    #[test]
    fn test_lower_expr_to_block_simple() {
        let type_ctx = TypeInferenceContext::new();
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let expr = Expr::Integer(99);
        let block = ctx.lower_expr_to_block(&expr).unwrap();
        assert!(block.stmts.is_empty());
        assert!(block.result.is_some());
        if let Some(ref r) = block.result {
            assert!(matches!(r.kind, CHIRExprKind::Integer(99)));
        }
    }

    #[test]
    fn test_lower_call_eprintln() {
        let type_ctx = TypeInferenceContext::new();
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let expr = Expr::Call {
            name: "eprintln".into(),
            args: vec![Expr::String("err".into())],
            type_args: None,
            named_args: vec![],
        };
        let chir = ctx.lower_expr(&expr).unwrap();
        assert!(matches!(chir.kind, CHIRExprKind::Print { fd: 2, .. }));
    }

    #[test]
    fn test_lower_constructor_unknown_returns_nop() {
        let type_ctx = TypeInferenceContext::new();
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let expr = Expr::ConstructorCall {
            name: "UnknownType".into(),
            type_args: None,
            args: vec![Expr::Integer(1)],
            named_args: vec![],
        };
        let chir = ctx.lower_expr(&expr).unwrap();
        assert!(matches!(chir.kind, CHIRExprKind::Nop));
    }

    #[test]
    fn test_lower_range_expr() {
        let type_ctx = TypeInferenceContext::new();
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let expr = Expr::Range {
            start: Box::new(Expr::Integer(0)),
            end: Box::new(Expr::Integer(10)),
            inclusive: false,
            step: None,
        };
        let chir = ctx.lower_expr(&expr).unwrap();
        assert!(matches!(chir.kind, CHIRExprKind::Nop));
    }

    #[test]
    fn test_lower_interpolate_expr() {
        let type_ctx = TypeInferenceContext::new();
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let expr = Expr::Interpolate(vec![
            crate::ast::InterpolatePart::Literal("hi ".into()),
            crate::ast::InterpolatePart::Expr(Box::new(Expr::Integer(42))),
        ]);
        let chir = ctx.lower_expr(&expr).unwrap();
        assert!(matches!(chir.kind, CHIRExprKind::Nop));
    }

    #[test]
    fn test_lower_null_coalesce_expr() {
        let mut type_ctx = TypeInferenceContext::new();
        type_ctx.add_local("opt".into(), Type::Int32);
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);
        ctx.alloc_local_typed("opt".into(), ValType::I32);

        let expr = Expr::NullCoalesce {
            option: Box::new(Expr::Var("opt".into())),
            default: Box::new(Expr::Integer(0)),
        };
        let chir = ctx.lower_expr(&expr).unwrap();
        assert!(matches!(chir.kind, CHIRExprKind::Nop));
    }

    #[test]
    fn test_lower_try_expr() {
        let type_ctx = TypeInferenceContext::new();
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let expr = Expr::Try(Box::new(Expr::Integer(42)));
        let chir = ctx.lower_expr(&expr).unwrap();
        assert!(matches!(chir.kind, CHIRExprKind::Nop));
    }

    #[test]
    fn test_lower_is_type_expr() {
        let type_ctx = TypeInferenceContext::new();
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let expr = Expr::IsType {
            expr: Box::new(Expr::Integer(42)),
            target_ty: Type::Int64,
        };
        let chir = ctx.lower_expr(&expr).unwrap();
        assert!(matches!(chir.kind, CHIRExprKind::Nop));
    }

    #[test]
    fn test_lower_try_block_with_finally() {
        let type_ctx = TypeInferenceContext::new();
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let expr = Expr::TryBlock {
            resources: vec![],
            body: vec![Stmt::Expr(Expr::Integer(1))],
            catch_var: None,
            catch_type: None,
            catch_body: vec![],
            finally_body: Some(vec![Stmt::Expr(Expr::Integer(2))]),
        };
        let chir = ctx.lower_expr(&expr).unwrap();
        assert!(matches!(chir.kind, CHIRExprKind::Block(..)));
    }

    #[test]
    fn test_lower_cast_expr_fallback() {
        let type_ctx = TypeInferenceContext::new();
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let expr = Expr::Cast {
            expr: Box::new(Expr::Integer(1000)),
            target_ty: Type::Int32,
        };
        let chir = ctx.lower_expr(&expr).unwrap();
        assert!(matches!(chir.kind, CHIRExprKind::Nop));
    }

    #[test]
    fn test_lower_binary_chain_mul_add() {
        let type_ctx = TypeInferenceContext::new();
        let fi = HashMap::new(); let fp = HashMap::new();
        let so = HashMap::new(); let co = HashMap::new(); let ci = HashMap::new();
        let mut ctx = make_ctx(&type_ctx, &fi, &fp, &so, &co, &ci);

        let expr = Expr::Binary {
            op: BinOp::Add,
            left: Box::new(Expr::Binary {
                op: BinOp::Mul,
                left: Box::new(Expr::Integer(2)),
                right: Box::new(Expr::Integer(3)),
            }),
            right: Box::new(Expr::Integer(4)),
        };
        let chir = ctx.lower_expr(&expr).unwrap();
        assert!(matches!(chir.kind, CHIRExprKind::Binary { .. }));
        if let CHIRExprKind::Binary { left, right, .. } = &chir.kind {
            assert!(matches!(left.kind, CHIRExprKind::Binary { .. }));
            assert!(matches!(right.kind, CHIRExprKind::Integer(4)));
        }
    }
}
