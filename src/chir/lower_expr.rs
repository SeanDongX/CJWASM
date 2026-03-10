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

    /// 类继承关系：class_name → parent_class_name
    pub class_extends: HashMap<String, String>,

    /// 函数返回类型：func_name → return Type
    pub func_return_types: HashMap<String, crate::ast::Type>,

    /// 枚举定义（用于 variant 模式匹配的判别值查找）
    pub enum_defs: Vec<crate::ast::EnumDef>,

    /// 下一个可用的局部变量索引
    next_local: u32,

    /// 当前函数返回值的 WASM 类型（用于 Return 语句的类型强制转换）
    pub return_wasm_ty: Option<ValType>,

    /// 局部变量的 AST 类型（用于内置方法推断）
    pub local_ast_types: HashMap<String, crate::ast::Type>,

    /// try-catch 错误标志局部变量索引栈（嵌套 try 时多层）
    pub err_flag_stack: Vec<u32>,
    /// try-catch 错误值局部变量索引栈
    pub err_val_stack: Vec<u32>,

    /// Lambda 计数器（用于生成 __lambda_N 名称，与 lower.rs 中收集顺序一致）
    pub lambda_counter: u32,
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
            class_extends: HashMap::new(),
            func_return_types: HashMap::new(),
            enum_defs: Vec::new(),
            next_local: 0,
            return_wasm_ty: None,
            local_ast_types: HashMap::new(),
            err_flag_stack: Vec::new(),
            err_val_stack: Vec::new(),
            lambda_counter: 0,
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
            t => {
                let inferred_wasm = t.to_wasm();
                // 对于 Var 表达式，如果 type_ctx 推断为 Int32 但 lowering 阶段分配了
                // 不同类型（如 For 循环变量），以 lowering 阶段的类型为准
                if let Expr::Var(name) = expr {
                    if let Some(local_ty) = self.local_map.get(name)
                        .and_then(|&idx| self.get_local_ty(idx))
                    {
                        if local_ty != inferred_wasm && matches!(ty, crate::ast::Type::Int32) {
                            local_ty
                        } else {
                            inferred_wasm
                        }
                    } else {
                        inferred_wasm
                    }
                } else {
                    inferred_wasm
                }
            }
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
                // 数学常量
                let math_const = match name.as_str() {
                    "PI" => Some(std::f64::consts::PI),
                    "E" => Some(std::f64::consts::E),
                    "TAU" => Some(std::f64::consts::TAU),
                    "INF" | "INFINITY" => Some(f64::INFINITY),
                    "NEG_INF" | "NEG_INFINITY" => Some(f64::NEG_INFINITY),
                    "NAN" => Some(f64::NAN),
                    _ => None,
                };
                if let Some(val) = math_const {
                    return Ok(CHIRExpr::new(
                        CHIRExprKind::Float(val),
                        crate::ast::Type::Float64, ValType::F64,
                    ));
                }
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
            Expr::Binary { op: crate::ast::BinOp::Pipeline, .. } => {
                CHIRExprKind::Nop
            }

            // !in 运算符：a !in b → !(b.contains(a))
            Expr::Binary { op: crate::ast::BinOp::NotIn, left, right } => {
                let contains_call = Expr::MethodCall {
                    object: right.clone(),
                    method: "contains".to_string(),
                    args: vec![left.as_ref().clone()],
                    named_args: vec![],
                    type_args: None,
                };
                let contains_chir = self.lower_expr(&contains_call)?;
                return Ok(CHIRExpr::new(
                    CHIRExprKind::Unary {
                        op: crate::ast::UnaryOp::Not,
                        expr: Box::new(contains_chir),
                    },
                    crate::ast::Type::Bool, wasm_encoder::ValType::I32,
                ));
            }

            // 幂运算：转换为函数调用
            Expr::Binary { op: crate::ast::BinOp::Pow, left, right } => {
                let left_chir = self.lower_expr(left)?;
                let right_chir = self.lower_expr(right)?;

                // 根据操作数类型选择 __pow_i64 或 __pow_f64
                let func_name = if left_chir.wasm_ty == ValType::F64 || right_chir.wasm_ty == ValType::F64 {
                    "__pow_f64"
                } else {
                    "__pow_i64"
                };

                // 解析函数索引
                let mangled = format!("{}$2", func_name);
                let func_idx = self.func_indices.get(&mangled)
                    .or_else(|| self.func_indices.get(func_name))
                    .copied()
                    .unwrap_or(0);

                // 确保两个操作数类型一致
                let target_ty = if func_name == "__pow_f64" { ValType::F64 } else { ValType::I64 };
                let left_casted = self.insert_cast_if_needed(left_chir, target_ty);
                let right_casted = self.insert_cast_if_needed(right_chir, target_ty);

                return Ok(CHIRExpr::new(
                    CHIRExprKind::Call {
                        func_idx,
                        args: vec![left_casted, right_casted],
                    },
                    if target_ty == ValType::F64 { crate::ast::Type::Float64 } else { crate::ast::Type::Int64 },
                    target_ty,
                ));
            }

            // 二元运算
            Expr::Binary { op, left, right } => {
                let left_chir = self.lower_expr(left)?;
                let right_chir = self.lower_expr(right)?;

                let is_comparison = matches!(op,
                    crate::ast::BinOp::Eq | crate::ast::BinOp::NotEq
                    | crate::ast::BinOp::Lt | crate::ast::BinOp::LtEq
                    | crate::ast::BinOp::Gt | crate::ast::BinOp::GtEq
                );

                // For arithmetic ops, if actual operand types are wider than
                // what type inference says, promote the result to match
                let effective_wasm_ty = if !is_comparison {
                    if left_chir.wasm_ty == ValType::F64 || right_chir.wasm_ty == ValType::F64 {
                        ValType::F64
                    } else if left_chir.wasm_ty == ValType::I64 || right_chir.wasm_ty == ValType::I64 {
                        ValType::I64
                    } else {
                        wasm_ty
                    }
                } else {
                    wasm_ty
                };

                let (left_chir, right_chir) = if is_comparison {
                    let operand_ty = if left_chir.wasm_ty == ValType::I64 || right_chir.wasm_ty == ValType::I64 {
                        ValType::I64
                    } else if left_chir.wasm_ty == ValType::F64 || right_chir.wasm_ty == ValType::F64 {
                        ValType::F64
                    } else {
                        left_chir.wasm_ty
                    };
                    (self.insert_cast_if_needed(left_chir, operand_ty),
                     self.insert_cast_if_needed(right_chir, operand_ty))
                } else {
                    (self.insert_cast_if_needed(left_chir, effective_wasm_ty),
                     self.insert_cast_if_needed(right_chir, effective_wasm_ty))
                };

                // Update result types if promoted
                if effective_wasm_ty != wasm_ty {
                    let promoted_ty = match effective_wasm_ty {
                        ValType::I64 => crate::ast::Type::Int64,
                        ValType::F64 => crate::ast::Type::Float64,
                        _ => ty.clone(),
                    };
                    return Ok(CHIRExpr::new(
                        CHIRExprKind::Binary {
                            op: op.clone(),
                            left: Box::new(left_chir),
                            right: Box::new(right_chir),
                        },
                        promoted_ty, effective_wasm_ty,
                    ));
                }

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
                        return Ok(CHIRExpr::new(
                            CHIRExprKind::Nop,
                            crate::ast::Type::String,
                            ValType::I32,
                        ));
                    }
                    "min" | "max" if args.len() == 2 => {
                        let a = self.lower_expr(&args[0])?;
                        let b = self.lower_expr(&args[1])?;
                        let cmp_op = if name == "min" { crate::ast::BinOp::Lt } else { crate::ast::BinOp::Gt };
                        let cond = CHIRExpr::new(
                            CHIRExprKind::Binary { op: cmp_op, left: Box::new(a.clone()), right: Box::new(b.clone()) },
                            crate::ast::Type::Bool, ValType::I32,
                        );
                        let then_block = crate::chir::CHIRBlock { stmts: vec![], result: Some(Box::new(a.clone())) };
                        let else_block = crate::chir::CHIRBlock { stmts: vec![], result: Some(Box::new(b.clone())) };
                        return Ok(CHIRExpr::new(
                            CHIRExprKind::If { cond: Box::new(cond), then_block, else_block: Some(else_block) },
                            a.ty.clone(), a.wasm_ty,
                        ));
                    }
                    // WASM 原生数学内置函数（f64 → f64），仅当无用户自定义函数时
                    "sqrt" | "floor" | "ceil" | "trunc" | "nearest" | "abs"
                    | "sin" | "cos" | "exp" | "log" if args.len() == 1
                        && !self.func_indices.contains_key(name.as_str())
                        && !self.func_indices.contains_key(&format!("{}$1", name)) => {
                        let a = self.lower_expr(&args[0])?;
                        let a = self.insert_cast_if_needed(a, ValType::F64);
                        return Ok(CHIRExpr::new(
                            CHIRExprKind::MathUnary { op: name.clone(), arg: Box::new(a) },
                            crate::ast::Type::Float64, ValType::F64,
                        ));
                    }
                    "pow" if args.len() == 2
                        && !self.func_indices.contains_key("pow")
                        && !self.func_indices.contains_key("pow$2") => {
                        let a = self.lower_expr(&args[0])?;
                        let a = self.insert_cast_if_needed(a, ValType::F64);
                        let b = self.lower_expr(&args[1])?;
                        let b = self.insert_cast_if_needed(b, ValType::F64);
                        return Ok(CHIRExpr::new(
                            CHIRExprKind::MathBinary { op: "pow".into(), left: Box::new(a), right: Box::new(b) },
                            crate::ast::Type::Float64, ValType::F64,
                        ));
                    }
                    // sort(arr): in-place bubble sort on i64 array [size:i32][data:i64*]
                    "sort" if args.len() == 1 => {
                        let arr = self.lower_expr(&args[0])?;
                        let arr = self.insert_cast_if_needed(arr, ValType::I32);
                        let arr_local = self.alloc_local_typed(format!("__sort_arr_{}", self.next_local), ValType::I32);
                        let n_local = self.alloc_local_typed(format!("__sort_n_{}", self.next_local), ValType::I64);
                        let i_local = self.alloc_local_typed(format!("__sort_i_{}", self.next_local), ValType::I64);
                        let j_local = self.alloc_local_typed(format!("__sort_j_{}", self.next_local), ValType::I64);
                        let tmp_local = self.alloc_local_typed(format!("__sort_tmp_{}", self.next_local), ValType::I64);

                        let mut stmts = Vec::new();
                        stmts.push(crate::chir::CHIRStmt::Let { local_idx: arr_local, value: arr });
                        // n = load i32 at arr (size)
                        stmts.push(crate::chir::CHIRStmt::Let {
                            local_idx: n_local,
                            value: CHIRExpr::new(CHIRExprKind::Cast {
                                expr: Box::new(CHIRExpr::new(CHIRExprKind::FieldGet {
                                    object: Box::new(CHIRExpr::new(CHIRExprKind::Local(arr_local), crate::ast::Type::Int32, ValType::I32)),
                                    field_offset: 0, field_ty: crate::ast::Type::Int32,
                                }, crate::ast::Type::Int32, ValType::I32)),
                                from_ty: ValType::I32, to_ty: ValType::I64,
                            }, crate::ast::Type::Int64, ValType::I64),
                        });
                        // i = 0
                        stmts.push(crate::chir::CHIRStmt::Let {
                            local_idx: i_local,
                            value: CHIRExpr::new(CHIRExprKind::Integer(0), crate::ast::Type::Int64, ValType::I64),
                        });

                        // helper: data_ptr + idx*8 + 4 → memory address of arr[idx]
                        let elem_addr = |idx_local: u32| -> CHIRExpr {
                            CHIRExpr::new(CHIRExprKind::Binary {
                                op: crate::ast::BinOp::Add,
                                left: Box::new(CHIRExpr::new(CHIRExprKind::Binary {
                                    op: crate::ast::BinOp::Add,
                                    left: Box::new(CHIRExpr::new(CHIRExprKind::Local(arr_local), crate::ast::Type::Int32, ValType::I32)),
                                    right: Box::new(CHIRExpr::new(CHIRExprKind::Integer(4), crate::ast::Type::Int32, ValType::I32)),
                                }, crate::ast::Type::Int32, ValType::I32)),
                                right: Box::new(CHIRExpr::new(CHIRExprKind::Binary {
                                    op: crate::ast::BinOp::Mul,
                                    left: Box::new(CHIRExpr::new(CHIRExprKind::Cast {
                                        expr: Box::new(CHIRExpr::new(CHIRExprKind::Local(idx_local), crate::ast::Type::Int64, ValType::I64)),
                                        from_ty: ValType::I64, to_ty: ValType::I32,
                                    }, crate::ast::Type::Int32, ValType::I32)),
                                    right: Box::new(CHIRExpr::new(CHIRExprKind::Integer(8), crate::ast::Type::Int32, ValType::I32)),
                                }, crate::ast::Type::Int32, ValType::I32)),
                            }, crate::ast::Type::Int32, ValType::I32)
                        };
                        let load_elem = |idx_local: u32| -> CHIRExpr {
                            CHIRExpr::new(CHIRExprKind::Load {
                                ptr: Box::new(elem_addr(idx_local)),
                                offset: 0, align: 3,
                            }, crate::ast::Type::Int64, ValType::I64)
                        };

                        // inner loop: j = 0; while j < n - i - 1
                        let j_init = crate::chir::CHIRStmt::Let {
                            local_idx: j_local,
                            value: CHIRExpr::new(CHIRExprKind::Integer(0), crate::ast::Type::Int64, ValType::I64),
                        };
                        let j_limit = CHIRExpr::new(CHIRExprKind::Binary {
                            op: crate::ast::BinOp::Sub,
                            left: Box::new(CHIRExpr::new(CHIRExprKind::Binary {
                                op: crate::ast::BinOp::Sub,
                                left: Box::new(CHIRExpr::new(CHIRExprKind::Local(n_local), crate::ast::Type::Int64, ValType::I64)),
                                right: Box::new(CHIRExpr::new(CHIRExprKind::Local(i_local), crate::ast::Type::Int64, ValType::I64)),
                            }, crate::ast::Type::Int64, ValType::I64)),
                            right: Box::new(CHIRExpr::new(CHIRExprKind::Integer(1), crate::ast::Type::Int64, ValType::I64)),
                        }, crate::ast::Type::Int64, ValType::I64);
                        let j_cond = CHIRExpr::new(CHIRExprKind::Binary {
                            op: crate::ast::BinOp::Lt,
                            left: Box::new(CHIRExpr::new(CHIRExprKind::Local(j_local), crate::ast::Type::Int64, ValType::I64)),
                            right: Box::new(j_limit),
                        }, crate::ast::Type::Bool, ValType::I32);

                        // j+1 local (we reuse j_local + 1 inline)
                        let j_plus1_local = self.alloc_local_typed(format!("__sort_j1_{}", self.next_local), ValType::I64);

                        // inner body: if arr[j] > arr[j+1] { swap }; j = j + 1
                        let swap_cond = CHIRExpr::new(CHIRExprKind::Binary {
                            op: crate::ast::BinOp::Gt,
                            left: Box::new(load_elem(j_local)),
                            right: Box::new(load_elem(j_plus1_local)),
                        }, crate::ast::Type::Bool, ValType::I32);

                        let swap_body = crate::chir::CHIRBlock {
                            stmts: vec![
                                // tmp = arr[j]
                                crate::chir::CHIRStmt::Let { local_idx: tmp_local, value: load_elem(j_local) },
                                // arr[j] = arr[j+1]
                                crate::chir::CHIRStmt::Expr(CHIRExpr::new(CHIRExprKind::Store {
                                    ptr: Box::new(elem_addr(j_local)),
                                    value: Box::new(load_elem(j_plus1_local)),
                                    offset: 0, align: 3,
                                }, crate::ast::Type::Unit, ValType::I32)),
                                // arr[j+1] = tmp
                                crate::chir::CHIRStmt::Expr(CHIRExpr::new(CHIRExprKind::Store {
                                    ptr: Box::new(elem_addr(j_plus1_local)),
                                    value: Box::new(CHIRExpr::new(CHIRExprKind::Local(tmp_local), crate::ast::Type::Int64, ValType::I64)),
                                    offset: 0, align: 3,
                                }, crate::ast::Type::Unit, ValType::I32)),
                            ],
                            result: None,
                        };

                        let inner_body = crate::chir::CHIRBlock {
                            stmts: vec![
                                // j1 = j + 1
                                crate::chir::CHIRStmt::Let {
                                    local_idx: j_plus1_local,
                                    value: CHIRExpr::new(CHIRExprKind::Binary {
                                        op: crate::ast::BinOp::Add,
                                        left: Box::new(CHIRExpr::new(CHIRExprKind::Local(j_local), crate::ast::Type::Int64, ValType::I64)),
                                        right: Box::new(CHIRExpr::new(CHIRExprKind::Integer(1), crate::ast::Type::Int64, ValType::I64)),
                                    }, crate::ast::Type::Int64, ValType::I64),
                                },
                                // if arr[j] > arr[j+1] { swap }
                                crate::chir::CHIRStmt::Expr(CHIRExpr::new(
                                    CHIRExprKind::If { cond: Box::new(swap_cond), then_block: swap_body, else_block: None },
                                    crate::ast::Type::Unit, ValType::I32,
                                )),
                                // j = j + 1
                                crate::chir::CHIRStmt::Assign {
                                    target: crate::chir::CHIRLValue::Local(j_local),
                                    value: CHIRExpr::new(CHIRExprKind::Local(j_plus1_local), crate::ast::Type::Int64, ValType::I64),
                                },
                            ],
                            result: None,
                        };

                        // outer loop body: j_init, while j < ..., i = i + 1
                        let outer_body = crate::chir::CHIRBlock {
                            stmts: vec![
                                j_init,
                                crate::chir::CHIRStmt::While { cond: j_cond, body: inner_body },
                                crate::chir::CHIRStmt::Assign {
                                    target: crate::chir::CHIRLValue::Local(i_local),
                                    value: CHIRExpr::new(CHIRExprKind::Binary {
                                        op: crate::ast::BinOp::Add,
                                        left: Box::new(CHIRExpr::new(CHIRExprKind::Local(i_local), crate::ast::Type::Int64, ValType::I64)),
                                        right: Box::new(CHIRExpr::new(CHIRExprKind::Integer(1), crate::ast::Type::Int64, ValType::I64)),
                                    }, crate::ast::Type::Int64, ValType::I64),
                                },
                            ],
                            result: None,
                        };

                        let outer_cond = CHIRExpr::new(CHIRExprKind::Binary {
                            op: crate::ast::BinOp::Lt,
                            left: Box::new(CHIRExpr::new(CHIRExprKind::Local(i_local), crate::ast::Type::Int64, ValType::I64)),
                            right: Box::new(CHIRExpr::new(CHIRExprKind::Local(n_local), crate::ast::Type::Int64, ValType::I64)),
                        }, crate::ast::Type::Bool, ValType::I32);

                        stmts.push(crate::chir::CHIRStmt::While { cond: outer_cond, body: outer_body });

                        return Ok(CHIRExpr::new(
                            CHIRExprKind::Block(crate::chir::CHIRBlock { stmts, result: None }),
                            crate::ast::Type::Unit, ValType::I32,
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
                        // 检查是否为函数类型的局部变量调用（Lambda / call_indirect）
                        if let Some(crate::ast::Type::Function { params: fn_params, ret }) = self.local_ast_types.get(name).cloned() {
                            let mut lowered_args = Vec::new();
                            for arg in args {
                                lowered_args.push(self.lower_expr(arg)?);
                            }
                            let callee_local = self.local_map.get(name).copied().unwrap_or(0);
                            let callee_expr = CHIRExpr::new(CHIRExprKind::Local(callee_local), crate::ast::Type::Int32, ValType::I32);
                            let ret_wasm = ret.as_ref().as_ref().map(|t| t.to_wasm()).unwrap_or(ValType::I64);
                            let ret_ty = ret.as_ref().as_ref().cloned().unwrap_or(crate::ast::Type::Int64);
                            return Ok(CHIRExpr::new(
                                CHIRExprKind::CallIndirect {
                                    type_idx: 0, // placeholder, resolved by chir_codegen
                                    args: lowered_args,
                                    callee: Box::new(callee_expr),
                                },
                                ret_ty,
                                ret_wasm,
                            ));
                        }
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
                // ── 静态方法调用：ClassName.method(args) ──
                if let Expr::Var(cls_name) = object.as_ref() {
                    let is_class = self.class_field_info.contains_key(cls_name.as_str())
                        || self.struct_field_offsets.contains_key(cls_name.as_str());
                    if is_class {
                        let mangled = format!("{}.{}", cls_name, method);
                        let arity_mangled = format!("{}${}", mangled, args.len() + named_args.len());
                        let func_idx = self.func_indices.get(&arity_mangled)
                            .or_else(|| self.func_indices.get(&mangled))
                            .copied();
                        if let Some(func_idx) = func_idx {
                            let mut call_args = Vec::new();
                            for arg in args.iter() {
                                call_args.push(self.lower_expr(arg)?);
                            }
                            for (_, val) in named_args.iter() {
                                call_args.push(self.lower_expr(val)?);
                            }
                            let ret_ty = self.func_return_types.get(&mangled)
                                .cloned()
                                .unwrap_or(ty.clone());
                            let ret_wasm = match &ret_ty {
                                crate::ast::Type::Unit | crate::ast::Type::Nothing => wasm_encoder::ValType::I32,
                                t => t.to_wasm(),
                            };
                            return Ok(CHIRExpr::new(
                                CHIRExprKind::Call { func_idx, args: call_args },
                                ret_ty, ret_wasm,
                            ));
                        }
                    }
                }

                // 推断 receiver 类型：优先使用局部 AST 类型映射，再用 type_ctx
                let obj_ty = if let Expr::Var(name) = object.as_ref() {
                    self.local_ast_types.get(name).cloned()
                        .unwrap_or_else(|| self.type_ctx.infer_expr(object).unwrap_or(crate::ast::Type::Int32))
                } else {
                    self.type_ctx.infer_expr(object)?
                };

                // ── 内置类型方法处理 ──
                if let Some(result) = self.try_lower_builtin_method(object, &obj_ty, method, args)? {
                    return Ok(result);
                }

                let class_name = match &obj_ty {
                    crate::ast::Type::Struct(name, _) => Some(name.clone()),
                    crate::ast::Type::Qualified(parts) => parts.last().cloned(),
                    _ => None,
                };

                if let Some(cls) = class_name {
                    let arity = 1 + args.len() + named_args.len();
                    // 查找方法：先在当前类，再沿继承链向上查找
                    let mut search_cls = cls.clone();
                    let mut func_idx = None;
                    let mut resolved_method = String::new();
                    loop {
                        let mangled_method = format!("{}.{}", search_cls, method);
                        let mangled_with_arity = format!("{}${}", mangled_method, arity);
                        func_idx = self.func_indices.get(&mangled_with_arity)
                            .or_else(|| self.func_indices.get(&mangled_method))
                            .copied();
                        if func_idx.is_some() {
                            resolved_method = mangled_method;
                            break;
                        }
                        // 向父类查找
                        if let Some(parent) = self.class_extends.get(&search_cls) {
                            search_cls = parent.clone();
                        } else {
                            resolved_method = format!("{}.{}", cls, method);
                            break;
                        }
                    }
                    let mangled_method = resolved_method;

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

                        let (ret_ty, ret_wasm) = self.func_return_types.get(&mangled_method)
                            .map(|rt| {
                                let wt = match rt {
                                    crate::ast::Type::Unit | crate::ast::Type::Nothing => ValType::I32,
                                    t => t.to_wasm(),
                                };
                                (rt.clone(), wt)
                            })
                            .unwrap_or_else(|| (ty.clone(), wasm_ty));
                        return Ok(CHIRExpr::new(
                            CHIRExprKind::Call { func_idx, args: call_args },
                            ret_ty,
                            ret_wasm,
                        ));
                    }
                }

                // 未能解析：返回 Nop（方法不在已知类中，或 vtable 调用等待后续实现）
                CHIRExprKind::Nop
            }

            // super() 调用：在 init 函数中调用父类 init → 直接设置父类字段
            Expr::SuperCall { method, args, .. } => {
                if (method == "init" || method.is_empty()) && self.current_class.is_some() {
                    let (class_name, this_idx) = self.current_class.clone().unwrap();
                    if let Some(parent) = self.class_extends.get(&class_name).cloned() {
                        if let Some(fields) = self.class_field_info.get(&parent) {
                            let mut sorted_fields: Vec<_> = fields.iter().collect();
                            sorted_fields.sort_by_key(|(_, (off, _))| *off);
                            let this_expr = CHIRExpr::new(
                                CHIRExprKind::Local(this_idx),
                                crate::ast::Type::Struct(class_name.clone(), vec![]),
                                ValType::I32,
                            );
                            let mut stmts = Vec::new();
                            for (i, arg) in args.iter().enumerate() {
                                if let Some((_, (offset, _))) = sorted_fields.get(i) {
                                    let arg_chir = self.lower_expr(arg)?;
                                    stmts.push(crate::chir::CHIRStmt::Assign {
                                        target: crate::chir::CHIRLValue::Field {
                                            object: Box::new(this_expr.clone()),
                                            offset: *offset,
                                        },
                                        value: arg_chir,
                                    });
                                }
                            }
                            let block = crate::chir::CHIRBlock { stmts, result: None };
                            return Ok(CHIRExpr::new(
                                CHIRExprKind::Block(block),
                                crate::ast::Type::Unit, ValType::I32,
                            ));
                        }
                    }
                }
                CHIRExprKind::Nop
            }

            // 字段访问
            Expr::Field { object, field } => {
                let obj_ty = if let Expr::Var(name) = object.as_ref() {
                    self.local_ast_types.get(name).cloned()
                        .unwrap_or_else(|| self.type_ctx.infer_expr(object).unwrap_or(crate::ast::Type::Int32))
                } else {
                    self.type_ctx.infer_expr(object)?
                };

                // String.size → load i32 length at offset 0, then extend to i64
                if matches!(obj_ty, crate::ast::Type::String) && field == "size" {
                    let obj_chir = self.lower_expr(object)?;
                    let obj_chir = self.insert_cast_if_needed(obj_chir, ValType::I32);
                    let i32_load = CHIRExpr::new(
                        CHIRExprKind::FieldGet {
                            object: Box::new(obj_chir),
                            field_offset: 0,
                            field_ty: crate::ast::Type::Int32,
                        },
                        crate::ast::Type::Int32, ValType::I32,
                    );
                    return Ok(self.insert_cast_if_needed(i32_load, ValType::I64));
                }

                // 先检查是否是 property getter（ClassName.__get_propName）
                let class_name = match &obj_ty {
                    crate::ast::Type::Struct(name, _) => Some(name.clone()),
                    _ => None,
                };
                if let Some(ref cls) = class_name {
                    let getter_name = format!("{}.__get_{}", cls, field);
                    if let Some(&func_idx) = self.func_indices.get(&getter_name) {
                        let obj_chir = self.lower_expr(object)?;
                        let obj_chir = self.insert_cast_if_needed(obj_chir, ValType::I32);
                        let ret_ty = self.func_return_types.get(&getter_name)
                            .cloned()
                            .unwrap_or(crate::ast::Type::Int64);
                        let ret_wasm = ret_ty.to_wasm();
                        return Ok(CHIRExpr::new(
                            CHIRExprKind::Call { func_idx, args: vec![obj_chir] },
                            ret_ty, ret_wasm,
                        ));
                    }
                }

                let obj_chir = self.lower_expr(object)?;
                let offset = self.get_field_offset(&obj_ty, field)?;
                let field_ty = self.type_ctx.infer_field_type(&obj_ty, field)?;
                let obj_chir = self.insert_cast_if_needed(obj_chir, ValType::I32);

                let field_wasm = match &field_ty {
                    crate::ast::Type::Unit | crate::ast::Type::Nothing => ValType::I32,
                    t => t.to_wasm(),
                };
                return Ok(CHIRExpr::new(
                    CHIRExprKind::FieldGet {
                        object: Box::new(obj_chir),
                        field_offset: offset,
                        field_ty: field_ty.clone(),
                    },
                    field_ty,
                    field_wasm,
                ));
            }

            // 数组
            Expr::Array(elems) => {
                let elements: Vec<CHIRExpr> = elems.iter()
                    .map(|e| self.lower_expr(e))
                    .collect::<Result<Vec<_>, _>>()?;
                CHIRExprKind::ArrayLiteral { elements }
            }

            // 数组索引
            Expr::Index { array, index } => {
                // Check if this is tuple indexing (pair[0]) vs array indexing
                let obj_ast_ty = if let Expr::Var(name) = array.as_ref() {
                    self.local_ast_types.get(name).cloned()
                        .or_else(|| self.type_ctx.locals.get(name.as_str()).cloned())
                } else {
                    None
                };

                if let Some(crate::ast::Type::Tuple(elem_types)) = &obj_ast_ty {
                    let tuple_chir = self.lower_expr(array)?;
                    let idx = match index.as_ref() {
                        Expr::Integer(n) => *n as usize,
                        _ => 0,
                    };
                    let elem_ty = elem_types.get(idx).cloned().unwrap_or(crate::ast::Type::Int64);
                    let elem_wasm = match &elem_ty {
                        crate::ast::Type::Unit | crate::ast::Type::Nothing => wasm_encoder::ValType::I32,
                        t => t.to_wasm(),
                    };
                    return Ok(CHIRExpr::new(
                        CHIRExprKind::TupleGet {
                            tuple: Box::new(tuple_chir),
                            index: idx,
                        },
                        elem_ty, elem_wasm,
                    ));
                }

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

                // Determine element type from the tuple's AST type
                let elem_ty = if let Expr::Var(name) = object.as_ref() {
                    self.local_ast_types.get(name)
                        .and_then(|t| if let crate::ast::Type::Tuple(types) = t {
                            types.get(*index as usize).cloned()
                        } else {
                            None
                        })
                        .unwrap_or(crate::ast::Type::Int64)
                } else {
                    crate::ast::Type::Int64
                };
                let elem_wasm = match &elem_ty {
                    crate::ast::Type::Unit | crate::ast::Type::Nothing => wasm_encoder::ValType::I32,
                    t => t.to_wasm(),
                };

                return Ok(CHIRExpr::new(
                    CHIRExprKind::TupleGet {
                        tuple: Box::new(tuple_chir),
                        index: *index as usize,
                    },
                    elem_ty, elem_wasm,
                ));
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
                // ArrayList<T>() → __arraylist_new
                if name == "ArrayList" && args.is_empty() {
                    if let Some(&idx) = self.func_indices.get("__arraylist_new") {
                        return Ok(CHIRExpr::new(
                            CHIRExprKind::Call { func_idx: idx, args: vec![] },
                            crate::ast::Type::Array(Box::new(crate::ast::Type::Int64)),
                            ValType::I32,
                        ));
                    }
                }
                // ArrayStack<T>() / LinkedList<T>() → backed by __arraylist_new
                if (name == "ArrayStack" || name == "LinkedList") && args.is_empty() {
                    if let Some(&idx) = self.func_indices.get("__arraylist_new") {
                        return Ok(CHIRExpr::new(
                            CHIRExprKind::Call { func_idx: idx, args: vec![] },
                            crate::ast::Type::Struct(name.to_string(), vec![]),
                            ValType::I32,
                        ));
                    }
                }
                // HashMap<K,V>() → __hashmap_new
                if name == "HashMap" && args.is_empty() {
                    if let Some(&idx) = self.func_indices.get("__hashmap_new") {
                        return Ok(CHIRExpr::new(
                            CHIRExprKind::Call { func_idx: idx, args: vec![] },
                            crate::ast::Type::Map(Box::new(crate::ast::Type::Int64), Box::new(crate::ast::Type::Int64)),
                            ValType::I32,
                        ));
                    }
                }
                // HashSet<T>() → __hashset_new
                if name == "HashSet" && args.is_empty() {
                    if let Some(&idx) = self.func_indices.get("__hashset_new") {
                        return Ok(CHIRExpr::new(
                            CHIRExprKind::Call { func_idx: idx, args: vec![] },
                            crate::ast::Type::Map(Box::new(crate::ast::Type::Int64), Box::new(crate::ast::Type::Int64)),
                            ValType::I32,
                        ));
                    }
                }
                // AtomicInt64(val) → alloc 8 bytes, store val
                if name == "AtomicInt64" {
                    let alloc_idx = *self.func_indices.get("__alloc").unwrap_or(&7);
                    let init_val = if args.is_empty() { 0i64 } else {
                        match &args[0] { Expr::Integer(n) => *n, _ => 0 }
                    };
                    let ptr_local = self.alloc_local_typed(format!("__tmp_{}", self.next_local), ValType::I32);
                    let mut stmts = Vec::new();
                    stmts.push(crate::chir::CHIRStmt::Let {
                        local_idx: ptr_local,
                        value: CHIRExpr::new(
                            CHIRExprKind::Call { func_idx: alloc_idx, args: vec![CHIRExpr::new(CHIRExprKind::Integer(8), crate::ast::Type::Int32, ValType::I32)] },
                            crate::ast::Type::Int32, ValType::I32,
                        ),
                    });
                    stmts.push(crate::chir::CHIRStmt::Expr(CHIRExpr::new(
                        CHIRExprKind::Store {
                            ptr: Box::new(CHIRExpr::new(CHIRExprKind::Local(ptr_local), crate::ast::Type::Int32, ValType::I32)),
                            value: Box::new(CHIRExpr::new(CHIRExprKind::Integer(init_val), crate::ast::Type::Int64, ValType::I64)),
                            offset: 0, align: 3,
                        },
                        crate::ast::Type::Unit, ValType::I32,
                    )));
                    return Ok(CHIRExpr::new(
                        CHIRExprKind::Block(crate::chir::CHIRBlock {
                            stmts,
                            result: Some(Box::new(CHIRExpr::new(CHIRExprKind::Local(ptr_local), crate::ast::Type::Struct("AtomicInt64".to_string(), vec![]), ValType::I32))),
                        }),
                        crate::ast::Type::Struct("AtomicInt64".to_string(), vec![]), ValType::I32,
                    ));
                }
                // AtomicBool() → alloc 8 bytes, store 0
                if name == "AtomicBool" {
                    let alloc_idx = *self.func_indices.get("__alloc").unwrap_or(&7);
                    let ptr_local = self.alloc_local_typed(format!("__tmp_{}", self.next_local), ValType::I32);
                    let mut stmts = Vec::new();
                    stmts.push(crate::chir::CHIRStmt::Let {
                        local_idx: ptr_local,
                        value: CHIRExpr::new(
                            CHIRExprKind::Call { func_idx: alloc_idx, args: vec![CHIRExpr::new(CHIRExprKind::Integer(8), crate::ast::Type::Int32, ValType::I32)] },
                            crate::ast::Type::Int32, ValType::I32,
                        ),
                    });
                    stmts.push(crate::chir::CHIRStmt::Expr(CHIRExpr::new(
                        CHIRExprKind::Store {
                            ptr: Box::new(CHIRExpr::new(CHIRExprKind::Local(ptr_local), crate::ast::Type::Int32, ValType::I32)),
                            value: Box::new(CHIRExpr::new(CHIRExprKind::Integer(0), crate::ast::Type::Int64, ValType::I64)),
                            offset: 0, align: 3,
                        },
                        crate::ast::Type::Unit, ValType::I32,
                    )));
                    return Ok(CHIRExpr::new(
                        CHIRExprKind::Block(crate::chir::CHIRBlock {
                            stmts,
                            result: Some(Box::new(CHIRExpr::new(CHIRExprKind::Local(ptr_local), crate::ast::Type::Struct("AtomicBool".to_string(), vec![]), ValType::I32))),
                        }),
                        crate::ast::Type::Struct("AtomicBool".to_string(), vec![]), ValType::I32,
                    ));
                }
                // Mutex() / ReentrantMutex() → alloc 4 bytes (stub marker)
                if name == "Mutex" || name == "ReentrantMutex" {
                    let alloc_idx = *self.func_indices.get("__alloc").unwrap_or(&7);
                    let ptr_local = self.alloc_local_typed(format!("__tmp_{}", self.next_local), ValType::I32);
                    let struct_name = name.to_string();
                    let mut stmts = Vec::new();
                    stmts.push(crate::chir::CHIRStmt::Let {
                        local_idx: ptr_local,
                        value: CHIRExpr::new(
                            CHIRExprKind::Call { func_idx: alloc_idx, args: vec![CHIRExpr::new(CHIRExprKind::Integer(4), crate::ast::Type::Int32, ValType::I32)] },
                            crate::ast::Type::Int32, ValType::I32,
                        ),
                    });
                    return Ok(CHIRExpr::new(
                        CHIRExprKind::Block(crate::chir::CHIRBlock {
                            stmts,
                            result: Some(Box::new(CHIRExpr::new(CHIRExprKind::Local(ptr_local), crate::ast::Type::Struct(struct_name.clone(), vec![]), ValType::I32))),
                        }),
                        crate::ast::Type::Struct(struct_name, vec![]), ValType::I32,
                    ));
                }
                // Array<T>(n, init) → alloc + loop fill
                if name == "Array" && args.len() == 2 {
                    let alloc_idx = self.func_indices.get("__alloc").copied().unwrap_or(0);
                    let n_chir = self.lower_expr(&args[0])?;
                    let n_chir = self.insert_cast_if_needed(n_chir, ValType::I32);
                    let init_chir = self.lower_expr(&args[1])?;
                    let elem_size: i64 = 8; // i64 elements
                    let ptr_local = self.alloc_local_typed("__arr_ptr".into(), ValType::I32);
                    let n_local = self.alloc_local_typed("__arr_n".into(), ValType::I32);
                    let i_local = self.alloc_local_typed("__arr_i".into(), ValType::I32);
                    let ptr_get = || CHIRExpr::new(CHIRExprKind::Local(ptr_local), crate::ast::Type::Int32, ValType::I32);
                    let n_get = || CHIRExpr::new(CHIRExprKind::Local(n_local), crate::ast::Type::Int32, ValType::I32);
                    let i_get = || CHIRExpr::new(CHIRExprKind::Local(i_local), crate::ast::Type::Int32, ValType::I32);

                    // total_bytes = 4 (len) + n * elem_size
                    let size_expr = CHIRExpr::new(
                        CHIRExprKind::Binary {
                            op: crate::ast::BinOp::Add,
                            left: Box::new(CHIRExpr::new(CHIRExprKind::Integer(4), crate::ast::Type::Int32, ValType::I32)),
                            right: Box::new(CHIRExpr::new(
                                CHIRExprKind::Binary {
                                    op: crate::ast::BinOp::Mul,
                                    left: Box::new(n_get()),
                                    right: Box::new(CHIRExpr::new(CHIRExprKind::Integer(elem_size), crate::ast::Type::Int32, ValType::I32)),
                                },
                                crate::ast::Type::Int32, ValType::I32,
                            )),
                        },
                        crate::ast::Type::Int32, ValType::I32,
                    );
                    let alloc_call = CHIRExpr::new(
                        CHIRExprKind::Call { func_idx: alloc_idx, args: vec![size_expr] },
                        crate::ast::Type::Int32, ValType::I32,
                    );

                    let mut stmts = vec![
                        crate::chir::CHIRStmt::Let { local_idx: n_local, value: n_chir },
                        crate::chir::CHIRStmt::Let { local_idx: ptr_local, value: alloc_call },
                        // store len at offset 0
                        crate::chir::CHIRStmt::Expr(CHIRExpr::new(
                            CHIRExprKind::Store { ptr: Box::new(ptr_get()), value: Box::new(n_get()), offset: 0, align: 2 },
                            crate::ast::Type::Unit, ValType::I32,
                        )),
                        crate::chir::CHIRStmt::Let { local_idx: i_local, value: CHIRExpr::new(CHIRExprKind::Integer(0), crate::ast::Type::Int32, ValType::I32) },
                    ];

                    // loop: while i < n { arr[4 + i*8] = init; i++ }
                    let is_lambda_init = matches!(&args[1], Expr::Lambda { .. });
                    let init_local = self.alloc_local_typed("__arr_init".into(), if is_lambda_init { ValType::I32 } else { ValType::I64 });
                    stmts.push(crate::chir::CHIRStmt::Let { local_idx: init_local, value: init_chir });

                    let cond = CHIRExpr::new(
                        CHIRExprKind::Binary {
                            op: crate::ast::BinOp::Lt,
                            left: Box::new(i_get()),
                            right: Box::new(n_get()),
                        },
                        crate::ast::Type::Bool, ValType::I32,
                    );
                    let elem_addr = CHIRExpr::new(
                        CHIRExprKind::Binary {
                            op: crate::ast::BinOp::Add,
                            left: Box::new(ptr_get()),
                            right: Box::new(CHIRExpr::new(
                                CHIRExprKind::Binary {
                                    op: crate::ast::BinOp::Add,
                                    left: Box::new(CHIRExpr::new(CHIRExprKind::Integer(4), crate::ast::Type::Int32, ValType::I32)),
                                    right: Box::new(CHIRExpr::new(
                                        CHIRExprKind::Binary {
                                            op: crate::ast::BinOp::Mul,
                                            left: Box::new(i_get()),
                                            right: Box::new(CHIRExpr::new(CHIRExprKind::Integer(elem_size), crate::ast::Type::Int32, ValType::I32)),
                                        },
                                        crate::ast::Type::Int32, ValType::I32,
                                    )),
                                },
                                crate::ast::Type::Int32, ValType::I32,
                            )),
                        },
                        crate::ast::Type::Int32, ValType::I32,
                    );
                    let store_value = if is_lambda_init {
                        // call_indirect lambda(i) to get value
                        let i_as_i64 = CHIRExpr::new(
                            CHIRExprKind::Cast { expr: Box::new(i_get()), from_ty: ValType::I32, to_ty: ValType::I64 },
                            crate::ast::Type::Int64, ValType::I64,
                        );
                        CHIRExpr::new(
                            CHIRExprKind::CallIndirect {
                                type_idx: 0,
                                args: vec![i_as_i64],
                                callee: Box::new(CHIRExpr::new(CHIRExprKind::Local(init_local), crate::ast::Type::Int32, ValType::I32)),
                            },
                            crate::ast::Type::Int64, ValType::I64,
                        )
                    } else {
                        CHIRExpr::new(CHIRExprKind::Local(init_local), crate::ast::Type::Int64, ValType::I64)
                    };
                    let store_stmt = crate::chir::CHIRStmt::Expr(CHIRExpr::new(
                        CHIRExprKind::Store { ptr: Box::new(elem_addr), value: Box::new(store_value), offset: 0, align: 3 },
                        crate::ast::Type::Unit, ValType::I32,
                    ));
                    let inc_stmt = crate::chir::CHIRStmt::Assign {
                        target: crate::chir::CHIRLValue::Local(i_local),
                        value: CHIRExpr::new(
                            CHIRExprKind::Binary {
                                op: crate::ast::BinOp::Add,
                                left: Box::new(i_get()),
                                right: Box::new(CHIRExpr::new(CHIRExprKind::Integer(1), crate::ast::Type::Int32, ValType::I32)),
                            },
                            crate::ast::Type::Int32, ValType::I32,
                        ),
                    };
                    let loop_body = crate::chir::CHIRBlock { stmts: vec![store_stmt, inc_stmt], result: None };
                    stmts.push(crate::chir::CHIRStmt::While { cond, body: loop_body });

                    return Ok(CHIRExpr::new(
                        CHIRExprKind::Block(crate::chir::CHIRBlock { stmts, result: Some(Box::new(ptr_get())) }),
                        crate::ast::Type::Array(Box::new(crate::ast::Type::Int64)), ValType::I32,
                    ));
                }

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
                // 查找 __ClassName_init（类构造函数）或直接同名函数（struct 等）
                let init_name = format!("__{}_init", name);
                let mangled_init = format!("{}${}", init_name, args.len());
                let func_idx = self.func_indices.get(mangled_init.as_str()).copied()
                    .or_else(|| self.func_indices.get(init_name.as_str()).copied())
                    .or_else(|| self.func_indices.get(name.as_str()).copied());
                let func_idx = match func_idx {
                    Some(idx) => idx,
                    None => {
                        // 如果是已知的 struct，生成 StructNew
                        if self.struct_field_offsets.contains_key(name.as_str())
                            || self.class_field_offsets.contains_key(name.as_str()) {
                            // 按字段定义顺序获取字段名
                            let field_names: Vec<String> = self.struct_field_offsets.get(name.as_str())
                                .or_else(|| self.class_field_offsets.get(name.as_str()))
                                .map(|m| {
                                    let mut entries: Vec<(&String, &u32)> = m.iter().collect();
                                    entries.sort_by_key(|(_, off)| **off);
                                    entries.iter().map(|(n, _)| (*n).clone()).collect()
                                })
                                .unwrap_or_default();
                            let args_chir: Vec<(String, CHIRExpr)> = args.iter().enumerate()
                                .map(|(i, a)| {
                                    let arg = self.lower_expr(a).unwrap_or_else(|_| CHIRExpr::new(CHIRExprKind::Nop, crate::ast::Type::Int32, ValType::I32));
                                    let fname = field_names.get(i).cloned().unwrap_or_else(|| format!("field{}", i));
                                    (fname, arg)
                                })
                                .collect();
                            return Ok(CHIRExpr {
                                kind: CHIRExprKind::StructNew {
                                    struct_name: name.clone(),
                                    fields: args_chir,
                                },
                                ty,
                                wasm_ty: ValType::I32,
                                span: None,
                            });
                        }
                        return Ok(CHIRExpr::new(CHIRExprKind::Nop, ty, wasm_ty));
                    }
                };

                let param_tys: Vec<ValType> = self.type_ctx.functions
                    .get(init_name.as_str())
                    .or_else(|| self.type_ctx.functions.get(name.as_str()))
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
                let mut arms_chir = Vec::new();
                for arm in arms {
                    // 展开 Or 模式为多个独立 arms
                    if let crate::ast::Pattern::Or(sub_patterns) = &arm.pattern {
                        for sub_pat in sub_patterns {
                            let pattern = self.lower_pattern(sub_pat)?;
                            let body = self.lower_expr_to_block(&arm.body)?;
                            arms_chir.push(CHIRMatchArm { pattern, guard: None, body });
                        }
                    } else {
                        arms_chir.push(self.lower_match_arm(arm)?);
                    }
                }

                // 推断 match 结果类型：优先从第一个 arm 的 body 推断
                let (match_ty, match_wasm_ty) = arms_chir.first()
                    .and_then(|arm| {
                        arm.body.result.as_ref().map(|r| (r.ty.clone(), r.wasm_ty))
                            .or_else(|| arm.body.stmts.last().and_then(|s| {
                                if let crate::chir::CHIRStmt::Expr(e) = s { Some((e.ty.clone(), e.wasm_ty)) } else { None }
                            }))
                    })
                    .unwrap_or((ty.clone(), wasm_ty));

                return Ok(CHIRExpr::new(
                    CHIRExprKind::Match {
                        subject: Box::new(subject_chir),
                        arms: arms_chir,
                    },
                    match_ty, match_wasm_ty,
                ));
            }

            // try-catch-finally using __err_flag pattern
            Expr::TryBlock { body, catch_body, catch_var, finally_body, resources, .. } => {
                let has_catch = catch_var.is_some() || !catch_body.is_empty();
                let mut stmts: Vec<crate::chir::CHIRStmt> = Vec::new();

                // try-with-resources: lower resource declarations as let bindings
                for (name, init_expr) in resources {
                    let value_chir = self.lower_expr(init_expr)?;
                    let local_wasm_ty = value_chir.wasm_ty;
                    self.local_ast_types.insert(name.clone(), value_chir.ty.clone());
                    let local_idx = self.alloc_local_typed(name.clone(), local_wasm_ty);
                    stmts.push(crate::chir::CHIRStmt::Let { local_idx, value: value_chir });
                }

                if has_catch {
                    let err_flag = self.alloc_local_typed("__err_flag".into(), ValType::I32);
                    let err_val = self.alloc_local_typed("__err_val".into(), ValType::I64);
                    // init err_flag = 0
                    stmts.push(crate::chir::CHIRStmt::Let {
                        local_idx: err_flag,
                        value: CHIRExpr::new(CHIRExprKind::Integer(0), crate::ast::Type::Int32, ValType::I32),
                    });
                    stmts.push(crate::chir::CHIRStmt::Let {
                        local_idx: err_val,
                        value: CHIRExpr::new(CHIRExprKind::Integer(0), crate::ast::Type::Int64, ValType::I64),
                    });

                    self.err_flag_stack.push(err_flag);
                    self.err_val_stack.push(err_val);

                    // try body: wrap each stmt in if(!err_flag) { stmt }
                    for stmt in body {
                        if let Ok(s) = self.lower_stmt(stmt) {
                            let flag_get = CHIRExpr::new(CHIRExprKind::Local(err_flag), crate::ast::Type::Int32, ValType::I32);
                            let not_flag = CHIRExpr::new(
                                CHIRExprKind::Unary { op: crate::ast::UnaryOp::Not, expr: Box::new(flag_get) },
                                crate::ast::Type::Bool, ValType::I32,
                            );
                            let body_block = crate::chir::CHIRBlock { stmts: vec![s], result: None };
                            stmts.push(crate::chir::CHIRStmt::Expr(CHIRExpr::new(
                                CHIRExprKind::If { cond: Box::new(not_flag), then_block: body_block, else_block: None },
                                crate::ast::Type::Unit, ValType::I32,
                            )));
                        }
                    }

                    self.err_flag_stack.pop();
                    self.err_val_stack.pop();

                    // catch: if err_flag { let catch_var = err_val; catch_body }
                    if !catch_body.is_empty() {
                        let mut catch_stmts: Vec<crate::chir::CHIRStmt> = Vec::new();
                        if let Some(cv) = catch_var {
                            let cv_local = self.alloc_local_typed(cv.clone(), ValType::I64);
                            catch_stmts.push(crate::chir::CHIRStmt::Let {
                                local_idx: cv_local,
                                value: CHIRExpr::new(CHIRExprKind::Local(err_val), crate::ast::Type::Int64, ValType::I64),
                            });
                        }
                        for stmt in catch_body {
                            if let Ok(s) = self.lower_stmt(stmt) {
                                catch_stmts.push(s);
                            }
                        }
                        let catch_block = crate::chir::CHIRBlock { stmts: catch_stmts, result: None };
                        let flag_get = CHIRExpr::new(CHIRExprKind::Local(err_flag), crate::ast::Type::Int32, ValType::I32);
                        stmts.push(crate::chir::CHIRStmt::Expr(CHIRExpr::new(
                            CHIRExprKind::If { cond: Box::new(flag_get), then_block: catch_block, else_block: None },
                            crate::ast::Type::Unit, ValType::I32,
                        )));
                    }
                } else {
                    // no catch: execute try body directly
                    for stmt in body {
                        if let Ok(s) = self.lower_stmt(stmt) {
                            stmts.push(s);
                        }
                    }
                }

                // finally: always execute
                if let Some(fin_stmts) = finally_body {
                    for stmt in fin_stmts {
                        if let Ok(s) = self.lower_stmt(stmt) {
                            stmts.push(s);
                        }
                    }
                }
                if stmts.is_empty() {
                    CHIRExprKind::Nop
                } else {
                    return Ok(CHIRExpr::new(
                        CHIRExprKind::Block(crate::chir::CHIRBlock { stmts, result: None }),
                        crate::ast::Type::Unit,
                        ValType::I32,
                    ));
                }
            }

            // 类型转换 (as)
            Expr::Cast { expr: inner, target_ty } => {
                let inner_chir = self.lower_expr(inner)?;
                let target_wasm = target_ty.to_wasm();
                let cast_chir = self.insert_cast_if_needed(inner_chir, target_wasm);
                return Ok(CHIRExpr::new(cast_chir.kind, target_ty.clone(), target_wasm));
            }

            // 枚举变体构造
            Expr::VariantConst { enum_name, variant_name, arg } => {
                let enum_def = self.enum_defs.iter().find(|e| e.name == *enum_name);
                let disc = enum_def
                    .and_then(|e| e.variant_index(variant_name))
                    .unwrap_or(0) as i64;
                let has_payload = enum_def.map_or(false, |e| e.has_payload());
                if has_payload {
                    let alloc_idx = self.func_indices.get("__alloc").copied().unwrap_or(0);
                    let tmp = self.alloc_local_typed("__enum_tmp".into(), ValType::I32);
                    let tmp_get = || CHIRExpr::new(
                        CHIRExprKind::Local(tmp),
                        crate::ast::Type::Int32, ValType::I32,
                    );
                    let disc_expr = CHIRExpr::new(
                        CHIRExprKind::Integer(disc),
                        crate::ast::Type::Int32, ValType::I32,
                    );
                    if let Some(arg_expr) = arg {
                        let arg_chir = self.lower_expr(arg_expr)?;
                        let alloc_size = CHIRExpr::new(
                            CHIRExprKind::Integer(12),
                            crate::ast::Type::Int32, ValType::I32,
                        );
                        let alloc_call = CHIRExpr::new(
                            CHIRExprKind::Call { func_idx: alloc_idx, args: vec![alloc_size] },
                            crate::ast::Type::Int32, ValType::I32,
                        );
                        let stmts = vec![
                            crate::chir::CHIRStmt::Let { local_idx: tmp, value: alloc_call },
                            crate::chir::CHIRStmt::Expr(CHIRExpr::new(
                                CHIRExprKind::Store { ptr: Box::new(tmp_get()), value: Box::new(disc_expr), offset: 0, align: 2 },
                                crate::ast::Type::Unit, ValType::I32,
                            )),
                            crate::chir::CHIRStmt::Expr(CHIRExpr::new(
                                CHIRExprKind::Store { ptr: Box::new(tmp_get()), value: Box::new(arg_chir), offset: 4, align: 3 },
                                crate::ast::Type::Unit, ValType::I32,
                            )),
                        ];
                        let result = Some(Box::new(tmp_get()));
                        return Ok(CHIRExpr::new(
                            CHIRExprKind::Block(crate::chir::CHIRBlock { stmts, result }),
                            crate::ast::Type::Int32, ValType::I32,
                        ));
                    } else {
                        let alloc_size = CHIRExpr::new(
                            CHIRExprKind::Integer(4),
                            crate::ast::Type::Int32, ValType::I32,
                        );
                        let alloc_call = CHIRExpr::new(
                            CHIRExprKind::Call { func_idx: alloc_idx, args: vec![alloc_size] },
                            crate::ast::Type::Int32, ValType::I32,
                        );
                        let stmts = vec![
                            crate::chir::CHIRStmt::Let { local_idx: tmp, value: alloc_call },
                            crate::chir::CHIRStmt::Expr(CHIRExpr::new(
                                CHIRExprKind::Store { ptr: Box::new(tmp_get()), value: Box::new(disc_expr), offset: 0, align: 2 },
                                crate::ast::Type::Unit, ValType::I32,
                            )),
                        ];
                        let result = Some(Box::new(tmp_get()));
                        return Ok(CHIRExpr::new(
                            CHIRExprKind::Block(crate::chir::CHIRBlock { stmts, result }),
                            crate::ast::Type::Int32, ValType::I32,
                        ));
                    }
                } else {
                    return Ok(CHIRExpr::new(
                        CHIRExprKind::Integer(disc),
                        crate::ast::Type::Int32, ValType::I32,
                    ));
                }
            }

            // Option::Some(v) → alloc [tag=1][value]
            Expr::Some(inner) => {
                let alloc_idx = self.func_indices.get("__alloc").copied().unwrap_or(0);
                let inner_chir = self.lower_expr(inner)?;
                let value_wasm = inner_chir.wasm_ty;
                let value_size: i64 = if value_wasm == ValType::I64 || value_wasm == ValType::F64 { 8 } else { 4 };
                let total_size = 4 + value_size;
                let store_align: u32 = if value_size == 8 { 3 } else { 2 };
                let tmp = self.alloc_local_typed("__opt_tmp".into(), ValType::I32);
                let tmp_get = || CHIRExpr::new(CHIRExprKind::Local(tmp), crate::ast::Type::Int32, ValType::I32);
                let alloc_call = CHIRExpr::new(
                    CHIRExprKind::Call { func_idx: alloc_idx, args: vec![
                        CHIRExpr::new(CHIRExprKind::Integer(total_size), crate::ast::Type::Int32, ValType::I32)
                    ] },
                    crate::ast::Type::Int32, ValType::I32,
                );
                let tag_one = CHIRExpr::new(CHIRExprKind::Integer(1), crate::ast::Type::Int32, ValType::I32);
                let stmts = vec![
                    crate::chir::CHIRStmt::Let { local_idx: tmp, value: alloc_call },
                    crate::chir::CHIRStmt::Expr(CHIRExpr::new(
                        CHIRExprKind::Store { ptr: Box::new(tmp_get()), value: Box::new(tag_one), offset: 0, align: 2 },
                        crate::ast::Type::Unit, ValType::I32,
                    )),
                    crate::chir::CHIRStmt::Expr(CHIRExpr::new(
                        CHIRExprKind::Store { ptr: Box::new(tmp_get()), value: Box::new(inner_chir), offset: 4, align: store_align },
                        crate::ast::Type::Unit, ValType::I32,
                    )),
                ];
                return Ok(CHIRExpr::new(
                    CHIRExprKind::Block(crate::chir::CHIRBlock { stmts, result: Some(Box::new(tmp_get())) }),
                    crate::ast::Type::Int32, ValType::I32,
                ));
            }

            // Option::None → alloc [tag=0]
            Expr::None => {
                let alloc_idx = self.func_indices.get("__alloc").copied().unwrap_or(0);
                let tmp = self.alloc_local_typed("__opt_tmp".into(), ValType::I32);
                let tmp_get = || CHIRExpr::new(CHIRExprKind::Local(tmp), crate::ast::Type::Int32, ValType::I32);
                let alloc_call = CHIRExpr::new(
                    CHIRExprKind::Call { func_idx: alloc_idx, args: vec![
                        CHIRExpr::new(CHIRExprKind::Integer(4), crate::ast::Type::Int32, ValType::I32)
                    ] },
                    crate::ast::Type::Int32, ValType::I32,
                );
                let tag_zero = CHIRExpr::new(CHIRExprKind::Integer(0), crate::ast::Type::Int32, ValType::I32);
                let stmts = vec![
                    crate::chir::CHIRStmt::Let { local_idx: tmp, value: alloc_call },
                    crate::chir::CHIRStmt::Expr(CHIRExpr::new(
                        CHIRExprKind::Store { ptr: Box::new(tmp_get()), value: Box::new(tag_zero), offset: 0, align: 2 },
                        crate::ast::Type::Unit, ValType::I32,
                    )),
                ];
                return Ok(CHIRExpr::new(
                    CHIRExprKind::Block(crate::chir::CHIRBlock { stmts, result: Some(Box::new(tmp_get())) }),
                    crate::ast::Type::Int32, ValType::I32,
                ));
            }

            // a ?? b: if option.tag == 1 then option.value else default
            Expr::NullCoalesce { option, default } => {
                let opt_chir = self.lower_expr(option)?;
                let default_chir = self.lower_expr(default)?;
                let result_wasm = default_chir.wasm_ty;
                let result_ty = default_chir.ty.clone();
                let value_align: u32 = if result_wasm == ValType::I64 || result_wasm == ValType::F64 { 3 } else { 2 };
                let tmp = self.alloc_local_typed("__nc_ptr".into(), ValType::I32);
                let tmp_get = || CHIRExpr::new(CHIRExprKind::Local(tmp), crate::ast::Type::Int32, ValType::I32);
                let tag_load = CHIRExpr::new(
                    CHIRExprKind::Load { ptr: Box::new(tmp_get()), offset: 0, align: 2 },
                    crate::ast::Type::Int32, ValType::I32,
                );
                let value_load = CHIRExpr::new(
                    CHIRExprKind::Load { ptr: Box::new(tmp_get()), offset: 4, align: value_align },
                    result_ty.clone(), result_wasm,
                );
                let cond = CHIRExpr::new(
                    CHIRExprKind::Binary {
                        op: crate::ast::BinOp::Eq,
                        left: Box::new(tag_load),
                        right: Box::new(CHIRExpr::new(CHIRExprKind::Integer(1), crate::ast::Type::Int32, ValType::I32)),
                    },
                    crate::ast::Type::Bool, ValType::I32,
                );
                let then_block = crate::chir::CHIRBlock { stmts: vec![], result: Some(Box::new(value_load)) };
                let else_block = crate::chir::CHIRBlock { stmts: vec![], result: Some(Box::new(default_chir)) };
                let stmts = vec![
                    crate::chir::CHIRStmt::Let { local_idx: tmp, value: opt_chir },
                ];
                let if_expr = CHIRExpr::new(
                    CHIRExprKind::If { cond: Box::new(cond), then_block, else_block: Some(else_block) },
                    result_ty.clone(), result_wasm,
                );
                return Ok(CHIRExpr::new(
                    CHIRExprKind::Block(crate::chir::CHIRBlock { stmts, result: Some(Box::new(if_expr)) }),
                    result_ty, result_wasm,
                ));
            }

            // if-let → desugar to Match
            Expr::IfLet { pattern, expr, then_branch, else_branch } => {
                let else_expr = else_branch
                    .clone()
                    .unwrap_or_else(|| Box::new(Expr::Integer(0)));
                let match_expr = Expr::Match {
                    expr: expr.clone(),
                    arms: vec![
                        crate::ast::MatchArm {
                            pattern: pattern.clone(),
                            guard: None,
                            body: then_branch.clone(),
                        },
                        crate::ast::MatchArm {
                            pattern: crate::ast::Pattern::Wildcard,
                            guard: None,
                            body: else_expr,
                        },
                    ],
                };
                return self.lower_expr(&match_expr);
            }

            // Result::Ok(v) → alloc [tag=0][value]
            Expr::Ok(inner) => {
                let alloc_idx = self.func_indices.get("__alloc").copied().unwrap_or(0);
                let inner_chir = self.lower_expr(inner)?;
                let value_wasm = inner_chir.wasm_ty;
                let value_size: i64 = if value_wasm == ValType::I64 || value_wasm == ValType::F64 { 8 } else { 4 };
                let total_size = 4 + value_size;
                let store_align: u32 = if value_size == 8 { 3 } else { 2 };
                let tmp = self.alloc_local_typed("__res_tmp".into(), ValType::I32);
                let tmp_get = || CHIRExpr::new(CHIRExprKind::Local(tmp), crate::ast::Type::Int32, ValType::I32);
                let alloc_call = CHIRExpr::new(
                    CHIRExprKind::Call { func_idx: alloc_idx, args: vec![
                        CHIRExpr::new(CHIRExprKind::Integer(total_size), crate::ast::Type::Int32, ValType::I32)
                    ] },
                    crate::ast::Type::Int32, ValType::I32,
                );
                let tag_zero = CHIRExpr::new(CHIRExprKind::Integer(0), crate::ast::Type::Int32, ValType::I32);
                let stmts = vec![
                    crate::chir::CHIRStmt::Let { local_idx: tmp, value: alloc_call },
                    crate::chir::CHIRStmt::Expr(CHIRExpr::new(
                        CHIRExprKind::Store { ptr: Box::new(tmp_get()), value: Box::new(tag_zero), offset: 0, align: 2 },
                        crate::ast::Type::Unit, ValType::I32,
                    )),
                    crate::chir::CHIRStmt::Expr(CHIRExpr::new(
                        CHIRExprKind::Store { ptr: Box::new(tmp_get()), value: Box::new(inner_chir), offset: 4, align: store_align },
                        crate::ast::Type::Unit, ValType::I32,
                    )),
                ];
                return Ok(CHIRExpr::new(
                    CHIRExprKind::Block(crate::chir::CHIRBlock { stmts, result: Some(Box::new(tmp_get())) }),
                    crate::ast::Type::Int32, ValType::I32,
                ));
            }

            // Result::Err(e) → alloc [tag=1][error_ptr]
            Expr::Err(inner) => {
                let alloc_idx = self.func_indices.get("__alloc").copied().unwrap_or(0);
                let inner_chir = self.lower_expr(inner)?;
                let tmp = self.alloc_local_typed("__res_tmp".into(), ValType::I32);
                let tmp_get = || CHIRExpr::new(CHIRExprKind::Local(tmp), crate::ast::Type::Int32, ValType::I32);
                let alloc_call = CHIRExpr::new(
                    CHIRExprKind::Call { func_idx: alloc_idx, args: vec![
                        CHIRExpr::new(CHIRExprKind::Integer(8), crate::ast::Type::Int32, ValType::I32)
                    ] },
                    crate::ast::Type::Int32, ValType::I32,
                );
                let tag_one = CHIRExpr::new(CHIRExprKind::Integer(1), crate::ast::Type::Int32, ValType::I32);
                let stmts = vec![
                    crate::chir::CHIRStmt::Let { local_idx: tmp, value: alloc_call },
                    crate::chir::CHIRStmt::Expr(CHIRExpr::new(
                        CHIRExprKind::Store { ptr: Box::new(tmp_get()), value: Box::new(tag_one), offset: 0, align: 2 },
                        crate::ast::Type::Unit, ValType::I32,
                    )),
                    crate::chir::CHIRStmt::Expr(CHIRExpr::new(
                        CHIRExprKind::Store { ptr: Box::new(tmp_get()), value: Box::new(inner_chir), offset: 4, align: 2 },
                        crate::ast::Type::Unit, ValType::I32,
                    )),
                ];
                return Ok(CHIRExpr::new(
                    CHIRExprKind::Block(crate::chir::CHIRBlock { stmts, result: Some(Box::new(tmp_get())) }),
                    crate::ast::Type::Int32, ValType::I32,
                ));
            }

            // throw expr → set err_flag=1, err_val=expr (only inside try-catch)
            Expr::Throw(inner) => {
                if let (Some(&err_flag), Some(&err_val)) = (self.err_flag_stack.last(), self.err_val_stack.last()) {
                    let inner_chir = self.lower_expr(inner)?;
                    let inner_chir = self.insert_cast_if_needed(inner_chir, ValType::I64);
                    let stmts = vec![
                        crate::chir::CHIRStmt::Assign {
                            target: crate::chir::CHIRLValue::Local(err_val),
                            value: inner_chir,
                        },
                        crate::chir::CHIRStmt::Assign {
                            target: crate::chir::CHIRLValue::Local(err_flag),
                            value: CHIRExpr::new(CHIRExprKind::Integer(1), crate::ast::Type::Int32, ValType::I32),
                        },
                    ];
                    return Ok(CHIRExpr::new(
                        CHIRExprKind::Block(crate::chir::CHIRBlock { stmts, result: None }),
                        crate::ast::Type::Unit, ValType::I32,
                    ));
                } else {
                    CHIRExprKind::Unreachable
                }
            }

            // Lambda → 返回 __lambda_N 的函数索引作为 i32 table index
            Expr::Lambda { .. } => {
                let idx = self.lambda_counter;
                self.lambda_counter += 1;
                let lambda_name = format!("__lambda_{}", idx);
                if let Some(&func_idx) = self.func_indices.get(&lambda_name) {
                    CHIRExprKind::Integer(func_idx as i64)
                } else {
                    CHIRExprKind::Integer(0)
                }
            }

            // 尾随闭包：callee(args) { params => body } → callee(args, lambda)
            Expr::TrailingClosure { callee, args, closure } => {
                if let Expr::Call { name, type_args, named_args, .. } = callee.as_ref() {
                    let mut all_args = args.clone();
                    all_args.push(*closure.clone());
                    let combined = Expr::Call {
                        name: name.clone(),
                        args: all_args,
                        type_args: type_args.clone(),
                        named_args: named_args.clone(),
                    };
                    return self.lower_expr(&combined);
                }
                CHIRExprKind::Nop
            }

            // Range 表达式：5..10 → 分配 [start:i64, end:i64]
            Expr::Range { start, end, .. } => {
                let start_chir = self.lower_expr(start)?;
                let start_chir = self.insert_cast_if_needed(start_chir, ValType::I64);
                let end_chir = self.lower_expr(end)?;
                let end_chir = self.insert_cast_if_needed(end_chir, ValType::I64);

                let alloc_idx = *self.func_indices.get("__alloc").unwrap_or(&7);
                let ptr_local = self.alloc_local_typed(format!("__tmp_{}", self.next_local), ValType::I32);
                let start_local = self.alloc_local_typed(format!("__tmp_{}", self.next_local), ValType::I64);
                let end_local = self.alloc_local_typed(format!("__tmp_{}", self.next_local), ValType::I64);

                let mut stmts = Vec::new();
                // save start/end
                stmts.push(crate::chir::CHIRStmt::Let {
                    local_idx: start_local,
                    value: start_chir,
                });
                stmts.push(crate::chir::CHIRStmt::Let {
                    local_idx: end_local,
                    value: end_chir,
                });
                // alloc 16 bytes
                stmts.push(crate::chir::CHIRStmt::Let {
                    local_idx: ptr_local,
                    value: CHIRExpr::new(
                        CHIRExprKind::Call {
                            func_idx: alloc_idx,
                            args: vec![CHIRExpr::new(CHIRExprKind::Integer(16), crate::ast::Type::Int32, ValType::I32)],
                        },
                        crate::ast::Type::Int32, ValType::I32,
                    ),
                });
                // store start at offset 0
                stmts.push(crate::chir::CHIRStmt::Expr(CHIRExpr::new(
                    CHIRExprKind::Store {
                        ptr: Box::new(CHIRExpr::new(CHIRExprKind::Local(ptr_local), crate::ast::Type::Int32, ValType::I32)),
                        value: Box::new(CHIRExpr::new(CHIRExprKind::Local(start_local), crate::ast::Type::Int64, ValType::I64)),
                        offset: 0, align: 3,
                    },
                    crate::ast::Type::Unit, ValType::I32,
                )));
                // store end at offset 8
                stmts.push(crate::chir::CHIRStmt::Expr(CHIRExpr::new(
                    CHIRExprKind::Store {
                        ptr: Box::new(CHIRExpr::new(CHIRExprKind::Local(ptr_local), crate::ast::Type::Int32, ValType::I32)),
                        value: Box::new(CHIRExpr::new(CHIRExprKind::Local(end_local), crate::ast::Type::Int64, ValType::I64)),
                        offset: 8, align: 3,
                    },
                    crate::ast::Type::Unit, ValType::I32,
                )));

                return Ok(CHIRExpr::new(
                    CHIRExprKind::Block(crate::chir::CHIRBlock {
                        stmts,
                        result: Some(Box::new(CHIRExpr::new(
                            CHIRExprKind::Local(ptr_local), crate::ast::Type::Struct("Range".to_string(), vec![]), ValType::I32,
                        ))),
                    }),
                    crate::ast::Type::Struct("Range".to_string(), vec![]), ValType::I32,
                ));
            }

            // 可选链 b?.field → b.field (单线程非 null 简化)
            Expr::OptionalChain { object, field } => {
                let field_expr = Expr::Field {
                    object: object.clone(),
                    field: field.clone(),
                };
                return self.lower_expr(&field_expr);
            }

            // spawn { body } → 单线程桩：直接执行 body
            Expr::Spawn { body } => {
                let mut stmts = Vec::new();
                for stmt in body {
                    stmts.push(self.lower_stmt(stmt)?);
                }
                return Ok(CHIRExpr::new(
                    CHIRExprKind::Block(crate::chir::CHIRBlock { stmts, result: None }),
                    crate::ast::Type::Unit, ValType::I32,
                ));
            }

            // synchronized(lock) { body } → 单线程桩：直接执行 body
            Expr::Synchronized { lock: _, body } => {
                let mut stmts = Vec::new();
                for stmt in body {
                    stmts.push(self.lower_stmt(stmt)?);
                }
                return Ok(CHIRExpr::new(
                    CHIRExprKind::Block(crate::chir::CHIRBlock { stmts, result: None }),
                    crate::ast::Type::Unit, ValType::I32,
                ));
            }

            // @Assert(a, b) → if a != b { unreachable }; @Assert(cond) → if !cond { unreachable }
            Expr::Macro { name, args } if name == "Assert" || name == "Expect" => {
                if args.len() == 2 {
                    let left = self.lower_expr(&args[0])?;
                    let right = self.lower_expr(&args[1])?;
                    let left = self.insert_cast_if_needed(left, ValType::I64);
                    let right = self.insert_cast_if_needed(right, ValType::I64);
                    let cond = CHIRExpr::new(
                        CHIRExprKind::Binary { op: crate::ast::BinOp::NotEq, left: Box::new(left), right: Box::new(right) },
                        crate::ast::Type::Bool, ValType::I32,
                    );
                    let then_block = crate::chir::CHIRBlock {
                        stmts: vec![],
                        result: Some(Box::new(CHIRExpr::new(CHIRExprKind::Unreachable, crate::ast::Type::Unit, ValType::I32))),
                    };
                    return Ok(CHIRExpr::new(
                        CHIRExprKind::If { cond: Box::new(cond), then_block, else_block: None },
                        crate::ast::Type::Unit, ValType::I32,
                    ));
                } else if args.len() == 1 {
                    let cond_expr = self.lower_expr(&args[0])?;
                    let cond = self.insert_cast_if_needed(cond_expr, ValType::I32);
                    let negated = CHIRExpr::new(
                        CHIRExprKind::Unary { op: crate::ast::UnaryOp::Not, expr: Box::new(cond) },
                        crate::ast::Type::Bool, ValType::I32,
                    );
                    let then_block = crate::chir::CHIRBlock {
                        stmts: vec![],
                        result: Some(Box::new(CHIRExpr::new(CHIRExprKind::Unreachable, crate::ast::Type::Unit, ValType::I32))),
                    };
                    return Ok(CHIRExpr::new(
                        CHIRExprKind::If { cond: Box::new(negated), then_block, else_block: None },
                        crate::ast::Type::Unit, ValType::I32,
                    ));
                }
                CHIRExprKind::Nop
            }

            Expr::Interpolate(parts) => {
                let concat_idx = self.func_indices.get("__str_concat").copied();
                let i64_to_str = self.func_indices.get("__i64_to_str").copied();
                let f64_to_str = self.func_indices.get("__f64_to_str").copied();
                let bool_to_str = self.func_indices.get("__bool_to_str").copied();

                let mut result: Option<CHIRExpr> = None;
                for part in parts {
                    let part_expr = match part {
                        crate::ast::InterpolatePart::Literal(s) => {
                            CHIRExpr::new(CHIRExprKind::String(s.clone()), Type::String, ValType::I32)
                        }
                        crate::ast::InterpolatePart::Expr(e) => {
                            let inner = self.lower_expr(e)?;
                            match inner.wasm_ty {
                                ValType::I64 => {
                                    if let Some(idx) = i64_to_str {
                                        CHIRExpr::new(
                                            CHIRExprKind::Call { func_idx: idx, args: vec![inner] },
                                            Type::String, ValType::I32,
                                        )
                                    } else { inner }
                                }
                                ValType::F64 => {
                                    if let Some(idx) = f64_to_str {
                                        CHIRExpr::new(
                                            CHIRExprKind::Call { func_idx: idx, args: vec![inner] },
                                            Type::String, ValType::I32,
                                        )
                                    } else { inner }
                                }
                                _ => {
                                    if matches!(inner.ty, Type::Bool) {
                                        if let Some(idx) = bool_to_str {
                                            CHIRExpr::new(
                                                CHIRExprKind::Call { func_idx: idx, args: vec![inner] },
                                                Type::String, ValType::I32,
                                            )
                                        } else { inner }
                                    } else if matches!(inner.ty, Type::String) {
                                        inner
                                    } else if let Some(idx) = i64_to_str {
                                        let cast = self.insert_cast_if_needed(inner, ValType::I64);
                                        CHIRExpr::new(
                                            CHIRExprKind::Call { func_idx: idx, args: vec![cast] },
                                            Type::String, ValType::I32,
                                        )
                                    } else {
                                        inner
                                    }
                                }
                            }
                        }
                    };
                    result = Some(match result {
                        None => part_expr,
                        Some(prev) => {
                            if let Some(idx) = concat_idx {
                                CHIRExpr::new(
                                    CHIRExprKind::Call { func_idx: idx, args: vec![prev, part_expr] },
                                    Type::String, ValType::I32,
                                )
                            } else {
                                part_expr
                            }
                        }
                    });
                }
                return Ok(result.unwrap_or_else(|| CHIRExpr::new(CHIRExprKind::String(String::new()), Type::String, ValType::I32)));
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
            Type::Struct(name, type_args) => {
                let names = Self::resolve_type_names(name, type_args);
                for n in &names {
                    if let Some(info) = self.class_field_info.get(n.as_str())
                        .and_then(|fields| fields.get(field)) {
                        return Ok(info.0);
                    }
                    if let Some(&offset) = self.class_field_offsets.get(n.as_str())
                        .and_then(|fields| fields.get(field)) {
                        return Ok(offset);
                    }
                    if let Some(&offset) = self.struct_field_offsets
                        .get(n.as_str())
                        .and_then(|fields| fields.get(field)) {
                        return Ok(offset);
                    }
                }
                Ok(0)
            }
            _ => {
                for (_class_name, fields) in self.class_field_info.iter() {
                    if let Some(info) = fields.get(field) {
                        return Ok(info.0);
                    }
                }
                Ok(0)
            },
        }
    }

    fn resolve_type_names(name: &str, type_args: &[Type]) -> Vec<String> {
        let mut names = Vec::new();
        if !type_args.is_empty() {
            names.push(crate::monomorph::mangle_name(name, type_args));
        }
        names.push(name.to_string());
        names
    }

    /// 将表达式转换为块
    /// 内置类型（Int64, Float64, Bool, String）的方法调用
    fn try_lower_builtin_method(
        &mut self,
        object: &Expr,
        obj_ty: &crate::ast::Type,
        method: &str,
        args: &[Expr],
    ) -> Result<Option<CHIRExpr>, String> {
        use crate::ast::Type;
        match obj_ty {
            Type::Int64 | Type::Int32 | Type::Int8 | Type::Int16 => {
                match method {
                    "format" if args.len() == 1 => {
                        // fallback: format(spec) → toString() (ignores spec)
                        if let Some(&func_idx) = self.func_indices.get("__i64_to_str") {
                            let inner = self.lower_expr(object)?;
                            let inner = self.insert_cast_if_needed(inner, ValType::I64);
                            return Ok(Some(CHIRExpr::new(
                                CHIRExprKind::Call { func_idx, args: vec![inner] },
                                Type::String, ValType::I32,
                            )));
                        }
                    }
                    "toFloat64" => {
                        let inner = self.lower_expr(object)?;
                        let inner = self.insert_cast_if_needed(inner, ValType::I64);
                        return Ok(Some(CHIRExpr::new(
                            CHIRExprKind::Cast { expr: Box::new(inner), from_ty: ValType::I64, to_ty: ValType::F64 },
                            Type::Float64, ValType::F64,
                        )));
                    }
                    "abs" => {
                        let inner = self.lower_expr(object)?;
                        let inner = self.insert_cast_if_needed(inner, ValType::I64);
                        let local = self.alloc_local_typed("__abs_tmp".into(), ValType::I64);
                        return Ok(Some(CHIRExpr::new(
                            CHIRExprKind::BuiltinAbs { val: Box::new(inner), tmp_local: local },
                            Type::Int64, ValType::I64,
                        )));
                    }
                    "compareTo" if args.len() == 1 => {
                        let left = self.lower_expr(object)?;
                        let left = self.insert_cast_if_needed(left, ValType::I64);
                        let right = self.lower_expr(&args[0])?;
                        let right = self.insert_cast_if_needed(right, ValType::I64);
                        return Ok(Some(CHIRExpr::new(
                            CHIRExprKind::BuiltinCompareTo { left: Box::new(left), right: Box::new(right) },
                            Type::Int64, ValType::I64,
                        )));
                    }
                    "toString" => {
                        let inner = self.lower_expr(object)?;
                        let inner = self.insert_cast_if_needed(inner, ValType::I64);
                        if let Some(&func_idx) = self.func_indices.get("__i64_to_str") {
                            return Ok(Some(CHIRExpr::new(
                                CHIRExprKind::Call { func_idx, args: vec![inner] },
                                Type::String, ValType::I32,
                            )));
                        }
                    }
                    _ => {}
                }
            }
            Type::Float64 | Type::Float32 => {
                match method {
                    "format" if args.len() == 1 => {
                        if let Some(&func_idx) = self.func_indices.get("__f64_to_str") {
                            let inner = self.lower_expr(object)?;
                            let inner = self.insert_cast_if_needed(inner, ValType::F64);
                            return Ok(Some(CHIRExpr::new(
                                CHIRExprKind::Call { func_idx, args: vec![inner] },
                                Type::String, ValType::I32,
                            )));
                        }
                    }
                    "toInt64" => {
                        let inner = self.lower_expr(object)?;
                        let inner = self.insert_cast_if_needed(inner, ValType::F64);
                        return Ok(Some(CHIRExpr::new(
                            CHIRExprKind::Cast { expr: Box::new(inner), from_ty: ValType::F64, to_ty: ValType::I64 },
                            Type::Int64, ValType::I64,
                        )));
                    }
                    "toString" => {
                        let inner = self.lower_expr(object)?;
                        let inner = self.insert_cast_if_needed(inner, ValType::F64);
                        if let Some(&func_idx) = self.func_indices.get("__f64_to_str") {
                            return Ok(Some(CHIRExpr::new(
                                CHIRExprKind::Call { func_idx, args: vec![inner] },
                                Type::String, ValType::I32,
                            )));
                        }
                    }
                    _ => {}
                }
            }
            Type::Bool => {
                if method == "toString" {
                    let inner = self.lower_expr(object)?;
                    if let Some(&func_idx) = self.func_indices.get("__bool_to_str") {
                        return Ok(Some(CHIRExpr::new(
                            CHIRExprKind::Call { func_idx, args: vec![inner] },
                            Type::String, ValType::I32,
                        )));
                    }
                }
            }
            Type::String => {
                match method {
                    "toInt64" => {
                        let inner = self.lower_expr(object)?;
                        let inner = self.insert_cast_if_needed(inner, ValType::I32);
                        if let Some(&func_idx) = self.func_indices.get("__str_to_i64") {
                            return Ok(Some(CHIRExpr::new(
                                CHIRExprKind::Call { func_idx, args: vec![inner] },
                                Type::Int64, ValType::I64,
                            )));
                        }
                    }
                    "isEmpty" => {
                        let inner = self.lower_expr(object)?;
                        let inner = self.insert_cast_if_needed(inner, ValType::I32);
                        return Ok(Some(CHIRExpr::new(
                            CHIRExprKind::BuiltinStringIsEmpty { val: Box::new(inner) },
                            Type::Bool, ValType::I32,
                        )));
                    }
                    "contains" if args.len() == 1 => {
                        if let Some(&func_idx) = self.func_indices.get("__str_contains") {
                            let obj = self.lower_expr(object)?;
                            let obj = self.insert_cast_if_needed(obj, ValType::I32);
                            let arg = self.lower_expr(&args[0])?;
                            let arg = self.insert_cast_if_needed(arg, ValType::I32);
                            return Ok(Some(CHIRExpr::new(
                                CHIRExprKind::Call { func_idx, args: vec![obj, arg] },
                                Type::Bool, ValType::I32,
                            )));
                        }
                    }
                    "startsWith" if args.len() == 1 => {
                        if let Some(&func_idx) = self.func_indices.get("__str_starts_with") {
                            let obj = self.lower_expr(object)?;
                            let obj = self.insert_cast_if_needed(obj, ValType::I32);
                            let arg = self.lower_expr(&args[0])?;
                            let arg = self.insert_cast_if_needed(arg, ValType::I32);
                            return Ok(Some(CHIRExpr::new(
                                CHIRExprKind::Call { func_idx, args: vec![obj, arg] },
                                Type::Bool, ValType::I32,
                            )));
                        }
                    }
                    "endsWith" if args.len() == 1 => {
                        if let Some(&func_idx) = self.func_indices.get("__str_ends_with") {
                            let obj = self.lower_expr(object)?;
                            let obj = self.insert_cast_if_needed(obj, ValType::I32);
                            let arg = self.lower_expr(&args[0])?;
                            let arg = self.insert_cast_if_needed(arg, ValType::I32);
                            return Ok(Some(CHIRExpr::new(
                                CHIRExprKind::Call { func_idx, args: vec![obj, arg] },
                                Type::Bool, ValType::I32,
                            )));
                        }
                    }
                    "trim" => {
                        if let Some(&func_idx) = self.func_indices.get("__str_trim") {
                            let obj = self.lower_expr(object)?;
                            let obj = self.insert_cast_if_needed(obj, ValType::I32);
                            return Ok(Some(CHIRExpr::new(
                                CHIRExprKind::Call { func_idx, args: vec![obj] },
                                Type::String, ValType::I32,
                            )));
                        }
                    }
                    "toArray" if args.is_empty() => {
                        if let Some(&func_idx) = self.func_indices.get("__str_to_array") {
                            let obj = self.lower_expr(object)?;
                            let obj = self.insert_cast_if_needed(obj, ValType::I32);
                            return Ok(Some(CHIRExpr::new(
                                CHIRExprKind::Call { func_idx, args: vec![obj] },
                                Type::Array(Box::new(Type::Int64)), ValType::I32,
                            )));
                        }
                    }
                    "indexOf" if args.len() == 1 => {
                        if let Some(&func_idx) = self.func_indices.get("__str_index_of") {
                            let obj = self.lower_expr(object)?;
                            let obj = self.insert_cast_if_needed(obj, ValType::I32);
                            let arg = self.lower_expr(&args[0])?;
                            let arg = self.insert_cast_if_needed(arg, ValType::I32);
                            return Ok(Some(CHIRExpr::new(
                                CHIRExprKind::Call { func_idx, args: vec![obj, arg] },
                                Type::Int64, ValType::I64,
                            )));
                        }
                    }
                    "replace" if args.len() == 2 => {
                        if let Some(&func_idx) = self.func_indices.get("__str_replace") {
                            let obj = self.lower_expr(object)?;
                            let obj = self.insert_cast_if_needed(obj, ValType::I32);
                            let old = self.lower_expr(&args[0])?;
                            let old = self.insert_cast_if_needed(old, ValType::I32);
                            let new = self.lower_expr(&args[1])?;
                            let new = self.insert_cast_if_needed(new, ValType::I32);
                            return Ok(Some(CHIRExpr::new(
                                CHIRExprKind::Call { func_idx, args: vec![obj, old, new] },
                                Type::String, ValType::I32,
                            )));
                        }
                    }
                    "isBlank" => {
                        // isBlank = trim().size == 0
                        if let (Some(&trim_idx), Some(&_)) = (
                            self.func_indices.get("__str_trim"),
                            self.func_indices.get("__str_trim"),
                        ) {
                            let obj = self.lower_expr(object)?;
                            let obj = self.insert_cast_if_needed(obj, ValType::I32);
                            let trimmed = CHIRExpr::new(
                                CHIRExprKind::Call { func_idx: trim_idx, args: vec![obj] },
                                Type::String, ValType::I32,
                            );
                            return Ok(Some(CHIRExpr::new(
                                CHIRExprKind::BuiltinStringIsEmpty { val: Box::new(trimmed) },
                                Type::Bool, ValType::I32,
                            )));
                        }
                    }
                    _ => {}
                }
            }
            // ArrayList methods (type Array for ArrayList): append, get, set, remove, size
            // Array/Struct methods: isEmpty, clone, slice
            Type::Array(_) | Type::Struct(_, _) => {
                match method {
                    "append" if args.len() == 1 => {
                        if let Some(&func_idx) = self.func_indices.get("__arraylist_append") {
                            let obj = self.lower_expr(object)?;
                            let obj = self.insert_cast_if_needed(obj, ValType::I32);
                            let arg = self.lower_expr(&args[0])?;
                            let arg = self.insert_cast_if_needed(arg, ValType::I64);
                            return Ok(Some(CHIRExpr::new(
                                CHIRExprKind::Call { func_idx, args: vec![obj, arg] },
                                Type::Unit, ValType::I32,
                            )));
                        }
                    }
                    "get" if args.len() == 1 => {
                        if let Some(&func_idx) = self.func_indices.get("__arraylist_get") {
                            let obj = self.lower_expr(object)?;
                            let obj = self.insert_cast_if_needed(obj, ValType::I32);
                            let idx = self.lower_expr(&args[0])?;
                            let idx = self.insert_cast_if_needed(idx, ValType::I64);
                            return Ok(Some(CHIRExpr::new(
                                CHIRExprKind::Call { func_idx, args: vec![obj, idx] },
                                Type::Int64, ValType::I64,
                            )));
                        }
                    }
                    "set" if args.len() == 2 => {
                        if let Some(&func_idx) = self.func_indices.get("__arraylist_set") {
                            let obj = self.lower_expr(object)?;
                            let obj = self.insert_cast_if_needed(obj, ValType::I32);
                            let idx = self.lower_expr(&args[0])?;
                            let idx = self.insert_cast_if_needed(idx, ValType::I64);
                            let val = self.lower_expr(&args[1])?;
                            let val = self.insert_cast_if_needed(val, ValType::I64);
                            return Ok(Some(CHIRExpr::new(
                                CHIRExprKind::Call { func_idx, args: vec![obj, idx, val] },
                                Type::Unit, ValType::I32,
                            )));
                        }
                    }
                    "remove" if args.len() == 1 => {
                        if let Some(&func_idx) = self.func_indices.get("__arraylist_remove") {
                            let obj = self.lower_expr(object)?;
                            let obj = self.insert_cast_if_needed(obj, ValType::I32);
                            let idx = self.lower_expr(&args[0])?;
                            let idx = self.insert_cast_if_needed(idx, ValType::I64);
                            return Ok(Some(CHIRExpr::new(
                                CHIRExprKind::Call { func_idx, args: vec![obj, idx] },
                                Type::Int64, ValType::I64,
                            )));
                        }
                    }
                    "size" if args.is_empty() => {
                        if let Some(&func_idx) = self.func_indices.get("__arraylist_size") {
                            let obj = self.lower_expr(object)?;
                            let obj = self.insert_cast_if_needed(obj, ValType::I32);
                            return Ok(Some(CHIRExpr::new(
                                CHIRExprKind::Call { func_idx, args: vec![obj] },
                                Type::Int64, ValType::I64,
                            )));
                        }
                    }
                    "isEmpty" if args.is_empty() => {
                        let obj = self.lower_expr(object)?;
                        let len_load = CHIRExpr::new(
                            CHIRExprKind::Load { ptr: Box::new(obj), offset: 0, align: 2 },
                            Type::Int32, ValType::I32,
                        );
                        return Ok(Some(CHIRExpr::new(
                            CHIRExprKind::Binary {
                                op: crate::ast::BinOp::Eq,
                                left: Box::new(len_load),
                                right: Box::new(CHIRExpr::new(CHIRExprKind::Integer(0), Type::Int32, ValType::I32)),
                            },
                            Type::Bool, ValType::I32,
                        )));
                    }
                    "clone" if args.is_empty() => {
                        let alloc_idx = self.func_indices.get("__alloc").copied().unwrap_or(0);
                        let obj = self.lower_expr(object)?;
                        let src_local = self.alloc_local_typed("__clone_src".into(), ValType::I32);
                        let dst_local = self.alloc_local_typed("__clone_dst".into(), ValType::I32);
                        let len_local = self.alloc_local_typed("__clone_len".into(), ValType::I32);
                        let src_get = || CHIRExpr::new(CHIRExprKind::Local(src_local), Type::Int32, ValType::I32);
                        let dst_get = || CHIRExpr::new(CHIRExprKind::Local(dst_local), Type::Int32, ValType::I32);
                        let len_get = || CHIRExpr::new(CHIRExprKind::Local(len_local), Type::Int32, ValType::I32);
                        let len_load = CHIRExpr::new(
                            CHIRExprKind::Load { ptr: Box::new(src_get()), offset: 0, align: 2 },
                            Type::Int32, ValType::I32,
                        );
                        // total_bytes = 4 + len * 8
                        let total = CHIRExpr::new(
                            CHIRExprKind::Binary {
                                op: crate::ast::BinOp::Add,
                                left: Box::new(CHIRExpr::new(CHIRExprKind::Integer(4), Type::Int32, ValType::I32)),
                                right: Box::new(CHIRExpr::new(
                                    CHIRExprKind::Binary {
                                        op: crate::ast::BinOp::Mul,
                                        left: Box::new(len_get()),
                                        right: Box::new(CHIRExpr::new(CHIRExprKind::Integer(8), Type::Int32, ValType::I32)),
                                    },
                                    Type::Int32, ValType::I32,
                                )),
                            },
                            Type::Int32, ValType::I32,
                        );
                        let alloc_call = CHIRExpr::new(
                            CHIRExprKind::Call { func_idx: alloc_idx, args: vec![total] },
                            Type::Int32, ValType::I32,
                        );
                        // memory.copy: copy total bytes from src to dst
                        // Emit as a loop: for i in 0..total_bytes { dst[i] = src[i] }
                        // Simplified: store len then copy elements in a loop
                        let stmts = vec![
                            crate::chir::CHIRStmt::Let { local_idx: src_local, value: obj },
                            crate::chir::CHIRStmt::Let { local_idx: len_local, value: len_load },
                            crate::chir::CHIRStmt::Let { local_idx: dst_local, value: alloc_call },
                            // store len
                            crate::chir::CHIRStmt::Expr(CHIRExpr::new(
                                CHIRExprKind::Store { ptr: Box::new(dst_get()), value: Box::new(len_get()), offset: 0, align: 2 },
                                Type::Unit, ValType::I32,
                            )),
                        ];
                        // copy elements loop
                        let i_local = self.alloc_local_typed("__clone_i".into(), ValType::I32);
                        let i_get = || CHIRExpr::new(CHIRExprKind::Local(i_local), Type::Int32, ValType::I32);
                        let cond = CHIRExpr::new(
                            CHIRExprKind::Binary { op: crate::ast::BinOp::Lt, left: Box::new(i_get()), right: Box::new(len_get()) },
                            Type::Bool, ValType::I32,
                        );
                        let src_elem = CHIRExpr::new(
                            CHIRExprKind::Load {
                                ptr: Box::new(CHIRExpr::new(
                                    CHIRExprKind::Binary { op: crate::ast::BinOp::Add, left: Box::new(src_get()),
                                        right: Box::new(CHIRExpr::new(CHIRExprKind::Binary { op: crate::ast::BinOp::Add,
                                            left: Box::new(CHIRExpr::new(CHIRExprKind::Integer(4), Type::Int32, ValType::I32)),
                                            right: Box::new(CHIRExpr::new(CHIRExprKind::Binary { op: crate::ast::BinOp::Mul,
                                                left: Box::new(i_get()), right: Box::new(CHIRExpr::new(CHIRExprKind::Integer(8), Type::Int32, ValType::I32))
                                            }, Type::Int32, ValType::I32))
                                        }, Type::Int32, ValType::I32)) },
                                    Type::Int32, ValType::I32,
                                )),
                                offset: 0, align: 3,
                            },
                            Type::Int64, ValType::I64,
                        );
                        let dst_elem_ptr = CHIRExpr::new(
                            CHIRExprKind::Binary { op: crate::ast::BinOp::Add, left: Box::new(dst_get()),
                                right: Box::new(CHIRExpr::new(CHIRExprKind::Binary { op: crate::ast::BinOp::Add,
                                    left: Box::new(CHIRExpr::new(CHIRExprKind::Integer(4), Type::Int32, ValType::I32)),
                                    right: Box::new(CHIRExpr::new(CHIRExprKind::Binary { op: crate::ast::BinOp::Mul,
                                        left: Box::new(i_get()), right: Box::new(CHIRExpr::new(CHIRExprKind::Integer(8), Type::Int32, ValType::I32))
                                    }, Type::Int32, ValType::I32))
                                }, Type::Int32, ValType::I32)) },
                            Type::Int32, ValType::I32,
                        );
                        let loop_body = crate::chir::CHIRBlock {
                            stmts: vec![
                                crate::chir::CHIRStmt::Expr(CHIRExpr::new(
                                    CHIRExprKind::Store { ptr: Box::new(dst_elem_ptr), value: Box::new(src_elem), offset: 0, align: 3 },
                                    Type::Unit, ValType::I32,
                                )),
                                crate::chir::CHIRStmt::Assign {
                                    target: crate::chir::CHIRLValue::Local(i_local),
                                    value: CHIRExpr::new(CHIRExprKind::Binary { op: crate::ast::BinOp::Add,
                                        left: Box::new(i_get()), right: Box::new(CHIRExpr::new(CHIRExprKind::Integer(1), Type::Int32, ValType::I32))
                                    }, Type::Int32, ValType::I32),
                                },
                            ],
                            result: None,
                        };
                        let mut all_stmts = stmts;
                        all_stmts.push(crate::chir::CHIRStmt::Let { local_idx: i_local, value: CHIRExpr::new(CHIRExprKind::Integer(0), Type::Int32, ValType::I32) });
                        all_stmts.push(crate::chir::CHIRStmt::While { cond, body: loop_body });
                        return Ok(Some(CHIRExpr::new(
                            CHIRExprKind::Block(crate::chir::CHIRBlock { stmts: all_stmts, result: Some(Box::new(dst_get())) }),
                            Type::Int32, ValType::I32,
                        )));
                    }
                    "slice" if args.len() == 2 => {
                        let alloc_idx = self.func_indices.get("__alloc").copied().unwrap_or(0);
                        let obj = self.lower_expr(object)?;
                        let start = self.lower_expr(&args[0])?;
                        let start = self.insert_cast_if_needed(start, ValType::I32);
                        let end = self.lower_expr(&args[1])?;
                        let end = self.insert_cast_if_needed(end, ValType::I32);
                        let src_local = self.alloc_local_typed("__slice_src".into(), ValType::I32);
                        let start_local = self.alloc_local_typed("__slice_start".into(), ValType::I32);
                        let end_local = self.alloc_local_typed("__slice_end".into(), ValType::I32);
                        let new_len_local = self.alloc_local_typed("__slice_len".into(), ValType::I32);
                        let dst_local = self.alloc_local_typed("__slice_dst".into(), ValType::I32);
                        let src_get = || CHIRExpr::new(CHIRExprKind::Local(src_local), Type::Int32, ValType::I32);
                        let start_get = || CHIRExpr::new(CHIRExprKind::Local(start_local), Type::Int32, ValType::I32);
                        let end_get = || CHIRExpr::new(CHIRExprKind::Local(end_local), Type::Int32, ValType::I32);
                        let new_len_get = || CHIRExpr::new(CHIRExprKind::Local(new_len_local), Type::Int32, ValType::I32);
                        let dst_get = || CHIRExpr::new(CHIRExprKind::Local(dst_local), Type::Int32, ValType::I32);
                        let new_len_val = CHIRExpr::new(
                            CHIRExprKind::Binary { op: crate::ast::BinOp::Sub, left: Box::new(end_get()), right: Box::new(start_get()) },
                            Type::Int32, ValType::I32,
                        );
                        let total = CHIRExpr::new(
                            CHIRExprKind::Binary { op: crate::ast::BinOp::Add,
                                left: Box::new(CHIRExpr::new(CHIRExprKind::Integer(4), Type::Int32, ValType::I32)),
                                right: Box::new(CHIRExpr::new(CHIRExprKind::Binary { op: crate::ast::BinOp::Mul,
                                    left: Box::new(new_len_get()), right: Box::new(CHIRExpr::new(CHIRExprKind::Integer(8), Type::Int32, ValType::I32))
                                }, Type::Int32, ValType::I32))
                            },
                            Type::Int32, ValType::I32,
                        );
                        let alloc_call = CHIRExpr::new(
                            CHIRExprKind::Call { func_idx: alloc_idx, args: vec![total] },
                            Type::Int32, ValType::I32,
                        );
                        let mut stmts = vec![
                            crate::chir::CHIRStmt::Let { local_idx: src_local, value: obj },
                            crate::chir::CHIRStmt::Let { local_idx: start_local, value: start },
                            crate::chir::CHIRStmt::Let { local_idx: end_local, value: end },
                            crate::chir::CHIRStmt::Let { local_idx: new_len_local, value: new_len_val },
                            crate::chir::CHIRStmt::Let { local_idx: dst_local, value: alloc_call },
                            crate::chir::CHIRStmt::Expr(CHIRExpr::new(
                                CHIRExprKind::Store { ptr: Box::new(dst_get()), value: Box::new(new_len_get()), offset: 0, align: 2 },
                                Type::Unit, ValType::I32,
                            )),
                        ];
                        // copy loop
                        let i_local = self.alloc_local_typed("__slice_i".into(), ValType::I32);
                        let i_get = || CHIRExpr::new(CHIRExprKind::Local(i_local), Type::Int32, ValType::I32);
                        let cond = CHIRExpr::new(
                            CHIRExprKind::Binary { op: crate::ast::BinOp::Lt, left: Box::new(i_get()), right: Box::new(new_len_get()) },
                            Type::Bool, ValType::I32,
                        );
                        let src_offset = CHIRExpr::new(
                            CHIRExprKind::Binary { op: crate::ast::BinOp::Add, left: Box::new(src_get()),
                                right: Box::new(CHIRExpr::new(CHIRExprKind::Binary { op: crate::ast::BinOp::Add,
                                    left: Box::new(CHIRExpr::new(CHIRExprKind::Integer(4), Type::Int32, ValType::I32)),
                                    right: Box::new(CHIRExpr::new(CHIRExprKind::Binary { op: crate::ast::BinOp::Mul,
                                        left: Box::new(CHIRExpr::new(CHIRExprKind::Binary { op: crate::ast::BinOp::Add, left: Box::new(start_get()), right: Box::new(i_get()) }, Type::Int32, ValType::I32)),
                                        right: Box::new(CHIRExpr::new(CHIRExprKind::Integer(8), Type::Int32, ValType::I32))
                                    }, Type::Int32, ValType::I32))
                                }, Type::Int32, ValType::I32))
                            },
                            Type::Int32, ValType::I32,
                        );
                        let src_val = CHIRExpr::new(
                            CHIRExprKind::Load { ptr: Box::new(src_offset), offset: 0, align: 3 },
                            Type::Int64, ValType::I64,
                        );
                        let dst_offset = CHIRExpr::new(
                            CHIRExprKind::Binary { op: crate::ast::BinOp::Add, left: Box::new(dst_get()),
                                right: Box::new(CHIRExpr::new(CHIRExprKind::Binary { op: crate::ast::BinOp::Add,
                                    left: Box::new(CHIRExpr::new(CHIRExprKind::Integer(4), Type::Int32, ValType::I32)),
                                    right: Box::new(CHIRExpr::new(CHIRExprKind::Binary { op: crate::ast::BinOp::Mul,
                                        left: Box::new(i_get()), right: Box::new(CHIRExpr::new(CHIRExprKind::Integer(8), Type::Int32, ValType::I32))
                                    }, Type::Int32, ValType::I32))
                                }, Type::Int32, ValType::I32))
                            },
                            Type::Int32, ValType::I32,
                        );
                        let loop_body = crate::chir::CHIRBlock {
                            stmts: vec![
                                crate::chir::CHIRStmt::Expr(CHIRExpr::new(
                                    CHIRExprKind::Store { ptr: Box::new(dst_offset), value: Box::new(src_val), offset: 0, align: 3 },
                                    Type::Unit, ValType::I32,
                                )),
                                crate::chir::CHIRStmt::Assign {
                                    target: crate::chir::CHIRLValue::Local(i_local),
                                    value: CHIRExpr::new(CHIRExprKind::Binary { op: crate::ast::BinOp::Add,
                                        left: Box::new(i_get()), right: Box::new(CHIRExpr::new(CHIRExprKind::Integer(1), Type::Int32, ValType::I32))
                                    }, Type::Int32, ValType::I32),
                                },
                            ],
                            result: None,
                        };
                        stmts.push(crate::chir::CHIRStmt::Let { local_idx: i_local, value: CHIRExpr::new(CHIRExprKind::Integer(0), Type::Int32, ValType::I32) });
                        stmts.push(crate::chir::CHIRStmt::While { cond, body: loop_body });
                        return Ok(Some(CHIRExpr::new(
                            CHIRExprKind::Block(crate::chir::CHIRBlock { stmts, result: Some(Box::new(dst_get())) }),
                            Type::Int32, ValType::I32,
                        )));
                    }
                    _ => {
                        // ArrayStack/LinkedList methods (backed by ArrayList runtime)
                        if let Type::Struct(sname, _) = obj_ty {
                            if sname == "ArrayStack" {
                                let obj = self.lower_expr(object)?;
                                let obj = self.insert_cast_if_needed(obj, ValType::I32);
                                match method {
                                    "push" if args.len() == 1 => {
                                        if let Some(&func_idx) = self.func_indices.get("__arraylist_append") {
                                            let val = self.lower_expr(&args[0])?;
                                            let val = self.insert_cast_if_needed(val, ValType::I64);
                                            return Ok(Some(CHIRExpr::new(
                                                CHIRExprKind::Call { func_idx, args: vec![obj, val] },
                                                Type::Unit, ValType::I32,
                                            )));
                                        }
                                    }
                                    "peek" if args.is_empty() => {
                                        if let (Some(&size_idx), Some(&get_idx)) = (
                                            self.func_indices.get("__arraylist_size"),
                                            self.func_indices.get("__arraylist_get"),
                                        ) {
                                            let ptr_local = self.alloc_local_typed(format!("__tmp_{}", self.next_local), ValType::I32);
                                            let stmts = vec![
                                                crate::chir::CHIRStmt::Let { local_idx: ptr_local, value: obj },
                                            ];
                                            let size_expr = CHIRExpr::new(
                                                CHIRExprKind::Call { func_idx: size_idx, args: vec![
                                                    CHIRExpr::new(CHIRExprKind::Local(ptr_local), Type::Int32, ValType::I32),
                                                ] },
                                                Type::Int64, ValType::I64,
                                            );
                                            let idx_expr = CHIRExpr::new(
                                                CHIRExprKind::Binary {
                                                    op: crate::ast::BinOp::Sub,
                                                    left: Box::new(size_expr),
                                                    right: Box::new(CHIRExpr::new(CHIRExprKind::Integer(1), Type::Int64, ValType::I64)),
                                                },
                                                Type::Int64, ValType::I64,
                                            );
                                            let get_expr = CHIRExpr::new(
                                                CHIRExprKind::Call { func_idx: get_idx, args: vec![
                                                    CHIRExpr::new(CHIRExprKind::Local(ptr_local), Type::Int32, ValType::I32),
                                                    idx_expr,
                                                ] },
                                                Type::Int64, ValType::I64,
                                            );
                                            return Ok(Some(CHIRExpr::new(
                                                CHIRExprKind::Block(crate::chir::CHIRBlock { stmts, result: Some(Box::new(get_expr)) }),
                                                Type::Int64, ValType::I64,
                                            )));
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            if sname == "LinkedList" {
                                let obj = self.lower_expr(object)?;
                                let obj = self.insert_cast_if_needed(obj, ValType::I32);
                                match method {
                                    "append" if args.len() == 1 => {
                                        if let Some(&func_idx) = self.func_indices.get("__arraylist_append") {
                                            let val = self.lower_expr(&args[0])?;
                                            let val = self.insert_cast_if_needed(val, ValType::I64);
                                            return Ok(Some(CHIRExpr::new(
                                                CHIRExprKind::Call { func_idx, args: vec![obj, val] },
                                                Type::Unit, ValType::I32,
                                            )));
                                        }
                                    }
                                    "prepend" if args.len() == 1 => {
                                        // prepend(val): shift right by 1, set index 0
                                        if let (Some(&append_idx), Some(&size_idx), Some(&get_idx), Some(&set_idx)) = (
                                            self.func_indices.get("__arraylist_append"),
                                            self.func_indices.get("__arraylist_size"),
                                            self.func_indices.get("__arraylist_get"),
                                            self.func_indices.get("__arraylist_set"),
                                        ) {
                                            let val = self.lower_expr(&args[0])?;
                                            let val = self.insert_cast_if_needed(val, ValType::I64);
                                            let ptr_local = self.alloc_local_typed(format!("__tmp_{}", self.next_local), ValType::I32);
                                            let val_local = self.alloc_local_typed(format!("__tmp_{}", self.next_local), ValType::I64);
                                            let i_local = self.alloc_local_typed(format!("__tmp_{}", self.next_local), ValType::I64);
                                            let mut stmts = Vec::new();
                                            stmts.push(crate::chir::CHIRStmt::Let { local_idx: ptr_local, value: obj });
                                            stmts.push(crate::chir::CHIRStmt::Let { local_idx: val_local, value: val });
                                            // append dummy to grow
                                            stmts.push(crate::chir::CHIRStmt::Expr(CHIRExpr::new(
                                                CHIRExprKind::Call { func_idx: append_idx, args: vec![
                                                    CHIRExpr::new(CHIRExprKind::Local(ptr_local), Type::Int32, ValType::I32),
                                                    CHIRExpr::new(CHIRExprKind::Integer(0), Type::Int64, ValType::I64),
                                                ] },
                                                Type::Unit, ValType::I32,
                                            )));
                                            // i = size - 1
                                            stmts.push(crate::chir::CHIRStmt::Let {
                                                local_idx: i_local,
                                                value: CHIRExpr::new(CHIRExprKind::Binary {
                                                    op: crate::ast::BinOp::Sub,
                                                    left: Box::new(CHIRExpr::new(
                                                        CHIRExprKind::Call { func_idx: size_idx, args: vec![
                                                            CHIRExpr::new(CHIRExprKind::Local(ptr_local), Type::Int32, ValType::I32),
                                                        ] },
                                                        Type::Int64, ValType::I64,
                                                    )),
                                                    right: Box::new(CHIRExpr::new(CHIRExprKind::Integer(1), Type::Int64, ValType::I64)),
                                                }, Type::Int64, ValType::I64),
                                            });
                                            // while i > 0: set(i, get(i-1)); i = i - 1
                                            let cond = CHIRExpr::new(CHIRExprKind::Binary {
                                                op: crate::ast::BinOp::Gt,
                                                left: Box::new(CHIRExpr::new(CHIRExprKind::Local(i_local), Type::Int64, ValType::I64)),
                                                right: Box::new(CHIRExpr::new(CHIRExprKind::Integer(0), Type::Int64, ValType::I64)),
                                            }, Type::Bool, ValType::I32);
                                            let prev_val = CHIRExpr::new(
                                                CHIRExprKind::Call { func_idx: get_idx, args: vec![
                                                    CHIRExpr::new(CHIRExprKind::Local(ptr_local), Type::Int32, ValType::I32),
                                                    CHIRExpr::new(CHIRExprKind::Binary {
                                                        op: crate::ast::BinOp::Sub,
                                                        left: Box::new(CHIRExpr::new(CHIRExprKind::Local(i_local), Type::Int64, ValType::I64)),
                                                        right: Box::new(CHIRExpr::new(CHIRExprKind::Integer(1), Type::Int64, ValType::I64)),
                                                    }, Type::Int64, ValType::I64),
                                                ] },
                                                Type::Int64, ValType::I64,
                                            );
                                            let loop_body = crate::chir::CHIRBlock {
                                                stmts: vec![
                                                    crate::chir::CHIRStmt::Expr(CHIRExpr::new(
                                                        CHIRExprKind::Call { func_idx: set_idx, args: vec![
                                                            CHIRExpr::new(CHIRExprKind::Local(ptr_local), Type::Int32, ValType::I32),
                                                            CHIRExpr::new(CHIRExprKind::Local(i_local), Type::Int64, ValType::I64),
                                                            prev_val,
                                                        ] },
                                                        Type::Unit, ValType::I32,
                                                    )),
                                                    crate::chir::CHIRStmt::Assign {
                                                        target: crate::chir::CHIRLValue::Local(i_local),
                                                        value: CHIRExpr::new(CHIRExprKind::Binary {
                                                            op: crate::ast::BinOp::Sub,
                                                            left: Box::new(CHIRExpr::new(CHIRExprKind::Local(i_local), Type::Int64, ValType::I64)),
                                                            right: Box::new(CHIRExpr::new(CHIRExprKind::Integer(1), Type::Int64, ValType::I64)),
                                                        }, Type::Int64, ValType::I64),
                                                    },
                                                ],
                                                result: None,
                                            };
                                            stmts.push(crate::chir::CHIRStmt::While { cond, body: loop_body });
                                            // set(0, val)
                                            stmts.push(crate::chir::CHIRStmt::Expr(CHIRExpr::new(
                                                CHIRExprKind::Call { func_idx: set_idx, args: vec![
                                                    CHIRExpr::new(CHIRExprKind::Local(ptr_local), Type::Int32, ValType::I32),
                                                    CHIRExpr::new(CHIRExprKind::Integer(0), Type::Int64, ValType::I64),
                                                    CHIRExpr::new(CHIRExprKind::Local(val_local), Type::Int64, ValType::I64),
                                                ] },
                                                Type::Unit, ValType::I32,
                                            )));
                                            return Ok(Some(CHIRExpr::new(
                                                CHIRExprKind::Block(crate::chir::CHIRBlock { stmts, result: None }),
                                                Type::Unit, ValType::I32,
                                            )));
                                        }
                                    }
                                    "get" if args.len() == 1 => {
                                        if let Some(&func_idx) = self.func_indices.get("__arraylist_get") {
                                            let idx = self.lower_expr(&args[0])?;
                                            let idx = self.insert_cast_if_needed(idx, ValType::I64);
                                            return Ok(Some(CHIRExpr::new(
                                                CHIRExprKind::Call { func_idx, args: vec![obj, idx] },
                                                Type::Int64, ValType::I64,
                                            )));
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                        // AtomicInt64/AtomicBool/Mutex/ReentrantMutex methods
                        if let Type::Struct(sname, _) = obj_ty {
                            if sname == "AtomicInt64" {
                                let obj = self.lower_expr(object)?;
                                let obj = self.insert_cast_if_needed(obj, ValType::I32);
                                match method {
                                    "store" if args.len() == 1 => {
                                        let val = self.lower_expr(&args[0])?;
                                        let val = self.insert_cast_if_needed(val, ValType::I64);
                                        return Ok(Some(CHIRExpr::new(
                                            CHIRExprKind::Store { ptr: Box::new(obj), value: Box::new(val), offset: 0, align: 3 },
                                            Type::Unit, ValType::I32,
                                        )));
                                    }
                                    "load" if args.is_empty() => {
                                        return Ok(Some(CHIRExpr::new(
                                            CHIRExprKind::Load { ptr: Box::new(obj), offset: 0, align: 3 },
                                            Type::Int64, ValType::I64,
                                        )));
                                    }
                                    "fetchAdd" if args.len() == 1 => {
                                        let delta = self.lower_expr(&args[0])?;
                                        let delta = self.insert_cast_if_needed(delta, ValType::I64);
                                        let old_local = self.alloc_local_typed(format!("__tmp_{}", self.next_local), ValType::I64);
                                        let ptr_local = self.alloc_local_typed(format!("__tmp_{}", self.next_local), ValType::I32);
                                        let stmts = vec![
                                            crate::chir::CHIRStmt::Let { local_idx: ptr_local, value: obj },
                                            crate::chir::CHIRStmt::Let {
                                                local_idx: old_local,
                                                value: CHIRExpr::new(CHIRExprKind::Load {
                                                    ptr: Box::new(CHIRExpr::new(CHIRExprKind::Local(ptr_local), Type::Int32, ValType::I32)),
                                                    offset: 0, align: 3,
                                                }, Type::Int64, ValType::I64),
                                            },
                                            crate::chir::CHIRStmt::Expr(CHIRExpr::new(CHIRExprKind::Store {
                                                ptr: Box::new(CHIRExpr::new(CHIRExprKind::Local(ptr_local), Type::Int32, ValType::I32)),
                                                value: Box::new(CHIRExpr::new(CHIRExprKind::Binary {
                                                    op: crate::ast::BinOp::Add,
                                                    left: Box::new(CHIRExpr::new(CHIRExprKind::Local(old_local), Type::Int64, ValType::I64)),
                                                    right: Box::new(delta),
                                                }, Type::Int64, ValType::I64)),
                                                offset: 0, align: 3,
                                            }, Type::Unit, ValType::I32)),
                                        ];
                                        return Ok(Some(CHIRExpr::new(
                                            CHIRExprKind::Block(crate::chir::CHIRBlock {
                                                stmts,
                                                result: Some(Box::new(CHIRExpr::new(CHIRExprKind::Local(old_local), Type::Int64, ValType::I64))),
                                            }),
                                            Type::Int64, ValType::I64,
                                        )));
                                    }
                                    "compareAndSwap" if args.len() == 2 => {
                                        let expected = self.lower_expr(&args[0])?;
                                        let expected = self.insert_cast_if_needed(expected, ValType::I64);
                                        let new_val = self.lower_expr(&args[1])?;
                                        let new_val = self.insert_cast_if_needed(new_val, ValType::I64);
                                        let cur_local = self.alloc_local_typed(format!("__tmp_{}", self.next_local), ValType::I64);
                                        let ptr_local = self.alloc_local_typed(format!("__tmp_{}", self.next_local), ValType::I32);
                                        let exp_local = self.alloc_local_typed(format!("__tmp_{}", self.next_local), ValType::I64);
                                        let new_local = self.alloc_local_typed(format!("__tmp_{}", self.next_local), ValType::I64);
                                        let stmts = vec![
                                            crate::chir::CHIRStmt::Let { local_idx: ptr_local, value: obj },
                                            crate::chir::CHIRStmt::Let { local_idx: exp_local, value: expected },
                                            crate::chir::CHIRStmt::Let { local_idx: new_local, value: new_val },
                                            crate::chir::CHIRStmt::Let {
                                                local_idx: cur_local,
                                                value: CHIRExpr::new(CHIRExprKind::Load {
                                                    ptr: Box::new(CHIRExpr::new(CHIRExprKind::Local(ptr_local), Type::Int32, ValType::I32)),
                                                    offset: 0, align: 3,
                                                }, Type::Int64, ValType::I64),
                                            },
                                        ];
                                        let if_body = crate::chir::CHIRBlock {
                                            stmts: vec![crate::chir::CHIRStmt::Expr(CHIRExpr::new(CHIRExprKind::Store {
                                                ptr: Box::new(CHIRExpr::new(CHIRExprKind::Local(ptr_local), Type::Int32, ValType::I32)),
                                                value: Box::new(CHIRExpr::new(CHIRExprKind::Local(new_local), Type::Int64, ValType::I64)),
                                                offset: 0, align: 3,
                                            }, Type::Unit, ValType::I32))],
                                            result: Some(Box::new(CHIRExpr::new(CHIRExprKind::Integer(1), Type::Int64, ValType::I64))),
                                        };
                                        let else_body = crate::chir::CHIRBlock {
                                            stmts: vec![],
                                            result: Some(Box::new(CHIRExpr::new(CHIRExprKind::Integer(0), Type::Int64, ValType::I64))),
                                        };
                                        let cond = CHIRExpr::new(CHIRExprKind::Binary {
                                            op: crate::ast::BinOp::Eq,
                                            left: Box::new(CHIRExpr::new(CHIRExprKind::Local(cur_local), Type::Int64, ValType::I64)),
                                            right: Box::new(CHIRExpr::new(CHIRExprKind::Local(exp_local), Type::Int64, ValType::I64)),
                                        }, Type::Bool, ValType::I32);
                                        return Ok(Some(CHIRExpr::new(
                                            CHIRExprKind::Block(crate::chir::CHIRBlock {
                                                stmts,
                                                result: Some(Box::new(CHIRExpr::new(
                                                    CHIRExprKind::If { cond: Box::new(cond), then_block: if_body, else_block: Some(else_body) },
                                                    Type::Int64, ValType::I64,
                                                ))),
                                            }),
                                            Type::Int64, ValType::I64,
                                        )));
                                    }
                                    _ => {}
                                }
                            }
                            if sname == "AtomicBool" {
                                let obj = self.lower_expr(object)?;
                                let obj = self.insert_cast_if_needed(obj, ValType::I32);
                                match method {
                                    "store" if args.len() == 1 => {
                                        let val = self.lower_expr(&args[0])?;
                                        let val = self.insert_cast_if_needed(val, ValType::I64);
                                        return Ok(Some(CHIRExpr::new(
                                            CHIRExprKind::Store { ptr: Box::new(obj), value: Box::new(val), offset: 0, align: 3 },
                                            Type::Unit, ValType::I32,
                                        )));
                                    }
                                    "load" if args.is_empty() => {
                                        return Ok(Some(CHIRExpr::new(
                                            CHIRExprKind::Load { ptr: Box::new(obj), offset: 0, align: 3 },
                                            Type::Int64, ValType::I64,
                                        )));
                                    }
                                    "compareAndSwap" if args.len() == 2 => {
                                        let expected = self.lower_expr(&args[0])?;
                                        let expected = self.insert_cast_if_needed(expected, ValType::I64);
                                        let new_val = self.lower_expr(&args[1])?;
                                        let new_val = self.insert_cast_if_needed(new_val, ValType::I64);
                                        let cur_local = self.alloc_local_typed(format!("__tmp_{}", self.next_local), ValType::I64);
                                        let ptr_local = self.alloc_local_typed(format!("__tmp_{}", self.next_local), ValType::I32);
                                        let exp_local = self.alloc_local_typed(format!("__tmp_{}", self.next_local), ValType::I64);
                                        let new_local = self.alloc_local_typed(format!("__tmp_{}", self.next_local), ValType::I64);
                                        let stmts = vec![
                                            crate::chir::CHIRStmt::Let { local_idx: ptr_local, value: obj },
                                            crate::chir::CHIRStmt::Let { local_idx: exp_local, value: expected },
                                            crate::chir::CHIRStmt::Let { local_idx: new_local, value: new_val },
                                            crate::chir::CHIRStmt::Let {
                                                local_idx: cur_local,
                                                value: CHIRExpr::new(CHIRExprKind::Load {
                                                    ptr: Box::new(CHIRExpr::new(CHIRExprKind::Local(ptr_local), Type::Int32, ValType::I32)),
                                                    offset: 0, align: 3,
                                                }, Type::Int64, ValType::I64),
                                            },
                                        ];
                                        let if_body = crate::chir::CHIRBlock {
                                            stmts: vec![crate::chir::CHIRStmt::Expr(CHIRExpr::new(CHIRExprKind::Store {
                                                ptr: Box::new(CHIRExpr::new(CHIRExprKind::Local(ptr_local), Type::Int32, ValType::I32)),
                                                value: Box::new(CHIRExpr::new(CHIRExprKind::Local(new_local), Type::Int64, ValType::I64)),
                                                offset: 0, align: 3,
                                            }, Type::Unit, ValType::I32))],
                                            result: Some(Box::new(CHIRExpr::new(CHIRExprKind::Integer(1), Type::Int64, ValType::I64))),
                                        };
                                        let else_body = crate::chir::CHIRBlock {
                                            stmts: vec![],
                                            result: Some(Box::new(CHIRExpr::new(CHIRExprKind::Integer(0), Type::Int64, ValType::I64))),
                                        };
                                        let cond = CHIRExpr::new(CHIRExprKind::Binary {
                                            op: crate::ast::BinOp::Eq,
                                            left: Box::new(CHIRExpr::new(CHIRExprKind::Local(cur_local), Type::Int64, ValType::I64)),
                                            right: Box::new(CHIRExpr::new(CHIRExprKind::Local(exp_local), Type::Int64, ValType::I64)),
                                        }, Type::Bool, ValType::I32);
                                        return Ok(Some(CHIRExpr::new(
                                            CHIRExprKind::Block(crate::chir::CHIRBlock {
                                                stmts,
                                                result: Some(Box::new(CHIRExpr::new(
                                                    CHIRExprKind::If { cond: Box::new(cond), then_block: if_body, else_block: Some(else_body) },
                                                    Type::Int64, ValType::I64,
                                                ))),
                                            }),
                                            Type::Int64, ValType::I64,
                                        )));
                                    }
                                    _ => {}
                                }
                            }
                            if sname == "Mutex" || sname == "ReentrantMutex" {
                                match method {
                                    "lock" | "unlock" => {
                                        return Ok(Some(CHIRExpr::new(CHIRExprKind::Nop, Type::Unit, ValType::I32)));
                                    }
                                    "tryLock" => {
                                        return Ok(Some(CHIRExpr::new(CHIRExprKind::Integer(1), Type::Int64, ValType::I64)));
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
            }
            // HashMap/HashSet methods (type Map for both)
            Type::Map(_, _) => {
                match method {
                    "put" if args.len() == 2 => {
                        if let Some(&func_idx) = self.func_indices.get("__hashmap_put") {
                            let obj = self.lower_expr(object)?;
                            let obj = self.insert_cast_if_needed(obj, ValType::I32);
                            let key = self.lower_expr(&args[0])?;
                            let key = self.insert_cast_if_needed(key, ValType::I64);
                            let val = self.lower_expr(&args[1])?;
                            let val = self.insert_cast_if_needed(val, ValType::I64);
                            return Ok(Some(CHIRExpr::new(
                                CHIRExprKind::Call { func_idx, args: vec![obj, key, val] },
                                Type::Unit, ValType::I32,
                            )));
                        }
                    }
                    "get" if args.len() == 1 => {
                        if let Some(&func_idx) = self.func_indices.get("__hashmap_get") {
                            let obj = self.lower_expr(object)?;
                            let obj = self.insert_cast_if_needed(obj, ValType::I32);
                            let key = self.lower_expr(&args[0])?;
                            let key = self.insert_cast_if_needed(key, ValType::I64);
                            return Ok(Some(CHIRExpr::new(
                                CHIRExprKind::Call { func_idx, args: vec![obj, key] },
                                Type::Int64, ValType::I64,
                            )));
                        }
                    }
                    "containsKey" if args.len() == 1 => {
                        if let Some(&func_idx) = self.func_indices.get("__hashmap_contains") {
                            let obj = self.lower_expr(object)?;
                            let obj = self.insert_cast_if_needed(obj, ValType::I32);
                            let key = self.lower_expr(&args[0])?;
                            let key = self.insert_cast_if_needed(key, ValType::I64);
                            return Ok(Some(CHIRExpr::new(
                                CHIRExprKind::Call { func_idx, args: vec![obj, key] },
                                Type::Bool, ValType::I32,
                            )));
                        }
                    }
                    "remove" if args.len() == 1 => {
                        if let Some(&func_idx) = self.func_indices.get("__hashmap_remove") {
                            let obj = self.lower_expr(object)?;
                            let obj = self.insert_cast_if_needed(obj, ValType::I32);
                            let key = self.lower_expr(&args[0])?;
                            let key = self.insert_cast_if_needed(key, ValType::I64);
                            return Ok(Some(CHIRExpr::new(
                                CHIRExprKind::Call { func_idx, args: vec![obj, key] },
                                Type::Int64, ValType::I64,
                            )));
                        }
                    }
                    "size" if args.is_empty() => {
                        if let Some(&func_idx) = self.func_indices.get("__hashmap_size") {
                            let obj = self.lower_expr(object)?;
                            let obj = self.insert_cast_if_needed(obj, ValType::I32);
                            return Ok(Some(CHIRExpr::new(
                                CHIRExprKind::Call { func_idx, args: vec![obj] },
                                Type::Int64, ValType::I64,
                            )));
                        }
                    }
                    "add" if args.len() == 1 => {
                        if let Some(&func_idx) = self.func_indices.get("__hashset_add") {
                            let obj = self.lower_expr(object)?;
                            let obj = self.insert_cast_if_needed(obj, ValType::I32);
                            let val = self.lower_expr(&args[0])?;
                            let val = self.insert_cast_if_needed(val, ValType::I64);
                            return Ok(Some(CHIRExpr::new(
                                CHIRExprKind::Call { func_idx, args: vec![obj, val] },
                                Type::Unit, ValType::I32,
                            )));
                        }
                    }
                    "contains" if args.len() == 1 => {
                        if let Some(&func_idx) = self.func_indices.get("__hashset_contains") {
                            let obj = self.lower_expr(object)?;
                            let obj = self.insert_cast_if_needed(obj, ValType::I32);
                            let val = self.lower_expr(&args[0])?;
                            let val = self.insert_cast_if_needed(val, ValType::I64);
                            return Ok(Some(CHIRExpr::new(
                                CHIRExprKind::Call { func_idx, args: vec![obj, val] },
                                Type::Bool, ValType::I32,
                            )));
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        Ok(None)
    }

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
    fn lower_match_arm(&mut self, arm: &crate::ast::MatchArm) -> Result<CHIRMatchArm, String> {
        let pattern = self.lower_pattern(&arm.pattern)?;
        let guard = if let Some(guard_expr) = &arm.guard {
            Some(self.lower_expr(guard_expr)?)
        } else {
            None
        };
        let body = self.lower_expr_to_block(&arm.body)?;
        Ok(CHIRMatchArm {
            pattern,
            guard,
            body,
        })
    }

    fn lower_pattern(&mut self, pat: &crate::ast::Pattern) -> Result<CHIRPattern, String> {
        match pat {
            crate::ast::Pattern::Wildcard => Ok(CHIRPattern::Wildcard),
            crate::ast::Pattern::Binding(name) => {
                let idx = self.alloc_local_typed(name.clone(), wasm_encoder::ValType::I64);
                Ok(CHIRPattern::Binding(idx))
            }
            crate::ast::Pattern::Literal(lit) => {
                let chir_lit = match lit {
                    crate::ast::Literal::Integer(n) => crate::chir::CHIRLiteral::Integer(*n),
                    crate::ast::Literal::Bool(b) => crate::chir::CHIRLiteral::Bool(*b),
                    crate::ast::Literal::String(s) => crate::chir::CHIRLiteral::String(s.clone()),
                    _ => crate::chir::CHIRLiteral::Integer(0),
                };
                Ok(CHIRPattern::Literal(chir_lit))
            }
            crate::ast::Pattern::Or(patterns) => {
                // 多模式：展开为多个 Literal arms（简化处理：只取第一个非 Wildcard）
                // 实际需要在 codegen 中处理 Or，这里暂时只取第一个
                if let Some(first) = patterns.first() {
                    self.lower_pattern(first)
                } else {
                    Ok(CHIRPattern::Wildcard)
                }
            }
            crate::ast::Pattern::Range { start, end, inclusive } => {
                let s = match start {
                    crate::ast::Literal::Integer(n) => *n,
                    _ => 0,
                };
                let e = match end {
                    crate::ast::Literal::Integer(n) => *n,
                    _ => 0,
                };
                Ok(CHIRPattern::Range { start: s, end: e, inclusive: *inclusive })
            }
            crate::ast::Pattern::Variant { enum_name, variant_name, payload } => {
                let enum_def = self.enum_defs.iter().find(|e| e.name == *enum_name);
                let discriminant = enum_def
                    .and_then(|e| e.variant_index(variant_name))
                    .unwrap_or(0) as i32;
                let enum_has_payload = enum_def.map_or(false, |e| e.has_payload());
                let payload_binding = if let Some(payload_pat) = payload {
                    if let crate::ast::Pattern::Binding(name) = payload_pat.as_ref() {
                        Some(self.alloc_local_typed(name.clone(), wasm_encoder::ValType::I64))
                    } else {
                        None
                    }
                } else {
                    None
                };
                Ok(CHIRPattern::Variant { discriminant, payload_binding, enum_has_payload })
            }
            crate::ast::Pattern::Struct { name, fields } => {
                let struct_ty = crate::ast::Type::Struct(name.clone(), vec![]);
                let mut chir_fields = Vec::new();
                for (field_name, sub_pat) in fields {
                    let offset = self.get_field_offset(&struct_ty, field_name).unwrap_or(0);
                    let field_ty = self.type_ctx.infer_field_type(&struct_ty, field_name)
                        .unwrap_or(crate::ast::Type::Int64);
                    let field_wasm = match &field_ty {
                        crate::ast::Type::Unit | crate::ast::Type::Nothing => wasm_encoder::ValType::I32,
                        t => t.to_wasm(),
                    };
                    match sub_pat {
                        crate::ast::Pattern::Literal(lit) => {
                            let val = match lit {
                                crate::ast::Literal::Integer(n) => *n,
                                crate::ast::Literal::Bool(b) => if *b { 1 } else { 0 },
                                _ => 0,
                            };
                            chir_fields.push(crate::chir::StructPatternField::Literal {
                                offset, value: val, wasm_ty: field_wasm,
                            });
                        }
                        crate::ast::Pattern::Binding(bind_name) => {
                            let local_idx = self.alloc_local_typed(bind_name.clone(), field_wasm);
                            self.local_ast_types.insert(bind_name.clone(), field_ty.clone());
                            chir_fields.push(crate::chir::StructPatternField::Binding {
                                offset, local_idx, wasm_ty: field_wasm,
                            });
                        }
                        crate::ast::Pattern::Wildcard => {
                            // wildcard field: no check needed, no binding
                        }
                        crate::ast::Pattern::Struct { name: inner_name, fields: inner_fields } => {
                            let inner_ty = crate::ast::Type::Struct(inner_name.clone(), vec![]);
                            for (inner_field_name, inner_sub_pat) in inner_fields {
                                let inner_offset = self.get_field_offset(&inner_ty, inner_field_name).unwrap_or(0);
                                let inner_field_ty = self.type_ctx.infer_field_type(&inner_ty, inner_field_name)
                                    .unwrap_or(crate::ast::Type::Int64);
                                let inner_wasm = match &inner_field_ty {
                                    crate::ast::Type::Unit | crate::ast::Type::Nothing => wasm_encoder::ValType::I32,
                                    t => t.to_wasm(),
                                };
                                match inner_sub_pat {
                                    crate::ast::Pattern::Literal(lit) => {
                                        let val = match lit {
                                            crate::ast::Literal::Integer(n) => *n,
                                            crate::ast::Literal::Bool(b) => if *b { 1 } else { 0 },
                                            _ => 0,
                                        };
                                        chir_fields.push(crate::chir::StructPatternField::NestedLiteral {
                                            outer_offset: offset, inner_offset, value: val, wasm_ty: inner_wasm,
                                        });
                                    }
                                    crate::ast::Pattern::Binding(bind_name) => {
                                        let local_idx = self.alloc_local_typed(bind_name.clone(), inner_wasm);
                                        self.local_ast_types.insert(bind_name.clone(), inner_field_ty.clone());
                                        chir_fields.push(crate::chir::StructPatternField::NestedBinding {
                                            outer_offset: offset, inner_offset, local_idx, wasm_ty: inner_wasm,
                                        });
                                    }
                                    _ => {}
                                }
                            }
                        }
                        _ => {
                            // Other nested patterns: treat as wildcard
                        }
                    }
                }
                Ok(CHIRPattern::Struct { fields: chir_fields })
            }
            _ => Ok(CHIRPattern::Wildcard),
        }
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
        assert!(matches!(chir.kind, CHIRExprKind::ArrayLiteral { .. }));
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
        assert!(matches!(chir.kind, CHIRExprKind::ArrayLiteral { .. }));
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
        assert_eq!(chir.wasm_ty, ValType::I32);
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
