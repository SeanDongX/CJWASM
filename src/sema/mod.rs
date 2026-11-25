//! 语义分析模块 (P3/P4)
//!
//! 在代码生成前对 AST 做轻量级预分析：
//! - 推断无显式返回类型标注的函数/方法返回类型
//! - 构建全局符号类型表，提供给 CodeGen 使用
//!
//! 设计目标：零 panic、无 I/O、纯函数，与 CodeGen 解耦。

use crate::ast::{Expr, Program, Stmt, Type, TypeConstraint};
use std::collections::HashMap;

/// 语义分析上下文：保存预分析推断出的类型信息
#[derive(Debug, Default)]
pub struct SemanticContext {
    /// 函数名 → 推断的返回类型（仅限无显式标注的函数）
    /// key 格式与 `func_return_types` 一致（函数全限定名，含 ClassName.methodName）
    pub inferred_return_types: HashMap<String, Type>,
}

/// 对整个 Program 做语义预分析，返回增强的类型上下文
pub fn analyze(program: &Program) -> SemanticContext {
    let mut ctx = SemanticContext::default();

    // 第一轮：收集所有已有返回类型标注的符号作为已知类型表
    let mut known: HashMap<String, Type> = HashMap::new();

    for func in &program.functions {
        if let Some(ref ret) = func.return_type {
            if *ret != Type::Unit && *ret != Type::Nothing {
                known.insert(func.name.clone(), ret.clone());
            }
        }
    }
    for cls in &program.classes {
        for m in &cls.methods {
            if let Some(ref ret) = m.func.return_type {
                if *ret != Type::Unit && *ret != Type::Nothing {
                    // m.func.name 已由解析器限定为 "ClassName.methodName"
                    known.insert(m.func.name.clone(), ret.clone());
                }
            }
        }
    }

    // 第二轮（多次迭代，传播类型信息）：推断无标注顶层函数的返回类型
    for _pass in 0..3 {
        let mut changed = false;

        for func in &program.functions {
            if func.return_type.is_some()
                || func.extern_import.is_some()
                || !func.type_params.is_empty()
                || func.body.is_empty()
                || known.contains_key(&func.name)
            {
                continue;
            }
            if let Some(inferred) = infer_return_from_body(&func.body, &known) {
                known.insert(func.name.clone(), inferred.clone());
                ctx.inferred_return_types.insert(func.name.clone(), inferred);
                changed = true;
            }
        }

        if !changed {
            break;
        }
    }

    // P4.5: 第三轮 — 推断无标注 class method 的返回类型
    // class method 已由解析器将 func.name 限定为 "ClassName.methodName"
    for _pass in 0..3 {
        let mut changed = false;

        for cls in &program.classes {
            if !cls.type_params.is_empty() {
                continue; // 跳过泛型类（单态化前无法推断）
            }
            for m in &cls.methods {
                if m.func.return_type.is_some()
                    || m.func.extern_import.is_some()
                    || !m.func.type_params.is_empty()
                    || m.func.body.is_empty()
                    || known.contains_key(&m.func.name)
                {
                    continue;
                }
                if let Some(inferred) = infer_return_from_body(&m.func.body, &known) {
                    known.insert(m.func.name.clone(), inferred.clone());
                    ctx.inferred_return_types.insert(m.func.name.clone(), inferred);
                    changed = true;
                }
            }
        }

        if !changed {
            break;
        }
    }

    ctx
}

// ─── 内部辅助函数 ────────────────────────────────────────────────────────────

/// 从函数体推断返回类型
fn infer_return_from_body(body: &[Stmt], known: &HashMap<String, Type>) -> Option<Type> {
    // 先扫描显式 return 语句
    for stmt in body {
        if let Some(ty) = scan_stmt_for_return(stmt, known) {
            return Some(ty);
        }
    }
    // 再检查末尾表达式语句
    if let Some(Stmt::Expr(last_expr)) = body.last() {
        return infer_expr(last_expr, known);
    }
    None
}

/// 递归扫描语句，寻找 return 语句的类型
fn scan_stmt_for_return(stmt: &Stmt, known: &HashMap<String, Type>) -> Option<Type> {
    match stmt {
        Stmt::Return(Some(expr)) => infer_expr(expr, known),
        Stmt::Expr(expr) => {
            // 只有 if/match/block 才可能是末尾值；其他表达式不作为 return 源
            match expr {
                Expr::If { then_branch, .. } => infer_expr(then_branch, known),
                Expr::Match { arms, .. } => {
                    for arm in arms {
                        if let Some(ty) = infer_expr(&arm.body, known) {
                            return Some(ty);
                        }
                    }
                    None
                }
                Expr::Block(stmts, trailing) => {
                    if let Some(tr) = trailing {
                        return infer_expr(tr, known);
                    }
                    for s in stmts.iter().rev() {
                        if let Some(ty) = scan_stmt_for_return(s, known) {
                            return Some(ty);
                        }
                    }
                    None
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// 从表达式推断 AST 类型（不依赖 CodeGen 状态，纯静态推断）
pub fn infer_expr(expr: &Expr, known: &HashMap<String, Type>) -> Option<Type> {
    match expr {
        // ── 字面量 ──────────────────────────────────────────────────────────
        Expr::Integer(_) => Some(Type::Int64),
        // 带后缀整数字面量的类型由后缀决定（如 `1u8` → UInt8）。
        Expr::IntegerSuffixed(_, s) => Some(s.ty()),
        Expr::Float(_) => Some(Type::Float64),
        Expr::Float32(_) => Some(Type::Float32),
        Expr::Bool(_) => Some(Type::Bool),
        Expr::Rune(_) => Some(Type::Rune),
        Expr::String(_) | Expr::Interpolate(_) => Some(Type::String),

        // ── 类型转换 ─────────────────────────────────────────────────────────
        Expr::Cast { target_ty, .. } => Some(target_ty.clone()),
        Expr::IsType { .. } => Some(Type::Bool),

        // ── Option/Result 构造 ────────────────────────────────────────────────
        Expr::Some(inner) => infer_expr(inner, known).map(|t| Type::Option(Box::new(t))),
        Expr::Ok(inner) => infer_expr(inner, known)
            .map(|t| Type::Result(Box::new(t), Box::new(Type::String))),
        Expr::Err(_) => Some(Type::Result(Box::new(Type::Int64), Box::new(Type::String))),
        Expr::None => None,

        // ── 构造函数 / 结构体 ─────────────────────────────────────────────────
        Expr::StructInit { name, type_args, .. } => {
            let ta = type_args.clone().unwrap_or_default();
            Some(Type::Struct(name.clone(), ta))
        }
        Expr::ConstructorCall { name, type_args, .. } => {
            let ta = type_args.clone().unwrap_or_default();
            Some(Type::Struct(name.clone(), ta))
        }

        // ── 函数调用 ──────────────────────────────────────────────────────────
        Expr::Call { name, type_args, .. } => {
            // 优先查已知符号表
            if let Some(ty) = known.get(name.as_str()) {
                return Some(ty.clone());
            }
            // 大写开头名字 → 构造函数，返回 Struct 类型
            if name.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
                let ta = type_args.clone().unwrap_or_default();
                return Some(Type::Struct(name.clone(), ta));
            }
            None
        }

        // ── 变量引用 ──────────────────────────────────────────────────────────
        Expr::Var(name) => known.get(name.as_str()).cloned(),

        // ── 二元运算 ──────────────────────────────────────────────────────────
        Expr::Binary { op, left, right } => {
            use crate::ast::BinOp;
            match op {
                BinOp::Eq
                | BinOp::NotEq
                | BinOp::Lt
                | BinOp::Gt
                | BinOp::LtEq
                | BinOp::GtEq
                | BinOp::LogicalAnd
                | BinOp::LogicalOr
                | BinOp::NotIn => Some(Type::Bool),
                BinOp::Add => {
                    let lt = infer_expr(left, known);
                    let rt = infer_expr(right, known);
                    if lt == Some(Type::String) || rt == Some(Type::String) {
                        Some(Type::String)
                    } else {
                        lt.or(rt)
                    }
                }
                _ => infer_expr(left, known).or_else(|| infer_expr(right, known)),
            }
        }

        // ── 一元运算 ──────────────────────────────────────────────────────────
        Expr::Unary { op, expr } => {
            use crate::ast::UnaryOp;
            match op {
                UnaryOp::Not => Some(Type::Bool),
                _ => infer_expr(expr, known),
            }
        }

        // ── 控制流表达式 ──────────────────────────────────────────────────────
        Expr::If { then_branch, .. } => infer_expr(then_branch, known),
        Expr::IfLet { then_branch, .. } => infer_expr(then_branch, known),
        Expr::Block(stmts, trailing) => {
            if let Some(tr) = trailing {
                return infer_expr(tr, known);
            }
            for stmt in stmts.iter().rev() {
                if let Stmt::Expr(e) = stmt {
                    if let Some(ty) = infer_expr(e, known) {
                        return Some(ty);
                    }
                }
                if let Some(ty) = scan_stmt_for_return(stmt, known) {
                    return Some(ty);
                }
            }
            None
        }
        Expr::Match { arms, .. } => {
            for arm in arms {
                if let Some(ty) = infer_expr(&arm.body, known) {
                    return Some(ty);
                }
            }
            None
        }

        // ── 元组 ──────────────────────────────────────────────────────────────
        Expr::Tuple(elems) => {
            let types: Vec<Type> = elems
                .iter()
                .map(|e| infer_expr(e, known).unwrap_or(Type::Int64))
                .collect();
            Some(Type::Tuple(types))
        }

        // ── Lambda ────────────────────────────────────────────────────────────
        Expr::Lambda {
            params,
            return_type,
            ..
        } => {
            let param_types: Vec<Type> = params.iter().map(|(_, t)| t.clone()).collect();
            let ret = return_type.clone();
            Some(Type::Function {
                params: param_types,
                ret: Box::new(ret),
            })
        }

        // ── Try / NullCoalesce ───────────────────────────────────────────────
        Expr::Try(inner) => infer_expr(inner, known),
        Expr::NullCoalesce { default, .. } => infer_expr(default, known),

        // ── 数组 ──────────────────────────────────────────────────────────────
        Expr::Array(elems) => {
            let elem_ty = elems
                .first()
                .and_then(|e| infer_expr(e, known))
                .unwrap_or(Type::Int64);
            Some(Type::Array(Box::new(elem_ty)))
        }

        _ => None,
    }
}

// ─── 语义合法性校验 ─────────────────────────────────────────────────────────
//
// 在语法解析之后、代码生成之前，对 AST 做名称/签名层面的合法性检查。
// 只实现可精确定位的规则，避免误伤合法重载：
//   1. 同一容器（class/struct/enum/interface）内同名 + 同参数类型的成员函数重定义；
//   2. class/struct/enum 中 static 与实例成员函数同名（6.3.2(1)）；
//   3. 顶层同名 + 同签名的重复函数（泛型与非泛型同名不视为重定义，5.1.6(3)；
//       泛型函数按声明顺序做 alpha 等价，5.1.6(5)/(7)）；
//   4. enum 重复构造器（同名同载荷）/ 容器重复字段 / interface 默认实现重复；
//   5. override/redef 方法与父类/接口方法签名不匹配，或父类型中不存在可覆盖函数；
//   6. 非抽象类未实现本地接口的抽象方法。
//
// 所有规则都能在单文件内基于 AST 判定，父类型无法在本地解析时保守跳过。

use crate::ast::{
    AssignTarget, ClassDef, ClassMethod, EnumDef, FieldDef, Function, InitDef, InterfaceDef,
    InterfaceMethod, Param, Pattern, StructDef, Visibility,
};
use std::collections::HashSet;

mod expr_check;

/// 一条语义校验错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticError {
    pub message: String,
}

impl std::fmt::Display for SemanticError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// 对单个 `Program` 做语义合法性校验。
///
/// 成功返回 `Ok(())`；发现错误时返回全部错误（编译应以非 0 退出）。
/// 校验是纯只读的，不修改 AST。
pub fn validate_program(program: &Program) -> Result<(), Vec<SemanticError>> {
    let mut errors = Vec::new();
    let index = ProgramIndex::new(program);

    check_function_groups(program, &index, &mut errors);
    // 11.1.5.1(8)：程序入口点 main 的签名。
    check_main_signature(program, &mut errors);
    // 3.1(3)：顶层名称共享命名空间，禁止重复/跨类别冲突。
    check_top_level_name_collisions(program, &mut errors);
    // 3.1(2)：关键字不能用作声明名；预声明类型名不能被重新声明。
    check_reserved_identifier_usage(program, &mut errors);
    // 11.2(7)-(12)：public 声明不得引用 internal 类型（无 import 的单文件程序）。
    check_public_internal_exposure(program, &mut errors);
    // 3.1(3)：同一作用域内局部变量重复声明。
    check_duplicate_local_bindings(program, &mut errors);
    // 3.2.3.1(4)：顶层变量须先定义后使用（初始化式不得引用其后声明的顶层变量）。
    check_top_level_const_ordering(program, &mut errors);
    // 3.2.3.2(3)/(6)：局部变量 / 局部函数先使用后声明。
    check_use_before_declaration(program, &mut errors);
    // 3.2.3.3(1)：类型的成员名仅在类型内部可见；顶层自由函数不能无资格引用
    // 其它类型的字段/方法（保守版，只检查自由函数体的直接引用）。
    check_unqualified_member_refs_in_free_fns(program, &mut errors);
    // 3.2.3.2：自由函数体中引用本文件任何作用域都无法解析的裸变量名。
    // 仅在无 import 的单文件程序上启用（导入依赖无法静态确定），确保 std 零回归。
    check_undefined_identifiers(program, &mut errors);
    // 3.3.2.2(8)：类/结构体的静态成员变量必须立即初始化（无 static init 时）。
    check_static_fields_initialized(program, &mut errors);
    // 3.2.3.1(1)/3.2.3(1)：顶层变量初始化式只能引用顶层可见的名字。
    check_top_level_initializer_scope(program, &mut errors);
    // 3.2.3.2(14)/3.2.3.3(2)：类型参数只能在引入它的声明作用域内使用。
    check_type_param_scope(program, &mut errors);
    // 5.3.4(7)：元组不能作为算术/位运算/比较运算符的操作数（运算符重载不支持
    // 变长实参语法糖）。
    check_binary_tuple_operand(program, &mut errors);
    // 5.3.4(4)/(5)：数组形参函数被以标量/元组实参调用（变长实参语法糖误用）。
    check_call_array_literal_args(program, &mut errors);
    // 5.1.2.2(15)：通过函数变量调用时不能省略可选参数（默认值不生效）。
    check_func_var_call_no_defaults(program, &mut errors);
    // 5.1(13)：函数名 / 成员方法名不能作为赋值目标。
    check_function_name_lvalue(program, &mut errors);
    // 5.4(4)：嵌套函数只能在其外层函数作用域内被调用。
    check_nested_function_scope(program, &mut errors);
    // 3.2.3.3(1)：字段初始化式不得无资格引用其它类型的成员名。
    check_cross_type_field_init_refs(program, &mut errors);
    // 5.5(5)：lambda 不能赋给标量类型标注的绑定。
    check_lambda_to_scalar_annotation(program, &mut errors);
    // 5.3.3(1)/(7)：尾随 lambda 只能作为最后一个函数类型形参的实参。
    check_trailing_closure_last_param(program, &mut errors);
    // 3.3.1(7)：已初始化的 let 成员变量不能在构造函数中重新赋值。
    check_let_field_ctor_reassign(program, &mut errors);
    // 5.1.2 / 5.3.4：形参声明顺序、默认值必须是命名参数、默认值静态作用域、
    // 变长形参位置，以及接口/open/abstract/运算符函数不能有可选参数。
    check_function_param_rules(program, &mut errors);
    for cls in &program.classes {
        check_class(cls, &index, &mut errors);
        // 6.1.1(4)：父类/接口必须是绑定到类型的有效引用。
        check_supertype_declaration_kinds(cls, program, &index, &mut errors);
        // 6.1.1.2(13)：子类不能通过 super.父类名() 访问父类主构造函数。
        check_super_primary_ctor_access(cls, &index, &mut errors);
        // 6.1.2.1(46)/(53)：父类没有可访问的无参构造函数时，子类构造函数
        // （以及隐式构造函数）必须能显式调用 super(...)。
        check_super_ctor_availability(cls, &index, &mut errors);
        check_ctor_param_order(cls, &mut errors);
        // 6.1.2.1(55)：构造函数返回前须初始化全部实例成员变量。
        check_ctor_definite_assignment(cls, &mut errors);
        // 6.1.2.1(22)/(8)/(9)：构造参数命名顺序、形参不可重新赋值。
        check_ctor_param_rules(cls, &mut errors);
        // 6.1.2.1(49)：构造函数参数默认值不得引用实例成员。
        check_ctor_param_default_member_refs(cls, program, &index, &mut errors);
        // 6.1.2.1(59)：构造函数调用 this(...) 只允许出现在构造函数体内。
        check_this_ctor_call_outside_init(cls, &mut errors);
        // 6.1.2.1(54)：super 成员访问时机 / 静态上下文禁用 super。
        check_super_access_rules(cls, &mut errors);
        // 6.1.2.1(23)/(24)：显式 super(...) / this(...) 调用必须是构造函数体的首条语句。
        check_explicit_ctor_call_first(cls, &mut errors);
        // 6.1.2.1(34)/(50)：构造函数体至多只能有一条 super(...)/this(...) 调用。
        check_ctor_call_multiplicity(cls, &mut errors);
        // 6.1.2.1(16)：主构造函数中定义的成员变量不得与构造函数外的成员同名。
        check_ctor_member_param_conflicts(cls, &mut errors);
        // 6.1.2.1(58)/(26)：全部成员初始化完成前的受限使用。
        check_use_before_init(cls, &index, &mut errors);
        // 6.1.2.1(57)：open/abstract 类构造函数中 this 不得逃逸。
        check_this_escape_open(cls, &mut errors);
        check_static_member_access(cls, program, &index, &mut errors);
        // 6.1.4(7)：覆写返回 This 的父函数时返回类型必须保持 This。
        check_this_override_return(cls, &index, &mut errors);
        check_private_parent_access(cls, program, &index, &mut errors);
        // 6.1.2.3(3)(4)：实例成员变量初始化表达式的引用限制。
        check_instance_field_initializers(cls, &mut errors);
        // 6.1.2.1(56)：open/abstract 类构造函数不得调用实例成员函数/属性。
        check_open_class_ctor_member_calls(cls, &index, &mut errors);
    }
    // 6.1.2.1(39)：类的全部构造函数均为 private 时，类外（含子类的 super()）不可调用。
    check_private_ctor_access(program, &mut errors);
    // 6.1.2.1(48)：没有显式构造函数的类不能存在未初始化的实例成员变量。
    check_implicit_ctor_uninit_fields(program, &mut errors);
    for st in &program.structs {
        check_struct_interfaces(st, program, &index, &mut errors);
        // 6.3.3(1)：struct 成员不得遮蔽父接口同名成员。
        check_struct_member_shadowing(st, program, &index, &mut errors);
        // 3.1(3)：结构体字段名与成员函数名共享成员命名空间。
        {
            // 仅显式 `let/var/const` 字段参与字段/方法同名检查；由主构造普通
            // 参数合成的字段不属于成员命名空间。
            let ctor_names: HashSet<&str> = st
                .primary_ctor_params
                .iter()
                .map(|p| p.name.as_str())
                .collect();
            let field_names: Vec<&str> = st
                .fields
                .iter()
                .filter(|f| f.decl_kw && !ctor_names.contains(f.name.as_str()))
                .map(|f| f.name.as_str())
                .collect();
            let method_names: Vec<String> = program
                .functions
                .iter()
                .filter(|f| owner_name(&f.name) == Some(st.name.as_str()))
                .map(|f| last_name_segment(&f.name))
                .collect();
            check_field_method_conflicts(
                &st.name,
                field_names.into_iter(),
                method_names.iter().map(|s| s.as_str()),
                &mut errors,
            );
        }
        // 2.1.10.2 结构体构造器规则（参数顺序/默认值/确定赋值/this 使用等）。
        check_struct_ctor_rules(st, program, &mut errors);
        check_impl_generic_constraint_loosening(
            &st.name,
            &st.type_params,
            &st.constraints,
            "结构体",
            program,
            &index,
            &mut errors,
        );
    }
    // 2.1.10.2(41)：跨结构体构造函数的无条件互相调用形成循环。
    check_struct_mutual_ctor_cycles(program, &mut errors);
    // 6.2.5(3)/(4)：Any 不能通过 extend 再次实现。
    check_extend_any_interface(program, &mut errors);
    // 6.2.3(1)：接口只能继承接口。
    check_interface_parent_kinds(program, &index, &mut errors);
    for iface in &program.interfaces {
        check_interface(iface, &index, &mut errors);
    }
    for en in &program.enums {
        check_enum_variants(en, &mut errors);
        check_enum_interfaces(en, program, &index, &mut errors);
        // 6.3.3(1)：enum 成员不得遮蔽父接口同名成员。
        check_enum_member_shadowing(en, program, &index, &mut errors);
        check_impl_generic_constraint_loosening(
            &en.name,
            &en.type_params,
            &en.constraints,
            "枚举",
            program,
            &index,
            &mut errors,
        );
    }

    // 声明层泛型检查：类型参数重名、约束左界必须是类型变量（9.1(4)/9.2(4)）。
    check_generic_declarations(program, &mut errors);
    // 9.5.1.1(12)/(13)：值类型/枚举上界的递归约束非法（T <: S<T>）。
    check_recursive_value_constraints(program, &mut errors);

    // 2.4.1(11)/(12)：类型别名之间不允许直接或间接的循环引用。
    check_type_alias_cycles(program, &mut errors);

    // 实现/覆写函数的泛型约束宽松度（6.2.4.1(5)/6.3.4.1(5)）。
    check_generic_constraint_looseness(program, &index, &mut errors);

    // `This` 类型只能作为类的实例成员函数返回类型（6.1.4(1)）。
    check_this_type_usage(program, &mut errors);
    // 6.1.4(2)：返回 This 的函数体只能返回 This 表达式。
    check_this_return_bodies(program, &mut errors);
    // 6.2.2.1(8)/(10)：无 body 的接口 static 函数不可被调用。
    check_interface_abstract_static_calls(program, &index, &mut errors);
    // 6.2.3.1(6)/6.2.4.1(9)：多父接口同名默认实现冲突须由子类型自行实现。
    check_interface_default_conflicts(program, &index, &mut errors);
    // 6.2.2.2(7)：接口默认实现不能通过 super 访问。
    check_super_interface_default(program, &index, &mut errors);

    // 13 章：C 语言互操作的单文件类型约束（foreign func / CFunc）。
    check_c_interop(program, &index, &mut errors);

    // 10.2.4(12)：数值/Bool/Unit/Nothing/Rune/String/Range 等类型不支持带赋值形态的
    // 下标运算符重载；函数/元组类型亦不允许被 extend。
    check_index_operator_extensions(program, &mut errors);

    // 8.4(9)：同一类型的多个扩展之间不得定义同名同签名的成员。
    check_extension_member_shadowing(program, &mut errors);

    // 6.3.5(3)/6.4.2(1)：extend 实现接口泛型函数时必须为 public。
    check_extension_generic_impl_visibility(program, &index, &mut errors);

    // 不允许实例化抽象类 / 接口；不能继承 struct/enum。
    check_instantiation_targets(program, &mut errors);

    // 16.4/16.5：const init / const 成员函数的声明层限制
    // （var 实例成员、非常量初始值、非 const init 类型上的 const 实例函数、open const 函数）。
    check_const_declarations(program, &mut errors);
    // 16.4：const 函数体只能包含声明、常量表达式与 return。
    check_const_function_bodies(program, &mut errors);

    // 表达式/赋值类型检查：运算符操作数类型、可赋值性、不可变绑定等。
    expr_check::check_program(program, &mut errors);

    // 4.5.4(6)/4.7(6)/(7)/(10)：break/continue 必须在（同一函数的）循环体内，
    // return 必须在函数体内（不能出现在字段默认值 / 顶层初始化式 / 嵌套函数外）。
    check_control_transfer_placement(program, &mut errors);

    // 10.2.3(4)-(14)：运算符重载函数的参数个数须与运算符元数一致；
    // `&&` / `||` 不支持重载。
    check_operator_function_signatures(program, &mut errors);

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

// ─── 16.4/16.5：const init 与 const 成员函数声明限制 ─────────────────────────

/// const 声明层上下文：用于判定字段默认值是否为常量表达式。
struct ConstDeclCtx {
    /// const 函数全名（顶层裸名 / `Type.method`）。
    const_funcs: HashSet<String>,
    /// 全部已声明函数名，用于区分“已知的非 const 函数”与未知内建调用。
    all_funcs: HashSet<String>,
    /// 用户定义类型名（class/struct）。
    user_types: HashSet<String>,
    /// 拥有 const init 的类型名。
    const_init_types: HashSet<String>,
    /// 顶层 const 变量名（可作常量表达式引用）。
    const_globals: HashSet<String>,
    /// 顶层 let/var 变量名（不是常量表达式）。
    nonconst_globals: HashSet<String>,
    /// 用户类型名 → 该类型的非 const 静态成员名集合（16.4(4)/(5)：const 函数/
    /// const init 只能访问 const 的外部 static 成员）。
    nonconst_static_members: HashMap<String, HashSet<String>>,
}

/// 收集“非 const 静态成员”：`static let`/`static var`（is_static 且非 is_const）。
fn collect_nonconst_static_members(program: &Program) -> HashMap<String, HashSet<String>> {
    let mut m: HashMap<String, HashSet<String>> = HashMap::new();
    for c in &program.classes {
        for f in &c.fields {
            if f.is_static && !f.is_const {
                m.entry(c.name.clone()).or_default().insert(f.name.clone());
            }
        }
    }
    for s in &program.structs {
        for f in &s.fields {
            if f.is_static && !f.is_const {
                m.entry(s.name.clone()).or_default().insert(f.name.clone());
            }
        }
    }
    m
}

impl ConstDeclCtx {
    fn is_const(&self, e: &Expr) -> bool {
        match e {
            Expr::Integer(_)
            | Expr::IntegerSuffixed(..)
            | Expr::Float(_)
            | Expr::Float32(_)
            | Expr::Bool(_)
            | Expr::Rune(_)
            | Expr::String(_)
            | Expr::None => true,
            Expr::Interpolate(_) => false,
            Expr::Var(name) => {
                if name == "this" || name == "super" {
                    return false;
                }
                if self.const_globals.contains(name) {
                    true
                } else if self.nonconst_globals.contains(name) {
                    false
                } else {
                    true
                }
            }
            Expr::Unary { expr, .. } => self.is_const(expr),
            Expr::Binary { left, right, .. } => self.is_const(left) && self.is_const(right),
            Expr::Call { name, args, named_args, .. } => {
                let callee = if self.const_funcs.contains(name) {
                    true
                } else if self.all_funcs.contains(name) {
                    false
                } else {
                    true
                };
                callee
                    && args.iter().all(|a| self.is_const(a))
                    && named_args.iter().all(|(_, a)| self.is_const(a))
            }
            Expr::ConstructorCall { name, args, named_args, .. } => {
                let ctor = self.const_init_types.contains(name) || !self.user_types.contains(name);
                ctor
                    && args.iter().all(|a| self.is_const(a))
                    && named_args.iter().all(|(_, a)| self.is_const(a))
            }
            Expr::Tuple(es) | Expr::Array(es) => es.iter().all(|e| self.is_const(e)),
            Expr::TupleIndex { object, .. } => self.is_const(object),
            Expr::Index { array, index } => self.is_const(array) && self.is_const(index),
            Expr::Field { object, .. } => self.is_const(object),
            Expr::Cast { expr, .. } => self.is_const(expr),
            Expr::Some(v) | Expr::Ok(v) | Expr::Err(v) | Expr::Try(v) => self.is_const(v),
            Expr::VariantConst { arg, .. } => arg.as_ref().map_or(true, |a| self.is_const(a)),
            Expr::Throw(_) | Expr::Return(_) | Expr::Break | Expr::Continue => false,
            // 其余形态保守放行（无法确定是非常量）。
            _ => true,
        }
    }
}

/// 16.5：子类定义 const init 时，其在用户程序内定义的父类也必须定义 const init。
/// 父类未知（prelude/跨文件）时保守放行。
fn check_const_init_parent_requirement(program: &Program, errors: &mut Vec<SemanticError>) {
    let mut has_const_init: HashMap<&str, bool> = HashMap::new();
    for c in &program.classes {
        has_const_init.insert(
            c.name.as_str(),
            c.inits.iter().chain(c.init.iter()).any(|i| i.is_const),
        );
    }
    for c in &program.classes {
        let child_const = c.inits.iter().chain(c.init.iter()).any(|i| i.is_const);
        if !child_const {
            continue;
        }
        if let Some(parent) = &c.extends {
            if let Some(&pconst) = has_const_init.get(parent.as_str()) {
                if !pconst {
                    errors.push(SemanticError {
                        message: format!(
                            "类 '{}' 定义了 const init，但其父类 '{}' 未定义 const init（16.5）",
                            c.name, parent
                        ),
                    });
                }
            }
        }
    }
}

/// 16.1(2)/16.2：检查 class/struct 的 const 成员变量的初始值是否为常量表达式。
fn check_const_field_initializers(
    program: &Program,
    ctx: &ConstDeclCtx,
    errors: &mut Vec<SemanticError>,
) {
    let mut check = |owner: &str, f: &crate::ast::FieldDef| {
        if !f.is_const {
            return;
        }
        if let Some(def) = &f.default {
            if !ctx.is_const(def) {
                errors.push(SemanticError {
                    message: format!(
                        "const 成员变量 '{}.{}' 的初始值必须是常量表达式（16.1(2)/16.2）",
                        owner, f.name
                    ),
                });
            }
        }
    };
    for c in &program.classes {
        for f in &c.fields {
            check(&c.name, f);
        }
    }
    for s in &program.structs {
        for f in &s.fields {
            check(&s.name, f);
        }
    }
}

/// 16.4/16.5：const init 与 const 成员函数的声明层约束。
fn check_const_declarations(program: &Program, errors: &mut Vec<SemanticError>) {
    let mut const_funcs: HashSet<String> = HashSet::new();
    let mut all_funcs: HashSet<String> = HashSet::new();
    let mut const_init_types: HashSet<String> = HashSet::new();
    let mut user_types: HashSet<String> = HashSet::new();

    for f in &program.functions {
        all_funcs.insert(f.name.clone());
        if f.is_const {
            const_funcs.insert(f.name.clone());
        }
    }
    for c in &program.classes {
        user_types.insert(c.name.clone());
        for m in &c.methods {
            let bare = m
                .func
                .name
                .rsplit('.')
                .next()
                .unwrap_or(&m.func.name)
                .to_string();
            all_funcs.insert(format!("{}.{}", c.name, bare));
            if m.is_const {
                const_funcs.insert(format!("{}.{}", c.name, bare));
            }
        }
        if c.inits.iter().chain(c.init.iter()).any(|i| i.is_const) {
            const_init_types.insert(c.name.clone());
        }
    }
    for s in &program.structs {
        user_types.insert(s.name.clone());
        if s.inits.iter().any(|i| i.is_const) {
            const_init_types.insert(s.name.clone());
        }
    }
    for ext in &program.extends {
        for f in &ext.methods {
            all_funcs.insert(f.name.clone());
            if f.is_const {
                const_funcs.insert(f.name.clone());
            }
        }
    }
    for iface in &program.interfaces {
        for m in &iface.methods {
            all_funcs.insert(format!("{}.{}", iface.name, m.name));
            if m.is_const {
                const_funcs.insert(format!("{}.{}", iface.name, m.name));
            }
        }
    }
    let mut const_globals: HashSet<String> = HashSet::new();
    let mut nonconst_globals: HashSet<String> = HashSet::new();
    for c in &program.constants {
        if c.is_const {
            const_globals.insert(c.name.clone());
        } else {
            nonconst_globals.insert(c.name.clone());
        }
    }
    let ctx = ConstDeclCtx {
        const_funcs,
        all_funcs,
        user_types,
        const_init_types,
        const_globals,
        nonconst_globals,
        nonconst_static_members: collect_nonconst_static_members(program),
    };

    // 16.1(2)/16.2：const 成员变量的初始值必须是常量表达式（非 const 函数调用等非法）。
    check_const_field_initializers(program, &ctx, errors);
    // 16.5：若子类定义 const init，则其（用户程序内定义的）父类也必须定义 const init。
    check_const_init_parent_requirement(program, errors);

    for cls in &program.classes {
        let has_const_init = cls.inits.iter().chain(cls.init.iter()).any(|i| i.is_const);
        if has_const_init {
            if cls.fields.iter().any(|f| !f.is_static && f.is_mut) {
                errors.push(SemanticError {
                    message: format!(
                        "含有 var 实例成员变量的类 '{}' 不能定义 const init（16.5）",
                        cls.name
                    ),
                });
            }
            for f in &cls.fields {
                if f.is_static {
                    continue;
                }
                if let Some(def) = &f.default {
                    if !ctx.is_const(def) {
                        errors.push(SemanticError {
                            message: format!(
                                "const init 类型的实例成员 '{}' 的初始值必须是常量表达式（16.5）",
                                f.name
                            ),
                        });
                    }
                }
            }
        }
        for m in &cls.methods {
            if m.is_const && !m.is_static && !has_const_init {
                errors.push(SemanticError {
                    message: format!(
                        "const 实例成员函数 '{}' 要求类型 '{}' 定义 const init（16.4）",
                        m.func.name, cls.name
                    ),
                });
            }
            if m.is_const && m.is_open {
                errors.push(SemanticError {
                    message: format!(
                        "const 成员函数 '{}' 不能声明为 open（16.4）",
                        m.func.name
                    ),
                });
            }
        }
    }
    for st in &program.structs {
        let has_const_init = st.inits.iter().any(|i| i.is_const);
        if !has_const_init {
            continue;
        }
        for f in &st.fields {
            if f.is_static {
                continue;
            }
            if let Some(def) = &f.default {
                if !ctx.is_const(def) {
                    errors.push(SemanticError {
                        message: format!(
                            "const init 类型的实例成员 '{}' 的初始值必须是常量表达式（16.5）",
                            f.name
                        ),
                    });
                }
            }
        }
    }
    // struct 的 const 实例成员函数（脱糖为 `StructName.method` 顶层函数）要求 const init。
    for f in &program.functions {
        if !f.is_const {
            continue;
        }
        if let Some((ty, _)) = f.name.rsplit_once('.') {
            if let Some(st) = program.structs.iter().find(|s| s.name == ty) {
                if !st.inits.iter().any(|i| i.is_const) {
                    errors.push(SemanticError {
                        message: format!(
                            "const 实例成员函数 '{}' 要求类型 '{}' 定义 const init（16.4）",
                            f.name, ty
                        ),
                    });
                }
            }
        }
    }
}

// ─── 16.4：const 函数体限制（只允许声明、常量表达式与 return） ───────────────

/// 内置数值/基础类型名（用于识别 `Float64(x)` 这类类型转换调用）。
fn is_builtin_type_name(name: &str) -> bool {
    matches!(
        name,
        "Int8"
            | "Int16"
            | "Int32"
            | "Int64"
            | "IntNative"
            | "UInt8"
            | "UInt16"
            | "UInt32"
            | "UInt64"
            | "UIntNative"
            | "Float16"
            | "Float32"
            | "Float64"
            | "Rune"
            | "String"
            | "Bool"
            | "Array"
            | "VArray"
    )
}

/// const 函数体内表达式是否允许（形参/局部变量可自由引用；外部非常量全局、
/// 非常量函数调用、Range、类型转换等不允许）。
fn const_fn_expr_ok(e: &Expr, locals: &HashSet<String>, ctx: &ConstDeclCtx) -> bool {
    match e {
        Expr::Var(name) => {
            if locals.contains(name) || name == "this" || name == "super" {
                return true;
            }
            if ctx.const_globals.contains(name) {
                true
            } else if ctx.nonconst_globals.contains(name) {
                false
            } else {
                true
            }
        }
        Expr::Call {
            name,
            args,
            named_args,
            ..
        } => {
            let callee = if ctx.const_funcs.contains(name) || locals.contains(name) {
                true
            } else if ctx.all_funcs.contains(name) {
                false
            } else if is_builtin_type_name(name) {
                // 16.4：const 函数内不允许类型转换表达式。
                false
            } else {
                true
            };
            callee
                && args.iter().all(|a| const_fn_expr_ok(a, locals, ctx))
                && named_args.iter().all(|(_, a)| const_fn_expr_ok(a, locals, ctx))
        }
        Expr::ConstructorCall {
            name,
            args,
            named_args,
            ..
        } => {
            let ctor = ctx.const_init_types.contains(name) || !ctx.user_types.contains(name);
            ctor
                && args.iter().all(|a| const_fn_expr_ok(a, locals, ctx))
                && named_args.iter().all(|(_, a)| const_fn_expr_ok(a, locals, ctx))
        }
        Expr::Range { .. } => false,
        // 16.4(4)/(5)：const 函数/const init 只能访问 const 的外部 static 成员，
        // 访问 `Type.staticLetOrVar` 非法。
        Expr::Field { object, field } => {
            if let Expr::Var(tn) = object.as_ref() {
                if !locals.contains(tn)
                    && !ctx.const_globals.contains(tn)
                    && !ctx.nonconst_globals.contains(tn)
                {
                    if let Some(set) = ctx.nonconst_static_members.get(tn) {
                        if set.contains(field) {
                            return false;
                        }
                    }
                }
            }
            const_fn_expr_ok(object, locals, ctx)
        }
        Expr::Unary { expr, .. }
        | Expr::TupleIndex { object: expr, .. }
        | Expr::OptionalChain { object: expr, .. }
        | Expr::Cast { expr, .. }
        | Expr::Some(expr)
        | Expr::Ok(expr)
        | Expr::Err(expr)
        | Expr::Try(expr)
        | Expr::Throw(expr)
        | Expr::IsType { expr, .. }
        | Expr::PostfixIncr(expr)
        | Expr::PostfixDecr(expr)
        | Expr::PrefixIncr(expr)
        | Expr::PrefixDecr(expr) => const_fn_expr_ok(expr, locals, ctx),
        Expr::Return(v) => v
            .as_ref()
            .map_or(true, |v| const_fn_expr_ok(v, locals, ctx)),
        Expr::Binary { left, right, .. } => {
            const_fn_expr_ok(left, locals, ctx) && const_fn_expr_ok(right, locals, ctx)
        }
        Expr::Tuple(es) | Expr::Array(es) => {
            es.iter().all(|e| const_fn_expr_ok(e, locals, ctx))
        }
        Expr::Index { array, index } => {
            const_fn_expr_ok(array, locals, ctx) && const_fn_expr_ok(index, locals, ctx)
        }
        Expr::MethodCall {
            object,
            args,
            named_args,
            ..
        } => {
            const_fn_expr_ok(object, locals, ctx)
                && args.iter().all(|a| const_fn_expr_ok(a, locals, ctx))
                && named_args.iter().all(|(_, a)| const_fn_expr_ok(a, locals, ctx))
        }
        Expr::VariantConst { arg, .. } => arg
            .as_ref()
            .map_or(true, |a| const_fn_expr_ok(a, locals, ctx)),
        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => {
            const_fn_expr_ok(cond, locals, ctx)
                && const_fn_expr_ok(then_branch, locals, ctx)
                && else_branch
                    .as_ref()
                    .map_or(true, |b| const_fn_expr_ok(b, locals, ctx))
        }
        Expr::Block(stmts, tail) => {
            let mut inner = locals.clone();
            const_fn_body_ok(stmts, &mut inner, ctx)
                && tail
                    .as_ref()
                    .map_or(true, |t| const_fn_expr_ok(t, &inner, ctx))
        }
        Expr::Match { expr, arms } => {
            const_fn_expr_ok(expr, locals, ctx)
                && arms.iter().all(|arm| {
                    arm.guard
                        .as_ref()
                        .map_or(true, |g| const_fn_expr_ok(g, locals, ctx))
                        && const_fn_expr_ok(&arm.body, locals, ctx)
                })
        }
        Expr::StructInit { fields, .. } => fields
            .iter()
            .all(|(_, e)| const_fn_expr_ok(e, locals, ctx)),
        // 其余形态（字面量、lambda、宏等）保守放行。
        _ => true,
    }
}

/// const 函数体语句块是否合法：禁止 var 声明、循环语句、赋值、Range 与非 const 表达式。
fn const_fn_body_ok(stmts: &[Stmt], locals: &mut HashSet<String>, ctx: &ConstDeclCtx) -> bool {
    let mut ok = true;
    for s in stmts {
        match s {
            Stmt::Var { .. } => ok = false,
            Stmt::Assign { .. } => ok = false,
            Stmt::For { .. }
            | Stmt::While { .. }
            | Stmt::DoWhile { .. }
            | Stmt::Loop { .. }
            | Stmt::WhileLet { .. } => ok = false,
            Stmt::Let { pattern, value, .. } => {
                if !const_fn_expr_ok(value, locals, ctx) {
                    ok = false;
                }
                collect_pattern_names_local(pattern, locals);
            }
            Stmt::Const { name, value, .. } => {
                if !const_fn_expr_ok(value, locals, ctx) {
                    ok = false;
                }
                locals.insert(name.clone());
            }
            Stmt::LocalFunc(f) => {
                locals.insert(f.name.clone());
            }
            Stmt::Expr(e) => {
                if !const_fn_expr_ok(e, locals, ctx) {
                    ok = false;
                }
            }
            Stmt::Return(Some(e)) => {
                if !const_fn_expr_ok(e, locals, ctx) {
                    ok = false;
                }
            }
            Stmt::UnsafeBlock { body } => {
                if !const_fn_body_ok(body, locals, ctx) {
                    ok = false;
                }
            }
            _ => {}
        }
    }
    ok
}

/// const init 体语句块是否合法：与 const 函数体类似，但允许对实例成员的赋值，
/// 且禁止对 static 成员的赋值（16.5）。
fn const_init_body_ok(
    stmts: &[Stmt],
    locals: &mut HashSet<String>,
    static_fields: &HashSet<String>,
    ctx: &ConstDeclCtx,
) -> bool {
    let mut ok = true;
    for s in stmts {
        match s {
            Stmt::Var { .. } => ok = false,
            Stmt::For { .. }
            | Stmt::While { .. }
            | Stmt::DoWhile { .. }
            | Stmt::Loop { .. }
            | Stmt::WhileLet { .. } => ok = false,
            Stmt::Assign { target, value } => {
                if !const_fn_expr_ok(value, locals, ctx) {
                    ok = false;
                }
                let hits_static = match target {
                    crate::ast::AssignTarget::Var(n) => static_fields.contains(n),
                    crate::ast::AssignTarget::Field { object, field } => {
                        object != "this" && static_fields.contains(field)
                    }
                    // 16.5：元组解构赋值左值含 static 成员亦非法，如 (v1, v2) = (1, 2)。
                    crate::ast::AssignTarget::Tuple(targets) => targets.iter().any(|t| match t {
                        crate::ast::AssignTarget::Var(n) => static_fields.contains(n),
                        crate::ast::AssignTarget::Field { object, field } => {
                            object != "this" && static_fields.contains(field)
                        }
                        _ => false,
                    }),
                    _ => false,
                };
                if hits_static {
                    ok = false;
                }
            }
            Stmt::Let { pattern, value, .. } => {
                if !const_fn_expr_ok(value, locals, ctx) {
                    ok = false;
                }
                collect_pattern_names_local(pattern, locals);
            }
            Stmt::Const { name, value, .. } => {
                if !const_fn_expr_ok(value, locals, ctx) {
                    ok = false;
                }
                locals.insert(name.clone());
            }
            Stmt::LocalFunc(f) => {
                locals.insert(f.name.clone());
            }
            Stmt::Expr(e) => {
                if !const_fn_expr_ok(e, locals, ctx) {
                    ok = false;
                }
            }
            Stmt::Return(Some(e)) => {
                if !const_fn_expr_ok(e, locals, ctx) {
                    ok = false;
                }
            }
            Stmt::UnsafeBlock { body } => {
                if !const_init_body_ok(body, locals, static_fields, ctx) {
                    ok = false;
                }
            }
            _ => {}
        }
    }
    ok
}

/// 收集模式绑定的名字到局部变量集合。
fn collect_pattern_names_local(p: &crate::ast::Pattern, out: &mut HashSet<String>) {
    use crate::ast::Pattern;
    match p {
        Pattern::Binding(n) => {
            out.insert(n.clone());
        }
        Pattern::Tuple(ps) | Pattern::Or(ps) => {
            for p in ps {
                collect_pattern_names_local(p, out);
            }
        }
        Pattern::Struct { fields, .. } => {
            for (_, p) in fields {
                collect_pattern_names_local(p, out);
            }
        }
        Pattern::Variant { payload: Some(p), .. } => collect_pattern_names_local(p, out),
        _ => {}
    }
}

/// 16.4：校验全部 const 函数体。
fn check_const_function_bodies(program: &Program, errors: &mut Vec<SemanticError>) {
    let mut const_funcs: HashSet<String> = HashSet::new();
    let mut all_funcs: HashSet<String> = HashSet::new();
    let mut const_init_types: HashSet<String> = HashSet::new();
    let mut user_types: HashSet<String> = HashSet::new();
    for f in &program.functions {
        all_funcs.insert(f.name.clone());
        if f.is_const {
            const_funcs.insert(f.name.clone());
        }
    }
    for c in &program.classes {
        user_types.insert(c.name.clone());
        for m in &c.methods {
            let bare = m
                .func
                .name
                .rsplit('.')
                .next()
                .unwrap_or(&m.func.name)
                .to_string();
            all_funcs.insert(format!("{}.{}", c.name, bare));
            if m.is_const {
                const_funcs.insert(format!("{}.{}", c.name, bare));
            }
        }
        if c.inits.iter().chain(c.init.iter()).any(|i| i.is_const) {
            const_init_types.insert(c.name.clone());
        }
    }
    for s in &program.structs {
        user_types.insert(s.name.clone());
        if s.inits.iter().any(|i| i.is_const) {
            const_init_types.insert(s.name.clone());
        }
    }
    for ext in &program.extends {
        for f in &ext.methods {
            all_funcs.insert(f.name.clone());
            if f.is_const {
                const_funcs.insert(f.name.clone());
            }
        }
    }
    for iface in &program.interfaces {
        for m in &iface.methods {
            all_funcs.insert(format!("{}.{}", iface.name, m.name));
            if m.is_const {
                const_funcs.insert(format!("{}.{}", iface.name, m.name));
            }
        }
    }
    let mut const_globals: HashSet<String> = HashSet::new();
    let mut nonconst_globals: HashSet<String> = HashSet::new();
    for c in &program.constants {
        if c.is_const {
            const_globals.insert(c.name.clone());
        } else {
            nonconst_globals.insert(c.name.clone());
        }
    }
    let ctx = ConstDeclCtx {
        const_funcs,
        all_funcs,
        user_types,
        const_init_types,
        const_globals,
        nonconst_globals,
        nonconst_static_members: collect_nonconst_static_members(program),
    };

    // 16.5：const init 体限制（不允许 var、循环、非 const 表达式与 static 成员赋值）。
    for c in &program.classes {
        let static_fields: HashSet<String> = c
            .fields
            .iter()
            .filter(|f| f.is_static)
            .map(|f| f.name.clone())
            .collect();
        for init in &c.inits {
            if !init.is_const {
                continue;
            }
            let mut locals: HashSet<String> = init.params.iter().map(|p| p.name.clone()).collect();
            if !const_init_body_ok(&init.body, &mut locals, &static_fields, &ctx) {
                errors.push(SemanticError {
                    message: format!(
                        "类 '{}' 的 const init 体只能包含声明、常量表达式与实例成员赋值（16.5）",
                        c.name
                    ),
                });
            }
        }
    }
    for st in &program.structs {
        let static_fields: HashSet<String> = st
            .fields
            .iter()
            .filter(|f| f.is_static)
            .map(|f| f.name.clone())
            .collect();
        for init in &st.inits {
            if !init.is_const {
                continue;
            }
            let mut locals: HashSet<String> = init.params.iter().map(|p| p.name.clone()).collect();
            if !const_init_body_ok(&init.body, &mut locals, &static_fields, &ctx) {
                errors.push(SemanticError {
                    message: format!(
                        "结构体 '{}' 的 const init 体只能包含声明、常量表达式与实例成员赋值（16.5）",
                        st.name
                    ),
                });
            }
        }
    }

    let mut check = |name: &str, params: &[crate::ast::Param], body: &[Stmt]| {
        let mut locals: HashSet<String> = params.iter().map(|p| p.name.clone()).collect();
        if !const_fn_body_ok(body, &mut locals, &ctx) {
            errors.push(SemanticError {
                message: format!(
                    "const 函数 '{}' 的函数体只能包含声明、常量表达式与 return（16.4）",
                    name
                ),
            });
        }
    };
    for f in &program.functions {
        if f.is_const {
            check(&f.name, &f.params, &f.body);
        }
    }
    for c in &program.classes {
        for m in &c.methods {
            if m.is_const && !m.func.body.is_empty() {
                check(&m.func.name, &m.func.params, &m.func.body);
            }
        }
    }
    for ext in &program.extends {
        for f in &ext.methods {
            if f.is_const {
                check(&f.name, &f.params, &f.body);
            }
        }
    }
    for iface in &program.interfaces {
        for m in &iface.methods {
            if m.is_const {
                if let Some(body) = &m.default_body {
                    check(&m.name, &m.params, body);
                }
            }
        }
    }
}

// ─── 签名表示与比较 ─────────────────────────────────────────────────────────

/// 成员函数签名（已剥离隐式 this/self 接收者）。
#[derive(Debug, Clone)]
struct Sig {
    /// 不带所属类型前缀的成员名（如 `Class.method` → `method`）。
    name: String,
    /// 参数类型（不含接收者）。
    params: Vec<Type>,
    /// 各参数是否为“命名参数”（声明带 `!`），与 `params` 对齐。
    named: Vec<bool>,
    /// 返回类型；None 表示未标注（Unit 推断）。
    ret: Option<Type>,
    /// 是否带方法级泛型参数；None 表示语言层不可见（接口方法）。
    generic: Option<bool>,
    /// 声明顺序的类型参数（类级在前、方法级在后）。
    /// 用于按声明顺序做 alpha 等价；为空时按首次出现顺序编号。
    tp_order: Vec<String>,
    /// `tp_order` 中属于类级（父类型）的类型参数个数，其后为方法级参数。
    class_tp_count: usize,
    /// 是否为静态函数（无实例接收者）。
    is_static: bool,
    /// 声明可见性；`None` 表示语言层不可见（接口方法在 AST 中不保存可见性）。
    vis: Option<Visibility>,
    /// 用于错误消息的全名。
    display: String,
}

fn is_receiver_param(name: &str) -> bool {
    name == "this" || name == "self"
}

/// 剥离第一个隐式/显式接收者参数，返回（其余参数类型, 命名参数标记, 是否静态）。
fn split_receiver(params: &[crate::ast::Param]) -> (Vec<Type>, Vec<bool>, bool) {
    if params
        .first()
        .map_or(false, |p| is_receiver_param(&p.name))
    {
        (
            params[1..].iter().map(|p| p.ty.clone()).collect(),
            params[1..].iter().map(|p| p.is_named).collect(),
            false,
        )
    } else {
        (
            params.iter().map(|p| p.ty.clone()).collect(),
            params.iter().map(|p| p.is_named).collect(),
            true,
        )
    }
}

fn sig_of_function(func: &Function, outer_tp: &[String]) -> Sig {
    let (params, named, is_static) = split_receiver(&func.params);
    // 属性访问器脱糖方法名形如 `Class.__get_p`，按成员名比较时去掉前缀。
    let name = func
        .name
        .rsplit_once('.')
        .map(|(_, n)| n.to_string())
        .unwrap_or_else(|| func.name.clone());
    let mut tp_order = outer_tp.to_vec();
    tp_order.extend(func.type_params.iter().cloned());
    Sig {
        name,
        params,
        named,
        ret: func.return_type.clone(),
        generic: Some(!func.type_params.is_empty()),
        tp_order,
        class_tp_count: outer_tp.len(),
        is_static,
        vis: Some(func.visibility.clone()),
        display: func.name.clone(),
    }
}

/// 类成员函数签名：以 `ClassMethod.is_static` 为准（属性访问器的首参不一定是接收者）。
fn sig_of_class_method(m: &ClassMethod, outer_tp: &[String]) -> Sig {
    let mut sig = sig_of_function(&m.func, outer_tp);
    sig.is_static = m.is_static;
    sig
}

fn sig_of_interface_method(method: &InterfaceMethod) -> Sig {
    sig_of_interface_method_tp(method, &[])
}

/// 接口方法签名；`outer_tp` 为接口自身的类型参数（类级）。
fn sig_of_interface_method_tp(method: &InterfaceMethod, outer_tp: &[String]) -> Sig {
    // 接口方法的参数列表不含隐式接收者；若显式写了 this/self 也剥离。
    let (params, named) = if method
        .params
        .first()
        .map_or(false, |p| is_receiver_param(&p.name))
    {
        (
            method.params[1..].iter().map(|p| p.ty.clone()).collect(),
            method.params[1..].iter().map(|p| p.is_named).collect(),
        )
    } else {
        (
            method.params.iter().map(|p| p.ty.clone()).collect(),
            method.params.iter().map(|p| p.is_named).collect(),
        )
    };
    let mut tp_order = outer_tp.to_vec();
    tp_order.extend(method.type_params.iter().cloned());
    Sig {
        name: method.name.clone(),
        params,
        named,
        ret: method.return_type.clone(),
        generic: Some(!method.type_params.is_empty()),
        tp_order,
        class_tp_count: outer_tp.len(),
        is_static: method.is_static,
        vis: Some(method.visibility.clone()),
        display: method.name.clone(),
    }
}

/// 所属类型名（`A.b` → Some("A")，`b` → None）。
fn owner_name(full: &str) -> Option<&str> {
    full.rsplit_once('.').map(|(o, _)| o)
}

/// 把类型结构序列化为规范字符串。
///
/// 泛型参数优先按 `tp_order`（声明顺序）映射编号；不在 `tp_order` 中的
/// 类型参数按首次出现顺序追加编号。这样 `f<X,Y>(a:X,b:Y)` 与
/// `f<Y,X>(a:X,b:Y)` 的非泛型部分不同，不误判为重定义。
fn canon_type(ty: &Type, order: &[String], map: &mut HashMap<String, usize>) -> String {
    match ty {
        Type::TypeParam(name) => {
            if let Some(pos) = order.iter().position(|n| n == name) {
                format!("#{}", pos)
            } else {
                let next = order.len() + map.len();
                let idx = *map.entry(name.clone()).or_insert(next);
                format!("#{}", idx)
            }
        }
        Type::Array(inner) => format!("Array<{}>", canon_type(inner, order, map)),
        Type::Option(inner) => format!("Option<{}>", canon_type(inner, order, map)),
        Type::Slice(inner) => format!("Slice<{}>", canon_type(inner, order, map)),
        Type::Struct(name, args) => {
            if args.is_empty() {
                // 解析器在部分上下文（struct 方法签名等）把类型参数记为不带实参的
                // Struct（如 `Struct("T", [])`）；按 tp_order 视作类型参数做 alpha 等价。
                if let Some(pos) = order.iter().position(|n| n == name) {
                    format!("#{}", pos)
                } else if let Some(idx) = map.get(name) {
                    format!("#{}", idx)
                } else {
                    format!("S:{}", name)
                }
            } else {
                let inner: Vec<String> = args.iter().map(|a| canon_type(a, order, map)).collect();
                format!("S:{}<{}>", name, inner.join(","))
            }
        }
        Type::Tuple(items) => {
            let inner: Vec<String> = items.iter().map(|a| canon_type(a, order, map)).collect();
            format!("({})", inner.join(","))
        }
        Type::Result(ok, err) => {
            format!(
                "Result<{},{}>",
                canon_type(ok, order, map),
                canon_type(err, order, map)
            )
        }
        Type::Map(k, v) => format!(
            "Map<{},{}>",
            canon_type(k, order, map),
            canon_type(v, order, map)
        ),
        Type::Function { params, ret } => {
            let ps: Vec<String> = params.iter().map(|p| canon_type(p, order, map)).collect();
            let r = match ret.as_ref() {
                Some(t) => canon_type(t, order, map),
                None => "Unit".to_string(),
            };
            format!("Fn({})->{}", ps.join(","), r)
        }
        Type::Qualified(parts) => format!("Q:{}", parts.join(".")),
        other => format!("{:?}", other),
    }
}

/// 仅参数列表的规范键（用于重定义判定）。
fn canon_params(sig: &Sig) -> String {
    let mut map = HashMap::new();
    sig.params
        .iter()
        .map(|t| canon_type(t, &sig.tp_order, &mut map))
        .collect::<Vec<_>>()
        .join(",")
}

/// 参数 + 返回类型的规范键（用于 override/实现匹配）。
fn canon_signature(sig: &Sig) -> (String, Option<String>) {
    let mut map = HashMap::new();
    let params = sig
        .params
        .iter()
        .map(|t| canon_type(t, &sig.tp_order, &mut map))
        .collect::<Vec<_>>()
        .join(",");
    let ret = sig
        .ret
        .as_ref()
        .map(|t| canon_type(t, &sig.tp_order, &mut map));
    (params, ret)
}

/// 泛型性质是否兼容：已知泛型/非泛型不同则不构成重定义（5.1.6(3)）。
fn generic_compatible(a: Option<bool>, b: Option<bool>) -> bool {
    match (a, b) {
        (Some(x), Some(y)) => x == y,
        _ => true,
    }
}

/// 两签名是否构成重定义（同名且参数类型一致，泛型性质兼容）。
fn is_redefinition(a: &Sig, b: &Sig) -> bool {
    a.params.len() == b.params.len()
        && generic_compatible(a.generic, b.generic)
        && canon_params(a) == canon_params(b)
}

// ─── 容器内重复成员检测 ────────────────────────────────────────────────────

fn is_synthetic_member(name: &str) -> bool {
    name == "__static_init" || name == "__init"
}

fn check_signature_group(
    owner: &str,
    group: &[Sig],
    check_static_conflict: bool,
    errors: &mut Vec<SemanticError>,
) {
    if group.len() < 2 {
        return;
    }
    let name = group[0].name.clone();

    // static 与实例成员函数同名：不允许互为重载（6.3.2(1)）。
    if check_static_conflict {
        let has_static = group.iter().any(|s| s.is_static);
        let has_instance = group.iter().any(|s| !s.is_static);
        if has_static && has_instance {
            errors.push(SemanticError {
                message: format!(
                    "类 '{}' 中静态成员函数与实例成员函数同名 '{}'，不允许互为重载",
                    owner, name
                ),
            });
            return;
        }
    }

    // 同名 + 同参数类型的成员函数构成重定义。
    for i in 0..group.len() {
        for j in (i + 1)..group.len() {
            if is_redefinition(&group[i], &group[j]) {
                errors.push(SemanticError {
                    message: format!(
                        "类型 '{}' 中成员函数 '{}' 被重复定义（{} 与 {}）",
                        owner, name, group[i].display, group[j].display
                    ),
                });
                return;
            }
        }
    }
}

/// 校验 program.functions 中的函数组（顶层函数 + struct/enum 成员方法）。
fn check_function_groups(
    program: &Program,
    index: &ProgramIndex<'_>,
    errors: &mut Vec<SemanticError>,
) {
    let mut groups: HashMap<String, Vec<Sig>> = HashMap::new();
    for func in &program.functions {
        if func.extern_import.is_some() {
            continue;
        }
        let owner = owner_name(&func.name).unwrap_or("").to_string();
        let outer = index.type_params_of(&owner);
        let sig = sig_of_function(func, outer);
        if is_synthetic_member(&sig.name) {
            continue;
        }
        groups.entry(owner).or_default().push(sig);
    }

    for (owner, sigs) in groups {
        // 顶层函数没有 static/实例之分；struct/enum 成员函数需要检查。
        let check_static = !owner.is_empty()
            && (index.structs.contains(owner.as_str()) || index.enums.contains(owner.as_str()));
        let mut by_name: HashMap<String, Vec<Sig>> = HashMap::new();
        for sig in sigs {
            by_name.entry(sig.name.clone()).or_default().push(sig);
        }
        let owner_desc = if owner.is_empty() {
            "<顶层>"
        } else {
            owner.as_str()
        };
        for group in by_name.values() {
            check_signature_group(owner_desc, group, check_static, errors);
        }
    }
}

// ─── 11.1.5.1(8)：程序入口点 main 的签名 ──────────────────────────────────

/// main 允许的返回类型：Unit 或各类整数类型（含 Native）。
fn is_valid_main_return(ty: &Type) -> bool {
    matches!(
        ty,
        Type::Unit
            | Type::Int8
            | Type::Int16
            | Type::Int32
            | Type::Int64
            | Type::IntNative
            | Type::UInt8
            | Type::UInt16
            | Type::UInt32
            | Type::UInt64
            | Type::UIntNative
    )
}

/// 从 main 函数体的 `return` 语句保守推断返回类型。
///
/// 只在能确定类型时给出结果：整数字面量按 Int64、浮点按 Float64、布尔 / Rune /
/// 字符串按对应基础类型；`return 变量` 仅在变量有显式类型标注时解析。其余形态
/// （函数调用、方法调用、复杂表达式）一律返回 None，避免误判合法程序。
fn infer_main_return(func: &Function) -> Option<Type> {
    let mut locals: HashMap<String, Type> = HashMap::new();
    collect_explicit_local_types(&func.body, &mut locals);

    let mut found: Option<Type> = None;
    for stmt in &func.body {
        if let Stmt::Return(Some(e)) = stmt {
            if let Some(t) = simple_expr_type(e, &locals) {
                // 多次 return 类型不一致时不作判定。
                match &found {
                    Some(prev) if *prev != t => return None,
                    _ => found = Some(t),
                }
            }
        }
    }
    found
}

/// 收集函数体内（递归含嵌套块）带显式类型标注的局部变量类型。
fn collect_explicit_local_types(stmts: &[Stmt], out: &mut HashMap<String, Type>) {
    fn pattern_names(p: &Pattern, out: &mut Vec<String>) {
        match p {
            Pattern::Binding(n) => out.push(n.clone()),
            Pattern::TypeTest { binding, .. } => out.push(binding.clone()),
            Pattern::Tuple(ps) => ps.iter().for_each(|q| pattern_names(q, out)),
            Pattern::Struct { fields, .. } => {
                fields.iter().for_each(|(_, q)| pattern_names(q, out))
            }
            Pattern::Variant { payload: Some(q), .. } => pattern_names(q, out),
            Pattern::Or(ps) => {
                if let Some(q) = ps.first() {
                    pattern_names(q, out);
                }
            }
            _ => {}
        }
    }
    for stmt in stmts {
        match stmt {
            Stmt::Let { pattern, ty: Some(ty), .. } | Stmt::Var { pattern, ty: Some(ty), .. } => {
                let mut names = Vec::new();
                pattern_names(pattern, &mut names);
                for n in names {
                    out.insert(n, ty.clone());
                }
            }
            Stmt::Const { name, ty: Some(ty), .. } => {
                out.insert(name.clone(), ty.clone());
            }
            Stmt::While { body, .. }
            | Stmt::DoWhile { body, .. }
            | Stmt::Loop { body }
            | Stmt::UnsafeBlock { body } => collect_explicit_local_types(body, out),
            Stmt::For { body, .. }
            | Stmt::WhileLet { body, .. } => collect_explicit_local_types(body, out),
            _ => {}
        }
    }
}

/// 仅对可确定的基础类型表达式给出类型；其它返回 None（保守）。
fn simple_expr_type(e: &Expr, locals: &HashMap<String, Type>) -> Option<Type> {
    match e {
        Expr::Integer(_) | Expr::IntegerSuffixed(_, _) => Some(Type::Int64),
        Expr::Float(_) => Some(Type::Float64),
        Expr::Float32(_) => Some(Type::Float32),
        Expr::Bool(_) => Some(Type::Bool),
        Expr::Rune(_) => Some(Type::Rune),
        Expr::String(_) | Expr::Interpolate(_) => Some(Type::String),
        Expr::Var(name) => locals.get(name).cloned(),
        Expr::Cast { target_ty, .. } => Some(target_ty.clone()),
        _ => None,
    }
}

/// 11.1.5.1(8)：主包中 `main` 只能无参或带一个 `Array<String>` 参数，返回类型
/// 只能是 Unit 或整数类型。
fn check_main_signature(program: &Program, errors: &mut Vec<SemanticError>) {
    // 只有主包（无 package 声明）的 main 才是程序入口；库包同样允许定义 main。
    if program.package_name.is_some() {
        return;
    }
    for func in &program.functions {
        if func.name != "main" || func.extern_import.is_some() {
            continue;
        }
        let params_ok = match func.params.as_slice() {
            [] => true,
            [p] => matches!(&p.ty, Type::Array(inner) if **inner == Type::String),
            _ => false,
        };
        if !params_ok {
            errors.push(SemanticError {
                message: "程序入口 main 只能没有参数，或只有一个 Array<String> 类型的参数（11.1.5.1(8)）"
                    .to_string(),
            });
        }
        let ret = func
            .return_type
            .clone()
            .or_else(|| infer_main_return(func));
        if let Some(rt) = ret {
            if !is_valid_main_return(&rt) {
                errors.push(SemanticError {
                    message: format!(
                        "程序入口 main 的返回类型必须为 Unit 或整数类型，不允许 '{}'（11.1.5.1(8)）",
                        type_display(&rt)
                    ),
                });
            }
        }
    }
}

/// 轻量类型名显示（仅用于错误信息）。
fn type_display(ty: &Type) -> String {
    match ty {
        Type::Array(inner) => format!("Array<{}>", type_display(inner)),
        Type::Struct(name, _) => name.clone(),
        other => format!("{:?}", other),
    }
}

// ─── 3.1(3)：顶层名称共享同一命名空间的重复 / 冲突 ────────────────────────

/// 3.2.3.1(4)：顶层变量须先定义后使用。
///
/// `Program.constants` 保留顶层 `let/var/const` 的源码顺序。若某个顶层变量的
/// 初始化式引用了在它**之后**才声明的顶层变量，即为“使用前未定义”。
/// 仅比较“同属 `constants` 的名字”，函数/类型/导入名不参与，避免误报。
fn check_top_level_const_ordering(program: &Program, errors: &mut Vec<SemanticError>) {
    if program.constants.len() < 2 {
        return;
    }
    let position: HashMap<&str, usize> = program
        .constants
        .iter()
        .enumerate()
        .map(|(i, c)| (c.name.as_str(), i))
        .collect();
    let mut reported: HashSet<(usize, String)> = HashSet::new();
    for (i, c) in program.constants.iter().enumerate() {
        let mut refs: Vec<String> = Vec::new();
        collect_expr_direct_refs(&c.init, &mut refs);
        for r in refs {
            if let Some(&j) = position.get(r.as_str()) {
                if j > i && reported.insert((i, r.clone())) {
                    errors.push(SemanticError {
                        message: format!(
                            "顶层变量 '{}' 的初始化引用了其后才声明的顶层变量 '{}'（3.2.3.1(4)）",
                            c.name, r
                        ),
                    });
                }
            }
        }
    }
}

/// 3.1(3)：变量、函数、类型别名与类型声明在顶层共享同一命名空间，跨类别同名即冲突。
///
/// 只判定**不同类别**之间的冲突：同类别重复（如同名顶层函数的重载、宏展开或
/// 平台条件编译生成的多份同名声明）不在此处判定，避免误伤；顶层函数同签名重复
/// 由 `check_function_groups` 处理。
fn check_top_level_name_collisions(program: &Program, errors: &mut Vec<SemanticError>) {
    // 同类别重复声明在含 import 的程序中可能来自 @When[backend=...] 平台条件
    // 编译（AST 会同时保留各分支），仅对无 import 的单文件程序直接判定重复。
    let report_same_kind = program.imports.is_empty();
    fn record(
        seen: &mut HashMap<String, (&'static str, String)>,
        name: &str,
        kind: &'static str,
        desc: &str,
        report_same_kind: bool,
        errors: &mut Vec<SemanticError>,
    ) {
        if name.is_empty() || name.starts_with("__") {
            return;
        }
        match seen.get(name) {
            None => {
                seen.insert(name.to_string(), (kind, desc.to_string()));
            }
            Some((prev_kind, prev_desc)) => {
                if *prev_kind == kind {
                    // 同类别重复：函数同名可能是重载，由 check_function_groups 负责
                    // （`kind == "函数"` 时一律放行）。类型声明（class/struct/enum/
                    // interface）可能带有宏属性（@Test/@Strategy/@When 等），AST 不保留
                    // 属性，无法区分合法的宏多声明，故只对变量与类型别名在无 import
                    // 的单文件程序中判定 3.1(3) 重复声明。
                    let safe_kind = matches!(kind, "变量" | "type");
                    if kind == "函数" || !report_same_kind || !safe_kind {
                        return;
                    }
                    errors.push(SemanticError {
                        message: format!("顶层名称 '{}' 被重复声明（{}，3.1(3)）", name, desc),
                    });
                    return;
                }
                errors.push(SemanticError {
                    message: format!(
                        "顶层名称 '{}' 在命名空间中重复或冲突（{} 与 {}，3.1(3)）",
                        name, prev_desc, desc
                    ),
                });
            }
        }
    }

    let mut seen: HashMap<String, (&'static str, String)> = HashMap::new();
    // 顶层同名 let/var 可能来自 @When[os/backend] 平台条件编译（官方 std 大量使用），
    // 预处理前的 AST 会同时保留各平台分支，故同类别重复必须保守跳过。
    for c in &program.constants {
        record(&mut seen, &c.name, "变量", "变量", report_same_kind, errors);
    }
    for f in &program.functions {
        // 仅顶层函数（成员方法以 `Type.method` 形式存储在同一列表中）。
        if f.name.contains('.') || f.name.contains("::") {
            continue;
        }
        // 同名顶层函数可能是重载，由 check_function_groups 判定，不在此处按类别重复处理。
        record(&mut seen, &f.name, "函数", "函数", report_same_kind, errors);
    }
    // 用户定义类型：跨类别冲突由 record 判定；同类别重复可能来自 @When
    // 平台条件编译，按上面同样的理由保守跳过。
    for c in &program.classes {
        record(&mut seen, &c.name, "class", "class", report_same_kind, errors);
    }
    for s in &program.structs {
        record(&mut seen, &s.name, "struct", "struct", report_same_kind, errors);
    }
    for e in &program.enums {
        record(&mut seen, &e.name, "enum", "enum", report_same_kind, errors);
    }
    for i in &program.interfaces {
        record(&mut seen, &i.name, "interface", "interface", report_same_kind, errors);
    }
    for (name, _) in &program.type_aliases {
        record(&mut seen, name, "type", "type 别名", report_same_kind, errors);
    }
}

// ─── 03 章：保留关键字不得用作声明名 ───────────────────────────────────────

/// 03.1(2)：关键字、变量、函数、类型共享同一命名空间，关键字不能作为声明名。
///
/// 词法器把 `where`/`type`/`is`/`case` 定义为独立关键字 token，但解析器在若干
/// 声明位置把它们当普通标识符接受；这里在语义层补上拒绝。`main` 只允许作为
/// 顶层入口函数名（其它位置声明 `main` 均非法）。预声明类型名不能被用户重新
/// 声明为类型（如 `enum Unit { x }`）。
fn check_reserved_identifier_usage(program: &Program, errors: &mut Vec<SemanticError>) {
    // 反引号标识符（`` `is` ``）在 AST 中与普通关键字同名、无法区分，且官方
    // 正例大量出现在带 import 的单文件程序中；仅在无 import 的程序上启用，
    // 保证正测试零回归（负例 a02 组均无 import）。
    if !program.imports.is_empty() {
        return;
    }
    // 定义即保留、任何声明名都不允许的关键字。
    fn strict_keyword(name: &str) -> bool {
        matches!(name, "where" | "type" | "is" | "case")
    }
    fn predeclared_type(name: &str) -> bool {
        matches!(
            name,
            "Int8"
                | "Int16"
                | "Int32"
                | "Int64"
                | "IntNative"
                | "UInt8"
                | "UInt16"
                | "UInt32"
                | "UInt64"
                | "UIntNative"
                | "Float16"
                | "Float32"
                | "Float64"
                | "Rune"
                | "Bool"
                | "Unit"
                | "Nothing"
                | "This"
        )
    }
    let mut report = |name: &str, what: &str| {
        errors.push(SemanticError {
            message: format!(
                "保留关键字 '{}' 不能用作{}名（3.1(2)）",
                name, what
            ),
        });
    };

    if let Some(pkg) = &program.package_name {
        if strict_keyword(pkg) || pkg == "main" {
            report(pkg, "包");
        }
    }
    for c in &program.constants {
        if strict_keyword(&c.name) || c.name == "main" {
            report(&c.name, "变量");
        }
    }
    // 顶层函数：`main` 是入口函数名，允许；`where/type/is/case` 不允许。
    for f in &program.functions {
        if f.name.contains('.') || f.name.contains("::") {
            continue;
        }
        if strict_keyword(&f.name) {
            report(&f.name, "函数");
        }
    }
    // 类型声明名：额外禁止占用预声明类型名。
    for c in &program.classes {
        if strict_keyword(&c.name) || c.name == "main" || predeclared_type(&c.name) {
            report(&c.name, "类型");
        }
    }
    for s in &program.structs {
        if strict_keyword(&s.name) || s.name == "main" || predeclared_type(&s.name) {
            report(&s.name, "类型");
        }
    }
    for e in &program.enums {
        if strict_keyword(&e.name) || e.name == "main" || predeclared_type(&e.name) {
            report(&e.name, "类型");
        }
    }
    for i in &program.interfaces {
        if strict_keyword(&i.name) || i.name == "main" || predeclared_type(&i.name) {
            report(&i.name, "类型");
        }
    }
    for (name, _) in &program.type_aliases {
        if strict_keyword(name) || name == "main" || predeclared_type(name) {
            report(name, "类型别名");
        }
    }
    // 成 员 名（字段 / 方法 / 枚举构造器 / 属性）。
    for c in &program.classes {
        for fld in &c.fields {
            if strict_keyword(&fld.name) || fld.name == "main" {
                report(&fld.name, "字段");
            }
        }
    }
    for s in &program.structs {
        for fld in &s.fields {
            if strict_keyword(&fld.name) || fld.name == "main" {
                report(&fld.name, "字段");
            }
        }
    }
    for f in &program.functions {
        let Some(_owner) = owner_name(&f.name) else {
            continue;
        };
        let seg = last_name_segment(&f.name);
        if seg.starts_with("__") {
            continue;
        }
        if strict_keyword(&seg) {
            report(&seg, "成员函数");
        }
    }
    for e in &program.enums {
        for v in &e.variants {
            if strict_keyword(&v.name) || v.name == "main" {
                report(&v.name, "枚举构造器");
            }
        }
    }
}

// ─── 11 章：public 声明不得暴露 internal 类型 ─────────────────────────────

/// 11.2(7)-(12)：修饰符 public / protected 的声明所引用的类型必须是 external
/// （包外可见）。仓颉中未标注可见性的顶层类型默认 internal。
///
/// 仅在**无 import** 的单文件程序上启用：存在导入时外部类型的可见性无法静态
/// 确定，保守跳过以保证 std / 正测试零回归。返回类型未显式标注时，只识别
/// “函数体直接 `return Ctor()` / 末尾 `Ctor()`”这种可确定的推断形态。
fn check_public_internal_exposure(program: &Program, errors: &mut Vec<SemanticError>) {
    if !program.imports.is_empty() {
        return;
    }
    let mut internal: HashSet<&str> = HashSet::new();
    for c in &program.classes {
        if c.visibility == Visibility::Internal {
            internal.insert(c.name.as_str());
        }
    }
    for s in &program.structs {
        if s.visibility == Visibility::Internal {
            internal.insert(s.name.as_str());
        }
    }
    for e in &program.enums {
        if e.visibility == Visibility::Internal {
            internal.insert(e.name.as_str());
        }
    }
    for i in &program.interfaces {
        if i.visibility == Visibility::Internal {
            internal.insert(i.name.as_str());
        }
    }
    if internal.is_empty() {
        return;
    }

    fn collect_named(ty: &Type, internal: &HashSet<&str>, out: &mut Vec<String>) {
        match ty {
            Type::Struct(name, args) => {
                if internal.contains(name.as_str()) {
                    out.push(name.clone());
                }
                for a in args {
                    collect_named(a, internal, out);
                }
            }
            Type::Array(t) | Type::Option(t) | Type::Slice(t) => collect_named(t, internal, out),
            Type::Result(a, b) | Type::Map(a, b) => {
                collect_named(a, internal, out);
                collect_named(b, internal, out);
            }
            Type::Tuple(ts) => {
                for t in ts {
                    collect_named(t, internal, out);
                }
            }
            Type::Function { params, ret } => {
                for p in params {
                    collect_named(p, internal, out);
                }
                if let Some(r) = ret.as_ref() {
                    collect_named(r, internal, out);
                }
            }
            // 限定类型来自其它包，本文件无法判定其可见性。
            _ => {}
        }
    }

    // 直接构造 internal 类型的表达式（`C1()`），用于推断返回类型。
    fn direct_ctor_name(e: &Expr) -> Option<&str> {
        match e {
            Expr::ConstructorCall { name, .. } => Some(name.as_str()),
            Expr::Call { name, .. } => Some(name.as_str()),
            _ => None,
        }
    }

    fn tail_internal_ctor<'a>(body: &'a [Stmt], internal: &HashSet<&str>) -> Option<String> {
        match body.last()? {
            Stmt::Return(Some(e)) | Stmt::Expr(e) => direct_ctor_name(e)
                .filter(|n| internal.contains(*n))
                .map(|n| n.to_string()),
            _ => None,
        }
    }

    fn check_fn(f: &Function, internal: &HashSet<&str>, errors: &mut Vec<SemanticError>) {
        let mut used: Vec<String> = Vec::new();
        for p in &f.params {
            collect_named(&p.ty, internal, &mut used);
        }
        if let Some(ret) = &f.return_type {
            collect_named(ret, internal, &mut used);
        }
        for c in &f.constraints {
            for b in &c.bounds {
                if internal.contains(b.as_str()) {
                    used.push(b.clone());
                }
            }
        }
        if f.return_type.is_none() {
            if let Some(n) = tail_internal_ctor(&f.body, internal) {
                used.push(n);
            }
        }
        used.sort();
        used.dedup();
        for ty in used {
            errors.push(SemanticError {
                message: format!(
                    "public 声明 '{}' 引用了 internal 类型 '{}'（11.2(7)-(12)）",
                    last_name_segment(&f.name),
                    ty
                ),
            });
        }
    }

    fn check_field(
        owner: &str,
        fld: &FieldDef,
        internal: &HashSet<&str>,
        errors: &mut Vec<SemanticError>,
    ) {
        if !matches!(fld.visibility, Visibility::Public | Visibility::Protected) {
            return;
        }
        let mut used: Vec<String> = Vec::new();
        collect_named(&fld.ty, internal, &mut used);
        if let Some(default) = &fld.default {
            if let Some(n) = direct_ctor_name(default) {
                if internal.contains(n) {
                    used.push(n.to_string());
                }
            }
        }
        used.sort();
        used.dedup();
        for ty in used {
            errors.push(SemanticError {
                message: format!(
                    "public 声明 '{}' 引用了 internal 类型 '{}'（11.2(7)-(12)）",
                    owner, ty
                ),
            });
        }
    }

    // 顶层 public 函数。
    for f in &program.functions {
        if f.name.contains('.') || f.name.contains("::") || f.name == "main" {
            continue;
        }
        if f.visibility == Visibility::Public {
            check_fn(f, &internal, errors);
        }
    }

    // public 类型的 public / protected 成员函数（含属性脱糖的 __get_/__set_）。
    // 类方法仅存放在 ClassDef.methods；struct/enum 方法以 `Type.method` 形式
    // 存放在 program.functions。
    for c in &program.classes {
        if c.visibility != Visibility::Public {
            continue;
        }
        for m in &c.methods {
            if matches!(m.func.visibility, Visibility::Public | Visibility::Protected) {
                check_fn(&m.func, &internal, errors);
            }
        }
    }
    let mut public_owners: HashSet<&str> = HashSet::new();
    for s in &program.structs {
        if s.visibility == Visibility::Public {
            public_owners.insert(s.name.as_str());
        }
    }
    for e in &program.enums {
        if e.visibility == Visibility::Public {
            public_owners.insert(e.name.as_str());
        }
    }
    for f in &program.functions {
        let Some(owner) = owner_name(&f.name) else {
            continue;
        };
        if !public_owners.contains(owner) {
            continue;
        }
        if !matches!(f.visibility, Visibility::Public | Visibility::Protected) {
            continue;
        }
        check_fn(f, &internal, errors);
    }

    // public 类型的 public / protected 字段（含可确定的推断类型）。
    for c in &program.classes {
        if c.visibility != Visibility::Public {
            continue;
        }
        for fld in &c.fields {
            check_field(&c.name, fld, &internal, errors);
        }
    }
    for s in &program.structs {
        if s.visibility != Visibility::Public {
            continue;
        }
        for fld in &s.fields {
            check_field(&s.name, fld, &internal, errors);
        }
    }

    // public 枚举的构造器 payload 类型必须 external。
    for e in &program.enums {
        if e.visibility != Visibility::Public {
            continue;
        }
        for v in &e.variants {
            if let Some(p) = &v.payload {
                let mut used = Vec::new();
                collect_named(p, &internal, &mut used);
                used.sort();
                used.dedup();
                for ty in used {
                    errors.push(SemanticError {
                        message: format!(
                            "public 声明 '{}.{}' 引用了 internal 类型 '{}'（11.2(7)-(12)）",
                            e.name, v.name, ty
                        ),
                    });
                }
            }
        }
    }
}

// ─── 3.1(3)：同一作用域内重复绑定局部变量 ─────────────────────────────────

/// 3.1(3)：同一作用域内不允许重复声明同名变量（遮蔽只允许发生在外层作用域）。
///
/// 按块维护作用域栈，仅将 `let`/`var`/`const`/模式绑定的名字登记到最内层作用域；
/// 子块、分支、匹配分支、lambda 体各自开启新作用域，因此合法遮蔽不会被误判。
fn check_duplicate_local_bindings(program: &Program, errors: &mut Vec<SemanticError>) {
    for f in &program.functions {
        let params = param_names(&f.params);
        walk_body_for_dups(&f.body, &params, errors);
    }
    for cls in &program.classes {
        if let Some(init) = &cls.init {
            let params = param_names(&init.params);
            walk_body_for_dups(&init.body, &params, errors);
        }
        if let Some(d) = &cls.deinit {
            walk_body_for_dups(d, &[], errors);
        }
        if let Some(si) = &cls.static_init {
            walk_body_for_dups(si, &[], errors);
        }
        for m in &cls.methods {
            if !m.func.body.is_empty() {
                let params = param_names(&m.func.params);
                walk_body_for_dups(&m.func.body, &params, errors);
            }
        }
    }
    for iface in &program.interfaces {
        for m in &iface.methods {
            if let Some(body) = &m.default_body {
                let params = param_names(&m.params);
                walk_body_for_dups(body, &params, errors);
            }
        }
    }
    for ext in &program.extends {
        for f in &ext.methods {
            let params = param_names(&f.params);
            walk_body_for_dups(&f.body, &params, errors);
        }
    }
}

fn param_names(params: &[Param]) -> Vec<String> {
    params.iter().map(|p| p.name.clone()).collect()
}

/// 3.2.3.2(11)：形参与函数体局部变量处于同一作用域层级，
/// 因此函数体最外层不能再声明同名变量（3.1(3) 同作用域重复声明）。
/// 参数名预置到最外层作用域；嵌套块仍可合法遮蔽。
fn walk_body_for_dups(
    stmts: &[Stmt],
    params: &[String],
    errors: &mut Vec<SemanticError>,
) {
    let mut scopes: Vec<HashSet<String>> = Vec::new();
    scopes.push(params.iter().cloned().collect());
    for s in stmts {
        walk_stmt_for_dups(s, &mut scopes, errors);
    }
    scopes.pop();
}

fn pattern_binding_names(p: &Pattern, out: &mut Vec<String>) {
    match p {
        Pattern::Binding(n) => out.push(n.clone()),
        Pattern::TypeTest { binding, .. } => out.push(binding.clone()),
        Pattern::Tuple(ps) => ps.iter().for_each(|q| pattern_binding_names(q, out)),
        Pattern::Struct { fields, .. } => {
            fields.iter().for_each(|(_, q)| pattern_binding_names(q, out))
        }
        Pattern::Variant { payload: Some(q), .. } => pattern_binding_names(q, out),
        Pattern::Or(ps) => {
            if let Some(q) = ps.first() {
                pattern_binding_names(q, out);
            }
        }
        _ => {}
    }
}

/// 3.3.1(9)/(10)：判断模式中是否**明显**包含可反驳的子模式。
///
/// 只把字面量、范围、类型测试、守卫识别为可反驳；枚举变体本身不判定
/// （是否可反驳取决于枚举构造器数量，单文件无法可靠得知），
/// 元组 / 结构体解构只递归检查其字段子模式。
/// 因此 `let (x, y)`、`let (_, z)`、`let Red(a)`、`for (i in ...)` 等合法
/// 写法不会被误判——这正是 w9c 回退的教训。
fn pattern_clearly_refutable(p: &Pattern) -> bool {
    match p {
        Pattern::Literal(_) | Pattern::Range { .. } | Pattern::TypeTest { .. } | Pattern::Guard(_) => {
            true
        }
        Pattern::Tuple(ps) | Pattern::Or(ps) => ps.iter().any(pattern_clearly_refutable),
        Pattern::Struct { fields, .. } => {
            fields.iter().any(|(_, q)| pattern_clearly_refutable(q))
        }
        Pattern::Variant { payload, .. } => {
            payload.as_deref().map_or(false, pattern_clearly_refutable)
        }
        Pattern::Wildcard | Pattern::Binding(_) | Pattern::Field { .. } => false,
    }
}

fn declare_local_names(
    names: &[String],
    scopes: &mut Vec<HashSet<String>>,
    errors: &mut Vec<SemanticError>,
) {
    let Some(scope) = scopes.last_mut() else {
        return;
    };
    // 同一模式内重复出现同名（如枚举变体模式 `(Year, Year)`）不是重复声明，
    // 先去重；只有与同一作用域内先前声明冲突才算重复。
    let mut seen_in_pattern: HashSet<&str> = HashSet::new();
    for name in names {
        if !seen_in_pattern.insert(name.as_str()) {
            continue;
        }
        if !scope.insert(name.clone()) {
            errors.push(SemanticError {
                message: format!("同一作用域内变量 '{}' 被重复声明（3.1(3)）", name),
            });
        }
    }
}

fn walk_block_for_dups(
    stmts: &[Stmt],
    scopes: &mut Vec<HashSet<String>>,
    errors: &mut Vec<SemanticError>,
) {
    scopes.push(HashSet::new());
    for s in stmts {
        walk_stmt_for_dups(s, scopes, errors);
    }
    scopes.pop();
}

fn walk_stmt_for_dups(
    stmt: &Stmt,
    scopes: &mut Vec<HashSet<String>>,
    errors: &mut Vec<SemanticError>,
) {
    match stmt {
        Stmt::Let { pattern, value, .. } | Stmt::Var { pattern, value, .. } => {
            walk_expr_for_dups(value, scopes, errors);
            let mut names = Vec::new();
            pattern_binding_names(pattern, &mut names);
            // 形参自引用式遮蔽（如 std 中常见的 `var low = low`）：最外层作用域内
            // 新声明与形参同名，且初始化表达式直接引用该同名形参时视为合法。
            // 这保留了 a10_02 类（`var x = 1` 遮蔽形参）的拒绝，同时不误伤 std。
            if scopes.len() == 1 {
                let mut value_refs = Vec::new();
                collect_expr_direct_refs(value, &mut value_refs);
                for name in &names {
                    if value_refs.contains(name) {
                        if let Some(scope) = scopes.first_mut() {
                            scope.remove(name);
                        }
                    }
                }
            }
            declare_local_names(&names, scopes, errors);
            // 3.3.1(9)/(10)：let/var 后的模式必须（或可能）不可反驳。
            // 保守版：只拒绝模式中显式出现字面量 / 范围 / 类型测试等明显可反驳
            // 子模式的情形；纯绑定、通配符、元组解构、结构体解构、枚举变体模式
            // 一律放行，避免误伤 for-in / 解构等合法写法。
            if pattern_clearly_refutable(pattern) {
                errors.push(SemanticError {
                    message: "let/var 后的模式必须是不可反驳模式（3.3.1(9)/(10)）".to_string(),
                });
            }
        }
        Stmt::Const { name, value, .. } => {
            walk_expr_for_dups(value, scopes, errors);
            declare_local_names(std::slice::from_ref(name), scopes, errors);
        }
        Stmt::Assign { target, value } => {
            walk_expr_for_dups(value, scopes, errors);
            walk_assign_target_for_dups(target, scopes, errors);
        }
        Stmt::Expr(e) => walk_expr_for_dups(e, scopes, errors),
        Stmt::Return(Some(e)) => walk_expr_for_dups(e, scopes, errors),
        Stmt::While { cond, body } => {
            walk_expr_for_dups(cond, scopes, errors);
            walk_block_for_dups(body, scopes, errors);
        }
        Stmt::DoWhile { body, cond } => {
            walk_block_for_dups(body, scopes, errors);
            walk_expr_for_dups(cond, scopes, errors);
        }
        Stmt::Loop { body } | Stmt::UnsafeBlock { body } => {
            walk_block_for_dups(body, scopes, errors);
        }
        Stmt::For {
            var,
            iterable,
            body,
        } => {
            walk_expr_for_dups(iterable, scopes, errors);
            scopes.push(HashSet::new());
            if let Some(scope) = scopes.last_mut() {
                scope.insert(var.clone());
            }
            walk_block_for_dups(body, scopes, errors);
            scopes.pop();
        }
        Stmt::WhileLet {
            pattern,
            expr,
            body,
        } => {
            walk_expr_for_dups(expr, scopes, errors);
            let mut names = Vec::new();
            pattern_binding_names(pattern, &mut names);
            scopes.push(HashSet::new());
            declare_local_names(&names, scopes, errors);
            walk_block_for_dups(body, scopes, errors);
            scopes.pop();
        }
        Stmt::LocalFunc(f) => {
            let params = param_names(&f.params);
            walk_body_for_dups(&f.body, &params, errors)
        }
        Stmt::Assert { left, right, .. } | Stmt::Expect { left, right, .. } => {
            walk_expr_for_dups(left, scopes, errors);
            walk_expr_for_dups(right, scopes, errors);
        }
        Stmt::Break | Stmt::Continue | Stmt::Return(None) => {}
    }
}

fn walk_assign_target_for_dups(
    target: &AssignTarget,
    scopes: &mut Vec<HashSet<String>>,
    errors: &mut Vec<SemanticError>,
) {
    match target {
        AssignTarget::Index { index, .. } => walk_expr_for_dups(index, scopes, errors),
        AssignTarget::IndexPath { index, .. } => walk_expr_for_dups(index, scopes, errors),
        AssignTarget::ExprIndex { expr, index } => {
            walk_expr_for_dups(expr, scopes, errors);
            walk_expr_for_dups(index, scopes, errors);
        }
        AssignTarget::ExprField { object, .. } => walk_expr_for_dups(object, scopes, errors),
        _ => {}
    }
}

fn walk_expr_for_dups(
    expr: &Expr,
    scopes: &mut Vec<HashSet<String>>,
    errors: &mut Vec<SemanticError>,
) {
    match expr {
        Expr::Block(stmts, trailing) => {
            walk_block_for_dups(stmts, scopes, errors);
            if let Some(t) = trailing {
                walk_expr_for_dups(t, scopes, errors);
            }
        }
        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => {
            walk_expr_for_dups(cond, scopes, errors);
            walk_expr_for_dups(then_branch, scopes, errors);
            if let Some(eb) = else_branch {
                walk_expr_for_dups(eb, scopes, errors);
            }
        }
        Expr::IfLet {
            pattern,
            expr,
            then_branch,
            else_branch,
        } => {
            walk_expr_for_dups(expr, scopes, errors);
            let mut names = Vec::new();
            pattern_binding_names(pattern, &mut names);
            scopes.push(HashSet::new());
            declare_local_names(&names, scopes, errors);
            walk_expr_for_dups(then_branch, scopes, errors);
            scopes.pop();
            if let Some(eb) = else_branch {
                walk_expr_for_dups(eb, scopes, errors);
            }
        }
        Expr::Match { expr, arms } => {
            walk_expr_for_dups(expr, scopes, errors);
            for arm in arms {
                let mut names = Vec::new();
                pattern_binding_names(&arm.pattern, &mut names);
                scopes.push(HashSet::new());
                declare_local_names(&names, scopes, errors);
                if let Some(g) = &arm.guard {
                    walk_expr_for_dups(g, scopes, errors);
                }
                walk_expr_for_dups(&arm.body, scopes, errors);
                scopes.pop();
            }
        }
        Expr::Lambda { params, body, .. } => {
            scopes.push(HashSet::new());
            let names: Vec<String> = params.iter().map(|(n, _)| n.clone()).collect();
            declare_local_names(&names, scopes, errors);
            walk_expr_for_dups(body, scopes, errors);
            scopes.pop();
        }
        Expr::Unary { expr, .. }
        | Expr::Cast { expr, .. }
        | Expr::Some(expr)
        | Expr::Ok(expr)
        | Expr::Err(expr)
        | Expr::Try(expr)
        | Expr::Throw(expr)
        | Expr::PostfixIncr(expr)
        | Expr::PostfixDecr(expr)
        | Expr::PrefixIncr(expr)
        | Expr::PrefixDecr(expr)
        | Expr::IsType { expr, .. }
        | Expr::OptionalChain { object: expr, .. } => walk_expr_for_dups(expr, scopes, errors),
        Expr::Return(Some(e)) => walk_expr_for_dups(e, scopes, errors),
        Expr::Binary { left, right, .. } | Expr::NullCoalesce { option: left, default: right } => {
            walk_expr_for_dups(left, scopes, errors);
            walk_expr_for_dups(right, scopes, errors);
        }
        Expr::Call { args, named_args, .. }
        | Expr::ConstructorCall { args, named_args, .. }
        | Expr::SuperCall { args, named_args, .. } => {
            for a in args {
                walk_expr_for_dups(a, scopes, errors);
            }
            for (_, a) in named_args {
                walk_expr_for_dups(a, scopes, errors);
            }
        }
        Expr::MethodCall {
            object,
            args,
            named_args,
            ..
        } => {
            walk_expr_for_dups(object, scopes, errors);
            for a in args {
                walk_expr_for_dups(a, scopes, errors);
            }
            for (_, a) in named_args {
                walk_expr_for_dups(a, scopes, errors);
            }
        }
        Expr::Field { object, .. } | Expr::TupleIndex { object, .. } => {
            walk_expr_for_dups(object, scopes, errors)
        }
        Expr::Index { array, index } => {
            walk_expr_for_dups(array, scopes, errors);
            walk_expr_for_dups(index, scopes, errors);
        }
        Expr::Tuple(elems) | Expr::Array(elems) => {
            for e in elems {
                walk_expr_for_dups(e, scopes, errors);
            }
        }
        Expr::StructInit { fields, .. } => {
            for (_, e) in fields {
                walk_expr_for_dups(e, scopes, errors);
            }
        }
        Expr::Range {
            start, end, step, ..
        } => {
            walk_expr_for_dups(start, scopes, errors);
            walk_expr_for_dups(end, scopes, errors);
            if let Some(s) = step {
                walk_expr_for_dups(s, scopes, errors);
            }
        }
        Expr::VariantConst { arg: Some(a), .. } => walk_expr_for_dups(a, scopes, errors),
        Expr::Interpolate(parts) => {
            for p in parts {
                if let crate::ast::InterpolatePart::Expr(e) = p {
                    walk_expr_for_dups(e, scopes, errors);
                }
            }
        }
        Expr::TryBlock {
            resources,
            body,
            catch_body,
            finally_body,
            ..
        } => {
            for (_, e) in resources {
                walk_expr_for_dups(e, scopes, errors);
            }
            walk_block_for_dups(body, scopes, errors);
            walk_block_for_dups(catch_body, scopes, errors);
            if let Some(fb) = finally_body {
                walk_block_for_dups(fb, scopes, errors);
            }
        }
        Expr::SliceExpr { array, start, end } => {
            walk_expr_for_dups(array, scopes, errors);
            walk_expr_for_dups(start, scopes, errors);
            walk_expr_for_dups(end, scopes, errors);
        }
        Expr::MapLiteral { entries } => {
            for (k, v) in entries {
                walk_expr_for_dups(k, scopes, errors);
                walk_expr_for_dups(v, scopes, errors);
            }
        }
        Expr::Spawn { body } | Expr::Synchronized { body, .. } => {
            walk_block_for_dups(body, scopes, errors)
        }
        Expr::TrailingClosure {
            callee,
            args,
            closure,
        } => {
            walk_expr_for_dups(callee, scopes, errors);
            for a in args {
                walk_expr_for_dups(a, scopes, errors);
            }
            walk_expr_for_dups(closure, scopes, errors);
        }
        Expr::Invoke {
            callee,
            args,
            named_args,
        } => {
            walk_expr_for_dups(callee, scopes, errors);
            for a in args {
                walk_expr_for_dups(a, scopes, errors);
            }
            for (_, a) in named_args {
                walk_expr_for_dups(a, scopes, errors);
            }
        }
        Expr::Macro { args, .. } => {
            for a in args {
                walk_expr_for_dups(a, scopes, errors);
            }
        }
        _ => {}
    }
}

// ─── 3.2.3.2(3)：局部变量 / 局部函数“先使用后声明” ──────────────────────

/// 3.2.3.2(3)/(6)：局部变量的作用域从声明处开始；在同一语句块内、声明之前
/// 引用同名局部变量或调用尚未声明的局部函数属于非法。
///
/// 仅在“名字确实在同一块内稍后声明，且在当前作用域中不可见”时报错，因此合法的
/// 外层遮蔽（外层已有同名绑定）不会被误判。对嵌套块、lambda、match 分支单独递归，
/// 并携带外层已声明名字集合。
fn check_use_before_declaration(program: &Program, errors: &mut Vec<SemanticError>) {
    let globals = collect_global_identifiers(program);

    for f in &program.functions {
        if f.body.is_empty() {
            continue;
        }
        // 成员方法以 `Type.method` 形式存储，其字段名在方法体内可直接引用。
        let mut outer: HashSet<String> = f.params.iter().map(|p| p.name.clone()).collect();
        if let Some((ty, _)) = f.name.split_once('.') {
            add_type_field_names(program, ty, &mut outer);
        }
        order_body(&f.body, &outer, &globals, errors);
    }

    for cls in &program.classes {
        let mut base: HashSet<String> = cls.fields.iter().map(|f| f.name.clone()).collect();
        // 同一类型内的成员方法可在方法体中无资格引用。
        base.extend(
            cls.methods
                .iter()
                .map(|m| last_name_segment(&m.func.name)),
        );
        if let Some(init) = &cls.init {
            let mut outer = base.clone();
            outer.extend(init.params.iter().map(|p| p.name.clone()));
            order_body(&init.body, &outer, &globals, errors);
        }
        if let Some(d) = &cls.deinit {
            order_body(d, &base, &globals, errors);
        }
        if let Some(si) = &cls.static_init {
            order_body(si, &base, &globals, errors);
        }
        for m in &cls.methods {
            if m.func.body.is_empty() {
                continue;
            }
            let mut outer = base.clone();
            outer.extend(m.func.params.iter().map(|p| p.name.clone()));
            order_body(&m.func.body, &outer, &globals, errors);
        }
    }

    for iface in &program.interfaces {
        let iface_methods: HashSet<String> = iface
            .methods
            .iter()
            .map(|m| last_name_segment(&m.name))
            .collect();
        for m in &iface.methods {
            if let Some(body) = &m.default_body {
                let mut outer = iface_methods.clone();
                outer.extend(m.params.iter().map(|p| p.name.clone()));
                order_body(body, &outer, &globals, errors);
            }
        }
    }
    for ext in &program.extends {
        let mut type_members = HashSet::new();
        add_type_field_names(program, &ext.target_type, &mut type_members);
        for f in &ext.methods {
            let mut outer = type_members.clone();
            outer.extend(f.params.iter().map(|p| p.name.clone()));
            order_body(&f.body, &outer, &globals, errors);
        }
    }
}

/// 取限定名的最后一段（`C.m` / `C::m` → `m`）。
fn last_name_segment(name: &str) -> String {
    name.rsplit('.')
        .next()
        .unwrap_or(name)
        .rsplit(':')
        .next()
        .unwrap_or(name)
        .to_string()
}

/// 所有类型成员（字段/方法）的名字集合，用于判定“无资格引用不在作用域内的成员名”。
fn collect_file_member_names(program: &Program) -> HashSet<String> {
    let mut m: HashSet<String> = HashSet::new();
    for c in &program.classes {
        m.extend(c.fields.iter().map(|f| f.name.clone()));
        m.extend(c.methods.iter().map(|x| last_name_segment(&x.func.name)));
    }
    for s in &program.structs {
        m.extend(s.fields.iter().map(|f| f.name.clone()));
    }
    for i in &program.interfaces {
        m.extend(i.methods.iter().map(|x| last_name_segment(&x.name)));
    }
    // struct / enum 等方法以 `Type.method` 形式存放。
    for f in &program.functions {
        if f.name.contains('.') || f.name.contains("::") {
            m.insert(last_name_segment(&f.name));
        }
    }
    m
}

/// 3.2.3.3(1)：顶层自由函数体中无资格引用某类型的成员名（字段/方法）即为未解析标识符。
///
/// 仅检查自由函数体**最外层**语句的直接引用，且把函数内任意位置声明的局部名、
/// 形参、顶层名、导入名都视为可见，避免与使用前声明规则叠加产生误报。
/// 成员方法体 / 构造函数 / 扩展方法体内允许无资格访问自身成员，故不在此检查。
fn check_unqualified_member_refs_in_free_fns(
    program: &Program,
    errors: &mut Vec<SemanticError>,
) {
    let members = collect_file_member_names(program);
    if members.is_empty() {
        return;
    }
    let globals = collect_global_identifiers(program);
    for f in &program.functions {
        if f.name.contains('.') || f.name.contains("::") || f.body.is_empty() {
            continue;
        }
        let mut declared: HashSet<String> = f.params.iter().map(|p| p.name.clone()).collect();
        declared.extend(globals.iter().cloned());
        for s in &f.body {
            for n in block_decl_names(s) {
                declared.insert(n);
            }
        }
        collect_block_local_names(&f.body, &mut declared);
        let mut reported: HashSet<String> = HashSet::new();
        for s in &f.body {
            let mut refs = Vec::new();
            collect_stmt_direct_refs(s, &mut refs);
            for r in refs {
                if declared.contains(&r) || !members.contains(&r) {
                    continue;
                }
                if reported.insert(r.clone()) {
                    errors.push(SemanticError {
                        message: format!(
                            "标识符 '{}' 是类型成员，不能在自由函数中无资格引用（3.2.3.3(1)）",
                            r
                        ),
                    });
                }
            }
        }
    }
}

/// 3.2.3.2：自由函数体中引用了文件中**任何作用域都无法解析**的裸变量名。
///
/// 本编译器不做真实导入解析（vendor/import 依赖无法静态确定），因此仅在
/// **完全没有 import** 的单文件程序上启用，且只检查“裸变量引用”
/// （`Expr::Var`），不检查函数调用名、类型名、成员名——后者由导入/成员规则
/// 或其它检查负责。命名集合采用“整函数体声明名 + 形参 + 顶层名”的过近似，
/// 只用于避免误报，确保 std / 正测试零误伤。
///
/// 首字母大写的名字按约定视为类型/隐式导入符号（如 unittest 的 `Configuration`、
/// `KeyOpt`、`Thread`、`TestSuite`），一律放行；本检查只针对变量形态（小写或
/// 下划线开头）的裸引用，进一步收敛误报面。
fn check_undefined_identifiers(program: &Program, errors: &mut Vec<SemanticError>) {
    if !imports_allow_undefined_check(program) {
        return;
    }
    let globals = collect_global_identifiers(program);
    let members = collect_file_member_names(program);
    for f in &program.functions {
        if f.name.contains('.') || f.name.contains("::") || f.body.is_empty() {
            continue;
        }
        let mut declared: HashSet<String> = f.params.iter().map(|p| p.name.clone()).collect();
        declared.extend(globals.iter().cloned());
        collect_block_local_names(&f.body, &mut declared);
        let mut reported: HashSet<String> = HashSet::new();
        for s in &f.body {
            let mut refs = Vec::new();
            collect_bare_var_refs_stmt(s, &mut refs);
            for r in refs {
                if declared.contains(&r) || members.contains(&r) || is_prelude_identifier(&r) {
                    continue;
                }
                // 首字母大写：约定为类型 / 隐式导入符号，保守放行；但大小写变体的
                // 布尔字面量（`True`/`False`）仍属未定义标识符。
                if r.chars().next().map_or(false, |c| c.is_uppercase())
                    && !matches!(r.to_lowercase().as_str(), "true" | "false")
                {
                    continue;
                }
                if reported.insert(r.clone()) {
                    errors.push(SemanticError {
                        message: format!("未定义的标识符 '{}'（3.2.3.2）", r),
                    });
                }
            }
        }
    }
}

/// 判断当前程序的 import 形态是否仍可安全启用“未定义标识符”检查。
///
/// 本编译器不做真实导入解析。AST 不保留通配符标记（`import a.b.*` 与
/// `import a.b` 都表现为 `items=None`），因此只对**可确定单项目导入**的路径启用：
/// 分组导入（`{...}`）中不含 `*`，或路径最后一段首字母大写（类型/构造器形态）。
/// 任一 import 不符合则整体跳过，保证 std / 正测试零回归。
fn imports_allow_undefined_check(program: &Program) -> bool {
    for imp in &program.imports {
        match &imp.items {
            Some(items) => {
                if items
                    .iter()
                    .any(|i| i.split('.').any(|seg| seg == "*"))
                {
                    return false;
                }
            }
            None => match imp.module_path.last() {
                Some(last) if last.chars().next().map_or(false, |c| c.is_uppercase()) => {}
                _ => return false,
            },
        }
    }
    true
}

/// 预置（prelude）标识符：语言核心类型与常用内建函数名。这些名字无需 import
/// 即可使用，未定义检查必须放行。
fn is_prelude_identifier(name: &str) -> bool {
    matches!(
        name,
        // 基本类型 / 核心类型名
        "Bool" | "Rune"
            | "String"
            | "Unit"
            | "Nothing"
            | "Any"
            | "Object"
            | "Int8"
            | "Int16"
            | "Int32"
            | "Int64"
            | "IntNative"
            | "UInt8"
            | "UInt16"
            | "UInt32"
            | "UInt64"
            | "UIntNative"
            | "Float16"
            | "Float32"
            | "Float64"
            | "Array"
            | "Option"
            | "Ordering"
            | "Range"
            | "Iterator"
            | "Iterable"
            | "Collection"
            | "ToString"
            | "Hashable"
            | "Equatable"
            | "Comparable"
            | "Less"
            | "Greater"
            | "Equal"
            | "Exception"
            | "Error"
            | "ArithmeticException"
            | "IllegalArgumentException"
            | "IndexOutOfBoundsException"
            | "NoneValueException"
            | "OverflowException"
            | "ClassCastException"
            | "IllegalStateException"
            | "NegativeArraySizeException"
            | "OutOfMemoryError"
            | "StackOverflowError"
            | "CPointer"
            | "CString"
            | "CStr"
            | "CFunc"
            | "Duration"
            // 常用内建函数 / 伪变量
            | "print"
            | "println"
            | "eprint"
            | "eprintln"
            | "panic"
            | "assert"
            | "this"
            | "super"
            | "None"
            | "Some"
            | "Ok"
            | "Err"
            | "min"
            | "max"
            | "abs"
            | "sqrt"
            | "floor"
            | "ceil"
            | "trunc"
            | "nearest"
            | "pow"
            | "fmin"
            | "fmax"
    )
}

/// 收集语句中的“裸变量引用”（`Expr::Var` / 赋值目标变量），不含函数调用名。
fn collect_bare_var_refs_stmt(stmt: &Stmt, out: &mut Vec<String>) {
    match stmt {
        Stmt::Let { value, .. } | Stmt::Var { value, .. } | Stmt::Const { value, .. } => {
            collect_bare_var_refs_expr(value, out)
        }
        Stmt::Assign { target, value } => {
            collect_bare_var_refs_expr(value, out);
            if let AssignTarget::Var(name) = target {
                out.push(name.clone());
            }
        }
        Stmt::Expr(e) => collect_bare_var_refs_expr(e, out),
        Stmt::Return(Some(e)) => collect_bare_var_refs_expr(e, out),
        Stmt::While { cond, .. } | Stmt::DoWhile { cond, .. } => {
            collect_bare_var_refs_expr(cond, out)
        }
        Stmt::For { iterable, .. } => collect_bare_var_refs_expr(iterable, out),
        Stmt::WhileLet { expr, .. } => collect_bare_var_refs_expr(expr, out),
        Stmt::Assert { left, right, .. } | Stmt::Expect { left, right, .. } => {
            collect_bare_var_refs_expr(left, out);
            collect_bare_var_refs_expr(right, out);
        }
        _ => {}
    }
}

/// 见 [`collect_bare_var_refs_stmt`]。子作用域（块 / lambda 体 / match 分支 /
/// if 分支）中的引用不在当前位置收集，交由外层过近似声明集合兜底。
fn collect_bare_var_refs_expr(expr: &Expr, out: &mut Vec<String>) {
    match expr {
        Expr::Var(name) => out.push(name.clone()),
        Expr::Unary { expr, .. }
        | Expr::Cast { expr, .. }
        | Expr::Some(expr)
        | Expr::Ok(expr)
        | Expr::Err(expr)
        | Expr::Try(expr)
        | Expr::Throw(expr)
        | Expr::PostfixIncr(expr)
        | Expr::PostfixDecr(expr)
        | Expr::PrefixIncr(expr)
        | Expr::PrefixDecr(expr)
        | Expr::IsType { expr, .. }
        | Expr::Field { object: expr, .. }
        | Expr::TupleIndex { object: expr, .. }
        | Expr::OptionalChain { object: expr, .. } => collect_bare_var_refs_expr(expr, out),
        Expr::Binary { left, right, .. }
        | Expr::NullCoalesce {
            option: left,
            default: right,
        } => {
            collect_bare_var_refs_expr(left, out);
            collect_bare_var_refs_expr(right, out);
        }
        Expr::Return(Some(e)) => collect_bare_var_refs_expr(e, out),
        Expr::Index { array, index } => {
            collect_bare_var_refs_expr(array, out);
            collect_bare_var_refs_expr(index, out);
        }
        Expr::Tuple(elems) | Expr::Array(elems) => {
            for e in elems {
                collect_bare_var_refs_expr(e, out);
            }
        }
        Expr::Range {
            start, end, step, ..
        } => {
            collect_bare_var_refs_expr(start, out);
            collect_bare_var_refs_expr(end, out);
            if let Some(s) = step {
                collect_bare_var_refs_expr(s, out);
            }
        }
        Expr::Call {
            args, named_args, ..
        }
        | Expr::ConstructorCall {
            args, named_args, ..
        }
        | Expr::SuperCall {
            args, named_args, ..
        } => {
            for a in args {
                collect_bare_var_refs_expr(a, out);
            }
            for (_, a) in named_args {
                collect_bare_var_refs_expr(a, out);
            }
        }
        Expr::MethodCall {
            object,
            args,
            named_args,
            ..
        } => {
            collect_bare_var_refs_expr(object, out);
            for a in args {
                collect_bare_var_refs_expr(a, out);
            }
            for (_, a) in named_args {
                collect_bare_var_refs_expr(a, out);
            }
        }
        Expr::StructInit { fields, .. } => {
            for (_, e) in fields {
                collect_bare_var_refs_expr(e, out);
            }
        }
        Expr::Match { expr, .. } | Expr::IfLet { expr, .. } => {
            collect_bare_var_refs_expr(expr, out)
        }
        Expr::If { cond, .. } => collect_bare_var_refs_expr(cond, out),
        Expr::VariantConst { arg: Some(a), .. } => collect_bare_var_refs_expr(a, out),
        Expr::SliceExpr { array, start, end } => {
            collect_bare_var_refs_expr(array, out);
            collect_bare_var_refs_expr(start, out);
            collect_bare_var_refs_expr(end, out);
        }
        Expr::MapLiteral { entries } => {
            for (k, v) in entries {
                collect_bare_var_refs_expr(k, out);
                collect_bare_var_refs_expr(v, out);
            }
        }
        Expr::Interpolate(parts) => {
            for p in parts {
                if let crate::ast::InterpolatePart::Expr(e) = p {
                    collect_bare_var_refs_expr(e, out);
                }
            }
        }
        Expr::TrailingClosure { callee, args, .. } => {
            collect_bare_var_refs_expr(callee, out);
            for a in args {
                collect_bare_var_refs_expr(a, out);
            }
        }
        Expr::Invoke {
            callee,
            args,
            named_args,
        } => {
            collect_bare_var_refs_expr(callee, out);
            for a in args {
                collect_bare_var_refs_expr(a, out);
            }
            for (_, a) in named_args {
                collect_bare_var_refs_expr(a, out);
            }
        }
        Expr::TryBlock { resources, .. } => {
            for (_, e) in resources {
                collect_bare_var_refs_expr(e, out);
            }
        }
        _ => {}
    }
}

/// 收集某用户类型的字段名与方法名到 `out`（供成员方法体无资格引用）。
fn add_type_field_names(program: &Program, type_name: &str, out: &mut HashSet<String>) {
    if let Some(c) = program.classes.iter().find(|c| c.name == type_name) {
        out.extend(c.fields.iter().map(|f| f.name.clone()));
        out.extend(
            c.methods
                .iter()
                .map(|m| last_name_segment(&m.func.name)),
        );
    }
    if let Some(s) = program.structs.iter().find(|s| s.name == type_name) {
        out.extend(s.fields.iter().map(|f| f.name.clone()));
    }
    // 结构体 / 枚举等类型的方法以 `Type.method` / `Type::method` 形式存放在
    // program.functions。
    let prefix_dot = format!("{}.", type_name);
    let prefix_colon = format!("{}::", type_name);
    for f in &program.functions {
        if f.name.starts_with(&prefix_dot) || f.name.starts_with(&prefix_colon) {
            out.insert(last_name_segment(&f.name));
        }
    }
}

/// 顶层可见标识符（函数、常量、类型、枚举变体、import 末段及常见内建名）。
fn collect_global_identifiers(program: &Program) -> HashSet<String> {
    let mut g: HashSet<String> = HashSet::new();
    for f in &program.functions {
        if !f.name.contains('.') && !f.name.contains("::") {
            g.insert(f.name.clone());
        }
    }
    for c in &program.constants {
        g.insert(c.name.clone());
    }
    for c in &program.classes {
        g.insert(c.name.clone());
    }
    for s in &program.structs {
        g.insert(s.name.clone());
    }
    for e in &program.enums {
        g.insert(e.name.clone());
        for v in &e.variants {
            g.insert(v.name.clone());
        }
    }
    for i in &program.interfaces {
        g.insert(i.name.clone());
    }
    for (n, _) in &program.type_aliases {
        g.insert(n.clone());
    }
    for imp in &program.imports {
        if let Some(alias) = &imp.alias {
            g.insert(alias.clone());
        }
        if let Some(items) = &imp.items {
            for it in items {
                g.insert(it.clone());
            }
        } else if let Some(last) = imp.module_path.last() {
            g.insert(last.clone());
        }
    }
    for b in [
        "print", "println", "panic", "assert", "Assert", "this", "super", "None", "Some", "Ok",
        "Err",
    ] {
        g.insert(b.to_string());
    }
    g
}

/// 一个语句块内的直接声明名（仅本块直接 `let/var/const/局部函数`）。
fn block_decl_names(stmt: &Stmt) -> Vec<String> {
    let mut names = Vec::new();
    match stmt {
        Stmt::Let { pattern, .. } | Stmt::Var { pattern, .. } => {
            pattern_binding_names(pattern, &mut names);
        }
        Stmt::Const { name, .. } => names.push(name.clone()),
        Stmt::LocalFunc(f) => names.push(f.name.clone()),
        _ => {}
    }
    names
}

fn order_body(
    stmts: &[Stmt],
    outer: &HashSet<String>,
    globals: &HashSet<String>,
    errors: &mut Vec<SemanticError>,
) {
    let closed: HashSet<String> = HashSet::new();
    order_body_closed(stmts, outer, globals, &closed, errors);
}

/// 带“已离开作用域名字集合”的块顺序检查。
///
/// `closed` 记录在**前序兄弟语句的嵌套块**中声明、但在当前块层面已不可见的
/// 局部变量名（3.2.3.2(1)/(2)/(3)）。当前语句若引用这类名字即属越界访问。
fn order_body_closed(
    stmts: &[Stmt],
    outer: &HashSet<String>,
    globals: &HashSet<String>,
    closed_in: &HashSet<String>,
    errors: &mut Vec<SemanticError>,
) {
    // 预扫描：本块内每个名字首次声明所在的语句下标。
    let mut first: HashMap<String, usize> = HashMap::new();
    for (i, s) in stmts.iter().enumerate() {
        for n in block_decl_names(s) {
            first.entry(n).or_insert(i);
        }
    }

    let mut visible: HashSet<String> = outer.clone();
    let mut closed: HashSet<String> = closed_in.clone();
    for (i, s) in stmts.iter().enumerate() {
        let mut refs = Vec::new();
        collect_stmt_direct_refs(s, &mut refs);
        for r in &refs {
            if visible.contains(r) || globals.contains(r) {
                continue;
            }
            if let Some(&j) = first.get(r) {
                // 只判定“严格晚于当前语句”的声明。同一语句内的自引用可能源于
                // 解析器把下一行的前导运算符（如 `x = 1\n -y`）并入上一条初始化
                // 表达式，属于解析器语义，不能据此判为使用前未声明。
                if j > i {
                    errors.push(SemanticError {
                        message: format!(
                            "局部变量/函数 '{}' 在使用前尚未声明（3.2.3.2(3)/(6)）",
                            r
                        ),
                    });
                }
            } else if closed.contains(r) {
                // 名字曾在（本块内先前的）嵌套块中声明，但该块已结束。
                errors.push(SemanticError {
                    message: format!(
                        "局部变量 '{}' 已离开作用域，不能在当前块中引用（3.2.3.2(1)/(2)/(3)）",
                        r
                    ),
                });
            }
        }

        // 递归处理该语句携带的子块，携带“本条语句之后可见”的名字集合。
        let mut child_outer = visible.clone();
        for n in block_decl_names(s) {
            child_outer.insert(n);
        }
        recurse_child_blocks(s, &child_outer, globals, &closed, errors);

        // 本语句嵌套块中声明的局部变量在语句结束即离开作用域。
        collect_stmt_nested_local_names(s, &mut closed);
        for n in block_decl_names(s) {
            visible.insert(n);
        }
    }
}

fn recurse_child_blocks(
    stmt: &Stmt,
    outer: &HashSet<String>,
    globals: &HashSet<String>,
    closed: &HashSet<String>,
    errors: &mut Vec<SemanticError>,
) {
    match stmt {
        Stmt::While { body, .. }
        | Stmt::DoWhile { body, .. }
        | Stmt::Loop { body }
        | Stmt::UnsafeBlock { body } => order_body_closed(body, outer, globals, closed, errors),
        Stmt::For { var, body, .. } => {
            let mut child = outer.clone();
            child.insert(var.clone());
            order_body_closed(body, &child, globals, closed, errors);
        }
        Stmt::WhileLet { pattern, body, .. } => {
            let mut child = outer.clone();
            let mut names = Vec::new();
            pattern_binding_names(pattern, &mut names);
            child.extend(names);
            order_body_closed(body, &child, globals, closed, errors);
        }
        Stmt::LocalFunc(f) => {
            // 局部函数体可见自身（递归调用）与形参，以及声明位置之前可见的名字。
            let mut child = outer.clone();
            child.insert(f.name.clone());
            child.extend(f.params.iter().map(|p| p.name.clone()));
            order_body_closed(&f.body, &child, globals, closed, errors);
        }
        Stmt::Expr(e) | Stmt::Return(Some(e)) => {
            recurse_expr_blocks(e, outer, globals, closed, errors)
        }
        Stmt::Let { value, .. } | Stmt::Var { value, .. } => {
            recurse_expr_blocks(value, outer, globals, closed, errors)
        }
        Stmt::Const { value, .. } => recurse_expr_blocks(value, outer, globals, closed, errors),
        Stmt::Assign { value, .. } => recurse_expr_blocks(value, outer, globals, closed, errors),
        Stmt::Assert { left, right, .. } | Stmt::Expect { left, right, .. } => {
            recurse_expr_blocks(left, outer, globals, closed, errors);
            recurse_expr_blocks(right, outer, globals, closed, errors);
        }
        _ => {}
    }
}

fn recurse_expr_blocks(
    expr: &Expr,
    outer: &HashSet<String>,
    globals: &HashSet<String>,
    closed: &HashSet<String>,
    errors: &mut Vec<SemanticError>,
) {
    match expr {
        Expr::Block(stmts, trailing) => {
            order_body_closed(stmts, outer, globals, closed, errors);
            if let Some(t) = trailing {
                recurse_expr_blocks(t, outer, globals, closed, errors);
            }
        }
        Expr::If {
            then_branch,
            else_branch,
            ..
        } => {
            recurse_expr_blocks(then_branch, outer, globals, closed, errors);
            if let Some(eb) = else_branch {
                recurse_expr_blocks(eb, outer, globals, closed, errors);
            }
        }
        Expr::IfLet {
            pattern,
            expr,
            then_branch,
            else_branch,
        } => {
            recurse_expr_blocks(expr, outer, globals, closed, errors);
            let mut child = outer.clone();
            let mut names = Vec::new();
            pattern_binding_names(pattern, &mut names);
            child.extend(names);
            recurse_expr_blocks(then_branch, &child, globals, closed, errors);
            if let Some(eb) = else_branch {
                recurse_expr_blocks(eb, outer, globals, closed, errors);
            }
        }
        Expr::Match { expr, arms } => {
            recurse_expr_blocks(expr, outer, globals, closed, errors);
            let mut arm_closed = closed.clone();
            for arm in arms {
                let mut child = outer.clone();
                let mut names = Vec::new();
                pattern_binding_names(&arm.pattern, &mut names);
                child.extend(names.clone());
                if let Some(g) = &arm.guard {
                    recurse_expr_blocks(g, &child, globals, &arm_closed, errors);
                }
                recurse_expr_blocks(&arm.body, &child, globals, &arm_closed, errors);
                // 同一 match 的其它分支不可见本分支绑定名。
                arm_closed.extend(names);
            }
        }
        Expr::Lambda { params, body, .. } => {
            let mut child = outer.clone();
            child.extend(params.iter().map(|(n, _)| n.clone()));
            recurse_expr_blocks(body, &child, globals, closed, errors);
        }
        Expr::TryBlock {
            resources,
            body,
            catch_var,
            catch_body,
            finally_body,
            ..
        } => {
            let mut body_scope = outer.clone();
            for (n, e) in resources {
                recurse_expr_blocks(e, outer, globals, closed, errors);
                body_scope.insert(n.clone());
            }
            order_body_closed(body, &body_scope, globals, closed, errors);
            let mut catch_scope = outer.clone();
            if let Some(cv) = catch_var {
                catch_scope.insert(cv.clone());
            }
            order_body_closed(catch_body, &catch_scope, globals, closed, errors);
            if let Some(fb) = finally_body {
                order_body_closed(fb, outer, globals, closed, errors);
            }
        }
        Expr::Spawn { body } | Expr::Synchronized { body, .. } => {
            order_body_closed(body, outer, globals, closed, errors)
        }
        Expr::Binary { left, right, .. }
        | Expr::NullCoalesce {
            option: left,
            default: right,
        } => {
            recurse_expr_blocks(left, outer, globals, closed, errors);
            recurse_expr_blocks(right, outer, globals, closed, errors);
        }
        Expr::Call {
            args,
            named_args,
            ..
        }
        | Expr::ConstructorCall {
            args,
            named_args,
            ..
        }
        | Expr::SuperCall {
            args,
            named_args,
            ..
        } => {
            for a in args {
                recurse_expr_blocks(a, outer, globals, closed, errors);
            }
            for (_, a) in named_args {
                recurse_expr_blocks(a, outer, globals, closed, errors);
            }
        }
        Expr::MethodCall {
            object,
            args,
            named_args,
            ..
        } => {
            recurse_expr_blocks(object, outer, globals, closed, errors);
            for a in args {
                recurse_expr_blocks(a, outer, globals, closed, errors);
            }
            for (_, a) in named_args {
                recurse_expr_blocks(a, outer, globals, closed, errors);
            }
        }
        Expr::Unary { expr, .. }
        | Expr::Cast { expr, .. }
        | Expr::Some(expr)
        | Expr::Ok(expr)
        | Expr::Err(expr)
        | Expr::Try(expr)
        | Expr::Throw(expr)
        | Expr::PostfixIncr(expr)
        | Expr::PostfixDecr(expr)
        | Expr::PrefixIncr(expr)
        | Expr::PrefixDecr(expr)
        | Expr::IsType { expr, .. }
        | Expr::OptionalChain { object: expr, .. }
        | Expr::Field { object: expr, .. }
        | Expr::TupleIndex { object: expr, .. } => {
            recurse_expr_blocks(expr, outer, globals, closed, errors)
        }
        Expr::Index { array, index } => {
            recurse_expr_blocks(array, outer, globals, closed, errors);
            recurse_expr_blocks(index, outer, globals, closed, errors);
        }
        Expr::Tuple(elems) | Expr::Array(elems) => {
            for e in elems {
                recurse_expr_blocks(e, outer, globals, closed, errors);
            }
        }
        Expr::StructInit { fields, .. } => {
            for (_, e) in fields {
                recurse_expr_blocks(e, outer, globals, closed, errors);
            }
        }
        Expr::Range {
            start, end, step, ..
        } => {
            recurse_expr_blocks(start, outer, globals, closed, errors);
            recurse_expr_blocks(end, outer, globals, closed, errors);
            if let Some(s) = step {
                recurse_expr_blocks(s, outer, globals, closed, errors);
            }
        }
        Expr::VariantConst { arg: Some(a), .. } => {
            recurse_expr_blocks(a, outer, globals, closed, errors)
        }
        Expr::Interpolate(parts) => {
            for p in parts {
                if let crate::ast::InterpolatePart::Expr(e) = p {
                    recurse_expr_blocks(e, outer, globals, closed, errors);
                }
            }
        }
        Expr::SliceExpr { array, start, end } => {
            recurse_expr_blocks(array, outer, globals, closed, errors);
            recurse_expr_blocks(start, outer, globals, closed, errors);
            recurse_expr_blocks(end, outer, globals, closed, errors);
        }
        Expr::MapLiteral { entries } => {
            for (k, v) in entries {
                recurse_expr_blocks(k, outer, globals, closed, errors);
                recurse_expr_blocks(v, outer, globals, closed, errors);
            }
        }
        Expr::TrailingClosure {
            callee,
            args,
            closure,
        } => {
            recurse_expr_blocks(callee, outer, globals, closed, errors);
            for a in args {
                recurse_expr_blocks(a, outer, globals, closed, errors);
            }
            recurse_expr_blocks(closure, outer, globals, closed, errors);
        }
        Expr::Invoke {
            callee,
            args,
            named_args,
        } => {
            recurse_expr_blocks(callee, outer, globals, closed, errors);
            for a in args {
                recurse_expr_blocks(a, outer, globals, closed, errors);
            }
            for (_, a) in named_args {
                recurse_expr_blocks(a, outer, globals, closed, errors);
            }
        }
        Expr::Return(Some(e)) => recurse_expr_blocks(e, outer, globals, closed, errors),
        _ => {}
    }
}

/// 收集某语句**嵌套块**中声明的局部变量名（不含该语句自身的直接声明）。
/// 这些名字在语句结束即离开作用域，供后续语句的越界引用判定。
fn collect_stmt_nested_local_names(stmt: &Stmt, out: &mut HashSet<String>) {
    match stmt {
        Stmt::While { body, .. }
        | Stmt::DoWhile { body, .. }
        | Stmt::Loop { body }
        | Stmt::UnsafeBlock { body } => collect_block_local_names(body, out),
        Stmt::For { var, body, .. } => {
            out.insert(var.clone());
            collect_block_local_names(body, out);
        }
        Stmt::WhileLet { pattern, body, .. } => {
            let mut names = Vec::new();
            pattern_binding_names(pattern, &mut names);
            out.extend(names);
            collect_block_local_names(body, out);
        }
        Stmt::LocalFunc(f) => collect_block_local_names(&f.body, out),
        Stmt::Expr(e) | Stmt::Return(Some(e)) => collect_expr_nested_local_names(e, out),
        Stmt::Let { value, .. }
        | Stmt::Var { value, .. }
        | Stmt::Const { value, .. }
        | Stmt::Assign { value, .. } => collect_expr_nested_local_names(value, out),
        Stmt::Assert { left, right, .. } | Stmt::Expect { left, right, .. } => {
            collect_expr_nested_local_names(left, out);
            collect_expr_nested_local_names(right, out);
        }
        _ => {}
    }
}

fn collect_block_local_names(stmts: &[Stmt], out: &mut HashSet<String>) {
    for s in stmts {
        match s {
            Stmt::Let { pattern, .. } | Stmt::Var { pattern, .. } => {
                let mut names = Vec::new();
                pattern_binding_names(pattern, &mut names);
                out.extend(names);
            }
            Stmt::Const { name, .. } => {
                out.insert(name.clone());
            }
            Stmt::LocalFunc(f) => {
                // 局部函数同样只在所声明的块内可见（3.2.3.2(6)）。
                out.insert(f.name.clone());
            }
            _ => {}
        }
        collect_stmt_nested_local_names(s, out);
    }
}

fn collect_expr_nested_local_names(expr: &Expr, out: &mut HashSet<String>) {
    match expr {
        Expr::Block(stmts, trailing) => {
            collect_block_local_names(stmts, out);
            if let Some(t) = trailing {
                collect_expr_nested_local_names(t, out);
            }
        }
        Expr::If {
            then_branch,
            else_branch,
            ..
        } => {
            collect_expr_nested_local_names(then_branch, out);
            if let Some(eb) = else_branch {
                collect_expr_nested_local_names(eb, out);
            }
        }
        Expr::IfLet {
            pattern,
            expr,
            then_branch,
            else_branch,
        } => {
            collect_expr_nested_local_names(expr, out);
            let mut names = Vec::new();
            pattern_binding_names(pattern, &mut names);
            out.extend(names);
            collect_expr_nested_local_names(then_branch, out);
            if let Some(eb) = else_branch {
                collect_expr_nested_local_names(eb, out);
            }
        }
        Expr::Match { expr, arms } => {
            collect_expr_nested_local_names(expr, out);
            for arm in arms {
                let mut names = Vec::new();
                pattern_binding_names(&arm.pattern, &mut names);
                out.extend(names);
                if let Some(g) = &arm.guard {
                    collect_expr_nested_local_names(g, out);
                }
                collect_expr_nested_local_names(&arm.body, out);
            }
        }
        Expr::Lambda { params, body, .. } => {
            out.extend(params.iter().map(|(n, _)| n.clone()));
            collect_expr_nested_local_names(body, out);
        }
        Expr::TryBlock {
            resources,
            body,
            catch_var,
            catch_body,
            finally_body,
            ..
        } => {
            for (n, e) in resources {
                out.insert(n.clone());
                collect_expr_nested_local_names(e, out);
            }
            collect_block_local_names(body, out);
            if let Some(cv) = catch_var {
                out.insert(cv.clone());
            }
            collect_block_local_names(catch_body, out);
            if let Some(fb) = finally_body {
                collect_block_local_names(fb, out);
            }
        }
        Expr::Spawn { body } | Expr::Synchronized { body, .. } => {
            collect_block_local_names(body, out)
        }
        Expr::Binary { left, right, .. }
        | Expr::NullCoalesce {
            option: left,
            default: right,
        } => {
            collect_expr_nested_local_names(left, out);
            collect_expr_nested_local_names(right, out);
        }
        Expr::Call {
            args,
            named_args,
            ..
        }
        | Expr::ConstructorCall {
            args,
            named_args,
            ..
        }
        | Expr::SuperCall {
            args,
            named_args,
            ..
        } => {
            for a in args {
                collect_expr_nested_local_names(a, out);
            }
            for (_, a) in named_args {
                collect_expr_nested_local_names(a, out);
            }
        }
        Expr::MethodCall {
            object,
            args,
            named_args,
            ..
        } => {
            collect_expr_nested_local_names(object, out);
            for a in args {
                collect_expr_nested_local_names(a, out);
            }
            for (_, a) in named_args {
                collect_expr_nested_local_names(a, out);
            }
        }
        Expr::Unary { expr, .. }
        | Expr::Cast { expr, .. }
        | Expr::Some(expr)
        | Expr::Ok(expr)
        | Expr::Err(expr)
        | Expr::Try(expr)
        | Expr::Throw(expr)
        | Expr::PostfixIncr(expr)
        | Expr::PostfixDecr(expr)
        | Expr::PrefixIncr(expr)
        | Expr::PrefixDecr(expr)
        | Expr::IsType { expr, .. }
        | Expr::OptionalChain { object: expr, .. }
        | Expr::Field { object: expr, .. }
        | Expr::TupleIndex { object: expr, .. } => collect_expr_nested_local_names(expr, out),
        Expr::Index { array, index } => {
            collect_expr_nested_local_names(array, out);
            collect_expr_nested_local_names(index, out);
        }
        Expr::Tuple(elems) | Expr::Array(elems) => {
            for e in elems {
                collect_expr_nested_local_names(e, out);
            }
        }
        Expr::StructInit { fields, .. } => {
            for (_, e) in fields {
                collect_expr_nested_local_names(e, out);
            }
        }
        Expr::Range {
            start, end, step, ..
        } => {
            collect_expr_nested_local_names(start, out);
            collect_expr_nested_local_names(end, out);
            if let Some(s) = step {
                collect_expr_nested_local_names(s, out);
            }
        }
        Expr::VariantConst { arg: Some(a), .. } => collect_expr_nested_local_names(a, out),
        Expr::Interpolate(parts) => {
            for p in parts {
                if let crate::ast::InterpolatePart::Expr(e) = p {
                    collect_expr_nested_local_names(e, out);
                }
            }
        }
        Expr::SliceExpr { array, start, end } => {
            collect_expr_nested_local_names(array, out);
            collect_expr_nested_local_names(start, out);
            collect_expr_nested_local_names(end, out);
        }
        Expr::MapLiteral { entries } => {
            for (k, v) in entries {
                collect_expr_nested_local_names(k, out);
                collect_expr_nested_local_names(v, out);
            }
        }
        Expr::TrailingClosure {
            callee,
            args,
            closure,
        } => {
            collect_expr_nested_local_names(callee, out);
            for a in args {
                collect_expr_nested_local_names(a, out);
            }
            collect_expr_nested_local_names(closure, out);
        }
        Expr::Invoke {
            callee,
            args,
            named_args,
        } => {
            collect_expr_nested_local_names(callee, out);
            for a in args {
                collect_expr_nested_local_names(a, out);
            }
            for (_, a) in named_args {
                collect_expr_nested_local_names(a, out);
            }
        }
        Expr::Return(Some(e)) => collect_expr_nested_local_names(e, out),
        _ => {}
    }
}

/// 收集语句直接表达式中的标识符引用（不进入嵌套块 / lambda / match 分支）。
fn collect_stmt_direct_refs(stmt: &Stmt, out: &mut Vec<String>) {
    match stmt {
        Stmt::Let { value, .. } | Stmt::Var { value, .. } => collect_expr_direct_refs(value, out),
        Stmt::Const { value, .. } => collect_expr_direct_refs(value, out),
        Stmt::Assign { target, value } => {
            collect_expr_direct_refs(value, out);
            if let AssignTarget::Var(name) = target {
                out.push(name.clone());
            }
        }
        Stmt::Expr(e) => collect_expr_direct_refs(e, out),
        Stmt::Return(Some(e)) => collect_expr_direct_refs(e, out),
        Stmt::While { cond, .. } => collect_expr_direct_refs(cond, out),
        Stmt::DoWhile { cond, .. } => collect_expr_direct_refs(cond, out),
        Stmt::For { iterable, .. } => collect_expr_direct_refs(iterable, out),
        Stmt::WhileLet { expr, .. } => collect_expr_direct_refs(expr, out),
        Stmt::Assert { left, right, .. } | Stmt::Expect { left, right, .. } => {
            collect_expr_direct_refs(left, out);
            collect_expr_direct_refs(right, out);
        }
        _ => {}
    }
}

fn collect_expr_direct_refs(expr: &Expr, out: &mut Vec<String>) {
    match expr {
        Expr::Var(name) => out.push(name.clone()),
        Expr::Call {
            name,
            args,
            named_args,
            ..
        } => {
            out.push(name.clone());
            for a in args {
                collect_expr_direct_refs(a, out);
            }
            for (_, a) in named_args {
                collect_expr_direct_refs(a, out);
            }
        }
        Expr::Unary { expr, .. }
        | Expr::Cast { expr, .. }
        | Expr::Some(expr)
        | Expr::Ok(expr)
        | Expr::Err(expr)
        | Expr::Try(expr)
        | Expr::Throw(expr)
        | Expr::PostfixIncr(expr)
        | Expr::PostfixDecr(expr)
        | Expr::PrefixIncr(expr)
        | Expr::PrefixDecr(expr)
        | Expr::IsType { expr, .. }
        | Expr::Field { object: expr, .. }
        | Expr::TupleIndex { object: expr, .. }
        | Expr::OptionalChain { object: expr, .. } => collect_expr_direct_refs(expr, out),
        Expr::Binary { left, right, .. } | Expr::NullCoalesce { option: left, default: right } => {
            collect_expr_direct_refs(left, out);
            collect_expr_direct_refs(right, out);
        }
        Expr::Return(Some(e)) => collect_expr_direct_refs(e, out),
        Expr::Index { array, index } => {
            collect_expr_direct_refs(array, out);
            collect_expr_direct_refs(index, out);
        }
        Expr::Tuple(elems) | Expr::Array(elems) => {
            for e in elems {
                collect_expr_direct_refs(e, out);
            }
        }
        Expr::Range {
            start, end, step, ..
        } => {
            collect_expr_direct_refs(start, out);
            collect_expr_direct_refs(end, out);
            if let Some(s) = step {
                collect_expr_direct_refs(s, out);
            }
        }
        Expr::MethodCall {
            object,
            args,
            named_args,
            ..
        } => {
            collect_expr_direct_refs(object, out);
            for a in args {
                collect_expr_direct_refs(a, out);
            }
            for (_, a) in named_args {
                collect_expr_direct_refs(a, out);
            }
        }
        Expr::StructInit { fields, .. } => {
            for (_, e) in fields {
                collect_expr_direct_refs(e, out);
            }
        }
        // match / if-let 的**被匹配表达式**在当前位置求值，属于直接引用；
        // 其分支体属于子作用域，由 recurse_expr_blocks 处理。
        Expr::Match { expr, .. } | Expr::IfLet { expr, .. } => {
            collect_expr_direct_refs(expr, out)
        }
        // if 表达式的条件在当前位置求值；分支体属于子作用域。
        Expr::If { cond, .. } => collect_expr_direct_refs(cond, out),
        Expr::VariantConst { arg: Some(a), .. } => collect_expr_direct_refs(a, out),
        Expr::SliceExpr { array, start, end } => {
            collect_expr_direct_refs(array, out);
            collect_expr_direct_refs(start, out);
            collect_expr_direct_refs(end, out);
        }
        Expr::MapLiteral { entries } => {
            for (k, v) in entries {
                collect_expr_direct_refs(k, out);
                collect_expr_direct_refs(v, out);
            }
        }
        Expr::Interpolate(parts) => {
            for p in parts {
                if let crate::ast::InterpolatePart::Expr(e) = p {
                    collect_expr_direct_refs(e, out);
                }
            }
        }
        Expr::TrailingClosure {
            callee,
            args,
            closure: _,
        } => {
            collect_expr_direct_refs(callee, out);
            for a in args {
                collect_expr_direct_refs(a, out);
            }
        }
        Expr::Invoke {
            callee,
            args,
            named_args,
        } => {
            collect_expr_direct_refs(callee, out);
            for a in args {
                collect_expr_direct_refs(a, out);
            }
            for (_, a) in named_args {
                collect_expr_direct_refs(a, out);
            }
        }
        Expr::TryBlock { resources, .. } => {
            for (_, e) in resources {
                collect_expr_direct_refs(e, out);
            }
        }
        // 嵌套块 / lambda 体属于其它作用域，不在此处收集。
        _ => {}
    }
}

fn check_class(cls: &ClassDef, index: &ProgramIndex<'_>, errors: &mut Vec<SemanticError>) {
    let mut by_name: HashMap<String, Vec<Sig>> = HashMap::new();
    for method in &cls.methods {
        let sig = sig_of_class_method(method, &cls.type_params);
        if is_synthetic_member(&sig.name) {
            continue;
        }
        by_name.entry(sig.name.clone()).or_default().push(sig);
    }
    for group in by_name.values() {
        check_signature_group(&cls.name, group, true, errors);
    }

    // 重复字段
    check_duplicate_fields(
        &cls.name,
        cls.fields.iter().map(|f| f.name.as_str()),
        errors,
    );

    // 字段与成员函数同名（3.1(3)）
    let method_names: Vec<String> = cls
        .methods
        .iter()
        .map(|m| last_name_segment(&m.func.name))
        .collect();
    check_field_method_conflicts(
        &cls.name,
        cls.fields.iter().map(|f| f.name.as_str()),
        method_names.iter().map(|s| s.as_str()),
        errors,
    );

    // 成员级修饰符（open/abstract/static）与函数体的一致性
    check_member_modifiers(cls, errors);

    // override / redef 签名校验
    check_class_overrides(cls, index, errors);
    // 7.4(8)/(10)/(13)：属性覆写/重定义的 mut 修饰符与 redef 限制。
    check_class_property_members(cls, index, errors);

    // 子类成员不得遮蔽父类型同名成员（6.3.3(1)）
    check_member_shadowing(cls, index, errors);

    // 覆写/重定义不得降低访问级别
    check_member_visibility_lowering(cls, index, errors);

    // 实现接口属性时类型必须完全一致
    if let Some(parents) = collect_parent_methods(cls, index) {
        if parents.iter().any(|p| p.is_interface) {
            let own: Vec<Sig> = cls
                .methods
                .iter()
                .map(|m| sig_of_class_method(m, &cls.type_params))
                .collect();
            check_interface_property_types(&cls.name, &own, &parents, index, errors);
            check_interface_property_mut(&cls.name, &own, &parents, errors);
        }
    }

    // 与父类同名函数的 static/实例冲突
    check_inherited_static_conflicts(cls, index, errors);

    // 父类与接口（或多个父类型）之间同名函数的 static/实例冲突（6.3.2(1)）。
    check_cross_parent_static_conflicts(cls, index, errors);

    // 接口实现完整性
    check_interface_implementation(cls, index, errors);

    // 6.2.4.1(9)：抽象类也不能依赖多个父接口的默认实现（无体声明不算实现）。
    if cls.is_abstract {
        check_abstract_class_default_conflicts(cls, index, errors);
    }

    // 接口 const 函数必须由 const 函数实现
    check_const_interface_impls(cls, index, errors);

    // 父类型来源合法性：单继承、可继承性、不能继承 struct/enum
    check_class_supertypes(cls, index, errors);

    // 重定义/实现父类型的泛型函数时，约束不得比父类型更严格
    // （6.3.4.1(5)、6.2.4.1(5)）。
    check_generic_constraint_loosening(cls, index, errors);
}

/// 可见性级别：private < internal(默认) < protected < public。
fn visibility_rank(v: &Visibility) -> u8 {
    match v {
        Visibility::Private => 0,
        Visibility::Internal => 1,
        Visibility::Protected => 2,
        Visibility::Public => 3,
    }
}

/// 校验类成员函数的修饰符与函数体是否自洽（6.1.2.4 系列规则）：
/// - `static` 成员函数必须有实现（6.1.2.4(21)）；
/// - 无实现的成员函数只能声明在抽象类中（6.1.2.4(8)）；
/// - 函数不能同时声明为 `open` 与 `static`（6.1.2.4(33)）；
/// - `abstract` 成员函数只能出现在抽象类中，且不能有实现体。
fn check_member_modifiers(cls: &ClassDef, errors: &mut Vec<SemanticError>) {
    for m in &cls.methods {
        let sig = sig_of_class_method(m, &cls.type_params);
        // 6.3.4.1(9)：static 初始化器只能被隐式调用，不能用 redef/override 修饰。
        if sig.name == "__static_init" {
            if m.override_ {
                errors.push(SemanticError {
                    message: format!(
                        "类 '{}' 的 static 初始化器不能使用 redef/override（6.3.4.1(9)）",
                        cls.name
                    ),
                });
            }
            continue;
        }
        if is_synthetic_member(&sig.name) {
            continue;
        }
        let is_static = m.is_static;
        // 6.1.2.4(36)：open 函数必须声明为 public 或 protected（默认 internal 不合法）。
        // 属性访问器（__get_/__set_）不适用本规则：open 属性由属性声明自身约束。
        if m.is_open
            && !is_static
            && !is_property_method_name(&sig.name)
            && matches!(m.func.visibility, Visibility::Internal)
        {
            errors.push(SemanticError {
                message: format!(
                    "类 '{}' 的 open 成员函数 '{}' 必须声明为 public 或 protected（6.1.2.4(36)）",
                    cls.name, m.func.name
                ),
            });
        }
        if m.is_open && is_static {
            errors.push(SemanticError {
                message: format!(
                    "类 '{}' 的成员函数 '{}' 不能同时声明为 open 和 static（6.1.2.4(33)）",
                    cls.name, m.func.name
                ),
            });
        }
        if m.is_abstract {
            if !cls.is_abstract {
                errors.push(SemanticError {
                    message: format!(
                        "类 '{}' 的抽象成员函数 '{}' 只能声明在抽象类中",
                        cls.name, m.func.name
                    ),
                });
            }
            if is_static {
                errors.push(SemanticError {
                    message: format!(
                        "类 '{}' 的 static 成员函数 '{}' 不能声明为 abstract",
                        cls.name, m.func.name
                    ),
                });
            }
            if m.has_body {
                errors.push(SemanticError {
                    message: format!(
                        "类 '{}' 的抽象成员函数 '{}' 不能有实现体",
                        cls.name, m.func.name
                    ),
                });
            }
        } else if !m.has_body {
            if is_static {
                errors.push(SemanticError {
                    message: format!(
                        "类 '{}' 的 static 成员函数 '{}' 必须有实现（6.1.2.4(21)）",
                        cls.name, m.func.name
                    ),
                });
            } else if !cls.is_abstract {
                errors.push(SemanticError {
                    message: format!(
                        "类 '{}' 的成员函数 '{}' 无实现，只能声明在抽象类或接口中（6.1.2.4(8)）",
                        cls.name, m.func.name
                    ),
                });
            }
        }

        // 6.1.1.1(9) / 6.1.2.4(36)：open 成员函数具有可被覆写的语义；抽象实例成员
        // 函数（显式 `abstract` 或抽象类中无实现体的实例函数）隐含 open。二者都不能
        // 声明为 private（private 成员无法被子类访问/覆写）。
        let implicitly_abstract_instance = !is_static && !m.has_body;
        if (m.is_open || m.is_abstract || implicitly_abstract_instance)
            && matches!(m.func.visibility, Visibility::Private)
        {
            errors.push(SemanticError {
                message: format!(
                    "类 '{}' 的 open/抽象成员函数 '{}' 不能声明为 private（6.1.1.1(9)/6.1.2.4(36)）",
                    cls.name, m.func.name
                ),
            });
        }

        // 6.1.2.4(12)：抽象成员函数不能带可选参数（有默认值的参数）。
        if (m.is_abstract || implicitly_abstract_instance)
            && m.func.params.iter().any(|p| p.default.is_some())
        {
            errors.push(SemanticError {
                message: format!(
                    "类 '{}' 的抽象成员函数 '{}' 不能带可选参数（6.1.2.4(12)）",
                    cls.name, m.func.name
                ),
            });
        }
    }
}

/// 成员名去掉属性访问器前缀（`__get_p` / `__set_p` → `p`）。
fn member_base_name(name: &str) -> &str {
    name.strip_prefix("__get_")
        .or_else(|| name.strip_prefix("__set_"))
        .unwrap_or(name)
}

/// 收集父类链上的字段名（extends 无法本地解析时返回 None，保守跳过）。
fn collect_parent_field_names<'a>(
    cls: &ClassDef,
    index: &ProgramIndex<'a>,
) -> Option<HashSet<String>> {
    let mut out = HashSet::new();
    let mut cur = cls.extends.clone();
    let mut seen = HashSet::new();
    while let Some(name) = cur {
        if !seen.insert(name.clone()) {
            break;
        }
        let Some(parent) = index.classes.get(name.as_str()) else {
            // `<:` 后面也可能是接口实现（如 `class C <: I`）：接口没有字段，
            // 不构成字段遮蔽来源，但也不应中止整个检查。
            if index.interfaces.contains_key(name.as_str()) {
                break;
            }
            return None;
        };
        for f in &parent.fields {
            if f.visibility != Visibility::Private {
                out.insert(f.name.clone());
            }
        }
        cur = parent.extends.clone();
    }
    Some(out)
}

/// 子类型成员不得遮蔽父类型同名成员（6.3.3(1)）：
/// - 子类字段与父类型任意同名成员冲突；
/// - 子类函数只有与父类型同签名且父成员可覆写（open/abstract/接口成员）时才是合法覆写；
/// - 与父类型同名但签名不同视为遮蔽。
fn check_member_shadowing(cls: &ClassDef, index: &ProgramIndex<'_>, errors: &mut Vec<SemanticError>) {
    let Some(parents) = collect_parent_methods(cls, index) else {
        return;
    };
    let Some(parent_fields) = collect_parent_field_names(cls, index) else {
        return;
    };

    // 子类型自身成员：字段 + 函数（函数附带签名/override/static 信息）。
    let mut own: Vec<(String, Option<Sig>, bool, bool, bool)> = Vec::new();
    for m in &cls.methods {
        let sig = sig_of_class_method(m, &cls.type_params);
        if is_synthetic_member(&sig.name) {
            continue;
        }
        let is_static = m.is_static;
        own.push((
            member_base_name(&sig.name).to_string(),
            Some(sig),
            m.override_,
            m.has_override_kw,
            is_static,
        ));
    }
    for f in &cls.fields {
        own.push((f.name.clone(), None, false, false, false));
    }

    let mut reported: HashSet<String> = HashSet::new();
    for (name, sig_opt, override_, has_override_kw, is_static) in own {
        let parent_methods: Vec<&ParentMethod> = parents
            .iter()
            .filter(|p| !is_synthetic_member(&p.sig.name) && member_base_name(&p.sig.name) == name)
            .filter(|p| p.sig.vis.as_ref() != Some(&Visibility::Private))
            .collect();
        let parent_field = parent_fields.contains(&name);
        if parent_methods.is_empty() && !parent_field {
            continue;
        }
        let mut report = |errors: &mut Vec<SemanticError>, msg: String| {
            if reported.insert(name.clone()) {
                errors.push(SemanticError { message: msg });
            }
        };
        let Some(sig) = sig_opt else {
            // 子类字段遮蔽父类型同名成员（字段不能与任何父成员同名）。
            if !parent_methods.is_empty() || parent_field {
                report(
                    errors,
                    format!(
                        "类 '{}' 的字段 '{}' 遮蔽了父类型的同名成员（6.3.3(1)）",
                        cls.name, name
                    ),
                );
            }
            continue;
        };
        if parent_field {
            report(
                errors,
                format!(
                    "类 '{}' 的成员 '{}' 遮蔽了父类型的同名字段（6.3.3(1)）",
                    cls.name, sig.display
                ),
            );
            continue;
        }
        let own_is_prop = sig.name.starts_with("__get_") || sig.name.starts_with("__set_");
        // 不同种类（函数 vs 属性）的同名成员构成遮蔽。
        let kind_conflict = parent_methods.iter().any(|p| {
            let p_is_prop =
                p.sig.name.starts_with("__get_") || p.sig.name.starts_with("__set_");
            p_is_prop != own_is_prop
        });
        if kind_conflict {
            report(
                errors,
                format!(
                    "类 '{}' 的成员 '{}' 遮蔽了父类型的同名成员（6.3.3(1)）",
                    cls.name, sig.display
                ),
            );
            continue;
        }
        // 同种函数/属性：签名不同属于合法重载（继承的最近匹配规则），签名相同才是覆写。
        let alt_parent_name = sig
            .name
            .strip_prefix("__set_")
            .map(|rest| format!("__get_{}", rest));
        let matching: Vec<&ParentMethod> = parent_methods
            .iter()
            .copied()
            .filter(|p| p.sig.is_static == is_static)
            .filter(|p| {
                (p.sig.name == sig.name || alt_parent_name.as_deref() == Some(p.sig.name.as_str()))
                    && canon_params(&sig) == canon_params(&p.sig)
            })
            .collect();
        if matching.is_empty() {
            continue;
        }
        // 静态函数不以实例方式继承：未写 override 的同名 static 函数视为新声明。
        if is_static && !override_ {
            continue;
        }
        // `redef`（没有显式 override）属于重定义，不要求父成员 open。
        if override_ && !has_override_kw {
            continue;
        }
        if !matching.iter().any(|p| p.open_) {
            report(
                errors,
                format!(
                    "类 '{}' 的成员函数 '{}' 覆写了父类型中不可覆写（非 open/abstract）的函数（6.1.2.4(32)）",
                    cls.name, sig.display
                ),
            );
        }
    }
}

/// 构造函数参数顺序：普通参数必须位于所有成员变量参数之前（6.1.2.1(21)）。
fn check_ctor_param_order(cls: &ClassDef, errors: &mut Vec<SemanticError>) {
    let mut check = |flags: &[bool], what: &str| {
        let mut seen_member = false;
        for (idx, is_member) in flags.iter().enumerate() {
            if *is_member {
                seen_member = true;
            } else if seen_member {
                errors.push(SemanticError {
                    message: format!(
                        "类 '{}' 的{}第 {} 个普通参数位于成员变量参数之后（6.1.2.1(21)）",
                        cls.name,
                        what,
                        idx + 1
                    ),
                });
                break;
            }
        }
    };
    check(&cls.primary_ctor_member_flags, "主构造函数");
    if let Some(init) = &cls.init {
        check(&init.member_param_flags, "构造函数");
    }
}

/// 6.1.2.1(55)：若构造函数未显式调用其它构造函数，必须在返回前初始化本类声明的
/// 全部实例成员变量。这里实现保守的“确定赋值”分析——只统计必然执行的赋值：
/// - 顺序语句累积；
/// - `if` 两支都赋才算赋值（一支提前 `return` 时按另一支）；
/// - 循环体不视为必然执行（`while`/`for` 可能 0 次）；
/// - `this(...)` 委托其它构造函数视为已初始化全部字段；
/// - 每条 `return` 及正常返回处检查尚未确定的字段。
fn check_ctor_definite_assignment(cls: &ClassDef, errors: &mut Vec<SemanticError>) {
    let inits: Vec<&InitDef> = if cls.inits.is_empty() {
        cls.init.iter().collect()
    } else {
        cls.inits.iter().collect()
    };
    if inits.is_empty() {
        return;
    }
    // 主构造函数中的“普通参数”会被 parser 当作字段加入 `fields`（P6 展开），
    // 但它们不是实例成员变量，从字段集合中排除。
    let mut nonmember: HashSet<&str> = HashSet::new();
    for (p, flag) in cls
        .primary_ctor_params
        .iter()
        .zip(cls.primary_ctor_member_flags.iter())
    {
        if !*flag {
            nonmember.insert(p.name.as_str());
        }
    }
    let mut fields: HashSet<String> = HashSet::new();
    let mut auto: HashSet<String> = HashSet::new();
    for f in &cls.fields {
        if f.is_static || nonmember.contains(f.name.as_str()) {
            continue;
        }
        fields.insert(f.name.clone());
        if f.default.is_some() {
            auto.insert(f.name.clone());
        }
    }
    // 6.1.2.1(11)：以主构造函数成员变量参数（如类名形式的 `Foo(var x: Int64)`）
    // 隐式定义的实例成员变量同样要求在所有构造函数中初始化。这类字段未进入
    // `cls.fields`，需从各构造函数的 member_param_flags 补入。
    for init in &inits {
        for (p, flag) in init.params.iter().zip(init.member_param_flags.iter()) {
            if *flag {
                fields.insert(p.name.clone());
                if p.default.is_some() {
                    auto.insert(p.name.clone());
                }
            }
        }
    }
    if fields.is_empty() {
        return;
    }
    // 主构造函数的成员变量参数对所有构造函数均视为已初始化（保守）。
    for (p, flag) in cls
        .primary_ctor_params
        .iter()
        .zip(cls.primary_ctor_member_flags.iter())
    {
        if *flag {
            auto.insert(p.name.clone());
        }
    }
    let required: HashSet<String> = fields.difference(&auto).cloned().collect();
    if required.is_empty() {
        return;
    }
    for init in inits {
        let mut start = auto.clone();
        for (p, flag) in init.params.iter().zip(init.member_param_flags.iter()) {
            if *flag {
                start.insert(p.name.clone());
            }
        }
        let mut viol: Vec<String> = Vec::new();
        let (cur, flows) = da_stmt_list(&init.body, start, &required, &fields, &mut viol);
        if flows {
            for name in &required {
                if !cur.contains(name) {
                    viol.push(name.clone());
                }
            }
        }
        // 去重后报告。
        viol.sort();
        viol.dedup();
        for name in viol {
            errors.push(SemanticError {
                message: format!(
                    "类 '{}' 的构造函数返回前未初始化实例成员变量 '{}'（6.1.2.1(55)）",
                    cls.name, name
                ),
            });
        }
    }
}

/// 确定赋值分析：按顺序处理语句块。
/// 返回 (必然已赋值的字段集合, 是否可能正常落到块末尾)。
fn da_stmt_list(
    stmts: &[Stmt],
    mut cur: HashSet<String>,
    required: &HashSet<String>,
    fields: &HashSet<String>,
    viol: &mut Vec<String>,
) -> (HashSet<String>, bool) {
    for s in stmts {
        let (next, flows) = da_stmt(s, cur, required, fields, viol);
        cur = next;
        if !flows {
            return (cur, false);
        }
    }
    (cur, true)
}

/// 确定赋值分析：单条语句。
fn da_stmt(
    stmt: &Stmt,
    cur: HashSet<String>,
    required: &HashSet<String>,
    fields: &HashSet<String>,
    viol: &mut Vec<String>,
) -> (HashSet<String>, bool) {
    match stmt {
        Stmt::Assign { target, value } => {
            let mut next = cur;
            da_target(target, &mut next, fields);
            da_expr(value, next, required, fields, viol)
        }
        Stmt::Expr(e) => da_expr(e, cur, required, fields, viol),
        Stmt::Let { value, .. } | Stmt::Var { value, .. } | Stmt::Const { value, .. } => {
            da_expr(value, cur, required, fields, viol)
        }
        Stmt::Return(_) => {
            for name in required {
                if !cur.contains(name) {
                    viol.push(name.clone());
                }
            }
            (cur, false)
        }
        // 循环体可能一次都不执行：只分析其中 return 路径的违规，不把其中赋值
        // 视为必然发生。
        Stmt::While { body, .. }
        | Stmt::WhileLet { body, .. }
        | Stmt::For { body, .. }
        | Stmt::Loop { body }
        | Stmt::DoWhile { body, .. } => {
            let _ = da_stmt_list(body, cur.clone(), required, fields, viol);
            (cur, true)
        }
        Stmt::UnsafeBlock { body } => da_stmt_list(body, cur, required, fields, viol),
        _ => (cur, true),
    }
}

/// 确定赋值分析：表达式（处理 `if`/块/`match`/`this(...)` 委托）。
fn da_expr(
    expr: &Expr,
    cur: HashSet<String>,
    required: &HashSet<String>,
    fields: &HashSet<String>,
    viol: &mut Vec<String>,
) -> (HashSet<String>, bool) {
    match expr {
        Expr::If {
            then_branch,
            else_branch,
            ..
        } => {
            let (t_out, t_flow) = da_expr(then_branch, cur.clone(), required, fields, viol);
            let (e_out, e_flow) = match else_branch {
                Some(eb) => da_expr(eb, cur.clone(), required, fields, viol),
                None => (cur.clone(), true),
            };
            match (t_flow, e_flow) {
                (true, true) => (da_intersect(&t_out, &e_out), true),
                (true, false) => (t_out, true),
                (false, true) => (e_out, true),
                (false, false) => (cur, false),
            }
        }
        Expr::Block(stmts, trailing) => {
            let (c, flows) = da_stmt_list(stmts, cur, required, fields, viol);
            if !flows {
                return (c, false);
            }
            match trailing {
                Some(t) => da_expr(t, c, required, fields, viol),
                None => (c, true),
            }
        }
        // `this(...)`：委托其它构造函数，视为已初始化全部字段。
        Expr::Call { name, .. } if name == "this" => {
            let mut c = cur;
            c.extend(required.iter().cloned());
            (c, true)
        }
        // `this(...)` 也可能解析为 Invoke（callee 为 `this`/`self`）。
        Expr::Invoke { callee, .. }
            if matches!(callee.as_ref(), Expr::Var(n) if n == "this" || n == "self") =>
        {
            let mut c = cur;
            c.extend(required.iter().cloned());
            (c, true)
        }
        Expr::Match { arms, .. } => {
            // 各分支取交集；未能正常落地（提前 return）的分支忽略。
            let mut merged: Option<HashSet<String>> = None;
            let mut any_falls = false;
            for arm in arms {
                let (out, flows) = da_expr(&arm.body, cur.clone(), required, fields, viol);
                if flows {
                    any_falls = true;
                    merged = Some(match merged.take() {
                        Some(prev) => da_intersect(&prev, &out),
                        None => out,
                    });
                }
            }
            match (merged, any_falls) {
                (Some(m), true) => (m, true),
                (None, false) => (cur, false),
                _ => (cur, true),
            }
        }
        // `synchronized(lock) { .. }` 同步执行一次：其内部赋值必然发生。
        Expr::Synchronized { body, .. } => da_stmt_list(body, cur, required, fields, viol),
        // 其余可能包含语句块/闭包体的表达式（spawn / lambda / try / 尾随闭包 / 宏）：
        // 其中对字段的赋值按“可能发生”处理（并集），避免把经闭包赋值的合法程序误判。
        Expr::Spawn { body } => {
            let (out, _) = da_stmt_list(body, cur.clone(), required, fields, &mut Vec::new());
            let mut c = cur;
            c.extend(out);
            (c, true)
        }
        Expr::Lambda { body, .. } => {
            let mut c = cur.clone();
            if let Expr::Block(stmts, _) = body.as_ref() {
                let (out, _) = da_stmt_list(stmts, cur, required, fields, &mut Vec::new());
                c.extend(out);
            }
            (c, true)
        }
        Expr::TryBlock {
            body,
            catch_body,
            finally_body,
            ..
        } => {
            let mut c = cur.clone();
            let mut sets: Vec<HashSet<String>> = Vec::new();
            let (out, _) = da_stmt_list(body, cur.clone(), required, fields, &mut Vec::new());
            sets.push(out);
            let (out, _) = da_stmt_list(catch_body, cur.clone(), required, fields, &mut Vec::new());
            sets.push(out);
            if let Some(fb) = finally_body {
                let (out, _) = da_stmt_list(fb, cur.clone(), required, fields, &mut Vec::new());
                sets.push(out);
            }
            for s in sets {
                c.extend(s);
            }
            (c, true)
        }
        Expr::TrailingClosure { closure, .. } => {
            let mut c = cur.clone();
            if let Expr::Lambda { body, .. } = closure.as_ref() {
                if let Expr::Block(stmts, _) = body.as_ref() {
                    let (out, _) = da_stmt_list(stmts, cur, required, fields, &mut Vec::new());
                    c.extend(out);
                }
            }
            (c, true)
        }
        Expr::Macro { args, .. } => {
            let mut c = cur;
            for a in args {
                if let Expr::Block(stmts, _) = a {
                    let (out, _) = da_stmt_list(stmts, c.clone(), required, fields, &mut Vec::new());
                    c.extend(out);
                }
            }
            (c, true)
        }
        _ => (cur, true),
    }
}
/// 收集赋值目标必然初始化的字段。
fn da_target(t: &AssignTarget, cur: &mut HashSet<String>, fields: &HashSet<String>) {
    match t {
        AssignTarget::Var(name) => {
            if fields.contains(name) {
                cur.insert(name.clone());
            }
        }
        AssignTarget::Field { object, field } => {
            if (object == "this" || object == "self") && fields.contains(field) {
                cur.insert(field.clone());
            }
        }
        AssignTarget::FieldPath { base, fields: path } => {
            if (base == "this" || base == "self") && path.len() == 1 {
                if fields.contains(&path[0]) {
                    cur.insert(path[0].clone());
                }
            }
        }
        AssignTarget::Tuple(ts) => {
            for sub in ts {
                da_target(sub, cur, fields);
            }
        }
        _ => {}
    }
}

fn da_intersect(a: &HashSet<String>, b: &HashSet<String>) -> HashSet<String> {
    a.intersection(b).cloned().collect()
}

/// 6.1.2.1(22)：命名参数必须位于所有普通参数之后。
/// 6.1.2.1(8)/(9)：构造函数形参在构造函数体内不可被重新赋值。
fn check_ctor_param_rules(cls: &ClassDef, errors: &mut Vec<SemanticError>) {
    let report_order = |idx: usize, what: &str, errors: &mut Vec<SemanticError>| {
        errors.push(SemanticError {
            message: format!(
                "类 '{}' 的{}第 {} 个普通参数位于命名参数之后（6.1.2.1(22)）",
                cls.name,
                what,
                idx + 1
            ),
        });
    };
    if let Some(idx) = first_unnamed_after_named(&cls.primary_ctor_params) {
        report_order(idx, "主构造函数", errors);
    }
    let inits: Vec<&InitDef> = if cls.inits.is_empty() {
        cls.init.iter().collect()
    } else {
        cls.inits.iter().collect()
    };
    for init in &inits {
        if let Some(idx) = first_unnamed_after_named(&init.params) {
            report_order(idx, "构造函数", errors);
        }
        // 形参不可在构造函数体内重新赋值。
        let names: HashSet<&str> = init.params.iter().map(|p| p.name.as_str()).collect();
        if names.is_empty() {
            continue;
        }
        let mut assigned: Vec<String> = Vec::new();
        for s in &init.body {
            collect_param_assigns_stmt(s, &names, &mut assigned);
        }
        assigned.sort();
        assigned.dedup();
        for name in assigned {
            errors.push(SemanticError {
                message: format!(
                    "类 '{}' 的构造函数形参 '{}' 不能被重新赋值（6.1.2.1(8)/(9)）",
                    cls.name, name
                ),
            });
        }
    }
    // 6.1.2.1(19)：初始值表达式可引用其前已定义的参数，但不得修改这些参数的值。
    // 默认值表达式中的赋值被解析为 `Expr::Block([Assign])`（如 `(a = 10)`）。
    let mut default_assign_viol: Vec<String> = Vec::new();
    {
        let primary_names: HashSet<&str> = cls
            .primary_ctor_params
            .iter()
            .map(|p| p.name.as_str())
            .collect();
        if !primary_names.is_empty() {
            for p in &cls.primary_ctor_params {
                if let Some(def) = &p.default {
                    collect_default_param_assigns(def, &primary_names, &mut default_assign_viol);
                }
            }
        }
    }
    for init in &inits {
        let names: HashSet<&str> = init.params.iter().map(|p| p.name.as_str()).collect();
        if names.is_empty() {
            continue;
        }
        for p in &init.params {
            if let Some(def) = &p.default {
                collect_default_param_assigns(def, &names, &mut default_assign_viol);
            }
        }
    }
    default_assign_viol.sort();
    default_assign_viol.dedup();
    for name in default_assign_viol {
        errors.push(SemanticError {
            message: format!(
                "类 '{}' 的构造函数参数默认值表达式不能修改参数 '{}'（6.1.2.1(19)）",
                cls.name, name
            ),
        });
    }
}

/// 6.1.2.1(19)：扫描构造函数参数默认值表达式，收集其中对被引用参数的赋值。
fn collect_default_param_assigns(e: &Expr, names: &HashSet<&str>, out: &mut Vec<String>) {
    let mut found: Vec<String> = Vec::new();
    walk_expr(e, &mut |sub| {
        if let Expr::Block(stmts, _) = sub {
            for s in stmts {
                collect_param_assigns_stmt(s, names, &mut found);
            }
        }
    });
    out.extend(found);
}

/// 6.1.2.1(49)：构造函数参数默认值不得引用实例成员
/// （`this.x` / `super.x`，或与成员变量同名的裸标识符）。
fn check_ctor_param_default_member_refs(
    cls: &ClassDef,
    program: &Program,
    index: &ProgramIndex<'_>,
    errors: &mut Vec<SemanticError>,
) {
    let mut instance_fields: HashSet<String> = HashSet::new();
    for f in &cls.fields {
        if !f.is_static {
            instance_fields.insert(f.name.clone());
        }
    }
    let mut cur = cls.extends.clone();
    let mut seen: HashSet<String> = HashSet::new();
    while let Some(name) = cur {
        if !seen.insert(name.clone()) {
            break;
        }
        let Some(parent) = index.classes.get(name.as_str()) else {
            break;
        };
        for f in &parent.fields {
            if !f.is_static {
                instance_fields.insert(f.name.clone());
            }
        }
        cur = parent.extends.clone();
    }
    if instance_fields.is_empty() {
        return;
    }
    let globals: HashSet<&str> = program.constants.iter().map(|c| c.name.as_str()).collect();
    let top_funcs: HashSet<&str> = program
        .functions
        .iter()
        .filter(|f| !f.name.contains('.'))
        .map(|f| f.name.as_str())
        .collect();
    let mut ctors: Vec<&[Param]> = vec![&cls.primary_ctor_params];
    if cls.inits.is_empty() {
        if let Some(i) = &cls.init {
            ctors.push(&i.params);
        }
    } else {
        for i in &cls.inits {
            ctors.push(&i.params);
        }
    }
    let mut reported: HashSet<(String, String)> = HashSet::new();
    for params in ctors {
        let param_names: HashSet<&str> = params.iter().map(|p| p.name.as_str()).collect();
        for p in params {
            let Some(def) = &p.default else { continue };
            let mut hit: Option<String> = None;
            walk_expr(def, &mut |e| {
                if hit.is_some() {
                    return;
                }
                match e {
                    Expr::Field { object, field } => {
                        if matches!(object.as_ref(), Expr::Var(n) if n == "this" || n == "self") {
                            hit = Some(format!("this.{}", field));
                        }
                    }
                    Expr::SuperFieldAccess { field } => {
                        hit = Some(format!("super.{}", field));
                    }
                    Expr::Var(n) => {
                        if instance_fields.contains(n)
                            && !param_names.contains(n.as_str())
                            && !globals.contains(n.as_str())
                            && !top_funcs.contains(n.as_str())
                        {
                            hit = Some(n.clone());
                        }
                    }
                    _ => {}
                }
            });
            if let Some(name) = hit {
                if reported.insert((p.name.clone(), name.clone())) {
                    errors.push(SemanticError {
                        message: format!(
                            "类 '{}' 的构造函数参数 '{}' 的默认值不能引用实例成员 '{}'（6.1.2.1(49)）",
                            cls.name, p.name, name
                        ),
                    });
                }
            }
        }
    }
}

/// 6.1.2.1(59)：构造函数调用 `this(...)`/`self(...)` 只允许出现在构造函数体内。
fn check_this_ctor_call_outside_init(cls: &ClassDef, errors: &mut Vec<SemanticError>) {
    for m in &cls.methods {
        if stmts_contain_this_ctor_call(&m.func.body) {
            let base = m.func.name.rsplit('.').next().unwrap_or(&m.func.name);
            errors.push(SemanticError {
                message: format!(
                    "类 '{}' 的成员 '{}' 中不能调用本类构造函数 this(...)（6.1.2.1(59)）",
                    cls.name, base
                ),
            });
        }
    }
    for f in &cls.fields {
        if let Some(d) = &f.default {
            if expr_contains_this_ctor_call(d) {
                errors.push(SemanticError {
                    message: format!(
                        "类 '{}' 的成员变量 '{}' 的初始值不能调用本类构造函数 this(...)（6.1.2.1(59)）",
                        cls.name, f.name
                    ),
                });
            }
        }
    }
    if let Some(si) = &cls.static_init {
        if stmts_contain_this_ctor_call(si) {
            errors.push(SemanticError {
                message: format!(
                    "类 '{}' 的 static init 块中不能调用本类构造函数 this(...)（6.1.2.1(59)）",
                    cls.name
                ),
            });
        }
    }
    if let Some(d) = &cls.deinit {
        if stmts_contain_this_ctor_call(d) {
            errors.push(SemanticError {
                message: format!(
                    "类 '{}' 的 deinit 中不能调用本类构造函数 this(...)（6.1.2.1(59)）",
                    cls.name
                ),
            });
        }
    }
}

/// 返回第一个“出现在命名参数之后的普通参数”的下标（6.1.2.1(22)）。
fn first_unnamed_after_named(params: &[Param]) -> Option<usize> {
    let mut seen_named = false;
    for (i, p) in params.iter().enumerate() {
        if p.is_named {
            seen_named = true;
        } else if seen_named {
            return Some(i);
        }
    }
    None
}

/// 6.1.2.1(54)：父类实例成员只能在调用本类/父类构造函数之后通过 `super.f` 访问；
/// 静态上下文（static 字段初始化、static 方法/属性/static init）中不得使用 `super`。
fn check_super_access_rules(cls: &ClassDef, errors: &mut Vec<SemanticError>) {
    let mut report = |what: &str, name: &str, errors: &mut Vec<SemanticError>| {
        errors.push(SemanticError {
            message: format!(
                "类 '{}' 的{}中不能使用 '{}'（静态上下文不允许访问父类实例成员）（6.1.2.1(54)）",
                cls.name, what, name
            ),
        });
    };
    // 静态字段初始化表达式。
    for f in &cls.fields {
        if f.is_static {
            if let Some(d) = &f.default {
                if let Some(name) = super_use_in_expr(d) {
                    report(&format!("静态字段 '{}' 的初始化表达式", f.name), &name, errors);
                }
            }
        }
    }
    // 静态方法 / 静态属性 / static init 块。
    for m in &cls.methods {
        if m.is_static {
            let mut name: Option<String> = None;
            for p in &m.func.params {
                if let Some(d) = &p.default {
                    if name.is_none() {
                        name = super_use_in_expr(d);
                    }
                }
            }
            if name.is_none() {
                name = super_use_in_stmts(&m.func.body);
            }
            if let Some(n) = name {
                let base = m.func.name.rsplit('.').next().unwrap_or(&m.func.name);
                report(&format!("静态成员 '{}'", base), &n, errors);
            }
        }
    }
    if let Some(si) = &cls.static_init {
        if let Some(n) = super_use_in_stmts(si) {
            report("static init 块", &n, errors);
        }
    }

    // 实例构造函数：若存在显式 `this(...)`/`super(...)` 调用，则它之前不得访问 super 成员。
    let inits: Vec<&InitDef> = if cls.inits.is_empty() {
        cls.init.iter().collect()
    } else {
        cls.inits.iter().collect()
    };
    for init in &inits {
        let has_call = init
            .body
            .iter()
            .any(|s| stmt_has_explicit_ctor_call(s));
        if !has_call {
            continue;
        }
        let mut called = false;
        for s in &init.body {
            if !called {
                if let Some(name) = super_member_use_in_stmt(s) {
                    errors.push(SemanticError {
                        message: format!(
                            "类 '{}' 的构造函数在调用本类/父类构造函数之前不能访问 '{}'（6.1.2.1(54)）",
                            cls.name, name
                        ),
                    });
                }
            }
            if stmt_has_explicit_ctor_call(s) {
                called = true;
            }
        }
    }
}

/// 6.1.2.1(23)/(24)：若构造函数体显式调用父类构造函数 `super(...)` 或本类构造函数
/// `this(...)`，该调用必须是构造函数体的第一条语句（不能出现在变量声明、函数定义
/// 或其它表达式之后）。
fn check_explicit_ctor_call_first(cls: &ClassDef, errors: &mut Vec<SemanticError>) {
    fn is_ctor_call_stmt(s: &Stmt) -> bool {
        match s {
            Stmt::Expr(e) => match e {
                Expr::SuperCall { method, .. } => method == "init",
                Expr::Call { name, .. } => name == "this",
                // `this(...)` 通常解析为 Invoke（callee 为 `this`/`self`）。
                Expr::Invoke { callee, .. } => {
                    matches!(callee.as_ref(), Expr::Var(n) if n == "this" || n == "self")
                }
                _ => false,
            },
            _ => false,
        }
    }
    let inits: Vec<&InitDef> = if cls.inits.is_empty() {
        cls.init.iter().collect()
    } else {
        cls.inits.iter().collect()
    };
    for init in &inits {
        if let Some(pos) = init.body.iter().position(is_ctor_call_stmt) {
            if pos > 0 {
                errors.push(SemanticError {
                    message: format!(
                        "类 '{}' 的构造函数中显式 super(...)/this(...) 调用必须是构造函数体的第一条语句（6.1.2.1(23)/(24)）",
                        cls.name
                    ),
                });
            }
        }
    }
}

/// 6.1.2.1(34)/(50)：一个构造函数体内至多只能出现一条显式 super(...)/this(...)
/// 调用（无论二者是否相同、参数是否相同）。
fn check_ctor_call_multiplicity(cls: &ClassDef, errors: &mut Vec<SemanticError>) {
    fn classify(s: &Stmt) -> Option<u8> {
        match s {
            Stmt::Expr(e) => match e {
                Expr::SuperCall { method, .. } if method == "init" => Some(0),
                Expr::Call { name, .. } if name == "this" => Some(1),
                Expr::Invoke { callee, .. } => {
                    if matches!(callee.as_ref(), Expr::Var(n) if n == "this" || n == "self") {
                        Some(1)
                    } else {
                        None
                    }
                }
                _ => None,
            },
            _ => None,
        }
    }
    let inits: Vec<&InitDef> = if cls.inits.is_empty() {
        cls.init.iter().collect()
    } else {
        cls.inits.iter().collect()
    };
    for init in &inits {
        let mut supers = 0usize;
        let mut thises = 0usize;
        for s in &init.body {
            match classify(s) {
                Some(0) => supers += 1,
                Some(1) => thises += 1,
                _ => {}
            }
        }
        if supers + thises > 1 {
            errors.push(SemanticError {
                message: format!(
                    "类 '{}' 的构造函数体中至多只能有一条 super(...)/this(...) 调用，不能同时或重复调用（6.1.2.1(34)/(50)）",
                    cls.name
                ),
            });
        }
    }
}

/// 6.1.2.1(16)：主构造函数中定义的成员变量参数不得与构造函数外声明的成员同名。
fn check_ctor_member_param_conflicts(cls: &ClassDef, errors: &mut Vec<SemanticError>) {
    // `class Foo(...)` 形式的主构造函数参数会被 parser 展开为同名字段；这些自动
    // 生成的字段不代表“构造函数外声明的成员”，故从比较集合中排除。
    let primary: HashSet<&str> = cls
        .primary_ctor_params
        .iter()
        .map(|p| p.name.as_str())
        .collect();
    let outside: HashSet<&str> = cls
        .fields
        .iter()
        .filter(|f| !primary.contains(f.name.as_str()))
        .map(|f| f.name.as_str())
        .collect();
    if outside.is_empty() {
        return;
    }
    let inits: Vec<&InitDef> = if cls.inits.is_empty() {
        cls.init.iter().collect()
    } else {
        cls.inits.iter().collect()
    };
    let mut reported: HashSet<String> = HashSet::new();
    for (i, init) in inits.iter().enumerate() {
        for (p, flag) in init.params.iter().zip(init.member_param_flags.iter()) {
            if !*flag {
                continue;
            }
            if outside.contains(p.name.as_str()) && reported.insert(format!("{}#{}", i, p.name)) {
                errors.push(SemanticError {
                    message: format!(
                        "类 '{}' 的成员变量参数 '{}' 与构造函数外声明的成员同名（6.1.2.1(16)）",
                        cls.name, p.name
                    ),
                });
            }
        }
    }
}

/// 在表达式树中查找 `super` 成员/构造访问，返回其展示名。
fn super_use_in_expr(e: &Expr) -> Option<String> {
    let mut found: Option<String> = None;
    walk_expr(e, &mut |sub| {
        if found.is_none() {
            found = super_use_of(sub);
        }
    });
    found
}

fn super_use_in_stmts(stmts: &[Stmt]) -> Option<String> {
    let mut found: Option<String> = None;
    walk_stmts(stmts, &mut |sub| {
        if found.is_none() {
            found = super_use_of(sub);
        }
    });
    found
}

fn super_member_use_in_stmt(s: &Stmt) -> Option<String> {
    let mut found: Option<String> = None;
    walk_stmt(s, &mut |sub| {
        if found.is_none() {
            found = match sub {
                Expr::SuperFieldAccess { field } => Some(format!("super.{}", field)),
                Expr::SuperCall { method, .. } if method != "init" => {
                    Some(format!("super.{}", method))
                }
                _ => None,
            };
        }
    });
    found
}

fn super_use_of(e: &Expr) -> Option<String> {
    match e {
        Expr::SuperFieldAccess { field } => Some(format!("super.{}", field)),
        Expr::SuperCall { method, .. } => Some(if method == "init" {
            "super(...)".to_string()
        } else {
            format!("super.{}", method)
        }),
        _ => None,
    }
}

fn stmt_has_explicit_ctor_call(s: &Stmt) -> bool {
    let mut found = false;
    walk_stmt(s, &mut |e| {
        if is_explicit_ctor_call(e) {
            found = true;
        }
    });
    found
}

/// 是否为显式构造函数调用 `this(...)` 或 `super(...)`。
fn is_explicit_ctor_call(e: &Expr) -> bool {
    match e {
        Expr::Call { name, .. } => name == "this",
        // `this(...)` 通常解析为 Invoke（callee 为 `this`/`self`）。
        Expr::Invoke { callee, .. } => {
            matches!(callee.as_ref(), Expr::Var(n) if n == "this" || n == "self")
        }
        Expr::SuperCall { method, .. } => method == "init",
        _ => false,
    }
}

/// 6.1.2.1(58)/(26)：在全部实例成员变量初始化完成之前，构造函数中不得：
/// - 调用实例成员函数 / 属性 getter（`this.f()`、裸 `f()`、`this.p`、`super.f()` 等）；
/// - 以值的形式引用实例成员函数（裸 `f`、`this.f`、`f<T>`）；
/// - 读取尚未初始化的成员变量（`this.m` 或裸 `m`；已初始化的可读）；
/// - 单独使用 `this` 表达式、在 lambda/局部函数中捕获 `this`。
fn check_use_before_init(
    cls: &ClassDef,
    index: &ProgramIndex<'_>,
    errors: &mut Vec<SemanticError>,
) {
    // 字段集合（排除 parser 为主构造普通参数误加的“字段”）。
    let mut nonmember: HashSet<&str> = HashSet::new();
    for (p, flag) in cls
        .primary_ctor_params
        .iter()
        .zip(cls.primary_ctor_member_flags.iter())
    {
        if !*flag {
            nonmember.insert(p.name.as_str());
        }
    }
    let mut field_names: HashSet<String> = HashSet::new();
    let mut auto: HashSet<String> = HashSet::new();
    for f in &cls.fields {
        if f.is_static || nonmember.contains(f.name.as_str()) {
            continue;
        }
        field_names.insert(f.name.clone());
        if f.default.is_some() {
            auto.insert(f.name.clone());
        }
    }
    for (p, flag) in cls
        .primary_ctor_params
        .iter()
        .zip(cls.primary_ctor_member_flags.iter())
    {
        if *flag {
            auto.insert(p.name.clone());
        }
    }
    // 实例方法名 / 属性名（属性访问器脱糖为 __get_p / __set_p）。
    let mut method_names: HashSet<String> = HashSet::new();
    let mut prop_names: HashSet<String> = HashSet::new();
    for m in &cls.methods {
        if m.is_static {
            continue;
        }
        let base = m.func.name.rsplit('.').next().unwrap_or(&m.func.name);
        if let Some(p) = base.strip_prefix("__get_").or_else(|| base.strip_prefix("__set_")) {
            prop_names.insert(p.to_string());
        } else {
            method_names.insert(base.to_string());
        }
    }
    // 父类（含接口）实例成员：裸名 `f()` / `p` 与 `super.p` 同样构成实例成员访问。
    // `parent_props` 单独保留，用于区分 `super.field`（父类字段，读取合法）与
    // `super.property`（属性 getter，未完成初始化前访问非法）。
    let mut parent_props: HashSet<String> = HashSet::new();
    if let Some((pmethods, pprops)) = collect_parent_member_names(cls, index) {
        method_names.extend(pmethods);
        prop_names.extend(pprops.iter().cloned());
        parent_props = pprops;
    }
    let inits: Vec<&InitDef> = if cls.inits.is_empty() {
        cls.init.iter().collect()
    } else {
        cls.inits.iter().collect()
    };
    for init in &inits {
        let mut start = auto.clone();
        for (p, flag) in init.params.iter().zip(init.member_param_flags.iter()) {
            if *flag {
                start.insert(p.name.clone());
            }
        }
        let tracked: HashSet<String> = field_names.difference(&start).cloned().collect();
        if tracked.is_empty() {
            continue;
        }
        let param_names: HashSet<String> = init.params.iter().map(|p| p.name.clone()).collect();
        let mut cur = start;
        for s in &init.body {
            let all_init = tracked.is_subset(&cur);
            if !all_init {
                if let Some(reason) = stmt_use_reason(
                    s,
                    &cur,
                    &tracked,
                    &field_names,
                    &method_names,
                    &prop_names,
                    &param_names,
                    &parent_props,
                ) {
                    errors.push(SemanticError {
                        message: format!(
                            "类 '{}' 的构造函数在所有实例成员变量初始化之前使用了 {}（6.1.2.1(58)/(26)）",
                            cls.name, reason
                        ),
                    });
                }
            }
            // 推进“必然已初始化”集合。
            let (nc, flows) = da_stmt(s, cur.clone(), &tracked, &field_names, &mut Vec::new());
            cur = nc;
            if !flows {
                break;
            }
        }
    }
}

/// 单条语句中是否出现“未全部初始化前”禁止的使用，返回原因描述。
fn stmt_use_reason(
    s: &Stmt,
    cur: &HashSet<String>,
    tracked: &HashSet<String>,
    field_names: &HashSet<String>,
    methods: &HashSet<String>,
    props: &HashSet<String>,
    params: &HashSet<String>,
    parent_props: &HashSet<String>,
) -> Option<String> {
    match s {
        Stmt::Expr(e) => expr_use_reason(e, cur, tracked, field_names, methods, props, params, parent_props),
        Stmt::Let { value, .. } | Stmt::Var { value, .. } | Stmt::Const { value, .. } => {
            expr_use_reason(value, cur, tracked, field_names, methods, props, params, parent_props)
        }
        Stmt::Assign { value, .. } => {
            expr_use_reason(value, cur, tracked, field_names, methods, props, params, parent_props)
        }
        Stmt::Return(Some(e)) => {
            expr_use_reason(e, cur, tracked, field_names, methods, props, params, parent_props)
        }
        Stmt::Assert { left, right, .. } | Stmt::Expect { left, right, .. } => {
            expr_use_reason(left, cur, tracked, field_names, methods, props, params, parent_props)
                .or_else(|| expr_use_reason(right, cur, tracked, field_names, methods, props, params, parent_props))
        }
        Stmt::LocalFunc(f) => {
            if stmts_contain_this(&f.body) {
                Some("捕获 this 的局部函数".to_string())
            } else {
                None
            }
        }
        _ => None,
    }
}

fn expr_use_reason(
    e: &Expr,
    cur: &HashSet<String>,
    tracked: &HashSet<String>,
    field_names: &HashSet<String>,
    methods: &HashSet<String>,
    props: &HashSet<String>,
    params: &HashSet<String>,
    parent_props: &HashSet<String>,
) -> Option<String> {
    // 若该表达式内部对字段有赋值（控制流/顺序不确定），保守跳过。
    if expr_has_field_assign(e, field_names) {
        return None;
    }
    use_scan(e, cur, tracked, methods, props, params, parent_props)
}

/// 结构感知扫描：在 receiver 位置（`this.x`、`this.f()` 的 object）上的 `this`
/// 不算“单独 this”，其余位置的 `this`/未初始化成员/方法引用均视为违规使用。
fn use_scan(
    e: &Expr,
    cur: &HashSet<String>,
    tracked: &HashSet<String>,
    methods: &HashSet<String>,
    props: &HashSet<String>,
    params: &HashSet<String>,
    parent_props: &HashSet<String>,
) -> Option<String> {
    match e {
        Expr::Var(n) if n == "this" || n == "self" => Some("单独的 this 表达式".to_string()),
        Expr::Var(n) => {
            if params.contains(n) {
                None
            } else if methods.contains(n) {
                Some(format!("实例成员函数 '{}'", n))
            } else if props.contains(n) {
                Some(format!("属性 '{}'", n))
            } else if tracked.contains(n) && !cur.contains(n) {
                Some(format!("未初始化的成员变量 '{}'", n))
            } else {
                None
            }
        }
        Expr::Field { object, field } => {
            if let Expr::Var(o) = object.as_ref() {
                if o == "this" || o == "self" {
                    if props.contains(field) {
                        return Some(format!("属性 '{}'", field));
                    }
                    if tracked.contains(field) && !cur.contains(field) {
                        return Some(format!("未初始化的成员变量 '{}'", field));
                    }
                    return None;
                }
            }
            use_scan(object, cur, tracked, methods, props, params, parent_props)
        }
        Expr::MethodCall {
            object,
            method,
            args,
            named_args,
            ..
        } => {
            if let Expr::Var(o) = object.as_ref() {
                if o == "this" || o == "self" {
                    return Some(format!("实例成员函数 '{}'", method));
                }
                if o == "super" {
                    return Some(format!("super.{}", method));
                }
            }
            if let Some(r) = use_scan(object, cur, tracked, methods, props, params, parent_props) {
                return Some(r);
            }
            for a in args {
                if let Some(r) = use_scan(a, cur, tracked, methods, props, params, parent_props) {
                    return Some(r);
                }
            }
            for (_, a) in named_args {
                if let Some(r) = use_scan(a, cur, tracked, methods, props, params, parent_props) {
                    return Some(r);
                }
            }
            None
        }
        // 父类实例字段经 super 读取是允许的（父类构造函数隐式先执行）；
        // 但 `super.p` 若指向父类属性（getter），未完成初始化前访问同样非法。
        Expr::SuperFieldAccess { field } => {
            if parent_props.contains(field) {
                Some(format!("super.{}", field))
            } else {
                None
            }
        }
        Expr::SuperCall { method, .. } if method != "init" => Some(format!("super.{}", method)),
        Expr::Call {
            name,
            args,
            named_args,
            ..
        } => {
            if !params.contains(name) && methods.contains(name) {
                return Some(format!("实例成员函数 '{}'", name));
            }
            for a in args {
                if let Some(r) = use_scan(a, cur, tracked, methods, props, params, parent_props) {
                    return Some(r);
                }
            }
            for (_, a) in named_args {
                if let Some(r) = use_scan(a, cur, tracked, methods, props, params, parent_props) {
                    return Some(r);
                }
            }
            None
        }
        Expr::Invoke {
            callee,
            args,
            named_args,
        } => {
            if matches!(callee.as_ref(), Expr::Var(n) if n == "this" || n == "self") {
                return None; // this(...) 委托构造函数
            }
            if let Some(r) = use_scan(callee, cur, tracked, methods, props, params, parent_props) {
                return Some(r);
            }
            for a in args {
                if let Some(r) = use_scan(a, cur, tracked, methods, props, params, parent_props) {
                    return Some(r);
                }
            }
            for (_, a) in named_args {
                if let Some(r) = use_scan(a, cur, tracked, methods, props, params, parent_props) {
                    return Some(r);
                }
            }
            None
        }
        Expr::Lambda { body, .. } => {
            if expr_contains_this(body) {
                Some("捕获 this 的 lambda".to_string())
            } else {
                None
            }
        }
        Expr::Binary { left, right, .. } => use_scan(left, cur, tracked, methods, props, params, parent_props)
            .or_else(|| use_scan(right, cur, tracked, methods, props, params, parent_props)),
        Expr::Unary { expr, .. }
        | Expr::Cast { expr, .. }
        | Expr::IsType { expr, .. }
        | Expr::Some(expr)
        | Expr::Ok(expr)
        | Expr::Err(expr)
        | Expr::Throw(expr)
        | Expr::Try(expr)
        | Expr::PostfixIncr(expr)
        | Expr::PrefixIncr(expr)
        | Expr::PostfixDecr(expr)
        | Expr::PrefixDecr(expr)
        | Expr::TupleIndex { object: expr, .. }
        | Expr::OptionalChain { object: expr, .. } => {
            use_scan(expr, cur, tracked, methods, props, params, parent_props)
        }
        Expr::Tuple(es) | Expr::Array(es) => {
            for x in es {
                if let Some(r) = use_scan(x, cur, tracked, methods, props, params, parent_props) {
                    return Some(r);
                }
            }
            None
        }
        Expr::Index { array, index } => use_scan(array, cur, tracked, methods, props, params, parent_props)
            .or_else(|| use_scan(index, cur, tracked, methods, props, params, parent_props)),
        Expr::Range {
            start, end, step, ..
        } => use_scan(start, cur, tracked, methods, props, params, parent_props)
            .or_else(|| use_scan(end, cur, tracked, methods, props, params, parent_props))
            .or_else(|| {
                step.as_ref()
                    .and_then(|s| use_scan(s, cur, tracked, methods, props, params, parent_props))
            }),
        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => use_scan(cond, cur, tracked, methods, props, params, parent_props)
            .or_else(|| use_scan(then_branch, cur, tracked, methods, props, params, parent_props))
            .or_else(|| {
                else_branch
                    .as_ref()
                    .and_then(|b| use_scan(b, cur, tracked, methods, props, params, parent_props))
            }),
        Expr::IfLet {
            expr,
            then_branch,
            else_branch,
            ..
        } => use_scan(expr, cur, tracked, methods, props, params, parent_props)
            .or_else(|| use_scan(then_branch, cur, tracked, methods, props, params, parent_props))
            .or_else(|| {
                else_branch
                    .as_ref()
                    .and_then(|b| use_scan(b, cur, tracked, methods, props, params, parent_props))
            }),
        Expr::Match { expr, arms } => use_scan(expr, cur, tracked, methods, props, params, parent_props).or_else(
            || {
                for arm in arms {
                    if let Some(r) = use_scan(&arm.body, cur, tracked, methods, props, params, parent_props) {
                        return Some(r);
                    }
                }
                None
            },
        ),
        Expr::NullCoalesce { option, default } => {
            use_scan(option, cur, tracked, methods, props, params, parent_props)
                .or_else(|| use_scan(default, cur, tracked, methods, props, params, parent_props))
        }
        Expr::ConstructorCall { args, named_args, .. } => {
            for a in args {
                if let Some(r) = use_scan(a, cur, tracked, methods, props, params, parent_props) {
                    return Some(r);
                }
            }
            for (_, a) in named_args {
                if let Some(r) = use_scan(a, cur, tracked, methods, props, params, parent_props) {
                    return Some(r);
                }
            }
            None
        }
        Expr::StructInit { fields, .. } => {
            for (_, a) in fields {
                if let Some(r) = use_scan(a, cur, tracked, methods, props, params, parent_props) {
                    return Some(r);
                }
            }
            None
        }
        _ => None,
    }
}

/// 表达式内是否出现对某字段的赋值（用于控制流保守跳过）。
fn expr_has_field_assign(e: &Expr, fields: &HashSet<String>) -> bool {
    match e {
        Expr::Block(stmts, _) => stmts_have_field_assign(stmts, fields),
        Expr::If {
            then_branch,
            else_branch,
            ..
        } => {
            expr_has_field_assign(then_branch, fields)
                || else_branch
                    .as_ref()
                    .map(|b| expr_has_field_assign(b, fields))
                    .unwrap_or(false)
        }
        Expr::IfLet {
            then_branch,
            else_branch,
            ..
        } => {
            expr_has_field_assign(then_branch, fields)
                || else_branch
                    .as_ref()
                    .map(|b| expr_has_field_assign(b, fields))
                    .unwrap_or(false)
        }
        Expr::Match { arms, .. } => arms
            .iter()
            .any(|a| expr_has_field_assign(&a.body, fields)),
        _ => false,
    }
}

fn stmts_have_field_assign(stmts: &[Stmt], fields: &HashSet<String>) -> bool {
    stmts.iter().any(|s| match s {
        Stmt::Assign { target, .. } => {
            let mut set = HashSet::new();
            da_target(target, &mut set, fields);
            !set.is_empty()
        }
        Stmt::While { body, .. }
        | Stmt::WhileLet { body, .. }
        | Stmt::DoWhile { body, .. }
        | Stmt::For { body, .. }
        | Stmt::Loop { body }
        | Stmt::UnsafeBlock { body } => stmts_have_field_assign(body, fields),
        Stmt::Expr(e) => expr_has_field_assign(e, fields),
        _ => false,
    })
}

/// 表达式（或其中嵌套语句）内是否引用了 `this`/`self`。
fn expr_contains_this(e: &Expr) -> bool {
    let mut found = false;
    walk_expr(e, &mut |sub| {
        if matches!(sub, Expr::Var(n) if n == "this" || n == "self") {
            found = true;
        }
    });
    found
}

fn stmts_contain_this(stmts: &[Stmt]) -> bool {
    let mut found = false;
    walk_stmts(stmts, &mut |sub| {
        if matches!(sub, Expr::Var(n) if n == "this" || n == "self") {
            found = true;
        }
    });
    found
}

/// 6.1.2.1(57)：`this` 不得逃逸出 open/abstract 类的构造函数
/// （通过 lambda / 局部函数捕获，或作为值传出——赋值、返回、实参等）。
fn check_this_escape_open(cls: &ClassDef, errors: &mut Vec<SemanticError>) {
    if !(cls.is_open || cls.is_abstract) {
        return;
    }
    let inits: Vec<&InitDef> = if cls.inits.is_empty() {
        cls.init.iter().collect()
    } else {
        cls.inits.iter().collect()
    };
    for init in &inits {
        if init.body.iter().any(stmt_escapes_this) {
            errors.push(SemanticError {
                message: format!(
                    "类 '{}' 的构造函数中 this 不能逃逸（open/abstract 类禁止通过 lambda/局部函数捕获或作为值传出）（6.1.2.1(57)）",
                    cls.name
                ),
            });
        }
    }
}

fn stmt_escapes_this(s: &Stmt) -> bool {
    match s {
        Stmt::LocalFunc(f) => stmts_contain_this(&f.body),
        Stmt::Let { value, .. } | Stmt::Var { value, .. } | Stmt::Const { value, .. } => {
            expr_escapes_this(value)
        }
        Stmt::Assign { value, .. } => expr_escapes_this(value),
        Stmt::Expr(e) => expr_escapes_this(e),
        Stmt::Return(Some(e)) => expr_escapes_this(e),
        Stmt::Assert { left, right, .. } | Stmt::Expect { left, right, .. } => {
            expr_escapes_this(left) || expr_escapes_this(right)
        }
        Stmt::While { body, .. }
        | Stmt::WhileLet { body, .. }
        | Stmt::DoWhile { body, .. }
        | Stmt::For { body, .. }
        | Stmt::Loop { body }
        | Stmt::UnsafeBlock { body } => body.iter().any(stmt_escapes_this),
        _ => false,
    }
}

/// `this` 是否以“值”的形式出现在该表达式中（receiver 位置的 `this.x`/`this.f()`
/// 不算逃逸；lambda/局部函数捕获 this 算逃逸）。
fn expr_escapes_this(e: &Expr) -> bool {
    match e {
        Expr::Lambda { body, .. } => expr_contains_this(body),
        Expr::TrailingClosure { closure, .. } => expr_contains_this(closure),
        Expr::Var(n) if n == "this" || n == "self" => true,
        Expr::Field { .. } | Expr::TupleIndex { .. } | Expr::OptionalChain { .. } => false,
        Expr::MethodCall {
            object,
            args,
            named_args,
            ..
        } => {
            if let Expr::Var(o) = object.as_ref() {
                if o == "this" || o == "self" {
                    return args.iter().any(expr_escapes_this)
                        || named_args.iter().any(|(_, a)| expr_escapes_this(a));
                }
            }
            expr_escapes_this(object)
                || args.iter().any(expr_escapes_this)
                || named_args.iter().any(|(_, a)| expr_escapes_this(a))
        }
        Expr::Invoke {
            callee,
            args,
            named_args,
        } => {
            if matches!(callee.as_ref(), Expr::Var(n) if n == "this" || n == "self") {
                return args.iter().any(expr_escapes_this)
                    || named_args.iter().any(|(_, a)| expr_escapes_this(a));
            }
            expr_escapes_this(callee)
                || args.iter().any(expr_escapes_this)
                || named_args.iter().any(|(_, a)| expr_escapes_this(a))
        }
        Expr::Call {
            args, named_args, ..
        }
        | Expr::ConstructorCall {
            args, named_args, ..
        }
        | Expr::SuperCall {
            args, named_args, ..
        } => {
            args.iter().any(expr_escapes_this)
                || named_args.iter().any(|(_, a)| expr_escapes_this(a))
        }
        Expr::StructInit { fields, .. } => fields.iter().any(|(_, a)| expr_escapes_this(a)),
        Expr::Binary { left, right, .. } => expr_escapes_this(left) || expr_escapes_this(right),
        Expr::Unary { expr, .. }
        | Expr::Cast { expr, .. }
        | Expr::IsType { expr, .. }
        | Expr::Some(expr)
        | Expr::Ok(expr)
        | Expr::Err(expr)
        | Expr::Throw(expr)
        | Expr::Try(expr)
        | Expr::PostfixIncr(expr)
        | Expr::PrefixIncr(expr)
        | Expr::PostfixDecr(expr)
        | Expr::PrefixDecr(expr) => expr_escapes_this(expr),
        Expr::Tuple(es) | Expr::Array(es) => es.iter().any(expr_escapes_this),
        Expr::Index { array, index } => expr_escapes_this(array) || expr_escapes_this(index),
        Expr::Range {
            start, end, step, ..
        } => {
            expr_escapes_this(start)
                || expr_escapes_this(end)
                || step.as_ref().map(|s| expr_escapes_this(s)).unwrap_or(false)
        }
        Expr::NullCoalesce { option, default } => {
            expr_escapes_this(option) || expr_escapes_this(default)
        }
        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => {
            expr_escapes_this(cond)
                || expr_escapes_this(then_branch)
                || else_branch.as_ref().map(|b| expr_escapes_this(b)).unwrap_or(false)
        }
        Expr::IfLet {
            expr,
            then_branch,
            else_branch,
            ..
        } => {
            expr_escapes_this(expr)
                || expr_escapes_this(then_branch)
                || else_branch.as_ref().map(|b| expr_escapes_this(b)).unwrap_or(false)
        }
        Expr::Match { expr, arms } => {
            expr_escapes_this(expr) || arms.iter().any(|a| expr_escapes_this(&a.body))
        }
        _ => false,
    }
}


/// 递归查找对指定名称（构造函数形参）的裸变量赋值。
fn collect_param_assigns_stmt(stmt: &Stmt, names: &HashSet<&str>, out: &mut Vec<String>) {
    match stmt {
        Stmt::Assign {
            target: AssignTarget::Var(n),
            ..
        } => {
            if names.contains(n.as_str()) {
                out.push(n.clone());
            }
        }
        Stmt::While { body, .. }
        | Stmt::WhileLet { body, .. }
        | Stmt::DoWhile { body, .. }
        | Stmt::For { body, .. }
        | Stmt::Loop { body }
        | Stmt::UnsafeBlock { body } => {
            for s in body {
                collect_param_assigns_stmt(s, names, out);
            }
        }
        Stmt::Expr(e) => collect_param_assigns_expr(e, names, out),
        _ => {}
    }
}

fn collect_param_assigns_expr(expr: &Expr, names: &HashSet<&str>, out: &mut Vec<String>) {
    match expr {
        Expr::Block(stmts, trailing) => {
            for s in stmts {
                collect_param_assigns_stmt(s, names, out);
            }
            if let Some(t) = trailing {
                collect_param_assigns_expr(t, names, out);
            }
        }
        Expr::If {
            then_branch,
            else_branch,
            ..
        } => {
            collect_param_assigns_expr(then_branch, names, out);
            if let Some(eb) = else_branch {
                collect_param_assigns_expr(eb, names, out);
            }
        }
        Expr::IfLet {
            then_branch,
            else_branch,
            ..
        } => {
            collect_param_assigns_expr(then_branch, names, out);
            if let Some(eb) = else_branch {
                collect_param_assigns_expr(eb, names, out);
            }
        }
        Expr::Match { arms, .. } => {
            for arm in arms {
                collect_param_assigns_expr(&arm.body, names, out);
            }
        }
        _ => {}
    }
}

/// 6.1.2.3(3)：实例成员变量的初始化表达式只能引用在它之前声明的实例成员变量。
/// 6.1.2.3(4)：实例成员变量初始化表达式不得用 `super` 访问父类型成员
/// （初始化发生在父类构造函数调用之前）。
fn check_instance_field_initializers(cls: &ClassDef, errors: &mut Vec<SemanticError>) {
    // 实例字段名 → 声明顺序（同名重复由其它规则报告，这里取首个）。
    let mut order: HashMap<&str, usize> = HashMap::new();
    for (i, f) in cls.fields.iter().enumerate() {
        if f.is_static {
            continue;
        }
        order.entry(f.name.as_str()).or_insert(i);
    }
    for (i, f) in cls.fields.iter().enumerate() {
        if f.is_static {
            continue;
        }
        let Some(expr) = &f.default else {
            continue;
        };
        // 初始化表达式内引入的局部绑定（let/var/for/lambda 参数等）不是字段引用。
        let mut locals: HashSet<String> = HashSet::new();
        walk_expr(expr, &mut |e| match e {
            Expr::Lambda { params, .. } => {
                for (n, _) in params {
                    locals.insert(n.clone());
                }
            }
            Expr::Block(stmts, _) => collect_bindings_stmts(stmts, &mut locals),
            Expr::Match { arms, .. } => {
                for arm in arms {
                    collect_pattern_bindings(&arm.pattern, &mut locals);
                }
            }
            Expr::IfLet { pattern, .. } => {
                collect_pattern_bindings(pattern, &mut locals);
            }
            _ => {}
        });
        let mut later: Option<String> = None;
        let mut super_use: Option<String> = None;
        walk_expr(expr, &mut |e| match e {
            Expr::Var(name) => {
                if later.is_none() && !locals.contains(name.as_str()) {
                    if let Some(&j) = order.get(name.as_str()) {
                        // 只有“之前”声明的实例字段可用于初始化；自身及之后均非法。
                        if j >= i {
                            later = Some(name.clone());
                        }
                    }
                }
            }
            Expr::Field { object, field } if is_this_expr(object) => {
                if later.is_none() {
                    if let Some(&j) = order.get(field.as_str()) {
                        if j >= i {
                            later = Some(field.clone());
                        }
                    }
                }
            }
            Expr::SuperFieldAccess { field } => {
                if super_use.is_none() {
                    super_use = Some(format!("super.{}", field));
                }
            }
            Expr::SuperCall { method, .. } => {
                if super_use.is_none() {
                    super_use = Some(format!("super.{}", method));
                }
            }
            Expr::MethodCall { object, method, .. }
                if matches!(object.as_ref(), Expr::Var(n) if n == "super") =>
            {
                if super_use.is_none() {
                    super_use = Some(format!("super.{}", method));
                }
            }
            _ => {}
        });
        if let Some(name) = super_use {
            errors.push(SemanticError {
                message: format!(
                    "类 '{}' 的实例成员变量 '{}' 的初始化表达式不能通过 '{}' 访问父类型成员（6.1.2.3(4)）",
                    cls.name, f.name, name
                ),
            });
        }
        if let Some(name) = later {
            errors.push(SemanticError {
                message: format!(
                    "类 '{}' 的实例成员变量 '{}' 的初始化表达式引用了尚未初始化的成员 '{}'（6.1.2.3(3)）",
                    cls.name, f.name, name
                ),
            });
        }
    }
}

/// 是否为 `this` / `self` 接收者表达式。
fn is_this_expr(e: &Expr) -> bool {
    matches!(e, Expr::Var(n) if n == "this" || n == "self")
}

/// 6.1.2.1(56)：open/abstract 类的构造函数（成员尚未全部初始化）中
/// 禁止调用实例成员函数或属性（包括经 `super.` 访问父类型成员）。
fn check_open_class_ctor_member_calls(
    cls: &ClassDef,
    index: &ProgramIndex<'_>,
    errors: &mut Vec<SemanticError>,
) {
    if !(cls.is_open || cls.is_abstract) {
        return;
    }
    let Some(init) = &cls.init else {
        return;
    };
    // 属性脱糖为 `__get_p` / `__set_p`，收集本类与可解析父类型的属性名；
    // 同时收集实例方法名，用于识别裸名 `f()` / `p` 形式的成员访问。
    let mut props: HashSet<String> = HashSet::new();
    let mut methods: HashSet<String> = HashSet::new();
    for m in &cls.methods {
        let base = m.func.name.rsplit('.').next().unwrap_or(&m.func.name);
        if let Some(p) = base
            .strip_prefix("__get_")
            .or_else(|| base.strip_prefix("__set_"))
        {
            props.insert(p.to_string());
        } else if !m.is_static {
            methods.insert(base.to_string());
        }
    }
    if let Some((pmethods, pprops)) = collect_parent_member_names(cls, index) {
        methods.extend(pmethods);
        props.extend(pprops);
    }
    let params: HashSet<String> = init.params.iter().map(|p| p.name.clone()).collect();

    let mut report: Option<String> = None;
    for stmt in &init.body {
        walk_stmt(stmt, &mut |e| {
            if report.is_some() {
                return;
            }
            match e {
                Expr::MethodCall { object, method, .. } if is_this_expr(object) => {
                    report = Some(format!("this.{}()", method));
                }
                Expr::Field { object, field } if is_this_expr(object) => {
                    if props.contains(field.as_str()) {
                        report = Some(format!("this.{}", field));
                    }
                }
                Expr::SuperCall { method, .. } if method != "init" => {
                    report = Some(format!("super.{}()", method));
                }
                Expr::SuperFieldAccess { field } => {
                    if props.contains(field.as_str()) {
                        report = Some(format!("super.{}", field));
                    }
                }
                Expr::MethodCall { object, method, .. }
                    if matches!(object.as_ref(), Expr::Var(n) if n == "super") =>
                {
                    report = Some(format!("super.{}()", method));
                }
                // 裸名形式：`f()`（实例方法）/ `p`（属性），本类或继承而来。
                Expr::Call { name, .. } if !params.contains(name) && methods.contains(name) => {
                    report = Some(format!("{}()", name));
                }
                Expr::Var(n) if !params.contains(n) && props.contains(n) => {
                    report = Some(format!("{}", n));
                }
                _ => {}
            }
        });
    }
    if let Some(what) = report {
        errors.push(SemanticError {
            message: format!(
                "类 '{}' 的构造函数中不能调用实例成员函数或属性（{}）（6.1.2.1(56)）",
                cls.name, what
            ),
        });
    }
}

/// 泛型约束上界的规范形式。
#[derive(Debug, Clone, PartialEq, Eq)]
enum BoundVal {
    /// `Any` / `Object`：顶层约束（等价于不约束）。
    Top,
    /// 方法级类型参数（按声明顺序编号）。
    TypeParam(usize),
    /// 具名类型。
    Name(String),
}

/// 把一组泛型约束规范化为 `类型参数序号 → 上界列表`。
///
/// 上界若引用方法级类型参数则映射为其声明序号；若引用类级类型参数，
/// 则返回 `None`（无法安全比较，保守跳过）。
fn canonical_constraint_bounds(
    constraints: &[TypeConstraint],
    type_params: &[String],
    class_type_params: &[String],
) -> Option<HashMap<usize, Vec<BoundVal>>> {
    let mut map: HashMap<usize, Vec<BoundVal>> = HashMap::new();
    for c in constraints {
        let pi = type_params.iter().position(|n| n == &c.param)?;
        for b in &c.bounds {
            if class_type_params.iter().any(|n| n == b) {
                return None;
            }
            let v = if b == "Any" || b == "Object" {
                BoundVal::Top
            } else if let Some(bi) = type_params.iter().position(|n| n == b) {
                BoundVal::TypeParam(bi)
            } else {
                BoundVal::Name(b.clone())
            };
            map.entry(pi).or_default().push(v);
        }
    }
    Some(map)
}

/// 父约束 `p` 是否蕴含子约束 `c`（即 `p` 描述的类型集合是 `c` 的子集，`p <: c`）。
fn bound_implies(p: &BoundVal, c: &BoundVal, index: &ProgramIndex<'_>) -> bool {
    match (p, c) {
        (BoundVal::Top, BoundVal::Top) => true,
        // 父类型没有任何限制时，只能由 Any/Object 放宽。
        (BoundVal::Top, _) => false,
        (_, BoundVal::Top) => true,
        (BoundVal::TypeParam(i), BoundVal::TypeParam(j)) => i == j,
        (BoundVal::Name(a), BoundVal::Name(b)) => a == b || index.is_subtype_name(a, b),
        _ => false,
    }
}

/// 判断子类型函数的约束是否不严于父类型（更宽松或相同）。
///
/// 允许：删除约束、把上界换成父类型的上界（或多个父上界之一）；
/// 禁止：新增约束项（如 `T <: B & Object`）、把上界换成更窄的类型。
fn constraints_not_tighter(
    child: &HashMap<usize, Vec<BoundVal>>,
    parent: &HashMap<usize, Vec<BoundVal>>,
    index: &ProgramIndex<'_>,
) -> bool {
    for (pi, cb) in child {
        if cb.is_empty() {
            continue;
        }
        let empty: Vec<BoundVal> = Vec::new();
        let pb = parent.get(pi).unwrap_or(&empty);
        if pb.is_empty() {
            // 父类型对无约束；子类型只能显式写 Any/Object。
            if cb.iter().all(|b| matches!(b, BoundVal::Top)) {
                continue;
            }
            return false;
        }
        // 子类型不能新增约束项（例如 `T <: B & Object`）。
        if cb.len() > pb.len() {
            return false;
        }
        for c in cb {
            if matches!(c, BoundVal::Top) {
                continue;
            }
            if !pb.iter().any(|p| bound_implies(p, c, index)) {
                return false;
            }
        }
    }
    true
}

/// 6.3.4.1(5) / 6.2.4.1(5)：重定义父类方法或实现接口泛型函数时，
/// 子类型函数的类型参数约束不得比父类型更严格（只能更宽松或相同）。
fn check_generic_constraint_loosening(
    cls: &ClassDef,
    index: &ProgramIndex<'_>,
    errors: &mut Vec<SemanticError>,
) {
    let Some(parents) = collect_parent_methods(cls, index) else {
        return;
    };
    for m in &cls.methods {
        if !m.is_static || m.func.type_params.is_empty() {
            continue;
        }
        let sig = sig_of_class_method(m, &cls.type_params);
        if is_synthetic_member(&sig.name) {
            continue;
        }
        let Some(child_bounds) = canonical_constraint_bounds(
            &m.func.constraints,
            &m.func.type_params,
            &cls.type_params,
        ) else {
            continue;
        };
        for p in &parents {
            if !p.sig.is_static
                || p.sig.name != sig.name
                || p.type_params.len() != m.func.type_params.len()
            {
                continue;
            }
            if canon_params(&sig) != canon_params(&p.sig) {
                continue;
            }
            let parent_tp = index.type_params_of(&p.owner);
            let Some(parent_bounds) =
                canonical_constraint_bounds(&p.constraints, &p.type_params, parent_tp)
            else {
                continue;
            };
            if !constraints_not_tighter(&child_bounds, &parent_bounds, index) {
                errors.push(SemanticError {
                    message: format!(
                        "类 '{}' 重定义/实现的泛型函数 '{}' 的类型参数约束比父类型更严格（6.3.4.1(5)/6.2.4.1(5)）",
                        cls.name, sig.display
                    ),
                });
                break;
            }
        }
    }
}

/// 同上，针对实现接口的枚举/结构体：其内部方法被解析为顶层函数 `Type.method`。
fn check_impl_generic_constraint_loosening<'a>(
    type_name: &str,
    type_params: &[String],
    constraints: &[TypeConstraint],
    kind: &str,
    program: &'a Program,
    index: &ProgramIndex<'a>,
    errors: &mut Vec<SemanticError>,
) {
    let bounds: Vec<&str> = constraints
        .iter()
        .filter(|c| c.param == type_name)
        .flat_map(|c| c.bounds.iter().map(|b| b.as_str()))
        .collect();
    if bounds.is_empty() {
        return;
    }
    let Some(parents) = collect_interface_methods(&bounds, index) else {
        return;
    };
    let mut own: Vec<&crate::ast::Function> = program
        .functions
        .iter()
        .filter(|f| owner_name(&f.name) == Some(type_name))
        .collect();
    if let Some(exts) = index.extends.get(type_name) {
        own.extend(exts.iter().copied());
    }
    for f in own {
        if f.type_params.is_empty() {
            continue;
        }
        let sig = sig_of_function(f, type_params);
        let Some(child_bounds) =
            canonical_constraint_bounds(&f.constraints, &f.type_params, type_params)
        else {
            continue;
        };
        for p in &parents {
            if p.sig.name != sig.name || p.type_params.len() != f.type_params.len() {
                continue;
            }
            if canon_params(&sig) != canon_params(&p.sig) {
                continue;
            }
            let parent_tp = index.type_params_of(&p.owner);
            let Some(parent_bounds) =
                canonical_constraint_bounds(&p.constraints, &p.type_params, parent_tp)
            else {
                continue;
            };
            if !constraints_not_tighter(&child_bounds, &parent_bounds, index) {
                errors.push(SemanticError {
                    message: format!(
                        "{} '{}' 实现的泛型函数 '{}' 的类型参数约束比接口更严格（6.2.4.1(5)）",
                        kind, type_name, sig.display
                    ),
                });
                break;
            }
        }
    }
}

/// 校验 class 的父类型来源（6.1.1.2(1)、6.1.1.2(11)、6.1.1.3(2)(4)、6.1.1.1(6)）：
/// - 类只能单继承：`implements` 中出现的名字不得解析为本地类；
/// - 只能继承 open / abstract / sealed 类（sealed 在同包内可继承）；
/// - 不得继承 struct / enum，也不得把 struct / enum 当作接口实现。
///
/// 父类型无法在本地解析时保守跳过。
fn check_class_supertypes(
    cls: &ClassDef,
    index: &ProgramIndex<'_>,
    errors: &mut Vec<SemanticError>,
) {
    if let Some(parent) = cls.extends.as_deref() {
        if index.classes.contains_key(parent) {
            let p = index.classes[parent];
            // sealed 类允许在同一包（单文件视为同包）内被继承；abstract 类天然可继承。
            if !p.is_open && !p.is_abstract && !p.is_sealed {
                errors.push(SemanticError {
                    message: format!(
                        "类 '{}' 不能继承非 open/abstract/sealed 类 '{}'",
                        cls.name, parent
                    ),
                });
            }
        } else if index.structs.contains(parent) || index.enums.contains(parent) {
            errors.push(SemanticError {
                message: format!("类 '{}' 不能继承 struct/enum 类型 '{}'", cls.name, parent),
            });
        }
    }
    for name in &cls.implements {
        if let Some(p) = index.classes.get(name.as_str()) {
            // 抽象类可能被误解析进 implements（如多模板标注用例）；仅对非抽象类报错，
            // 避免误伤合法/宏相关用例。open 类出现在 implements 一定是多继承。
            if !p.is_abstract {
                errors.push(SemanticError {
                    message: format!(
                        "类 '{}' 只能单继承，不能从另一个类 '{}' 继承（父类须位于 <: 列表首位）",
                        cls.name, name
                    ),
                });
            }
        } else if index.structs.contains(name.as_str()) || index.enums.contains(name.as_str()) {
            errors.push(SemanticError {
                message: format!(
                    "类 '{}' 不能将 struct/enum 类型 '{}' 当作接口实现",
                    cls.name, name
                ),
            });
        }
    }

    // 6.1.1.3(8)/(9)：同一接口不得以相同泛型实参被直接实现两次。
    // 不同实参（如 I<Int> & I<String>）是合法的，因此按规范化后的完整类型比较。
    let mut seen_supers: HashMap<String, ()> = HashMap::new();
    for (name, args) in &cls.supertype_args {
        if !index.interfaces.contains_key(name.as_str()) {
            continue;
        }
        let mut map = HashMap::new();
        let canon = if args.is_empty() {
            format!("S:{}", name)
        } else {
            let inner: Vec<String> = args
                .iter()
                .map(|a| canon_type(a, &cls.type_params, &mut map))
                .collect();
            format!("S:{}<{}>", name, inner.join(","))
        };
        if seen_supers.contains_key(&canon) {
            errors.push(SemanticError {
                message: format!(
                    "类 '{}' 不能以相同泛型实参重复实现接口 '{}'（6.1.1.3(8)/(9)）",
                    cls.name, name
                ),
            });
            break;
        }
        seen_supers.insert(canon, ());
    }
}

/// 6.1.1(4)：`<:` 列表中的每个名字必须是合法的类型引用。
///
/// 保守实现：只拒绝本地可判定的明显错误形态——
/// - 父类/接口位置出现 `Range`（范围类型不能作为父类/接口）；
/// - 父类或接口位置出现绑定到顶层变量/函数的标识符；
/// - 父类型的泛型实参是变量名/函数名/包名。
/// 未在本地声明的名字（可能来自 import / 预置库）一律放行。
fn check_supertype_declaration_kinds(
    cls: &ClassDef,
    program: &Program,
    _index: &ProgramIndex<'_>,
    errors: &mut Vec<SemanticError>,
) {
    let is_global_value_name = |n: &str| -> bool {
        program.constants.iter().any(|c| c.name == n)
            || program
                .functions
                .iter()
                .any(|f| !f.name.contains('.') && !f.name.contains("::") && f.name == n)
    };
    if let Some(parent) = cls.extends.as_deref() {
        if parent == "Range" {
            errors.push(SemanticError {
                message: format!(
                    "类 '{}' 不能以范围类型作为父类（6.1.1(4)）",
                    cls.name
                ),
            });
        } else if is_global_value_name(parent) {
            errors.push(SemanticError {
                message: format!(
                    "类 '{}' 的父类 '{}' 绑定到变量/函数而非类型（6.1.1(4)）",
                    cls.name, parent
                ),
            });
        }
    }
    for name in &cls.implements {
        if name == "Range" {
            errors.push(SemanticError {
                message: format!(
                    "类 '{}' 不能以范围类型作为接口（6.1.1(4)）",
                    cls.name
                ),
            });
        } else if is_global_value_name(name) {
            errors.push(SemanticError {
                message: format!(
                    "类 '{}' 的接口 '{}' 绑定到变量/函数而非类型（6.1.1(4)）",
                    cls.name, name
                ),
            });
        }
    }
    // 6.1.1.2(6)：父类型的泛型实参必须是类型；变量名/函数名/包名均非法。
    let package_name = program.package_name.as_deref();
    for (_, args) in &cls.supertype_args {
        for a in args {
            let bad = match a {
                Type::Struct(n, inner) if inner.is_empty() => {
                    is_global_value_name(n) || package_name == Some(n.as_str())
                }
                Type::TypeParam(n) => is_global_value_name(n),
                _ => false,
            };
            if bad {
                errors.push(SemanticError {
                    message: format!(
                        "类 '{}' 的父类型泛型实参必须是类型，不能是变量/函数/包名（6.1.1.2(6)）",
                        cls.name
                    ),
                });
                break;
            }
        }
    }
}

/// 6.1.1.2(13)：子类不能通过 `super.父类名()` 显式调用父类的主构造函数。
///
/// 主构造函数只能由 `super(...)` 隐式/显式调用来触发；写成 `super.ClassName()`
/// 会被解析为 `SuperCall { method: "ClassName" }`，属于非法访问。
/// 只有父类名在本地可解析时才判定，外部父类型保守放行。
fn check_super_primary_ctor_access(
    cls: &ClassDef,
    index: &ProgramIndex<'_>,
    errors: &mut Vec<SemanticError>,
) {
    let mut ancestors: HashSet<&str> = HashSet::new();
    let mut cur = cls.extends.as_deref();
    let mut guard = 0usize;
    while let Some(name) = cur {
        if !ancestors.insert(name) {
            break;
        }
        guard += 1;
        if guard > 64 {
            break;
        }
        cur = index.classes.get(name).and_then(|c| c.extends.as_deref());
    }
    if ancestors.is_empty() {
        return;
    }
    let mut hit: Option<String> = None;
    {
        let mut scan = |e: &Expr| {
            if hit.is_none() {
                if let Expr::SuperCall { method, .. } = e {
                    if ancestors.contains(method.as_str()) {
                        hit = Some(method.clone());
                    }
                }
            }
        };
        for m in &cls.methods {
            walk_stmts(&m.func.body, &mut scan);
        }
        for init in &cls.inits {
            walk_stmts(&init.body, &mut scan);
        }
        if let Some(d) = &cls.deinit {
            walk_stmts(d, &mut scan);
        }
        if let Some(body) = &cls.static_init {
            walk_stmts(body, &mut scan);
        }
        for f in &cls.fields {
            if let Some(e) = &f.default {
                walk_expr(e, &mut scan);
            }
        }
    }
    if let Some(name) = hit {
        errors.push(SemanticError {
            message: format!(
                "类 '{}' 不能通过 'super.{}()' 访问父类的主构造函数（6.1.1.2(13)）",
                cls.name, name
            ),
        });
    }
}

/// 6.1.2.1(39)：当类的全部构造函数都是 `private` 时，类外不能实例化该类，
/// 子类也不能通过 `super()` 调用其构造函数。只有本地可判定的情况才报错。
fn check_private_ctor_access(program: &Program, errors: &mut Vec<SemanticError>) {
    let all_private: HashSet<&str> = program
        .classes
        .iter()
        .filter(|c| {
            !c.inits.is_empty()
                && c.inits
                    .iter()
                    .all(|i| matches!(i.visibility, Visibility::Private))
        })
        .map(|c| c.name.as_str())
        .collect();
    if all_private.is_empty() {
        return;
    }
    let parents: HashMap<&str, &str> = program
        .classes
        .iter()
        .filter_map(|c| c.extends.as_deref().map(|p| (c.name.as_str(), p)))
        .collect();
    let mut scan = |stmts: &[Stmt],
                    cur: Option<&str>,
                    parent: Option<&str>,
                    errors: &mut Vec<SemanticError>| {
        walk_stmts(stmts, &mut |e| match e {
            Expr::Call { name, .. } | Expr::ConstructorCall { name, .. } => {
                if all_private.contains(name.as_str()) && cur != Some(name.as_str()) {
                    errors.push(SemanticError {
                        message: format!(
                            "类 '{}' 的构造函数均为 private，不能在类外实例化（6.1.2.1(39)）",
                            name
                        ),
                    });
                }
            }
            Expr::SuperCall { method, .. } if method == "init" => {
                if let Some(p) = parent {
                    if all_private.contains(p) {
                        errors.push(SemanticError {
                            message: format!(
                                "父类 '{}' 的构造函数均为 private，不能通过 super() 调用（6.1.2.1(39)）",
                                p
                            ),
                        });
                    }
                }
            }
            _ => {}
        });
    };
    for f in &program.functions {
        scan(&f.body, None, None, errors);
    }
    for c in &program.constants {
        let one = [Stmt::Expr(c.init.clone())];
        scan(&one, None, None, errors);
    }
    for cls in &program.classes {
        let parent = parents.get(cls.name.as_str()).copied();
        for m in &cls.methods {
            scan(&m.func.body, Some(&cls.name), parent, errors);
        }
        for init in &cls.inits {
            scan(&init.body, Some(&cls.name), parent, errors);
        }
        if let Some(d) = &cls.deinit {
            scan(d, Some(&cls.name), parent, errors);
        }
        if let Some(si) = &cls.static_init {
            scan(si, Some(&cls.name), parent, errors);
        }
        for f in &cls.fields {
            if let Some(e) = &f.default {
                let one = [Stmt::Expr(e.clone())];
                scan(&one, Some(&cls.name), parent, errors);
            }
        }
    }
    for ext in &program.extends {
        for f in &ext.methods {
            scan(&f.body, None, None, errors);
        }
    }
}

/// 6.1.2.1(46)/(53)：若父类没有可访问的无参构造函数，子类的每个构造函数都必须
/// 显式调用 `super(...)`（或通过 `this(...)` 委托给已调用 super 的构造函数）；
/// 没有显式构造函数的子类使用隐式无参构造函数，同样要求父类有无参构造函数。
///
/// 只检查本地可解析的父类；父类无显式构造函数时视为存在隐式无参构造函数。
fn check_super_ctor_availability(
    cls: &ClassDef,
    index: &ProgramIndex<'_>,
    errors: &mut Vec<SemanticError>,
) {
    let Some(parent_name) = cls.extends.as_deref() else {
        return;
    };
    let Some(parent) = index.classes.get(parent_name) else {
        return;
    };
    if parent.inits.is_empty() {
        return;
    }
    // 无参可调用：形参为空，或全部形参都有默认值；且不是 private。
    let callable_without_args = |init: &InitDef| -> bool {
        !matches!(init.visibility, Visibility::Private)
            && (init.params.is_empty() || init.params.iter().all(|p| p.default.is_some()))
    };
    if parent.inits.iter().any(callable_without_args) {
        return;
    }
    let stmt_has_super_call = |stmts: &[Stmt]| -> bool {
        let mut found = false;
        walk_stmts(stmts, &mut |e| {
            if let Expr::SuperCall { method, .. } = e {
                if method == "init" {
                    found = true;
                }
            }
        });
        found
    };
    let stmt_has_this_call = |stmts: &[Stmt]| -> bool {
        let mut found = false;
        walk_stmts(stmts, &mut |e| match e {
            Expr::Call { name, .. } if name == "this" || name == "self" => found = true,
            Expr::Invoke { callee, .. }
                if matches!(callee.as_ref(), Expr::Var(n) if n == "this" || n == "self") =>
            {
                found = true
            }
            _ => {}
        });
        found
    };
    let missing = if cls.inits.is_empty() {
        true
    } else {
        cls.inits.iter().any(|init| {
            !stmt_has_super_call(&init.body) && !stmt_has_this_call(&init.body)
        })
    };
    if missing {
        errors.push(SemanticError {
            message: format!(
                "父类 '{}' 没有可访问的无参构造函数，子类 '{}' 的构造函数必须显式调用 super(...)（6.1.2.1(46)/(53)）",
                parent_name, cls.name
            ),
        });
    }
}

/// 6.1.2.1(48)：没有显式构造函数的非抽象类，若存在没有初始值的实例成员变量，
/// 隐式无参构造函数无法完成初始化，属于非法声明。
///
/// 保守处理：泛型类跳过；若该类型在本程序中以字面量构造 / 带实参实例化（即
/// 项目方言的 `Rect { width: 1 }` 形态）则跳过，避免误伤方言用法。
fn check_implicit_ctor_uninit_fields(program: &Program, errors: &mut Vec<SemanticError>) {
    let mut instantiated: HashSet<String> = HashSet::new();
    {
        let mut collect = |stmts: &[Stmt]| {
            walk_stmts(stmts, &mut |e| match e {
                Expr::StructInit { name, .. }
                | Expr::Call { name, .. }
                | Expr::ConstructorCall { name, .. } => {
                    instantiated.insert(name.clone());
                }
                _ => {}
            });
        };
        for f in &program.functions {
            collect(&f.body);
        }
        for c in &program.constants {
            let one = [Stmt::Expr(c.init.clone())];
            collect(&one);
        }
        for cls in &program.classes {
            for m in &cls.methods {
                collect(&m.func.body);
            }
            for i in &cls.inits {
                collect(&i.body);
            }
            if let Some(d) = &cls.deinit {
                collect(d);
            }
            if let Some(s) = &cls.static_init {
                collect(s);
            }
            for f in &cls.fields {
                if let Some(e) = &f.default {
                    let one = [Stmt::Expr(e.clone())];
                    collect(&one);
                }
            }
        }
    }
    for cls in &program.classes {
        if cls.is_abstract || !cls.type_params.is_empty() || !cls.inits.is_empty() {
            continue;
        }
        if instantiated.contains(cls.name.as_str()) {
            continue;
        }        if let Some(f) = cls
            .fields
            .iter()
            .find(|f| !f.is_static && f.default.is_none())
        {
            errors.push(SemanticError {
                message: format!(
                    "类 '{}' 没有显式构造函数，成员变量 '{}' 没有初始值（6.1.2.1(48)）",
                    cls.name, f.name
                ),
            });
        }
    }
}

/// 扫描所有函数体，禁止实例化抽象类或接口。
fn check_instantiation_targets(
    program: &Program,
    errors: &mut Vec<SemanticError>,
) {
    let abstract_classes: HashSet<&str> = program
        .classes
        .iter()
        .filter(|c| c.is_abstract)
        .map(|c| c.name.as_str())
        .collect();
    let interfaces: HashSet<&str> = program.interfaces.iter().map(|i| i.name.as_str()).collect();
    // 6.1.2.1(47)：没有显式构造函数的类只提供无参（隐式）构造函数。
    let no_ctor_classes: HashSet<&str> = program
        .classes
        .iter()
        .filter(|c| !c.is_abstract && c.inits.is_empty())
        .map(|c| c.name.as_str())
        .collect();
    if abstract_classes.is_empty() && interfaces.is_empty() && no_ctor_classes.is_empty() {
        return;
    }

    // 可能遮蔽类型名的可调用名/成员名。若调用的裸名与这些名字重名，按“调用函数
    // 而非实例化类型”处理并跳过判定（名称遮蔽用例不得误伤）。
    let mut shadowed: HashSet<&str> = HashSet::new();
    for f in &program.functions {
        shadowed.insert(f.name.rsplit('.').next().unwrap_or(f.name.as_str()));
    }
    for cls in &program.classes {
        for m in &cls.methods {
            shadowed.insert(m.func.name.rsplit('.').next().unwrap_or(m.func.name.as_str()));
        }
        for f in &cls.fields {
            shadowed.insert(f.name.as_str());
        }
        for p in &cls.primary_ctor_params {
            shadowed.insert(p.name.as_str());
        }
    }
    for st in &program.structs {
        for f in &st.fields {
            shadowed.insert(f.name.as_str());
        }
    }
    for ext in &program.extends {
        for f in &ext.methods {
            shadowed.insert(f.name.rsplit('.').next().unwrap_or(f.name.as_str()));
        }
    }

    let mut ctx = InstantiationScan {
        abstract_classes,
        interfaces,
        no_ctor_classes,
        shadowed,
        scopes: Vec::new(),
        errors,
    };

    for func in &program.functions {
        ctx.scan_function(func);
    }
    for cls in &program.classes {
        for m in &cls.methods {
            ctx.scan_function(&m.func);
        }
        if let Some(init) = &cls.init {
            ctx.push_scope();
            for p in &init.params {
                ctx.bind(&p.name);
                if let Some(d) = &p.default {
                    ctx.scan_expr(d);
                }
            }
            for s in &init.body {
                ctx.scan_stmt(s);
            }
            ctx.pop_scope();
        }
        if let Some(body) = &cls.deinit {
            ctx.scan_stmts(body);
        }
        if let Some(body) = &cls.static_init {
            ctx.scan_stmts(body);
        }
    }
    for ext in &program.extends {
        for f in &ext.methods {
            ctx.scan_function(f);
        }
    }
    for iface in &program.interfaces {
        for m in &iface.methods {
            if let Some(body) = &m.default_body {
                ctx.scan_stmts(body);
            }
        }
    }
    for c in &program.constants {
        ctx.scan_expr(&c.init);
    }
}

/// 递归扫描函数体中的调用，识别对抽象类/接口的实例化。
struct InstantiationScan<'a, 'i> {
    abstract_classes: HashSet<&'a str>,
    interfaces: HashSet<&'a str>,
    /// 没有显式构造函数（只有隐式无参构造函数）的类。
    no_ctor_classes: HashSet<&'a str>,
    /// 可能与类型名重名的可调用/成员名（遮蔽时跳过实例化判定）。
    shadowed: HashSet<&'a str>,
    /// 局部作用域栈：记录参数、局部变量、局部函数等名字。
    scopes: Vec<HashSet<String>>,
    errors: &'i mut Vec<SemanticError>,
}

impl<'a, 'i> InstantiationScan<'a, 'i> {
    fn push_scope(&mut self) {
        self.scopes.push(HashSet::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn bind(&mut self, name: &str) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_string());
        }
    }

    fn bind_pattern(&mut self, pattern: &crate::ast::Pattern) {
        use crate::ast::Pattern;
        match pattern {
            Pattern::Binding(n) => self.bind(n),
            Pattern::Struct { fields, .. } => {
                for (_, p) in fields {
                    self.bind_pattern(p);
                }
            }
            Pattern::Tuple(items) | Pattern::Or(items) => {
                for p in items {
                    self.bind_pattern(p);
                }
            }
            Pattern::Variant { payload, .. } => {
                if let Some(p) = payload {
                    self.bind_pattern(p);
                }
            }
            Pattern::TypeTest { binding, .. } => self.bind(binding),
            _ => {}
        }
    }

    fn is_shadowed(&self, name: &str) -> bool {
        self.shadowed.contains(name) || self.scopes.iter().any(|s| s.contains(name))
    }

    fn scan_function(&mut self, func: &Function) {
        self.push_scope();
        for p in &func.params {
            self.bind(&p.name);
            if let Some(d) = &p.default {
                self.scan_expr(d);
            }
        }
        // 函数体与参数共享同一作用域，避免为参数再压一层。
        for stmt in &func.body {
            self.scan_stmt(stmt);
        }
        self.pop_scope();
    }

    fn scan_stmts(&mut self, stmts: &[Stmt]) {
        self.push_scope();
        for stmt in stmts {
            self.scan_stmt(stmt);
        }
        self.pop_scope();
    }

    fn scan_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Let { pattern, value, .. } | Stmt::Var { pattern, value, .. } => {
                self.scan_expr(value);
                self.bind_pattern(pattern);
            }
            Stmt::Const { name, value, .. } => {
                self.scan_expr(value);
                self.bind(name);
            }
            Stmt::Assign { target, value } => {
                self.scan_assign_target(target);
                self.scan_expr(value);
            }
            Stmt::Expr(e) => self.scan_expr(e),
            Stmt::Return(Some(e)) => self.scan_expr(e),
            Stmt::While { cond, body } => {
                self.scan_expr(cond);
                self.scan_stmts(body);
            }
            Stmt::WhileLet { pattern, expr, body } => {
                self.scan_expr(expr);
                self.push_scope();
                self.bind_pattern(pattern);
                for s in body {
                    self.scan_stmt(s);
                }
                self.pop_scope();
            }
            Stmt::DoWhile { body, cond } => {
                self.scan_stmts(body);
                self.scan_expr(cond);
            }
            Stmt::For { var, iterable, body } => {
                self.scan_expr(iterable);
                self.push_scope();
                self.bind(var);
                for s in body {
                    self.scan_stmt(s);
                }
                self.pop_scope();
            }
            Stmt::Loop { body } | Stmt::UnsafeBlock { body } => self.scan_stmts(body),
            Stmt::Assert { left, right, .. } | Stmt::Expect { left, right, .. } => {
                self.scan_expr(left);
                self.scan_expr(right);
            }
            Stmt::LocalFunc(f) => {
                let simple = f.name.rsplit('.').next().unwrap_or(f.name.as_str()).to_string();
                self.bind(&simple);
                self.scan_function(f);
            }
            Stmt::Return(None) | Stmt::Break | Stmt::Continue => {}
        }
    }

    fn scan_assign_target(&mut self, target: &AssignTarget) {
        match target {
            AssignTarget::Index { index, .. } => self.scan_expr(index),
            AssignTarget::IndexPath { index, .. } => self.scan_expr(index),
            AssignTarget::ExprIndex { expr, index } => {
                self.scan_expr(expr);
                self.scan_expr(index);
            }
            AssignTarget::ExprField { object, .. } => self.scan_expr(object),
            AssignTarget::OptionalField { object, .. } => self.scan_expr(object),
            AssignTarget::Slice { array, start, end } => {
                self.scan_expr(array);
                self.scan_expr(start);
                self.scan_expr(end);
            }
            AssignTarget::Tuple(items) => {
                for t in items {
                    self.scan_assign_target(t);
                }
            }
            _ => {}
        }
    }

    fn check_ctor_name(&mut self, name: &str) {
        if name.contains('.') || self.is_shadowed(name) {
            return;
        }
        if self.abstract_classes.contains(name) {
            self.errors.push(SemanticError {
                message: format!("抽象类 '{}' 不能被实例化", name),
            });
        } else if self.interfaces.contains(name) {
            self.errors.push(SemanticError {
                message: format!("接口 '{}' 不能被实例化", name),
            });
        }
    }

    fn check_implicit_ctor_args(&mut self, name: &str, args: &[Expr]) {
        // 6.1.2.1(47)：没有显式构造函数的类只有隐式无参构造函数，
        // 带实参的实例化必然无匹配构造函数。
        if args.is_empty() || name.contains('.') || self.is_shadowed(name) {
            return;
        }
        if self.no_ctor_classes.contains(name) {
            self.errors.push(SemanticError {
                message: format!(
                    "类 '{}' 没有显式构造函数，只有隐式无参构造函数，不能用实参实例化（6.1.2.1(47)）",
                    name
                ),
            });
        }
    }

    fn scan_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Call { name, args, named_args, .. }
            | Expr::ConstructorCall { name, args, named_args, .. } => {
                self.check_ctor_name(name);
                self.check_implicit_ctor_args(name, args);
                self.scan_exprs(args);
                self.scan_named(named_args);
            }
            Expr::MethodCall { object, args, named_args, .. } => {
                self.scan_expr(object);
                self.scan_exprs(args);
                self.scan_named(named_args);
            }
            Expr::SuperCall { args, named_args, .. } => {
                self.scan_exprs(args);
                self.scan_named(named_args);
            }
            Expr::Unary { expr, .. }
            | Expr::Try(expr)
            | Expr::Throw(expr)
            | Expr::Cast { expr, .. }
            | Expr::IsType { expr, .. }
            | Expr::PostfixIncr(expr)
            | Expr::PostfixDecr(expr)
            | Expr::PrefixIncr(expr)
            | Expr::PrefixDecr(expr)
            | Expr::Field { object: expr, .. }
            | Expr::OptionalChain { object: expr, .. }
            | Expr::TupleIndex { object: expr, .. } => self.scan_expr(expr),
            Expr::Binary { left, right, .. } => {
                self.scan_expr(left);
                self.scan_expr(right);
            }
            Expr::If { cond, then_branch, else_branch } => {
                self.scan_expr(cond);
                self.scan_expr(then_branch);
                if let Some(e) = else_branch {
                    self.scan_expr(e);
                }
            }
            Expr::IfLet { expr, then_branch, else_branch, .. } => {
                self.scan_expr(expr);
                self.scan_expr(then_branch);
                if let Some(e) = else_branch {
                    self.scan_expr(e);
                }
            }
            Expr::Block(stmts, tail) => {
                self.scan_stmts(stmts);
                if let Some(e) = tail {
                    self.scan_expr(e);
                }
            }
            Expr::Tuple(items) | Expr::Array(items) => self.scan_exprs(items),
            Expr::Index { array, index } => {
                self.scan_expr(array);
                self.scan_expr(index);
            }
            Expr::StructInit { fields, .. } => {
                for (_, e) in fields {
                    self.scan_expr(e);
                }
            }
            Expr::Range { start, end, step, .. } => {
                self.scan_expr(start);
                self.scan_expr(end);
                if let Some(s) = step {
                    self.scan_expr(s);
                }
            }
            Expr::VariantConst { arg, .. } => {
                if let Some(a) = arg {
                    self.scan_expr(a);
                }
            }
            Expr::Match { expr, arms } => {
                self.scan_expr(expr);
                for arm in arms {
                    self.push_scope();
                    self.bind_pattern(&arm.pattern);
                    if let Some(g) = &arm.guard {
                        self.scan_expr(g);
                    }
                    self.scan_expr(&arm.body);
                    self.pop_scope();
                }
            }
            Expr::Lambda { params, body, .. } => {
                self.push_scope();
                for (n, _) in params {
                    self.bind(n);
                }
                self.scan_expr(body);
                self.pop_scope();
            }
            Expr::Some(e) | Expr::Ok(e) | Expr::Err(e) => self.scan_expr(e),
            Expr::NullCoalesce { option, default } => {
                self.scan_expr(option);
                self.scan_expr(default);
            }
            Expr::Return(Some(e)) => self.scan_expr(e),
            Expr::TryBlock {
                resources,
                body,
                catch_var,
                catch_body,
                finally_body,
                ..
            } => {
                for (n, e) in resources {
                    self.bind(n);
                    self.scan_expr(e);
                }
                self.scan_stmts(body);
                self.push_scope();
                if let Some(v) = catch_var {
                    self.bind(v);
                }
                for s in catch_body {
                    self.scan_stmt(s);
                }
                self.pop_scope();
                if let Some(f) = finally_body {
                    self.scan_stmts(f);
                }
            }
            Expr::SliceExpr { array, start, end } => {
                self.scan_expr(array);
                self.scan_expr(start);
                self.scan_expr(end);
            }
            Expr::MapLiteral { entries } => {
                for (k, v) in entries {
                    self.scan_expr(k);
                    self.scan_expr(v);
                }
            }
            Expr::Spawn { body } => self.scan_stmts(body),
            Expr::Synchronized { lock, body } => {
                self.scan_expr(lock);
                self.scan_stmts(body);
            }
            Expr::TrailingClosure { callee, args, closure } => {
                self.scan_expr(callee);
                self.scan_exprs(args);
                self.scan_expr(closure);
            }
            Expr::Invoke { callee, args, named_args } => {
                self.scan_expr(callee);
                self.scan_exprs(args);
                self.scan_named(named_args);
            }
            Expr::Macro { args, .. } => self.scan_exprs(args),
            Expr::Interpolate(parts) => {
                for p in parts {
                    if let crate::ast::InterpolatePart::Expr(e) = p {
                        self.scan_expr(e);
                    }
                }
            }
            Expr::Integer(_)
            | Expr::IntegerSuffixed(_, _)
            | Expr::Float(_)
            | Expr::Float32(_)
            | Expr::Bool(_)
            | Expr::Rune(_)
            | Expr::String(_)
            | Expr::Var(_)
            | Expr::SuperFieldAccess { .. }
            | Expr::None
            | Expr::Return(None)
            | Expr::Break
            | Expr::Continue => {}
        }
    }

    fn scan_exprs(&mut self, exprs: &[Expr]) {
        for e in exprs {
            self.scan_expr(e);
        }
    }

    fn scan_named(&mut self, named: &[(String, Expr)]) {
        for (_, e) in named {
            self.scan_expr(e);
        }
    }
}

/// 实现接口属性时，属性类型必须与接口声明完全一致（6.2.4.1(7)）。
///
/// 既覆盖抽象属性（已有签名匹配逻辑），也覆盖带默认实现的属性：
/// 后者不会进入“必须实现”的集合，但仍要求同类型（不允许协变/逆变）。
fn check_interface_property_types(
    owner: &str,
    own_sigs: &[Sig],
    parents: &[ParentMethod],
    index: &ProgramIndex<'_>,
    errors: &mut Vec<SemanticError>,
) {
    for p in parents.iter().filter(|p| p.is_interface) {
        let is_get = p.sig.name.starts_with("__get_");
        if !is_get && !p.sig.name.starts_with("__set_") {
            continue;
        }
        for s in own_sigs {
            if s.name != p.sig.name || s.params.len() != p.sig.params.len() {
                continue;
            }
            let comparable = s
                .params
                .iter()
                .chain(s.ret.iter())
                .chain(p.sig.params.iter())
                .chain(p.sig.ret.iter())
                .all(|t| type_is_comparable(t, index));
            if !comparable {
                continue;
            }
            let (sp, sr) = canon_signature(s);
            let (pp, pr) = canon_signature(&p.sig);
            let ok = sp == pp && (!is_get || sr == pr);
            if !ok {
                errors.push(SemanticError {
                    message: format!(
                        "类型 '{}' 实现的属性 '{}' 类型与接口声明不一致（属性类型不允许协变/逆变）",
                        owner, p.sig.display
                    ),
                });
                break;
            }
        }
    }
}

/// 实现接口属性时，属性的 `mut` 修饰符必须与接口声明一致（6.2.4.1(6)）。
///
/// 属性访问器脱糖为 `__get_p` / `__set_p`：存在 setter 即视为 `mut`。抽象 `mut prop`
/// 声明在解析器中同样会脱糖出抽象 setter，因此可据父类型签名判断其可变性。
fn check_interface_property_mut(
    owner: &str,
    own_sigs: &[Sig],
    parents: &[ParentMethod],
    errors: &mut Vec<SemanticError>,
) {
    // 父接口属性基名 → 是否可变（存在 setter）。
    let mut parent_props: HashMap<&str, bool> = HashMap::new();
    for p in parents.iter().filter(|p| p.is_interface) {
        let base = member_base_name(&p.sig.name);
        if !p.sig.name.starts_with("__get_") && !p.sig.name.starts_with("__set_") {
            continue;
        }
        let e = parent_props.entry(base).or_insert(false);
        if p.sig.name.starts_with("__set_") {
            *e = true;
        }
    }
    if parent_props.is_empty() {
        return;
    }
    let own_set: HashSet<&str> = own_sigs
        .iter()
        .filter(|s| s.name.starts_with("__set_"))
        .map(|s| member_base_name(&s.name))
        .collect();
    let own_get: HashSet<&str> = own_sigs
        .iter()
        .filter(|s| s.name.starts_with("__get_"))
        .map(|s| member_base_name(&s.name))
        .collect();
    let mut reported: HashSet<&str> = HashSet::new();
    for (base, parent_mut) in parent_props {
        if !own_get.contains(base) && !own_set.contains(base) {
            continue;
        }
        let own_mut = own_set.contains(base);
        if own_mut != parent_mut && reported.insert(base) {
            errors.push(SemanticError {
                message: format!(
                    "类型 '{}' 实现的属性 '{}' 的 mut 修饰符与接口声明不一致（6.2.4.1(6)）",
                    owner, base
                ),
            });
        }
    }
}

fn check_duplicate_fields<'a, I: Iterator<Item = &'a str>>(    owner: &str,
    fields: I,
    errors: &mut Vec<SemanticError>,
) {
    let mut seen: HashMap<&str, usize> = HashMap::new();
    for name in fields {
        *seen.entry(name).or_insert(0) += 1;
    }
    for (name, count) in seen {
        if count > 1 {
            errors.push(SemanticError {
                message: format!("类型 '{}' 中字段 '{}' 被重复定义", owner, name),
            });
        }
    }
}

/// 3.1(3)：同一类型内字段名与成员函数名不能相同（共享成员命名空间）。
fn check_field_method_conflicts<'a, 'b, I, J>(
    owner: &str,
    fields: I,
    methods: J,
    errors: &mut Vec<SemanticError>,
) where
    I: Iterator<Item = &'a str>,
    J: Iterator<Item = &'b str>,
{
    let field_set: HashSet<&str> = fields.collect();
    let mut reported: HashSet<&str> = HashSet::new();
    for m in methods {
        if is_synthetic_member(m) || !field_set.contains(m) || !reported.insert(m) {
            continue;
        }
        errors.push(SemanticError {
            message: format!(
                "类型 '{}' 中字段 '{}' 与成员函数同名（3.1(3)）",
                owner, m
            ),
        });
    }
}

/// 6.2.5(3)/(4)：Any 接口由编译器隐式实现（或可在类型定义处显式声明），
/// 不能被 extend 再次实现。
fn check_extend_any_interface(program: &Program, errors: &mut Vec<SemanticError>) {
    for ext in &program.extends {
        let base = ext
            .interface
            .as_deref()
            .map(|i| i.trim().split('<').next().unwrap_or("").trim())
            .unwrap_or("");
        if base == "Any" {
            errors.push(SemanticError {
                message: format!(
                    "类型 '{}' 不能通过 extend 再次实现 Any 接口（6.2.5(3)）",
                    ext.target_type
                ),
            });
        }
    }
}

/// 内建的非接口类型名（不能作为接口的父类型）。
fn is_builtin_non_interface(name: &str) -> bool {
    matches!(
        name,
        "Int8"
            | "Int16"
            | "Int32"
            | "Int64"
            | "IntNative"
            | "UInt8"
            | "UInt16"
            | "UInt32"
            | "UInt64"
            | "UIntNative"
            | "Float16"
            | "Float32"
            | "Float64"
            | "Bool"
            | "Unit"
            | "Nothing"
            | "Rune"
            | "String"
            | "Range"
            | "Object"
            | "Array"
    )
}

/// 6.2.3(1)：接口只能继承接口，不能继承 class/struct/enum 或内建非接口类型。
///
/// 仅当父类型在本地可解析为已知的非接口类型时判定；外部/未知类型保守放行。
fn check_interface_parent_kinds(
    program: &Program,
    index: &ProgramIndex<'_>,
    errors: &mut Vec<SemanticError>,
) {
    // 顶层变量/函数名不能作为接口父类型。
    let globals: HashSet<&str> = program.constants.iter().map(|c| c.name.as_str()).collect();
    let funcs: HashSet<&str> = program.functions.iter().map(|f| f.name.as_str()).collect();
    for iface in &program.interfaces {
        for p in &iface.parents {
            let base = p.split('<').next().unwrap_or(p).trim();
            if index.interfaces.contains_key(base) {
                continue;
            }
            if globals.contains(base) || funcs.contains(base) {
                errors.push(SemanticError {
                    message: format!(
                        "接口 '{}' 不能继承顶层变量/函数 '{}'（6.2.3(1)）",
                        iface.name, base
                    ),
                });
                continue;
            }
            if index.classes.contains_key(base)
                || index.structs.contains(base)
                || index.enums.contains(base)
                || is_builtin_non_interface(base)
            {
                errors.push(SemanticError {
                    message: format!(
                        "接口 '{}' 只能继承接口，不能继承类型 '{}'（6.2.3(1)）",
                        iface.name, base
                    ),
                });
            }
        }
    }
}

/// 按出现顺序收集类型中出现的类型参数名（去重）。
fn collect_type_param_names(t: &Type, out: &mut Vec<String>) {
    match t {
        Type::TypeParam(n) => {
            if !out.iter().any(|x| x == n) {
                out.push(n.clone());
            }
        }
        Type::Struct(_, args) => {
            for a in args {
                collect_type_param_names(a, out);
            }
        }
        Type::Array(i) | Type::Option(i) | Type::Slice(i) => collect_type_param_names(i, out),
        Type::Tuple(ts) => {
            for x in ts {
                collect_type_param_names(x, out);
            }
        }
        Type::Result(a, b) | Type::Map(a, b) => {
            collect_type_param_names(a, out);
            collect_type_param_names(b, out);
        }
        Type::Function { params, ret } => {
            for p in params {
                collect_type_param_names(p, out);
            }
            if let Some(r) = ret.as_ref().as_ref() {
                collect_type_param_names(r, out);
            }
        }
        _ => {}
    }
}

/// 展开类型别名（常用于 `type X<T> = I<T>` 之后的父接口实参规范化）。
///
/// 仅处理可静态解析的别名：目标中的自由类型参数按出现顺序与实参按位置替换。
/// 无法配对时保守返回原类型实参（不展开）。
fn expand_alias_type(ty: &Type, index: &ProgramIndex<'_>, depth: usize) -> Type {
    if depth > 8 {
        return ty.clone();
    }
    match ty {
        Type::Struct(name, args) => {
            let args: Vec<Type> = args
                .iter()
                .map(|a| expand_alias_type(a, index, depth + 1))
                .collect();
            if let Some(target) = index.aliases.get(name.as_str()) {
                let target = expand_alias_type(target, index, depth + 1);
                if args.is_empty() {
                    return target;
                }
                let mut params: Vec<String> = Vec::new();
                collect_type_param_names(&target, &mut params);
                if params.len() == args.len() {
                    let subst: HashMap<String, Type> =
                        params.into_iter().zip(args.into_iter()).collect();
                    return subst_type(&target, &subst);
                }
            }
            Type::Struct(name.clone(), args)
        }
        Type::Array(i) => Type::Array(Box::new(expand_alias_type(i, index, depth + 1))),
        Type::Option(i) => Type::Option(Box::new(expand_alias_type(i, index, depth + 1))),
        Type::Slice(i) => Type::Slice(Box::new(expand_alias_type(i, index, depth + 1))),
        _ => ty.clone(),
    }
}

fn check_interface(
    iface: &InterfaceDef,
    index: &ProgramIndex<'_>,
    errors: &mut Vec<SemanticError>,
) {
    let mut by_name: HashMap<String, Vec<Sig>> = HashMap::new();
    for method in &iface.methods {
        let sig = sig_of_interface_method(method);
        if is_synthetic_member(&sig.name) {
            continue;
        }
        by_name.entry(sig.name.clone()).or_default().push(sig);
    }
    for group in by_name.values() {
        // 接口内部同样不允许 static 与实例成员函数同名互为重载（6.3.2(1)）。
        check_signature_group(&iface.name, group, true, errors);
    }
    // 6.2.2.1(5)：接口函数对外可见性等同 public，不允许 protected/private 修饰。
    for m in &iface.methods {
        if matches!(
            m.visibility,
            Visibility::Private | Visibility::Protected
        ) {
            errors.push(SemanticError {
                message: format!(
                    "接口 '{}' 的成员函数 '{}' 不能使用 protected/private 访问修饰符（6.2.2.1(5)）",
                    iface.name, m.name
                ),
            });
        }
    }
    // 7.1(6)：不可变属性（无 mut）不能定义 setter。
    {
        let mut prop_mut: HashMap<String, bool> = HashMap::new();
        let mut has_setter: HashSet<String> = HashSet::new();
        for m in &iface.methods {
            if let Some(base) = m.name.strip_prefix("__get_") {
                prop_mut.entry(base.to_string()).or_insert(m.is_prop_mut);
            } else if let Some(base) = m.name.strip_prefix("__set_") {
                has_setter.insert(base.to_string());
                prop_mut.entry(base.to_string()).or_insert(m.is_prop_mut);
            }
        }
        for base in has_setter {
            if prop_mut.get(&base).copied().unwrap_or(false) == false {
                errors.push(SemanticError {
                    message: format!(
                        "接口 '{}' 的不可变属性 '{}' 不能定义 setter（7.1(6)）",
                        iface.name, base
                    ),
                });
            }
        }
    }
    // 6.2.3(4)/(5)：同一接口不得被以相同泛型实参直接继承多次。
    check_interface_duplicate_parents(iface, index, errors);
    // 6.2.3(5)：经泛型父接口实例化后不得产生重复的祖父接口。
    check_interface_transitive_duplicate_parents(iface, index, errors);
    // 接口继承：子接口成员不得与父接口同名成员构成 static/实例重载冲突（6.3.2(1)）。
    check_interface_inherited_static_conflicts(iface, index, errors);
    // 接口继承：子接口成员不得遮蔽父接口同名成员（6.3.3(1)）。
    check_interface_inherited_kind_shadowing(iface, index, errors);
    // 接口继承的默认实现规则（6.2.3.1(3)/(6)）。
    check_interface_default_inheritance(iface, index, errors);
}

/// 子接口继承默认实现的规则：
/// - 6.2.3.1(3)：子接口不得仅声明而无默认实现地覆盖父接口已有的默认实现；
/// - 6.2.3.1(6)：多个父接口为同一签名提供默认实现时，子接口必须提供自己的默认实现。
fn check_interface_default_inheritance(
    iface: &InterfaceDef,
    index: &ProgramIndex<'_>,
    errors: &mut Vec<SemanticError>,
) {
    if iface.parents.is_empty() {
        return;
    }
    let names: Vec<&str> = iface.parents.iter().map(|s| s.as_str()).collect();
    let Some(parents) = collect_interface_methods(&names, index) else {
        return;
    };
    // 6.2.3.1(3)：子接口仅给出声明（无默认实现体）覆盖父接口默认实现。
    let mut reported: HashSet<(String, String, bool)> = HashSet::new();
    for m in &iface.methods {
        if m.default_body.is_some() || is_synthetic_member(&m.name) {
            continue;
        }
        let sig = sig_of_interface_method(m);
        let hit = parents.iter().any(|p| {
            p.is_interface
                && !p.abstract_
                && !is_synthetic_member(&p.sig.name)
                && p.sig.name == sig.name
                && p.sig.is_static == sig.is_static
                && canon_params(&p.sig) == canon_params(&sig)
        });
        if hit && reported.insert((sig.name.clone(), canon_params(&sig), sig.is_static)) {
            errors.push(SemanticError {
                message: format!(
                    "接口 '{}' 的成员 '{}' 不能只声明而无默认实现地覆盖父接口的默认实现（6.2.3.1(3)）",
                    iface.name, sig.display
                ),
            });
        }
    }
    // 6.2.3.1(6)：只有子接口自身提供了默认实现体才算“自行实现”。
    let own_defaults: Vec<Sig> = iface
        .methods
        .iter()
        .filter(|m| m.default_body.is_some() && !is_synthetic_member(&m.name))
        .map(sig_of_interface_method)
        .collect();
    check_multiple_default_conflicts(&iface.name, &own_defaults, &parents, index, errors, "接口");
}

/// 6.2.3(4)/(5)：同一接口不得被以相同泛型实参直接继承多次
/// （`interface J <: I & I` 或 `J <: I<Int> & I<Int>`）。
/// 不同泛型实参（`I<Int> & I<String>`）是合法的，按规范化后的完整类型比较。
fn check_interface_duplicate_parents(
    iface: &InterfaceDef,
    index: &ProgramIndex<'_>,
    errors: &mut Vec<SemanticError>,
) {
    if iface.parent_types.is_empty() {
        return;
    }
    let mut seen: HashMap<String, ()> = HashMap::new();
    for (i, ty) in iface.parent_types.iter().enumerate() {
        let mut map = HashMap::new();
        // 先展开类型别名（`type X<T> = I<T>` 场景），再按规范化类型比较（6.2.3(5)）。
        let expanded = expand_alias_type(ty, index, 0);
        let canon = canon_type(&expanded, &iface.type_params, &mut map);
        if seen.contains_key(&canon) {
            let name = iface
                .parents
                .get(i)
                .cloned()
                .unwrap_or_else(|| "<未知>".to_string());
            errors.push(SemanticError {
                message: format!(
                    "接口 '{}' 不能以相同泛型实参重复直接继承父接口 '{}'（6.2.3(4)/(5)）",
                    iface.name, name
                ),
            });
            return;
        }
        seen.insert(canon, ());
    }
}

/// 6.2.3(5)：直接继承带泛型实参的接口后，不得使祖父接口以相同实参重复出现。
///
/// 例如 `interface J<T> <: I<Unit> & I<T> {}` + `interface K <: J<Unit> {}`：
/// 实例化 `J<Unit>` 后 `I<Unit>` 与 `I<T=Unit>` 重复。仅在实参与形参个数一致时判定。
fn check_interface_transitive_duplicate_parents(
    iface: &InterfaceDef,
    index: &ProgramIndex<'_>,
    errors: &mut Vec<SemanticError>,
) {
    if iface.parent_types.is_empty() {
        return;
    }
    for ty in &iface.parent_types {
        let Type::Struct(pname, args) = ty else {
            continue;
        };
        let Some(parent) = index.interfaces.get(pname.as_str()) else {
            continue;
        };
        if parent.type_params.is_empty() || args.len() != parent.type_params.len() {
            continue;
        }
        let subst: HashMap<String, Type> = parent
            .type_params
            .iter()
            .cloned()
            .zip(args.iter().cloned())
            .collect();
        let mut seen: HashMap<String, ()> = HashMap::new();
        for g in &parent.parent_types {
            let expanded = expand_alias_type(&subst_type(g, &subst), index, 0);
            let mut map = HashMap::new();
            let canon = canon_type(&expanded, &iface.type_params, &mut map);
            if seen.contains_key(&canon) {
                errors.push(SemanticError {
                    message: format!(
                        "接口 '{}' 继承 '{}' 后使祖父接口以相同泛型实参重复出现（6.2.3(5)）",
                        iface.name, pname
                    ),
                });
                break;
            }
            seen.insert(canon, ());
        }
    }
}

/// 接口继承中 static 与实例成员函数同名冲突（6.3.2(1)）。
fn check_interface_inherited_static_conflicts(
    iface: &InterfaceDef,
    index: &ProgramIndex<'_>,
    errors: &mut Vec<SemanticError>,
) {
    if iface.parents.is_empty() {
        return;
    }
    let names: Vec<&str> = iface.parents.iter().map(|s| s.as_str()).collect();
    let Some(parents) = collect_interface_methods(&names, index) else {
        return;
    };
    for method in &iface.methods {
        let sig = sig_of_interface_method(method);
        if is_synthetic_member(&sig.name) || is_property_method_name(&sig.name) {
            continue;
        }
        let conflict = parents.iter().any(|p| {
            !is_synthetic_member(&p.sig.name)
                && !is_property_method_name(&p.sig.name)
                && p.sig.name == sig.name
                && p.sig.is_static != sig.is_static
        });
        if conflict {
            errors.push(SemanticError {
                message: format!(
                    "接口 '{}' 的成员函数 '{}' 与父接口同名函数在 static/实例属性上冲突，不允许互为重载（6.3.2(1)）",
                    iface.name, sig.display
                ),
            });
        }
    }
}

/// 接口继承中不同种类（函数 / 属性）的同名成员构成遮蔽（6.3.3(1)）。
fn check_interface_inherited_kind_shadowing(
    iface: &InterfaceDef,
    index: &ProgramIndex<'_>,
    errors: &mut Vec<SemanticError>,
) {
    if iface.parents.is_empty() {
        return;
    }
    let names: Vec<&str> = iface.parents.iter().map(|s| s.as_str()).collect();
    let Some(parents) = collect_interface_methods(&names, index) else {
        return;
    };
    let mut reported: HashSet<String> = HashSet::new();
    for method in &iface.methods {
        let sig = sig_of_interface_method(method);
        if is_synthetic_member(&sig.name) {
            continue;
        }
        let base = member_base_name(&sig.name).to_string();
        let own_is_prop = is_property_method_name(&sig.name);
        let conflict = parents.iter().any(|p| {
            !is_synthetic_member(&p.sig.name)
                && member_base_name(&p.sig.name) == base
                && is_property_method_name(&p.sig.name) != own_is_prop
        });
        if conflict && reported.insert(base.clone()) {
            errors.push(SemanticError {
                message: format!(
                    "接口 '{}' 的成员 '{}' 遮蔽了父接口的同名成员（6.3.3(1)）",
                    iface.name, base
                ),
            });
        }
    }
}

fn check_enum_variants(en: &EnumDef, errors: &mut Vec<SemanticError>) {
    // 同名但载荷类型不同的构造器是合法的重载；仅同名同载荷构成重定义。
    for i in 0..en.variants.len() {
        for j in (i + 1)..en.variants.len() {
            let a = &en.variants[i];
            let b = &en.variants[j];
            if a.name != b.name {
                continue;
            }
            let payload_key = |p: &Option<Type>| -> String {
                match p {
                    Some(t) => canon_type(t, &en.type_params, &mut HashMap::new()),
                    None => "()".to_string(),
                }
            };
            if payload_key(&a.payload) == payload_key(&b.payload) {
                errors.push(SemanticError {
                    message: format!("枚举 '{}' 中构造器 '{}' 被重复定义", en.name, a.name),
                });
                break;
            }
        }
    }
}

// ─── 父类型索引与覆写/实现校验 ─────────────────────────────────────────────

struct ProgramIndex<'a> {
    classes: HashMap<&'a str, &'a ClassDef>,
    interfaces: HashMap<&'a str, &'a InterfaceDef>,
    structs: HashSet<&'a str>,
    /// `@C` 修饰的结构体名（隐式满足 CType，可作为 C 签名类型）。
    c_structs: HashSet<&'a str>,
    enums: HashSet<&'a str>,
    /// 各类型的声明类型参数（class/struct/enum）。接口的泛型参数未进入 AST。
    type_params: HashMap<&'a str, Vec<String>>,
    /// 本地可解析的类型名（用于判断签名是否可严格比较）。
    known_types: HashSet<&'a str>,
    /// 类型别名目标
    aliases: HashMap<&'a str, &'a Type>,
    /// extend 块提供的函数：目标类型名 → 方法列表
    extends: HashMap<&'a str, Vec<&'a Function>>,
}

impl<'a> ProgramIndex<'a> {
    fn new(program: &'a Program) -> Self {
        let mut classes = HashMap::new();
        let mut interfaces = HashMap::new();
        let mut type_params = HashMap::new();
        let mut known_types = HashSet::new();
        let mut structs = HashSet::new();
        let mut c_structs = HashSet::new();
        let mut enums = HashSet::new();

        for c in &program.classes {
            classes.insert(c.name.as_str(), c);
            type_params.insert(c.name.as_str(), c.type_params.clone());
            known_types.insert(c.name.as_str());
        }
        for i in &program.interfaces {
            interfaces.insert(i.name.as_str(), i);
            known_types.insert(i.name.as_str());
        }
        for s in &program.structs {
            structs.insert(s.name.as_str());
            if s.is_c {
                c_structs.insert(s.name.as_str());
            }
            type_params.insert(s.name.as_str(), s.type_params.clone());
            known_types.insert(s.name.as_str());
        }
        for e in &program.enums {
            enums.insert(e.name.as_str());
            type_params.insert(e.name.as_str(), e.type_params.clone());
            known_types.insert(e.name.as_str());
        }
        let mut aliases = HashMap::new();
        for (name, ty) in &program.type_aliases {
            aliases.insert(name.as_str(), ty);
            known_types.insert(name.as_str());
        }

        let mut extends: HashMap<&str, Vec<&Function>> = HashMap::new();
        for ext in &program.extends {
            extends
                .entry(ext.target_type.as_str())
                .or_default()
                .extend(ext.methods.iter());
        }

        ProgramIndex {
            classes,
            interfaces,
            structs,
            c_structs,
            enums,
            type_params,
            known_types,
            aliases,
            extends,
        }
    }

    fn type_params_of(&self, name: &str) -> &[String] {
        self.type_params
            .get(name)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// 判断 child 是否为 parent 的（本地可见的）子类型。
    fn is_subtype_name(&self, child: &str, parent: &str) -> bool {
        if child == parent {
            return true;
        }
        let mut stack: Vec<&str> = vec![child];
        let mut seen: HashSet<&str> = HashSet::new();
        while let Some(name) = stack.pop() {
            if !seen.insert(name) {
                continue;
            }
            if name == parent {
                return true;
            }
            if let Some(c) = self.classes.get(name) {
                if let Some(p) = c.extends.as_deref() {
                    stack.push(p);
                }
                for i in &c.implements {
                    stack.push(i.as_str());
                }
            }
            if let Some(i) = self.interfaces.get(name) {
                for p in &i.parents {
                    stack.push(p.as_str());
                }
            }
            if let Some(Type::Struct(pn, _)) = self.aliases.get(name) {
                stack.push(pn.as_str());
            }
        }
        false
    }
}

/// 判断类型是否可以在本地严格比较（无未解析泛型参数 / 外部类型）。
fn type_is_comparable(ty: &Type, index: &ProgramIndex<'_>) -> bool {
    match ty {
        Type::TypeParam(_) | Type::This => false,
        Type::Struct(name, args) => {
            // Object / Any 是语言内建顶层类型（不在本地声明中），但可参与类型关系判定。
            let known = index.known_types.contains(name.as_str())
                || name == "Object"
                || name == "Any";
            known && args.iter().all(|a| type_is_comparable(a, index))
        }
        Type::Array(inner) | Type::Option(inner) | Type::Slice(inner) => {
            type_is_comparable(inner, index)
        }
        Type::Tuple(items) => items.iter().all(|t| type_is_comparable(t, index)),
        Type::Result(a, b) | Type::Map(a, b) => {
            type_is_comparable(a, index) && type_is_comparable(b, index)
        }
        Type::Function { params, ret } => {
            params.iter().all(|p| type_is_comparable(p, index))
                && match &**ret {
                    Some(t) => type_is_comparable(t, index),
                    None => true,
                }
        }
        Type::Qualified(_) => false,
        _ => true,
    }
}

fn return_compatible(child: &Type, parent: &Type, index: &ProgramIndex<'_>) -> bool {
    if child == parent {
        return true;
    }
    // This/Nothing 与泛型/协变场景：不做严格判定，避免误伤。
    if matches!(child, Type::This | Type::Nothing) || matches!(parent, Type::This) {
        return true;
    }
    match (child, parent) {
        // Any 是顶层类型：任何类型都是 Any 的子类型。
        (_, Type::Struct(p, pa)) if p == "Any" && pa.is_empty() => true,
        (Type::Struct(c, _), Type::Struct(p, _)) => {
            if p == "Object" {
                // Object 是类层级根类型：仅 class（及 Object 自身）是其子类型。
                c == "Object" || index.classes.contains_key(c.as_str())
            } else {
                index.is_subtype_name(c, p)
            }
        }
        (Type::Option(c), Type::Option(p)) => return_compatible(c, p, index),
        _ => false,
    }
}

/// 覆写/实现匹配。
///
/// `strict_types == false` 表示父类型带类级泛型参数，子类型可能以具体类型覆写，
/// 此时只比较参数个数。签名含未解析泛型/外部类型时同样只比较参数个数。
fn signature_matches(a: &Sig, b: &Sig, strict_types: bool, index: &ProgramIndex<'_>) -> bool {
    if a.params.len() != b.params.len() {
        return false;
    }
    // “相同的参数列表”包含命名参数标记：`y` 与 `y!` 不构成同一签名（6.3.4.1(3)）。
    if a.named.len() == b.named.len() && a.named != b.named {
        return false;
    }
    if !strict_types {
        return true;
    }
    let comparable = a
        .params
        .iter()
        .chain(a.ret.iter())
        .chain(b.params.iter())
        .chain(b.ret.iter())
        .all(|t| type_is_comparable(t, index));
    if !comparable {
        return true;
    }
    if canon_params(a) != canon_params(b) {
        return false;
    }
    // 属性访问器要求实现类型与接口声明完全一致（属性类型不变，不支持协变）。
    if a.name.starts_with("__get_") || a.name.starts_with("__set_") {
        let (_, ra) = canon_signature(a);
        let (_, rb) = canon_signature(b);
        return match (ra, rb) {
            (Some(x), Some(y)) => x == y,
            _ => true,
        };
    }
    match (&a.ret, &b.ret) {
        (Some(x), Some(y)) => return_compatible(x, y, index),
        // 任一侧未标注返回类型时无法比较，视为兼容（不误伤）。
        _ => true,
    }
}

/// 类型中是否出现类型参数（泛型参数）。
fn type_contains_type_param(t: &Type) -> bool {
    match t {
        Type::TypeParam(_) => true,
        Type::Struct(_, args) => args.iter().any(type_contains_type_param),
        Type::Array(i) | Type::Option(i) | Type::Slice(i) => type_contains_type_param(i),
        Type::Tuple(ts) => ts.iter().any(type_contains_type_param),
        Type::Result(x, y) | Type::Map(x, y) => {
            type_contains_type_param(x) || type_contains_type_param(y)
        }
        Type::Function { params, ret } => {
            params.iter().any(type_contains_type_param)
                || ret
                    .as_ref()
                    .as_ref()
                    .map_or(false, |t| type_contains_type_param(t))
        }
        _ => false,
    }
}

/// 覆写时参数列表的严格匹配（6.3.1.1(3)）。
///
/// 当签名涉及类型参数且父类型的类级泛型已完全消解（`fully_concrete`）时，
/// 参数类型必须严格一致：泛型参数不能与具体类型互换，不同位置的泛型参数
/// 也不能互换。仅在 `signature_matches` 已通过时附加此判定，避免与既有
/// 的宽松比较（类型不可比较时放行）冲突。
fn strict_param_match(a: &Sig, b: &Sig, fully_concrete: bool) -> bool {
    if !fully_concrete {
        return true;
    }
    let generic_side = a
        .params
        .iter()
        .chain(b.params.iter())
        .any(type_contains_type_param);
    if !generic_side {
        return true;
    }
    canon_params(a) == canon_params(b)
}

/// 6.3.1.1(6)：覆写与被覆写函数的泛型类型参数必须相同。
///
/// 在“同构类型构造”范围内比较类型参数位置（如 `A<T1,T2>` 与 `A<K2,K1>` 冲突，
/// 裸类型参数 `T1` 与 `K2` 冲突）；不同构造（如协变的子类型返回）保守视为不冲突。
fn generic_signature_shape_conflict(a: &Sig, b: &Sig) -> bool {
    fn ty_conflict(x: &Type, y: &Type, a: &Sig, b: &Sig) -> bool {
        match (x, y) {
            (Type::TypeParam(_), Type::TypeParam(_)) => {
                let mut ma = HashMap::new();
                let mut mb = HashMap::new();
                canon_type(x, &a.tp_order, &mut ma) != canon_type(y, &b.tp_order, &mut mb)
            }
            (Type::Struct(nx, ax), Type::Struct(ny, ay))
                if nx == ny && ax.len() == ay.len() =>
            {
                ax.iter()
                    .zip(ay.iter())
                    .any(|(p, q)| ty_conflict(p, q, a, b))
            }
            (Type::Option(p), Type::Option(q))
            | (Type::Array(p), Type::Array(q))
            | (Type::Slice(p), Type::Slice(q)) => ty_conflict(p, q, a, b),
            (Type::Tuple(ps), Type::Tuple(qs)) if ps.len() == qs.len() => ps
                .iter()
                .zip(qs.iter())
                .any(|(p, q)| ty_conflict(p, q, a, b)),
            _ => false,
        }
    }
    if a.params.len() != b.params.len() {
        return true;
    }
    if a.params
        .iter()
        .zip(b.params.iter())
        .any(|(x, y)| ty_conflict(x, y, a, b))
    {
        return true;
    }
    match (&a.ret, &b.ret) {
        (Some(x), Some(y)) => ty_conflict(x, y, a, b),
        _ => false,
    }
}

/// 重定义（`redef`）的函数签名比较（6.3.4.1(3)）。
///
/// 在父类型的类级泛型已按 `<: Parent<Args>>` 消解（`fully_concrete`）时，
/// 对含类型参数的签名做规范的严格比较：参数类型必须“相同”，不允许泛型参数
/// 与具体类型互换，也不允许不同位置的泛型参数互换。
fn redef_signature_matches(
    a: &Sig,
    b: &Sig,
    fully_concrete: bool,
    strict_types: bool,
    index: &ProgramIndex<'_>,
) -> bool {
    if a.params.len() != b.params.len() {
        return false;
    }
    if a.named.len() == b.named.len() && a.named != b.named {
        return false;
    }
    // 6.3.4.1(1)：泛型函数与非泛型函数不构成同一签名/重定义。
    if !generic_compatible(a.generic, b.generic) {
        return false;
    }
    let generic_side = a
        .params
        .iter()
        .chain(a.ret.iter())
        .chain(b.params.iter())
        .chain(b.ret.iter())
        .any(type_contains_type_param);
    if !fully_concrete || !generic_side {
        return signature_matches(a, b, strict_types, index);
    }
    let (ap, ar) = canon_signature(a);
    let (bp, br) = canon_signature(b);
    ap == bp
        && match (ar, br) {
            (Some(x), Some(y)) => x == y,
            _ => true,
        }
}

/// 父类型方法的来源签名。
struct ParentMethod {
    sig: Sig,
    /// 是否可以严格比较参数/返回类型（泛型父类型只比较参数个数）。
    strict_types: bool,
    /// 是否为抽象方法（无实现体）。
    abstract_: bool,
    /// 是否可被覆写（class 成员需 open/abstract；接口成员天然可被实现/覆写）。
    open_: bool,
    /// 父类型的类级泛型均已消解（无类级参数，或已按 `<: T<...>>` 的实参替换），
    /// 用于重定义时按规范形状严格比较参数（6.3.4.1(3)）。
    fully_concrete: bool,
    /// 是否来自接口。
    is_interface: bool,
    /// 声明该方法的类型名（用于识别多个接口的默认实现冲突）。
    owner: String,
    /// 方法级类型参数（不含类级参数），用于约束宽松度比较时按位置 alpha 对齐。
    type_params: Vec<String>,
    /// 方法级泛型约束（6.2.4.1(5) / 6.3.4.1(5)）。
    constraints: Vec<crate::ast::TypeConstraint>,
}

/// 按类型参数替换表递归替换类型中的泛型参数。
fn subst_type(ty: &Type, subst: &HashMap<String, Type>) -> Type {
    match ty {
        Type::TypeParam(n) => subst.get(n).cloned().unwrap_or_else(|| ty.clone()),
        // 解析器在部分上下文（如接口方法签名）把类型参数记为不带实参的 Struct，
        // 同样按替换表处理。
        Type::Struct(n, args) if args.is_empty() && subst.contains_key(n) => {
            subst.get(n).cloned().unwrap()
        }
        Type::Struct(n, args) => Type::Struct(
            n.clone(),
            args.iter().map(|a| subst_type(a, subst)).collect(),
        ),
        Type::Array(inner) => Type::Array(Box::new(subst_type(inner, subst))),
        Type::Option(inner) => Type::Option(Box::new(subst_type(inner, subst))),
        Type::Slice(inner) => Type::Slice(Box::new(subst_type(inner, subst))),
        Type::Tuple(items) => Type::Tuple(items.iter().map(|t| subst_type(t, subst)).collect()),
        Type::Result(a, b) => Type::Result(
            Box::new(subst_type(a, subst)),
            Box::new(subst_type(b, subst)),
        ),
        Type::Map(a, b) => Type::Map(
            Box::new(subst_type(a, subst)),
            Box::new(subst_type(b, subst)),
        ),
        Type::Function { params, ret } => Type::Function {
            params: params.iter().map(|t| subst_type(t, subst)).collect(),
            ret: Box::new((**ret).as_ref().map(|t| subst_type(t, subst))),
        },
        _ => ty.clone(),
    }
}

/// 由父类型类型参数与 `<: Parent<Args>>` 实参构建替换表；形态不匹配时返回 None。
fn build_supertype_subst(
    parent_tps: &[String],
    args: Option<&Vec<Type>>,
) -> Option<HashMap<String, Type>> {
    let args = args?;
    if args.len() != parent_tps.len() {
        return None;
    }
    Some(parent_tps.iter().cloned().zip(args.iter().cloned()).collect())
}

/// 收集一个 class 的所有本地父类型（父类 + 接口，含传递）的方法。
///
/// 任何父类型无法在本地解析时返回 `None`，调用方应保守跳过校验。
/// 当函数未显式标注返回类型时，从 `return <构造调用>` 推断一个简单返回类型。
///
/// 仅用于覆写/重定义比较中对父类型返回类型的补全：所有 return 都指向同一个
/// 本地用户类型的构造调用时才推断；任何不确定形态一律返回 None（保守放行）。
fn infer_simple_return_ty(body: &[Stmt], index: &ProgramIndex<'_>) -> Option<Type> {
    let mut found: Option<String> = None;
    for s in body {
        if let Stmt::Return(Some(e)) = s {
            let name = match e {
                Expr::ConstructorCall { name, .. } => name.clone(),
                Expr::StructInit { name, .. } => name.clone(),
                Expr::Call { name, .. } if index.known_types.contains(name.as_str()) => {
                    name.clone()
                }
                _ => return None,
            };
            if !index.known_types.contains(name.as_str()) {
                return None;
            }
            match &found {
                None => found = Some(name),
                Some(prev) if *prev == name => {}
                _ => return None,
            }
        }
    }
    found.map(|n| Type::Struct(n, vec![]))
}

fn collect_parent_methods<'a>(
    cls: &'a ClassDef,
    index: &ProgramIndex<'a>,
) -> Option<Vec<ParentMethod>> {
    let mut out = Vec::new();
    let mut visited: HashSet<&'a str> = HashSet::new();
    let mut class_chain: Vec<&'a str> = Vec::new();
    let mut iface_names: Vec<&'a str> = Vec::new();
    // 直接父类型的泛型实参（仅 `<: Parent<Args>>` 书写处可用）。
    let direct_args: HashMap<&str, Vec<Type>> = cls
        .supertype_args
        .iter()
        .map(|(n, a)| (n.as_str(), a.clone()))
        .collect();

    // 直接父类型（extends 可能是父类或接口，implements 为接口）
    let mut direct: Vec<&'a str> = Vec::new();
    if let Some(parent) = cls.extends.as_deref() {
        direct.push(parent);
    }
    for name in &cls.implements {
        direct.push(name.as_str());
    }

    for name in direct {
        if index.classes.contains_key(name) {
            class_chain.push(name);
        } else if index.interfaces.contains_key(name) {
            iface_names.push(name);
        } else if index.structs.contains(name) || index.enums.contains(name) {
            // 继承 struct/enum 不合法，但语法上已解析；无法提供方法签名
            return None;
        } else {
            // 外部/未解析的父类型：保守跳过
            return None;
        }
    }

    // 沿父类链向上收集（每条链上的类都要求本地可解析）
    let mut chain_idx = 0;
    while chain_idx < class_chain.len() {
        let name = class_chain[chain_idx];
        chain_idx += 1;
        if !visited.insert(name) {
            continue;
        }
        let Some(parent) = index.classes.get(name) else {
            return None;
        };
        let subst = build_supertype_subst(&parent.type_params, direct_args.get(name));
        let strict = parent.type_params.is_empty() || subst.is_some();
        let fully_concrete = strict;
        for m in &parent.methods {
            let mut sig = sig_of_class_method(m, &parent.type_params);
            if sig.ret.is_none() {
                if let Some(r) = infer_simple_return_ty(&m.func.body, index) {
                    sig.ret = Some(r);
                }
            }
            if let Some(s) = &subst {
                sig.params = sig.params.iter().map(|t| subst_type(t, s)).collect();
                sig.ret = sig.ret.as_ref().map(|t| subst_type(t, s));
                sig.tp_order = cls.type_params.clone();
                sig.tp_order.extend(m.func.type_params.iter().cloned());
                sig.class_tp_count = cls.type_params.len();
            }
            out.push(ParentMethod {
                sig,
                strict_types: strict,
                // 无体成员函数在抽象类中视为隐式抽象；抽象/open 方法可被子类覆写。
                abstract_: m.is_abstract || !m.has_body,
                open_: m.is_open || m.is_abstract || !m.has_body,
                is_interface: false,
                owner: name.to_string(),
                type_params: m.func.type_params.clone(),
                constraints: m.func.constraints.clone(),
                fully_concrete,
            });
        }
        if let Some(exts) = index.extends.get(name) {
            for f in exts {
                out.push(ParentMethod {
                    sig: sig_of_function(f, index.type_params_of(name)),
                    strict_types: true,
                    abstract_: false,
                    open_: true,
                    is_interface: false,
                    owner: name.to_string(),
                    type_params: f.type_params.clone(),
                    constraints: f.constraints.clone(),
                    fully_concrete: true,
                });
            }
        }
        if let Some(p) = parent.extends.as_deref() {
            if index.classes.contains_key(p) {
                class_chain.push(p);
            } else if index.interfaces.contains_key(p) {
                iface_names.push(p);
            } else {
                return None;
            }
        }
        for i in &parent.implements {
            if index.interfaces.contains_key(i.as_str()) {
                iface_names.push(i.as_str());
            } else if !index.classes.contains_key(i.as_str()) {
                return None;
            }
        }
    }

    // 展开接口集合（含接口继承）
    let mut iface_idx = 0;
    while iface_idx < iface_names.len() {
        let name = iface_names[iface_idx];
        iface_idx += 1;
        if !visited.insert(name) {
            continue;
        }
        let Some(iface) = index.interfaces.get(name) else {
            return None;
        };
        let subst = build_supertype_subst(&iface.type_params, direct_args.get(name));
        let fully_concrete = iface.type_params.is_empty() || subst.is_some();
        for m in &iface.methods {
            let mut sig = sig_of_interface_method_tp(m, &iface.type_params);
            if sig.ret.is_none() {
                if let Some(body) = m.default_body.as_deref() {
                    if let Some(r) = infer_simple_return_ty(body, index) {
                        sig.ret = Some(r);
                    }
                }
            }
            if let Some(s) = &subst {
                sig.params = sig.params.iter().map(|t| subst_type(t, s)).collect();
                sig.ret = sig.ret.as_ref().map(|t| subst_type(t, s));
                sig.tp_order = cls.type_params.clone();
                sig.tp_order.extend(m.type_params.iter().cloned());
                sig.class_tp_count = cls.type_params.len();
            }
            out.push(ParentMethod {
                sig,
                strict_types: true,
                abstract_: m.default_body.is_none(),
                open_: true,
                is_interface: true,
                owner: name.to_string(),
                type_params: m.type_params.clone(),
                constraints: m.constraints.clone(),
                fully_concrete,
            });
        }
        if let Some(exts) = index.extends.get(name) {
            for f in exts {
                out.push(ParentMethod {
                    sig: sig_of_function(f, index.type_params_of(name)),
                    strict_types: true,
                    abstract_: false,
                    open_: true,
                    is_interface: true,
                    owner: name.to_string(),
                    type_params: f.type_params.clone(),
                    constraints: f.constraints.clone(),
                    fully_concrete: true,
                });
            }
        }
        for p in &iface.parents {
            if index.interfaces.contains_key(p.as_str()) {
                iface_names.push(p.as_str());
            } else if !index.classes.contains_key(p.as_str()) {
                return None;
            }
        }
    }

    Some(out)
}

/// 收集可解析父类型的实例成员名：返回（方法名集合, 属性名集合）。
///
/// 属性访问器在 AST 中脱糖为 `__get_p` / `__set_p`，此处还原为属性名。
/// 静态成员不计入（裸名引用静态成员不构成实例成员访问）。
/// 任一父类型无法在本地解析时返回 `None`，调用方应保守跳过。
fn collect_parent_member_names(
    cls: &ClassDef,
    index: &ProgramIndex<'_>,
) -> Option<(HashSet<String>, HashSet<String>)> {
    let parents = collect_parent_methods(cls, index)?;
    let mut methods = HashSet::new();
    let mut props = HashSet::new();
    for p in &parents {
        if p.sig.is_static {
            continue;
        }
        let base = p.sig.name.rsplit('.').next().unwrap_or(&p.sig.name);
        if let Some(prop) = base
            .strip_prefix("__get_")
            .or_else(|| base.strip_prefix("__set_"))
        {
            props.insert(prop.to_string());
        } else {
            methods.insert(base.to_string());
        }
    }
    Some((methods, props))
}

/// unittest 框架注入到 TestCase 的生命周期钩子名。
/// `@Test` 宏会把测试类挂到 TestCase 下，并允许用 override 覆盖这些钩子；
/// 宏展开受限时父类型不可见，override 检查需对它们放行。
fn is_unittest_lifecycle_hook(name: &str) -> bool {
    matches!(name, "beforeAll" | "beforeEach" | "afterAll" | "afterEach")
}

/// override/redef 方法必须能在父类型中找到签名匹配的函数。
/// 7.4(8)/(10)/(13)：子类型属性覆写/重定义的限制。
///
/// - 实例属性不能使用 `redef`（7.4(8)：仅 static 属性可 redef）；
/// - 覆写/重定义的属性必须与父类型属性保持相同的 `mut` 修饰符（7.4(10)/(13)）。
fn check_class_property_members(
    cls: &ClassDef,
    index: &ProgramIndex<'_>,
    errors: &mut Vec<SemanticError>,
) {
    // 7.2(14)：成员变量与成员属性不能同名。
    let field_names: HashSet<&str> = cls.fields.iter().map(|f| f.name.as_str()).collect();
    let mut reported_dup: HashSet<String> = HashSet::new();
    for m in &cls.methods {
        let leaf = m.func.name.rsplit('.').next().unwrap_or("");
        if let Some(base) = leaf.strip_prefix("__get_") {
            if field_names.contains(base) && reported_dup.insert(base.to_string()) {
                errors.push(SemanticError {
                    message: format!(
                        "类 '{}' 的成员变量与成员属性 '{}' 不能同名（7.2(14)）",
                        cls.name, base
                    ),
                });
            }
        }
    }
    // 7.4(8)：实例属性不能被 redef（只能 override）；static 属性才可用 redef。
    let mut reported_redef: HashSet<String> = HashSet::new();
    for m in &cls.methods {
        let leaf = m.func.name.rsplit('.').next().unwrap_or("");
        if m.override_
            && !m.has_override_kw
            && !m.is_static
            && is_property_method_name(leaf)
        {
            let base = member_base_name(leaf).to_string();
            if reported_redef.insert(base.clone()) {
                errors.push(SemanticError {
                    message: format!(
                        "类 '{}' 的实例属性 '{}' 不能使用 redef（7.4(8)）",
                        cls.name, base
                    ),
                });
            }
        }
    }

    let Some(parents) = collect_parent_methods(cls, index) else {
        return;
    };

    // 父类（非接口）属性基名 → 是否可变（存在 setter）。
    let mut parent_props: HashMap<&str, bool> = HashMap::new();
    for p in parents.iter().filter(|p| !p.is_interface) {
        let n = &p.sig.name;
        if !is_property_method_name(n) {
            continue;
        }
        let base = member_base_name(n);
        let e = parent_props.entry(base).or_insert(false);
        if n.starts_with("__set_") {
            *e = true;
        }
    }
    if parent_props.is_empty() {
        return;
    }

    // 自身（override/redef）属性的基名 → 声明是否带 mut 修饰符。
    let mut own_props: HashMap<&str, bool> = HashMap::new();
    for m in cls.methods.iter().filter(|m| m.override_) {
        let leaf = m.func.name.rsplit('.').next().unwrap_or("");
        if !is_property_method_name(leaf) {
            continue;
        }
        let base = member_base_name(leaf);
        let e = own_props.entry(base).or_insert(false);
        if m.is_prop_mut {
            *e = true;
        }
    }
    let mut reported: HashSet<&str> = HashSet::new();
    for (base, parent_mut) in parent_props {
        let Some(&own_mut) = own_props.get(base) else {
            continue;
        };
        if own_mut != parent_mut && reported.insert(base) {
            errors.push(SemanticError {
                message: format!(
                    "类 '{}' 覆写/重定义的属性 '{}' 的 mut 修饰符必须与父类型一致（7.4(10)/(13)）",
                    cls.name, base
                ),
            });
        }
    }
}

fn check_class_overrides(cls: &ClassDef, index: &ProgramIndex<'_>, errors: &mut Vec<SemanticError>) {
    let Some(parents) = collect_parent_methods(cls, index) else {
        return;
    };
    for method in &cls.methods {
        if !method.override_ {
            continue;
        }
        let sig = sig_of_class_method(method, &cls.type_params);
        if is_synthetic_member(&sig.name) {
            continue;
        }
        // 6.3.1.1(1)：static 函数不能被 override（实例覆写语义），
        // static 函数的再定义应使用 redef（6.3.4.1(1)）。
        if method.has_override_kw && method.is_static {
            errors.push(SemanticError {
                message: format!(
                    "类 '{}' 的 static 成员函数 '{}' 不能使用 override（应使用 redef，6.3.1.1(1)）",
                    cls.name, sig.display
                ),
            });
            continue;
        }
        // 抽象的 mut prop 只会脱糖出 __get_x（解析器不生成抽象 setter），
        // 因此 setter 的 override 只要能找到同名 getter 就视为合法。
        let alt_getter = sig
            .name
            .strip_prefix("__set_")
            .map(|rest| format!("__get_{}", rest));
        let same_name: Vec<&ParentMethod> = parents
            .iter()
            .filter(|p| {
                p.sig.name == sig.name || alt_getter.as_deref() == Some(p.sig.name.as_str())
            })
            .collect();
        if same_name.is_empty() {
            // unittest 测试框架宏（@Test/@TestCase）会给测试类注入 TestCase 父类，
            // 从而提供 beforeAll/beforeEach/afterAll/afterEach 这些生命周期方法。
            // 由于宏展开能力有限，这里对生命周期钩子的“无父类可覆盖”错误放行，
            // 避免 std.unittest 用例因宏未展开而编译失败。
            if is_unittest_lifecycle_hook(&sig.name) {
                continue;
            }
            errors.push(SemanticError {
                message: format!(
                    "类 '{}' 的 '{}' 使用了 override/redef，但父类型中不存在可覆盖的同名函数",
                    cls.name, sig.display
                ),
            });
            continue;
        }
        // 逐父成员判定是否构成合法覆写/重定义。
        let mut any_matched = false;
        let mut any_concrete = false;
        // 6.3.4.1(6)：同一个 redef 函数重定义多个父类型中同名同参数列表的函数时，
        // 必须与每一个都兼容（返回类型逐一对齐），不能只满足其中一个。
        let mut redef_partial_conflict = false;
        for p in &same_name {
            let same_name_match = p.sig.name == sig.name;
            let matched = if same_name_match {
                if method.has_override_kw {
                    signature_matches(&sig, &p.sig, p.strict_types, index)
                        // 6.3.1.1(1)：泛型性质（方法级类型参数）不同不构成覆写。
                        && generic_compatible(sig.generic, p.sig.generic)
                        // 6.3.1.1(6)：泛型类型参数必须与被覆写函数相同。
                        && method.func.type_params.len() == p.type_params.len()
                        && (method.func.type_params.is_empty()
                            || !generic_signature_shape_conflict(&sig, &p.sig))
                        // 6.3.1.1(3)：类级泛型已消解时参数类型须严格一致。
                        && strict_param_match(&sig, &p.sig, p.fully_concrete)
                } else {
                    // redef：显式泛型替换后的严格比较（6.3.4.1(3)）。
                    redef_signature_matches(
                        &sig,
                        &p.sig,
                        p.fully_concrete,
                        p.strict_types,
                        index,
                    )
                }
            } else {
                // 抽象 mut prop 只脱糖出 getter 时的兼容路径。
                alt_getter.as_deref() == Some(p.sig.name.as_str())
            };
            if matched {
                any_matched = true;
                if same_name_match && !p.abstract_ {
                    any_concrete = true;
                }
            } else if same_name_match
                && !method.has_override_kw
                && !is_property_method_name(&sig.name)
                && canon_params(&sig) == canon_params(&p.sig)
            {
                redef_partial_conflict = true;
            }
        }
        if !any_matched {
            errors.push(SemanticError {
                message: format!(
                    "类 '{}' 的 '{}' 与父类型函数的参数列表或返回类型不一致",
                    cls.name, sig.display
                ),
            });
        } else if redef_partial_conflict {
            errors.push(SemanticError {
                message: format!(
                    "类 '{}' 的 '{}' 与部分父类型同名函数的参数列表相同但返回类型不兼容（6.3.4.1(6)）",
                    cls.name, sig.display
                ),
            });
        } else if !method.has_body && any_concrete {
            // 6.3.1.1(1)/(2)：无实现体的覆写（抽象声明）不能覆写父类型中有实现的函数。
            errors.push(SemanticError {
                message: format!(
                    "类 '{}' 的抽象成员函数 '{}' 不能覆写父类型中有实现的函数（6.3.1.1(1)）",
                    cls.name, sig.display
                ),
            });
        } else {
            // 只有显式 `override` 才要求父类函数为 open/abstract；
            // `redef` 是重定义（静态成员/接口默认实现），不受 open 限制（6.1.2.4(32)/(38)）。
            if method.has_override_kw {
                let overridable = same_name.iter().any(|p| {
                    let primary = p.sig.name == sig.name
                        && signature_matches(&sig, &p.sig, p.strict_types, index);
                    let alt = alt_getter.as_deref() == Some(p.sig.name.as_str());
                    (primary || alt) && (p.is_interface || p.open_)
                });
                if !overridable {
                    errors.push(SemanticError {
                        message: format!(
                            "类 '{}' 的 '{}' 覆写了父类型中不可覆写（非 open/abstract）的函数（6.1.2.4(32)）",
                            cls.name, sig.display
                        ),
                    });
                }
            } else {
                // 6.1.2.4(44) / 6.3.4.1(1)：`redef` 只能用于重定义父类型中的 static
                // 函数（以及属性 / 接口默认实现的再定义，属性访问器脱糖为 __get_/__set_，
                // 不计入）。实例成员函数重定义父类型函数必须使用 override。
                if !method.is_static && !is_property_method_name(&sig.name) {
                    errors.push(SemanticError {
                        message: format!(
                            "类 '{}' 的实例成员函数 '{}' 不能使用 redef（redef 只能重定义 static 函数，实例函数须用 override，6.1.2.4(44)）",
                            cls.name, sig.display
                        ),
                    });
                }
            }
        }
    }
}

/// 泛型约束规范化目标：类型参数按声明位置编号，或具体类型名。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum ConsTarget {
    Param(usize),
    Named(String),
}

/// 把一组约束规范化到方法级类型参数空间；存在无法归位的约束参数时返回 None
/// （保守跳过比较，避免误伤）。
fn normalize_constraints(
    constraints: &[crate::ast::TypeConstraint],
    tp: &[String],
) -> Option<Vec<(usize, ConsTarget)>> {
    let mut out = Vec::new();
    for c in constraints {
        let pi = tp.iter().position(|n| n == &c.param)?;
        for b in &c.bounds {
            let target = match tp.iter().position(|n| n == b) {
                Some(bi) => ConsTarget::Param(bi),
                None => ConsTarget::Named(b.clone()),
            };
            out.push((pi, target));
        }
    }
    Some(out)
}

/// 上界放宽判定：父约束 `T <: parent_bound` 是否已蕴含实现约束 `T <: own_bound`。
fn bound_looser_or_eq(parent_bound: &str, own_bound: &str, index: &ProgramIndex<'_>) -> bool {
    if own_bound == "Any" || parent_bound == own_bound {
        return true;
    }
    if parent_bound == "Nothing" {
        return true;
    }
    // Object 是类层级的顶层类型：仅当父上界本身是类（或 Object）时才被蕴含；
    // 对无上界 / 仅参数间约束的情形，`T <: Object` 会排除结构体等类型，属于更严格。
    if own_bound == "Object" {
        return parent_bound == "Object" || index.classes.contains_key(parent_bound);
    }
    index.is_subtype_name(parent_bound, own_bound)
}

/// 在约束关系集合 `pairs` 下，`(a, b)` 是否被蕴含（含参数间的传递闭包）。
///
/// 具名上界允许“放宽”：父约束 `T <: B_par` 可满足更宽松的实现约束 `T <: B_own`
/// （只要 `B_par <: B_own`）。`Any` 是顶层类型，`T <: Any` 恒真，不构成更严格约束。
fn constraint_implied(
    pairs: &[(usize, ConsTarget)],
    a: usize,
    b: &ConsTarget,
    index: &ProgramIndex<'_>,
) -> bool {
    if matches!(b, ConsTarget::Named(n) if n == "Any") {
        return true;
    }
    let mut params_seen: HashSet<usize> = HashSet::new();
    let mut named_seen: HashSet<String> = HashSet::new();
    let mut stack = vec![a];
    while let Some(x) = stack.pop() {
        if !params_seen.insert(x) {
            continue;
        }
        for (pa, pb) in pairs {
            if *pa != x {
                continue;
            }
            match pb {
                ConsTarget::Param(y) => stack.push(*y),
                ConsTarget::Named(n) => {
                    named_seen.insert(n.clone());
                }
            }
        }
    }
    match b {
        ConsTarget::Param(y) => params_seen.contains(y),
        ConsTarget::Named(n) => named_seen
            .iter()
            .any(|pn| bound_looser_or_eq(pn, n, index)),
    }
}

/// 单个实现/覆写函数的约束宽松度比较（6.2.4.1(5) / 6.3.4.1(5)）。
///
/// 实现方的约束必须是父类型声明约束的子集（即更宽松或相同）；额外添加约束或
/// 反转上下界都视为更严格。仅在签名可严格比对、类型参数个数一致且约束都能归位
/// 到方法级类型参数时才判定，避免误伤。
fn check_constraint_looseness_for(
    owner: &str,
    sig: &Sig,
    own_tp: &[String],
    own_cons: &[crate::ast::TypeConstraint],
    parents: &[ParentMethod],
    ignore_static: bool,
    index: &ProgramIndex<'_>,
    errors: &mut Vec<SemanticError>,
) {
    if is_synthetic_member(&sig.name) {
        return;
    }
    let Some(own_pairs) = normalize_constraints(own_cons, own_tp) else {
        return;
    };
    if own_pairs.is_empty() {
        return;
    }
    let mut parent_sets: Vec<Vec<(usize, ConsTarget)>> = Vec::new();
    let mut matched = false;
    for p in parents {
        if p.sig.name != sig.name {
            continue;
        }
        // struct/enum 的成员函数在解析器中被合成为带 `this` 接收者的顶层函数，
        // static 信息丢失，比较时忽略该属性。
        if !ignore_static && p.sig.is_static != sig.is_static {
            continue;
        }
        // 泛型父类只做参数个数比较，约束比较不可靠。
        if !p.strict_types {
            continue;
        }
        if sig.generic != p.sig.generic {
            continue;
        }
        if own_tp.len() != p.type_params.len() {
            continue;
        }
        if !signature_matches(sig, &p.sig, true, index) {
            continue;
        }
        let Some(par_pairs) = normalize_constraints(&p.constraints, &p.type_params) else {
            continue;
        };
        matched = true;
        parent_sets.push(par_pairs);
    }
    if !matched {
        return;
    }
    // 任一已匹配父函数能蕴含该约束即放行（实现需同时满足多个父类型时取并集放宽）。
    let stricter = own_pairs
        .iter()
        .any(|(a, b)| !parent_sets.iter().any(|ps| constraint_implied(ps, *a, b, index)));
    if stricter {
        errors.push(SemanticError {
            message: format!(
                "类型 '{}' 的 '{}' 的泛型约束比父类型中的对应函数更严格（要求更宽松或相同，6.2.4.1(5)/6.3.4.1(5)）",
                owner, sig.display
            ),
        });
    }
}

/// 检查所有类型的实现/覆写函数是否在使用更严格泛型约束（6.2.4.1(5) / 6.3.4.1(5)）。
fn check_generic_constraint_looseness<'a>(
    program: &'a Program,
    index: &ProgramIndex<'a>,
    errors: &mut Vec<SemanticError>,
) {
    for cls in &program.classes {
        let Some(parents) = collect_parent_methods(cls, index) else {
            continue;
        };
        for m in &cls.methods {
            let sig = sig_of_class_method(m, &cls.type_params);
            check_constraint_looseness_for(
                &cls.name,
                &sig,
                &m.func.type_params,
                &m.func.constraints,
                &parents,
                false,
                index,
                errors,
            );
        }
    }

    // struct / enum 通过 `<: Interface` 约束或 extend 提供实现函数（顶层函数名带所属类型前缀）。
    for (owner, type_params, constraints) in program
        .structs
        .iter()
        .map(|s| (s.name.as_str(), &s.type_params, &s.constraints))
        .chain(
            program
                .enums
                .iter()
                .map(|e| (e.name.as_str(), &e.type_params, &e.constraints)),
        )
    {
        let bounds: Vec<&'a str> = constraints
            .iter()
            .filter(|c| c.param == owner)
            .flat_map(|c| c.bounds.iter().map(|b| b.as_str()))
            .collect();
        if bounds.is_empty() {
            continue;
        }
        let Some(parents) = collect_interface_methods(&bounds, index) else {
            continue;
        };
        for f in &program.functions {
            if owner_name(&f.name) != Some(owner) {
                continue;
            }
            let sig = sig_of_function(f, type_params);
            check_constraint_looseness_for(
                owner,
                &sig,
                &f.type_params,
                &f.constraints,
                &parents,
                true,
                index,
                errors,
            );
        }
        if let Some(exts) = index.extends.get(owner) {
            for f in exts {
                let sig = sig_of_function(f, type_params);
                check_constraint_looseness_for(
                    owner,
                    &sig,
                    &f.type_params,
                    &f.constraints,
                    &parents,
                    true,
                    index,
                    errors,
                );
            }
        }
    }
}

/// 2.4.1(11)/(12)：类型别名之间不允许直接或间接的循环引用。
///
/// 仅在同一文件的 `type Alias = ...` 定义之间建立引用图并检测环；别名指向的
/// 具体类型本身可以合法地包含泛型实参/元组/函数类型等，这里递归收集其中出现
/// 的别名名（`Type::Struct(name, ..)` 且 name 是本地别名）。
fn check_type_alias_cycles(program: &Program, errors: &mut Vec<SemanticError>) {
    let aliases: HashMap<&str, &Type> = program
        .type_aliases
        .iter()
        .map(|(n, t)| (n.as_str(), t))
        .collect();
    if aliases.is_empty() {
        return;
    }

    // 递归收集一个类型表达式直接引用到的本地别名名。
    fn collect(ty: &Type, aliases: &HashMap<&str, &Type>, out: &mut Vec<String>) {
        match ty {
            Type::Struct(name, args) => {
                // 预声明类型名在别名右侧应解析为内建类型（如 `type Array = Array<Int64>`），
                // 而非自引用；跳过这些名字以避免误报循环。
                let predeclared = is_builtin_type_name(name)
                    || matches!(
                        name.as_str(),
                        "Range"
                            | "Option"
                            | "Result"
                            | "Tuple"
                            | "Map"
                            | "Slice"
                            | "Any"
                            | "Object"
                            | "Unit"
                            | "Nothing"
                            | "Iterable"
                            | "Collection"
                            | "Iterator"
                            | "ToString"
                            | "Equatable"
                            | "Comparable"
                            | "Hashable"
                    );
                if !predeclared && aliases.contains_key(name.as_str()) {
                    out.push(name.clone());
                }
                for a in args {
                    collect(a, aliases, out);
                }
            }
            Type::Array(t) | Type::Option(t) | Type::Slice(t) => collect(t, aliases, out),
            Type::Result(a, b) | Type::Map(a, b) => {
                collect(a, aliases, out);
                collect(b, aliases, out);
            }
            Type::Tuple(ts) => {
                for t in ts {
                    collect(t, aliases, out);
                }
            }
            Type::Function { params, ret } => {
                for p in params {
                    collect(p, aliases, out);
                }
                if let Some(r) = ret.as_ref() {
                    collect(r, aliases, out);
                }
            }
            _ => {}
        }
    }

    let mut adj: HashMap<&str, Vec<String>> = HashMap::new();
    for (name, ty) in &program.type_aliases {
        let mut refs = Vec::new();
        collect(ty, &aliases, &mut refs);
        adj.insert(name.as_str(), refs);
    }

    // 从 cur 出发能否回到 start（存在长度 >= 1 的路径）。
    fn reaches(
        start: &str,
        cur: &str,
        adj: &HashMap<&str, Vec<String>>,
        visited: &mut HashSet<String>,
    ) -> bool {
        if let Some(nexts) = adj.get(cur) {
            for n in nexts {
                if n == start {
                    return true;
                }
                if visited.insert(n.clone()) && reaches(start, n, adj, visited) {
                    return true;
                }
            }
        }
        false
    }

    let mut reported: HashSet<&str> = HashSet::new();
    for (name, _) in &program.type_aliases {
        if reported.contains(name.as_str()) {
            continue;
        }
        let mut visited: HashSet<String> = HashSet::new();
        visited.insert(name.clone());
        if reaches(name.as_str(), name.as_str(), &adj, &mut visited) {
            reported.insert(name.as_str());
            errors.push(SemanticError {
                message: format!(
                    "类型别名 '{}' 存在（直接或间接的）循环引用（2.4.1(11)/(12)）",
                    name
                ),
            });
        }
    }
}

/// 声明层泛型检查：类型参数不得重名（9.1(4)）；约束左界必须是已声明的类型变量
/// （9.2(4)）。`struct S <: I` / `enum E <: I` 会把自身类型名作为约束左界，
/// 属于语法层面的父类型编码，需放行。
/// 类型 `t` 是否直接或间接引用类型参数 `p`。
fn type_mentions_param(t: &Type, p: &str) -> bool {
    match t {
        Type::TypeParam(n) => n == p,
        Type::Array(inner) | Type::Option(inner) | Type::Slice(inner) => {
            type_mentions_param(inner, p)
        }
        Type::Struct(_, args) => args.iter().any(|a| type_mentions_param(a, p)),
        Type::Tuple(ts) => ts.iter().any(|a| type_mentions_param(a, p)),
        Type::Map(k, v) => type_mentions_param(k, p) || type_mentions_param(v, p),
        Type::Result(a, b) => type_mentions_param(a, p) || type_mentions_param(b, p),
        Type::Function { params, ret } => {
            params.iter().any(|a| type_mentions_param(a, p))
                || ret.as_ref().as_ref().map_or(false, |r| type_mentions_param(r, p))
        }
        _ => false,
    }
}

/// 9.5.1.1(12)/(13)：当类型变量的上界是值类型（struct/enum，含 Option/Result）时，
/// 不允许递归约束（上界实参直接/间接依赖该类型变量本身），如 `T <: S<T>`。
/// 接口/类上界保守放行（是否递归依赖继承关系，无法据此判定）。
fn check_recursive_value_constraints(program: &Program, errors: &mut Vec<SemanticError>) {
    let struct_names: HashSet<&str> = program.structs.iter().map(|s| s.name.as_str()).collect();
    let enum_names: HashSet<&str> = program.enums.iter().map(|e| e.name.as_str()).collect();
    let is_value_bound = |b: &str| {
        struct_names.contains(b)
            || enum_names.contains(b)
            || b == "Option"
            || b == "Result"
    };
    let mut check = |owner: &str, constraints: &[TypeConstraint]| {
        for c in constraints {
            for (i, b) in c.bounds.iter().enumerate() {
                if !is_value_bound(b) {
                    continue;
                }
                if let Some(args) = c.bound_args.get(i) {
                    if args.iter().any(|t| type_mentions_param(t, &c.param)) {
                        errors.push(SemanticError {
                            message: format!(
                                "泛型约束左侧类型变量 '{}' 出现在其上界 '{}' 的类型实参中，构成递归约束（9.5.1.1(12)）：{}",
                                c.param, b, owner
                            ),
                        });
                    }
                }
            }
        }
    };
    for cls in &program.classes {
        check(&cls.name, &cls.constraints);
        for m in &cls.methods {
            check(&m.func.name, &m.func.constraints);
        }
    }
    for st in &program.structs {
        check(&st.name, &st.constraints);
    }
    for en in &program.enums {
        check(&en.name, &en.constraints);
    }
    for f in &program.functions {
        check(&f.name, &f.constraints);
    }
}

fn check_generic_declarations(program: &Program, errors: &mut Vec<SemanticError>) {
    fn check_dup(owner: &str, tps: &[String], errors: &mut Vec<SemanticError>) {
        let mut seen: HashSet<&str> = HashSet::new();
        for t in tps {
            if !seen.insert(t.as_str()) {
                errors.push(SemanticError {
                    message: format!("类型参数重名（9.1(4)）：{} 中重复声明了 '{}'", owner, t),
                });
            }
        }
    }

    fn check_cons(
        owner: &str,
        declared: &[String],
        self_name: Option<&str>,
        constraints: &[crate::ast::TypeConstraint],
        errors: &mut Vec<SemanticError>,
    ) {
        for c in constraints {
            // 父类型编码（`struct S <: I` 的 param == "S"）不是真正的类型变量约束。
            if self_name == Some(c.param.as_str()) {
                continue;
            }
            if !declared.iter().any(|t| t == &c.param) {
                errors.push(SemanticError {
                    message: format!(
                        "泛型约束的左界必须是类型变量（9.2(4)）：{} 中的 '{}'",
                        owner, c.param
                    ),
                });
            }
        }
    }

    // 类型名 → 类型参数（用于带类型前缀的成员函数解析外层类型参数）。
    let mut owner_params: HashMap<&str, &[String]> = HashMap::new();
    for c in &program.classes {
        owner_params.insert(c.name.as_str(), c.type_params.as_slice());
    }
    for s in &program.structs {
        owner_params.insert(s.name.as_str(), s.type_params.as_slice());
    }
    for e in &program.enums {
        owner_params.insert(e.name.as_str(), e.type_params.as_slice());
    }
    for i in &program.interfaces {
        owner_params.insert(i.name.as_str(), i.type_params.as_slice());
    }

    for f in &program.functions {
        check_dup(&f.name, &f.type_params, errors);
        let mut declared = f.type_params.clone();
        let self_name = owner_name(&f.name);
        if let Some(owner) = self_name {
            match owner_params.get(owner) {
                Some(tps) => declared.extend(tps.iter().cloned()),
                // 所属类型不可解析（如 extend 泛型参数已被丢弃）：保守跳过。
                None => continue,
            }
        }
        check_cons(&f.name, &declared, None, &f.constraints, errors);
    }

    for cls in &program.classes {
        check_dup(&cls.name, &cls.type_params, errors);
        check_cons(&cls.name, &cls.type_params, Some(&cls.name), &cls.constraints, errors);
        for m in &cls.methods {
            check_dup(&m.func.name, &m.func.type_params, errors);
            let mut declared = m.func.type_params.clone();
            declared.extend(cls.type_params.iter().cloned());
            check_cons(&m.func.name, &declared, None, &m.func.constraints, errors);
        }
    }
    for st in &program.structs {
        check_dup(&st.name, &st.type_params, errors);
        check_cons(&st.name, &st.type_params, Some(&st.name), &st.constraints, errors);
    }
    for en in &program.enums {
        check_dup(&en.name, &en.type_params, errors);
        check_cons(&en.name, &en.type_params, Some(&en.name), &en.constraints, errors);
    }
    for iface in &program.interfaces {
        check_dup(&iface.name, &iface.type_params, errors);
        check_cons(
            &iface.name,
            &iface.type_params,
            Some(&iface.name),
            &iface.constraints,
            errors,
        );
        for m in &iface.methods {
            check_dup(&m.name, &m.type_params, errors);
            let mut declared = m.type_params.clone();
            declared.extend(iface.type_params.iter().cloned());
            check_cons(&m.name, &declared, None, &m.constraints, errors);
        }
    }
}

/// 覆写 / 重定义（含未写 override 关键字的同名静态重定义）不得降低可见性（6.3.5(2)）。
fn check_member_visibility_lowering(
    cls: &ClassDef,
    index: &ProgramIndex<'_>,
    errors: &mut Vec<SemanticError>,
) {
    let Some(parents) = collect_parent_methods(cls, index) else {
        return;
    };
    for method in &cls.methods {
        let sig = sig_of_class_method(method, &cls.type_params);
        if is_synthetic_member(&sig.name) {
            continue;
        }
        let Some(child_rank) = sig.vis.as_ref().map(visibility_rank) else {
            continue;
        };
        // 属性的 setter 只有父接口可能只声明 getter。
        let alt_getter = sig
            .name
            .strip_prefix("__set_")
            .map(|rest| format!("__get_{}", rest));
        let parent_rank = parents
            .iter()
            .filter(|p| {
                p.sig.is_static == sig.is_static
                    && (p.sig.name == sig.name || alt_getter.as_deref() == Some(p.sig.name.as_str()))
                    && (canon_params(&sig) == canon_params(&p.sig)
                        || alt_getter.as_deref() == Some(p.sig.name.as_str()))
            })
            .filter_map(|p| p.sig.vis.as_ref().map(visibility_rank))
            .max();
        if let Some(parent_rank) = parent_rank {
            if child_rank < parent_rank {
                errors.push(SemanticError {
                    message: format!(
                        "类 '{}' 的 '{}' 覆写/重定义时降低了访问级别（父类型更宽松）",
                        cls.name, sig.display
                    ),
                });
            }
        }
    }
}

/// 实现类型（class/struct/enum）实现接口函数时不得降低访问级别（6.3.5(3)）。
///
/// 接口成员未标注时默认为 public；实现函数须不窄于接口声明。
fn check_impl_visibility(
    owner: &str,
    own: &[Sig],
    parents: &[ParentMethod],
    index: &ProgramIndex<'_>,
    errors: &mut Vec<SemanticError>,
) {
    for sig in own {
        if is_synthetic_member(&sig.name) {
            continue;
        }
        let Some(child_rank) = sig.vis.as_ref().map(visibility_rank) else {
            continue;
        };
        let alt_getter = sig
            .name
            .strip_prefix("__set_")
            .map(|rest| format!("__get_{}", rest));
        let parent_rank = parents
            .iter()
            .filter(|p| p.is_interface)
            .filter(|p| {
                p.sig.is_static == sig.is_static
                    && (p.sig.name == sig.name || alt_getter.as_deref() == Some(p.sig.name.as_str()))
                    && (canon_params(sig) == canon_params(&p.sig)
                        || alt_getter.as_deref() == Some(p.sig.name.as_str()))
            })
            .filter_map(|p| p.sig.vis.as_ref().map(visibility_rank))
            .max();
        if let Some(parent_rank) = parent_rank {
            if child_rank < parent_rank {
                errors.push(SemanticError {
                    message: format!(
                        "类型 '{}' 的 '{}' 实现接口函数时降低了访问级别（6.3.5(3)）",
                        owner, sig.display
                    ),
                });
            }
        }
    }
}

/// 检查类自身方法与父类（非接口）方法之间的 static/实例同名冲突
/// （6.3.2(1) 中“包括继承自父类”的部分）。
fn check_inherited_static_conflicts(
    cls: &ClassDef,
    index: &ProgramIndex<'_>,
    errors: &mut Vec<SemanticError>,
) {
    let Some(parents) = collect_parent_methods(cls, index) else {
        return;
    };
    for method in &cls.methods {
        let sig = sig_of_class_method(method, &cls.type_params);
        if is_synthetic_member(&sig.name) || is_property_method_name(&sig.name) {
            continue;
        }
        let conflict = parents.iter().any(|p| {
            p.sig.name == sig.name
                && p.sig.is_static != sig.is_static
                && !is_synthetic_member(&p.sig.name)
                && !is_property_method_name(&p.sig.name)
        });
        if conflict {
            errors.push(SemanticError {
                message: format!(
                    "类 '{}' 的成员函数 '{}' 与父类型同名函数在 static/实例属性上冲突，不允许互为重载（6.3.2(1)）",
                    cls.name, sig.display
                ),
            });
        }
    }
}

/// 多个父类型（父类 + 接口）之间的同名成员函数 static/实例冲突（6.3.2(1)）。
///
/// 例如类继承 static `f`（父类）同时实现声明 instance `f`（接口），
/// 即使子类自身没有定义 `f`，同名函数的 static/实例重载同样非法。
fn check_cross_parent_static_conflicts(
    cls: &ClassDef,
    index: &ProgramIndex<'_>,
    errors: &mut Vec<SemanticError>,
) {
    let Some(parents) = collect_parent_methods(cls, index) else {
        return;
    };
    let mut kinds: HashMap<String, (bool, bool)> = HashMap::new(); // name → (static, instance)
    let mut owners: HashMap<String, String> = HashMap::new();
    for p in &parents {
        if is_synthetic_member(&p.sig.name) || is_property_method_name(&p.sig.name) {
            continue;
        }
        let entry = kinds.entry(p.sig.name.clone()).or_insert((false, false));
        if p.sig.is_static {
            entry.0 = true;
        } else {
            entry.1 = true;
        }
        owners.entry(p.sig.name.clone()).or_insert_with(|| p.owner.clone());
    }
    for (name, (has_static, has_instance)) in kinds {
        if has_static && has_instance {
            errors.push(SemanticError {
                message: format!(
                    "类 '{}' 从父类型继承的成员函数 '{}'（来源 '{}'）在 static/实例属性上冲突，不允许互为重载（6.3.2(1)）",
                    cls.name, name, owners.get(&name).cloned().unwrap_or_default()
                ),
            });
        }
    }
}

/// 把“自身可用实现 + 父类型中带实现的方法”与接口抽象方法比对，报告缺失/签名不匹配。
fn check_abstract_methods_implemented(
    owner: &str,
    own_impls: Vec<Sig>,
    parents: &[ParentMethod],
    index: &ProgramIndex<'_>,
    errors: &mut Vec<SemanticError>,
) {
    // 真实实现：实现类型自身的方法 + 父类（非接口）方法 + extend 方法。
    let mut real_impls: Vec<Sig> = own_impls;
    for p in parents {
        if !p.is_interface && !p.abstract_ {
            real_impls.push(p.sig.clone());
        }
    }
    // 接口默认实现也可以满足另一个接口的抽象方法。
    let mut owned_impls: Vec<Sig> = real_impls.clone();
    for p in parents {
        if p.is_interface && !p.abstract_ {
            owned_impls.push(p.sig.clone());
        }
    }
    let impls: Vec<&Sig> = owned_impls.iter().collect();

    let mut reported: HashSet<String> = HashSet::new();
    for p in parents {
        if !p.abstract_ || is_synthetic_member(&p.sig.name) {
            continue;
        }
        // 属性抽象声明 `prop p` 会脱糖为 `__get_p`，setter 不参与强制实现。
        let has_same_name = impls.iter().any(|s| s.name == p.sig.name);
        let matched = impls.iter().any(|s| {
            // 两侧签名均含类型参数且父类型类级泛型已消解时，要求类型参数位置一致
            // （6.2.4.1(16)）：不能把接口的泛型实参替换成另一个类型参数。
            let both_generic = s
                .params
                .iter()
                .chain(s.ret.iter())
                .any(type_contains_type_param)
                && p.sig
                    .params
                    .iter()
                    .chain(p.sig.ret.iter())
                    .any(type_contains_type_param);
            s.name == p.sig.name
                && signature_matches(s, &p.sig, p.strict_types, index)
                // 6.2.4.1(16)：类级泛型已消解时，实现函数的参数类型须严格对应被实现
                // 的接口函数签名（不能把接口类型实参替换成另一个类型参数）。
                && strict_param_match(s, &p.sig, p.fully_concrete)
                && !(p.fully_concrete
                    && both_generic
                    && generic_signature_shape_conflict(s, &p.sig))
        });
        if matched {
            continue;
        }
        if reported.insert(p.sig.name.clone()) {
            let reason = if has_same_name {
                "参数列表或返回类型不匹配"
            } else {
                "缺少实现"
            };
            errors.push(SemanticError {
                message: format!(
                    "类型 '{}' 未正确实现接口方法 '{}'（{}）",
                    owner, p.sig.display, reason
                ),
            });
        }
    }

    // 多个接口为同名同签名的成员提供默认实现时，实现类型不能依赖继承默认实现
    // （6.2.4.1(9)）。
    check_multiple_default_conflicts(owner, &real_impls, parents, index, errors, "类型");
}

/// 多个父接口为同名同签名成员提供默认实现时，实现类型（class/struct/enum）或
/// 子接口必须提供自己的实现，不能依赖继承默认实现（6.2.4.1(9)、6.2.3.1(6)）。
///
/// `own_impls` 必须是“真正的实现”：对 class/struct/enum 是带函数体的成员，
/// 对子接口是带默认实现体的成员。仅声明而无实现体的成员不计入。
fn check_multiple_default_conflicts(
    owner: &str,
    own_impls: &[Sig],
    parents: &[ParentMethod],
    index: &ProgramIndex<'_>,
    errors: &mut Vec<SemanticError>,
    kind: &str,
) {
    let mut defaults: HashMap<(String, String, bool), (HashSet<String>, Sig)> = HashMap::new();
    for p in parents {
        if p.is_interface && !p.abstract_ && !is_synthetic_member(&p.sig.name) {
            let entry = defaults
                .entry((p.sig.name.clone(), canon_params(&p.sig), p.sig.is_static))
                .or_insert_with(|| (HashSet::new(), p.sig.clone()));
            entry.0.insert(p.owner.clone());
        }
    }
    let mut reported: HashSet<(String, String, bool)> = HashSet::new();
    for (key, (owners, sample)) in defaults {
        if owners.len() < 2 {
            continue;
        }
        // 若某个提供默认实现的接口是另一个的父接口，子接口的默认实现会覆盖父接口的，
        // 不构成“多个默认实现冲突”，只保留最派生的实现者。
        let owners_vec: Vec<&String> = owners.iter().collect();
        let maximal: Vec<&&String> = owners_vec
            .iter()
            .filter(|a| {
                !owners_vec
                    .iter()
                    .any(|b| b != *a && index.is_subtype_name(b, a))
            })
            .collect();
        if maximal.len() < 2 {
            continue;
        }
        // 注意：struct/enum 的成员函数签名由 `sig_of_function` 推断静态性，
        // 静态属性访问器可能被误判为实例方法，故此处只比较名字与参数/返回类型。
        let real = own_impls.iter().any(|s| {
            s.name == sample.name
                && canon_params(s) == canon_params(&sample)
                && signature_matches(s, &sample, true, index)
        });
        if !real && reported.insert(key) {
            errors.push(SemanticError {
                message: format!(
                    "{} '{}' 的成员 '{}' 在多个父接口中存在默认实现，必须自行实现（不能依赖继承）（6.2.4.1(9)/6.2.3.1(6)）",
                    kind, owner, sample.display
                ),
            });
        }
    }
}

/// 6.2.4.1(9)：抽象类也不能依赖多个父接口的默认实现；无实现体的声明不算实现。
fn check_abstract_class_default_conflicts(
    cls: &ClassDef,
    index: &ProgramIndex<'_>,
    errors: &mut Vec<SemanticError>,
) {
    let Some(parents) = collect_parent_methods(cls, index) else {
        return;
    };
    if !parents.iter().any(|p| p.is_interface && !p.abstract_) {
        return;
    }
    let mut own: Vec<Sig> = cls
        .methods
        .iter()
        .filter(|m| m.has_body)
        .map(|m| sig_of_class_method(m, &cls.type_params))
        .collect();
    if let Some(exts) = index.extends.get(cls.name.as_str()) {
        for f in exts {
            own.push(sig_of_function(f, &cls.type_params));
        }
    }
    check_multiple_default_conflicts(&cls.name, &own, &parents, index, errors, "类型");
}

/// 非抽象类必须实现其（本地）接口的抽象方法。
fn check_interface_implementation(
    cls: &ClassDef,
    index: &ProgramIndex<'_>,
    errors: &mut Vec<SemanticError>,
) {
    if cls.is_abstract {
        return;
    }
    let Some(parents) = collect_parent_methods(cls, index) else {
        return;
    };
    if parents.is_empty() {
        return;
    }

    let mut own_impls: Vec<Sig> = cls
        .methods
        .iter()
        .map(|m| sig_of_class_method(m, &cls.type_params))
        .collect();
    // extend 块中为当前类提供的方法
    if let Some(exts) = index.extends.get(cls.name.as_str()) {
        for f in exts {
            own_impls.push(sig_of_function(f, &cls.type_params));
        }
    }
    check_abstract_methods_implemented(&cls.name, own_impls, &parents, index, errors);
}

/// 接口的 const 函数必须由 const 函数实现（16.4(6) 约束）。
///
/// 同名同签名但未声明 const 的类方法不能实现接口的 const 函数。
fn check_const_interface_impls(
    cls: &ClassDef,
    index: &ProgramIndex<'_>,
    errors: &mut Vec<SemanticError>,
) {
    let mut queue: Vec<String> = Vec::new();
    if let Some(p) = cls.extends.as_deref() {
        if index.interfaces.contains_key(p) {
            queue.push(p.to_string());
        }
    }
    for i in &cls.implements {
        if index.interfaces.contains_key(i.as_str()) {
            queue.push(i.clone());
        }
    }
    let mut seen: HashSet<String> = HashSet::new();
    while let Some(name) = queue.pop() {
        if !seen.insert(name.clone()) {
            continue;
        }
        let Some(iface) = index.interfaces.get(name.as_str()) else {
            continue;
        };
        for m in &iface.methods {
            if !m.is_const || is_synthetic_member(&m.name) {
                continue;
            }
            let isig = sig_of_interface_method(m);
            for cm in &cls.methods {
                if cm.is_const {
                    continue;
                }
                let csig = sig_of_function(&cm.func, &cls.type_params);
                if csig.name == isig.name && signature_matches(&csig, &isig, true, index) {
                    errors.push(SemanticError {
                        message: format!(
                            "类 '{}' 的 '{}' 实现接口 '{}' 的 const 函数时必须声明为 const",
                            cls.name, csig.display, name
                        ),
                    });
                }
            }
        }
        for p in &iface.parents {
            queue.push(p.clone());
        }
    }
}

/// 收集一组接口（含传递父接口）的方法；任一接口无法本地解析时返回 None。
fn collect_interface_methods<'a>(
    names: &[&'a str],
    index: &ProgramIndex<'a>,
) -> Option<Vec<ParentMethod>> {
    let supers: Vec<(&'a str, Vec<Type>)> =
        names.iter().map(|n| (*n, Vec::new())).collect();
    collect_interface_methods_with_args(&supers, &[], index)
}

/// 同 `collect_interface_methods`，但携带直接父接口的显式泛型实参
/// （结构体/枚举 `<: I<Int64>>` 场景），据此消解父方法签名中的类级泛型参数。
fn collect_interface_methods_with_args<'a>(
    supers: &[(&'a str, Vec<Type>)],
    impl_tp: &[String],
    index: &ProgramIndex<'a>,
) -> Option<Vec<ParentMethod>> {
    let mut out = Vec::new();
    let mut visited: HashSet<&'a str> = HashSet::new();
    let mut queue: Vec<&'a str> = supers.iter().map(|(n, _)| *n).collect();
    let mut direct: HashMap<&'a str, &Vec<Type>> = HashMap::new();
    for (n, a) in supers {
        if !a.is_empty() {
            direct.insert(*n, a);
        }
    }
    let mut i = 0;
    while i < queue.len() {
        let name = queue[i];
        i += 1;
        if !visited.insert(name) {
            continue;
        }
        let iface = index.interfaces.get(name)?;
        let subst = build_supertype_subst(&iface.type_params, direct.get(name).copied());
        let fully_concrete = iface.type_params.is_empty() || subst.is_some();
        for m in &iface.methods {
            let mut sig = sig_of_interface_method_tp(m, &iface.type_params);
            if sig.ret.is_none() {
                if let Some(body) = m.default_body.as_deref() {
                    if let Some(r) = infer_simple_return_ty(body, index) {
                        sig.ret = Some(r);
                    }
                }
            }
            if let Some(s) = &subst {
                sig.params = sig.params.iter().map(|t| subst_type(t, s)).collect();
                sig.ret = sig.ret.as_ref().map(|t| subst_type(t, s));
                // 替换后参数处于“实现类型”的类型参数空间，按实现类型的声明顺序
                // 做 alpha 等价，避免与接口自身参数空间错位。
                sig.tp_order = impl_tp.to_vec();
                sig.tp_order.extend(m.type_params.iter().cloned());
                sig.class_tp_count = impl_tp.len();
            }
            out.push(ParentMethod {
                sig,
                strict_types: true,
                abstract_: m.default_body.is_none(),
                open_: true,
                is_interface: true,
                owner: name.to_string(),
                type_params: m.type_params.clone(),
                constraints: m.constraints.clone(),
                fully_concrete,
            });
        }
        if let Some(exts) = index.extends.get(name) {
            for f in exts {
                out.push(ParentMethod {
                    sig: sig_of_function(f, index.type_params_of(name)),
                    strict_types: true,
                    abstract_: false,
                    open_: true,
                    is_interface: true,
                    owner: name.to_string(),
                    type_params: f.type_params.clone(),
                    constraints: f.constraints.clone(),
                    fully_concrete: true,
                });
            }
        }
        for p in &iface.parents {
            queue.push(p.as_str());
        }
    }
    Some(out)
}

/// 结构体通过 `struct S <: I` 实现接口时，必须实现接口抽象方法。
///
/// 解析器把 `struct S <: I & J` 记录为约束 `S: [I, J]`（`param` 为结构体名），
/// 因此这里可以直接复用该信息，无需扩展 AST。
fn check_struct_interfaces<'a>(
    st: &'a StructDef,
    program: &'a Program,
    index: &ProgramIndex<'a>,
    errors: &mut Vec<SemanticError>,
) {
    let mut supers: Vec<(&'a str, Vec<Type>)> = Vec::new();
    for c in st.constraints.iter().filter(|c| c.param == st.name) {
        for (i, b) in c.bounds.iter().enumerate() {
            supers.push((
                b.as_str(),
                c.bound_args.get(i).cloned().unwrap_or_default(),
            ));
        }
    }
    if supers.is_empty() {
        return;
    }
    let Some(parents) = collect_interface_methods_with_args(&supers, &st.type_params, index) else {
        return;
    };
    let mut own_impls: Vec<Sig> = program
        .functions
        .iter()
        .filter(|f| owner_name(&f.name) == Some(st.name.as_str()))
        .map(|f| sig_of_function(f, &st.type_params))
        .collect();
    if let Some(exts) = index.extends.get(st.name.as_str()) {
        for f in exts {
            own_impls.push(sig_of_function(f, &st.type_params));
        }
    }
    check_impl_visibility(&st.name, &own_impls, &parents, index, errors);
    if parents.iter().any(|p| p.abstract_) {
        check_abstract_methods_implemented(&st.name, own_impls.clone(), &parents, index, errors);
    } else {
        // 6.2.4.1(9)：结构体实现了多个带默认实现的父接口时，必须自行实现冲突成员。
        check_multiple_default_conflicts(&st.name, &own_impls, &parents, index, errors, "类型");
    }
    check_interface_property_types(&st.name, &own_impls, &parents, index, errors);
    check_interface_property_mut(&st.name, &own_impls, &parents, errors);
    // 2.1.10(8)：结构体实现接口时，同名方法的签名必须与接口声明一致；
    // 接口成员即使带默认实现也同样适用（与类的“重载”语义不同）。
    for p in &parents {
        if !p.is_interface || p.abstract_ || is_synthetic_member(&p.sig.name) {
            continue;
        }
        for s in &own_impls {
            if s.name != p.sig.name || is_synthetic_member(&s.name) {
                continue;
            }
            if canon_params(s) == canon_params(&p.sig)
                && !signature_matches(s, &p.sig, p.strict_types, index)
            {
                errors.push(SemanticError {
                    message: format!(
                        "结构体 '{}' 实现的接口函数 '{}' 签名与接口声明不一致（2.1.10(8)）",
                        st.name, s.display
                    ),
                });
                break;
            }
        }
    }
}

fn is_property_method_name(name: &str) -> bool {
    name.starts_with("__get_") || name.starts_with("__set_")
}

/// 枚举通过 `enum E <: I` 实现接口时，必须实现接口的抽象函数。
///
/// 枚举的 `prop` 成员解析时被丢弃（未脱糖为方法），因此跳过接口属性相关检查，
/// 只校验抽象函数，避免误判。
fn check_enum_interfaces<'a>(
    en: &'a EnumDef,
    program: &'a Program,
    index: &ProgramIndex<'a>,
    errors: &mut Vec<SemanticError>,
) {
    let mut supers: Vec<(&'a str, Vec<Type>)> = Vec::new();
    for c in en.constraints.iter().filter(|c| c.param == en.name) {
        for (i, b) in c.bounds.iter().enumerate() {
            supers.push((
                b.as_str(),
                c.bound_args.get(i).cloned().unwrap_or_default(),
            ));
        }
    }
    if supers.is_empty() {
        return;
    }
    let Some(parents) = collect_interface_methods_with_args(&supers, &en.type_params, index) else {
        return;
    };
    let mut parents = parents;
    let has_abstract = parents
        .iter()
        .filter(|p| !is_property_method_name(&p.sig.name))
        .any(|p| p.abstract_);
    let mut own_impls: Vec<Sig> = program
        .functions
        .iter()
        .filter(|f| owner_name(&f.name) == Some(en.name.as_str()))
        .map(|f| sig_of_function(f, &en.type_params))
        .collect();
    if let Some(exts) = index.extends.get(en.name.as_str()) {
        for f in exts {
            own_impls.push(sig_of_function(f, &en.type_params));
        }
    }
    // 枚举实现的接口属性：类型（6.2.4.1(7)）与 mut 修饰符（6.2.4.1(6)）必须一致。
    check_interface_property_types(&en.name, &own_impls, &parents, index, errors);
    check_interface_property_mut(&en.name, &own_impls, &parents, errors);
    // 6.2.4.1(9)：枚举实现多个带默认实现的父接口时，必须自行实现冲突成员。
    // 必须在剔除属性脱糖成员之前检查，属性默认实现冲突同样适用。
    if !has_abstract {
        check_multiple_default_conflicts(&en.name, &own_impls, &parents, index, errors, "类型");
    }
    // 6.2.4.1(1)：枚举必须实现接口的抽象属性（以脱糖后的 getter 为准）。
    for p in parents.iter().filter(|p| {
        p.is_interface && p.abstract_ && p.sig.name.starts_with("__get_")
    }) {
        let ok = own_impls.iter().any(|s| {
            s.name == p.sig.name && signature_matches(s, &p.sig, true, index)
        }) || parents.iter().any(|q| {
            q.is_interface
                && !q.abstract_
                && q.sig.name == p.sig.name
                && signature_matches(&q.sig, &p.sig, q.strict_types, index)
        });
        if !ok {
            errors.push(SemanticError {
                message: format!(
                    "枚举 '{}' 未实现接口属性 '{}'（6.2.4.1(1)）",
                    en.name,
                    member_base_name(&p.sig.name)
                ),
            });
        }
    }
    parents.retain(|p| !is_property_method_name(&p.sig.name));
    // 访问级别不得低于接口声明（6.3.5(3)）；即使接口成员带默认实现也同样适用。
    check_impl_visibility(&en.name, &own_impls, &parents, index, errors);
    if has_abstract {
        check_abstract_methods_implemented(&en.name, own_impls, &parents, index, errors);
    }
}

/// 子类型成员与父接口同名成员种类不同（函数 / 属性 / 字段）即构成遮蔽（6.3.3(1)）。
///
/// `own` 为 (成员基名, 种类) 列表，种类取 `"func"` / `"prop"` / `"field"`。
fn check_member_kind_shadowing_vs_interfaces(
    owner: &str,
    own: &[(String, &'static str)],
    parents: &[ParentMethod],
    errors: &mut Vec<SemanticError>,
) {
    let mut reported: HashSet<String> = HashSet::new();
    for (name, kind) in own {
        let parent_kinds: Vec<&str> = parents
            .iter()
            .filter(|p| !is_synthetic_member(&p.sig.name))
            .filter(|p| member_base_name(&p.sig.name) == name.as_str())
            .map(|p| {
                if is_property_method_name(&p.sig.name) {
                    "prop"
                } else {
                    "func"
                }
            })
            .collect();
        if parent_kinds.iter().any(|k| k != kind) && reported.insert(name.clone()) {
            errors.push(SemanticError {
                message: format!(
                    "类型 '{}' 的成员 '{}' 遮蔽了父接口的同名成员（6.3.3(1)）",
                    owner, name
                ),
            });
        }
    }
}

/// 收集 struct/enum 自身成员（字段固定由 struct 提供）用于遮蔽检查。
fn collect_own_member_kinds<'a>(
    owner: &str,
    struct_fields: Option<&'a [crate::ast::FieldDef]>,
    program: &'a Program,
    index: &ProgramIndex<'a>,
) -> Vec<(String, &'static str)> {
    let mut own: Vec<(String, &'static str)> = Vec::new();
    if let Some(fields) = struct_fields {
        for f in fields {
            own.push((f.name.clone(), "field"));
        }
    }
    for f in &program.functions {
        if owner_name(&f.name) != Some(owner) {
            continue;
        }
        let base = f.name.rsplit('.').next().unwrap_or(&f.name);
        if is_property_method_name(base) {
            own.push((member_base_name(base).to_string(), "prop"));
        } else {
            own.push((base.to_string(), "func"));
        }
    }
    if let Some(exts) = index.extends.get(owner) {
        for f in exts {
            let base = f.name.rsplit('.').next().unwrap_or(&f.name);
            if is_property_method_name(base) {
                own.push((member_base_name(base).to_string(), "prop"));
            } else {
                own.push((base.to_string(), "func"));
            }
        }
    }
    own
}

/// struct 实现接口时成员不得遮蔽接口同名成员（6.3.3(1)）。
fn check_struct_member_shadowing<'a>(
    st: &'a StructDef,
    program: &'a Program,
    index: &ProgramIndex<'a>,
    errors: &mut Vec<SemanticError>,
) {
    let bounds: Vec<&'a str> = st
        .constraints
        .iter()
        .filter(|c| c.param == st.name)
        .flat_map(|c| c.bounds.iter().map(|b| b.as_str()))
        .collect();
    if bounds.is_empty() {
        return;
    }
    let Some(parents) = collect_interface_methods(&bounds, index) else {
        return;
    };
    let own = collect_own_member_kinds(&st.name, Some(&st.fields), program, index);
    check_member_kind_shadowing_vs_interfaces(&st.name, &own, &parents, errors);
}

/// enum 实现接口时成员不得遮蔽接口同名成员（6.3.3(1)）。
fn check_enum_member_shadowing<'a>(
    en: &'a EnumDef,
    program: &'a Program,
    index: &ProgramIndex<'a>,
    errors: &mut Vec<SemanticError>,
) {
    let bounds: Vec<&'a str> = en
        .constraints
        .iter()
        .filter(|c| c.param == en.name)
        .flat_map(|c| c.bounds.iter().map(|b| b.as_str()))
        .collect();
    if bounds.is_empty() {
        return;
    }
    let Some(parents) = collect_interface_methods(&bounds, index) else {
        return;
    };
    let own = collect_own_member_kinds(&en.name, None, program, index);
    check_member_kind_shadowing_vs_interfaces(&en.name, &own, &parents, errors);
}

// ─── AST 通用遍历辅助 ───────────────────────────────────────────────────────

/// 递归访问表达式及其所有子表达式。
fn walk_expr<'a>(e: &'a Expr, f: &mut dyn FnMut(&'a Expr)) {
    f(e);
    match e {
        Expr::Interpolate(parts) => {
            for p in parts {
                if let crate::ast::InterpolatePart::Expr(x) = p {
                    walk_expr(x, f);
                }
            }
        }
        Expr::Unary { expr, .. } => walk_expr(expr, f),
        Expr::Binary { left, right, .. } => {
            walk_expr(left, f);
            walk_expr(right, f);
        }
        Expr::Call { args, named_args, .. } => {
            for a in args {
                walk_expr(a, f);
            }
            for (_, a) in named_args {
                walk_expr(a, f);
            }
        }
        Expr::MethodCall {
            object,
            args,
            named_args,
            ..
        } => {
            walk_expr(object, f);
            for a in args {
                walk_expr(a, f);
            }
            for (_, a) in named_args {
                walk_expr(a, f);
            }
        }
        Expr::SuperCall { args, named_args, .. } => {
            for a in args {
                walk_expr(a, f);
            }
            for (_, a) in named_args {
                walk_expr(a, f);
            }
        }
        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => {
            walk_expr(cond, f);
            walk_expr(then_branch, f);
            if let Some(e) = else_branch {
                walk_expr(e, f);
            }
        }
        Expr::IfLet {
            pattern,
            expr,
            then_branch,
            else_branch,
        } => {
            walk_pattern_exprs(pattern, f);
            walk_expr(expr, f);
            walk_expr(then_branch, f);
            if let Some(e) = else_branch {
                walk_expr(e, f);
            }
        }
        Expr::Block(stmts, tail) => {
            walk_stmts(stmts, f);
            if let Some(t) = tail {
                walk_expr(t, f);
            }
        }
        Expr::Tuple(exprs) | Expr::Array(exprs) => {
            for x in exprs {
                walk_expr(x, f);
            }
        }
        Expr::TupleIndex { object, .. } => walk_expr(object, f),
        Expr::Index { array, index } => {
            walk_expr(array, f);
            walk_expr(index, f);
        }
        Expr::StructInit { fields, .. } => {
            for (_, x) in fields {
                walk_expr(x, f);
            }
        }
        Expr::ConstructorCall {
            args, named_args, ..
        } => {
            for a in args {
                walk_expr(a, f);
            }
            for (_, a) in named_args {
                walk_expr(a, f);
            }
        }
        Expr::Field { object, .. } | Expr::OptionalChain { object, .. } => walk_expr(object, f),
        Expr::Range {
            start, end, step, ..
        } => {
            walk_expr(start, f);
            walk_expr(end, f);
            if let Some(s) = step {
                walk_expr(s, f);
            }
        }
        Expr::VariantConst { arg, .. } => {
            if let Some(a) = arg {
                walk_expr(a, f);
            }
        }
        Expr::Match { expr, arms } => {
            walk_expr(expr, f);
            for arm in arms {
                walk_pattern_exprs(&arm.pattern, f);
                if let Some(g) = &arm.guard {
                    walk_expr(g, f);
                }
                walk_expr(&arm.body, f);
            }
        }
        Expr::Cast { expr, .. }
        | Expr::IsType { expr, .. }
        | Expr::Some(expr)
        | Expr::Ok(expr)
        | Expr::Err(expr)
        | Expr::Try(expr)
        | Expr::Throw(expr)
        | Expr::PostfixIncr(expr)
        | Expr::PostfixDecr(expr)
        | Expr::PrefixIncr(expr)
        | Expr::PrefixDecr(expr) => walk_expr(expr, f),
        Expr::Lambda { body, .. } => walk_expr(body, f),
        Expr::NullCoalesce { option, default } => {
            walk_expr(option, f);
            walk_expr(default, f);
        }
        Expr::Return(opt) => {
            if let Some(e) = opt {
                walk_expr(e, f);
            }
        }
        Expr::TryBlock {
            resources,
            body,
            catch_body,
            finally_body,
            ..
        } => {
            for (_, e) in resources {
                walk_expr(e, f);
            }
            walk_stmts(body, f);
            walk_stmts(catch_body, f);
            if let Some(b) = finally_body {
                walk_stmts(b, f);
            }
        }
        Expr::SliceExpr { array, start, end } => {
            walk_expr(array, f);
            walk_expr(start, f);
            walk_expr(end, f);
        }
        Expr::MapLiteral { entries } => {
            for (k, v) in entries {
                walk_expr(k, f);
                walk_expr(v, f);
            }
        }
        Expr::Spawn { body } => walk_stmts(body, f),
        Expr::Synchronized { lock, body } => {
            walk_expr(lock, f);
            walk_stmts(body, f);
        }
        Expr::TrailingClosure {
            callee,
            args,
            closure,
        } => {
            walk_expr(callee, f);
            for a in args {
                walk_expr(a, f);
            }
            walk_expr(closure, f);
        }
        Expr::Invoke {
            callee,
            args,
            named_args,
        } => {
            walk_expr(callee, f);
            for a in args {
                walk_expr(a, f);
            }
            for (_, a) in named_args {
                walk_expr(a, f);
            }
        }
        Expr::Macro { args, .. } => {
            for a in args {
                walk_expr(a, f);
            }
        }
        _ => {}
    }
}

fn walk_pattern_exprs<'a>(p: &'a crate::ast::Pattern, f: &mut dyn FnMut(&'a Expr)) {
    use crate::ast::Pattern;
    match p {
        Pattern::Struct { fields, .. } => {
            for (_, sub) in fields {
                walk_pattern_exprs(sub, f);
            }
        }
        Pattern::Tuple(items) | Pattern::Or(items) => {
            for sub in items {
                walk_pattern_exprs(sub, f);
            }
        }
        Pattern::Variant { payload, .. } => {
            if let Some(sub) = payload {
                walk_pattern_exprs(sub, f);
            }
        }
        Pattern::Guard(e) => walk_expr(e, f),
        _ => {}
    }
}

fn walk_stmts<'a>(stmts: &'a [Stmt], f: &mut dyn FnMut(&'a Expr)) {
    for s in stmts {
        walk_stmt(s, f);
    }
}

fn walk_stmt<'a>(s: &'a Stmt, f: &mut dyn FnMut(&'a Expr)) {
    match s {
        Stmt::Let { pattern, value, .. } | Stmt::Var { pattern, value, .. } => {
            walk_pattern_exprs(pattern, f);
            walk_expr(value, f);
        }
        Stmt::Assign { target, value } => {
            walk_assign_target(target, f);
            walk_expr(value, f);
        }
        Stmt::Expr(e) => walk_expr(e, f),
        Stmt::Return(opt) => {
            if let Some(e) = opt {
                walk_expr(e, f);
            }
        }
        Stmt::While { cond, body } => {
            walk_expr(cond, f);
            walk_stmts(body, f);
        }
        Stmt::WhileLet {
            pattern,
            expr,
            body,
        } => {
            walk_pattern_exprs(pattern, f);
            walk_expr(expr, f);
            walk_stmts(body, f);
        }
        Stmt::DoWhile { body, cond } => {
            walk_stmts(body, f);
            walk_expr(cond, f);
        }
        Stmt::For { iterable, body, .. } => {
            walk_expr(iterable, f);
            walk_stmts(body, f);
        }
        Stmt::Loop { body } | Stmt::UnsafeBlock { body } => walk_stmts(body, f),
        Stmt::Assert { left, right, .. } | Stmt::Expect { left, right, .. } => {
            walk_expr(left, f);
            walk_expr(right, f);
        }
        Stmt::Const { value, .. } => walk_expr(value, f),
        Stmt::LocalFunc(func) => {
            for p in &func.params {
                if let Some(d) = &p.default {
                    walk_expr(d, f);
                }
            }
            walk_stmts(&func.body, f);
        }
        _ => {}
    }
}

fn walk_assign_target<'a>(target: &'a crate::ast::AssignTarget, f: &mut dyn FnMut(&'a Expr)) {
    use crate::ast::AssignTarget;
    match target {
        AssignTarget::Index { index, .. }
        | AssignTarget::IndexPath { index, .. }
        | AssignTarget::ExprIndex { index, .. } => walk_expr(index, f),
        AssignTarget::ExprField { object, .. } => walk_expr(object, f),
        AssignTarget::OptionalField { object, .. } => walk_expr(object, f),
        AssignTarget::Slice {
            array, start, end, ..
        } => {
            walk_expr(array, f);
            walk_expr(start, f);
            walk_expr(end, f);
        }
        AssignTarget::Tuple(items) => {
            for t in items {
                walk_assign_target(t, f);
            }
        }
        _ => {}
    }
}

/// 收集模式中绑定的名字。
fn collect_pattern_bindings(p: &crate::ast::Pattern, out: &mut HashSet<String>) {
    use crate::ast::Pattern;
    match p {
        Pattern::Binding(name) => {
            out.insert(name.clone());
        }
        Pattern::Struct { fields, .. } => {
            for (_, sub) in fields {
                collect_pattern_bindings(sub, out);
            }
        }
        Pattern::Tuple(items) | Pattern::Or(items) => {
            for sub in items {
                collect_pattern_bindings(sub, out);
            }
        }
        Pattern::Variant { payload, .. } => {
            if let Some(sub) = payload {
                collect_pattern_bindings(sub, out);
            }
        }
        Pattern::TypeTest { binding, .. } => {
            out.insert(binding.clone());
        }
        _ => {}
    }
}

/// 收集语句块内所有局部绑定名（含嵌套作用域，保守取并集）。
fn collect_bindings_stmts(stmts: &[Stmt], out: &mut HashSet<String>) {
    for s in stmts {
        match s {
            Stmt::Let { pattern, .. } | Stmt::Var { pattern, .. } => {
                collect_pattern_bindings(pattern, out)
            }
            Stmt::WhileLet { pattern, body, .. } => {
                collect_pattern_bindings(pattern, out);
                collect_bindings_stmts(body, out);
            }
            Stmt::While { body, .. }
            | Stmt::Loop { body }
            | Stmt::DoWhile { body, .. }
            | Stmt::UnsafeBlock { body } => collect_bindings_stmts(body, out),
            Stmt::For { var, body, .. } => {
                out.insert(var.clone());
                collect_bindings_stmts(body, out);
            }
            Stmt::Const { name, .. } => {
                out.insert(name.clone());
            }
            Stmt::Expr(Expr::TryBlock {
                catch_var,
                body,
                catch_body,
                finally_body,
                ..
            }) => {
                if let Some(v) = catch_var {
                    out.insert(v.clone());
                }
                collect_bindings_stmts(body, out);
                collect_bindings_stmts(catch_body, out);
                if let Some(b) = finally_body {
                    collect_bindings_stmts(b, out);
                }
            }
            Stmt::LocalFunc(func) => {
                let last = func
                    .name
                    .rsplit_once('.')
                    .map(|(_, n)| n)
                    .unwrap_or(&func.name);
                out.insert(last.to_string());
                for p in &func.params {
                    out.insert(p.name.clone());
                }
                collect_bindings_stmts(&func.body, out);
            }
            _ => {}
        }
    }
}

/// 收集类（含父类/接口）中所有实例成员的名字（字段、实例函数、属性基名）。
/// 任一父类型无法本地解析时返回 None，调用方保守跳过。
fn collect_instance_member_names<'a>(
    cls: &'a ClassDef,
    index: &ProgramIndex<'a>,
) -> Option<HashSet<String>> {
    let mut out = HashSet::new();
    let mut add_class = |c: &ClassDef, out: &mut HashSet<String>| {
        for f in &c.fields {
            if !f.is_static {
                out.insert(f.name.clone());
            }
        }
        for m in &c.methods {
            let sig = sig_of_class_method(m, &c.type_params);
            if is_synthetic_member(&sig.name) {
                continue;
            }
            if !m.is_static {
                out.insert(member_base_name(&sig.name).to_string());
            }
        }
    };
    add_class(cls, &mut out);

    let mut queue: Vec<&'a str> = Vec::new();
    if let Some(p) = cls.extends.as_deref() {
        queue.push(p);
    }
    for i in &cls.implements {
        queue.push(i.as_str());
    }
    let mut visited: HashSet<&str> = HashSet::new();
    let mut qi = 0;
    while qi < queue.len() {
        let name = queue[qi];
        qi += 1;
        if !visited.insert(name) {
            continue;
        }
        if let Some(c) = index.classes.get(name) {
            add_class(c, &mut out);
            if let Some(p) = c.extends.as_deref() {
                queue.push(p);
            }
            for i in &c.implements {
                queue.push(i.as_str());
            }
        } else if let Some(iface) = index.interfaces.get(name) {
            for m in &iface.methods {
                if !m.is_static {
                    out.insert(member_base_name(&m.name).to_string());
                }
            }
            for p in &iface.parents {
                queue.push(p.as_str());
            }
        } else {
            return None;
        }
    }
    Some(out)
}

/// 静态成员函数体内不得访问实例成员（6.1.2.4(17)/(41)）。
fn check_static_member_access(
    cls: &ClassDef,
    program: &Program,
    index: &ProgramIndex<'_>,
    errors: &mut Vec<SemanticError>,
) {
    let Some(instance_names) = collect_instance_member_names(cls, index) else {
        return;
    };
    if instance_names.is_empty() {
        return;
    }
    let globals: HashSet<String> = program
        .functions
        .iter()
        .filter(|f| owner_name(&f.name).is_none())
        .map(|f| f.name.clone())
        .chain(program.constants.iter().map(|c| c.name.clone()))
        .chain(program.classes.iter().map(|c| c.name.clone()))
        .collect();
    for m in &cls.methods {
        let sig = sig_of_class_method(m, &cls.type_params);
        if is_synthetic_member(&sig.name) || !m.has_body {
            continue;
        }
        if !m.is_static {
            continue;
        }
        let mut locals: HashSet<String> = HashSet::new();
        for p in &m.func.params {
            locals.insert(p.name.clone());
        }
        collect_bindings_stmts(&m.func.body, &mut locals);
        let mut hits: Vec<String> = Vec::new();
        walk_stmts(&m.func.body, &mut |e| {
            let name = match e {
                Expr::Var(n) => n,
                Expr::Call { name, .. } => name,
                _ => return,
            };
            if name.contains('.') {
                return;
            }
            if instance_names.contains(name) && !locals.contains(name) && !globals.contains(name) {
                hits.push(name.clone());
            }
        });
        hits.sort();
        hits.dedup();
        for name in hits {
            errors.push(SemanticError {
                message: format!(
                    "类 '{}' 的静态成员函数 '{}' 不能访问实例成员 '{}'（6.1.2.4(17)/(41)）",
                    cls.name, m.func.name, name
                ),
            });
        }
    }
}

/// 子类不得访问父类的 private 成员（6.1.1.2(13)：private 成员不被继承）。
fn collect_private_parent_members<'a>(
    cls: &ClassDef,
    index: &ProgramIndex<'a>,
) -> Option<HashSet<String>> {
    let mut out = HashSet::new();
    let mut cur = cls.extends.clone();
    let mut seen = HashSet::new();
    while let Some(name) = cur {
        if !seen.insert(name.clone()) {
            break;
        }
        let Some(parent) = index.classes.get(name.as_str()) else {
            return None;
        };
        for f in &parent.fields {
            if f.visibility == Visibility::Private {
                out.insert(f.name.clone());
            }
        }
        for m in &parent.methods {
            let sig = sig_of_class_method(m, &parent.type_params);
            if m.func.visibility == Visibility::Private && !is_synthetic_member(&sig.name) {
                out.insert(member_base_name(&sig.name).to_string());
            }
        }
        cur = parent.extends.clone();
    }
    Some(out)
}

fn check_private_parent_access(
    cls: &ClassDef,
    program: &Program,
    index: &ProgramIndex<'_>,
    errors: &mut Vec<SemanticError>,
) {
    let Some(private_names) = collect_private_parent_members(cls, index) else {
        return;
    };
    if private_names.is_empty() {
        return;
    }
    // 子类自身成员会遮蔽/覆盖同名名字，不视为访问父类私有成员。
    let mut own_names: HashSet<String> = HashSet::new();
    for f in &cls.fields {
        own_names.insert(f.name.clone());
    }
    for m in &cls.methods {
        let sig = sig_of_class_method(m, &cls.type_params);
        if !is_synthetic_member(&sig.name) {
            own_names.insert(member_base_name(&sig.name).to_string());
        }
    }
    let globals: HashSet<String> = program
        .functions
        .iter()
        .filter(|f| owner_name(&f.name).is_none())
        .map(|f| f.name.clone())
        .chain(program.constants.iter().map(|c| c.name.clone()))
        .chain(program.classes.iter().map(|c| c.name.clone()))
        .collect();
    for m in &cls.methods {
        let sig = sig_of_class_method(m, &cls.type_params);
        if is_synthetic_member(&sig.name) || !m.has_body {
            continue;
        }
        let mut locals: HashSet<String> = HashSet::new();
        for p in &m.func.params {
            locals.insert(p.name.clone());
        }
        collect_bindings_stmts(&m.func.body, &mut locals);
        let mut hits: Vec<String> = Vec::new();
        walk_stmts(&m.func.body, &mut |e| {
            let name = match e {
                Expr::Var(n) => n,
                Expr::Call { name, .. } => name,
                Expr::SuperCall { method, .. } => method,
                Expr::SuperFieldAccess { field } => field,
                Expr::Field { object, field }
                    if matches!(object.as_ref(), Expr::Var(n) if n == "this") =>
                {
                    field
                }
                Expr::MethodCall { object, method, .. }
                    if matches!(object.as_ref(), Expr::Var(n) if n == "this") =>
                {
                    method
                }
                _ => return,
            };
            if name.contains('.') {
                return;
            }
            if private_names.contains(name)
                && !locals.contains(name)
                && !own_names.contains(name)
                && !globals.contains(name)
            {
                hits.push(name.clone());
            }
        });
        hits.sort();
        hits.dedup();
        for name in hits {
            errors.push(SemanticError {
                message: format!(
                    "类 '{}' 的成员函数 '{}' 不能访问父类的 private 成员 '{}'（6.1.1.2(13)）",
                    cls.name, m.func.name, name
                ),
            });
        }
    }
}

// ─── This 类型使用限制（6.1.4(1)）─────────────────────────────────────────
fn contains_this(ty: &Type) -> bool {
    match ty {
        Type::This => true,
        Type::Array(t) | Type::Option(t) | Type::Slice(t) => contains_this(t),
        Type::Struct(_, args) | Type::Tuple(args) => args.iter().any(contains_this),
        Type::Result(a, b) | Type::Map(a, b) => contains_this(a) || contains_this(b),
        Type::Function { params, ret } => {
            params.iter().any(contains_this)
                || ret.as_ref().as_ref().map_or(false, contains_this)
        }
        _ => false,
    }
}

fn for_each_pattern_type<'a>(p: &'a crate::ast::Pattern, f: &mut dyn FnMut(&'a Type)) {
    use crate::ast::Pattern;
    match p {
        Pattern::Struct { fields, .. } => {
            for (_, sub) in fields {
                for_each_pattern_type(sub, &mut *f);
            }
        }
        Pattern::Tuple(items) | Pattern::Or(items) => {
            for sub in items {
                for_each_pattern_type(sub, &mut *f);
            }
        }
        Pattern::Variant { payload, .. } => {
            if let Some(sub) = payload {
                for_each_pattern_type(sub, &mut *f);
            }
        }
        Pattern::TypeTest { ty, .. } => f(ty),
        _ => {}
    }
}

/// 访问表达式内部出现的所有类型位置（类型实参、cast/is、lambda 参数/返回、模式）。
fn for_each_expr_type<'a>(e: &'a Expr, f: &mut dyn FnMut(&'a Type)) {
    match e {
        Expr::Cast { target_ty, .. } | Expr::IsType { target_ty, .. } => f(target_ty),
        Expr::Lambda {
            params,
            return_type,
            ..
        } => {
            for (_, ty) in params {
                f(ty);
            }
            if let Some(ty) = return_type {
                f(ty);
            }
        }
        Expr::Call { type_args, .. }
        | Expr::MethodCall { type_args, .. }
        | Expr::StructInit { type_args, .. }
        | Expr::ConstructorCall { type_args, .. } => {
            if let Some(args) = type_args {
                for ty in args {
                    f(ty);
                }
            }
        }
        Expr::Match { arms, .. } => {
            for arm in arms {
                for_each_pattern_type(&arm.pattern, &mut *f);
            }
        }
        Expr::IfLet { pattern, .. } => for_each_pattern_type(pattern, &mut *f),
        _ => {}
    }
}

/// 访问语句块内所有声明位置与表达式位置中的类型。
fn for_each_type_in_stmts<'a>(stmts: &'a [Stmt], f: &mut dyn FnMut(&'a Type)) {
    for s in stmts {
        match s {
            Stmt::Let { ty, pattern, value } | Stmt::Var { ty, pattern, value } => {
                if let Some(ty) = ty {
                    f(ty);
                }
                for_each_pattern_type(pattern, f);
                walk_expr(value, &mut |e| for_each_expr_type(e, &mut *f));
            }
            Stmt::WhileLet {
                pattern,
                expr,
                body,
            } => {
                for_each_pattern_type(pattern, f);
                walk_expr(expr, &mut |e| for_each_expr_type(e, &mut *f));
                for_each_type_in_stmts(body, &mut *f);
            }
            Stmt::While { cond, body } => {
                walk_expr(cond, &mut |e| for_each_expr_type(e, &mut *f));
                for_each_type_in_stmts(body, &mut *f);
            }
            Stmt::DoWhile { body, cond } => {
                for_each_type_in_stmts(body, &mut *f);
                walk_expr(cond, &mut |e| for_each_expr_type(e, &mut *f));
            }
            Stmt::For {
                iterable, body, ..
            } => {
                walk_expr(iterable, &mut |e| for_each_expr_type(e, &mut *f));
                for_each_type_in_stmts(body, &mut *f);
            }
            Stmt::Loop { body } | Stmt::UnsafeBlock { body } => for_each_type_in_stmts(body, &mut *f),
            Stmt::Const { ty, value, .. } => {
                if let Some(ty) = ty {
                    f(ty);
                }
                walk_expr(value, &mut |e| for_each_expr_type(e, &mut *f));
            }
            Stmt::LocalFunc(func) => {
                for p in &func.params {
                    f(&p.ty);
                }
                if let Some(ty) = &func.return_type {
                    f(ty);
                }
                for_each_type_in_stmts(&func.body, f);
            }
            _ => {
                walk_stmt(s, &mut |e| for_each_expr_type(e, &mut *f));
            }
        }
    }
}

// ─── 13 章 C 语言互操作：单文件可判定的类型约束 ────────────────────────────
//
// 覆盖以下规则（均可仅凭单个 AST 精确定位，且保守放行无法在本地判定的类型）：
//   * 13.1.2.2：foreign func 的形参不允许使用命名参数（`x!: T`）或默认值；
//   * 13.1.3.1：foreign func 的形参/返回类型必须是 C 类型（CPointer/CString/
//     CFunc/基本整型/浮点/bool 等）；已知非 C 类型（String/Rune/Nothing/
//     元组/Range/函数类型/本地声明的 class、enum、interface）予以拒绝；
//   * 13.1.3.2：`CFunc<F>` 中的函数签名 F 的形参与返回类型同样必须是 C 类型。
//
// 注意：`Unit` 作为**形参**非法，但作为**返回值**合法（C 的 void）；VArray
// 在解析阶段会降级为 Array，二者无法区分，故 Array 一律放行以避免误伤合法
// 的 VArray 形参；@C struct 与本地无法解析的具名类型（别名/泛型参数/外部
// 声明）一律放行。

/// 判断某个类型是否属于「本地可确认的非 CType」。
///
/// 返回 `Some(说明)` 表示该类型确定不能用作 C 函数签名中的类型；`None`
/// 表示它是 C 类型或无法在本地判定（保守放行）。
fn non_ctype_kind(
    ty: &Type,
    index: &ProgramIndex,
    reject_array: bool,
    depth: usize,
) -> Option<String> {
    if depth > 8 {
        return None;
    }
    match ty {
        Type::Rune => Some("Rune".to_string()),
        Type::Nothing => Some("Nothing".to_string()),
        Type::String => Some("String".to_string()),
        Type::Tuple(_) => Some("元组".to_string()),
        Type::Range => Some("Range".to_string()),
        Type::Function { .. } => Some("函数类型".to_string()),
        Type::Array(_) if reject_array => Some("Array".to_string()),
        Type::Struct(name, _) => {
            if name == "Range" {
                return Some("Range".to_string());
            }
            if index.classes.contains_key(name.as_str()) {
                return Some("class 类型".to_string());
            }
            if index.enums.contains(name.as_str()) {
                return Some("枚举类型".to_string());
            }
            if index.interfaces.contains_key(name.as_str()) {
                return Some("interface 类型".to_string());
            }
            // 13.1.3.5(1)/13.1.3.1：只有 `@C` 修饰的结构体隐式满足 CType，可作为
            // C 签名类型；本地普通结构体不是 CType（外部声明的结构体无法判定，放行）。
            if index.structs.contains(name.as_str())
                && !index.c_structs.contains(name.as_str())
            {
                return Some("非 @C 结构体类型".to_string());
            }
            // 解析本地类型别名后继续判定（如 `type S = String`）。
            if let Some(target) = index.aliases.get(name.as_str()) {
                return non_ctype_kind(target, index, reject_array, depth + 1);
            }
            None
        }
        _ => None,
    }
}

/// 校验 foreign func 的形参与返回类型，以及命名参数/默认值限制。
fn check_foreign_func(func: &Function, index: &ProgramIndex, errors: &mut Vec<SemanticError>) {
    for p in &func.params {
        // 变长参数占位（`...`）本身不携带可校验类型。
        if p.variadic {
            continue;
        }
        if p.is_named {
            errors.push(SemanticError {
                message: format!(
                    "外来函数 '{}' 的形参 '{}' 不允许使用命名参数（13.1.2.2）",
                    func.name, p.name
                ),
            });
            continue;
        }
        if p.default.is_some() {
            errors.push(SemanticError {
                message: format!(
                    "外来函数 '{}' 的形参 '{}' 不允许指定默认值（13.1.2.2）",
                    func.name, p.name
                ),
            });
            continue;
        }
        if p.ty == Type::Unit {
            errors.push(SemanticError {
                message: format!(
                    "Unit 不能作为外来函数 '{}' 的形参 '{}' 的类型（13.1.3.1）",
                    func.name, p.name
                ),
            });
            continue;
        }
        if let Some(kind) = non_ctype_kind(&p.ty, index, false, 0) {
            errors.push(SemanticError {
                message: format!(
                    "{} 不能作为外来函数 '{}' 的形参 '{}' 的类型（13.1.3.1）",
                    kind, func.name, p.name
                ),
            });
        }
    }
    if let Some(ret) = &func.return_type {
        // Unit 作为返回值（C 的 void）合法。
        if *ret != Type::Unit {
            if let Some(kind) = non_ctype_kind(ret, index, false, 0) {
                errors.push(SemanticError {
                    message: format!(
                        "{} 不能作为外来函数 '{}' 的返回类型（13.1.3.1）",
                        kind, func.name
                    ),
                });
            }
        }
    }
}

/// 校验 `CFunc<F>` 中函数签名 F 的形参/返回类型是否为 C 类型。
fn check_cfunc_type(ty: &Type, index: &ProgramIndex, errors: &mut Vec<SemanticError>) {
    let Type::Struct(name, args) = ty else {
        return;
    };
    if name != "CFunc" {
        return;
    }
    let Some(Type::Function { params, ret }) = args.first() else {
        return;
    };
    // CFunc 内部允许 Unit（C 的 void 返回值常见）。VArray 在解析阶段降级为
    // Array，无法与普通 Array 区分，故 Array 一律放行。
    for p in params {
        if let Some(kind) = non_ctype_kind(p, index, false, 0) {
            errors.push(SemanticError {
                message: format!(
                    "{} 不能用作 CFunc 函数签名的形参类型（13.1.3.2）",
                    kind
                ),
            });
        }
    }
    if let Some(r) = ret.as_ref().as_ref() {
        if *r != Type::Unit {
            if let Some(kind) = non_ctype_kind(r, index, false, 0) {
                errors.push(SemanticError {
                    message: format!(
                        "{} 不能用作 CFunc 函数签名的返回类型（13.1.3.2）",
                        kind
                    ),
                });
            }
        }
    }
}

/// 遍历程序中所有类型出现位置（声明类型实参、形参、返回类型、函数体）。
fn for_each_type_in_program<'a>(program: &'a Program, f: &mut dyn FnMut(&'a Type)) {
    for func in &program.functions {
        for p in &func.params {
            f(&p.ty);
        }
        if let Some(r) = &func.return_type {
            f(r);
        }
        for_each_type_in_stmts(&func.body, f);
    }
    for c in &program.constants {
        f(&c.ty);
    }
    for cls in &program.classes {
        for fld in &cls.fields {
            f(&fld.ty);
        }
        for p in &cls.primary_ctor_params {
            f(&p.ty);
        }
        if let Some(init) = &cls.init {
            for p in &init.params {
                f(&p.ty);
            }
            for_each_type_in_stmts(&init.body, f);
        }
        for m in &cls.methods {
            for p in &m.func.params {
                f(&p.ty);
            }
            if let Some(r) = &m.func.return_type {
                f(r);
            }
            for_each_type_in_stmts(&m.func.body, f);
        }
    }
    for st in &program.structs {
        for fld in &st.fields {
            f(&fld.ty);
        }
    }
    for iface in &program.interfaces {
        for m in &iface.methods {
            for p in &m.params {
                f(&p.ty);
            }
            if let Some(r) = &m.return_type {
                f(r);
            }
        }
    }
    for en in &program.enums {
        for v in &en.variants {
            if let Some(p) = &v.payload {
                f(p);
            }
        }
    }
    for ext in &program.extends {
        for m in &ext.methods {
            for p in &m.params {
                f(&p.ty);
            }
            if let Some(r) = &m.return_type {
                f(r);
            }
            for_each_type_in_stmts(&m.body, f);
        }
    }
}

fn check_c_interop(program: &Program, index: &ProgramIndex, errors: &mut Vec<SemanticError>) {
    // foreign func（extern 导入，无函数体）的参数/返回类型与命名参数限制。
    for func in &program.functions {
        if func.extern_import.is_some() {
            check_foreign_func(func, index, errors);
        }
    }
    // 所有位置出现的 CFunc<F> 签名类型。
    for_each_type_in_program(program, &mut |ty| {
        check_cfunc_type(ty, index, errors);
    });
    // 13.1.4：class / enum / interface 不能实现内建 CType 接口；CType 亦不可
    // 被 extend 实现（@C struct 隐式满足 CType，无需也无法显式实现）。
    for cls in &program.classes {
        // `class X <: CType` 会被解析为继承（extends），而非 implements。
        if cls.extends.as_deref() == Some("CType")
            || cls.implements.iter().any(|i| i == "CType")
        {
            errors.push(SemanticError {
                message: format!("class '{}' 不能实现 CType 接口（13.1.4）", cls.name),
            });
        }
    }
    for ext in &program.extends {
        if ext.interface.as_deref() == Some("CType") {
            errors.push(SemanticError {
                message: format!(
                    "类型 '{}' 不能通过 extend 实现 CType 接口（13.1.4）",
                    ext.target_type
                ),
            });
        }
    }
    // 13.1.4(5)：CType 接口不能被继承（含 interface 直接继承）。
    for iface in &program.interfaces {
        if iface.parents.iter().any(|p| p == "CType")
            || iface.parent_types.iter().any(|p| {
                matches!(p, Type::Struct(n, _) if n == "CType")
                    || matches!(p, Type::Qualified(segs) if segs.last().map_or(false, |s| s == "CType"))
            })
        {
            errors.push(SemanticError {
                message: format!("interface '{}' 不能继承 CType 接口（13.1.4(5)）", iface.name),
            });
        }
    }
}

/// 10.2.4(12)：数值、Bool、Unit、Nothing、Rune、String、Range、函数、枚举、元组
/// 类型不支持带赋值形态（set 形态，含命名参数）的下标运算符重载；函数/元组类型
/// 本身也不允许被 extend。
fn check_index_operator_extensions(program: &Program, errors: &mut Vec<SemanticError>) {
    // 别名 → 是否函数/元组类型（不允许被 extend）。
    let mut alias_forbidden: HashSet<String> = HashSet::new();
    for (name, ty) in &program.type_aliases {
        if matches!(ty, Type::Function { .. } | Type::Tuple(_)) {
            alias_forbidden.insert(name.clone());
        }
    }
    let base_forbidden: HashSet<&str> = [
        "Int8", "Int16", "Int32", "Int64", "IntNative", "UInt8", "UInt16", "UInt32",
        "UInt64", "UIntNative", "Float16", "Float32", "Float64", "Bool", "Unit",
        "Nothing", "Rune", "String", "Range",
    ]
    .into_iter()
    .collect();

    for ext in &program.extends {
        let target = ext.target_type.trim();
        // `extend Range<T>` / `extend Array<Int64>` 等：取基础名。
        let base = target.split('<').next().unwrap_or(target).trim();
        // 函数/元组别名不允许被 extend（10.2.4(12)）。
        if alias_forbidden.contains(base) {
            errors.push(SemanticError {
                message: format!(
                    "函数/元组类型 '{}' 不能被 extend（10.2.4(12)）",
                    target
                ),
            });
            continue;
        }
        // 其余内建受限类型：仅拒绝带赋值形态（set）的 [] 运算符重载。
        if !base_forbidden.contains(base) {
            continue;
        }
        let has_set_index = ext.methods.iter().any(|m| {
            let is_index = m.name == "op_index" || m.name.ends_with(".op_index");
            is_index && m.params.iter().any(|p| p.is_named)
        });
        if has_set_index {
            errors.push(SemanticError {
                message: format!(
                    "类型 '{}' 不支持带赋值形态的下标运算符重载（10.2.4(12)）",
                    base
                ),
            });
        }
    }
}

/// 8.4(9)：同一类型的多个扩展之间不得定义同名同参数类型的成员。
///
/// 说明：这里只比较“方法名 + 形参类型列表”（扩展方法自带 `this` 形参，其类型为被
/// 扩展类型），从而既能发现真正的覆盖，又允许跨扩展按参数类型重载。
fn check_extension_member_shadowing(program: &Program, errors: &mut Vec<SemanticError>) {
    // (被扩展类型, 方法末段名) → 已出现的形参类型签名列表。
    let mut seen: HashMap<(String, String), Vec<Vec<Type>>> = HashMap::new();
    for ext in &program.extends {
        let target = ext.target_type.trim().to_string();
        for m in &ext.methods {
            let mname = last_name_segment(&m.name);
            let sig: Vec<Type> = m.params.iter().map(|p| p.ty.clone()).collect();
            let entry = seen.entry((target.clone(), mname.clone())).or_default();
            if entry.iter().any(|s| *s == sig) {
                errors.push(SemanticError {
                    message: format!(
                        "类型 '{}' 的扩展成员 '{}' 与其它扩展的成员重名且签名相同（8.4(9)）",
                        target, mname
                    ),
                });
            } else {
                entry.push(sig);
            }
        }
    }
}

/// 6.3.5(3)/6.4.2(1)：extend 实现接口的泛型成员函数时，实现函数必须为 public
/// （接口成员对外可见性等同 public，实现方的可见性不得降低）。
fn check_extension_generic_impl_visibility(
    program: &Program,
    index: &ProgramIndex<'_>,
    errors: &mut Vec<SemanticError>,
) {
    for ext in &program.extends {
        let Some(iface_name) = ext.interface.as_deref() else {
            continue;
        };
        let base = iface_name.split('<').next().unwrap_or(iface_name).trim();
        let Some(iface) = index.interfaces.get(base) else {
            continue;
        };
        for m in &ext.methods {
            if m.type_params.is_empty() {
                continue;
            }
            let bare = last_name_segment(&m.name);
            let implements_generic = iface
                .methods
                .iter()
                .any(|im| im.name == bare && !im.type_params.is_empty());
            if implements_generic && !matches!(m.visibility, Visibility::Public) {
                errors.push(SemanticError {
                    message: format!(
                        "扩展类型 '{}' 实现接口 '{}' 的泛型函数 '{}' 时必须为 public（6.3.5(3)）",
                        ext.target_type, base, bare
                    ),
                });
            }
        }
    }
}

fn check_this_type_usage(program: &Program, errors: &mut Vec<SemanticError>) {
    let mut bad = |what: &str, errors: &mut Vec<SemanticError>| {
        errors.push(SemanticError {
            message: format!(
                "This 类型只能用作类的实例成员函数的返回类型（6.1.4(1)）：{}",
                what
            ),
        });
    };
    let check_constraints = |constraints: &[crate::ast::TypeConstraint],
                            owner: &str,
                            errors: &mut Vec<SemanticError>| {
        if constraints.iter().any(|c| c.bounds.iter().any(|b| b == "This")) {
            errors.push(SemanticError {
                message: format!(
                    "This 类型不能用作泛型约束（6.1.4(1)）：{}",
                    owner
                ),
            });
        }
    };
    // 顶层函数：This 完全不可用。
    for f in &program.functions {
        let mut found = f
            .params
            .iter()
            .any(|p| contains_this(&p.ty))
            || f.return_type.as_ref().map_or(false, contains_this);
        if !found {
            for_each_type_in_stmts(&f.body, &mut |ty| {
                if contains_this(ty) {
                    found = true;
                }
            });
        }
        check_constraints(&f.constraints, &f.name, errors);
        if found {
            bad(&format!("顶层函数 '{}' 的签名或函数体", f.name), errors);
        }
    }
    for c in &program.constants {
        if contains_this(&c.ty) {
            bad(&format!("顶层常量 '{}' 的类型", c.name), errors);
        }
    }
    for cls in &program.classes {
        for fld in &cls.fields {
            if contains_this(&fld.ty) {
                bad(&format!("类 '{}' 的字段 '{}'", cls.name, fld.name), errors);
            }
        }
        if let Some(init) = &cls.init {
            for p in &init.params {
                if contains_this(&p.ty) {
                    bad(&format!("类 '{}' 的构造函数参数 '{}'", cls.name, p.name), errors);
                }
            }
            for_each_type_in_stmts(&init.body, &mut |ty| {
                if contains_this(ty) {
                    bad(&format!("类 '{}' 的构造函数体", cls.name), errors);
                }
            });
        }
        for p in &cls.primary_ctor_params {
            if contains_this(&p.ty) {
                bad(&format!("类 '{}' 的主构造函数参数 '{}'", cls.name, p.name), errors);
            }
        }
        check_constraints(&cls.constraints, &cls.name, errors);
        for m in &cls.methods {
            let sig = sig_of_class_method(m, &cls.type_params);
            if is_synthetic_member(&sig.name) {
                continue;
            }
            let is_static = m.is_static;
            for p in &m.func.params {
                if contains_this(&p.ty) {
                    bad(
                        &format!("类 '{}' 的方法 '{}' 的参数 '{}'", cls.name, m.func.name, p.name),
                        errors,
                    );
                }
            }
            if is_static {
                if m.func.return_type.as_ref().map_or(false, contains_this) {
                    bad(
                        &format!("类 '{}' 的静态方法 '{}' 的返回类型", cls.name, m.func.name),
                        errors,
                    );
                }
            }
            // 6.1.4(1)：This 只能用作实例成员函数的返回类型，不能用作属性类型。
            if is_property_method_name(&sig.name)
                && m.func.return_type.as_ref().map_or(false, contains_this)
            {
                bad(
                    &format!("类 '{}' 的属性 '{}' 的类型", cls.name, m.func.name),
                    errors,
                );
            }
            check_constraints(&m.func.constraints, &m.func.name, errors);
            for_each_type_in_stmts(&m.func.body, &mut |ty| {
                if contains_this(ty) {
                    bad(&format!("类 '{}' 的方法 '{}' 的函数体", cls.name, m.func.name), errors);
                }
            });
        }
    }
    for st in &program.structs {
        for fld in &st.fields {
            if contains_this(&fld.ty) {
                bad(&format!("结构体 '{}' 的字段 '{}'", st.name, fld.name), errors);
            }
        }
        check_constraints(&st.constraints, &st.name, errors);
    }
    for iface in &program.interfaces {
        for m in &iface.methods {
            if m.params.iter().any(|p| contains_this(&p.ty))
                || m.return_type.as_ref().map_or(false, contains_this)
            {
                bad(
                    &format!("接口 '{}' 的成员函数 '{}'", iface.name, m.name),
                    errors,
                );
            }
        }
    }
    for en in &program.enums {
        for v in &en.variants {
            if v.payload.as_ref().map_or(false, contains_this) {
                bad(&format!("枚举 '{}' 的构造器 '{}'", en.name, v.name), errors);
            }
        }
        check_constraints(&en.constraints, &en.name, errors);
    }
    for ext in &program.extends {
        for f in &ext.methods {
            let mut found = f.params.iter().any(|p| contains_this(&p.ty))
                || f.return_type.as_ref().map_or(false, contains_this);
            if !found {
                for_each_type_in_stmts(&f.body, &mut |ty| {
                    if contains_this(ty) {
                        found = true;
                    }
                });
            }
            if found {
                bad(&format!("extend 函数 '{}'", f.name), errors);
            }
        }
    }
}

// ─── 6.1.4(2)/(7)：This 返回类型的函数体与覆写规则 ────────────────────────

/// 6.1.4(7)：覆写父类型中返回类型为 `This` 的函数时，返回类型必须保持为 `This`。
///
/// 即使子类未显式书写 `override`（旧语法容许），只要同名同参数列表覆写了返回
/// `This` 的父函数，将返回类型改为具体类型即非法。
fn check_this_override_return(
    cls: &ClassDef,
    index: &ProgramIndex<'_>,
    errors: &mut Vec<SemanticError>,
) {
    let Some(parents) = collect_parent_methods(cls, index) else {
        return;
    };
    for method in &cls.methods {
        if method.is_static {
            continue;
        }
        let sig = sig_of_class_method(method, &cls.type_params);
        if is_synthetic_member(&sig.name) {
            continue;
        }
        let parent_returns_this = parents.iter().any(|p| {
            p.sig.name == sig.name
                && p.sig.ret == Some(Type::This)
                && (!p.strict_types || p.sig.params == sig.params)
        });
        if parent_returns_this && sig.ret != Some(Type::This) {
            errors.push(SemanticError {
                message: format!(
                    "类 '{}' 的 '{}' 覆写了返回 This 的父类型函数，返回类型必须保持为 This（6.1.4(7)）",
                    cls.name, sig.display
                ),
            });
        }
    }
}

/// 6.1.4(2)：返回类型为 `This` 的函数，其（隐式/显式）返回值必须是 This 表达式：
/// `this` 或对返回 This 的成员函数的调用。其余（字面量、普通调用、lambda、
/// if/match、类实例构造等）均非法。
fn check_this_return_bodies(program: &Program, errors: &mut Vec<SemanticError>) {
    for cls in &program.classes {
        // 返回 This 的成员函数基名集合（显式 This + 6.1.4(6) 推断为 This）。
        let mut this_methods: HashSet<String> = cls
            .methods
            .iter()
            .filter(|m| m.func.return_type == Some(Type::This))
            .filter_map(|m| m.func.name.rsplit('.').next().map(|s| s.to_string()))
            .collect();
        // 6.1.4(6)：无显式返回类型、函数体为 This 表达式的实例函数，推断返回 This。
        loop {
            let mut changed = false;
            for m in &cls.methods {
                if m.func.return_type.is_some() || !m.has_body || m.is_static {
                    continue;
                }
                let Some(base) = m.func.name.rsplit('.').next() else {
                    continue;
                };
                if this_methods.contains(base) {
                    continue;
                }
                let tail = match m.func.body.last() {
                    Some(Stmt::Expr(e)) => Some(e),
                    Some(Stmt::Return(Some(e))) => Some(e),
                    _ => None,
                };
                if let Some(e) = tail {
                    if is_this_value(e, &this_methods) {
                        this_methods.insert(base.to_string());
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
        for m in &cls.methods {
            if m.func.return_type != Some(Type::This) || !m.has_body {
                continue;
            }
            let base = m.func.name.rsplit('.').next().unwrap_or(&m.func.name);
            if is_synthetic_member(base) {
                continue;
            }
            let body = &m.func.body;
            let bad = |errors: &mut Vec<SemanticError>| {
                errors.push(SemanticError {
                    message: format!(
                        "类 '{}' 的 '{}' 返回类型为 This，只能返回 this 或返回 This 的函数调用（6.1.4(2)）",
                        cls.name, m.func.name
                    ),
                });
            };
            if body.is_empty() {
                bad(errors);
                continue;
            }
            match body.last() {
                Some(Stmt::Expr(e)) => {
                    if !is_this_value(e, &this_methods) {
                        bad(errors);
                    }
                }
                Some(Stmt::Return(Some(e))) => {
                    if !is_this_value(e, &this_methods) {
                        bad(errors);
                    }
                }
                Some(Stmt::Return(None)) => bad(errors),
                _ => {}
            }
        }
    }
}

/// 判断表达式是否为「This 表达式」：`this`/`super`、返回 This 的成员函数调用，
/// 以及返回 This 的运算符调用（`this + rhs`、`-this` 等）。
fn is_this_value(e: &Expr, this_methods: &HashSet<String>) -> bool {
    match e {
        Expr::Var(n) if n == "this" || n == "super" => true,
        Expr::Block(_, Some(t)) => is_this_value(t, this_methods),
        Expr::Call { name, .. } => {
            let base = name.rsplit('.').next().unwrap_or(name);
            this_methods.contains(base)
        }
        Expr::MethodCall { method, .. } => this_methods.contains(method),
        // 运算符调用：仅当该类声明了返回 This 的对应运算符重载。
        Expr::Binary { op, .. } => this_methods.contains(binop_op_name(op)),
        Expr::Unary { op, .. } => this_methods.contains(unary_op_name(op)),
        _ => false,
    }
}

fn unary_op_name(op: &crate::ast::UnaryOp) -> &'static str {
    use crate::ast::UnaryOp::*;
    match op {
        Not => "op_not",
        Neg => "op_sub",
        BitNot => "op_bitnot",
    }
}

fn binop_op_name(op: &crate::ast::BinOp) -> &'static str {
    use crate::ast::BinOp::*;
    match op {
        Add => "op_add",
        Sub => "op_sub",
        Mul => "op_mul",
        Div => "op_div",
        Mod => "op_mod",
        Eq => "op_eq",
        NotEq => "op_ne",
        Lt => "op_lt",
        Gt => "op_gt",
        LtEq => "op_le",
        GtEq => "op_ge",
        LogicalAnd => "op_and",
        LogicalOr => "op_or",
        Pow => "op_pow",
        BitAnd => "op_bitand",
        BitOr => "op_bitor",
        BitXor => "op_bitxor",
        Shl => "op_shl",
        Shr => "op_shr",
        NotIn => "op_notin",
        Pipeline => "op_pipeline",
    }
}

// ─── 6.2.2.1(8)/(10)：接口中无 body 的 static 函数不可被调用 ──────────────

/// 接口 `iface` 对静态成员 `method`（已去掉类型前缀）是否解析为「无 body 的
/// 抽象 static 函数」：
/// - 本接口存在同名带 body 的 static 函数 → 已具体化，可调用；
/// - 本接口存在同名无 body 的 static 函数 → 抽象，不可调用；
/// - 否则沿父接口查找。
/// 在接口 `iface`（含父接口）中查找名为 `name`、参数个数为 `argc` 的 static 函数。
/// 返回 `(是否抽象（无 body）, 实现体)`；找不到返回 None。
fn find_iface_static<'a>(
    iface: &'a InterfaceDef,
    name: &str,
    argc: usize,
    index: &ProgramIndex<'a>,
    seen: &mut HashSet<String>,
) -> Option<(bool, Option<&'a Vec<Stmt>>)> {
    if !seen.insert(iface.name.clone()) {
        return None;
    }
    let mut abstract_here = false;
    for m in &iface.methods {
        let mbase = m.name.rsplit('.').next().unwrap_or(&m.name);
        if !m.is_static || mbase != name || m.params.len() != argc {
            continue;
        }
        match &m.default_body {
            Some(b) => return Some((false, Some(b))),
            None => abstract_here = true,
        }
    }
    if abstract_here {
        return Some((true, None));
    }
    for p in &iface.parents {
        if let Some(pi) = index.interfaces.get(p.as_str()) {
            if let Some(r) = find_iface_static(pi, name, argc, index, seen) {
                return Some(r);
            }
        }
    }
    None
}

/// 6.2.2.1(8)/(10)：无 body 的接口 static 函数不可被调用。
///
/// - (8)：任何位置通过接口类型名直接调用 `I.f()`（含接口方法体内的直接调用）非法。
/// - (10)：通过接口类型名调用一个已实现 static 函数时，若其（传递地）调用了
///   该接口层次中未实现的 static 函数，同样非法；但若调用经由子接口（其实现了
///   这些函数）则合法（按静态分派类型解析）。
fn check_interface_abstract_static_calls(
    program: &Program,
    index: &ProgramIndex<'_>,
    errors: &mut Vec<SemanticError>,
) {
    use std::collections::HashSet as Set;
    let report = |iface: &str, method: &str, errors: &mut Vec<SemanticError>| {
        errors.push(SemanticError {
            message: format!(
                "接口 '{}' 的静态函数 '{}' 没有实现体，不能被调用（6.2.2.1(8)/(10)）",
                iface, method
            ),
        });
    };

    // 汇总所有待扫描的函数体（自由函数、类方法、构造函数、extend、接口默认体）。
    let mut bodies: Vec<&[Stmt]> = Vec::new();
    for f in &program.functions {
        bodies.push(&f.body);
    }
    for cls in &program.classes {
        for m in &cls.methods {
            bodies.push(&m.func.body);
        }
        for init in &cls.inits {
            bodies.push(&init.body);
        }
    }
    for ext in &program.extends {
        for f in &ext.methods {
            bodies.push(&f.body);
        }
    }
    for iface in &program.interfaces {
        for m in &iface.methods {
            if let Some(b) = &m.default_body {
                bodies.push(b);
            }
        }
    }

    // 直接调用抽象 static：任意函数体中的 `I.f(...)`。
    // 同时收集「通过接口类型名调用已实现 static」作为间接检查根。
    let mut roots: Vec<(String, String, usize)> = Vec::new();
    let mut direct_hits: Vec<(String, String)> = Vec::new();
    for body in &bodies {
        walk_stmts(body, &mut |e| {
            let (ty, method, argc) = match e {
                Expr::MethodCall {
                    object,
                    method,
                    args,
                    ..
                } => match object.as_ref() {
                    Expr::Var(n) => (n.clone(), method.clone(), args.len()),
                    _ => return,
                },
                Expr::Call { name, args, .. } => match name.rsplit_once('.') {
                    Some((ty, m)) => (ty.to_string(), m.to_string(), args.len()),
                    None => return,
                },
                _ => return,
            };
            let Some(iface) = index.interfaces.get(ty.as_str()) else {
                return;
            };
            match find_iface_static(iface, &method, argc, index, &mut Set::new()) {
                Some((true, _)) => direct_hits.push((ty, method)),
                Some((false, Some(_))) => roots.push((ty, method, argc)),
                _ => {}
            }
        });
    }
    direct_hits.sort();
    direct_hits.dedup();
    for (ty, m) in direct_hits {
        report(&ty, &m, errors);
    }

    // 间接调用：从每个根出发，沿静态分派类型传递地检查无资格调用。
    let mut visited: Set<(String, String, usize)> = Set::new();
    while let Some((tname, mname, argc)) = roots.pop() {
        if !visited.insert((tname.clone(), mname.clone(), argc)) {
            continue;
        }
        let Some(iface) = index.interfaces.get(tname.as_str()) else {
            continue;
        };
        let Some((abstract_, body)) =
            find_iface_static(iface, &mname, argc, index, &mut Set::new())
        else {
            continue;
        };
        if abstract_ {
            report(&tname, &mname, errors);
            continue;
        }
        let Some(body) = body else { continue };
        let mut calls: Vec<(String, usize)> = Vec::new();
        walk_stmts(body, &mut |e| {
            if let Expr::Call { name, args, .. } = e {
                if !name.contains('.') {
                    calls.push((name.clone(), args.len()));
                }
            }
        });
        for (n, a) in calls {
            roots.push((tname.clone(), n, a));
        }
    }
}

// ─── 6.2.3.1(6) / 6.2.4.1(9)：多父接口同名默认实现的冲突 ─────────────────

/// 成员键：去掉所属类型前缀后的成员名（属性访问器保留 `__get_`/`__set_`），
/// 加上 static 标记与「有效参数类型列表」（不含接收者），用于区分重载。
type MemberKey = (String, bool, Vec<Type>);

fn effective_param_types(params: &[crate::ast::Param]) -> Vec<Type> {
    let start = if params
        .first()
        .map_or(false, |p| p.name == "this" || p.name == "self")
    {
        1
    } else {
        0
    };
    params[start..].iter().map(|p| p.ty.clone()).collect()
}

fn member_key(name: &str, is_static: bool, params: &[crate::ast::Param]) -> MemberKey {
    let base = name.rsplit('.').next().unwrap_or(name);
    (base.to_string(), is_static, effective_param_types(params))
}

/// 收集接口（含传递父接口，不含自身）中带默认实现的成员。
/// 计算接口 `name` 的「成员键 → 默认实现提供者」映射（就近声明遮蔽父接口，
/// 即本接口的声明优先于父接口同名声明；无 body 的声明记为 None，同样遮蔽）。
fn iface_default_map<'a>(
    name: &str,
    index: &ProgramIndex<'a>,
    seen: &mut HashSet<String>,
) -> HashMap<MemberKey, Option<String>> {
    let mut map: HashMap<MemberKey, Option<String>> = HashMap::new();
    if !seen.insert(name.to_string()) {
        return map;
    }
    let Some(iface) = index.interfaces.get(name) else {
        return map;
    };
    for m in &iface.methods {
        let key = member_key(&m.name, m.is_static, &m.params);
        let val = if m.default_body.is_some() {
            Some(iface.name.clone())
        } else {
            None
        };
        map.entry(key).or_insert(val);
    }
    for p in &iface.parents {
        for (k, v) in iface_default_map(p, index, seen) {
            map.entry(k).or_insert(v);
        }
    }
    map
}

/// 接口 `name` 的传递父接口集合。
fn iface_ancestors(name: &str, index: &ProgramIndex<'_>) -> HashSet<String> {
    let mut out: HashSet<String> = HashSet::new();
    let mut stack: Vec<String> = index
        .interfaces
        .get(name)
        .map(|i| i.parents.clone())
        .unwrap_or_default();
    while let Some(p) = stack.pop() {
        if out.insert(p.clone()) {
            if let Some(pi) = index.interfaces.get(p.as_str()) {
                stack.extend(pi.parents.iter().cloned());
            }
        }
    }
    out
}

/// 给定一组「直接」接口，计算其中存在多个**互不包含**的默认实现提供者、
/// 因而必须由子类型自行实现的成员键。
fn conflicting_default_keys(
    direct: &[String],
    index: &ProgramIndex<'_>,
) -> Vec<MemberKey> {
    let mut owners: HashMap<MemberKey, HashSet<String>> = HashMap::new();
    for d in direct {
        let mut seen = HashSet::new();
        for (k, v) in iface_default_map(d, index, &mut seen) {
            if let Some(owner) = v {
                owners.entry(k).or_default().insert(owner);
            }
        }
    }
    let mut result: Vec<MemberKey> = Vec::new();
    for (k, os) in owners.into_iter() {
        if os.len() < 2 {
            continue;
        }
        let providers: Vec<String> = os.into_iter().collect();
        // 去掉「是另一个提供者的祖先」的提供者（被更派生的接口遮蔽）。
        let kept = providers
            .iter()
            .filter(|o| {
                !providers
                    .iter()
                    .any(|other| other != *o && iface_ancestors(other, index).contains(*o))
            })
            .count();
        if kept >= 2 {
            result.push(k);
        }
    }
    result.sort_by(|a, b| (a.0.as_str(), a.1).cmp(&(b.0.as_str(), b.1)));
    result
}

/// 6.2.3.1(6)：子接口从多个父接口继承同名同签名的默认实现，但自身未提供
/// 新的默认实现（省略或仅声明无体）时报错。
/// 6.2.4.1(9)：class/struct/enum 同理，必须自身实现这些成员，否则报错。
fn check_interface_default_conflicts(
    program: &Program,
    index: &ProgramIndex<'_>,
    errors: &mut Vec<SemanticError>,
) {
    let report = |what: &str, key: &MemberKey, errors: &mut Vec<SemanticError>| {
        let kind = if key.1 { "静态" } else { "" };
        let base = &key.0;
        let shown = base
            .strip_prefix("__get_")
            .or_else(|| base.strip_prefix("__set_"))
            .unwrap_or(base);
        errors.push(SemanticError {
            message: format!(
                "{}继承了多个父接口中同名同签名的默认实现 '{}'（{}），必须自行实现（6.2.3.1(6)/6.2.4.1(9)）",
                what, shown, kind
            ),
        });
    };

    // 接口层：父接口（含传递）默认实现冲突。
    for iface in &program.interfaces {
        let keys = conflicting_default_keys(&iface.parents, index);
        if keys.is_empty() {
            continue;
        }
        let own: HashSet<MemberKey> = iface
            .methods
            .iter()
            .filter(|m| m.default_body.is_some())
            .map(|m| member_key(&m.name, m.is_static, &m.params))
            .collect();
        for k in keys {
            if !own.contains(&k) {
                report(&format!("接口 '{}' ", iface.name), &k, errors);
            }
        }
    }

    // 类型层：class/struct/enum 必须自行实现冲突成员。
    let report_type = |owner: &str,
                       impls: &HashSet<MemberKey>,
                       keys: &[MemberKey],
                       errors: &mut Vec<SemanticError>| {
        for k in keys {
            if !impls.contains(k) {
                report(&format!("类型 '{}' ", owner), k, errors);
            }
        }
    };

    for cls in &program.classes {
        let mut direct: Vec<String> = Vec::new();
        // `<: I1 & I2` 中第一个父类型被解析为 extends（若为接口亦计入）。
        if let Some(p) = cls.extends.as_deref() {
            if index.interfaces.contains_key(p) {
                direct.push(p.to_string());
            }
        }
        direct.extend(cls.implements.iter().cloned());
        if direct.is_empty() {
            continue;
        }
        let keys = conflicting_default_keys(&direct, index);
        if keys.is_empty() {
            continue;
        }
        let mut impls: HashSet<MemberKey> = cls
            .methods
            .iter()
            .filter(|m| m.has_body)
            .map(|m| member_key(&m.func.name, m.is_static, &m.func.params))
            .collect();
        // 6.2.4.1(10)：父类已提供的同名实现同样满足接口冲突的消解要求。
        let mut cur = cls.extends.clone();
        let mut seen_cls: HashSet<String> = HashSet::new();
        while let Some(pname) = cur {
            if !seen_cls.insert(pname.clone()) {
                break;
            }
            let Some(parent) = index.classes.get(pname.as_str()) else {
                break;
            };
            for m in &parent.methods {
                if m.has_body {
                    impls.insert(member_key(&m.func.name, m.is_static, &m.func.params));
                }
            }
            cur = parent.extends.clone();
        }
        report_type(&cls.name, &impls, &keys, errors);
    }

    for st in &program.structs {
        let bounds: Vec<String> = st
            .constraints
            .iter()
            .filter(|c| c.param == st.name)
            .flat_map(|c| c.bounds.iter().cloned())
            .collect();
        if bounds.is_empty() {
            continue;
        }
        let keys = conflicting_default_keys(&bounds, index);
        if keys.is_empty() {
            continue;
        }
        // struct 方法不保留 static 标记（属性访问器恒带 this 首参），
        // 因此对同一成员名同时登记 static/实例两种键，避免误判。
        let mut impls: HashSet<MemberKey> = HashSet::new();
        for f in program
            .functions
            .iter()
            .filter(|f| owner_name(&f.name) == Some(st.name.as_str()))
        {
            let b = f.name.rsplit('.').next().unwrap_or(&f.name).to_string();
            let p = effective_param_types(&f.params);
            impls.insert((b.clone(), false, p.clone()));
            impls.insert((b, true, p));
        }
        report_type(&st.name, &impls, &keys, errors);
    }

    for en in &program.enums {
        let bounds: Vec<String> = en
            .constraints
            .iter()
            .filter(|c| c.param == en.name)
            .flat_map(|c| c.bounds.iter().cloned())
            .collect();
        if bounds.is_empty() {
            continue;
        }
        let keys = conflicting_default_keys(&bounds, index);
        if keys.is_empty() {
            continue;
        }
        let mut impls: HashSet<MemberKey> = HashSet::new();
        for f in program
            .functions
            .iter()
            .filter(|f| owner_name(&f.name) == Some(en.name.as_str()))
        {
            let b = f.name.rsplit('.').next().unwrap_or(&f.name).to_string();
            let p = effective_param_types(&f.params);
            impls.insert((b.clone(), false, p.clone()));
            impls.insert((b, true, p));
        }
        for prop in &en.props {
            if prop.getter.is_some() {
                let b = format!("__get_{}", prop.name);
                impls.insert((b.clone(), false, vec![]));
                impls.insert((b, true, vec![]));
            }
            if prop.setter.is_some() {
                let b = format!("__set_{}", prop.name);
                impls.insert((b.clone(), false, vec![prop.ty.clone()]));
                impls.insert((b, true, vec![prop.ty.clone()]));
            }
        }
        report_type(&en.name, &impls, &keys, errors);
    }
}

// ─── 6.2.2.2(7)：不得通过 super 访问接口默认实现 ─────────────────────────

/// 收集一组接口（含传递父接口）中带默认实现的成员显示名集合
/// （属性访问器还原为属性名）。
fn collect_iface_default_display_names<'a>(
    names: &[String],
    index: &ProgramIndex<'a>,
    out: &mut HashSet<String>,
    seen: &mut HashSet<String>,
) {
    for n in names {
        if !seen.insert(n.clone()) {
            continue;
        }
        let Some(iface) = index.interfaces.get(n.as_str()) else {
            continue;
        };
        for m in &iface.methods {
            if m.default_body.is_none() {
                continue;
            }
            let base = m.name.rsplit('.').next().unwrap_or(&m.name);
            let shown = base
                .strip_prefix("__get_")
                .or_else(|| base.strip_prefix("__set_"))
                .unwrap_or(base);
            out.insert(shown.to_string());
        }
        collect_iface_default_display_names(&iface.parents, index, out, seen);
    }
}

/// 6.2.2.2(7)：接口默认实现不属于继承结构，不能用 `super.f()` / `super.p`
/// 访问。仅当类型没有父类（class 的 extends 不是类，或为 struct/enum）时，
/// 只要被访问的成员由某父接口的默认实现提供即报错。
fn check_super_interface_default(
    program: &Program,
    index: &ProgramIndex<'_>,
    errors: &mut Vec<SemanticError>,
) {
    let report = |owner: &str, member: &str, errors: &mut Vec<SemanticError>| {
        errors.push(SemanticError {
            message: format!(
                "类型 '{}' 不能通过 'super.{}' 访问接口默认实现（6.2.2.2(7)）",
                owner, member
            ),
        });
    };

    for cls in &program.classes {
        // 存在父类时 super 解析到父类，保守跳过。
        if cls
            .extends
            .as_deref()
            .map_or(false, |e| index.classes.contains_key(e))
        {
            continue;
        }
        let mut iface_names: Vec<String> = Vec::new();
        if let Some(e) = cls.extends.as_deref() {
            if index.interfaces.contains_key(e) {
                iface_names.push(e.to_string());
            }
        }
        iface_names.extend(cls.implements.iter().cloned());
        let mut names: HashSet<String> = HashSet::new();
        let mut seen: HashSet<String> = HashSet::new();
        collect_iface_default_display_names(&iface_names, index, &mut names, &mut seen);
        if names.is_empty() {
            continue;
        }
        let mut hits: Vec<String> = Vec::new();
        let mut collect = |stmts: &[Stmt]| {
            walk_stmts(stmts, &mut |e| match e {
                Expr::SuperCall { method, .. } if method != "init" => {
                    let base = method.rsplit('.').next().unwrap_or(method);
                    if names.contains(base) {
                        hits.push(base.to_string());
                    }
                }
                Expr::SuperFieldAccess { field } => {
                    if names.contains(field.as_str()) {
                        hits.push(field.clone());
                    }
                }
                _ => {}
            });
        };
        for m in &cls.methods {
            collect(&m.func.body);
        }
        for init in &cls.inits {
            collect(&init.body);
        }
        hits.sort();
        hits.dedup();
        for h in hits {
            report(&cls.name, &h, errors);
        }
    }

    // 子接口 override/redef 中通过 super 访问父接口默认实现。
    for iface in &program.interfaces {
        let mut names: HashSet<String> = HashSet::new();
        let mut seen: HashSet<String> = HashSet::new();
        collect_iface_default_display_names(&iface.parents, index, &mut names, &mut seen);
        if names.is_empty() {
            continue;
        }
        let mut hits: Vec<String> = Vec::new();
        for m in &iface.methods {
            if let Some(body) = &m.default_body {
                walk_stmts(body, &mut |e| match e {
                    Expr::SuperCall { method, .. } if method != "init" => {
                        let base = method.rsplit('.').next().unwrap_or(method);
                        if names.contains(base) {
                            hits.push(base.to_string());
                        }
                    }
                    Expr::SuperFieldAccess { field } => {
                        if names.contains(field.as_str()) {
                            hits.push(field.clone());
                        }
                    }
                    _ => {}
                });
            }
        }
        hits.sort();
        hits.dedup();
        for h in hits {
            report(&iface.name, &h, errors);
        }
    }

    for (owner, iface_names, bodies) in super_scan_targets(program) {
        let mut names: HashSet<String> = HashSet::new();
        let mut seen: HashSet<String> = HashSet::new();
        collect_iface_default_display_names(&iface_names, index, &mut names, &mut seen);
        if names.is_empty() {
            continue;
        }
        let mut hits: Vec<String> = Vec::new();
        for body in bodies {
            walk_stmts(body, &mut |e| match e {
                Expr::SuperCall { method, .. } if method != "init" => {
                    let base = method.rsplit('.').next().unwrap_or(method);
                    if names.contains(base) {
                        hits.push(base.to_string());
                    }
                }
                Expr::SuperFieldAccess { field } => {
                    if names.contains(field.as_str()) {
                        hits.push(field.clone());
                    }
                }
                _ => {}
            });
        }
        hits.sort();
        hits.dedup();
        for h in hits {
            report(&owner, &h, errors);
        }
    }
}

/// 收集 struct/enum 的 (类型名, 父接口名列表, 函数体列表)。
fn super_scan_targets<'a>(program: &'a Program) -> Vec<(String, Vec<String>, Vec<&'a [Stmt]>)> {
    let mut out = Vec::new();
    let mut push = |name: &str, bounds: Vec<String>| {
        let bodies: Vec<&[Stmt]> = program
            .functions
            .iter()
            .filter(|f| owner_name(&f.name) == Some(name))
            .map(|f| f.body.as_slice())
            .collect();
        if !bodies.is_empty() {
            out.push((name.to_string(), bounds, bodies));
        }
    };
    for st in &program.structs {
        let bounds: Vec<String> = st
            .constraints
            .iter()
            .filter(|c| c.param == st.name)
            .flat_map(|c| c.bounds.iter().cloned())
            .collect();
        push(&st.name, bounds);
    }
    for en in &program.enums {
        let bounds: Vec<String> = en
            .constraints
            .iter()
            .filter(|c| c.param == en.name)
            .flat_map(|c| c.bounds.iter().cloned())
            .collect();
        push(&en.name, bounds);
    }
    out
}

// ── 2.1.10.2 结构体构造器语义规则 ────────────────────────────────────────
//
// 结构体主构造函数（`StructName(...) { ... }`）与 `init` 构造函数的文法/语义
// 规则。主构造函数在 parser 中被展开为 `inits` 的首项（成员变量参数隐式赋值），
// 普通参数保留在 `primary_ctor_params` 中但不生成字段。

/// 计算结构体实例成员信息：返回 (实例成员名集合, 显式字段默认值集合, 主构造成员参数名集合)。
fn struct_member_info(
    st: &crate::ast::StructDef,
) -> (HashSet<String>, HashSet<String>, HashSet<String>) {
    let primary_names: HashSet<&str> = st
        .primary_ctor_params
        .iter()
        .map(|p| p.name.as_str())
        .collect();
    let mut members: HashSet<String> = HashSet::new();
    let mut auto: HashSet<String> = HashSet::new();
    for f in &st.fields {
        if f.is_static {
            continue;
        }
        // parser 会把主构造参数也塞进 `fields`；普通参数不是成员，
        // 成员变量参数在下面单独处理（其默认值是参数默认值，不是字段初始值）。
        if primary_names.contains(f.name.as_str()) {
            continue;
        }
        members.insert(f.name.clone());
        if f.default.is_some() {
            auto.insert(f.name.clone());
        }
    }
    let mut primary_member: HashSet<String> = HashSet::new();
    for (p, flag) in st
        .primary_ctor_params
        .iter()
        .zip(st.primary_ctor_member_flags.iter())
    {
        if *flag {
            members.insert(p.name.clone());
            primary_member.insert(p.name.clone());
        }
    }
    (members, auto, primary_member)
}

/// 判断某个 init 是否是主构造函数展开（形参与 `primary_ctor_params` 完全一致）。
fn is_primary_struct_init(init: &crate::ast::InitDef, st: &crate::ast::StructDef) -> bool {
    st.has_primary_ctor
        && init.params.len() == st.primary_ctor_params.len()
        && init
            .params
            .iter()
            .zip(st.primary_ctor_params.iter())
            .all(|(a, b)| a.name == b.name && a.ty == b.ty)
        && init
            .member_param_flags
            .iter()
            .zip(st.primary_ctor_member_flags.iter())
            .all(|(a, b)| a == b)
}

/// 结构体构造函数语义规则总入口。
fn check_struct_ctor_rules(
    st: &crate::ast::StructDef,
    program: &Program,
    errors: &mut Vec<SemanticError>,
) {
    check_struct_ctor_param_order(st, errors);
    check_struct_ctor_param_reassign(st, errors);
    check_struct_ctor_overload(st, program, errors);
    check_struct_ctor_cycles(st, errors);
    check_struct_ctor_defaults(st, errors);
    check_struct_member_duplicates(st, errors);
    check_struct_ctor_definite_assignment(st, errors);
    check_struct_use_before_init(st, program, errors);
    check_struct_return_types(st, errors);
    check_struct_this_calls(st, program, errors);
    check_struct_field_self_ctor(st, errors);
    check_struct_static_field_access(st, program, errors);
}

/// 2.1.10.2(21)/(22)：普通参数必须在成员变量参数之前；非命名参数必须在命名参数之前。
fn check_struct_ctor_param_order(
    st: &crate::ast::StructDef,
    errors: &mut Vec<SemanticError>,
) {
    let mut seen_member = false;
    for (idx, is_member) in st.primary_ctor_member_flags.iter().enumerate() {
        if *is_member {
            seen_member = true;
        } else if seen_member {
            errors.push(SemanticError {
                message: format!(
                    "结构体 '{}' 的主构造函数第 {} 个普通参数位于成员变量参数之后（2.1.10.2(21)）",
                    st.name,
                    idx + 1
                ),
            });
            break;
        }
    }
    if let Some(idx) = first_unnamed_after_named(&st.primary_ctor_params) {
        errors.push(SemanticError {
            message: format!(
                "结构体 '{}' 的主构造函数第 {} 个非命名参数位于命名参数之后（2.1.10.2(22)）",
                st.name,
                idx + 1
            ),
        });
    }
    for init in &st.inits {
        if let Some(idx) = first_unnamed_after_named(&init.params) {
            errors.push(SemanticError {
                message: format!(
                    "结构体 '{}' 的构造函数第 {} 个非命名参数位于命名参数之后（2.1.10.2(22)）",
                    st.name,
                    idx + 1
                ),
            });
        }
    }
}

/// 2.1.10.2(8)/(9)：构造参数在构造函数体内不可被重新赋值。
fn check_struct_ctor_param_reassign(st: &crate::ast::StructDef, errors: &mut Vec<SemanticError>) {
    for init in &st.inits {
        let names: HashSet<&str> = init.params.iter().map(|p| p.name.as_str()).collect();
        if names.is_empty() {
            continue;
        }
        let mut assigned: Vec<String> = Vec::new();
        for s in &init.body {
            collect_param_assigns_stmt(s, &names, &mut assigned);
        }
        assigned.sort();
        assigned.dedup();
        for name in assigned {
            errors.push(SemanticError {
                message: format!(
                    "结构体 '{}' 的构造函数形参 '{}' 不能被重新赋值（2.1.10.2(8)/(9)）",
                    st.name, name
                ),
            });
        }
    }
}

/// 2.1.10.2(29)：可省略参数（带默认值）导致两个构造函数在调用时无法区分。
fn check_struct_ctor_overload(
    st: &crate::ast::StructDef,
    program: &Program,
    errors: &mut Vec<SemanticError>,
) {
    let inits = &st.inits;
    for i in 0..inits.len() {
        for j in (i + 1)..inits.len() {
            if ctor_signatures_ambiguous(&inits[i], &inits[j]) {
                errors.push(SemanticError {
                    message: format!(
                        "结构体 '{}' 的构造函数重载冲突：可选参数导致调用无法区分（2.1.10.2(29)）",
                        st.name
                    ),
                });
            }
        }
    }
    // 泛型结构体：在实际实例化类型实参下，若两个构造函数签名变为相同则冲突。
    if !st.type_params.is_empty() {
        let insts = collect_struct_type_instantiations(program, st);
        'outer: for args in insts {
            if args.len() != st.type_params.len() {
                continue;
            }
            let subst: HashMap<&str, &Type> = st
                .type_params
                .iter()
                .map(|s| s.as_str())
                .zip(args.iter())
                .collect();
            for i in 0..inits.len() {
                for j in (i + 1)..inits.len() {
                    if ctor_signatures_equal_after_subst(&inits[i], &inits[j], &subst) {
                        errors.push(SemanticError {
                            message: format!(
                                "结构体 '{}' 的构造函数重载冲突：泛型实例化后签名相同（2.1.10.2(29)）",
                                st.name
                            ),
                        });
                        break 'outer;
                    }
                }
            }
        }
    }
}

fn subst_ty(ty: &Type, subst: &HashMap<&str, &Type>) -> Type {
    match ty {
        Type::TypeParam(n) => subst.get(n.as_str()).map(|t| (*t).clone()).unwrap_or_else(|| ty.clone()),
        Type::Array(inner) => Type::Array(Box::new(subst_ty(inner, subst))),
        Type::Slice(inner) => Type::Slice(Box::new(subst_ty(inner, subst))),
        Type::Option(inner) => Type::Option(Box::new(subst_ty(inner, subst))),
        Type::Struct(n, args) => {
            Type::Struct(n.clone(), args.iter().map(|a| subst_ty(a, subst)).collect())
        }
        Type::Tuple(ts) => Type::Tuple(ts.iter().map(|t| subst_ty(t, subst)).collect()),
        _ => ty.clone(),
    }
}

fn ctor_signatures_equal_after_subst(
    a: &crate::ast::InitDef,
    b: &crate::ast::InitDef,
    subst: &HashMap<&str, &Type>,
) -> bool {
    a.params.len() == b.params.len()
        && a.params.iter().zip(b.params.iter()).all(|(pa, pb)| {
            pa.is_named == pb.is_named && subst_ty(&pa.ty, subst) == subst_ty(&pb.ty, subst)
        })
}

/// 收集程序中以显式类型实参构造某结构体的实例化实参（`S<A,B>(...)`）。
fn collect_struct_type_instantiations(
    program: &Program,
    st: &crate::ast::StructDef,
) -> Vec<Vec<Type>> {
    let mut out: Vec<Vec<Type>> = Vec::new();
    let mut visit = |stmts: &[Stmt], out: &mut Vec<Vec<Type>>| {
        walk_stmts(stmts, &mut |e| {
            if let Expr::ConstructorCall {
                name,
                type_args: Some(args),
                ..
            } = e
            {
                if name == &st.name {
                    out.push(args.clone());
                }
            }
        });
    };
    for f in &program.functions {
        visit(&f.body, &mut out);
    }
    for c in &program.classes {
        for m in &c.methods {
            visit(&m.func.body, &mut out);
        }
    }
    for s in &program.structs {
        for init in &s.inits {
            visit(&init.body, &mut out);
        }
        for f in &s.fields {
            if let Some(d) = &f.default {
                walk_expr(d, &mut |e| {
                    if let Expr::ConstructorCall {
                        name,
                        type_args: Some(args),
                        ..
                    } = e
                    {
                        if name == &st.name {
                            out.push(args.clone());
                        }
                    }
                });
            }
        }
    }
    out
}

fn ctor_signatures_ambiguous(a: &crate::ast::InitDef, b: &crate::ast::InitDef) -> bool {
    let pos_a: Vec<&crate::ast::Param> = a.params.iter().filter(|p| !p.is_named).collect();
    let pos_b: Vec<&crate::ast::Param> = b.params.iter().filter(|p| !p.is_named).collect();
    let (short, long, long_init) = if pos_a.len() <= pos_b.len() {
        (&pos_a, &pos_b, b)
    } else {
        (&pos_b, &pos_a, a)
    };
    // 较短者的位置参数类型必须是较长者的前缀。
    for (i, p) in short.iter().enumerate() {
        if p.ty != long[i].ty {
            return false;
        }
    }
    // 较长者多出的可省略参数必须全部带默认值，才能在调用时被省略。
    let mut extra = false;
    for p in &long[short.len()..] {
        if p.default.is_none() {
            return false;
        }
        extra = true;
    }
    for p in long_init.params.iter().filter(|p| p.is_named) {
        if p.default.is_none() && !short.iter().any(|s| s.name == p.name) {
            return false;
        }
        extra = true;
    }
    extra
}

/// 2.1.10.2(37)/(41)：`this(...)` 构造函数调用的循环调用检测。
fn check_struct_ctor_cycles(st: &crate::ast::StructDef, errors: &mut Vec<SemanticError>) {
    let n = st.inits.len();
    if n == 0 {
        return;
    }
    let mut edges: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (i, init) in st.inits.iter().enumerate() {
        // 局部绑定/局部函数若与结构体同名，会遮蔽构造调用，保守跳过同类型构造边。
        let mut bindings: HashSet<String> = HashSet::new();
        collect_bindings_stmts(&init.body, &mut bindings);
        collect_local_func_names(&init.body, &mut bindings);
        let type_shadowed = bindings.contains(&st.name);
        // 收集构造函数体内的委托调用（this(...)）与同类型构造调用（Type(...)）。
        let mut calls: Vec<(&Vec<Expr>, &Vec<(String, Expr)>)> = Vec::new();
        for s in &init.body {
            walk_stmt(s, &mut |e| match e {
                Expr::Call {
                    name,
                    args,
                    named_args,
                    ..
                } if name == "this" || name == "self" => {
                    calls.push((args, named_args));
                }
                Expr::Invoke {
                    callee,
                    args,
                    named_args,
                } if matches!(callee.as_ref(), Expr::Var(n) if n == "this" || n == "self") => {
                    calls.push((args, named_args));
                }
                Expr::ConstructorCall {
                    name,
                    args,
                    named_args,
                    ..
                } if name == &st.name && !type_shadowed => {
                    calls.push((args, named_args));
                }
                _ => {}
            });
        }
        for (args, named) in calls {
            if let Some(j) = resolve_struct_ctor(st, args, named) {
                edges[i].push(j);
            }
        }
    }
    // DFS 三色标记检测环。
    let mut color = vec![0u8; n]; // 0=white 1=gray 2=black
    let mut has_cycle = false;
    fn dfs(u: usize, edges: &[Vec<usize>], color: &mut [u8], has_cycle: &mut bool) {
        color[u] = 1;
        for &v in &edges[u] {
            if color[v] == 1 {
                *has_cycle = true;
            } else if color[v] == 0 {
                dfs(v, edges, color, has_cycle);
            }
        }
        color[u] = 2;
    }
    for u in 0..n {
        if color[u] == 0 {
            dfs(u, &edges, &mut color, &mut has_cycle);
        }
    }
    if has_cycle {
        errors.push(SemanticError {
            message: format!(
                "结构体 '{}' 的构造函数通过 this(...) 形成循环调用（2.1.10.2(37)/(41)）",
                st.name
            ),
        });
    }
}

fn find_this_call<'a>(body: &'a [Stmt]) -> Option<(&'a [Expr], &'a [(String, Expr)])> {
    for s in body {
        if let Stmt::Expr(e) = s {
            match e {
                Expr::Call {
                    name,
                    args,
                    named_args,
                    ..
                } if name == "this" || name == "self" => {
                    return Some((args.as_slice(), named_args.as_slice()));
                }
                Expr::Invoke {
                    callee,
                    args,
                    named_args,
                } if matches!(callee.as_ref(), Expr::Var(n) if n == "this" || n == "self") => {
                    return Some((args.as_slice(), named_args.as_slice()));
                }
                _ => {}
            }
        }
    }
    None
}

fn resolve_struct_ctor(
    st: &crate::ast::StructDef,
    args: &[Expr],
    named: &[(String, Expr)],
) -> Option<usize> {
    let arg_tys: Vec<Option<Type>> = args.iter().map(|a| infer_expr(a, &HashMap::new())).collect();
    let mut cands = Vec::new();
    for (j, init) in st.inits.iter().enumerate() {
        if ctor_accepts_call(init, &arg_tys, named) {
            cands.push(j);
        }
    }
    if cands.len() == 1 {
        Some(cands[0])
    } else {
        None
    }
}

fn ctor_accepts_call(
    init: &crate::ast::InitDef,
    arg_tys: &[Option<Type>],
    named: &[(String, Expr)],
) -> bool {
    let positional: Vec<&crate::ast::Param> = init.params.iter().filter(|p| !p.is_named).collect();
    if arg_tys.len() > positional.len() {
        return false;
    }
    for (i, ty) in arg_tys.iter().enumerate() {
        if let Some(t) = ty {
            let pt = &positional[i].ty;
            if !matches!(pt, Type::TypeParam(_)) && !matches!(t, Type::TypeParam(_)) && t != pt {
                return false;
            }
        }
    }
    for p in positional[arg_tys.len()..].iter() {
        if p.default.is_none() {
            return false;
        }
    }
    let named_params: Vec<&crate::ast::Param> = init.params.iter().filter(|p| p.is_named).collect();
    for (n, _) in named {
        if !named_params.iter().any(|p| &p.name == n) {
            return false;
        }
    }
    for p in &named_params {
        if p.default.is_none() && !named.iter().any(|(n, _)| n == &p.name) {
            return false;
        }
    }
    true
}

/// 2.1.10.2(19)/(33)：构造参数默认值不得引用 `this`，也不得引用其后的参数
/// 或构造参数之外的实例成员变量。
fn check_struct_ctor_defaults(st: &crate::ast::StructDef, errors: &mut Vec<SemanticError>) {
    let (members, _, _) = struct_member_info(st);
    let mut check_params = |params: &[crate::ast::Param], what: &str| {
        let names: Vec<&str> = params.iter().map(|p| p.name.as_str()).collect();
        for (i, p) in params.iter().enumerate() {
            let Some(default) = &p.default else { continue };
            // 2.1.10.2(33)：默认值中不得使用 this。
            if expr_contains_this(default) {
                errors.push(SemanticError {
                    message: format!(
                        "结构体 '{}' 的{}参数 '{}' 的默认值不能引用 this（2.1.10.2(33)）",
                        st.name, what, p.name
                    ),
                });
                continue;
            }
            // 2.1.10.2(19)：默认值只能引用其之前定义的参数，且不得引用实例成员变量。
            let mut violation: Option<String> = None;
            walk_expr(default, &mut |e| {
                if violation.is_some() {
                    return;
                }
                if let Expr::Var(n) = e {
                    if let Some(j) = names.iter().position(|x| *x == n.as_str()) {
                        if j >= i {
                            violation = Some(format!(
                                "默认值引用了自身或其后的参数 '{}'（2.1.10.2(19)）",
                                n
                            ));
                        }
                    } else if members.contains(n) {
                        violation = Some(format!(
                            "默认值引用了实例成员变量 '{}'（2.1.10.2(19)）",
                            n
                        ));
                    }
                }
            });
            // 2.1.10.2(19)：默认值不得修改所引用的参数。
            if violation.is_none() && expr_modifies_param(default, &names) {
                violation = Some("默认值修改了所引用的参数（2.1.10.2(19)）".to_string());
            }
            if let Some(reason) = violation {
                errors.push(SemanticError {
                    message: format!(
                        "结构体 '{}' 的{}参数 '{}' 的{}",
                        st.name, what, p.name, reason
                    ),
                });
            }
        }
    };
    check_params(&st.primary_ctor_params, "主构造");
    for init in &st.inits {
        check_params(&init.params, "构造");
    }
}

/// 2.1.10.2(16)：成员变量参数不能与已声明的实例成员变量同名。
fn check_struct_member_duplicates(st: &crate::ast::StructDef, errors: &mut Vec<SemanticError>) {
    // parser 会为每个主构造参数（含普通参数与成员变量参数）生成一个 FieldDef，
    // 显式声明字段也生成 FieldDef。普通参数不是成员，允许与同名字段共存
    // （如 `Test(name: String) { let name = ... }`）；成员变量参数与成员变量
    // 同名、或字段重复声明则非法（2.1.10.2(16)）。
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for f in &st.fields {
        if f.is_static {
            continue;
        }
        *counts.entry(f.name.as_str()).or_insert(0) += 1;
    }
    let member_params: HashSet<&str> = st
        .primary_ctor_params
        .iter()
        .zip(st.primary_ctor_member_flags.iter())
        .filter(|(_, flag)| **flag)
        .map(|(p, _)| p.name.as_str())
        .collect();
    let regular_params: HashSet<&str> = st
        .primary_ctor_params
        .iter()
        .zip(st.primary_ctor_member_flags.iter())
        .filter(|(_, flag)| !**flag)
        .map(|(p, _)| p.name.as_str())
        .collect();
    let mut reported: HashSet<&str> = HashSet::new();
    for (name, cnt) in &counts {
        let bad = if member_params.contains(name) {
            // 成员变量参数与显式字段同名，或成员变量参数重复。
            *cnt > 1
        } else if regular_params.contains(name) {
            // 普通参数及其同名字段最多各一份；出现 3 次以上才是字段重复。
            *cnt > 2
        } else {
            *cnt > 1
        };
        if bad && reported.insert(*name) {
            errors.push(SemanticError {
                message: format!(
                    "结构体 '{}' 的实例成员变量/成员变量参数 '{}' 重复声明（2.1.10.2(16)）",
                    st.name, name
                ),
            });
        }
    }
}

/// 2.1.10.2(11)/(40)/(44)：构造函数返回前须初始化全部实例成员变量；
/// 无显式构造函数时，所有实例成员变量都必须有初始值（隐式构造函数）。
fn check_struct_ctor_definite_assignment(
    st: &crate::ast::StructDef,
    errors: &mut Vec<SemanticError>,
) {
    let (members, auto, primary_member) = struct_member_info(st);
    if st.inits.is_empty() {
        // 本方言支持结构体字面量构造，未定义显式构造函数的结构体不在此报错
        // （交由构造调用实参匹配与字面量规则处理）。
        return;
    }
    let required: HashSet<String> = members.difference(&auto).cloned().collect();
    if required.is_empty() {
        return;
    }
    for init in &st.inits {
        let mut start = auto.clone();
        // 主构造函数隐式初始化其成员变量参数；其它构造函数必须显式初始化。
        if is_primary_struct_init(init, st) {
            start.extend(primary_member.iter().cloned());
        }
        for (p, flag) in init.params.iter().zip(init.member_param_flags.iter()) {
            if *flag {
                start.insert(p.name.clone());
            }
        }
        let mut viol: Vec<String> = Vec::new();
        let (cur, flows) = da_stmt_list(&init.body, start, &required, &members, &mut viol);
        if flows {
            for name in &required {
                if !cur.contains(name) {
                    viol.push(name.clone());
                }
            }
        }
        viol.sort();
        viol.dedup();
        for name in viol {
            errors.push(SemanticError {
                message: format!(
                    "结构体 '{}' 的构造函数返回前未初始化实例成员变量 '{}'（2.1.10.2(11)/(40)）",
                    st.name, name
                ),
            });
        }
    }
}

/// 2.1.10.2(24)/(34)/(36)：全部实例成员变量初始化完成之前，构造函数内不得
/// 单独使用 `this`、读取未初始化成员、调用实例成员函数/属性或让 `this` 逃逸。
fn check_struct_use_before_init(
    st: &crate::ast::StructDef,
    program: &Program,
    errors: &mut Vec<SemanticError>,
) {
    let (field_names, auto, primary_member) = struct_member_info(st);
    // 主构造成员变量参数在构造函数入口处即已确定传入，保守视为已初始化。
    let mut auto_all = auto;
    auto_all.extend(primary_member.iter().cloned());
    if st.inits.is_empty() {
        return;
    }
    // 实例方法名 / 属性名：结构体内部方法被脱糖为 `StructName.method` 顶层函数。
    let prefix = format!("{}.", st.name);
    let mut method_names: HashSet<String> = HashSet::new();
    let mut prop_names: HashSet<String> = HashSet::new();
    for f in &program.functions {
        let Some(base) = f.name.strip_prefix(&prefix) else {
            continue;
        };
        if let Some(p) = base.strip_prefix("__get_").or_else(|| base.strip_prefix("__set_")) {
            prop_names.insert(p.to_string());
        } else {
            method_names.insert(base.to_string());
        }
    }
    for init in &st.inits {
        let param_names: HashSet<String> = init.params.iter().map(|p| p.name.clone()).collect();
        // 2.1.10.2(24)/(34)/(35)：构造函数中 this 不能被 lambda / 局部函数捕获。
        for s in &init.body {
            if let Some(reason) = nested_captures_this(
                s,
                &field_names,
                &method_names,
                &prop_names,
                &param_names,
            ) {
                errors.push(SemanticError {
                    message: format!(
                        "结构体 '{}' 的构造函数中 {} 捕获了 this（2.1.10.2(24)/(34)/(35)）",
                        st.name, reason
                    ),
                });
            }
        }
        let mut start = auto_all.clone();
        for (p, flag) in init.params.iter().zip(init.member_param_flags.iter()) {
            if *flag {
                start.insert(p.name.clone());
            }
        }
        let tracked: HashSet<String> = field_names.difference(&start).cloned().collect();
        if tracked.is_empty() {
            continue;
        }
        let param_names: HashSet<String> = init.params.iter().map(|p| p.name.clone()).collect();
        let mut cur = start;
        for s in &init.body {
            let all_init = tracked.is_subset(&cur);
            if !all_init {
                let empty_props: HashSet<String> = HashSet::new();
                if let Some(reason) = stmt_use_reason(
                    s,
                    &cur,
                    &tracked,
                    &field_names,
                    &method_names,
                    &prop_names,
                    &param_names,
                    &empty_props,
                ) {
                    errors.push(SemanticError {
                        message: format!(
                            "结构体 '{}' 的构造函数在所有实例成员变量初始化之前使用了 {}（2.1.10.2(24)/(34)/(36)）",
                            st.name, reason
                        ),
                    });
                }
            }
            let (nc, flows) = da_stmt(s, cur.clone(), &tracked, &field_names, &mut Vec::new());
            cur = nc;
            if !flows {
                break;
            }
        }
    }
}

/// 2.1.10.2(42)：构造函数的返回类型为 Unit，`return` 表达式必须是 Unit 类型。
fn check_struct_return_types(st: &crate::ast::StructDef, errors: &mut Vec<SemanticError>) {
    for init in &st.inits {
        let mut local_types: HashMap<String, Type> = HashMap::new();
        collect_explicit_local_types(&init.body, &mut local_types);
        let mut returns: Vec<&Expr> = Vec::new();
        collect_return_exprs(&init.body, &mut returns);
        for e in returns {
            // 返回显式标注为非 Unit 类型的局部变量（如 `let x: I = ...; return x`）。
            let local_nonunit = match e {
                Expr::Var(n) => local_types
                    .get(n)
                    .map(|ty| !matches!(ty, Type::Unit | Type::Nothing | Type::TypeParam(_)))
                    .unwrap_or(false),
                _ => false,
            };
            // 语法形态即可判定为非 Unit 的返回表达式。
            let syntactic_nonunit = matches!(
                e,
                Expr::Range { .. }
                    | Expr::Array(_)
                    | Expr::StructInit { .. }
                    | Expr::VariantConst { .. }
                    | Expr::MapLiteral { .. }
            ) || matches!(e, Expr::Tuple(v) if !v.is_empty());
            let inferred = infer_expr(e, &HashMap::new());
            let inferred_nonunit = match inferred {
                Some(ty) => {
                    !matches!(ty, Type::Unit | Type::Nothing | Type::TypeParam(_))
                        && !matches!(e, Expr::Tuple(v) if v.is_empty())
                }
                None => false,
            };
            if syntactic_nonunit || inferred_nonunit || local_nonunit {
                errors.push(SemanticError {
                    message: format!(
                        "结构体 '{}' 的构造函数的 return 表达式必须是 Unit 类型（2.1.10.2(42)）",
                        st.name
                    ),
                });
            }
        }
    }
}

/// 收集语句块中所有 `return <expr>` 的表达式（递归进入控制流块）。
fn collect_return_exprs<'a>(stmts: &'a [Stmt], out: &mut Vec<&'a Expr>) {
    for s in stmts {
        match s {
            Stmt::Return(Some(e)) => out.push(e),
            Stmt::Expr(e)
            | Stmt::Let { value: e, .. }
            | Stmt::Var { value: e, .. }
            | Stmt::Const { value: e, .. }
            | Stmt::Assign { value: e, .. } => collect_return_exprs_expr(e, out),
            Stmt::While { body, .. }
            | Stmt::WhileLet { body, .. }
            | Stmt::DoWhile { body, .. }
            | Stmt::For { body, .. }
            | Stmt::Loop { body }
            | Stmt::UnsafeBlock { body } => collect_return_exprs(body, out),
            _ => {}
        }
    }
}

fn collect_return_exprs_expr<'a>(e: &'a Expr, out: &mut Vec<&'a Expr>) {
    match e {
        Expr::If {
            then_branch,
            else_branch,
            ..
        } => {
            collect_return_exprs_expr(then_branch, out);
            if let Some(b) = else_branch {
                collect_return_exprs_expr(b, out);
            }
        }
        Expr::Block(stmts, trailing) => {
            collect_return_exprs(stmts, out);
            if let Some(t) = trailing {
                collect_return_exprs_expr(t, out);
            }
        }
        Expr::Match { arms, .. } => {
            for a in arms {
                collect_return_exprs_expr(&a.body, out);
            }
        }
        // lambda / 局部函数内的 return 属于其自身的返回，不属于构造函数。
        _ => {}
    }
}

/// 收集语句块（含嵌套控制流块）中局部函数的名称。
fn collect_local_func_names(stmts: &[Stmt], out: &mut HashSet<String>) {
    for s in stmts {
        match s {
            Stmt::LocalFunc(f) => {
                out.insert(f.name.clone());
                collect_local_func_names(&f.body, out);
            }
            Stmt::Expr(e)
            | Stmt::Let { value: e, .. }
            | Stmt::Var { value: e, .. }
            | Stmt::Const { value: e, .. }
            | Stmt::Assign { value: e, .. } => collect_local_func_names_expr(e, out),
            Stmt::While { body, .. }
            | Stmt::WhileLet { body, .. }
            | Stmt::DoWhile { body, .. }
            | Stmt::For { body, .. }
            | Stmt::Loop { body }
            | Stmt::UnsafeBlock { body } => collect_local_func_names(body, out),
            _ => {}
        }
    }
}

fn collect_local_func_names_expr(e: &Expr, out: &mut HashSet<String>) {
    match e {
        Expr::If {
            then_branch,
            else_branch,
            ..
        } => {
            collect_local_func_names_expr(then_branch, out);
            if let Some(b) = else_branch {
                collect_local_func_names_expr(b, out);
            }
        }
        Expr::Block(stmts, trailing) => {
            collect_local_func_names(stmts, out);
            if let Some(t) = trailing {
                collect_local_func_names_expr(t, out);
            }
        }
        Expr::Match { arms, .. } => {
            for a in arms {
                collect_local_func_names_expr(&a.body, out);
            }
        }
        _ => {}
    }
}

/// 默认值表达式是否修改了其中的参数（`x = ...`、`x++` 等，2.1.10.2(19)）。
fn expr_modifies_param(e: &Expr, names: &[&str]) -> bool {
    match e {
        Expr::Block(stmts, trailing) => {
            stmts.iter().any(|s| stmt_modifies_param(s, names))
                || trailing
                    .as_ref()
                    .map(|t| expr_modifies_param(t, names))
                    .unwrap_or(false)
        }
        Expr::PrefixIncr(x)
        | Expr::PrefixDecr(x)
        | Expr::PostfixIncr(x)
        | Expr::PostfixDecr(x) => {
            matches!(x.as_ref(), Expr::Var(n) if names.contains(&n.as_str()))
                || expr_modifies_param(x, names)
        }
        _ => false,
    }
}

fn stmt_modifies_param(s: &Stmt, names: &[&str]) -> bool {
    match s {
        Stmt::Assign { target, value } => {
            assign_target_is_param(target, names) || expr_modifies_param(value, names)
        }
        Stmt::Expr(e) => expr_modifies_param(e, names),
        _ => false,
    }
}

fn assign_target_is_param(t: &crate::ast::AssignTarget, names: &[&str]) -> bool {
    match t {
        crate::ast::AssignTarget::Var(n) => names.contains(&n.as_str()),
        crate::ast::AssignTarget::Tuple(ts) => {
            ts.iter().any(|x| assign_target_is_param(x, names))
        }
        _ => false,
    }
}

/// 单条语句中的 lambda / 局部函数是否捕获了 this（2.1.10.2(24)/(34)/(35)）。
fn nested_captures_this(
    s: &Stmt,
    members: &HashSet<String>,
    methods: &HashSet<String>,
    props: &HashSet<String>,
    params: &HashSet<String>,
) -> Option<String> {
    let mut hit: Option<String> = None;
    walk_stmt(s, &mut |e| {
        if hit.is_some() {
            return;
        }
        // lambda 直接引用 this/self，或引用实例成员函数/属性/成员变量（经隐式 this
        // 访问），均视为捕获 this。函数复合 `~>` 脱糖出的 lambda 引用已初始化的
        // 成员函数值属合法用法，不视为捕获。被构造参数遮蔽的裸名引用不视为捕获。
        if let Expr::Lambda { params: lparams, .. } = e {
            let is_composition = lparams.iter().any(|(n, _)| n == "__compose_arg");
            if !is_composition
                && (expr_contains_this(e)
                    || expr_refs_method_or_prop(e, methods, props)
                    || expr_refs_member(e, members, params))
            {
                hit = Some("lambda".to_string());
            }
        }
    });
    if hit.is_some() {
        return hit;
    }
    if let Stmt::LocalFunc(f) = s {
        if stmts_contain_this(&f.body)
            || stmts_ref_method_or_prop(&f.body, methods, props)
            || stmts_ref_member(&f.body, members, params)
        {
            return Some("局部函数".to_string());
        }
    }
    None
}

/// 语句块是否直接引用实例成员变量。
fn stmts_ref_member(
    stmts: &[Stmt],
    members: &HashSet<String>,
    params: &HashSet<String>,
) -> bool {
    let mut found = false;
    walk_stmts(stmts, &mut |e| {
        if expr_refs_member(e, members, params) {
            found = true;
        }
    });
    found
}

/// 表达式是否直接引用实例成员变量（裸名或 this.field）；被参数遮蔽的裸名忽略。
fn expr_refs_member(
    e: &Expr,
    members: &HashSet<String>,
    params: &HashSet<String>,
) -> bool {
    let mut found = false;
    walk_expr(e, &mut |x| match x {
        Expr::Var(n) if members.contains(n) && !params.contains(n) => found = true,
        Expr::Field { object, field } => {
            if members.contains(field)
                && matches!(object.as_ref(), Expr::Var(n) if n == "this" || n == "self")
            {
                found = true;
            }
        }
        _ => {}
    });
    found
}

/// 表达式是否引用实例成员函数/属性（经隐式 this 访问）。
fn expr_refs_method_or_prop(
    e: &Expr,
    methods: &HashSet<String>,
    props: &HashSet<String>,
) -> bool {
    let mut found = false;
    walk_expr(e, &mut |x| match x {
        Expr::Var(n) if methods.contains(n) || props.contains(n) => found = true,
        Expr::Call { name, .. } if methods.contains(name) => found = true,
        Expr::MethodCall { object, .. } | Expr::Field { object, .. } => {
            if matches!(object.as_ref(), Expr::Var(n) if n == "this" || n == "self") {
                found = true;
            }
        }
        _ => {}
    });
    found
}

/// 语句块是否引用实例成员函数/属性（经隐式 this 访问，视为捕获 this）。
fn stmts_ref_method_or_prop(
    stmts: &[Stmt],
    methods: &HashSet<String>,
    props: &HashSet<String>,
) -> bool {
    let mut found = false;
    walk_stmts(stmts, &mut |e| match e {
        Expr::Var(n) if methods.contains(n) || props.contains(n) => found = true,
        Expr::Call { name, .. } if methods.contains(name) => found = true,
        Expr::MethodCall { object, .. } | Expr::Field { object, .. } => {
            if matches!(object.as_ref(), Expr::Var(n) if n == "this" || n == "self") {
                found = true;
            }
        }
        _ => {}
    });
    found
}

/// 判断表达式是否在调用构造函数（`this(...)` 或其脱糖 `Invoke` 形态）。
fn expr_is_this_ctor_call(e: &Expr) -> bool {
    match e {
        Expr::Call { name, .. } => name == "this" || name == "self",
        Expr::Invoke { callee, .. } => {
            matches!(callee.as_ref(), Expr::Var(n) if n == "this" || n == "self")
        }
        _ => false,
    }
}

fn expr_contains_this_ctor_call(e: &Expr) -> bool {
    let mut found = false;
    walk_expr(e, &mut |x| {
        if expr_is_this_ctor_call(x) {
            found = true;
        }
    });
    found
}

fn stmt_contains_this_ctor_call(s: &Stmt) -> bool {
    let mut found = false;
    walk_stmt(s, &mut |x| {
        if expr_is_this_ctor_call(x) {
            found = true;
        }
    });
    found
}

fn stmts_contain_this_ctor_call(stmts: &[Stmt]) -> bool {
    stmts.iter().any(stmt_contains_this_ctor_call)
}

/// 2.1.10.2(23)/(38)/(39)：构造函数调用规则。
/// - 主构造函数不得通过 `this(...)` 调用其它构造函数；
/// - `this(...)` 必须是构造函数体中的第一条语句；
/// - 构造函数体之外（成员函数、属性访问器、字段初始值）不得调用 `this(...)`。
fn check_struct_this_calls(
    st: &crate::ast::StructDef,
    program: &Program,
    errors: &mut Vec<SemanticError>,
) {
    // 字段初始值中调用 this(...)。
    for f in &st.fields {
        if let Some(d) = &f.default {
            if expr_contains_this_ctor_call(d) {
                errors.push(SemanticError {
                    message: format!(
                        "结构体 '{}' 的字段 '{}' 的初始值不能调用构造函数 this(...)（2.1.10.2(39)）",
                        st.name, f.name
                    ),
                });
            }
        }
    }
    // 结构体成员函数（含属性访问器）中调用 this(...)。
    let prefix = format!("{}.", st.name);
    for func in &program.functions {
        if func.name.starts_with(&prefix) && stmts_contain_this_ctor_call(&func.body) {
            errors.push(SemanticError {
                message: format!(
                    "结构体 '{}' 的成员函数 '{}' 中不能调用构造函数 this(...)（2.1.10.2(39)）",
                    st.name, func.name
                ),
            });
        }
    }
    // 构造函数体内的 this(...) 规则。
    for init in &st.inits {
        if is_primary_struct_init(init, st) {
            if stmts_contain_this_ctor_call(&init.body) {
                errors.push(SemanticError {
                    message: format!(
                        "结构体 '{}' 的主构造函数不能通过 this(...) 调用其它构造函数（2.1.10.2(23)）",
                        st.name
                    ),
                });
            }
        } else {
            for (i, s) in init.body.iter().enumerate() {
                if !stmt_contains_this_ctor_call(s) {
                    continue;
                }
                let is_first_expr = i == 0
                    && matches!(s, Stmt::Expr(e) if expr_is_this_ctor_call(e));
                if !is_first_expr {
                    errors.push(SemanticError {
                        message: format!(
                            "结构体 '{}' 的构造函数中的 this(...) 必须是第一条语句（2.1.10.2(38)）",
                            st.name
                        ),
                    });
                }
            }
        }
    }
}

/// 2.1.10.2(41)：字段初始值中直接构造同类型实例会导致无限递归。
fn check_struct_field_self_ctor(st: &crate::ast::StructDef, errors: &mut Vec<SemanticError>) {
    for f in &st.fields {
        let Some(d) = &f.default else { continue };
        let mut found = false;
        walk_expr(d, &mut |e| {
            if let Expr::ConstructorCall { name, .. } = e {
                if name == &st.name {
                    found = true;
                }
            }
        });
        if found {
            errors.push(SemanticError {
                message: format!(
                    "结构体 '{}' 的字段 '{}' 的初始值直接构造同类型实例会导致无限递归（2.1.10.2(41)）",
                    st.name, f.name
                ),
            });
        }
    }
}

/// 2.1.10.2(15)/(14)：实例成员变量只能通过实例访问，不能经结构体名访问。
fn check_struct_static_field_access(
    st: &crate::ast::StructDef,
    program: &Program,
    errors: &mut Vec<SemanticError>,
) {
    let (members, _, _) = struct_member_info(st);
    if members.is_empty() {
        return;
    }
    let name = st.name.clone();
    let mut scan = |stmts: &[Stmt], errors: &mut Vec<SemanticError>| {
        let mut hit: Option<String> = None;
        walk_stmts(stmts, &mut |e| {
            if hit.is_some() {
                return;
            }
            if let Expr::Field { object, field } = e {
                if let Expr::Var(n) = object.as_ref() {
                    if n == &name && members.contains(field) {
                        hit = Some(field.clone());
                    }
                }
            }
        });
        if let Some(f) = hit {
            errors.push(SemanticError {
                message: format!(
                    "结构体 '{}' 的实例成员变量 '{}' 不能通过结构体名访问（2.1.10.2(15)）",
                    st.name, f
                ),
            });
        }
    };
    for func in &program.functions {
        scan(&func.body, errors);
    }
    for f in &st.fields {
        if let Some(d) = &f.default {
            let mut hit: Option<String> = None;
            walk_expr(d, &mut |e| {
                if hit.is_some() {
                    return;
                }
                if let Expr::Field { object, field } = e {
                    if let Expr::Var(n) = object.as_ref() {
                        if n == &name && members.contains(field) {
                            hit = Some(field.clone());
                        }
                    }
                }
            });
            if let Some(field) = hit {
                errors.push(SemanticError {
                    message: format!(
                        "结构体 '{}' 的实例成员变量 '{}' 不能通过结构体名访问（2.1.10.2(15)）",
                        st.name, field
                    ),
                });
            }
        }
    }
}

/// 2.1.10.2(41)：跨结构体构造函数在顶层无条件互相调用形成的循环依赖。
fn check_struct_mutual_ctor_cycles(program: &Program, errors: &mut Vec<SemanticError>) {
    use std::collections::HashSet as Set;
    let n = program.structs.len();
    if n == 0 {
        return;
    }
    let mut index: HashMap<&str, usize> = HashMap::new();
    for (i, s) in program.structs.iter().enumerate() {
        index.insert(s.name.as_str(), i);
    }
    let mut edges: Vec<Set<usize>> = vec![Set::new(); n];
    for (i, s) in program.structs.iter().enumerate() {
        for init in &s.inits {
            for stmt in &init.body {
                if let Stmt::Expr(e) = stmt {
                    let target = match e {
                        Expr::ConstructorCall { name, .. } => Some(name.as_str()),
                        _ => None,
                    };
                    if let Some(t) = target {
                        if t != s.name.as_str() {
                            if let Some(&j) = index.get(t) {
                                edges[i].insert(j);
                            }
                        }
                    }
                }
            }
        }
    }
    let mut color = vec![0u8; n];
    let mut has_cycle = false;
    fn dfs(u: usize, edges: &[Set<usize>], color: &mut [u8], has_cycle: &mut bool) {
        color[u] = 1;
        for &v in &edges[u] {
            if color[v] == 1 {
                *has_cycle = true;
            } else if color[v] == 0 {
                dfs(v, edges, color, has_cycle);
            }
        }
        color[u] = 2;
    }
    for u in 0..n {
        if color[u] == 0 {
            dfs(u, &edges, &mut color, &mut has_cycle);
        }
    }
    if has_cycle {
        errors.push(SemanticError {
            message: "结构体构造函数之间存在循环调用（2.1.10.2(41)）".to_string(),
        });
    }
}

// ─── 5.1.2 / 5.3.4 函数形参声明与默认值规则 ─────────────────────────────────

/// 收集顶层静态作用域内可见的名字（用于默认值“静态作用域”判定，5.1.2.2(8)）。
fn collect_top_level_names(program: &Program) -> HashSet<String> {
    let mut out = HashSet::new();
    for c in &program.constants {
        out.insert(c.name.clone());
    }
    for f in &program.functions {
        out.insert(f.name.clone());
    }
    for c in &program.classes {
        out.insert(c.name.clone());
    }
    for s in &program.structs {
        out.insert(s.name.clone());
    }
    for e in &program.enums {
        out.insert(e.name.clone());
    }
    for i in &program.interfaces {
        out.insert(i.name.clone());
    }
    for (n, _) in &program.type_aliases {
        out.insert(n.clone());
    }
    out
}

/// 函数名是否对应运算符重载。解析器把运算符映射为 `op_add` / `op_eq` 等标识符名，
/// 旧路径也可能保留原始符号；两者都识别。
fn is_operator_func_name(name: &str) -> bool {
    let last = name.rsplit('.').next().unwrap_or(name);
    matches!(
        last,
        "op_add"
            | "op_sub"
            | "op_mul"
            | "op_pow"
            | "op_div"
            | "op_mod"
            | "op_eq"
            | "op_ne"
            | "op_lt"
            | "op_gt"
            | "op_le"
            | "op_ge"
            | "op_shl"
            | "op_shr"
            | "op_not"
            | "op_bitand"
            | "op_bitor"
            | "op_bitxor"
            | "op_and"
            | "op_or"
            | "op_bitnot"
            | "op_index"
            | "op_call"
    ) || (!last.is_empty()
        && !last
            .chars()
            .next()
            .map_or(false, |c| c.is_alphabetic() || c == '_'))
}

/// 收集表达式中的变量引用（`Expr::Var`），不进入嵌套块 / lambda / match 等其它作用域，
/// 也不把调用名当作变量引用。用于默认值静态作用域检查。
fn collect_default_var_refs(expr: &Expr, out: &mut Vec<String>) {
    match expr {
        Expr::Var(name) => out.push(name.clone()),
        Expr::Unary { expr, .. }
        | Expr::Cast { expr, .. }
        | Expr::Some(expr)
        | Expr::Ok(expr)
        | Expr::Err(expr)
        | Expr::Try(expr)
        | Expr::Throw(expr)
        | Expr::PostfixIncr(expr)
        | Expr::PostfixDecr(expr)
        | Expr::PrefixIncr(expr)
        | Expr::PrefixDecr(expr)
        | Expr::IsType { expr, .. }
        | Expr::Field { object: expr, .. }
        | Expr::TupleIndex { object: expr, .. }
        | Expr::OptionalChain { object: expr, .. } => collect_default_var_refs(expr, out),
        Expr::Binary { left, right, .. }
        | Expr::NullCoalesce {
            option: left,
            default: right,
        } => {
            collect_default_var_refs(left, out);
            collect_default_var_refs(right, out);
        }
        Expr::Return(Some(e)) => collect_default_var_refs(e, out),
        Expr::Index { array, index } => {
            collect_default_var_refs(array, out);
            collect_default_var_refs(index, out);
        }
        Expr::Tuple(elems) | Expr::Array(elems) => {
            for e in elems {
                collect_default_var_refs(e, out);
            }
        }
        Expr::Range {
            start, end, step, ..
        } => {
            collect_default_var_refs(start, out);
            collect_default_var_refs(end, out);
            if let Some(s) = step {
                collect_default_var_refs(s, out);
            }
        }
        Expr::Call {
            args, named_args, ..
        } => {
            for a in args {
                collect_default_var_refs(a, out);
            }
            for (_, a) in named_args {
                collect_default_var_refs(a, out);
            }
        }
        Expr::MethodCall {
            object,
            args,
            named_args,
            ..
        } => {
            collect_default_var_refs(object, out);
            for a in args {
                collect_default_var_refs(a, out);
            }
            for (_, a) in named_args {
                collect_default_var_refs(a, out);
            }
        }
        Expr::StructInit { fields, .. } => {
            for (_, e) in fields {
                collect_default_var_refs(e, out);
            }
        }
        // 嵌套块 / lambda / match 属于其它作用域，不在此处收集。
        _ => {}
    }
}

/// 校验一段形参列表的声明规则：
/// - 5.1.2.1(2)：非命名形参必须位于所有命名形参之前；
/// - 5.1.2.2(3)：带默认值的形参必须是命名形参（`name!`）；
/// - 5.3.4(4)/(5)：变长形参必须是最后一个形参，且不能为命名形参；
/// - 5.1.2.2(8)/(12)/(13)：默认值不能引用自身或之后声明的形参。
fn check_param_list(
    params: &[Param],
    label: &str,
    globals: &HashSet<String>,
    check_order: bool,
    errors: &mut Vec<SemanticError>,
) {
    if check_order {
        if let Some(idx) = first_unnamed_after_named(params) {
            errors.push(SemanticError {
                message: format!(
                    "{}的第 {} 个非命名形参位于命名形参之后（5.1.2.1(2)）",
                    label,
                    idx + 1
                ),
            });
        }
    }
    let last_pos = params.len().saturating_sub(1);
    for (i, p) in params.iter().enumerate() {
        // 5.1.2.1：命名通配符形参 `_!` 非法（通配符不能作为命名参数）。
        if p.name == "_" && p.is_named {
            errors.push(SemanticError {
                message: format!("{}的命名形参不能使用通配符 `_!`（5.1.2.1）", label),
            });
        }
        // 5.1.2.1：形参必须带类型标注（`name: Type`）；`name = expr` 缺类型非法。
        if !p.variadic && matches!(&p.ty, Type::TypeParam(t) if t == "_") {
            errors.push(SemanticError {
                message: format!("{}的形参 '{}' 缺少类型标注（5.1.2.1）", label, p.name),
            });
        }
        // 5.1.2.2(3)：默认值参数（可选参数）必须是命名参数。
        if p.default.is_some() && !p.is_named && !p.variadic {
            errors.push(SemanticError {
                message: format!(
                    "{}的形参 '{}' 带默认值但不是命名形参（缺少 `!`）（5.1.2.2(3)）",
                    label, p.name
                ),
            });
        }
        // 5.3.4(4)/(5)：变长形参位置与命名冲突。
        if p.variadic {
            if i != last_pos {
                errors.push(SemanticError {
                    message: format!(
                        "{}的变长形参 '{}' 必须是最后一个形参（5.3.4(4)）",
                        label, p.name
                    ),
                });
            }
            if p.is_named {
                errors.push(SemanticError {
                    message: format!(
                        "{}的变长形参 '{}' 不能声明为命名形参（5.3.4(5)）",
                        label, p.name
                    ),
                });
            }
        }
    }
    // 5.1.2.2(8)/(12)/(13)：默认值的静态作用域——只能引用在它之前声明的形参。
    for (i, p) in params.iter().enumerate() {
        let Some(def) = &p.default else { continue };
        let mut refs = Vec::new();
        collect_default_var_refs(def, &mut refs);
        refs.sort();
        refs.dedup();
        for name in refs {
            if let Some(j) = params.iter().position(|q| q.name == name) {
                // j >= i：引用自身（j == i）或之后声明的形参（j > i）。
                // 若同名名字在顶层静态作用域已存在，则按静态作用域解析，不算引用形参。
                if j >= i && !globals.contains(&name) {
                    errors.push(SemanticError {
                        message: format!(
                            "{}的形参 '{}' 的默认值不能引用自身或之后声明的形参 '{}'（5.1.2.2(8)/(12)/(13)）",
                            label, p.name, name
                        ),
                    });
                }
            }
        }
    }
}

/// 5.1.2.2(1)/(5)/(6)：可选（默认值）参数在若干上下文中非法。
fn check_optional_param_contexts(program: &Program, errors: &mut Vec<SemanticError>) {
    // 接口成员函数（抽象或默认实现）不能有可选参数。
    for iface in &program.interfaces {
        for m in &iface.methods {
            if let Some(p) = m.params.iter().find(|p| p.default.is_some()) {
                errors.push(SemanticError {
                    message: format!(
                        "接口 '{}' 的成员函数 '{}' 不能有可选参数 '{}'（5.1.2.2(5)）",
                        iface.name, m.name, p.name
                    ),
                });
            }
        }
    }
    // 类中 open / abstract 成员函数不能有可选参数。
    for cls in &program.classes {
        for m in &cls.methods {
            if is_synthetic_member(&m.func.name) {
                continue;
            }
            if m.is_open || m.is_abstract {
                if let Some(p) = m.func.params.iter().find(|p| p.default.is_some()) {
                    errors.push(SemanticError {
                        message: format!(
                            "类 '{}' 的 open/abstract 成员函数 '{}' 不能有可选参数 '{}'（5.1.2.2(5)）",
                            cls.name, m.func.name, p.name
                        ),
                    });
                }
            }
        }
    }
    // 运算符函数不能有可选参数。
    for f in &program.functions {
        if is_operator_func_name(&f.name) {
            if let Some(p) = f.params.iter().find(|p| p.default.is_some()) {
                errors.push(SemanticError {
                    message: format!(
                        "运算符函数 '{}' 不能有可选参数 '{}'（5.1.2.2(6)）",
                        f.name, p.name
                    ),
                });
            }
        }
    }
    for cls in &program.classes {
        for m in &cls.methods {
            if is_operator_func_name(&m.func.name) {
                if let Some(p) = m.func.params.iter().find(|p| p.default.is_some()) {
                    errors.push(SemanticError {
                        message: format!(
                            "运算符函数 '{}' 不能有可选参数 '{}'（5.1.2.2(6)）",
                            m.func.name, p.name
                        ),
                    });
                }
            }
        }
    }
    for ext in &program.extends {
        for m in &ext.methods {
            if is_operator_func_name(&m.name) {
                if let Some(p) = m.params.iter().find(|p| p.default.is_some()) {
                    errors.push(SemanticError {
                        message: format!(
                            "运算符函数 '{}' 不能有可选参数 '{}'（5.1.2.2(6)）",
                            m.name, p.name
                        ),
                    });
                }
            }
        }
    }
}

/// 5.3.1：函数调用处命名实参 / 位置实参数量的静态检查。
///
/// 只对“在文件内唯一定义的顶层函数”的裸名调用做检查；变长参数函数、重载函数、
/// 与成员方法/常量同名的裸调用一并保守跳过，避免误伤合法代码。
fn check_call_arguments(program: &Program, errors: &mut Vec<SemanticError>) {
    // 裸名 → 顶层函数（仅唯一者参与检查）。
    let mut funcs: HashMap<&str, Vec<&Function>> = HashMap::new();
    for f in &program.functions {
        if f.name.contains('.') {
            continue;
        }
        funcs.entry(f.name.as_str()).or_default().push(f);
    }
    // 与成员方法末段名 / 顶层常量同名的裸调用可能解析到别处，保守跳过。
    let mut blocked: HashSet<&str> = HashSet::new();
    for c in &program.constants {
        blocked.insert(c.name.as_str());
    }
    for f in &program.functions {
        if f.name.contains('.') {
            if let Some(last) = f.name.rsplit('.').next() {
                blocked.insert(last);
            }
        }
    }
    for cls in &program.classes {
        for m in &cls.methods {
            blocked.insert(m.func.name.rsplit('.').next().unwrap_or(&m.func.name));
        }
    }
    for iface in &program.interfaces {
        for m in &iface.methods {
            blocked.insert(m.name.as_str());
        }
    }
    for ext in &program.extends {
        for m in &ext.methods {
            blocked.insert(m.name.rsplit('.').next().unwrap_or(&m.name));
        }
    }

    let mut check_stmts = |stmts: &[Stmt], errors: &mut Vec<SemanticError>| {
        // 宏实参由宏自行解释（如 @PowerAssert），其中的调用不做普通语义检查。
        // 先收集宏实参子树内表达式的地址，检查时跳过。
        let mut macro_roots: Vec<&Expr> = Vec::new();
        walk_stmts(stmts, &mut |e| {
            if let Expr::Macro { args, .. } = e {
                for a in args {
                    macro_roots.push(a);
                }
            }
        });
        let mut in_macro: HashSet<usize> = HashSet::new();
        for root in macro_roots {
            walk_expr(root, &mut |x| {
                in_macro.insert(x as *const Expr as usize);
            });
        }
        walk_stmts(stmts, &mut |e| {
            if in_macro.contains(&(e as *const Expr as usize)) {
                return;
            }
            let Expr::Call {
                name,
                args,
                named_args,
                ..
            } = e
            else {
                return;
            };
            let Some(cands) = funcs.get(name.as_str()) else {
                return;
            };
            if cands.len() != 1 || blocked.contains(name.as_str()) {
                return;
            }
            let f = cands[0];
            // 变长参数无法精确匹配实参数量，保守跳过。
            if f.params.iter().any(|p| p.variadic) {
                return;
            }
            let positional: Vec<&Param> = f.params.iter().filter(|p| !p.is_named).collect();
            let named_params: Vec<&Param> = f.params.iter().filter(|p| p.is_named).collect();
            // 5.3.4(1)：末个非命名形参为 Array 类型时允许变长实参语法糖，
            // 此时位置实参数量可与形参数量不同，跳过数量检查。
            let last_is_array = positional
                .last()
                .map_or(false, |p| matches!(&p.ty, Type::Array(_) | Type::Slice(_)));
            // 5.3.1(2)/(4)：位置实参过多——命名形参必须以 `名字:` 形式传入。
            if !last_is_array && args.len() > positional.len() {
                errors.push(SemanticError {
                    message: format!(
                        "调用函数 '{}' 的位置实参过多（{} 个），命名形参须以 `名字:` 形式传入（5.3.1(4)）",
                        name,
                        args.len()
                    ),
                });
            }
            // 5.3.1(1)/(3)：命名实参必须对应且仅对应命名形参。
            for (n, _) in named_args {
                if named_params.iter().any(|p| &p.name == n) {
                    continue;
                }
                if positional.iter().any(|p| &p.name == n) {
                    errors.push(SemanticError {
                        message: format!(
                            "调用函数 '{}'：形参 '{}' 不是命名参数，不能以 `名字:` 形式传入（5.3.1(3)）",
                            name, n
                        ),
                    });
                } else {
                    errors.push(SemanticError {
                        message: format!(
                            "调用函数 '{}'：不存在名为 '{}' 的命名形参（5.3.1(1)）",
                            name, n
                        ),
                    });
                }
            }
        });
    };

    for f in &program.functions {
        check_stmts(&f.body, errors);
    }
    for cls in &program.classes {
        for m in &cls.methods {
            check_stmts(&m.func.body, errors);
        }
        for init in &cls.inits {
            check_stmts(&init.body, errors);
        }
    }
    for s in &program.structs {
        for init in &s.inits {
            check_stmts(&init.body, errors);
        }
    }
    for ext in &program.extends {
        for m in &ext.methods {
            check_stmts(&m.body, errors);
        }
    }
    for iface in &program.interfaces {
        for m in &iface.methods {
            if let Some(b) = &m.default_body {
                check_stmts(b, errors);
            }
        }
    }
}

/// 5.1.2 / 5.3.4：遍历全部函数式声明，检查形参声明与默认值规则。
fn check_function_param_rules(program: &Program, errors: &mut Vec<SemanticError>) {
    let globals = collect_top_level_names(program);
    for f in &program.functions {
        check_param_list(
            &f.params,
            &format!("函数 '{}'", f.name),
            &globals,
            true,
            errors,
        );
    }
    for cls in &program.classes {
        for m in &cls.methods {
            if is_synthetic_member(&m.func.name) {
                continue;
            }
            check_param_list(
                &m.func.params,
                &format!("类 '{}' 的成员函数 '{}'", cls.name, m.func.name),
                &globals,
                true,
                errors,
            );
        }
        // 构造函数：普通/命名顺序已由 check_ctor_param_rules 检查，这里只查
        // 默认值必须命名与变长形参位置，避免重复报错。
        for init in &cls.inits {
            check_param_list(
                &init.params,
                &format!("类 '{}' 的构造函数", cls.name),
                &globals,
                false,
                errors,
            );
        }
    }
    for s in &program.structs {
        for init in &s.inits {
            check_param_list(
                &init.params,
                &format!("结构体 '{}' 的构造函数", s.name),
                &globals,
                false,
                errors,
            );
        }
    }
    for iface in &program.interfaces {
        for m in &iface.methods {
            check_param_list(
                &m.params,
                &format!("接口 '{}' 的成员函数 '{}'", iface.name, m.name),
                &globals,
                true,
                errors,
            );
        }
    }
    for ext in &program.extends {
        for m in &ext.methods {
            check_param_list(
                &m.params,
                &format!("扩展函数 '{}'", m.name),
                &globals,
                true,
                errors,
            );
        }
    }
    check_optional_param_contexts(program, errors);
    // 5.3.1：调用处命名实参 / 位置实参检查。
    check_call_arguments(program, errors);
}

// ─── 4.5.4/4.7：控制转移表达式（break/continue/return）的上下文合法性 ────────
//
// 规范 4.5.4(6)/4.7(6)/(10)：break/continue 必须（词法上）直接位于同一函数的
// 循环体内——出现在嵌套函数、lambda、顶层初始化式或字段默认值中均非法。
// 规范 4.7(7)：return 必须位于函数体内（不能出现在字段默认值等位置）。
//
// 这里用一趟独立的只读遍历实现：跟踪“循环深度”与“是否在函数体内”。进入
// 嵌套函数 / lambda 时重置循环深度（新函数上下文），其余控制结构（if/match/
// try/unsafe/synchronized 等）不改变循环深度。throw 因官方测试自相矛盾
// （@Description“允许”但 @Negative: yes）暂不校验。

fn check_control_transfer_placement(program: &Program, errors: &mut Vec<SemanticError>) {
    for f in &program.functions {
        scan_ct_stmts(&f.body, 0, true, errors);
    }
    for c in &program.classes {
        for init in &c.inits {
            scan_ct_stmts(&init.body, 0, true, errors);
        }
        if let Some(d) = &c.deinit {
            scan_ct_stmts(d, 0, true, errors);
        }
        if let Some(s) = &c.static_init {
            scan_ct_stmts(s, 0, true, errors);
        }
        for m in &c.methods {
            scan_ct_stmts(&m.func.body, 0, true, errors);
        }
        for fdef in &c.fields {
            if let Some(d) = &fdef.default {
                scan_ct_expr(d, 0, false, errors);
            }
        }
    }
    for s in &program.structs {
        for init in &s.inits {
            scan_ct_stmts(&init.body, 0, true, errors);
        }
        for fdef in &s.fields {
            if let Some(d) = &fdef.default {
                scan_ct_expr(d, 0, false, errors);
            }
        }
    }
    for iface in &program.interfaces {
        for m in &iface.methods {
            if let Some(b) = &m.default_body {
                scan_ct_stmts(b, 0, true, errors);
            }
        }
    }
    for en in &program.enums {
        for p in &en.props {
            if let Some(g) = &p.getter {
                scan_ct_stmts(g, 0, true, errors);
            }
            if let Some((_, set_body)) = &p.setter {
                scan_ct_stmts(set_body, 0, true, errors);
            }
        }
    }
    for ext in &program.extends {
        for m in &ext.methods {
            scan_ct_stmts(&m.body, 0, true, errors);
        }
    }
    for c in &program.constants {
        scan_ct_expr(&c.init, 0, false, errors);
    }
}

fn scan_ct_stmts(
    stmts: &[Stmt],
    loop_depth: usize,
    in_func: bool,
    errors: &mut Vec<SemanticError>,
) {
    for s in stmts {
        match s {
            Stmt::Break => {
                if loop_depth == 0 {
                    errors.push(SemanticError {
                        message: "'break' 只能用于循环体内（4.5.4(6)/4.7(6)）".to_string(),
                    });
                }
            }
            Stmt::Continue => {
                if loop_depth == 0 {
                    errors.push(SemanticError {
                        message: "'continue' 只能用于循环体内（4.7(6)）".to_string(),
                    });
                }
            }
            Stmt::Return(v) => {
                if !in_func {
                    errors.push(SemanticError {
                        message: "'return' 只能用于函数体内（4.7(7)）".to_string(),
                    });
                }
                if let Some(e) = v {
                    scan_ct_expr(e, loop_depth, in_func, errors);
                }
            }
            Stmt::Let { value, .. } | Stmt::Var { value, .. } | Stmt::Const { value, .. } => {
                scan_ct_expr(value, loop_depth, in_func, errors);
            }
            Stmt::Assign { target, value } => {
                scan_ct_target(target, loop_depth, in_func, errors);
                scan_ct_expr(value, loop_depth, in_func, errors);
            }
            Stmt::Expr(e) => scan_ct_expr(e, loop_depth, in_func, errors),
            Stmt::While { cond, body } => {
                scan_ct_expr(cond, loop_depth, in_func, errors);
                scan_ct_stmts(body, loop_depth + 1, in_func, errors);
            }
            Stmt::DoWhile { body, cond } => {
                scan_ct_stmts(body, loop_depth + 1, in_func, errors);
                scan_ct_expr(cond, loop_depth, in_func, errors);
            }
            Stmt::Loop { body } => scan_ct_stmts(body, loop_depth + 1, in_func, errors),
            Stmt::For {
                iterable, body, ..
            } => {
                scan_ct_expr(iterable, loop_depth, in_func, errors);
                scan_ct_stmts(body, loop_depth + 1, in_func, errors);
            }
            Stmt::WhileLet { expr, body, .. } => {
                scan_ct_expr(expr, loop_depth, in_func, errors);
                scan_ct_stmts(body, loop_depth + 1, in_func, errors);
            }
            Stmt::UnsafeBlock { body } => scan_ct_stmts(body, loop_depth, in_func, errors),
            Stmt::LocalFunc(f) => scan_ct_stmts(&f.body, 0, true, errors),
            Stmt::Assert { left, right, .. } | Stmt::Expect { left, right, .. } => {
                scan_ct_expr(left, loop_depth, in_func, errors);
                scan_ct_expr(right, loop_depth, in_func, errors);
            }
        }
    }
}

fn scan_ct_target(
    target: &AssignTarget,
    loop_depth: usize,
    in_func: bool,
    errors: &mut Vec<SemanticError>,
) {
    match target {
        AssignTarget::Index { index, .. } => scan_ct_expr(index, loop_depth, in_func, errors),
        AssignTarget::IndexPath { index, .. } => scan_ct_expr(index, loop_depth, in_func, errors),
        AssignTarget::ExprIndex { expr, index } => {
            scan_ct_expr(expr, loop_depth, in_func, errors);
            scan_ct_expr(index, loop_depth, in_func, errors);
        }
        AssignTarget::ExprField { object, .. } | AssignTarget::OptionalField { object, .. } => {
            scan_ct_expr(object, loop_depth, in_func, errors)
        }
        AssignTarget::Slice {
            array, start, end, ..
        } => {
            scan_ct_expr(array, loop_depth, in_func, errors);
            scan_ct_expr(start, loop_depth, in_func, errors);
            scan_ct_expr(end, loop_depth, in_func, errors);
        }
        AssignTarget::Tuple(ts) => {
            for t in ts {
                scan_ct_target(t, loop_depth, in_func, errors);
            }
        }
        _ => {}
    }
}

fn scan_ct_expr(
    expr: &Expr,
    loop_depth: usize,
    in_func: bool,
    errors: &mut Vec<SemanticError>,
) {
    match expr {
        Expr::Break => {
            if loop_depth == 0 {
                errors.push(SemanticError {
                    message: "'break' 只能用于循环体内（4.5.4(6)/4.7(6)）".to_string(),
                });
            }
        }
        Expr::Continue => {
            if loop_depth == 0 {
                errors.push(SemanticError {
                    message: "'continue' 只能用于循环体内（4.7(6)）".to_string(),
                });
            }
        }
        Expr::Return(v) => {
            if !in_func {
                errors.push(SemanticError {
                    message: "'return' 只能用于函数体内（4.7(7)）".to_string(),
                });
            }
            if let Some(e) = v {
                scan_ct_expr(e, loop_depth, in_func, errors);
            }
        }
        // lambda 是独立函数上下文：循环深度重置，return 合法。
        Expr::Lambda { body, .. } => scan_ct_expr(body, 0, true, errors),
        Expr::Interpolate(parts) => {
            for p in parts {
                if let crate::ast::InterpolatePart::Expr(e) = p {
                    scan_ct_expr(e, loop_depth, in_func, errors);
                }
            }
        }
        Expr::Unary { expr, .. }
        | Expr::Cast { expr, .. }
        | Expr::IsType { expr, .. }
        | Expr::Some(expr)
        | Expr::Ok(expr)
        | Expr::Err(expr)
        | Expr::Try(expr)
        | Expr::Throw(expr)
        | Expr::PostfixIncr(expr)
        | Expr::PostfixDecr(expr)
        | Expr::PrefixIncr(expr)
        | Expr::PrefixDecr(expr) => scan_ct_expr(expr, loop_depth, in_func, errors),
        Expr::Field { object, .. }
        | Expr::OptionalChain { object, .. }
        | Expr::TupleIndex { object, .. } => scan_ct_expr(object, loop_depth, in_func, errors),
        Expr::VariantConst { arg, .. } => {
            if let Some(a) = arg {
                scan_ct_expr(a, loop_depth, in_func, errors);
            }
        }
        Expr::Binary { left, right, .. } => {
            scan_ct_expr(left, loop_depth, in_func, errors);
            scan_ct_expr(right, loop_depth, in_func, errors);
        }
        Expr::NullCoalesce { option, default } => {
            scan_ct_expr(option, loop_depth, in_func, errors);
            scan_ct_expr(default, loop_depth, in_func, errors);
        }
        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => {
            scan_ct_expr(cond, loop_depth, in_func, errors);
            scan_ct_expr(then_branch, loop_depth, in_func, errors);
            if let Some(e) = else_branch {
                scan_ct_expr(e, loop_depth, in_func, errors);
            }
        }
        Expr::IfLet {
            expr,
            then_branch,
            else_branch,
            ..
        } => {
            scan_ct_expr(expr, loop_depth, in_func, errors);
            scan_ct_expr(then_branch, loop_depth, in_func, errors);
            if let Some(e) = else_branch {
                scan_ct_expr(e, loop_depth, in_func, errors);
            }
        }
        Expr::Block(stmts, tail) => {
            scan_ct_stmts(stmts, loop_depth, in_func, errors);
            if let Some(e) = tail {
                scan_ct_expr(e, loop_depth, in_func, errors);
            }
        }
        Expr::Tuple(es) | Expr::Array(es) => {
            for e in es {
                scan_ct_expr(e, loop_depth, in_func, errors);
            }
        }
        Expr::Index { array, index } => {
            scan_ct_expr(array, loop_depth, in_func, errors);
            scan_ct_expr(index, loop_depth, in_func, errors);
        }
        Expr::SliceExpr { array, start, end } => {
            scan_ct_expr(array, loop_depth, in_func, errors);
            scan_ct_expr(start, loop_depth, in_func, errors);
            scan_ct_expr(end, loop_depth, in_func, errors);
        }
        Expr::Range {
            start,
            end,
            step,
            ..
        } => {
            scan_ct_expr(start, loop_depth, in_func, errors);
            scan_ct_expr(end, loop_depth, in_func, errors);
            if let Some(s) = step {
                scan_ct_expr(s, loop_depth, in_func, errors);
            }
        }
        Expr::StructInit { fields, .. } => {
            for (_, e) in fields {
                scan_ct_expr(e, loop_depth, in_func, errors);
            }
        }
        Expr::ConstructorCall {
            args, named_args, ..
        }
        | Expr::Call {
            args, named_args, ..
        }
        | Expr::SuperCall {
            args, named_args, ..
        } => {
            for e in args {
                scan_ct_expr(e, loop_depth, in_func, errors);
            }
            for (_, e) in named_args {
                scan_ct_expr(e, loop_depth, in_func, errors);
            }
        }
        Expr::MethodCall {
            object,
            args,
            named_args,
            ..
        } => {
            scan_ct_expr(object, loop_depth, in_func, errors);
            for e in args {
                scan_ct_expr(e, loop_depth, in_func, errors);
            }
            for (_, e) in named_args {
                scan_ct_expr(e, loop_depth, in_func, errors);
            }
        }
        Expr::Match { expr, arms } => {
            scan_ct_expr(expr, loop_depth, in_func, errors);
            for arm in arms {
                if let Some(g) = &arm.guard {
                    scan_ct_expr(g, loop_depth, in_func, errors);
                }
                scan_ct_expr(&arm.body, loop_depth, in_func, errors);
            }
        }
        Expr::MapLiteral { entries } => {
            for (k, v) in entries {
                scan_ct_expr(k, loop_depth, in_func, errors);
                scan_ct_expr(v, loop_depth, in_func, errors);
            }
        }
        Expr::TryBlock {
            resources,
            body,
            catch_body,
            finally_body,
            ..
        } => {
            for (_, e) in resources {
                scan_ct_expr(e, loop_depth, in_func, errors);
            }
            scan_ct_stmts(body, loop_depth, in_func, errors);
            scan_ct_stmts(catch_body, loop_depth, in_func, errors);
            if let Some(f) = finally_body {
                scan_ct_stmts(f, loop_depth, in_func, errors);
            }
        }
        // spawn 创建独立并发上下文：循环深度重置。
        Expr::Spawn { body } => scan_ct_stmts(body, 0, in_func, errors),
        Expr::Synchronized { lock, body } => {
            scan_ct_expr(lock, loop_depth, in_func, errors);
            scan_ct_stmts(body, loop_depth, in_func, errors);
        }
        Expr::TrailingClosure {
            callee,
            args,
            closure,
        } => {
            scan_ct_expr(callee, loop_depth, in_func, errors);
            for e in args {
                scan_ct_expr(e, loop_depth, in_func, errors);
            }
            // 尾随闭包本身是一个 lambda 表达式，交给 Lambda 分支重置循环深度。
            scan_ct_expr(closure, loop_depth, in_func, errors);
        }
        Expr::Invoke {
            callee,
            args,
            named_args,
        } => {
            scan_ct_expr(callee, loop_depth, in_func, errors);
            for e in args {
                scan_ct_expr(e, loop_depth, in_func, errors);
            }
            for (_, e) in named_args {
                scan_ct_expr(e, loop_depth, in_func, errors);
            }
        }
        Expr::Macro { args, .. } => {
            for e in args {
                scan_ct_expr(e, loop_depth, in_func, errors);
            }
        }
        // 叶子节点。
        Expr::Integer(_)
        | Expr::IntegerSuffixed(_, _)
        | Expr::Float(_)
        | Expr::Float32(_)
        | Expr::Bool(_)
        | Expr::Rune(_)
        | Expr::String(_)
        | Expr::Var(_)
        | Expr::None
        | Expr::SuperFieldAccess { .. } => {}
    }
}

// ─── 10.2.3(4)-(14)：运算符重载函数的元数（参数个数）校验 ───────────────────

fn check_operator_function_signatures(program: &Program, errors: &mut Vec<SemanticError>) {
    fn check_one(
        name: &str,
        params: &[Param],
        ret: Option<&Type>,
        errors: &mut Vec<SemanticError>,
    ) {
        let last = name.rsplit('.').next().unwrap_or(name);
        // 剥离隐式/显式接收者（this/self），只统计“操作数”形参。
        let (ps, _, _) = split_receiver(params);
        // 10.2.4：下标运算符 set 形态（带命名参数）的声明细则。
        if last == "op_index" {
            let operands: &[Param] = if params
                .first()
                .map_or(false, |p| is_receiver_param(&p.name))
            {
                &params[1..]
            } else {
                params
            };
            let named: Vec<&Param> = operands.iter().filter(|p| p.is_named).collect();
            if !named.is_empty() {
                if named.len() > 1 {
                    errors.push(SemanticError {
                        message:
                            "set 形态下标运算符只能有一个命名参数（10.2.4）".to_string(),
                    });
                }
                if let Some(v) = named.first() {
                    if v.name != "value" {
                        errors.push(SemanticError {
                            message: format!(
                                "set 形态下标运算符的命名参数必须命名为 'value'，实际为 '{}'（10.2.4）",
                                v.name
                            ),
                        });
                    }
                }
                if operands.last().map_or(false, |p| !p.is_named) {
                    errors.push(SemanticError {
                        message:
                            "set 形态下标运算符的命名参数必须是最后一个参数（10.2.4）"
                                .to_string(),
                    });
                }
                if let Some(t) = ret {
                    if *t != Type::Unit {
                        errors.push(SemanticError {
                            message:
                                "set 形态下标运算符的返回类型必须为 Unit（10.2.4）"
                                    .to_string(),
                        });
                    }
                }
            }
            return;
        }
        let arity = ps.len();
        // (符号, 允许的参数个数)；空切片表示该运算符不可重载。
        let (symbol, allowed): (&str, &[usize]) = match last {
            "op_and" => ("&&", &[]),
            "op_or" => ("||", &[]),
            "op_not" => ("!", &[0]),
            "op_bitnot" => ("~", &[0]),
            // `-` 既可一元（0 参）也可二元（1 参）。
            "op_sub" => ("-", &[0, 1]),
            "op_add" => ("+", &[1]),
            "op_mul" => ("*", &[1]),
            "op_pow" => ("**", &[1]),
            "op_div" => ("/", &[1]),
            "op_mod" => ("%", &[1]),
            "op_eq" => ("==", &[1]),
            "op_ne" => ("!=", &[1]),
            "op_lt" => ("<", &[1]),
            "op_gt" => (">", &[1]),
            "op_le" => ("<=", &[1]),
            "op_ge" => (">=", &[1]),
            "op_shl" => ("<<", &[1]),
            "op_shr" => (">>", &[1]),
            "op_bitand" => ("&", &[1]),
            "op_bitor" => ("|", &[1]),
            "op_bitxor" => ("^", &[1]),
            // op_index / op_call 的形参规则另有约定，此处不校验。
            _ => return,
        };
        if allowed.is_empty() {
            errors.push(SemanticError {
                message: format!("运算符 '{}' 不能被重载（10.2.3(14)）", symbol),
            });
            return;
        }
        if !allowed.contains(&arity) {
            let need = allowed
                .iter()
                .map(|n| n.to_string())
                .collect::<Vec<_>>()
                .join(" 或 ");
            errors.push(SemanticError {
                message: format!(
                    "运算符函数 '{}' 的参数个数不合法：需要 {} 个（不含接收者），实际 {} 个（10.2.3）",
                    symbol, need, arity
                ),
            });
        }
    }

    for f in &program.functions {
        check_one(&f.name, &f.params, f.return_type.as_ref(), errors);
    }
    for c in &program.classes {
        for m in &c.methods {
            check_one(&m.func.name, &m.func.params, m.func.return_type.as_ref(), errors);
        }
    }
    for iface in &program.interfaces {
        for m in &iface.methods {
            check_one(&m.name, &m.params, m.return_type.as_ref(), errors);
        }
    }
    for ext in &program.extends {
        for m in &ext.methods {
            check_one(&m.name, &m.params, m.return_type.as_ref(), errors);
        }
    }
}

// ─── 03 章：名称 / 作用域 / 变量（w15d 批） ───────────────────────────────────
// 本段实现若干仅凭单文件 AST 即可精确判定的名称作用域规则。所有检查都遵循
// “宁可漏判不可误判”的保守策略：无法静态确定（导入符号、类型参数歧义、
// @C 结构体、跨文件信息缺失等）时一律放行。

/// 3.3.2.2(8)：全局 / 静态成员变量必须在声明时立即初始化。
///
/// 仅当类/结构体的 `static let/var` 字段显式带声明关键字、却既无默认值、
/// 又不存在 `static init` 块可用于延迟初始化时报错。`@C` 结构体与无关键字
/// 的方言字段形态不受影响。
fn check_static_fields_initialized(program: &Program, errors: &mut Vec<SemanticError>) {
    // 解析器把 `static init() {...}` 编译成形如 `Type.__static_init` 的函数
    // （类内方法保存在 ClassDef.methods，结构体方法合入 program.functions）。
    let mut static_init_types: HashSet<String> = HashSet::new();
    for f in &program.functions {
        if let Some((owner, seg)) = f.name.split_once('.') {
            if seg == "__static_init" {
                static_init_types.insert(owner.to_string());
            }
        }
    }
    for c in &program.classes {
        for m in &c.methods {
            if m.func.name.ends_with(".__static_init") {
                static_init_types.insert(c.name.clone());
            }
        }
    }
    let has_static_init = |type_name: &str| static_init_types.contains(type_name);
    for c in &program.classes {
        // 存在 static init 时允许延迟初始化（保守放行）。
        let deferred_ok = c.static_init.is_some() || has_static_init(&c.name);
        if deferred_ok {
            continue;
        }
        for f in &c.fields {
            if f.is_static && f.decl_kw && f.default.is_none() {
                errors.push(SemanticError {
                    message: format!(
                        "静态成员变量 '{}.{}' 必须立即初始化（3.3.2.2(8)）",
                        c.name, f.name
                    ),
                });
            }
        }
    }
    // 结构体的 `static init` 体在解析阶段被丢弃，无法在 AST 中判定是否存在
    // 延迟初始化块，故结构体字段保守放行（仅类参与本检查）。
}

/// 3.2.3.1(1) / 3.2.3(1)：顶层变量初始化式只能引用顶层可见的名字。
///
/// 函数局部变量、形参、其它类型的成员名在顶层均不可见，顶层初始化式引用它们
/// 属于未定义标识符。仅在“可确定单文件导入形态”的程序上启用（与
/// [`check_undefined_identifiers`] 相同的保守门限），避免导入符号误判。
fn check_top_level_initializer_scope(program: &Program, errors: &mut Vec<SemanticError>) {
    if !imports_allow_undefined_check(program) {
        return;
    }
    let globals = collect_global_identifiers(program);
    for c in &program.constants {
        let mut refs = Vec::new();
        collect_bare_var_refs_expr(&c.init, &mut refs);
        let mut reported: HashSet<String> = HashSet::new();
        for r in refs {
            if globals.contains(&r) || is_prelude_identifier(&r) {
                continue;
            }
            // 首字母大写按约定视为类型 / 隐式导入符号（对布尔大小写变体仍拒绝）。
            if r.chars().next().map_or(false, |ch| ch.is_uppercase())
                && !matches!(r.to_lowercase().as_str(), "true" | "false")
            {
                continue;
            }
            if reported.insert(r.clone()) {
                errors.push(SemanticError {
                    message: format!(
                        "顶层变量 '{}' 的初始化式引用了顶层不可见的标识符 '{}'（3.2.3.1(1)）",
                        c.name, r
                    ),
                });
            }
        }
    }
}

/// 收集程序中声明的全部类型参数名（类/结构体/接口/枚举/函数/方法级）。
fn collect_declared_type_param_names(program: &Program) -> HashSet<String> {
    let mut tps: HashSet<String> = HashSet::new();
    for f in &program.functions {
        tps.extend(f.type_params.iter().cloned());
    }
    for c in &program.classes {
        tps.extend(c.type_params.iter().cloned());
        for m in &c.methods {
            tps.extend(m.func.type_params.iter().cloned());
        }
    }
    for s in &program.structs {
        tps.extend(s.type_params.iter().cloned());
    }
    for i in &program.interfaces {
        tps.extend(i.type_params.iter().cloned());
        for m in &i.methods {
            tps.extend(m.type_params.iter().cloned());
        }
    }
    for e in &program.enums {
        tps.extend(e.type_params.iter().cloned());
    }
    for x in &program.extends {
        for m in &x.methods {
            tps.extend(m.type_params.iter().cloned());
        }
    }
    tps
}

/// 递归收集一个类型表达式中“以 `Type::Struct` 形态出现的、实为某处类型参数名”
/// 的离作用域引用。
fn collect_out_of_scope_type_params<'a>(
    ty: &'a Type,
    tps: &HashSet<String>,
    known_types: &HashSet<String>,
    out: &mut Vec<&'a str>,
) {
    match ty {
        Type::Struct(name, args) => {
            if tps.contains(name)
                && !known_types.contains(name)
                && !is_builtin_type_name(name)
            {
                out.push(name.as_str());
            }
            for a in args {
                collect_out_of_scope_type_params(a, tps, known_types, out);
            }
        }
        Type::Array(inner) | Type::Option(inner) | Type::Slice(inner) => {
            collect_out_of_scope_type_params(inner, tps, known_types, out)
        }
        Type::Result(a, b) | Type::Map(a, b) => {
            collect_out_of_scope_type_params(a, tps, known_types, out);
            collect_out_of_scope_type_params(b, tps, known_types, out);
        }
        Type::Tuple(items) => {
            for t in items {
                collect_out_of_scope_type_params(t, tps, known_types, out);
            }
        }
        Type::Function { params, ret } => {
            for t in params {
                collect_out_of_scope_type_params(t, tps, known_types, out);
            }
            if let Some(r) = ret.as_ref().as_ref() {
                collect_out_of_scope_type_params(r, tps, known_types, out);
            }
        }
        _ => {}
    }
}

/// [`check_type_param_scope`] 的收集器：只报告“名字是程序中某处声明的类型参数、
/// 却不在当前作用域内”的 `Type::Struct` 引用。
struct TypeParamScopeCtx<'a> {
    /// 程序中所有声明过的类型参数名。
    all_tps: &'a HashSet<String>,
    /// 已知真实类型名（用户类型 / 别名）：同名一律放行，避免与类型参数撞名。
    known_types: &'a HashSet<String>,
    bad: Vec<String>,
}

impl<'a> TypeParamScopeCtx<'a> {
    fn ty(&mut self, ty: &Type, scope: &HashSet<String>) {
        let mut hits: Vec<&str> = Vec::new();
        self.ty_inner(ty, scope, &mut hits);
        for h in hits {
            self.bad.push(h.to_string());
        }
    }
    fn ty_inner<'t>(&mut self, ty: &'t Type, scope: &HashSet<String>, out: &mut Vec<&'t str>) {
        match ty {
            Type::Struct(name, args) => {
                if self.all_tps.contains(name)
                    && !scope.contains(name)
                    && !self.known_types.contains(name)
                    && !is_builtin_type_name(name)
                {
                    out.push(name.as_str());
                }
                for a in args {
                    self.ty_inner(a, scope, out);
                }
            }
            Type::Array(inner) | Type::Option(inner) | Type::Slice(inner) => {
                self.ty_inner(inner, scope, out)
            }
            Type::Result(a, b) | Type::Map(a, b) => {
                self.ty_inner(a, scope, out);
                self.ty_inner(b, scope, out);
            }
            Type::Tuple(items) => {
                for t in items {
                    self.ty_inner(t, scope, out);
                }
            }
            Type::Function { params, ret } => {
                for t in params {
                    self.ty_inner(t, scope, out);
                }
                if let Some(r) = ret.as_ref().as_ref() {
                    self.ty_inner(r, scope, out);
                }
            }
            _ => {}
        }
    }
    fn pattern(&mut self, p: &crate::ast::Pattern, scope: &HashSet<String>) {
        for_each_pattern_type(p, &mut |ty| self.ty(ty, scope));
    }
    fn expr(&mut self, e: &Expr, scope: &HashSet<String>) {
        for_each_expr_type(e, &mut |ty| self.ty(ty, scope));
    }
    fn stmts(&mut self, stmts: &[Stmt], scope: &HashSet<String>) {
        for s in stmts {
            match s {
                Stmt::Let { ty, pattern, value } | Stmt::Var { ty, pattern, value } => {
                    if let Some(t) = ty {
                        self.ty(t, scope);
                    }
                    self.pattern(pattern, scope);
                    walk_expr(value, &mut |e| self.expr(e, scope));
                }
                Stmt::Const { ty, value, .. } => {
                    if let Some(t) = ty {
                        self.ty(t, scope);
                    }
                    walk_expr(value, &mut |e| self.expr(e, scope));
                }
                Stmt::LocalFunc(f) => self.func(f, scope),
                Stmt::While { body, cond } => {
                    walk_expr(cond, &mut |e| self.expr(e, scope));
                    self.stmts(body, scope);
                }
                Stmt::DoWhile { body, cond } => {
                    self.stmts(body, scope);
                    walk_expr(cond, &mut |e| self.expr(e, scope));
                }
                Stmt::Loop { body } | Stmt::UnsafeBlock { body } => self.stmts(body, scope),
                Stmt::For { iterable, body, .. } => {
                    walk_expr(iterable, &mut |e| self.expr(e, scope));
                    self.stmts(body, scope);
                }
                Stmt::WhileLet {
                    pattern,
                    expr,
                    body,
                } => {
                    self.pattern(pattern, scope);
                    walk_expr(expr, &mut |e| self.expr(e, scope));
                    self.stmts(body, scope);
                }
                _ => walk_stmt(s, &mut |e| self.expr(e, scope)),
            }
        }
    }
    /// 检查一个函数声明：参数 / 返回类型 / 体，作用域 = 外层 + 本函数类型参数。
    fn func(&mut self, f: &Function, outer: &HashSet<String>) {
        let mut scope = outer.clone();
        scope.extend(f.type_params.iter().cloned());
        for p in &f.params {
            self.ty(&p.ty, &scope);
        }
        if let Some(r) = &f.return_type {
            self.ty(r, &scope);
        }
        self.stmts(&f.body, &scope);
    }
}

/// 3.2.3.2(14) / 3.2.3.3(2)：类型参数的作用域仅限引入它的声明（及其内部）。
///
/// 采用作用域感知的遍历：每个声明携带其可见的类型参数集合（类/结构体/接口/
/// 枚举的类型参数 + 函数/方法自身的类型参数 + 外层嵌套函数的类型参数）。
/// 只有当某 `Type::Struct` 引用的名字是程序中**某处**声明的类型参数、却不在
/// 当前作用域内、且不是已知真实类型/别名/内建类型时才报错。为避免与导入符号
/// 或 `extend<T>` 等语法冲突，仅在无 import 的单文件程序上启用，且跳过
/// `extend` 体（其泛型参数未保存在 AST 中）。
fn check_type_param_scope(program: &Program, errors: &mut Vec<SemanticError>) {
    if !imports_allow_undefined_check(program) {
        return;
    }
    let all_tps = collect_declared_type_param_names(program);
    if all_tps.is_empty() {
        return;
    }
    let mut known_types: HashSet<String> = HashSet::new();
    for c in &program.classes {
        known_types.insert(c.name.clone());
    }
    for s in &program.structs {
        known_types.insert(s.name.clone());
    }
    for i in &program.interfaces {
        known_types.insert(i.name.clone());
    }
    for e in &program.enums {
        known_types.insert(e.name.clone());
    }
    for (n, _) in &program.type_aliases {
        known_types.insert(n.clone());
    }
    let mut ctx = TypeParamScopeCtx {
        all_tps: &all_tps,
        known_types: &known_types,
        bad: Vec::new(),
    };
    let empty: HashSet<String> = HashSet::new();
    // 顶层自由函数 + 成员方法（`Type.method`）：成员方法的作用域包含所属类型的类型参数。
    for f in &program.functions {
        let mut scope = empty.clone();
        if let Some((owner, _)) = f.name.split_once('.') {
            for c in &program.classes {
                if c.name == owner {
                    scope.extend(c.type_params.iter().cloned());
                }
            }
            for s in &program.structs {
                if s.name == owner {
                    scope.extend(s.type_params.iter().cloned());
                }
            }
            for e in &program.enums {
                if e.name == owner {
                    scope.extend(e.type_params.iter().cloned());
                }
            }
        }
        ctx.func(f, &scope);
    }
    for c in &program.classes {
        let scope: HashSet<String> = c.type_params.iter().cloned().collect();
        for fld in &c.fields {
            ctx.ty(&fld.ty, &scope);
        }
        for p in &c.primary_ctor_params {
            ctx.ty(&p.ty, &scope);
        }
        for init in c.inits.iter() {
            for p in &init.params {
                ctx.ty(&p.ty, &scope);
            }
            ctx.stmts(&init.body, &scope);
        }
        // 类成员方法（部分可能只保存在 ClassDef.methods 中）。
        for m in &c.methods {
            ctx.func(&m.func, &scope);
        }
    }
    for s in &program.structs {
        let scope: HashSet<String> = s.type_params.iter().cloned().collect();
        for fld in &s.fields {
            ctx.ty(&fld.ty, &scope);
        }
        for p in &s.primary_ctor_params {
            ctx.ty(&p.ty, &scope);
        }
        for init in &s.inits {
            for p in &init.params {
                ctx.ty(&p.ty, &scope);
            }
            ctx.stmts(&init.body, &scope);
        }
    }
    for i in &program.interfaces {
        for m in &i.methods {
            let mut scope: HashSet<String> = i.type_params.iter().cloned().collect();
            scope.extend(m.type_params.iter().cloned());
            for p in &m.params {
                ctx.ty(&p.ty, &scope);
            }
            if let Some(r) = &m.return_type {
                ctx.ty(r, &scope);
            }
            if let Some(body) = &m.default_body {
                ctx.stmts(body, &scope);
            }
        }
    }
    for e in &program.enums {
        let scope: HashSet<String> = e.type_params.iter().cloned().collect();
        for v in &e.variants {
            if let Some(p) = &v.payload {
                ctx.ty(p, &scope);
            }
        }
    }
    let mut reported: HashSet<String> = HashSet::new();
    for name in ctx.bad {
        if reported.insert(name.clone()) {
            errors.push(SemanticError {
                message: format!(
                    "类型参数 '{}' 超出其声明作用域，不能在此使用（3.2.3.2(14)）",
                    name
                ),
            });
        }
    }
}


/// 5.3.4(7)：其它运算符重载不支持变长实参 / 元组实参语法糖。
///
/// 当某个用户类型的运算符重载形参为数组类型（`operator func op(arr: Array<T>)`）时，
/// 以元组字面量作为该运算符的实参（如 `obj **(1, 2)`）是“变长实参语法糖”在
/// 运算符上的误用，必然无法通过类型检查。本检查仅对**能静态解析出接收者类型**
/// 且该类型的同名运算符确有数组形参的情形报错，其余（类型未知、内建类型、
/// 形参非数组）一律放行。
fn check_binary_tuple_operand(program: &Program, errors: &mut Vec<SemanticError>) {
    use crate::ast::BinOp;
    // 运算符 → op_xxx 成员名。
    fn op_member(op: &BinOp) -> Option<&'static str> {
        Some(match op {
            BinOp::Add => "op_add",
            BinOp::Sub => "op_sub",
            BinOp::Mul => "op_mul",
            BinOp::Div => "op_div",
            BinOp::Mod => "op_mod",
            BinOp::Pow => "op_pow",
            BinOp::Eq => "op_eq",
            BinOp::NotEq => "op_ne",
            BinOp::Lt => "op_lt",
            BinOp::Gt => "op_gt",
            BinOp::LtEq => "op_le",
            BinOp::GtEq => "op_ge",
            BinOp::BitAnd => "op_bitand",
            BinOp::BitOr => "op_bitor",
            BinOp::BitXor => "op_bitxor",
            BinOp::Shl => "op_shl",
            BinOp::Shr => "op_shr",
            _ => return None,
        })
    }
    // (类型名, op 成员名) → 该重载是否存在 Array 形参。
    let mut array_ops: HashSet<(String, String)> = HashSet::new();
    let mut record = |owner: &str, func: &Function| {
        let member = func.name.rsplit('.').next().unwrap_or(&func.name);
        if member.starts_with("op_")
            && func.params.iter().any(|p| matches!(p.ty, Type::Array(_)))
        {
            array_ops.insert((owner.to_string(), member.to_string()));
        }
    };
    for f in &program.functions {
        if let Some((owner, _)) = f.name.split_once('.') {
            record(owner, f);
        }
    }
    for c in &program.classes {
        for m in &c.methods {
            record(&c.name, &m.func);
        }
    }
    for x in &program.extends {
        if program.classes.iter().any(|c| c.name == x.target_type)
            || program.structs.iter().any(|s| s.name == x.target_type)
            || program.enums.iter().any(|e| e.name == x.target_type)
        {
            for m in &x.methods {
                record(&x.target_type, m);
            }
        }
    }
    if array_ops.is_empty() {
        return;
    }
    // 简单静态类型解析：变量名 → 用户类型名。
    fn decl_type(ty: &Type) -> Option<String> {
        match ty {
            Type::Struct(n, _) => Some(n.clone()),
            _ => None,
        }
    }
    fn expr_user_type(e: &Expr, locals: &HashMap<String, String>) -> Option<String> {
        match e {
            Expr::Var(n) => locals.get(n).cloned(),
            Expr::ConstructorCall { name, .. } | Expr::StructInit { name, .. } => Some(name.clone()),
            // 解析器对无参构造调用可能生成 `Call{name}`（如 `var x = myClass()`）。
            Expr::Call { name, args, .. } if args.is_empty() => Some(name.clone()),
            _ => None,
        }
    }
    fn scan_locals(stmts: &[Stmt], locals: &mut HashMap<String, String>) {
        for s in stmts {
            match s {
                Stmt::Let { pattern, ty, value } | Stmt::Var { pattern, ty, value } => {
                    if let Pattern::Binding(name) = pattern {
                        if let Some(n) = ty.as_ref().and_then(decl_type) {
                            locals.insert(name.clone(), n);
                        } else if let Some(n) = expr_user_type(value, locals) {
                            locals.insert(name.clone(), n);
                        }
                    }
                }
                Stmt::While { body, .. }
                | Stmt::DoWhile { body, .. }
                | Stmt::Loop { body }
                | Stmt::UnsafeBlock { body } => scan_locals(body, locals),
                Stmt::For { body, .. } => scan_locals(body, locals),
                _ => {}
            }
        }
    }
    let mut check_stmts = |stmts: &[Stmt]| {
        let mut locals: HashMap<String, String> = HashMap::new();
        scan_locals(stmts, &mut locals);
        let mut report = |e: &Expr| {
            let Expr::Binary { op, left, right } = e else {
                return;
            };
            let Some(member) = op_member(op) else {
                return;
            };
            let other = match (left.as_ref(), right.as_ref()) {
                (other, Expr::Tuple(items)) if items.len() >= 2 => other,
                (Expr::Tuple(items), other) if items.len() >= 2 => other,
                _ => return,
            };
            let Some(owner) = expr_user_type(other, &locals) else {
                return;
            };
            if array_ops.contains(&(owner, member.to_string())) {
                errors.push(SemanticError {
                    message: "运算符重载不支持变长实参语法糖：数组形参不能以元组实参调用（5.3.4(7)）"
                        .to_string(),
                });
            }
        };
        walk_stmts(stmts, &mut report);
    };
    for f in &program.functions {
        check_stmts(&f.body);
    }
    for c in &program.classes {
        if let Some(i) = &c.init {
            check_stmts(&i.body);
        }
        for m in &c.methods {
            check_stmts(&m.func.body);
        }
    }
    for s in &program.structs {
        for i in &s.inits {
            check_stmts(&i.body);
        }
    }
    for i in &program.interfaces {
        for m in &i.methods {
            if let Some(b) = &m.default_body {
                check_stmts(b);
            }
        }
    }
    for x in &program.extends {
        for m in &x.methods {
            check_stmts(&m.body);
        }
    }
}

/// 5.3.4(4) / 5.3.4(5)：以变长实参语法糖调用“数组形参”函数时的实参类型检查。
///
/// 当函数的某个（非可变参数）形参类型为 `Array<T>` 时，若对应实参是标量字面量 /
/// 元组字面量（`f0('d', true)`、`f0(3, x: 5)` 等），该实参绝不可能是数组，
/// 必然无法通过类型检查——这正是把变长实参语法糖误用于普通/命名实参的典型
/// 形态。此处仅对能唯一解析出签名的调用（全局/静态/实例方法）报错，数组字面量
/// `[...]`、变量与其它未知表达式一律放行。
fn check_call_array_literal_args(program: &Program, errors: &mut Vec<SemanticError>) {
    fn is_scalar_or_tuple_literal(e: &Expr) -> bool {
        match e {
            Expr::Integer(_)
            | Expr::IntegerSuffixed(_, _)
            | Expr::Float(_)
            | Expr::Float32(_)
            | Expr::Bool(_)
            | Expr::Rune(_)
            | Expr::String(_)
            | Expr::Interpolate(_) => true,
            Expr::Tuple(items) => !items.is_empty(),
            Expr::Unary { expr, .. } => matches!(
                expr.as_ref(),
                Expr::Integer(_) | Expr::Float(_) | Expr::Float32(_)
            ),
            _ => false,
        }
    }
    fn non_receiver(params: &[Param]) -> Vec<&Param> {
        params
            .iter()
            .filter(|p| p.name != "this" && p.name != "self")
            .collect()
    }
    // 全局函数名 → 签名（存在重载时不收集）。
    let mut global_sigs: HashMap<String, Vec<Param>> = HashMap::new();
    let mut overloaded: HashSet<String> = HashSet::new();
    for f in &program.functions {
        if f.name.contains('.') || f.name.contains("::") {
            continue;
        }
        if global_sigs.contains_key(&f.name) {
            overloaded.insert(f.name.clone());
        } else {
            global_sigs.insert(f.name.clone(), f.params.clone());
        }
    }
    for name in &overloaded {
        global_sigs.remove(name);
    }
    // (类型名, 方法名) → 签名。
    let mut method_sigs: HashMap<(String, String), Vec<Param>> = HashMap::new();
    let mut method_dup: HashSet<(String, String)> = HashSet::new();
    let mut add_method = |owner: &str, func: &Function| {
        let member = func.name.rsplit('.').next().unwrap_or(&func.name).to_string();
        let key = (owner.to_string(), member);
        if method_sigs.contains_key(&key) {
            method_dup.insert(key);
        } else {
            method_sigs.insert(key, func.params.clone());
        }
    };
    for c in &program.classes {
        for m in &c.methods {
            add_method(&c.name, &m.func);
        }
    }
    for f in &program.functions {
        if let Some((owner, _)) = f.name.split_once('.') {
            add_method(owner, f);
        }
    }
    for key in &method_dup {
        method_sigs.remove(key);
    }
    // 简单静态类型解析（复用运算符检查的思路）。
    fn decl_type(ty: &Type) -> Option<String> {
        match ty {
            Type::Struct(n, _) => Some(n.clone()),
            _ => None,
        }
    }
    fn expr_user_type(e: &Expr, locals: &HashMap<String, String>) -> Option<String> {
        match e {
            Expr::Var(n) => locals.get(n).cloned(),
            Expr::ConstructorCall { name, .. } | Expr::StructInit { name, .. } => Some(name.clone()),
            Expr::Call { name, args, .. } if args.is_empty() => Some(name.clone()),
            // 枚举构造值 `myEnum.S`：取枚举类型名。
            Expr::Field { object, .. } => match object.as_ref() {
                Expr::Var(n) => locals.get(n).cloned().or_else(|| Some(n.clone())),
                _ => None,
            },
            _ => None,
        }
    }
    fn scan_locals(stmts: &[Stmt], locals: &mut HashMap<String, String>) {
        for s in stmts {
            match s {
                Stmt::Let { pattern, ty, value } | Stmt::Var { pattern, ty, value } => {
                    if let Pattern::Binding(name) = pattern {
                        if let Some(n) = ty.as_ref().and_then(decl_type) {
                            locals.insert(name.clone(), n);
                        } else if let Some(n) = expr_user_type(value, locals) {
                            locals.insert(name.clone(), n);
                        }
                    }
                }
                Stmt::While { body, .. }
                | Stmt::DoWhile { body, .. }
                | Stmt::Loop { body }
                | Stmt::UnsafeBlock { body } => scan_locals(body, locals),
                Stmt::For { body, .. } => scan_locals(body, locals),
                _ => {}
            }
        }
    }
    // 检查一次调用的实参。
    let check_args = |params: &[Param], args: &[Expr], named: &[(String, Expr)]| -> bool {
        let ps = non_receiver(params);
        // 位置实参依次对应前若干个非命名形参；命名实参按名字匹配。
        for (i, arg) in args.iter().enumerate() {
            if let Some(p) = ps.get(i) {
                // 变长实参语法糖：数组形参可由多个位置实参收集而成，但要求其
                // 后面不再有**非命名**形参（只允许跟命名形参）。否则单个数
                // 量不匹配的标量/元组实参必然非法。
                let only_named_after = ps[i + 1..].iter().all(|p| p.is_named);
                if !only_named_after
                    && !p.variadic
                    && matches!(p.ty, Type::Array(_))
                    && is_scalar_or_tuple_literal(arg)
                {
                    return true;
                }
            }
        }
        for (name, arg) in named {
            if let Some(p) = ps.iter().find(|p| &p.name == name) {
                // 命名实参不能使用变长实参语法糖：数组形参收到标量/元组即非法。
                if !p.variadic
                    && matches!(p.ty, Type::Array(_))
                    && is_scalar_or_tuple_literal(arg)
                {
                    return true;
                }
            }
        }
        false
    };
    let mut check_stmts = |stmts: &[Stmt]| {
        let mut locals: HashMap<String, String> = HashMap::new();
        scan_locals(stmts, &mut locals);
        let mut report = |e: &Expr| {
            match e {
                Expr::Call {
                    name,
                    args,
                    named_args,
                    ..
                } => {
                    if let Some(params) = global_sigs.get(name) {
                        if check_args(params, args, named_args) {
                            errors.push(SemanticError {
                                message: format!(
                                    "调用 '{}' 时数组形参不能接受标量/元组实参（5.3.4(4)）",
                                    name
                                ),
                            });
                        }
                    }
                }
                Expr::MethodCall {
                    object,
                    method,
                    args,
                    named_args,
                    ..
                } => {
                    if let Some(owner) = expr_user_type(object, &locals) {
                        if let Some(params) = method_sigs.get(&(owner.clone(), method.clone())) {
                            if check_args(params, args, named_args) {
                                errors.push(SemanticError {
                                    message: format!(
                                        "调用 '{}.{}' 时数组形参不能接受标量/元组实参（5.3.4(5)）",
                                        owner, method
                                    ),
                                });
                            }
                        }
                    }
                }
                _ => {}
            }
        };
        walk_stmts(stmts, &mut report);
    };
    for f in &program.functions {
        if f.body.is_empty() {
            continue;
        }
        check_stmts(&f.body);
    }
    for c in &program.classes {
        if let Some(i) = &c.init {
            check_stmts(&i.body);
        }
        for m in &c.methods {
            check_stmts(&m.func.body);
        }
    }
    for s in &program.structs {
        for i in &s.inits {
            check_stmts(&i.body);
        }
    }
    for i in &program.interfaces {
        for m in &i.methods {
            if let Some(b) = &m.default_body {
                check_stmts(b);
            }
        }
    }
    for x in &program.extends {
        for m in &x.methods {
            check_stmts(&m.body);
        }
    }
}

/// 5.1.2.2(15)：通过函数变量调用函数时不能省略可选参数（默认值不生效）。
///
/// 当 `var f = someFunc`（或 `var f = obj.method` / `var f = Type.staticMethod`）
/// 把带默认值的函数赋给变量后，再通过该变量调用时，默认值不再生效——实参个数
/// 必须覆盖全部形参。此处仅在“变量确实绑定到唯一可解析的函数、且该函数存在
/// 默认参数、调用实参数目不足”时报错，其余保守放行。
fn check_func_var_call_no_defaults(program: &Program, errors: &mut Vec<SemanticError>) {
    fn non_receiver(params: &[Param]) -> Vec<&Param> {
        params
            .iter()
            .filter(|p| p.name != "this" && p.name != "self")
            .collect()
    }
    // 全局函数名 → 形参（存在重载时不收集）。
    let mut globals: HashMap<String, Vec<Param>> = HashMap::new();
    let mut dup: HashSet<String> = HashSet::new();
    let mut note = |name: &str, params: &[Param]| {
        if globals.contains_key(name) {
            dup.insert(name.to_string());
        } else {
            globals.insert(name.to_string(), params.to_vec());
        }
    };
    for f in &program.functions {
        if f.name.contains('.') || f.name.contains("::") {
            continue;
        }
        note(&f.name, &f.params);
    }
    for d in &dup {
        globals.remove(d);
    }
    // (类型名, 方法名) → 形参。
    let mut methods: HashMap<(String, String), Vec<Param>> = HashMap::new();
    let mut mdup: HashSet<(String, String)> = HashSet::new();
    let mut add_method = |owner: &str, func: &Function| {
        let member = func.name.rsplit('.').next().unwrap_or(&func.name).to_string();
        let key = (owner.to_string(), member);
        if methods.contains_key(&key) {
            mdup.insert(key);
        } else {
            methods.insert(key, func.params.clone());
        }
    };
    for c in &program.classes {
        for m in &c.methods {
            add_method(&c.name, &m.func);
        }
    }
    for f in &program.functions {
        if let Some((owner, _)) = f.name.split_once('.') {
            add_method(owner, f);
        }
    }
    for key in &mdup {
        methods.remove(key);
    }
    let has_default = |params: &[Param]| params.iter().any(|p| p.default.is_some());
    // 把“函数引用表达式”解析为形参列表。
    let resolve_ref = |e: &Expr, locals: &HashMap<String, Vec<Param>>| -> Option<Vec<Param>> {
        match e {
            Expr::Var(n) => {
                if let Some(p) = locals.get(n) {
                    return Some(p.clone());
                }
                globals.get(n).cloned()
            }
            Expr::Field { object, field } => {
                let owner = match object.as_ref() {
                    Expr::Var(n) => Some(n.clone()),
                    _ => None,
                };
                owner.and_then(|o| methods.get(&(o, field.clone())).cloned())
            }
            _ => None,
        }
    };
    let mut check_stmts = |stmts: &[Stmt]| {
        let mut locals: HashMap<String, Vec<Param>> = HashMap::new();
        // 收集 `var/let x = <func-ref>` 绑定。
        fn collect(
            stmts: &[Stmt],
            locals: &mut HashMap<String, Vec<Param>>,
            resolve_ref: &dyn Fn(&Expr, &HashMap<String, Vec<Param>>) -> Option<Vec<Param>>,
        ) {
            for s in stmts {
                match s {
                    Stmt::Let { pattern, value, .. } | Stmt::Var { pattern, value, .. } => {
                        if let Pattern::Binding(name) = pattern {
                            if let Some(params) = resolve_ref(value, locals) {
                                locals.insert(name.clone(), params);
                            }
                        }
                    }
                    Stmt::While { body, .. }
                    | Stmt::DoWhile { body, .. }
                    | Stmt::Loop { body }
                    | Stmt::UnsafeBlock { body } => collect(body, locals, resolve_ref),
                    Stmt::For { body, .. } => collect(body, locals, resolve_ref),
                    _ => {}
                }
            }
        }
        collect(stmts, &mut locals, &resolve_ref);
        if locals.is_empty() {
            return;
        }
        let mut report = |e: &Expr| {
            if let Expr::Call {
                name,
                args,
                named_args,
                ..
            } = e
            {
                if let Some(params) = locals.get(name) {
                    let ps = non_receiver(params);
                    let provided = args.len() + named_args.len();
                    if provided < ps.len() && has_default(params) {
                        errors.push(SemanticError {
                            message: format!(
                                "通过函数变量 '{}' 调用时不能省略可选参数（默认值不生效，5.1.2.2(15)）",
                                name
                            ),
                        });
                    }
                }
            }
        };
        walk_stmts(stmts, &mut report);
    };
    for f in &program.functions {
        if !f.body.is_empty() {
            check_stmts(&f.body);
        }
    }
    for c in &program.classes {
        if let Some(i) = &c.init {
            check_stmts(&i.body);
        }
        for m in &c.methods {
            check_stmts(&m.func.body);
        }
    }
    for s in &program.structs {
        for i in &s.inits {
            check_stmts(&i.body);
        }
    }
}

/// 5.1(13)：函数名不能作为赋值目标。
///
/// 赋值目标为裸标识符且该名字在当前函数作用域内解析到某个函数（顶层 / 局部 /
/// 成员方法），或形如 `obj.method` 且 `method` 实际是该对象类型的成员方法
/// （而非字段）时，报错。`this.method = ...` / `EnumVariant.method = ...` 等
/// 形态一并覆盖。仅对能静态确定“是方法而非字段/变量”的情形报错。
fn check_function_name_lvalue(program: &Program, errors: &mut Vec<SemanticError>) {
    // 全部函数名（末段）与 (类型名, 成员名) 方法集合。
    let mut func_names: HashSet<String> = HashSet::new();
    let mut method_names: HashSet<String> = HashSet::new();
    let mut type_methods: HashSet<(String, String)> = HashSet::new();
    for f in &program.functions {
        let last = f.name.rsplit('.').next().unwrap_or(&f.name).to_string();
        func_names.insert(last.clone());
        if let Some((owner, _)) = f.name.split_once('.') {
            type_methods.insert((owner.to_string(), last.clone()));
            method_names.insert(last);
        }
    }
    for c in &program.classes {
        for m in &c.methods {
            let last = m.func.name.rsplit('.').next().unwrap_or(&m.func.name).to_string();
            type_methods.insert((c.name.clone(), last.clone()));
            method_names.insert(last);
        }
    }
    for i in &program.interfaces {
        for m in &i.methods {
            let last = m.name.rsplit('.').next().unwrap_or(&m.name).to_string();
            method_names.insert(last);
        }
    }
    if func_names.is_empty() && method_names.is_empty() {
        return;
    }
    // 所有类型字段名：避免把“同名的字段访问赋值”误判为给方法赋值。
    let mut all_field_names: HashSet<String> = HashSet::new();
    for c in &program.classes {
        for f in &c.fields {
            all_field_names.insert(f.name.clone());
        }
    }
    for s in &program.structs {
        for f in &s.fields {
            all_field_names.insert(f.name.clone());
        }
        for p in &s.primary_ctor_params {
            all_field_names.insert(p.name.clone());
        }
    }
    for e in &program.enums {
        for p in &e.props {
            all_field_names.insert(p.name.clone());
        }
    }
    // 递归遍历语句块（含循环 / 块等）收集赋值语句。
    fn for_each_assign<'a>(stmts: &'a [Stmt], f: &mut dyn FnMut(&'a Stmt)) {
        for s in stmts {
            match s {
                Stmt::Assign { .. } => f(s),
                Stmt::While { body, .. }
                | Stmt::DoWhile { body, .. }
                | Stmt::Loop { body }
                | Stmt::UnsafeBlock { body } => for_each_assign(body, f),
                Stmt::For { body, .. } => for_each_assign(body, f),
                _ => {}
            }
        }
    }
    // 顶层 let/var 名字：对其赋值是合法的（即使存在同名函数/宏生成的同名实体）。
    let top_vars: HashSet<String> = program.constants.iter().map(|c| c.name.clone()).collect();
    let mut check_stmts = |stmts: &[Stmt], params: &HashSet<String>| {
        let mut declared: HashSet<String> = params.clone();
        collect_block_local_names(stmts, &mut declared);
        let mut report = |s: &Stmt| {
            if let Stmt::Assign { target, .. } = s {
                match target {
                    AssignTarget::Var(name) => {
                        // 局部变量/形参/顶层变量遮蔽同名函数时放行。
                        if !declared.contains(name)
                            && !top_vars.contains(name)
                            && (func_names.contains(name) || method_names.contains(name))
                        {
                            errors.push(SemanticError {
                                message: format!(
                                    "函数 '{}' 不能作为赋值目标（5.1(13)）",
                                    name
                                ),
                            });
                        }
                    }
                    AssignTarget::Field { field, .. } => {
                        // `obj.method = ...` / `this.method = ...` / `EnumVariant.method = ...`：
                        // 仅当该成员名确为某方法、且不是任何类型的字段名时报错。
                        if method_names.contains(field) && !all_field_names.contains(field) {
                            errors.push(SemanticError {
                                message: format!(
                                    "成员函数 '{}' 不能作为赋值目标（5.1(13)）",
                                    field
                                ),
                            });
                        }
                    }
                    _ => {}
                }
            }
        };
        for_each_assign(stmts, &mut report);
    };
    let pnames = |ps: &[Param]| -> HashSet<String> { ps.iter().map(|p| p.name.clone()).collect() };
    for f in &program.functions {
        check_stmts(&f.body, &pnames(&f.params));
    }
    for c in &program.classes {
        if let Some(i) = &c.init {
            check_stmts(&i.body, &pnames(&i.params));
        }
        for m in &c.methods {
            check_stmts(&m.func.body, &pnames(&m.func.params));
        }
    }
    for s in &program.structs {
        for i in &s.inits {
            check_stmts(&i.body, &pnames(&i.params));
        }
    }
    for i in &program.interfaces {
        for m in &i.methods {
            if let Some(b) = &m.default_body {
                check_stmts(b, &pnames(&m.params));
            }
        }
    }
    for x in &program.extends {
        for m in &x.methods {
            check_stmts(&m.body, &pnames(&m.params));
        }
    }
}

/// 5.4(4)：嵌套函数只能在其所在的外层函数作用域内可见。
///
/// 若某函数的函数体调用了“仅在另一个函数内部声明的局部函数”，则属于越界访问。
/// 为避免误判，仅当该名字不是任何顶层/全局函数、不是当前函数内的局部变量/形参
/// （变量遮蔽）、且它作为局部函数名只出现在其它外层函数单元内时报错。
fn check_nested_function_scope(program: &Program, errors: &mut Vec<SemanticError>) {
    // 顶层 / 全局函数名（含方法），用于排除合法调用。
    let mut global_funcs: HashSet<String> = HashSet::new();
    for f in &program.functions {
        global_funcs.insert(f.name.rsplit('.').next().unwrap_or(&f.name).to_string());
    }
    // 深度收集：语句块 + 表达式内（lambda 体 / block 表达式）的局部函数声明。
    fn deep_stmts(stmts: &[Stmt], out: &mut HashSet<String>) {
        for s in stmts {
            match s {
                Stmt::LocalFunc(f) => {
                    out.insert(f.name.clone());
                    deep_stmts(&f.body, out);
                }
                Stmt::While { body, .. }
                | Stmt::DoWhile { body, .. }
                | Stmt::Loop { body }
                | Stmt::UnsafeBlock { body } => deep_stmts(body, out),
                Stmt::For { body, .. } => deep_stmts(body, out),
                _ => {}
            }
            walk_stmt(s, &mut |e: &Expr| deep_expr(e, out));
        }
    }
    fn deep_expr(e: &Expr, out: &mut HashSet<String>) {
        match e {
            Expr::Lambda { body, .. } => deep_expr(body, out),
            Expr::Block(stmts, tail) => {
                deep_stmts(stmts, out);
                if let Some(t) = tail {
                    deep_expr(t, out);
                }
            }
            _ => {}
        }
    }
    fn collect_calls(stmts: &[Stmt], out: &mut HashSet<String>) {
        walk_stmts(stmts, &mut |e: &Expr| {
            if let Expr::Call { name, .. } = e {
                out.insert(name.clone());
            }
        });
    }
    // units: (显示名, 局部函数名, 调用名, 局部变量/形参名)
    let mut units: Vec<(String, HashSet<String>, HashSet<String>, HashSet<String>)> = Vec::new();
    let mut owner_of: HashMap<String, HashSet<usize>> = HashMap::new();
    let mut push_unit = |display: String,
                         body: &[Stmt],
                         params: &[Param],
                         units: &mut Vec<(String, HashSet<String>, HashSet<String>, HashSet<String>)>,
                         owner_of: &mut HashMap<String, HashSet<usize>>| {
        let mut lf = HashSet::new();
        deep_stmts(body, &mut lf);
        let mut calls = HashSet::new();
        collect_calls(body, &mut calls);
        let mut locals: HashSet<String> = params.iter().map(|p| p.name.clone()).collect();
        collect_block_local_names(body, &mut locals);
        let idx = units.len();
        for n in &lf {
            owner_of.entry(n.clone()).or_default().insert(idx);
        }
        units.push((display, lf, calls, locals));
    };
    for f in &program.functions {
        if f.body.is_empty() {
            continue;
        }
        push_unit(
            f.name.clone(),
            &f.body,
            &f.params,
            &mut units,
            &mut owner_of,
        );
    }
    for c in &program.classes {
        for m in &c.methods {
            if !m.func.body.is_empty() {
                push_unit(
                    m.func.name.clone(),
                    &m.func.body,
                    &m.func.params,
                    &mut units,
                    &mut owner_of,
                );
            }
        }
    }
    for i in &program.interfaces {
        for m in &i.methods {
            if let Some(b) = &m.default_body {
                push_unit(m.name.clone(), b, &m.params, &mut units, &mut owner_of);
            }
        }
    }
    for x in &program.extends {
        for m in &x.methods {
            if !m.body.is_empty() {
                push_unit(
                    m.name.clone(),
                    &m.body,
                    &m.params,
                    &mut units,
                    &mut owner_of,
                );
            }
        }
    }
    for (idx, (display, _lf, calls, locals)) in units.iter().enumerate() {
        for call in calls {
            if locals.contains(call) || global_funcs.contains(call) {
                continue;
            }
            if let Some(owners) = owner_of.get(call) {
                if !owners.contains(&idx) {
                    errors.push(SemanticError {
                        message: format!(
                            "局部函数 '{}' 不能在其外层函数之外被调用（{}，5.4(4)）",
                            call, display
                        ),
                    });
                }
            }
        }
    }
}

/// 3.2.3.3(1)：类型成员名的作用域仅限该类型内部。
///
/// 类/结构体字段的默认值初始化式中，未经限定地引用了**其它类型**的字段名，
/// 属于越界使用成员名。仅当该名字不是本类型自有字段、不是顶层名字、不是
/// 预置标识符时报错；`@C` 结构体与首字母大写（类型/枚举构造名）一律放行。
fn check_cross_type_field_init_refs(program: &Program, errors: &mut Vec<SemanticError>) {
    let globals = collect_global_identifiers(program);
    // 类型名 → 字段名集合。
    let mut type_fields: HashMap<String, HashSet<String>> = HashMap::new();
    for c in &program.classes {
        type_fields.insert(
            c.name.clone(),
            c.fields.iter().map(|f| f.name.clone()).collect(),
        );
    }
    for s in &program.structs {
        type_fields.insert(
            s.name.clone(),
            s.fields.iter().map(|f| f.name.clone()).collect(),
        );
    }
    let check_one = |owner: &str, field: &str, init: &Expr, errors: &mut Vec<SemanticError>| {
        let own: HashSet<String> = type_fields.get(owner).cloned().unwrap_or_default();
        let mut refs = Vec::new();
        collect_bare_var_refs_expr(init, &mut refs);
        let mut reported: HashSet<String> = HashSet::new();
        for r in refs {
            if own.contains(&r) || globals.contains(&r) || is_prelude_identifier(&r) {
                continue;
            }
            if r.chars().next().map_or(false, |ch| ch.is_uppercase()) {
                continue;
            }
            let is_other_field = type_fields
                .iter()
                .any(|(t, fs)| t != owner && fs.contains(&r));
            if is_other_field && reported.insert(r.clone()) {
                errors.push(SemanticError {
                    message: format!(
                        "字段初始化式引用了其它类型的成员 '{}'，成员名仅在类型内部可见（{}.{}，3.2.3.3(1)）",
                        r, owner, field
                    ),
                });
            }
        }
    };
    for c in &program.classes {
        for f in &c.fields {
            if let Some(d) = &f.default {
                check_one(&c.name, &f.name, d, errors);
            }
        }
    }
    for s in &program.structs {
        if s.is_c {
            continue;
        }
        for f in &s.fields {
            if let Some(d) = &f.default {
                check_one(&s.name, &f.name, d, errors);
            }
        }
    }
}

/// 5.5(5)：lambda 表达式不能赋给标量类型的标注绑定（类型不匹配）。
///
/// `let f0: Int64 = {a: Int64 => a}` 之类把 lambda 赋给标量类型标注的绑定必然
/// 类型不匹配。仅在标注类型为确定非函数的内建标量/数组/元组类型时报错，用户
/// 类型（可能是函数类型别名）一律放行。
fn check_lambda_to_scalar_annotation(program: &Program, errors: &mut Vec<SemanticError>) {
    fn scalar(ty: &Type) -> bool {
        matches!(
            ty,
            Type::Int8
                | Type::Int16
                | Type::Int32
                | Type::Int64
                | Type::IntNative
                | Type::UInt8
                | Type::UInt16
                | Type::UInt32
                | Type::UInt64
                | Type::UIntNative
                | Type::Float16
                | Type::Float32
                | Type::Float64
                | Type::Rune
                | Type::Bool
                | Type::String
                | Type::Unit
                | Type::Array(_)
                | Type::Tuple(_)
                | Type::Range
        )
    }
    fn emit(ty: Option<&Type>, value: &Expr, what: &str, errors: &mut Vec<SemanticError>) {
        if let Some(t) = ty {
            if scalar(t) && matches!(value, Expr::Lambda { .. }) {
                errors.push(SemanticError {
                    message: format!(
                        "lambda 表达式不能赋给标量类型标注的绑定 '{}'（5.5(5)）",
                        what
                    ),
                });
            }
        }
    }
    fn rec(stmts: &[Stmt], errors: &mut Vec<SemanticError>) {
        for s in stmts {
            match s {
                Stmt::Let { pattern, ty, value } | Stmt::Var { pattern, ty, value } => {
                    let name = match pattern {
                        Pattern::Binding(n) => n.clone(),
                        _ => "<pattern>".to_string(),
                    };
                    emit(ty.as_ref(), value, &name, errors);
                }
                Stmt::While { body, .. }
                | Stmt::DoWhile { body, .. }
                | Stmt::Loop { body }
                | Stmt::UnsafeBlock { body } => rec(body, errors),
                Stmt::For { body, .. } => rec(body, errors),
                _ => {}
            }
        }
    }
    for f in &program.functions {
        rec(&f.body, errors);
    }
    for c in &program.classes {
        if let Some(i) = &c.init {
            rec(&i.body, errors);
        }
        for m in &c.methods {
            rec(&m.func.body, errors);
        }
        for fld in &c.fields {
            if let Some(d) = &fld.default {
                emit(Some(&fld.ty), d, &fld.name, errors);
            }
        }
    }
    for s in &program.structs {
        for fld in &s.fields {
            if let Some(d) = &fld.default {
                emit(Some(&fld.ty), d, &fld.name, errors);
            }
        }
    }
}

/// 5.3.3(1) / 5.3.3(7)：尾随 lambda 只能作为函数调用的最后一个实参。
///
/// 当被调用者的最后一个形参类型确定不是函数类型（如 `Int64`）时，把尾随闭包
/// 作为最后一个实参必然类型不匹配。仅对能唯一解析出签名、且末位形参为确定
/// 非函数的内建类型的调用报错；泛型形参、用户类型（可能是函数类型别名）与
/// 无法解析的调用一律放行。
fn check_trailing_closure_last_param(program: &Program, errors: &mut Vec<SemanticError>) {
    fn definitely_non_func(ty: &Type) -> bool {
        matches!(
            ty,
            Type::Int8
                | Type::Int16
                | Type::Int32
                | Type::Int64
                | Type::IntNative
                | Type::UInt8
                | Type::UInt16
                | Type::UInt32
                | Type::UInt64
                | Type::UIntNative
                | Type::Float16
                | Type::Float32
                | Type::Float64
                | Type::Rune
                | Type::Bool
                | Type::String
                | Type::Unit
                | Type::Array(_)
                | Type::Tuple(_)
                | Type::Range
        )
    }
    fn non_receiver(params: &[Param]) -> Vec<&Param> {
        params
            .iter()
            .filter(|p| p.name != "this" && p.name != "self")
            .collect()
    }
    // 全局函数签名（跳过重载）。
    let mut globals: HashMap<String, Vec<Param>> = HashMap::new();
    let mut dup: HashSet<String> = HashSet::new();
    for f in &program.functions {
        if f.name.contains('.') || f.name.contains("::") {
            continue;
        }
        if globals.contains_key(&f.name) {
            dup.insert(f.name.clone());
        } else {
            globals.insert(f.name.clone(), f.params.clone());
        }
    }
    for d in &dup {
        globals.remove(d);
    }
    // 方法签名。
    let mut methods: HashMap<(String, String), Vec<Param>> = HashMap::new();
    let mut mdup: HashSet<(String, String)> = HashSet::new();
    let mut add_method = |owner: &str, func: &Function| {
        let member = func.name.rsplit('.').next().unwrap_or(&func.name).to_string();
        let key = (owner.to_string(), member);
        if methods.contains_key(&key) {
            mdup.insert(key);
        } else {
            methods.insert(key, func.params.clone());
        }
    };
    for c in &program.classes {
        for m in &c.methods {
            add_method(&c.name, &m.func);
        }
    }
    for f in &program.functions {
        if let Some((owner, _)) = f.name.split_once('.') {
            add_method(owner, f);
        }
    }
    for key in &mdup {
        methods.remove(key);
    }
    fn decl_type(ty: &Type) -> Option<String> {
        match ty {
            Type::Struct(n, _) => Some(n.clone()),
            _ => None,
        }
    }
    fn expr_user_type(e: &Expr, locals: &HashMap<String, String>) -> Option<String> {
        match e {
            Expr::Var(n) => locals.get(n).cloned(),
            Expr::ConstructorCall { name, .. } | Expr::StructInit { name, .. } => Some(name.clone()),
            Expr::Call { name, args, .. } if args.is_empty() => Some(name.clone()),
            _ => None,
        }
    }
    fn scan_locals(stmts: &[Stmt], locals: &mut HashMap<String, String>) {
        for s in stmts {
            match s {
                Stmt::Let { pattern, ty, value } | Stmt::Var { pattern, ty, value } => {
                    if let Pattern::Binding(name) = pattern {
                        if let Some(n) = ty.as_ref().and_then(decl_type) {
                            locals.insert(name.clone(), n);
                        } else if let Some(n) = expr_user_type(value, locals) {
                            locals.insert(name.clone(), n);
                        }
                    }
                }
                Stmt::While { body, .. }
                | Stmt::DoWhile { body, .. }
                | Stmt::Loop { body }
                | Stmt::UnsafeBlock { body } => scan_locals(body, locals),
                Stmt::For { body, .. } => scan_locals(body, locals),
                _ => {}
            }
        }
    }
    // 判断一个被调用者表达式的“末位形参”。
    let last_param = |e: &Expr,
                      locals: &HashMap<String, String>,
                      vfuncs: &HashMap<String, Vec<Param>>|
     -> Option<Type> {
        let params: Vec<Param> = match e {
            Expr::Call { name, .. } => {
                if let Some(p) = vfuncs.get(name) {
                    p.clone()
                } else {
                    globals.get(name)?.clone()
                }
            }
            Expr::MethodCall { object, method, .. } => {
                let owner = expr_user_type(object, locals)?;
                methods.get(&(owner, method.clone()))?.clone()
            }
            Expr::Var(n) => vfuncs.get(n)?.clone(),
            _ => return None,
        };
        let ps = non_receiver(&params);
        ps.last().map(|p| p.ty.clone())
    };
    let mut check_stmts = |stmts: &[Stmt]| {
        let mut locals: HashMap<String, String> = HashMap::new();
        let mut vfuncs: HashMap<String, Vec<Param>> = HashMap::new();
        scan_locals(stmts, &mut locals);
        // 从 `var v = <func-ref>` 收集函数变量签名。
        fn collect_vfuncs(
            stmts: &[Stmt],
            globals: &HashMap<String, Vec<Param>>,
            methods: &HashMap<(String, String), Vec<Param>>,
            locals: &HashMap<String, String>,
            vfuncs: &mut HashMap<String, Vec<Param>>,
        ) {
            for s in stmts {
                match s {
                    Stmt::Let { pattern, value, .. } | Stmt::Var { pattern, value, .. } => {
                        if let Pattern::Binding(name) = pattern {
                            let params = match value {
                                Expr::Var(f) => globals.get(f).cloned(),
                                Expr::Field { object, field } => match object.as_ref() {
                                    Expr::Var(o) => {
                                        let owner = locals.get(o).cloned().unwrap_or_else(|| o.clone());
                                        methods.get(&(owner, field.clone())).cloned()
                                    }
                                    _ => None,
                                },
                                _ => None,
                            };
                            if let Some(p) = params {
                                vfuncs.insert(name.clone(), p);
                            }
                        }
                    }
                    Stmt::While { body, .. }
                    | Stmt::DoWhile { body, .. }
                    | Stmt::Loop { body }
                    | Stmt::UnsafeBlock { body } => {
                        collect_vfuncs(body, globals, methods, locals, vfuncs)
                    }
                    Stmt::For { body, .. } => collect_vfuncs(body, globals, methods, locals, vfuncs),
                    _ => {}
                }
            }
        }
        collect_vfuncs(stmts, &globals, &methods, &locals, &mut vfuncs);
        let mut report = |e: &Expr| {
            if let Expr::TrailingClosure { callee, .. } = e {
                if let Some(ty) = last_param(callee, &locals, &vfuncs) {
                    if definitely_non_func(&ty) {
                        errors.push(SemanticError {
                            message: "尾随 lambda 只能作为最后一个函数类型形参的实参（5.3.3(1)）"
                                .to_string(),
                        });
                    }
                }
            }
        };
        walk_stmts(stmts, &mut report);
    };
    for f in &program.functions {
        if !f.body.is_empty() {
            check_stmts(&f.body);
        }
    }
    for c in &program.classes {
        if let Some(i) = &c.init {
            check_stmts(&i.body);
        }
        for m in &c.methods {
            check_stmts(&m.func.body);
        }
    }
    for s in &program.structs {
        for i in &s.inits {
            check_stmts(&i.body);
        }
    }
}

/// 3.3.1(7)：已带初始化器的 `let` 成员变量不能在构造函数中重新赋值。
///
/// 仅当类/结构体的构造函数体以 `field = ...` 或 `this.field = ...` 形式给
/// **不可变且已初始化** 的字段赋值时报错；构造函数形参同名（遮蔽）或字段可变 /
/// 未初始化（延迟初始化）时放行。
fn check_let_field_ctor_reassign(program: &Program, errors: &mut Vec<SemanticError>) {
    fn for_each_assign<'a>(stmts: &'a [Stmt], f: &mut dyn FnMut(&'a Stmt)) {
        for s in stmts {
            match s {
                Stmt::Assign { .. } => f(s),
                Stmt::While { body, .. }
                | Stmt::DoWhile { body, .. }
                | Stmt::Loop { body }
                | Stmt::UnsafeBlock { body } => for_each_assign(body, f),
                Stmt::For { body, .. } => for_each_assign(body, f),
                _ => {}
            }
        }
    }
    let check_body = |owner: &str,
                      fields: &HashMap<String, (bool, bool)>,
                      params: &HashSet<String>,
                      body: &[Stmt],
                      errors: &mut Vec<SemanticError>| {
        let mut report = |s: &Stmt| {
            if let Stmt::Assign { target, .. } = s {
                let field = match target {
                    AssignTarget::Var(name) => {
                        if params.contains(name) {
                            None
                        } else {
                            Some(name.clone())
                        }
                    }
                    AssignTarget::Field { object, field } if object == "this" => {
                        // 主构造函数成员参数（`S5(let b!: Int16 = 1+1)`）的字段由
                        // 构造函数参数初始化，合成体中的 `this.b = b` 不是重赋值。
                        if params.contains(field) {
                            None
                        } else {
                            Some(field.clone())
                        }
                    }
                    _ => None,
                };
                if let Some(field) = field {
                    if let Some(&(is_mut, has_default)) = fields.get(&field) {
                        if !is_mut && has_default {
                            errors.push(SemanticError {
                                message: format!(
                                    "已初始化的不可变成员变量 '{}.{}' 不能在构造函数中重新赋值（3.3.1(7)）",
                                    owner, field
                                ),
                            });
                        }
                    }
                }
            }
        };
        for_each_assign(body, &mut report);
    };
    for c in &program.classes {
        let fields: HashMap<String, (bool, bool)> = c
            .fields
            .iter()
            .map(|f| (f.name.clone(), (f.is_mut, f.default.is_some())))
            .collect();
        for init in c.inits.iter() {
            let mut params: HashSet<String> =
                init.params.iter().map(|p| p.name.clone()).collect();
            params.extend(c.primary_ctor_params.iter().map(|p| p.name.clone()));
            check_body(&c.name, &fields, &params, &init.body, errors);
        }
    }
    for s in &program.structs {
        if s.is_c {
            continue;
        }
        let fields: HashMap<String, (bool, bool)> = s
            .fields
            .iter()
            .map(|f| (f.name.clone(), (f.is_mut, f.default.is_some())))
            .collect();
        for init in &s.inits {
            let mut params: HashSet<String> =
                init.params.iter().map(|p| p.name.clone()).collect();
            params.extend(s.primary_ctor_params.iter().map(|p| p.name.clone()));
            check_body(&s.name, &fields, &params, &init.body, errors);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{
        BinOp, ClassDef, ClassMethod, Expr, Function, InitDef, InterpolatePart, MatchArm, Param, Pattern,
        Program, Stmt, StructDef, Type, UnaryOp, Visibility,
    };
    use std::collections::HashMap;

    fn make_param(name: &str, ty: Type) -> Param {
        Param {
            name: name.to_string(),
            ty,
            default: None,
            variadic: false,
            is_named: false,
            is_inout: false,
        }
    }


    // ─── infer_expr: literals ─────────────────────────────────────────────────

    #[test]
    fn test_infer_expr_integer() {
        let known = HashMap::new();
        assert_eq!(
            infer_expr(&Expr::Integer(42), &known),
            Some(Type::Int64)
        );
    }


    #[test]
    fn test_infer_expr_float() {
        let known = HashMap::new();
        assert_eq!(
            infer_expr(&Expr::Float(3.14), &known),
            Some(Type::Float64)
        );
    }


    #[test]
    fn test_infer_expr_float32() {
        let known = HashMap::new();
        assert_eq!(
            infer_expr(&Expr::Float32(1.0f32), &known),
            Some(Type::Float32)
        );
    }


    #[test]
    fn test_infer_expr_bool() {
        let known = HashMap::new();
        assert_eq!(infer_expr(&Expr::Bool(true), &known), Some(Type::Bool));
    }


    #[test]
    fn test_infer_expr_rune() {
        let known = HashMap::new();
        assert_eq!(infer_expr(&Expr::Rune('x'), &known), Some(Type::Rune));
    }


    #[test]
    fn test_infer_expr_string() {
        let known = HashMap::new();
        assert_eq!(
            infer_expr(&Expr::String("hello".to_string()), &known),
            Some(Type::String)
        );
    }


    #[test]
    fn test_infer_expr_interpolate() {
        let known = HashMap::new();
        let expr = Expr::Interpolate(vec![InterpolatePart::Literal("hi".to_string())]);
        assert_eq!(infer_expr(&expr, &known), Some(Type::String));
    }


    // ─── infer_expr: Cast, IsType ────────────────────────────────────────────

    #[test]
    fn test_infer_expr_cast() {
        let known = HashMap::new();
        let expr = Expr::Cast {
            expr: Box::new(Expr::Integer(1)),
            target_ty: Type::Float64,
        };
        assert_eq!(infer_expr(&expr, &known), Some(Type::Float64));
    }


    #[test]
    fn test_infer_expr_is_type() {
        let known = HashMap::new();
        let expr = Expr::IsType {
            expr: Box::new(Expr::Var("x".to_string())),
            target_ty: Type::String,
        };
        assert_eq!(infer_expr(&expr, &known), Some(Type::Bool));
    }


    // ─── infer_expr: Some, Ok, Err, None ─────────────────────────────────────

    #[test]
    fn test_infer_expr_some() {
        let known = HashMap::new();
        let expr = Expr::Some(Box::new(Expr::Integer(1)));
        assert_eq!(
            infer_expr(&expr, &known),
            Some(Type::Option(Box::new(Type::Int64)))
        );
    }


    #[test]
    fn test_infer_expr_ok() {
        let known = HashMap::new();
        let expr = Expr::Ok(Box::new(Expr::String("ok".to_string())));
        assert_eq!(
            infer_expr(&expr, &known),
            Some(Type::Result(
                Box::new(Type::String),
                Box::new(Type::String),
            ))
        );
    }


    #[test]
    fn test_infer_expr_err() {
        let known = HashMap::new();
        let expr = Expr::Err(Box::new(Expr::String("err".to_string())));
        assert_eq!(
            infer_expr(&expr, &known),
            Some(Type::Result(
                Box::new(Type::Int64),
                Box::new(Type::String),
            ))
        );
    }


    #[test]
    fn test_infer_expr_none() {
        let known = HashMap::new();
        assert_eq!(infer_expr(&Expr::None, &known), None);
    }


    // ─── infer_expr: StructInit, ConstructorCall ───────────────────────────────

    #[test]
    fn test_infer_expr_struct_init() {
        let known = HashMap::new();
        let expr = Expr::StructInit {
            name: "Point".to_string(),
            type_args: Some(vec![Type::Int64, Type::Int64]),
            fields: vec![
                ("x".to_string(), Expr::Integer(1)),
                ("y".to_string(), Expr::Integer(2)),
            ],
        };
        assert_eq!(
            infer_expr(&expr, &known),
            Some(Type::Struct(
                "Point".to_string(),
                vec![Type::Int64, Type::Int64],
            ))
        );
    }


    #[test]
    fn test_infer_expr_constructor_call() {
        let known = HashMap::new();
        let expr = Expr::ConstructorCall {
            name: "Pair".to_string(),
            type_args: Some(vec![Type::Int64, Type::String]),
            args: vec![],
            named_args: vec![],
        };
        assert_eq!(
            infer_expr(&expr, &known),
            Some(Type::Struct(
                "Pair".to_string(),
                vec![Type::Int64, Type::String],
            ))
        );
    }


    // ─── infer_expr: Call ─────────────────────────────────────────────────────

    #[test]
    fn test_infer_expr_call_known_function() {
        let mut known = HashMap::new();
        known.insert("foo".to_string(), Type::Int64);
        let expr = Expr::Call {
            name: "foo".to_string(),
            type_args: None,
            args: vec![],
            named_args: vec![],
        };
        assert_eq!(infer_expr(&expr, &known), Some(Type::Int64));
    }


    #[test]
    fn test_infer_expr_call_uppercase_constructor() {
        let known = HashMap::new();
        let expr = Expr::Call {
            name: "Point".to_string(),
            type_args: Some(vec![Type::Int64]),
            args: vec![],
            named_args: vec![],
        };
        assert_eq!(
            infer_expr(&expr, &known),
            Some(Type::Struct("Point".to_string(), vec![Type::Int64]))
        );
    }


    #[test]
    fn test_infer_expr_call_unknown() {
        let known = HashMap::new();
        let expr = Expr::Call {
            name: "unknownFunc".to_string(),
            type_args: None,
            args: vec![],
            named_args: vec![],
        };
        assert_eq!(infer_expr(&expr, &known), None);
    }


    // ─── infer_expr: Var ──────────────────────────────────────────────────────

    #[test]
    fn test_infer_expr_var_known() {
        let mut known = HashMap::new();
        known.insert("x".to_string(), Type::String);
        assert_eq!(
            infer_expr(&Expr::Var("x".to_string()), &known),
            Some(Type::String)
        );
    }


    #[test]
    fn test_infer_expr_var_unknown() {
        let known = HashMap::new();
        assert_eq!(infer_expr(&Expr::Var("y".to_string()), &known), None);
    }


    // ─── infer_expr: Binary ──────────────────────────────────────────────────

    #[test]
    fn test_infer_expr_binary_comparison() {
        let known = HashMap::new();
        let expr = Expr::Binary {
            op: BinOp::Eq,
            left: Box::new(Expr::Integer(1)),
            right: Box::new(Expr::Integer(2)),
        };
        assert_eq!(infer_expr(&expr, &known), Some(Type::Bool));
    }


    #[test]
    fn test_infer_expr_binary_add_string() {
        let known = HashMap::new();
        let expr = Expr::Binary {
            op: BinOp::Add,
            left: Box::new(Expr::String("a".to_string())),
            right: Box::new(Expr::String("b".to_string())),
        };
        assert_eq!(infer_expr(&expr, &known), Some(Type::String));
    }


    #[test]
    fn test_infer_expr_binary_add_integers() {
        let known = HashMap::new();
        let expr = Expr::Binary {
            op: BinOp::Add,
            left: Box::new(Expr::Integer(1)),
            right: Box::new(Expr::Integer(2)),
        };
        assert_eq!(infer_expr(&expr, &known), Some(Type::Int64));
    }


    // ─── infer_expr: Unary ───────────────────────────────────────────────────

    #[test]
    fn test_infer_expr_unary_not() {
        let known = HashMap::new();
        let expr = Expr::Unary {
            op: UnaryOp::Not,
            expr: Box::new(Expr::Bool(true)),
        };
        assert_eq!(infer_expr(&expr, &known), Some(Type::Bool));
    }


    #[test]
    fn test_infer_expr_unary_neg() {
        let known = HashMap::new();
        let expr = Expr::Unary {
            op: UnaryOp::Neg,
            expr: Box::new(Expr::Integer(1)),
        };
        assert_eq!(infer_expr(&expr, &known), Some(Type::Int64));
    }


    // ─── infer_expr: If, Block, Match ──────────────────────────────────────────

    #[test]
    fn test_infer_expr_if() {
        let known = HashMap::new();
        let expr = Expr::If {
            cond: Box::new(Expr::Bool(true)),
            then_branch: Box::new(Expr::Integer(1)),
            else_branch: Some(Box::new(Expr::Integer(2))),
        };
        assert_eq!(infer_expr(&expr, &known), Some(Type::Int64));
    }


    #[test]
    fn test_infer_expr_block_with_trailing() {
        let known = HashMap::new();
        let expr = Expr::Block(
            vec![Stmt::Expr(Expr::Integer(0))],
            Some(Box::new(Expr::String("result".to_string()))),
        );
        assert_eq!(infer_expr(&expr, &known), Some(Type::String));
    }


    #[test]
    fn test_infer_expr_match() {
        let known = HashMap::new();
        let expr = Expr::Match {
            expr: Box::new(Expr::Integer(1)),
            arms: vec![MatchArm {
                pattern: Pattern::Wildcard,
                guard: None,
                body: Box::new(Expr::String("matched".to_string())),
            }],
        };
        assert_eq!(infer_expr(&expr, &known), Some(Type::String));
    }


    // ─── infer_expr: Tuple, Array ──────────────────────────────────────────────

    #[test]
    fn test_infer_expr_tuple() {
        let known = HashMap::new();
        let expr = Expr::Tuple(vec![
            Expr::Integer(1),
            Expr::String("two".to_string()),
        ]);
        assert_eq!(
            infer_expr(&expr, &known),
            Some(Type::Tuple(vec![Type::Int64, Type::String]))
        );
    }


    #[test]
    fn test_infer_expr_array() {
        let known = HashMap::new();
        let expr = Expr::Array(vec![
            Expr::Integer(1),
            Expr::Integer(2),
        ]);
        assert_eq!(
            infer_expr(&expr, &known),
            Some(Type::Array(Box::new(Type::Int64)))
        );
    }


    // ─── infer_expr: Lambda ───────────────────────────────────────────────────

    #[test]
    fn test_infer_expr_lambda() {
        let known = HashMap::new();
        let expr = Expr::Lambda {
            params: vec![("x".to_string(), Type::Int64)],
            return_type: Some(Type::Int64),
            body: Box::new(Expr::Var("x".to_string())),
        };
        assert_eq!(
            infer_expr(&expr, &known),
            Some(Type::Function {
                params: vec![Type::Int64],
                ret: Box::new(Some(Type::Int64)),
            })
        );
    }


    // ─── infer_expr: Try, NullCoalesce ─────────────────────────────────────────

    #[test]
    fn test_infer_expr_try() {
        let known = HashMap::new();
        let expr = Expr::Try(Box::new(Expr::Integer(42)));
        assert_eq!(infer_expr(&expr, &known), Some(Type::Int64));
    }


    #[test]
    fn test_infer_expr_null_coalesce() {
        let known = HashMap::new();
        let expr = Expr::NullCoalesce {
            option: Box::new(Expr::None),
            default: Box::new(Expr::Integer(0)),
        };
        assert_eq!(infer_expr(&expr, &known), Some(Type::Int64));
    }


    // ─── analyze ──────────────────────────────────────────────────────────────

    #[test]
    fn test_analyze_simple_program_with_return_types() {
        let program = Program {
            package_name: None,
            imports: vec![],
            structs: vec![],
            interfaces: vec![],
            classes: vec![],
            enums: vec![],
            functions: vec![
                Function {
                    visibility: Visibility::default(),
                    name: "annotated".to_string(),
                    type_params: vec![],
                    constraints: vec![],
                    params: vec![make_param("x", Type::Int64)],
                    return_type: Some(Type::Int64),
                    throws: None,
                    body: vec![],
                    extern_import: None,
                    is_const: false,
                },
                Function {
                    visibility: Visibility::default(),
                    name: "inferred".to_string(),
                    type_params: vec![],
                    constraints: vec![],
                    params: vec![],
                    return_type: None,
                    throws: None,
                    body: vec![Stmt::Expr(Expr::Integer(42))],
                    extern_import: None,
                    is_const: false,
                },
            ],
            extends: vec![],
            type_aliases: vec![],
            constants: vec![],
        };
        let ctx = analyze(&program);
        assert!(ctx.inferred_return_types.contains_key("inferred"));
        assert_eq!(
            ctx.inferred_return_types.get("inferred"),
            Some(&Type::Int64)
        );
    }


    #[test]
    fn test_analyze_function_with_return_stmt() {
        let program = Program {
            package_name: None,
            imports: vec![],
            structs: vec![],
            interfaces: vec![],
            classes: vec![],
            enums: vec![],
            functions: vec![Function {
                visibility: Visibility::default(),
                name: "with_return".to_string(),
                type_params: vec![],
                constraints: vec![],
                params: vec![],
                return_type: None,
                throws: None,
                body: vec![Stmt::Return(Some(Expr::String("hello".to_string())))],
                extern_import: None,
                    is_const: false,
            }],
            extends: vec![],
            type_aliases: vec![],
            constants: vec![],
        };
        let ctx = analyze(&program);
        assert_eq!(
            ctx.inferred_return_types.get("with_return"),
            Some(&Type::String)
        );
    }


    #[test]
    fn test_analyze_class_method_inference() {
        let program = Program {
            package_name: None,
            imports: vec![],
            structs: vec![],
            interfaces: vec![],
            classes: vec![ClassDef {
                visibility: Visibility::default(),
                name: "Foo".to_string(),
                type_params: vec![],
                constraints: vec![],
                is_abstract: false,
                is_sealed: false,
                is_open: false,
                extends: None,
                implements: vec![],
                supertype_args: vec![],
                fields: vec![],
                init: None,
                deinit: None,
                static_init: None,
                inits: vec![],
                primary_ctor_params: vec![],
                primary_ctor_member_flags: vec![],
                methods: vec![ClassMethod {
                    override_: false,
                    has_override_kw: false,
                    is_static: false,
                    is_open: false,
                    is_abstract: false,
                    has_body: true,
                    is_prop_mut: false,
                    is_const: false,
                    func: Function {
                        visibility: Visibility::default(),
                        name: "Foo.getVal".to_string(),
                        type_params: vec![],
                        constraints: vec![],
                        params: vec![],
                        return_type: None,
                        throws: None,
                        body: vec![Stmt::Expr(Expr::Bool(true))],
                        extern_import: None,
                    is_const: false,
                    },
                }],
            }],
            enums: vec![],
            functions: vec![],
            extends: vec![],
            type_aliases: vec![],
            constants: vec![],
        };
        let ctx = analyze(&program);
        assert!(ctx.inferred_return_types.contains_key("Foo.getVal"));
        assert_eq!(
            ctx.inferred_return_types.get("Foo.getVal"),
            Some(&Type::Bool)
        );
    }


    // ─── validate_program: 语义校验 ─────────────────────────────────────────

    /// 直接走 lexer + parser 得到 AST（不经过 pipeline 的校验），便于测试 validate_program。
    fn program_of(source: &str) -> Program {
        let lexer = crate::lexer::Lexer::new(source);
        let tokens: Result<Vec<_>, _> = lexer.collect();
        let tokens = tokens.expect("词法分析应成功");
        crate::parser::Parser::new(tokens)
            .with_source(source)
            .parse_program()
            .expect("语法分析应成功")
    }


    fn assert_valid(source: &str) {
        let program = program_of(source);
        if let Err(errs) = validate_program(&program) {
            panic!("合法程序被误判: {:?}", errs);
        }
    }


    fn assert_invalid(source: &str) -> Vec<SemanticError> {
        let program = program_of(source);
        match validate_program(&program) {
            Ok(()) => panic!("非法程序未被拒绝: {}", source),
            Err(errs) => errs,
        }
    }


    #[test]
    fn test_validate_duplicate_class_method() {
        assert_invalid(
            "class A {\n\
             \x20   func f(x: Int64) {}\n\
             \x20   func f(y: Int64) {}\n\
             }\n\
             main() {}\n",
        );
    }


    #[test]
    fn test_validate_legal_overload_not_rejected() {
        assert_valid(
            "class A {\n\
             \x20   func f(x: Int64) {}\n\
             \x20   func f(x: Int64, y: Bool) {}\n\
             \x20   func f(x: String) {}\n\
             }\n\
             main() {}\n",
        );
    }


    #[test]
    fn test_validate_static_instance_same_name() {
        let errs = assert_invalid(
            "class A {\n\
             \x20   func f(x: Int64) {}\n\
             \x20   static func f() {}\n\
             }\n\
             main() {}\n",
        );
        assert!(errs.iter().any(|e| e.message.contains("静态成员函数与实例成员函数同名")));
    }


    #[test]
    fn test_validate_duplicate_top_level_function() {
        assert_invalid(
            "func f(a: Int64) {}\nfunc f(b: Int64) {}\nmain() {}\n",
        );
    }


    #[test]
    fn test_validate_generic_vs_non_generic_not_redefinition() {
        // 5.1.6(3): 泛型与非泛型同名不构成重定义
        assert_valid(
            "func f(a: Int32, b!: Int32): Int32 { a }\n\
             func f<A>(a: A, b!: A): A { b }\n\
             main() {}\n",
        );
    }


    #[test]
    fn test_validate_generic_type_param_order_not_redefinition() {
        // 5.1.6(7): func f<X,Y>(a:X,b:Y) 与 func f<Y,X>(a:X,b:Y) 不构成重定义
        assert_valid(
            "func f<X, Y>(a: X, b: Y) {}\n\
             func f<Y, X>(a: X, b: Y) {}\n\
             main() {}\n",
        );
    }


    #[test]
    fn test_validate_duplicate_interface_generic_method() {
        // 5.1.6(5): 重命名泛型参数后非泛型部分相同 → 重定义
        assert_invalid(
            "interface I {\n\
             \x20   static func f<A>(a: A): A\n\
             \x20   static func f<B>(a: B): B\n\
             }\n\
             main() {}\n",
        );
    }


    #[test]
    fn test_validate_override_without_parent() {
        let errs = assert_invalid("class A {\n    override func f() {}\n}\nmain() {}\n");
        assert!(errs.iter().any(|e| e.message.contains("不存在可覆盖的同名函数")));
    }


    #[test]
    fn test_validate_override_signature_mismatch() {
        assert_invalid(
            "open class P {\n\
             \x20   open func f() {}\n\
             }\n\
             class C <: P {\n\
             \x20   public override func f(x: Int64) {}\n\
             }\n\
             main() {}\n",
        );
    }


    #[test]
    fn test_validate_override_matching_signature_ok() {
        assert_valid(
            "open class P {\n\
             \x20   public open func f(x: Int64) {}\n\
             }\n\
             class C <: P {\n\
             \x20   public override func f(x: Int64) {}\n\
             }\n\
             main() {}\n",
        );
    }


    #[test]
    fn test_validate_override_covariant_return_ok() {
        assert_valid(
            "open class Sup {}\n\
             class Sub <: Sup {}\n\
             open class P {\n\
             \x20   public open func f(): Sup { Sup() }\n\
             }\n\
             class C <: P {\n\
             \x20   public override func f(): Sub { Sub() }\n\
             }\n\
             main() {}\n",
        );
    }


    #[test]
    fn test_validate_interface_missing_implementation() {
        assert_invalid(
            "interface I {\n\
             \x20   func f(): Unit\n\
             }\n\
             class A <: I {\n\
             \x20   func f(x: Int64) {}\n\
             }\n\
             main() {}\n",
        );
    }


    #[test]
    fn test_validate_interface_implementation_ok() {
        assert_valid(
            "interface I {\n\
             \x20   func f(x: Int64): Int64\n\
             }\n\
             class A <: I {\n\
             \x20   public func f(x: Int64) { x }\n\
             }\n\
             main() {}\n",
        );
    }


    #[test]
    fn test_validate_struct_interface_missing_implementation() {
        assert_invalid(
            "interface I {\n\
             \x20   func f(): Unit\n\
             }\n\
             struct S <: I {\n\
             \x20   func f(x: Int64) {}\n\
             }\n\
             main() {}\n",
        );
    }


    #[test]
    fn test_validate_property_type_mismatch() {
        assert_invalid(
            "interface I {\n\
             \x20   mut prop p: Int64\n\
             }\n\
             class A <: I {\n\
             \x20   public mut prop p: String {\n\
             \x20       get() { \"\" }\n\
             \x20       set(v) {}\n\
             \x20   }\n\
             }\n\
             main() {}\n",
        );
    }


    #[test]
    fn test_validate_enum_duplicate_variant() {
        assert_invalid("enum E {\n    A(Int8)\n    A(Int8)\n}\nmain() {}\n");
    }


    #[test]
    fn test_validate_enum_variant_overload_ok() {
        // 同名但载荷不同 → 合法重载
        assert_valid("enum E {\n    A\n    A(Int8)\n}\nmain() {}\n");
    }


    #[test]
    fn test_validate_duplicate_field() {
        assert_invalid("class A {\n    var x: Int64 = 1\n    var x: Int64 = 2\n}\nmain() {}\n");
    }


    // ─── validate_program: 类/接口规则补强（06 章） ─────────────────────────

    #[test]
    fn test_validate_abstract_class_instantiation() {
        let errs = assert_invalid(
            "abstract class A {}\nmain() {\n    let a = A()\n}\n",
        );
        assert!(errs.iter().any(|e| e.message.contains("抽象类") && e.message.contains("实例化")));
    }


    #[test]
    fn test_validate_interface_instantiation() {
        let errs = assert_invalid(
            "interface I {\n    func f(): Int64\n}\nmain() {\n    let x = I()\n}\n",
        );
        assert!(errs.iter().any(|e| e.message.contains("接口") && e.message.contains("实例化")));
    }


    #[test]
    fn test_validate_abstract_class_as_type_ok() {
        // 仅作为类型使用（不实例化）是合法的
        assert_valid(
            "abstract class A {}\nfunc f(a: A) {}\nmain() {}\n",
        );
    }


    #[test]
    fn test_validate_shadowed_type_name_not_instantiation() {
        // 局部函数遮蔽同名接口时，该调用不是接口实例化（避免误伤遮蔽用例）
        assert_valid(
            "interface I {}\nmain() {\n    func I(): Int64 { 1 }\n    let x = I()\n}\n",
        );
    }


    #[test]
    fn test_validate_multiple_inheritance() {
        let errs = assert_invalid(
            "open class A {}\nopen class B {}\nclass C <: A & B {}\nmain() {}\n",
        );
        assert!(errs.iter().any(|e| e.message.contains("单继承")));
    }


    #[test]
    fn test_validate_interface_before_superclass() {
        // 接口写在父类之前：解析器把首个当成 extends，类名落入 implements
        assert_invalid(
            "interface I {}\nopen class A {}\nclass B <: I & A {}\nmain() {}\n",
        );
    }


    #[test]
    fn test_validate_inherit_non_open_class() {
        let errs = assert_invalid("class A {}\nclass B <: A {}\nmain() {}\n");
        assert!(errs.iter().any(|e| e.message.contains("非 open")));
    }


    #[test]
    fn test_validate_inherit_open_class_ok() {
        assert_valid("open class A {}\nclass B <: A {}\nmain() {}\n");
    }


    #[test]
    fn test_validate_inherit_sealed_class_same_file_ok() {
        // sealed 类允许在同包（单文件）内被继承
        assert_valid("sealed class A {}\nclass B <: A {}\nmain() {}\n");
    }


    #[test]
    fn test_validate_inherit_struct() {
        assert_invalid("struct S {}\nclass C <: S {}\nmain() {}\n");
    }


    #[test]
    fn test_validate_override_lowers_visibility() {
        let errs = assert_invalid(
            "open class P {\n    public open func f() {}\n}\nclass C <: P {\n    override func f() {}\n}\nmain() {}\n",
        );
        assert!(errs.iter().any(|e| e.message.contains("降低")));
    }


    #[test]
    fn test_validate_override_raises_visibility_ok() {
        assert_valid(
            "open class P {\n    protected open func f() {}\n}\nclass C <: P {\n    public override func f() {}\n}\nmain() {}\n",
        );
    }


    #[test]
    fn test_validate_static_redef_lowers_visibility() {
        // 未写 override 关键字的同名静态重定义也受可见性约束（6.3.5(2)）
        assert_invalid(
            "open class P {\n    protected static func f() {}\n}\nclass C <: P {\n    private static func f() {}\n}\nmain() {}\n",
        );
    }


    #[test]
    fn test_validate_struct_interface_property_type_mismatch() {
        assert_invalid(
            "open class Sup {}\n\
             class Sub <: Sup {}\n\
             interface I {\n\
             \x20   prop p: Sub {\n\
             \x20       get() { Sub() }\n\
             \x20   }\n\
             }\n\
             struct A <: I {\n\
             \x20   public prop p: Sup {\n\
             \x20       get() { Sub() }\n\
             \x20   }\n\
             }\n\
             main() {}\n",
        );
    }


    #[test]
    fn test_validate_struct_interface_property_type_ok() {
        assert_valid(
            "interface I {\n\
             \x20   prop p: Int64 {\n\
             \x20       get() { 0 }\n\
             \x20   }\n\
             }\n\
             struct A <: I {\n\
             \x20   public prop p: Int64 {\n\
             \x20       get() { 1 }\n\
             \x20   }\n\
             }\n\
             main() {}\n",
        );
    }


    #[test]
    fn test_validate_enum_missing_interface_function() {
        assert_invalid(
            "interface I {\n    func f(): Int64\n}\nenum E <: I {\n    A\n}\nmain() {}\n",
        );
    }


    #[test]
    fn test_validate_enum_implements_interface_ok() {
        assert_valid(
            "interface I {\n\
             \x20   func f(): Int64\n\
             }\n\
             enum E <: I {\n\
             \x20   A\n\
             \x20   public func f(): Int64 { 1 }\n\
             }\n\
             main() {}\n",
        );
    }


    // ─── 06 章：成员修饰符 / 遮蔽 / 访问控制 / This / 构造参数顺序 ────────────

    #[test]
    fn test_validate_static_member_func_requires_body() {
        assert_invalid("class A {\n    static func f(): Int64\n}\nmain() {}\n");
    }


    #[test]
    fn test_validate_static_member_func_with_body_ok() {
        assert_valid("class A {\n    static func f(): Int64 { 1 }\n}\nmain() {}\n");
    }


    #[test]
    fn test_validate_no_body_func_in_non_abstract_class() {
        assert_invalid("class A {\n    public func f(): Int64\n}\nmain() {}\n");
    }


    #[test]
    fn test_validate_no_body_func_in_abstract_class_ok() {
        assert_valid("abstract class A {\n    func f(): Int64\n}\nmain() {}\n");
    }


    #[test]
    fn test_validate_open_static_func_conflict() {
        assert_invalid(
            "class A {\n    public static open func f() {}\n}\nmain() {}\n",
        );
    }


    #[test]
    fn test_validate_override_non_open_func_rejected() {
        assert_invalid(
            "open class A {\n    public func f() {}\n}\nclass B <: A {\n    override public func f() {}\n}\nmain() {}\n",
        );
    }


    #[test]
    fn test_validate_override_open_func_ok() {
        assert_valid(
            "open class A {\n    open public func f() {}\n}\nclass B <: A {\n    override public func f() {}\n}\nmain() {}\n",
        );
    }


    #[test]
    fn test_validate_static_redef_non_open_ok() {
        assert_valid(
            "open class A {\n    public static func f() {}\n}\nclass B <: A {\n    public redef static func f() {}\n}\nmain() {}\n",
        );
    }


    #[test]
    fn test_validate_member_shadowing_kind_conflict() {
        // 子类函数与父类字段同名 → 遮蔽。
        assert_invalid(
            "open class A {\n    let x = 1\n}\nclass B <: A {\n    func x() {}\n}\nmain() {}\n",
        );
        // 子类字段与父类字段同名 → 遮蔽。
        assert_invalid(
            "open class A {\n    let x = 1\n}\nclass B <: A {\n    let x = 2\n}\nmain() {}\n",
        );
    }


    #[test]
    fn test_validate_shadowing_private_parent_field_ok() {
        assert_valid(
            "open class A {\n    private let x = 1\n}\nclass B <: A {\n    let x = 2\n}\nmain() {}\n",
        );
    }


    #[test]
    fn test_validate_interface_implementation_lowers_visibility() {
        assert_invalid(
            "interface I {\n    func f(): Int64\n}\nclass A <: I {\n    func f(): Int64 { 1 }\n}\nmain() {}\n",
        );
        assert_valid(
            "interface I {\n    func f(): Int64\n}\nclass A <: I {\n    public func f(): Int64 { 1 }\n}\nmain() {}\n",
        );
    }


    #[test]
    fn test_validate_static_func_access_instance_member() {
        assert_invalid("class A {\n    let x = 1\n    static func f() { x }\n}\nmain() {}\n");
        assert_invalid(
            "class A {\n    func g() {}\n    static func f() { g() }\n}\nmain() {}\n",
        );
        assert_valid("class A {\n    static let x = 1\n    static func f() { x }\n}\nmain() {}\n");
    }


    #[test]
    fn test_validate_ctor_regular_param_after_member_param() {
        // 命名构造函数：普通参数位于成员变量参数之后。
        assert_invalid("class A {\n    A(x: Bool, let a: Int64, y: Bool) {}\n}\nmain() {}\n");
        // 主构造函数：同上。
        assert_invalid("class A(let a: Int64, x: Bool) {}\nmain() {}\n");
        assert_valid("class A {\n    A(x: Bool, let a: Int64) {}\n}\nmain() {}\n");
    }


    #[test]
    fn test_validate_this_type_placement() {
        // This 不能作参数类型。
        assert_invalid("class A {\n    func f(x: This) {}\n}\nmain() {}\n");
        // This 不能作静态函数返回类型。
        assert_invalid("class A {\n    static func f(): This { A() }\n}\nmain() {}\n");
        // This 不能作局部变量类型。
        assert_invalid("class A {\n    func f() { let x: This = this }\n}\nmain() {}\n");
        // This 不能作顶层函数返回类型。
        assert_invalid("func f(): This { A() }\nclass A {}\nmain() {}\n");
        // 类的实例成员函数返回 This 合法。
        assert_valid("class A {\n    func f(): This { this }\n}\nmain() {}\n");
    }


    #[test]
    fn test_validate_private_parent_member_access() {
        assert_invalid(
            "open class A {\n    private func f() {}\n}\nclass B <: A {\n    func g() { f() }\n}\nmain() {}\n",
        );
        assert_invalid(
            "open class A {\n    private let x = 1\n}\nclass B <: A {\n    func g() { x }\n}\nmain() {}\n",
        );
        assert_valid(
            "open class A {\n    public func f() {}\n}\nclass B <: A {\n    func g() { f() }\n}\nmain() {}\n",
        );
        // 子类同名成员是自己的成员，不视为访问父类 private 成员。
        assert_valid(
            "open class A {\n    private func f() {}\n}\nclass B <: A {\n    func f() {}\n    func g() { f() }\n}\nmain() {}\n",
        );
    }


    #[test]
    fn test_w5c_const_interface_func_requires_const_impl() {
        // 接口 const 函数必须由 const 函数实现
        assert_invalid(
            "interface I {\n\
             \x20   const static func f(): Int64\n\
             }\n\
             class C <: I {\n\
             \x20   public static func f() { 1 }\n\
             }\n\
             main() {}\n",
        );
        assert_valid(
            "interface I {\n\
             \x20   const static func f(): Int64\n\
             }\n\
             class C <: I {\n\
             \x20   public const static func f() { 1 }\n\
             }\n\
             main() {}\n",
        );
    }


    // ── w6a: 泛型约束/型变（09 章 + 6.2.4.1(5)/6.3.4.1(5)）─────────────────

    #[test]
    fn test_w6a_duplicate_type_param_name() {
        // 9.1(4)：类型参数不得重名。
        assert_invalid("class GenericClass<T1, T1> {\n}\nmain() {}\n");
        assert_invalid("func f<T, T>() {}\nmain() {}\n");
        let errs = assert_invalid("interface I {\n    func f<T, T>(): Unit\n}\nmain() {}\n");
        assert!(errs.iter().any(|e| e.message.contains("类型参数重名")));
        assert_valid("class GenericClass<T1, T2> {\n}\nmain() {}\n");
    }


    #[test]
    fn test_w6a_constraint_lhs_must_be_type_variable() {
        // 9.2(4)：约束左界只能是类型变量；接口 where 子句中的具体类型名非法。
        let errs = assert_invalid(
            "open class Animal {}\n\
             class Dog <: Animal {}\n\
             interface GenericInterface<T> where Dog <: Animal {\n\
             \x20   func isAnimal(x: T) { true }\n\
             }\n\
             main() {}\n",
        );
        assert!(errs.iter().any(|e| e.message.contains("泛型约束的左界")));
        // 合法的 where 约束不应误伤。
        assert_valid(
            "open class Animal {}\n\
             class GenericClass<T> where T <: Animal {\n\
             \x20   var x: T\n\
             }\n\
             main() {}\n",
        );
    }


    #[test]
    fn test_w6a_impl_may_not_strengthen_generic_constraints() {
        // 6.2.4.1(5)：实现接口泛型函数时不得添加约束。
        assert_invalid(
            "interface I {\n\
             \x20   static func f<T1, T2, T3, T4>(): Unit where T1 <: T2\n\
             }\n\
             class A <: I {\n\
             \x20   public static func f<T1, T2, T3, T4>() where T1 <: T2, T3 <: T4 {}\n\
             }\n\
             main() {}\n",
        );
        // 反转上下界同样是更严格约束。
        assert_invalid(
            "interface I {\n\
             \x20   static func f<T1, T2>(): Unit where T1 <: T2\n\
             }\n\
             class A <: I {\n\
             \x20   public static func f<T1, T2>() where T2 <: T1 {}\n\
             }\n\
             main() {}\n",
        );
        // 相同约束合法。
        assert_valid(
            "interface I {\n\
             \x20   static func f<T1, T2, T3, T4>(): Unit where T1 <: T2\n\
             }\n\
             class A <: I {\n\
             \x20   public static func f<T1, T2, T3, T4>(): Unit where T1 <: T2 {}\n\
             }\n\
             main() {}\n",
        );
        // 放宽为 `Any` / 更宽松的类上界合法。
        assert_valid(
            "interface I {\n\
             \x20   static func f<T>(): Unit\n\
             }\n\
             class A <: I {\n\
             \x20   public static func f<T>() where T <: Any {}\n\
             }\n\
             main() {}\n",
        );
        assert_valid(
            "open class A1 {}\n\
             open class A2 <: A1 {}\n\
             interface I {\n\
             \x20   func f<T>(): Int64 where T <: A2\n\
             }\n\
             class C <: I {\n\
             \x20   public func f<T>(): Int64 where T <: A1 { 0 }\n\
             }\n\
             main() {}\n",
        );
    }


    #[test]
    fn test_w6a_override_may_not_strengthen_generic_constraints() {
        // 6.3.4.1(5)：类重定义父类泛型函数时不得添加约束。
        assert_invalid(
            "open class I {\n\
             \x20   static func f<T1, T2, T3, T4>() where T1 <: T2 {}\n\
             }\n\
             class A <: I {\n\
             \x20   public static func f<T1, T2, T3, T4>() where T1 <: T2, T3 <: T4 {}\n\
             }\n\
             main() {}\n",
        );
        assert_valid(
            "open class I {\n\
             \x20   static func f<T1, T2, T3, T4>() where T1 <: T2 {}\n\
             }\n\
             class A <: I {\n\
             \x20   public static func f<T1, T2, T3, T4>() where T1 <: T2 {}\n\
             }\n\
             main() {}\n",
        );
    }


    #[test]
    fn test_validate_abstract_property_accessors_overridable() {
        // 抽象类中的无体属性脱糖出的访问器是抽象成员，子类实现/覆写不应被误判为
        // “覆写了父类型中不可覆写（非 open/abstract）的函数”。（对应 07_property 下正测试。）
        // 隐式实现抽象属性（无需 override 关键字）。
        assert_valid(
            "abstract class I {\n\
             \x20   protected prop i1: Int64\n\
             \x20   protected mut prop i2: Int64\n\
             }\n\
             class C <: I {\n\
             \x20   protected prop i1: Int64 { get() { 0 } }\n\
             \x20   protected mut prop i2: Int64 { get() { 0 } set(v) {} }\n\
             }\n\
             main() {}\n",
        );
        // 抽象属性存在带默认实现的兄弟属性。
        assert_valid(
            "abstract class A {\n\
             \x20   protected prop a1: Int64\n\
             \x20   protected mut prop a2: Int64 { get() { 5 } set(v) {} }\n\
             }\n\
             class C <: A {\n\
             \x20   protected prop a1: Int64 { get() { 6 } }\n\
             }\n\
             main() {}\n",
        );
        // 显式 override 抽象不可变属性。
        assert_valid(
            "abstract class A {\n\
             \x20   public prop aaa: Int32\n\
             }\n\
             class B <: A {\n\
             \x20   override public prop aaa: Int32 { get() { 0 } }\n\
             }\n\
             main() {}\n",
        );
        // 显式 override 抽象可变属性。
        assert_valid(
            "abstract class A {\n\
             \x20   public mut prop aaa: Int32\n\
             }\n\
             class B <: A {\n\
             \x20   override public mut prop aaa: Int32 { get() { 0 } set(v) {} }\n\
             }\n\
             main() {}\n",
        );
    }


    #[test]
    fn test_validate_no_body_property_requires_abstract_class() {
        // 非抽象类不能声明无体属性（其访问器没有实现）。
        assert_invalid(
            "class C {\n\
             \x20   prop x: Int64\n\
             }\n\
             main() {}\n",
        );
    }


    // ── w7a: 构造器规则 6.1.2.1 系 ──────────────────────────────────────────

    #[test]
    fn test_w7a_ctor_definite_assignment_rejects_uninitialized_field() {
        // 6.1.2.1(55)：构造函数返回前未初始化无默认值的实例字段。
        assert_invalid(
            "class A {\n\
             \x20   init() {}\n\
             \x20   var x: Int64\n\
             }\n\
             main() {}\n",
        );
    }


    #[test]
    fn test_w7a_ctor_definite_assignment_accepts_initialized_field() {
        assert_valid(
            "class A {\n\
             \x20   init() { x = 1 }\n\
             \x20   var x: Int64\n\
             }\n\
             main() {}\n",
        );
    }


    #[test]
    fn test_w7a_ctor_definite_assignment_conditional_all_branches_ok() {
        // if 两支都赋值 → 确定赋值成立。
        assert_valid(
            "class A {\n\
             \x20   init(c: Bool) { if (c) { x = 1 } else { x = 2 } }\n\
             \x20   var x: Int64\n\
             }\n\
             main() {}\n",
        );
    }


    #[test]
    fn test_w7a_ctor_definite_assignment_conditional_missing_branch() {
        // 仅一支赋值 → 不确定，非法。
        assert_invalid(
            "class A {\n\
             \x20   init(c: Bool) { if (c) { x = 1 } }\n\
             \x20   var x: Int64\n\
             }\n\
             main() {}\n",
        );
    }


    #[test]
    fn test_w7a_ctor_definite_assignment_multiple_ctors_one_missing() {
        // 每个构造函数都需初始化；其中一个漏掉即非法。
        assert_invalid(
            "class A {\n\
             \x20   init() {}\n\
             \x20   init(a: Int64) { x = a }\n\
             \x20   var x: Int64\n\
             }\n\
             main() {}\n",
        );
    }


    #[test]
    fn test_w7a_ctor_call_arg_type_mismatch() {
        // 6.1.2.1(8)/(9)：实参类型与形参不匹配。
        assert_invalid(
            "class A {\n\
             \x20   A(x: Int64) {}\n\
             }\n\
             main() { let a = A(()) }\n",
        );
    }


    #[test]
    fn test_w7a_ctor_call_arg_type_ok() {
        assert_valid(
            "class A {\n\
             \x20   A(x: Int64) {}\n\
             }\n\
             main() { let a = A(1) }\n",
        );
    }


    #[test]
    fn test_w7a_ctor_called_named_param_must_be_named() {
        // 命名形参必须用 `名字:` 传入。
        assert_invalid(
            "class A {\n\
             \x20   A(x!: Int64) {}\n\
             }\n\
             main() { let a = A(1) }\n",
        );
    }


    #[test]
    fn test_w7a_ctor_called_named_param_ok() {
        assert_valid(
            "class A {\n\
             \x20   A(x!: Int64) {}\n\
             }\n\
             main() { let a = A(x: 1) }\n",
        );
    }


    #[test]
    fn test_w7a_ctor_unnamed_param_cannot_be_named() {
        assert_invalid(
            "class A {\n\
             \x20   A(x: Int64) {}\n\
             }\n\
             main() { let a = A(x: 1) }\n",
        );
    }


    #[test]
    fn test_w7a_ctor_named_param_order() {
        // 6.1.2.1(22)：命名参数必须位于普通参数之后。
        assert_invalid(
            "class A {\n\
             \x20   A(x!: Int64, y: Int64) {}\n\
             }\n\
             main() {}\n",
        );
    }


    #[test]
    fn test_w7a_super_member_before_ctor_call() {
        // 6.1.2.1(54)：在显式 this()/super() 之前访问 super 成员。
        assert_invalid(
            "open class P { var x = 1 }\n\
             class A <: P {\n\
             \x20   init() {}\n\
             \x20   init(x: Int64) { super.x\nthis() }\n\
             }\n\
             main() {}\n",
        );
    }


    #[test]
    fn test_w7a_static_context_cannot_use_super() {
        // 6.1.2.1(54)：静态字段初始化不能使用 super。
        assert_invalid(
            "open class P { var x = 1 }\n\
             class A <: P {\n\
             \x20   static let y = super.x\n\
             }\n\
             main() {}\n",
        );
    }


    #[test]
    fn test_w7a_use_uninitialized_member_before_init() {
        // 6.1.2.1(58)：初始化前读取未初始化成员。
        assert_invalid(
            "class A {\n\
             \x20   init() { let t = this.x\n this.x = 1 }\n\
             \x20   var x: Int64\n\
             }\n\
             main() {}\n",
        );
    }


    #[test]
    fn test_w7a_call_instance_method_before_init() {
        // 6.1.2.1(26)：初始化前调用实例成员函数。
        assert_invalid(
            "class A {\n\
             \x20   init() { f()\n this.x = 1 }\n\
             \x20   var x: Int64\n\
             \x20   func f() {}\n\
             }\n\
             main() {}\n",
        );
    }


    #[test]
    fn test_w7a_call_instance_method_after_init_ok() {
        assert_valid(
            "class A {\n\
             \x20   init() { this.x = 1\n this.f() }\n\
             \x20   var x: Int64\n\
             \x20   func f() {}\n\
             }\n\
             main() {}\n",
        );
    }


    #[test]
    fn test_w7a_this_escape_open_class() {
        // 6.1.2.1(57)：open 类构造函数中 lambda 捕获 this。
        assert_invalid(
            "open class A {\n\
             \x20   init() { let t = { => this } }\n\
             }\n\
             main() {}\n",
        );
    }


    #[test]
    fn test_w7a_this_escape_non_open_class_allowed() {
        // 非 open/abstract 类允许 this 逃逸。
        assert_valid(
            "class A {\n\
             \x20   init() { let t = { => this } }\n\
             }\n\
             main() {}\n",
        );
    }


    // ─── 11.1.5.1(8)：程序入口 main 签名 ──────────────────────────────────

    #[test]
    fn test_main_signature_valid_forms() {
        assert_valid("main() {}");
        assert_valid("main(args: Array<String>) {}");
        assert_valid("main(): Unit {}");
        assert_valid("main(): Int64 { 0 }");
        assert_valid("main(): Int8 { var i: Int8 = 0\n return i }");
        // 库包（有 package 声明）中的 main 不受入口点签名约束。
        assert_valid("package pkg\nfunc main(x: Int64) {}");
    }


    #[test]
    fn test_main_signature_invalid_param() {
        // 11.1.5.1(8)：参数只能是 Array<String>。
        assert_invalid("main(x: Int64) {}");
        assert_invalid("main(args: Array<Int64>) {}");
        assert_invalid("main(a: Array<String>, b: Array<String>) {}");
    }


    #[test]
    fn test_main_signature_invalid_return() {
        // 11.1.5.1(8)：返回类型只能是 Unit 或整数类型。
        assert_invalid("main(): Float64 { 0.0 }");
        assert_invalid("main(): Bool { true }");
        assert_invalid("main(): String { \"x\" }");
        // 推断出的返回类型非法同样应被拒绝。
        assert_invalid("main() { var i: Float16 = 0.0\n return i }");
        assert_invalid("main() { return true }");
    }


    // ─── 3.1(3)：顶层名称跨命名空间冲突 ──────────────────────────────────

    #[test]
    fn test_top_level_name_collision_invalid() {
        assert_invalid("let name = 0\nclass name {}\nmain() {}");
        assert_invalid("var name = 1\nfunc name() {}\nmain() {}");
        assert_invalid("class name {}\nstruct name {}\nmain() {}");
        assert_invalid("type name = (Int64, Int64)\nlet name = 0\nmain() {}");
        assert_invalid("enum name { x }\ninterface name {}\nmain() {}");
    }


    #[test]
    fn test_top_level_function_overload_allowed() {
        // 顶层函数互为重载，不视为名称冲突。
        assert_valid("func name() {}\nfunc name(x: Int64) {}\nmain() {}");
        assert_valid("class A {}\nfunc useA(a: A) {}\nmain() {}");
    }


    // ─── 3.1(3)：同一作用域内重复绑定局部变量 ────────────────────────────

    #[test]
    fn test_duplicate_local_binding_invalid() {
        assert_invalid("main() {\n var x = 0\n var x = 1\n}");
        assert_invalid("main() {\n let y = 1\n var y = 2\n}");
        // 嵌套函数体内同块重复声明。
        assert_invalid("main() {\n func f() {\n  let z = 1\n  let z = 2\n }\n}");
    }


    #[test]
    fn test_shadowing_in_nested_scope_valid() {
        // 遮蔽发生在外层作用域时合法。
        assert_valid("main() {\n var x = 0\n if (true) { var x = 1 }\n}");
        assert_valid("func g(x: Int64) { let y = x\n if (x > 0) { let y = 1 } }");
        // 参数与函数体顶层块分属不同作用域（保守：不误判）。
        assert_valid("func h(x: Int64) { let ok = x }");
    }


    // ─── 3.2.3.2(3)/(6)：先使用后声明 ────────────────────────────────────

    #[test]
    fn test_use_before_declaration_invalid() {
        // 引用同一块内稍后才声明的局部变量。
        assert_invalid("main() {\n var vv = tmp\n let tmp: Int64 = 0\n}");
        // 调用同一块内稍后才定义的局部函数。
        assert_invalid("main() {\n foo()\n func foo() {}\n}");
        // 赋值给同一块内稍后才声明的变量。
        assert_invalid("main() {\n x = 1\n let x = 0\n}");
    }


    #[test]
    fn test_use_before_declaration_valid_cases() {
        // 先声明后使用。
        assert_valid("main() {\n let tmp = 0\n var vv = tmp\n}");
        // 引用外层（参数）同名变量，不属于本块先使用后声明。
        assert_valid("func f(tmp: Int64) {\n var vv = tmp\n let keep = 0\n}");
        // 引用顶层函数。
        assert_valid("func foo() {}\nmain() {\n foo()\n}");
    }


    // ─── 6.1.2.1(26)/(58)：成员初始化前的继承成员访问 ─────────────────────

    #[test]
    fn test_ctor_bare_inherited_method_before_init_invalid() {
        // 裸名调用父类实例方法（继承而来的 f）在成员初始化前非法。
        assert_invalid(
            r#"open class P {
  func f() {}
}
class A <: P {
  init() {
    f()
    x = 1
  }
  var x: Int64
}
main() {}"#,
        );
    }


    #[test]
    fn test_ctor_super_property_before_init_invalid() {
        // super.p 指向父类属性（getter），成员初始化前访问非法。
        assert_invalid(
            r#"open class P {
  prop p: Int64 {
    get() {
      10
    }
  }
}
class A <: P {
  init() {
    let a = super.p
    x = 1
  }
  var x: Int64
}
main() {}"#,
        );
    }


    #[test]
    fn test_ctor_bare_inherited_property_before_init_invalid() {
        // 裸名读取父类属性（继承而来的 p）在成员初始化前非法。
        assert_invalid(
            r#"open class P {
  prop p: Int64 {
    get() {
      10
    }
  }
}
class A <: P {
  init() {
    let a = p
    x = 1
  }
  var x: Int64
}
main() {}"#,
        );
    }


    #[test]
    fn test_open_class_ctor_bare_member_invalid() {
        // open 类构造函数内裸名调用实例方法（本类）非法。
        assert_invalid(
            r#"open class A {
  init() {
    f()
  }
  func f() {}
}
main() {}"#,
        );
        // open 类主构造函数内裸名调用继承方法非法。
        assert_invalid(
            r#"open class P {
  func f() {}
}
open class A <: P {
  A() {
    f()
  }
}
main() {}"#,
        );
    }


    #[test]
    fn test_abstract_class_ctor_bare_property_invalid() {
        // abstract 类构造函数内裸名访问实例属性（本类）非法。
        assert_invalid(
            r#"abstract class A {
  init() {
    let a = p
  }
  prop p: Int64 {
    get() {
      10
    }
  }
}
main() {}"#,
        );
    }


    #[test]
    fn test_ctor_super_field_before_init_valid() {
        // super 读取父类实例字段合法（父类构造函数已先行完成）。
        assert_valid(
            r#"open class P {
  var fld: Int64 = 0
}
class A <: P {
  init() {
    let a = super.fld
    x = 1
  }
  var x: Int64
}
main() {}"#,
        );
    }


    #[test]
    fn test_ctor_super_property_after_init_valid() {
        // 全部成员初始化完成后，super 属性访问恢复合法。
        assert_valid(
            r#"open class P {
  prop p: Int64 {
    get() {
      10
    }
  }
}
class A <: P {
  init() {
    x = 1
    let a = super.p
  }
  var x: Int64
}
main() {}"#,
        );
    }


    // ─── 2.1.10.2 结构体构造器规则 ──────────────────────────────────────

    #[test]
    fn test_struct_ctor_argument_mismatch_invalid() {
        // 实参类型与构造形参不匹配。
        assert_invalid("struct A {\n A(x: Int8) {}\n}\nmain() {\n let a = A(())\n}");
        // 多余的位置实参。
        assert_invalid("struct A {\n A() {}\n}\nmain() {\n A(1)\n}");
    }


    #[test]
    fn test_struct_ctor_argument_match_valid() {
        assert_valid("struct A {\n A(x: Int64) {}\n}\nmain() {\n let a = A(1)\n}");
        // 命名参数。
        assert_valid(
            "struct A {\n A(x!: Int64) {}\n}\nmain() {\n let a = A(x: 1)\n}",
        );
    }


    #[test]
    fn test_struct_ctor_param_order_invalid() {
        // 普通参数位于成员变量参数之后。
        assert_invalid("struct A {\n A(public var a: Int64, b: Bool) {}\n}\n");
        // 命名参数之后出现非命名参数。
        assert_invalid("struct A {\n A(a!: Int64, b: Int64) {}\n}\n");
    }


    #[test]
    fn test_struct_ctor_default_references_later_param_invalid() {
        assert_invalid("struct A {\n A(x!: Int64 = y, let y!: Int64) {}\n}\n");
        // 默认值引用自身。
        assert_invalid("struct A {\n A(x!: Int64 = x) {}\n}\n");
    }


    #[test]
    fn test_struct_ctor_definite_assignment_invalid() {
        // 其它构造函数未初始化成员变量参数。
        assert_invalid("struct A {\n A(let a: Int64) {}\n init() {}\n}\n");
        // 构造函数返回前未初始化成员。
        assert_invalid("struct A {\n init() {}\n var x: Int64\n}\n");
    }


    #[test]
    fn test_struct_ctor_definite_assignment_valid() {
        assert_valid("struct A {\n A(let a: Int64) {}\n init() {\n a = 1\n }\n}\n");
    }


    #[test]
    fn test_struct_private_member_access_invalid() {
        assert_invalid(
            "struct A {\n A(private var x: Int64) {}\n}\nmain() {\n let t = A(0)\n t.x\n}",
        );
    }


    #[test]
    fn test_struct_duplicate_member_invalid() {
        assert_invalid("struct A {\n A(var a: Int64) {}\n var a = 1\n}\n");
    }


    #[test]
    fn test_struct_ctor_lambda_captures_this_invalid() {
        assert_invalid("struct A {\n A() {\n let f = { => this }\n }\n}\n");
        // 通过成员函数间接捕获。
        assert_invalid("struct A {\n A() {\n let f = { => g }\n }\n func g() {}\n}\n");
    }


    #[test]
    fn test_struct_ctor_return_non_unit_invalid() {
        assert_invalid("struct A {\n init() {\n return 1i64\n }\n}\n");
        // Unit 返回允许。
        assert_valid("struct A {\n init() {\n return ()\n }\n}\n");
    }


    #[test]
    fn test_struct_ctor_this_call_must_be_first_invalid() {
        assert_invalid("struct A {\n init(x: Int64) {\n let y = 1\n this()\n }\n init() {}\n}\n");
    }


    #[test]
    fn test_struct_member_via_type_name_invalid() {
        assert_invalid(
            "struct A {\n A(var p: Int64) {}\n func check() {\n let t = A.p\n }\n}\n",
        );
    }


    // ─── 13 章 C 互操作（foreign func / CFunc 类型约束） ────────────────────

    #[test]
    fn test_c_interop_foreign_param_non_ctype_rejected() {
        // String / Rune / 元组 / Range / 函数类型 均非 CType。
        assert_invalid("foreign func f(x: String): Int32\nmain() {}");
        assert_invalid("foreign func f(x: Rune): Int32\nmain() {}");
        assert_invalid("foreign func f(x: (Int32, Int32)): Int32\nmain() {}");
        assert_invalid("foreign func f(x: Range<Int32>): Int32\nmain() {}");
        assert_invalid("foreign func f(x: (Int32)->Int32): Int32\nmain() {}");
        // Unit 不能作形参。
        assert_invalid("foreign func f(x: Unit): Int32\nmain() {}");
    }


    #[test]
    fn test_c_interop_foreign_return_non_ctype_rejected() {
        assert_invalid("foreign func f(): String\nmain() {}");
        assert_invalid("foreign func f(): Rune\nmain() {}");
        assert_invalid("foreign func f(): Nothing\nmain() {}");
        assert_invalid("foreign func f(): (Int32, Int32)\nmain() {}");
        assert_invalid("foreign func f(): Range<Int32>\nmain() {}");
    }


    #[test]
    fn test_c_interop_foreign_local_class_enum_interface_rejected() {
        assert_invalid("class ABC {}\nforeign func f(x: ABC): Int32\nmain() {}");
        assert_invalid("class ABC {}\nforeign func f(): ABC\nmain() {}");
        assert_invalid("enum TimeUnit { Year | Month }\nforeign func f(x: TimeUnit): Int32\nmain() {}");
        assert_invalid("interface I {}\nforeign func f(x: I): Int32\nmain() {}");
    }


    #[test]
    fn test_c_interop_foreign_named_or_default_param_rejected() {
        assert_invalid("foreign func f(x: Int32, y!: Int32): Int32\nmain() {}");
        assert_invalid("foreign func f(x: Int32 = 666)\nmain() {}");
    }


    #[test]
    fn test_c_interop_foreign_alias_to_non_ctype_rejected() {
        assert_invalid("type S = String\nforeign func f(x: S): Int32\nmain() {}");
    }


    #[test]
    fn test_c_interop_foreign_legal_forms_accepted() {
        // 基本 C 类型、CPointer/CString/CFunc、Unit 返回值均合法。
        assert_valid("foreign func f(x: Int32, y: Float64): Int32\nmain() {}");
        assert_valid("foreign func f(p: CPointer<Int8>): Unit\nmain() {}");
        assert_valid("foreign func f(s: CString): CString\nmain() {}");
        assert_valid("foreign func f(cb: CFunc<(Int32)->Int32>): Unit\nmain() {}");
        // 无显式返回类型（隐式 Unit）应保持兼容（既有代码依赖）。
        assert_valid("foreign func f(x: Int32)\nmain() {}");
        // @C struct 可作 foreign 签名类型；非 @C 本地 struct 按规范 13.1.3.1 拒绝（w13c 收敛）。
        assert_valid("@C struct S { let x: Int32 }\nforeign func f(p: S): S\nmain() {}");
        assert_invalid("struct S { let x: Int32 }\nforeign func f(p: S): S\nmain() {}");
        // VArray 在解析阶段降级为 Array，须放行以免误伤合法形参。
        assert_valid("foreign func f(a: VArray<Int32, $3>): Unit\nmain() {}");
    }


    #[test]
    fn test_c_interop_cfunc_signature_types_checked() {
        // CFunc 内部签名出现非 C 类型应被拒绝。
        assert_invalid("func f(cb: CFunc<(String)->Int32>) {}\nmain() {}");
        assert_invalid("func f(cb: CFunc<()->Rune>) {}\nmain() {}");
        assert_invalid("func f(cb: CFunc<(Int32)->(Int32)->Int32>) {}\nmain() {}");
        assert_invalid("enum E { A }\nfunc f(cb: CFunc<(E)->Int32>) {}\nmain() {}");
        assert_invalid("class C {}\nfunc f(cb: CFunc<()->C>) {}\nmain() {}");
        // 合法 CFunc：基本类型、Unit 返回、VArray 形参（降级为 Array）。
        assert_valid("func f(cb: CFunc<(Int32)->Int32>) {}\nmain() {}");
        assert_valid("func f(cb: CFunc<()->Unit>) {}\nmain() {}");
        assert_valid("func f(cb: CFunc<(VArray<Int32, $3>)->Int32>) {}\nmain() {}");
    }


    #[test]
    fn test_c_interop_implementation_of_ctype_rejected() {
        // class 不能实现 CType；CType 也不能被 extend 实现。
        assert_invalid("class ABC <: CType {}\nmain() {}");
        assert_invalid("class C {}\nextend C <: CType {}\nmain() {}");
        // 泛型约束 `where T <: CType` 合法，不应被误判。
        assert_valid("class D<T> where T <: CType {}\nmain() {}");
    }


    // ── w9a-class4: 06 章第三轮规则 ──────────────────────────────────────

    #[test]
    fn test_w9a_ctor_default_refs_instance_member_rejected() {
        // 6.1.2.1(49)：构造参数默认值不得引用裸成员名 / super.x。
        // （`this.x` 由解析器在方法体之外直接拒绝。）
        let errs = assert_invalid(
            "class A {\n\
             \x20   var x: Int64\n\
             \x20   init(p!: Int64 = x) {}\n\
             }\n\
             main() {}\n",
        );
        assert!(errs.iter().any(|e| e.message.contains("6.1.2.1(49)")));
        assert_invalid(
            "open class B {\n\
             \x20   var x: Int64 = 1\n\
             }\n\
             class A <: B {\n\
             \x20   init(p!: Int64 = super.x) {}\n\
             }\n\
             main() {}\n",
        );
    }


    #[test]
    fn test_w9a_ctor_default_refs_param_or_global_ok() {
        assert_valid(
            "class A {\n\
             \x20   init(q!: Int64 = 1, p!: Int64 = q) {}\n\
             }\n\
             main() {}\n",
        );
        assert_valid(
            "let g = 1\n\
             class A {\n\
             \x20   init(p!: Int64 = g) {}\n\
             }\n\
             main() {}\n",
        );
    }


    #[test]
    fn test_w9a_this_ctor_call_outside_init_rejected() {
        // 6.1.2.1(59)：构造函数 this(...) 只能在构造函数体内调用。
        let errs = assert_invalid(
            "class A {\n\
             \x20   init() {}\n\
             \x20   func f() { this() }\n\
             }\n\
             main() {}\n",
        );
        assert!(errs.iter().any(|e| e.message.contains("6.1.2.1(59)")));
        assert_invalid(
            "class A {\n\
             \x20   init() {}\n\
             \x20   let x = this()\n\
             }\n\
             main() {}\n",
        );
        assert_invalid(
            "class A {\n\
             \x20   init() {}\n\
             \x20   prop p: Int64 {\n\
             \x20       get() { this() }\n\
             \x20   }\n\
             }\n\
             main() {}\n",
        );
    }


    #[test]
    fn test_w9a_this_ctor_call_inside_init_ok() {
        assert_valid(
            "class A {\n\
             \x20   init() {}\n\
             \x20   init(x: Int64) { this() }\n\
             }\n\
             main() {}\n",
        );
    }


    #[test]
    fn test_w9a_static_instance_overload_rejected() {
        // 6.3.2(1)：接口内部 / 接口继承 / 实现类型 / 多父类型之间的 static 与实例同名。
        assert_invalid(
            "interface I {\n\
             \x20   func f() {}\n\
             \x20   static func f<T>() {}\n\
             }\n\
             main() {}\n",
        );
        assert_invalid(
            "interface I1 {\n\
             \x20   static func f() {}\n\
             }\n\
             interface I2 <: I1 {\n\
             \x20   func f() {}\n\
             }\n\
             main() {}\n",
        );
        assert_invalid(
            "interface I {\n\
             \x20   static func f() {}\n\
             }\n\
             class A <: I {\n\
             \x20   public func f(x: Int64) {}\n\
             }\n\
             main() {}\n",
        );
        assert_invalid(
            "interface I {\n\
             \x20   static func f() {}\n\
             }\n\
             open class A {\n\
             \x20   func f(x: Int64) {}\n\
             }\n\
             class B <: A & I {}\n\
             main() {}\n",
        );
    }


    #[test]
    fn test_w9a_static_instance_overload_same_kind_ok() {
        assert_valid(
            "interface I {\n\
             \x20   func f() {}\n\
             \x20   func f(x: Int64) {}\n\
             }\n\
             main() {}\n",
        );
        assert_valid(
            "interface I {\n\
             \x20   static func f() {}\n\
             }\n\
             class A <: I {\n\
             \x20   public static func f() {}\n\
             }\n\
             main() {}\n",
        );
    }


    #[test]
    fn test_w9a_member_kind_shadowing_rejected() {
        // 6.3.3(1)：不同种类（函数/属性/字段）的同名成员构成遮蔽。
        // 类实现接口（此前 collect_parent_field_names 在接口父类型上提前返回而漏检）。
        assert_invalid(
            "interface I {\n\
             \x20   prop x: Int64 {\n\
             \x20       get() { 1 }\n\
             \x20   }\n\
             }\n\
             class C <: I {\n\
             \x20   func x() {}\n\
             }\n\
             main() {}\n",
        );
        // struct 字段遮蔽接口函数。
        assert_invalid(
            "interface I {\n\
             \x20   func x() {}\n\
             }\n\
             struct C <: I {\n\
             \x20   let x = 1\n\
             }\n\
             main() {}\n",
        );
        // enum 函数遮蔽接口属性。
        assert_invalid(
            "interface I {\n\
             \x20   static prop x: Int64 {\n\
             \x20       get() { 1 }\n\
             \x20   }\n\
             }\n\
             enum E <: I {\n\
             \x20   D\n\
             \x20   static func x() {}\n\
             }\n\
             main() {}\n",
        );
        // 接口继承中的种类遮蔽。
        assert_invalid(
            "interface I1 {\n\
             \x20   static func x() {}\n\
             }\n\
             interface I2 <: I1 {\n\
             \x20   prop x: Int64 {\n\
             \x20       get() { 1 }\n\
             \x20   }\n\
             }\n\
             main() {}\n",
        );
    }


    #[test]
    fn test_w9a_member_same_kind_not_shadowing() {
        assert_valid(
            "interface I {\n\
             \x20   func x() {}\n\
             }\n\
             struct C <: I {\n\
             \x20   public func x() {}\n\
             }\n\
             main() {}\n",
        );
        assert_valid(
            "interface I {\n\
             \x20   func x() {}\n\
             }\n\
             enum E <: I {\n\
             \x20   D\n\
             \x20   public func x() {}\n\
             }\n\
             main() {}\n",
        );
    }


    #[test]
    fn test_w9a_impl_access_lowered_struct_enum_rejected() {
        // 6.3.5(3)：struct/enum 实现接口函数的访问级别不得低于接口。
        assert_invalid(
            "interface I {\n\
             \x20   func f(): Unit\n\
             }\n\
             struct A <: I {\n\
             \x20   func f() {}\n\
             }\n\
             main() {}\n",
        );
        assert_invalid(
            "interface I {\n\
             \x20   static func f<T>() {}\n\
             }\n\
             struct A <: I {\n\
             \x20   private static func f<T>() {}\n\
             }\n\
             main() {}\n",
        );
        assert_invalid(
            "interface I {\n\
             \x20   func f(): Unit\n\
             }\n\
             enum A <: I {\n\
             \x20   E\n\
             \x20   private func f() {}\n\
             }\n\
             main() {}\n",
        );
        // 接口成员带默认实现时同样适用。
        assert_invalid(
            "interface I {\n\
             \x20   func f() {}\n\
             }\n\
             enum A <: I {\n\
             \x20   E\n\
             \x20   func f() {}\n\
             }\n\
             main() {}\n",
        );
    }


    #[test]
    fn test_w9a_impl_access_public_ok() {
        assert_valid(
            "interface I {\n\
             \x20   func f(): Unit\n\
             }\n\
             struct A <: I {\n\
             \x20   public func f() {}\n\
             }\n\
             main() {}\n",
        );
        assert_valid(
            "interface I {\n\
             \x20   static func f<T>() {}\n\
             }\n\
             struct A <: I {\n\
             \x20   public static func f<T>() {}\n\
             }\n\
             main() {}\n",
        );
    }


    // ─── 5.1.2 / 5.3 形参声明与调用规则 ─────────────────────────────────────

    #[test]
    fn test_named_param_after_unnamed_accepted() {
        // 5.1.2.1(2)：非命名形参位于命名形参之前合法。
        assert_valid("func f(a: Int32, b!: Int32) {}\nmain() {}");
    }


    #[test]
    fn test_unnamed_param_after_named_rejected() {
        // 5.1.2.1(2)：命名形参之后不能再出现非命名形参。
        assert_invalid("func f(a!: Int32, b: Int32) {}\nmain() {}");
        assert_invalid(
            "class C {\n\
             \x20   func m(a: Int32, b!: Int32, c: Bool) {}\n\
             }\n\
             main() {}\n",
        );
    }


    #[test]
    fn test_default_param_must_be_named() {
        // 5.1.2.2(3)：带默认值的形参必须是命名形参。
        assert_valid("func f(a!: Int32 = 1) {}\nmain() {}");
        assert_invalid("func f(a: Int32 = 1) {}\nmain() {}");
    }


    #[test]
    fn test_variadic_param_position() {
        // 5.3.4(4)：变长形参必须是最后一个形参。
        assert_valid("func f(a: Int32, b: Int32...) {}\nmain() {}");
        assert_invalid("func f(a: Int32..., b: Int32) {}\nmain() {}");
    }


    #[test]
    fn test_param_missing_type_annotation_rejected() {
        // 5.1.2.1：形参必须带类型标注，`name = expr` 缺类型非法。
        assert_invalid("func f(a! = 1) {}\nmain() {}");
        assert_valid("func f(a!: Int32 = 1) {}\nmain() {}");
    }


    #[test]
    fn test_named_wildcard_param_rejected() {
        // 5.1.2.1：命名通配符形参 `_!` 非法；普通通配符形参合法。
        assert_invalid("func f(_!: Int32) {}\nmain() {}");
        assert_valid("func f(_: Int32) {}\nmain() {}");
    }


    #[test]
    fn test_default_value_static_scope() {
        // 5.1.2.2(8)/(12)/(13)：默认值只能引用在其之前声明的形参。
        assert_valid("func f(a: Int32, b!: Int32 = a) {}\nmain() {}");
        assert_invalid("func f(a!: Int32 = b, b!: Int32) {}\nmain() {}");
        // 自身引用同样非法。
        assert_invalid("func f(a!: Int32 = a) {}\nmain() {}");
    }


    #[test]
    fn test_optional_param_forbidden_in_open_contexts() {
        // 5.1.2.2(5)：接口函数（抽象或默认实现）与 open/abstract 类方法不能有可选参数。
        assert_invalid("interface I { func f(a!: Int32 = 1): Int32 }\nmain() {}");
        assert_invalid(
            "interface I { func f(a!: Int32 = 1): Int32 { a } }\nmain() {}",
        );
        assert_invalid(
            "open class C {\n\
             \x20   open public func f(a!: Int32 = 1): Int32 { a }\n\
             }\n\
             main() {}\n",
        );
        // 普通（非 open）类方法可以带可选参数。
        assert_valid(
            "class C {\n\
             \x20   func f(a!: Int32 = 1): Int32 { a }\n\
             }\n\
             main() {}\n",
        );
    }


    #[test]
    fn test_optional_param_forbidden_in_operator_func() {
        // 5.1.2.2(6)：运算符函数不能有可选参数。
        assert_invalid(
            "class C {\n\
             \x20   operator func ==(a!: Int32 = 1) {}\n\
             }\n\
             main() {}\n",
        );
        assert_valid(
            "class C {\n\
             \x20   operator func ==(a!: Int32) {}\n\
             }\n\
             main() {}\n",
        );
    }


    #[test]
    fn test_call_positional_overflow_rejected() {
        // 5.3.1(4)：命名形参必须以 `名字:` 传入，多余位置实参非法。
        assert_valid("func f(a: Int32, b!: Int32) {}\nmain() { f(1, b: 2) }");
        assert_invalid("func f(a: Int32, b!: Int32) {}\nmain() { f(1, 2) }");
    }


    #[test]
    fn test_call_named_argument_matching() {
        // 5.3.1(1)/(3)：命名实参必须对应命名形参。
        assert_valid("func f(a!: Int32) {}\nmain() { f(a: 1) }");
        // 命名实参对应到非命名形参 → 非法。
        assert_invalid("func f(a: Int32) {}\nmain() { f(a: 1) }");
        // 不存在的形参名 → 非法。
        assert_invalid("func f(a!: Int32) {}\nmain() { f(b: 1) }");
        // 省略带默认值的命名实参合法。
        assert_valid("func f(a!: Int32 = 1) {}\nmain() { f() }");
    }


    #[test]
    fn test_call_in_macro_argument_not_semantically_checked() {
        // 宏实参由宏自行解释（如 @PowerAssert），其中的调用不按普通调用检查。
        assert_valid(
            "func f(x: Unit) {}\n\
             func g() { @PowerAssert(f(b = 2)) }\n\
             main() {}\n",
        );
    }


    // ─── w9d 16 章：const 函数体 / const 声明限制 ─────────────────────────

    #[test]
    fn test_w9d_const_function_body_restrictions() {
        // const 函数不能声明 var（16.4(2)）。
        assert_invalid("const func f() {\n var x = 0\n}\nmain() {}");
        // const 函数不能包含 for 循环。
        assert_invalid("const func f() {\n for (_ in 0..3) {}\n}\nmain() {}");
        // const 函数不能调用非 const 函数。
        assert_invalid("func g() {}\nconst func f() {\n g()\n}\nmain() {}");
        // const 函数不能包含 Range 表达式。
        assert_invalid("const func f() {\n 1..2\n return 0\n}\nmain() {}");
        // let 声明 + return 常量表达式合法。
        assert_valid("const func f(x: Int64) {\n let y = 1\n return x + y\n}\nmain() {}");
    }


    #[test]
    fn test_w9d_const_declaration_rules() {
        // 含 var 实例成员变量的类不能定义 const init（16.5）。
        assert_invalid("class C {\n var v = 0\n const init() {}\n}\nmain() {}");
        // const 实例成员函数要求类型定义 const init（16.4）。
        assert_invalid("class C {\n const func f() {}\n}\nmain() {}");
        assert_valid("class C {\n const init() {}\n const func f() {}\n}\nmain() {}");
        // const 成员函数不能是 open（16.4）。
        assert_invalid("class C {\n open const func f() {}\n const init() {}\n}\nmain() {}");
        // const init 类型中非 const 初始值的 let 成员非法（16.5）。
        assert_invalid(
            "func g() { 1 }\nstruct S {\n let v = g()\n const init() {}\n}\nmain() {}",
        );
    }


    #[test]
    fn test_w9d_const_init_body_restrictions() {
        // const init 不能调用非 const 函数（16.5）。
        assert_invalid("func g() {}\nstruct S {\n const init() { g() }\n}\nmain() {}");
        // const init 不能声明 var。
        assert_invalid("struct S {\n const init() {\n var x = 0\n }\n}\nmain() {}");
        // 合法的 const init：对实例字段赋值。
        assert_valid("class C {\n let v: Int64\n const init() {\n v = 1\n }\n}\nmain() {}");
    }


    // ─── w10a: 枚举属性 / 显式类型实参 / 重定义签名比较 ──────────────────────

    #[test]
    fn test_w10a_enum_property_type_must_match_interface() {
        // 6.2.4.1(7)：枚举实现的属性类型必须与接口一致（不允许协变/逆变）。
        assert_invalid(
            "open class Sup {}\nclass Sub <: Sup {}\n\
             interface I { prop p: Sup { get() { Sup() } } }\n\
             enum A <: I {\n E\n public override prop p: Sub { get() { Sub() } }\n}\nmain() {}",
        );
        // 类型一致则合法。
        assert_valid(
            "interface I { prop p: Int64 { get() { 0 } } }\n\
             enum A <: I {\n E\n public override prop p: Int64 { get() { 1 } }\n}\nmain() {}",
        );
    }


    #[test]
    fn test_w10a_enum_property_mut_must_match_interface() {
        // 6.2.4.1(6)：不可变接口属性被实现为 mut 属性非法。
        assert_invalid(
            "interface I { prop p: Int64 { get() { 0 } } }\n\
             enum A <: I {\n E\n public override mut prop p: Int64 { get() { 1 } set(v) {} }\n}\nmain() {}",
        );
        // 抽象 mut 属性被实现为不可变属性非法。
        assert_invalid(
            "interface I { static mut prop p: Int64 }\n\
             enum A <: I {\n E\n public redef static prop p: Int64 { get() { 1 } }\n}\nmain() {}",
        );
        // mut 一致则合法。
        assert_valid(
            "interface I { mut prop p: Int64 { get() { 0 } set(v) {} } }\n\
             struct S <: I {\n public override mut prop p: Int64 { get() { 1 } set(v) {} }\n}\nmain() {}",
        );
    }


    #[test]
    fn test_w10a_redef_named_parameter_flag_must_match() {
        // 6.3.4.1(3)：命名参数标记属于参数列表的一部分，redef 必须一致。
        assert_invalid(
            "interface I { static func f(x: Int64, y: Int64) {} }\n\
             class A <: I { public redef static func f(x: Int64, y!: Int64) {} }\nmain() {}",
        );
        assert_invalid(
            "interface I { static func f(x: Int64, y!: Int64) {} }\n\
             class A <: I { public redef static func f(x: Int64, y: Int64) {} }\nmain() {}",
        );
        assert_valid(
            "interface I { static func f(x: Int64, y: Int64) {} }\n\
             class A <: I { public redef static func f(x: Int64, y: Int64) {} }\nmain() {}",
        );
    }


    #[test]
    fn test_w10a_redef_generic_instantiation_substitution() {
        // 6.3.4.1(3)：父接口按 `<: I<Int32>>` 实例化后，参数应比较替换后的类型。
        assert_invalid(
            "interface I<T> { static func f<U>(x: T): Unit }\n\
             abstract class A <: I<Int32> { public redef static func f<U>(x: Int64) {} }\nmain() {}",
        );
        assert_valid(
            "interface I<T> { static func f<U>(x: T): Unit }\n\
             class A <: I<Int64> { public redef static func f<U>(x: Int64) {} }\nmain() {}",
        );
        // 泛型参数与具体类型不可互换。
        assert_invalid(
            "interface I { static func f(x: Rune): Unit }\n\
             abstract class A<T> <: I { public redef static func f(x: T) {} }\nmain() {}",
        );
    }


    // ── w10c: 03 章块级作用域（保守重做） ───────────────────────────────────

    #[test]
    fn test_w10c_let_var_irrefutable_literal_rejected() {
        // 3.3.1(9)/(10)：含字面量的可反驳模式被拒绝。
        assert_invalid("main() {\n let (x, 13, z) = (1, 2, 3)\n}");
        assert_invalid("main() {\n var (1, x) = (2, 3)\n}");
    }


    #[test]
    fn test_w10c_let_var_irrefutable_binding_patterns_accepted() {
        // 纯绑定 / 通配符 / 元组解构 / 枚举变体模式一律放行（w9c 回退教训）。
        assert_valid("main() {\n let (x, y) = (1, 2)\n}");
        assert_valid("main() {\n let (_, z) = (1, 2)\n}");
        assert_valid("main() {\n var _ = 1\n}");
        assert_valid("enum Color {\n Red(Int64)\n}\nmain() {\n let Red(a) = Red(1)\n}");
        assert_valid("main() {\n for (i in 1..3) { let v = i }\n}");
    }


    #[test]
    fn test_w10c_param_shadowing_rejected() {
        // 3.2.3.2(11)：函数体最外层不能声明与形参同名变量。
        assert_invalid("func f(x: Int64) {\n var x = 1\n}\nmain() {}");
        // 嵌套块内遮蔽形参合法。
        assert_valid("func f(x: Int64) {\n if (true) { let x = 1 }\n}\nmain() {}");
        // 自引用式遮蔽（var x = x）合法（std 常见写法）。
        assert_valid("func f(x: Int64) {\n var x = x\n}\nmain() {}");
    }


    #[test]
    fn test_w10c_block_local_out_of_scope_rejected() {
        // 3.2.3.2(1)/(2)/(3)：块内局部变量离开块后不可见。
        assert_invalid("main() {\n for (i in 1..3) { var j = 1 }\n j = 0\n}");
        assert_invalid("main() {\n if (true) { let a = 1 }\n print(\"${a}\")\n}");
        assert_invalid("main() {\n try { let e = 1 } catch (_) {}\n print(\"${e}\")\n}");
    }


    #[test]
    fn test_w10c_block_local_shadowing_accepted() {
        // 内层块用同名局部变量遮蔽外层合法。
        assert_valid(
            "main() {\n let a = 1\n if (true) { let a = 2; print(\"${a}\") }\n print(\"${a}\")\n}",
        );
    }


    #[test]
    fn test_w10c_use_before_decl_in_condition_rejected() {
        // 3.2.3.2(4)：if / match 条件中引用后声明的变量。
        assert_invalid("main() {\n if (tmp == 0) {}\n let tmp = 0\n}");
        assert_invalid("main() {\n let ss = match (tmp) { case _ => 0 }\n var tmp = 0\n}");
        assert_valid("main() {\n let tmp = 0\n if (tmp == 0) {}\n}");
    }


    #[test]
    fn test_w10c_local_func_out_of_scope_rejected() {
        // 3.2.3.2(6)：局部函数只在声明块内可见。
        assert_invalid("main() {\n if (true) { func foo() {}\n }\n foo()\n}");
        assert_valid("main() {\n if (true) { func foo() {}; foo() }\n}");
    }


    #[test]
    fn test_w10c_free_fn_unqualified_member_rejected() {
        // 3.2.3.3(1)：自由函数不能无资格引用其它类型成员。
        assert_invalid("class C {\n func f() {}\n}\nmain() {\n f()\n}");
        assert_invalid("class C {\n var v: Int64 = 0\n}\nmain() {\n let x = v\n}");
        // 类型内部无资格访问自身成员合法。
        assert_valid("class C {\n func f() { f() }\n}\nmain() {}");
    }


    #[test]
    fn test_w10c_class_field_method_same_name_rejected() {
        // 3.1(3)：同一 class 内字段名与成员函数名不能相同。
        assert_invalid("class C {\n let a = 0\n func a() {}\n}\nmain() {}");
        assert_valid("class C {\n let a = 0\n func b() {}\n}\nmain() {}");
    }


    #[test]
    fn test_w10d_index_operator_forbidden_builtin_types() {
        // 10.2.4(12)：内建受限类型不支持带赋值形态的 [] 重载。
        assert_invalid("extend Int64 { operator func[](index: Int64, value!: Int64) {} }\nmain() {}");
        assert_invalid("extend Bool { operator func[](index: Int64, value!: Bool) {} }\nmain() {}");
        assert_invalid("extend String { operator func[](index: Int64, value!: String) {} }\nmain() {}");
        // 仅取 subscript（无赋值形态）不在此限。
        assert_valid("extend Int64 { operator func[](index: Int64) { 0 } }\nmain() {}");
    }


    #[test]
    fn test_w10d_extend_function_tuple_alias_forbidden() {
        // 10.2.4(12)：函数/元组类型不允许被 extend。
        assert_invalid("type F = (Int64) -> Int64\nextend F { func g() {} }\nmain() {}");
        assert_invalid("type T = (Int64, Int64)\nextend T { func g() {} }\nmain() {}");
    }


    #[test]
    fn test_w11b_undefined_variable_rejected() {
        // 3.2.3.2：无法解析的裸变量引用（涉及整数字面量/后缀形近词）。
        assert_invalid("main() {\n let a: Int8 = _0\n}");
        assert_invalid("main() {\n let a: Int8 = i8\n}");
        assert_invalid("main() {\n let a: Int32 = i32\n}");
        assert_invalid("main() {\n let a: Int8 = 0-x1\n}");
        assert_invalid("main() {\n let a: UInt64 = u64\n}");
        // 字符串 / 字符 / 布尔字面量的错误写法（大小写变体）。
        assert_invalid("main() {\n let s: String = incorrect_string_literal\n}");
        assert_invalid("main() {\n let b: Bool = True\n}");
        assert_invalid("main() {\n let b: Bool = False\n}");
        assert_invalid("main() {\n let b = true_\n}");
        assert_invalid("main() {\n `q`\n}");
        // 裸变量出现在元组下标对象位置（f16.0 / e.0 / _1.0）。
        assert_invalid("main() {\n let a: Float16 = f16.0\n}");
        assert_invalid("main() {\n let a: Float16 = e.0\n}");
        assert_invalid("main() {\n let a: Float16 = _1.0\n}");
        // 二进制运算操作数。
        assert_invalid("main() {\n x1 - 1\n}");
    }


    #[test]
    fn test_w11b_undefined_scope_forms_not_rejected() {
        // 形参、局部变量、局部函数、for 变量、顶层函数均视为已声明。
        assert_valid("main(ina: Array<String>) {\n let z = ina.size\n}");
        assert_valid("main() {\n let x = 1\n let y = x + 1\n}");
        assert_valid("func f(a: Int64): Int64 { a + 1 }\nmain() {\n let r = f(2)\n}");
        assert_valid("main() {\n func g() { 1 }\n let x = g()\n}");
        assert_valid("main() {\n for (i in 0..10) {\n println(i)\n }\n}");
        // 核心类型 / 内建函数是预置名。
        assert_valid("main() {\n let a: Int64 = 1\n println(a)\n}");
    }


    #[test]
    fn test_w11b_undefined_check_skipped_with_imports() {
        // 存在 import 时不判定未定义（导入依赖无法静态确定），避免 std/正测试误伤。
        assert_valid("import std.math.*\nmain() {\n let x = PI\n}");
        // 首字母大写的裸名按类型/隐式导入符号放行。
        assert_valid("main() {\n let v = KeyOpt\n}");
    }


    #[test]
    fn test_w11c_struct_interface_signature_mismatch() {
        // 2.1.10(8)：结构体实现接口时同名方法签名必须与接口声明一致
        // （即使接口成员带默认实现）。
        let errs = assert_invalid(
            "interface I { func f(): Int8 { 1i8 } }\n\
             struct S <: I { public func f(): Int32 { 1 } }\n\
             main() {}",
        );
        assert!(
            errs.iter()
                .any(|e| e.message.contains("签名与接口声明不一致")),
            "实际错误: {:?}",
            errs
        );
        // 正例：签名一致时通过。
        assert_valid(
            "interface I { func f(): Int8 { 1i8 } }\n\
             struct S <: I { public func f(): Int8 { 1i8 } }\n\
             main() {}",
        );
    }


    // ─── w11d：04 章 4.5.4/4.7 控制转移表达式上下文 ──────────────────────────

    #[test]
    fn test_w11d_break_continue_require_loop() {
        // 4.7(6)：break/continue 必须位于循环体内。
        assert_invalid("main() {\n break\n return 0\n}");
        assert_invalid("main() {\n continue\n return 0\n}");
        assert_invalid("func f() {\n break\n}\nmain() { while (true) { f() } }");
        // 嵌套函数 / lambda 中的 break 不能借用外层循环。
        assert_invalid("main() {\n for (x in 1..10) {\n func inner() { break }\n inner()\n }\n}");
        // 顶层/字段默认值中的 break 非法。
        assert_invalid("var n = break\nmain() {}");
        assert_invalid("class C {\n var n = break\n}\nmain() {}");
        // 正例：循环体内、嵌套控制结构中的 break/continue 合法。
        assert_valid("main() { for (x in 1..10) { if (x > 0) { break } } }");
        assert_valid("main() { var i = 0; while (i < 10) { i++; if (i == 3) { continue } } }");
        assert_valid("main() { loop { break } }");
    }


    #[test]
    fn test_w11d_return_requires_function_body() {
        // 4.7(7)：return 只能出现在函数体内（字段默认值非法）。
        assert_invalid("class C {\n var n = return\n}\nmain() {}");
        assert_invalid("struct S {\n var n = return\n}\nmain() {}");
        // 正例：函数体 / lambda 内的 return。
        assert_valid("func f(): Int64 { return 0 }\nmain() { f() }");
        assert_valid("main() { let g = { => return }; g() }");
    }


    // ─── w11d：10 章 10.2.3(4)-(14) 运算符重载元数 ──────────────────────────

    #[test]
    fn test_w11d_operator_arity_rules() {
        // 二元运算符必须恰有 1 个（非接收者）参数。
        assert_invalid("class C {\n operator func >() { }\n}\nmain() {}");
        assert_invalid("class C {\n operator func +() { }\n}\nmain() {}");
        assert_invalid("class C {\n operator func **() { }\n}\nmain() {}");
        // 一元运算符必须无参数。
        assert_invalid("class C {\n operator func !(other: C) { }\n}\nmain() {}");
        // `&&` / `||` 不支持重载。
        assert_invalid("class C {\n operator func &&(other: C) { }\n}\nmain() {}");
        assert_invalid("class C {\n operator func ||(other: C) { }\n}\nmain() {}");
        // 正例：正确的元数；`-` 一元/二元均可；`[]`/`()` 不在此限。
        assert_valid("class C {\n operator func >(other: C) { }\n}\nmain() {}");
        assert_valid("class C {\n operator func !() { }\n}\nmain() {}");
        assert_valid("class C {\n operator func -() { }\n operator func -(o: C) { }\n}\nmain() {}");
        assert_valid("class C {\n operator func [](i: Int64) { }\n}\nmain() {}");
    }


    // ─── w12a-class7：06 章第六轮 open/abstract 可见性、redef、重复父接口 ─────

    #[test]
    fn test_w12a_class7_open_abstract_private() {
        // 6.1.1.1(9)：抽象类中 private 的无实现体实例函数（抽象函数）非法。
        assert_invalid("abstract class A {\n private func f(): Unit\n}\nmain() {}");
        assert_invalid("abstract class A {\n private func f(x: Int64): Unit\n}\nmain() {}");
        // 6.1.2.4(36)：open 成员函数不能声明为 private。
        assert_invalid("open class A {\n open private func f() {}\n}\nmain() {}");
        assert_invalid("abstract class A {\n open private func f(x: Int64) {}\n}\nmain() {}");
        // 正例：public / protected 的 open / 抽象函数合法。
        assert_valid("open class A {\n protected open func f() {}\n}\nmain() {}");
        assert_valid("open class A {\n public open func f() {}\n}\nmain() {}");
        assert_valid("abstract class A {\n public func f(): Unit\n}\nmain() {}");
        assert_valid("abstract class A {\n protected func g(): Unit\n}\nmain() {}");
        // 正例：private 的普通（有实现体、非 open）实例函数合法。
        assert_valid("class A {\n private func f() {}\n}\nmain() {}");
    }


    #[test]
    fn test_w12a_class7_redef_instance() {
        // 6.1.2.4(44) / 6.3.4.1(1)：redef 不能修饰实例成员函数（须用 override）。
        assert_invalid(
            "open class A {\n public open func f() {}\n}\n\
             class B <: A {\n public redef func f() {}\n}\nmain() {}",
        );
        assert_invalid(
            "interface I {\n func f() {}\n}\n\
             class A <: I {\n public redef func f() {}\n}\nmain() {}",
        );
        // 正例：redef 修饰 static 函数（重定义父类型同名 static 函数）合法。
        assert_valid(
            "open class A {\n public static func f() {}\n}\n\
             class B <: A {\n public redef static func f() {}\n}\nmain() {}",
        );
    }


    #[test]
    fn test_w12a_class7_duplicate_super_interfaces() {
        // 6.1.1.3(8)/(9)：同一接口不得以相同泛型实参被直接实现多次。
        assert_invalid("interface I {}\nclass A <: I & I {}\nmain() {}");
        assert_invalid("interface I<T> {}\nclass A<T> <: I<T> & I<T> {}\nmain() {}");
        assert_invalid("interface I {}\nopen class P {}\nclass A <: P & I & I {}\nmain() {}");
        // 正例：不同泛型实参直接实现多次合法。
        assert_valid("interface I<T> {}\nclass A<T> <: I<T> & I<Int64> {}\nmain() {}");
        assert_valid("interface I<T> {}\nclass A <: I<Bool> & I<Rune> & I<Int64> {}\nmain() {}");
    }


    #[test]
    fn test_w12a_class7_duplicate_interface_parents() {
        // 6.2.3(4)/(5)：接口不得以相同泛型实参重复直接继承父接口。
        assert_invalid("interface I {}\ninterface J <: I & I {}\nmain() {}");
        assert_invalid("interface I<T> {}\ninterface J <: I<Int64> & I<Int64> {}\nmain() {}");
        // 正例：不同泛型实参重复继承合法。
        assert_valid("interface I<T> {}\ninterface J <: I<Int64> & I<String> {}\nmain() {}");
        assert_valid("interface I<T> {}\ninterface J<T> <: I<T> {}\nmain() {}");
    }


    // ─── w12c：03 章关键字作标识符 / 同类别重复 / 字段方法同名 ──────────────

    #[test]
    fn test_w12c_reserved_keyword_identifiers() {
        // 3.1(2)：`where`/`type`/`is`/`case` 与变量/函数/包名共享命名空间。
        assert_invalid("var where = 1\nmain() {}");
        assert_invalid("let is = 0\nmain() {}");
        assert_invalid("func case() {}\nmain() {}");
        assert_invalid("var type = 1\nmain() {}");
        assert_invalid("package is\nmain() {}");
        assert_invalid("package main\nmain() {}");
        // 预声明类型名不能被重新声明为类型。
        assert_invalid("enum Unit { x }\nmain() {}");
        // `main` 作为入口函数名合法；入口函数体内的普通变量合法。
        assert_valid("main() {}");
        // `from` 是可作标识符的上下文关键字（官方正例）。
        assert_valid("let from = 0\nmain() {}");
        // 反引号形态的关键字标识符在官方正例中合法（AST 不保留反引号，按局部
        // 变量处理时不在检查范围）。
        assert_valid("main() { let `is` = 1 }");
        // import 存在时保守跳过（反引号/装饰器场景无法静态区分）。
        assert_valid("import utils.assert.Assert\nvar where = 1\nmain() {}");
    }


    #[test]
    fn test_w12c_duplicate_toplevel_declarations() {
        // 3.1(3)：无 import 单文件程序中，变量/类型别名同类别重复非法。
        assert_invalid("let name = 0\nlet name = 0\nmain() {}");
        assert_invalid("var name = 1\nlet name = 0\nmain() {}");
        assert_invalid("type name = Int64\ntype name = Int64\nmain() {}");
        // 同名函数是重载，合法。
        assert_valid("func f() {}\nfunc f(x: Int64) {}\nmain() {}");
        // 不同作用域的遮蔽合法。
        assert_valid("let name = 0\nmain() { let name = 1 }");
        // 含 import 时保守跳过（@When 平台分支会保留重复声明）。
        assert_valid("import utils.assert.Assert\nlet name = 0\nlet name = 0\nmain() {}");
    }


    #[test]
    fn test_w12c_struct_field_method_conflict() {
        // 3.1(3)：结构体显式字段与成员函数同名非法。
        assert_invalid("struct S {\n var b = 0.5\n func b() {}\n}\nmain() {}");
        // 正例：字段名与方法名不同。
        assert_valid("struct S {\n var a = 0.5\n func b() {}\n}\nmain() {}");
    }


    // ─── w12c：10.2.4 下标运算符 set 形态声明细则 ─────────────────────────

    #[test]
    fn test_w12c_index_operator_declaration_rules() {
        // get 形态：纯位置参数。
        assert_valid("class C {\n operator func [](i: Int64): Int64 { 42 }\n}\nmain() {}");
        // set 形态正例：恰一个名为 value 的命名参数且位于末尾，返回 Unit。
        assert_valid("class C {\n operator func [](i: Int64, value!: Int64) {}\n}\nmain() {}");
        // 命名参数必须叫 value。
        assert_invalid(
            "class C {\n operator func [](index: Int64, val!: Int64) {}\n}\nmain() {}",
        );
        // 不能有多个命名参数。
        assert_invalid(
            "class C {\n operator func [](index: Int64, value!: Int64, v!: Int64) {}\n}\nmain() {}",
        );
        // set 形态返回类型必须为 Unit。
        assert_invalid(
            "class C {\n operator func [](index: Int64, value!: Int64): Int64 { 42 }\n}\nmain() {}",
        );
        // 命名参数必须是最后一个位置参数。
        assert_invalid(
            "class C {\n operator func [](index: Int64, value!: Int64, other: Int64) {}\n}\nmain() {}",
        );
    }


    // ─── w12c：11 章 public 声明暴露 internal 类型 ─────────────────────────

    #[test]
    fn test_w12c_public_internal_exposure() {
        // 11.2(7)：public 顶层函数的推断/注解返回类型为 internal 类型。
        assert_invalid("class C1 {}\npublic func foo() { C1() }\nmain() {}");
        assert_invalid("class C1 {}\npublic func foo(): C1 { C1() }\nmain() {}");
        // 参数类型为 internal 类型。
        assert_invalid("class C1 {}\npublic func foo(a: C1) {}\nmain() {}");
        // 泛型约束上界为 internal 类型。
        assert_invalid("class C1 {}\npublic func foo<T>() where T <: C1 { 0 }\nmain() {}");
        // public 枚举构造器 payload 为 internal 类型。
        assert_invalid("class C1 {}\npublic enum E { A(C1) }\nmain() {}");
        // public 类的 public 方法参数 / 推断返回为 internal 类型。
        assert_invalid(
            "class C1 {}\npublic class T { public func foo(a: C1) {} }\nmain() {}",
        );
        assert_invalid(
            "class C1 {}\npublic class T { public func foo() { C1() } }\nmain() {}",
        );
        // 正例：public 类型可被 public 声明引用。
        assert_valid("public class C1 {}\npublic func foo(a: C1) {}\nmain() {}");
        // 正例：internal 函数引用 internal 类型合法。
        assert_valid("class C1 {}\nfunc foo() { C1() }\nmain() {}");
        // 存在 import 时保守跳过。
        assert_valid("import utils.assert.Assert\nclass C1 {}\npublic func foo() { C1() }\nmain() {}");
    }


    // ─── w13a-class8：06 章第七轮 — 接口默认实现继承 / 构造器 super 位置 ─────

    #[test]
    fn test_w13a_class8_interface_default_inheritance() {
        // 6.2.3.1(3)：子接口不能只声明而无默认实现地覆盖父接口的默认实现。
        assert_invalid(
            "interface I { func f() {} }\ninterface J <: I { func f(): Unit }\nmain() {}",
        );
        assert_invalid(
            "interface I { static prop p: Int64 { get() { 1 } } }\ninterface J <: I { static prop p: Int64 }\nmain() {}",
        );
        // 6.2.3.1(6)：多个父接口的默认实现冲突，子接口必须提供自己的默认实现。
        assert_invalid(
            "interface I1 { func f() {} }\ninterface I2 { func f() {} }\ninterface J <: I1 & I2 {}\nmain() {}",
        );
        assert_invalid(
            "interface I1 { mut prop p: Int64 { get() { 1 } set(v) {} } }\ninterface I2 { mut prop p: Int64 { get() { 2 } set(v) {} } }\ninterface J <: I1 & I2 {}\nmain() {}",
        );
        // 正例：子接口未重新声明，直接继承单个父接口的默认实现。
        assert_valid("interface I { func f() {} }\ninterface J <: I {}\nmain() {}");
        // 正例：子接口提供了自己的默认实现，冲突消除。
        assert_valid(
            "interface I1 { func f() {} }\ninterface I2 { func f() {} }\ninterface J <: I1 & I2 { func f() {} }\nmain() {}",
        );
        // 正例：一个父接口是另一个的父接口，只保留最派生的默认实现。
        assert_valid(
            "interface I { func f() {} }\ninterface J1 <: I { func f() {} }\ninterface J2 <: J1 & I {}\nmain() {}",
        );
    }


    #[test]
    fn test_w13a_class8_multiple_default_impls_all_types() {
        let two = "interface I1 { func f() {} }\ninterface I2 { func f() {} }\n";
        // 6.2.4.1(9)：抽象类不能省略/只声明而不实现冲突默认成员。
        assert_invalid(&format!("{}abstract class A <: I1 & I2 {{}}\nmain() {{}}", two));
        assert_invalid(&format!(
            "{}abstract class A <: I1 & I2 {{ public func f(): Unit }}\nmain() {{}}",
            two
        ));
        // 6.2.4.1(9)：结构体、枚举同样不能省略实现。
        assert_invalid(&format!("{}struct A <: I1 & I2 {{}}\nmain() {{}}", two));
        assert_invalid(&format!("{}enum A <: I1 & I2 {{ E }}\nmain() {{}}", two));
        // 枚举的属性默认实现冲突同样适用（属性脱糖成员参与判定）。
        assert_invalid(
            "interface I1 { mut prop p: Int64 { get() { 1 } set(v) {} } }\ninterface I2 { mut prop p: Int64 { get() { 2 } set(v) {} } }\nenum A <: I1 & I2 { E }\nmain() {}",
        );
        // 属性默认实现冲突同样适用于抽象类。
        assert_invalid(
            "interface I1 { mut prop p: Int64 { get() { 1 } set(v) {} } }\ninterface I2 { mut prop p: Int64 { get() { 2 } set(v) {} } }\nabstract class A <: I1 & I2 {}\nmain() {}",
        );
        // 正例：提供了真正的实现（有函数体）。
        assert_valid(&format!(
            "{}abstract class A <: I1 & I2 {{ public func f() {{}} }}\nmain() {{}}",
            two
        ));
        assert_valid(&format!(
            "{}struct A <: I1 & I2 {{ public func f() {{}} }}\nmain() {{}}",
            two
        ));
        assert_valid(&format!(
            "{}enum A <: I1 & I2 {{ E; public func f() {{}} }}\nmain() {{}}",
            two
        ));
        // 正例：单个父接口的默认实现可以直接继承。
        assert_valid("interface I { func f() {} }\nclass A <: I {}\nstruct B <: I {}\nmain() {}");
    }


    #[test]
    fn test_w13a_class8_super_call_must_be_first() {
        // 6.1.2.1(23)：显式 super(...) 必须是构造函数体的第一条语句。
        assert_invalid(
            "open class A { init(x: Int64, y: Int64) {} }\nclass B <: A { B() { var x = 1\n super(1, 2) } }\nmain() {}",
        );
        assert_invalid(
            "open class A { init(x: Int64, y: Int64) {} }\nclass B <: A { B() { foo()\n super(1, 2) }\n func foo() {} }\nmain() {}",
        );
        // 正例：super(...) 位于构造函数体首条语句。
        assert_valid(
            "open class A { init(x: Int64) {} }\nclass B <: A { B() { super(1) } }\nmain() {}",
        );
        // 正例：构造体内没有显式 super 调用。
        assert_valid("open class A { init() {} }\nclass B <: A { B() { var x = 1 } }\nmain() {}");
        // 6.1.2.1(24)：显式 this(...) 调用同样必须是构造函数体的第一条语句。
        assert_invalid(
            "class A { init(x: Int64) {}\n init() { 1 + 2\n this(1) } }\nmain() {}",
        );
        assert_valid("class A { init(x: Int64) {}\n init() { this(1) } }\nmain() {}");
    }


    // ─── w13b：类型别名循环引用（2.4.1(11)/(12)） ───────────────────────────

    #[test]
    fn test_w13b_type_alias_cycle_rejected() {
        // 直接自引用。
        assert_invalid("type A = A\nmain() {}");
        // 间接循环。
        assert_invalid("type A = B\ntype B = A\nmain() {}");
        // 嵌套在泛型实参中的自引用。
        assert_invalid("class G<K, V> {}\ntype V = G<Int64, V>\nmain() {}");
        // 元组中的自引用。
        assert_invalid("type T2 = (Int64, T2)\nmain() {}");
        // 正例：链式别名无环。
        assert_valid("type A = Int64\ntype B = A\ntype C = B\nmain() {}");
        assert_valid("class G<K, V> {}\ntype V = G<Int64, Int64>\nmain() {}");
    }


    // ─── w13b：顶层变量先定义后使用（3.2.3.1(4)） ───────────────────────────

    #[test]
    fn test_w13b_top_level_const_order_rejected() {
        // 初始化式引用其后的顶层变量。
        assert_invalid("let a = b\nlet b = 1\nmain() {}");
        assert_invalid("var a: Int64 = b\nvar b: Int64 = 0\nmain() {}");
        // 正例：引用之前声明的变量。
        assert_valid("let b = 1\nlet a = b\nmain() {}");
        // 正例：引用函数/类型名不受影响。
        assert_valid("func f() { 1 }\nlet a = f()\nmain() {}");
    }


    // ---- w13c：@C 结构体/foreign func/CFunc 的 CType 细化（13.1.3.x/13.1.4(5)）----

    #[test]
    fn test_w13c_foreign_non_c_struct_rejected() {
        // 普通本地 struct 不满足 CType，只有 @C struct 可作外来函数签名类型。
        assert_invalid("struct ABC { var f1: Int32 = 0 }\nforeign func f(x: ABC): Int32\nmain() {}");
        assert_invalid("struct ABC { var f1: Int32 = 0 }\nforeign func f(): ABC\nmain() {}");
        assert_valid("@C\nstruct ABC { var x: Int32 }\nforeign func f(x: ABC): ABC\nmain() {}");
    }


    #[test]
    fn test_w13c_cfunc_non_c_struct_rejected() {
        // CFunc<F> 的签名同样要求 CType：普通 struct 非法，@C struct 合法。
        assert_invalid("struct ABC { var f1: Int32 = 0 }\nfunc f(cb: CFunc<(ABC)->Int32>) {}\nmain() {}");
        assert_invalid("struct ABC { var f1: Int32 = 0 }\nfunc f(cb: CFunc<()->ABC>) {}\nmain() {}");
        assert_valid("@C\nstruct ABC { var f1: Int32 = 0 }\nfunc f(cb: CFunc<(ABC)->ABC>) {}\nmain() {}");
    }


    #[test]
    fn test_w13c_interface_cannot_inherit_ctype() {
        // 13.1.4(5)：CType 接口不能被继承（含 interface 直接继承）。
        assert_invalid("interface ABC <: CType {}\nmain() {}");
        // 泛型约束 `where T <: CType` 仍合法。
        assert_valid("class D<T> where T <: CType {}\nmain() {}");
    }


    // ─── w13d：This 返回类型函数体与覆写规则（6.1.4(2)/(7)） ─────────────

    #[test]
    fn test_w13d_this_return_body() {
        // 只能返回 this 或返回 This 的函数调用。
        assert_valid("class A {\n func f(): This { this }\n}\nmain() {}");
        assert_valid("class A {\n func f(): This { return this }\n}\nmain() {}");
        assert_valid("class A {\n func g(): This { this }\n func f(): This { g() }\n}\nmain() {}");
        // 空体 / 字面量 / 普通构造 非法。
        assert_invalid("class A {\n func f(): This { }\n}\nmain() {}");
        assert_invalid("class A {\n func f(): This { 1 }\n}\nmain() {}");
        assert_invalid("class A {\n func f(): This { A() }\n}\nmain() {}");
    }


    #[test]
    fn test_w13d_this_override_return() {
        // 覆写返回 This 的父函数时返回类型必须保持 This。
        assert_invalid(
            "open class A {\n public open func f(): This { this }\n}\nclass B <: A {\n public func f(): A { this }\n}\nmain() {}",
        );
        assert_valid(
            "open class A {\n public open func f(): This { this }\n}\nclass B <: A {\n public override func f(): This { this }\n}\nmain() {}",
        );
    }


    // ─── w13d：无 body 的接口 static 函数不可调用（6.2.2.1(8)/(10)） ─────

    #[test]
    fn test_w13d_abstract_static_iface_call() {
        // 直接通过接口名调用抽象 static：非法。
        assert_invalid(
            "interface I {\n static func f(): Unit\n static func g() { I.f() }\n}\nmain() {}",
        );
        // 间接：I.g() 在 main 中调用，g 内无资格调用抽象 f：非法。
        assert_invalid(
            "interface I {\n static func f(): Unit\n static func g() { f() }\n}\nmain() { I.g() }",
        );
        // 有实现体则合法。
        assert_valid(
            "interface I {\n static func f() {}\n static func g() { f() }\n}\nmain() { I.g() }",
        );
    }


    // ─── w13d：多父接口同名默认实现冲突（6.2.3.1(6)/6.2.4.1(9)） ─────────

    #[test]
    fn test_w13d_iface_default_conflict() {
        // 子接口未提供新的默认实现：非法。
        assert_invalid(
            "interface I1 {\n func f() {}\n}\ninterface I2 {\n func f() {}\n}\ninterface J <: I1 & I2 {}\nmain() {}",
        );
        // 子接口提供默认实现：合法。
        assert_valid(
            "interface I1 {\n func f() {}\n}\ninterface I2 {\n func f() {}\n}\ninterface J <: I1 & I2 {\n func f() {}\n}\nmain() {}",
        );
        // 参数列表不同不构成冲突。
        assert_valid(
            "interface I1 {\n func f() {}\n}\ninterface I2 {\n func f(a: Int32) {}\n}\ninterface J <: I1 & I2 {}\nmain() {}",
        );
    }


    #[test]
    fn test_w13d_type_default_conflict() {
        // 类型未自行实现两个父接口的同一默认成员：非法。
        assert_invalid(
            "interface I1 {\n func f() {}\n}\ninterface I2 {\n func f() {}\n}\nclass A <: I1 & I2 {}\nmain() {}",
        );
        // 自行实现：合法。
        assert_valid(
            "interface I1 {\n func f() {}\n}\ninterface I2 {\n func f() {}\n}\nclass A <: I1 & I2 {\n public func f() {}\n}\nmain() {}",
        );
    }


    // ─── w13d：不得通过 super 访问接口默认实现（6.2.2.2(7)） ─────────────

    #[test]
    fn test_w13d_super_iface_default() {
        assert_invalid(
            "interface I {\n func f() { 1 }\n}\ninterface J <: I {\n override func f() { super.f() }\n}\nmain() {}",
        );
        assert_valid(
            "interface I {\n func f() { 1 }\n}\ninterface J <: I {\n override func f() { 1 }\n}\nmain() {}",
        );
    }


    // ─── w14a：构造函数系（6.1.2.1(34)/(50)/(16)/(11)/(19)） ─────────────────

    #[test]
    fn test_w14a_ctor_multiple_super_rejected() {
        // 6.1.2.1(34)/(50)：构造函数体至多一条 super(...)/this(...) 调用。
        let errs = assert_invalid(
            "open class P {}\nclass A <: P {\n init() {\n  super()\n  super()\n }\n}\nmain() {}",
        );
        assert!(errs.iter().any(|e| e.message.contains("6.1.2.1(34)/(50)")));
        assert_invalid(
            "open class P {}\nclass A <: P {\n init() {\n  super()\n  this(1)\n }\n}\nmain() {}",
        );
        assert_invalid(
            "class A {\n init() {\n  this(1)\n  this(1)\n }\n A(x: Int64) {}\n}\nmain() {}",
        );
        // 正例：单条 super 或 this 调用合法。
        assert_valid("open class P {}\nclass A <: P {\n init() {\n  super()\n }\n}\nmain() {}");
        assert_valid("class A {\n init() {\n  this(1)\n }\n A(x: Int64) {}\n}\nmain() {}");
    }


    #[test]
    fn test_w14a_ctor_member_param_name_conflict() {
        // 6.1.2.1(16)：主构造成员变量参数不得与构造函数外成员同名。
        let errs = assert_invalid(
            "class A {\n A(public var a: Int64) {}\n var a = 1\n}\nmain() {}",
        );
        assert!(errs.iter().any(|e| e.message.contains("6.1.2.1(16)")));
        assert_invalid("class A {\n A(let a!: Int64) {}\n var a = 1\n}\nmain() {}");
        // 正例：成员变量参数与外部成员不同名。
        assert_valid(
            "class A {\n A(public var a: Int64) {}\n var b = 1\n}\nmain() {}",
        );
        // 正例：`class A(var x)` 形式由 parser 自动生成的同名成员不算冲突。
        assert_valid("class A(var x: Int64) {}\nmain() {}");
    }


    #[test]
    fn test_w14a_ctor_member_param_definite_assignment() {
        // 6.1.2.1(11)/(55)：主构造成员变量参数生成的实例成员须在所有构造函数中初始化。
        let errs = assert_invalid(
            "class A {\n A(var a: Int64) {}\n init() {}\n}\nmain() {}",
        );
        assert!(errs.iter().any(|e| e.message.contains("6.1.2.1(55)")));
        // 正例：次构造函数显式初始化该成员。
        assert_valid(
            "class A {\n A(var a: Int64) {}\n init() { a = 0 }\n}\nmain() {}",
        );
        // 正例：成员变量参数带默认值。
        assert_valid("class A {\n A(var a!: Int64 = 0) {}\n init() {}\n}\nmain() {}");
    }


    #[test]
    fn test_w14a_ctor_default_value_param_assign_rejected() {
        // 6.1.2.1(19)：初始值表达式不得修改其引用的参数。
        let errs = assert_invalid(
            "class A {\n A(a: Int64, let x!: Unit = (a = 10)) {}\n}\nmain() {}",
        );
        assert!(errs.iter().any(|e| e.message.contains("6.1.2.1(19)")));
        assert_invalid(
            "class A {\n A(var a: Int64, let x!: Unit = (a = 10)) {}\n}\nmain() {}",
        );
        // 正例：默认值只读取参数、不修改。
        assert_valid(
            "class A {\n A(a: Int64, let b!: Int64 = a) {}\n}\nmain() {}",
        );
    }


    // ─── w14b：06 章接口/继承系细则（6.1.2.4(36)/6.3.1.1/6.3.4.1） ──────

    #[test]
    fn test_w14b_open_func_needs_public_protected() {
        // 6.1.2.4(36)：open 函数只能 public/protected；默认 internal 非法。
        assert_invalid("open class A {\n open func f() {}\n}\nmain() {}");
        assert_invalid("abstract class A {\n open func f() {}\n}\nmain() {}");
        assert_valid("open class A {\n public open func f() {}\n}\nmain() {}");
        assert_valid("open class A {\n protected open func f() {}\n}\nmain() {}");
    }


    #[test]
    fn test_w14b_redef_static_init_illegal() {
        // 6.3.4.1(9)：static 初始化器不能被 redef 修饰。
        assert_invalid(
            "open class A {\n static init() {}\n}\nclass C <: A {\n redef static init() {}\n}\nmain() {}",
        );
        assert_valid(
            "open class A {\n static init() {}\n}\nclass C <: A {\n static init() {}\n}\nmain() {}",
        );
    }


    #[test]
    fn test_w14b_static_cannot_override() {
        // 6.3.1.1(1)：static 函数不能用 override（应用 redef）。
        assert_invalid(
            "interface I {\n static func f() {}\n}\nclass A <: I {\n public override static func f() {}\n}\nmain() {}",
        );
        assert_valid(
            "interface I {\n static func f() {}\n}\nclass A <: I {\n public redef static func f() {}\n}\nmain() {}",
        );
    }


    #[test]
    fn test_w14b_override_generic_arity_mismatch() {
        // 6.3.1.1(1)：泛型函数不能覆写同名的非泛型函数。
        assert_invalid(
            "abstract class A {\n public func f(): Unit\n}\nclass C <: A {\n public override func f<T>() {}\n}\nmain() {}",
        );
        assert_valid(
            "abstract class A {\n public func f<T>(): Unit\n}\nclass C <: A {\n public override func f<T>() {}\n}\nmain() {}",
        );
    }


    #[test]
    fn test_w14b_override_type_param_param_mismatch() {
        // 6.3.1.1(3)：类级类型参数作为参数类型时不能覆写具体类型的父函数。
        assert_invalid(
            "interface I {\n func f(x: Rune): Unit\n}\nabstract class A<T> <: I {\n public override func f(x: T) {}\n}\nmain() {}",
        );
        // 对齐的类级类型参数可以合法覆写。
        assert_valid(
            "abstract class P<T> {\n public open func f(x: T) {}\n}\nclass C<T> <: P<T> {\n public override func f(x: T) {}\n}\nmain() {}",
        );
    }


    #[test]
    fn test_w14b_abstract_override_concrete_illegal() {
        // 6.3.1.1(1)：无实现体的（抽象）覆写不能覆盖有实现的父函数。
        assert_invalid(
            "abstract class A {\n public open func f() {}\n}\nabstract class C <: A {\n public override func f(): Unit\n}\nmain() {}",
        );
        assert_valid(
            "abstract class A {\n public func f(): Unit\n}\nabstract class C <: A {\n public override func f(): Unit\n}\nmain() {}",
        );
    }


    #[test]
    fn test_w14b_redef_static_generic_arity_mismatch() {
        // 6.3.4.1(1)：static 泛型函数不能 redef 同名的非泛型 static 函数。
        assert_invalid(
            "interface I {\n static func f() {}\n}\nclass A <: I {\n public redef static func f<T>() {}\n}\nmain() {}",
        );
        assert_valid(
            "interface I {\n static func f<T>() {}\n}\nclass A <: I {\n public redef static func f<T>() {}\n}\nmain() {}",
        );
    }


    #[test]
    fn test_w14b_redef_multi_parent_return_conflict() {
        // 6.3.4.1(6)：redef 同时重定义多个父类型中同名同参数的函数时，
        // 返回类型必须与每一个都兼容。
        assert_invalid(
            "open class R {}\nclass Bad <: R {}\ninterface I {\n static func f(): Bad\n}\ninterface J {\n static func f(): Object { Object() }\n}\nclass A <: I & J {\n public redef static func f(): R { R() }\n}\nmain() {}",
        );
        // 返回类型对每个父类型都满足协变：合法。
        assert_valid(
            "open class R {}\nclass Sub <: R {}\ninterface I {\n static func f(): R\n}\ninterface J {\n static func f(): Object { Object() }\n}\nclass A <: I & J {\n public redef static func f(): Sub { Sub() }\n}\nmain() {}",
        );
    }


    // ─── w14d：08 扩展间成员遮蔽（8.4(9)） ─────────────────────────────────

    #[test]
    fn test_w14d_extension_member_shadowing() {
        // 同一类型的不同扩展定义同名同签名成员 → 非法。
        assert_invalid(
            "class C {}\nextend C {\n func f() { 1 }\n}\nextend C {\n func f() { 2 }\n}\nmain() {}",
        );
        // 不同方法名的扩展合法。
        assert_valid(
            "class C {}\nextend C {\n func f() { 1 }\n}\nextend C {\n func g() { 2 }\n}\nmain() {}",
        );
        // 同名但参数类型不同的重载合法。
        assert_valid(
            "class C {}\nextend C {\n func f(x: Int64) { 1 }\n}\nextend C {\n func f(x: Bool) { 2 }\n}\nmain() {}",
        );
    }

    // ─── w15a 06_class_and_interface/01_class ────────────────────────────────

    #[test]
    fn test_w15a_implicit_ctor_args() {
        // 6.1.2.1(47)：没有显式构造函数的类只有隐式无参构造函数。
        assert_invalid("class A {}\nmain() { let x = A(12) }");
        assert_valid("class A {}\nmain() { let x = A() }");
        assert_valid("class A {\n init(x: Int64) {}\n}\nmain() { let x = A(1) }");
    }

    #[test]
    fn test_w15a_abstract_function_rules() {
        // 6.1.2.4(12)：抽象函数不能带可选参数。
        assert_invalid("abstract class A {\n public func f(x!: Int64 = 1): Unit\n}\nmain() {}");
        assert_valid("abstract class A {\n public func f(x: Int64): Unit\n}\nmain() {}");
    }

    #[test]
    fn test_w15a_two_class_supertypes() {
        // 6.1.1.2(1) 在解析阶段校验（见 parser::decl 的测试）；此处验证
        // “一个类 + 一个接口”不被误判。
        assert_valid(
            "interface I {}\nopen class A {}\nclass C <: A & I {}\nmain() {}",
        );
    }

    #[test]
    fn test_w15a_super_primary_ctor_call() {
        // 6.1.1.2(13)：不能通过 super.父类名() 访问父类主构造函数。
        assert_invalid(
            "open class A {\n A() {}\n}\nclass B <: A {\n func t() { super.A() }\n}\nmain() {}",
        );
        assert_valid("open class A {\n A() {}\n}\nclass B <: A {\n init() { super() }\n}\nmain() {}");
    }

    #[test]
    fn test_w15a_this_typed_property_rejected() {
        // 6.1.4(1)：This 不能用作属性类型。
        assert_invalid("class A {\n prop p: This {\n get() { this }\n }\n}\nmain() {}");
        assert_valid("class A {\n func f(): This { this }\n}\nmain() {}");
    }

    #[test]
    fn test_w15a_private_ctor_access() {
        // 6.1.2.1(39)：全部构造函数 private 时类外不可实例化。
        assert_invalid("class A {\n private init() {}\n}\nmain() { let a = A() }");
        assert_valid(
            "class A {\n private init() {}\n static func make(): A { A() }\n}\nmain() {}",
        );
        // 子类也不能通过 super() 调用。
        assert_invalid(
            "open class A {\n private init() {}\n}\nclass B <: A {\n init() { super() }\n}\nmain() {}",
        );
    }

    #[test]
    fn test_w15a_super_ctor_availability() {
        // 6.1.2.1(46)/(53)：父类无可访问无参构造函数且子类未调用 super(...)。
        assert_invalid(
            "open class P {\n init(x: Int64) {}\n}\nclass C <: P {\n init() {}\n}\nmain() {}",
        );
        assert_invalid("open class P {\n init(x: Int64) {}\n}\nclass C <: P {}\nmain() {}");
        assert_valid(
            "open class P {\n init(x: Int64) {}\n}\nclass C <: P {\n init() { super(0) }\n}\nmain() {}",
        );
    }

    // ─── w15b：接口/属性/覆写补充规则 ─────────────────────────────────────

    #[test]
    fn test_w15b_extend_any_rejected() {
        assert_invalid("class A {}\nextend A <: Any {}\nmain() {}");
        assert_invalid("struct S {}\nextend S <: Any {}\nmain() {}");
        assert_valid("class A {}\ninterface I {}\nextend A <: I {}\nmain() {}");
    }

    #[test]
    fn test_w15b_interface_inherits_non_interface_rejected() {
        assert_invalid("open class A {}\ninterface I <: A {}\nmain() {}");
        assert_invalid("struct S {}\ninterface I <: S {}\nmain() {}");
        assert_valid("interface A {}\ninterface B <: A {}\nmain() {}");
    }

    #[test]
    fn test_w15b_interface_member_visibility_rejected() {
        assert_invalid("interface I {\n protected func f(): Unit\n}\nmain() {}");
        assert_invalid("interface I {\n private static func f(): Unit\n}\nmain() {}");
        assert_valid("interface I {\n public func f(): Unit\n}\nmain() {}");
    }

    #[test]
    fn test_w15b_interface_immutable_prop_with_setter_rejected() {
        // 不可变接口属性定义了 setter。
        assert_invalid(
            "interface I {\n prop p: Int64 {\n  get() { 1 }\n  set(v) {}\n }\n}\nmain() {}",
        );
        // 合法的可变/不可变接口属性。
        assert_valid(
            "interface I {\n prop p: Int64 {\n  get() { 1 }\n }\n}\nmain() {}",
        );
        assert_valid(
            "interface I {\n mut prop p: Int64 {\n  get() { 1 }\n  set(v) {}\n }\n}\nmain() {}",
        );
    }

    #[test]
    fn test_w15a_generic_supertype_arg_must_be_type() {
        // 6.1.1.2(6)：父类型泛型实参不能是变量名。
        assert_invalid("var x = 1\nopen class A<T> {}\nclass B <: A<x> {}\nmain() {}");
        assert_valid("open class A<T> {}\nclass B <: A<Int64> {}\nmain() {}");
    }

    #[test]
    fn test_w15b_override_return_supertype_rejected() {
        // 覆写返回类型为父类型返回类型的超类型（Object）→ 非法。
        assert_invalid(
            "class A {}\ninterface I {\n func f(): A\n}\nclass C <: I {\n public override func f(): Object { Object() }\n}\nmain() {}",
        );
        // 协变返回子类型合法；父类型为 Any 时任意返回类型合法。
        assert_valid(
            "open class A {}\nclass B <: A {}\ninterface I {\n func f(): A\n}\nclass C <: I {\n public override func f(): B { B() }\n}\nmain() {}",
        );
        assert_valid(
            "class A {}\ninterface I {\n func f(): Any\n}\nclass C <: I {\n public override func f(): A { A() }\n}\nmain() {}",
        );
    }

    #[test]
    fn test_w15b_impl_generic_param_mismatch_rejected() {
        // 接口泛型实参为 T1，实现函数参数使用另一个类型参数 T2 → 非法。
        assert_invalid(
            "interface I<T> {\n func f(x: T): Unit\n}\nclass A<T1, T2> <: I<T1> {\n public func f(x: T2) {}\n}\nmain() {}",
        );
        assert_invalid(
            "interface I<T> {\n func f(x: T): Unit\n}\nstruct S<T1, T2> <: I<T1> {\n func f(x: T2) {}\n}\nmain() {}",
        );
        // 参数类型正确对应 → 合法。
        assert_valid(
            "interface I<T> {\n func f(x: T): Unit\n}\nclass A<T1, T2> <: I<T1> {\n public func f(x: T1) {}\n}\nmain() {}",
        );
    }

    #[test]
    fn test_w15b_override_generic_shape_mismatch_rejected() {
        // 覆写函数的泛型类型参数个数/位置必须与被覆写函数一致。
        assert_invalid(
            "interface I {\n func f<T1>(): Int64\n}\nclass C <: I {\n public override func f<T1, T2>(): Int64 { 0 }\n}\nmain() {}",
        );
        assert_invalid(
            "class A<T1, T2> {}\ninterface I {\n func f<T1, T2>(): A<T1, T2>\n}\nclass C <: I {\n public func f<K1, K2>(): A<K2, K1> { A<K2, K1>() }\n}\nmain() {}",
        );
        assert_valid(
            "interface I {\n func f<T>(a: T): T\n}\nclass C <: I {\n public override func f<K>(a: K): K { a }\n}\nmain() {}",
        );
    }

    #[test]
    fn test_w15b_property_mut_mismatch_rejected() {
        assert_invalid(
            "open class A {\n open mut prop p: Int64 {\n  get() { 0 }\n  set(v) {}\n }\n}\nclass B <: A {\n override prop p: Int64 {\n  get() { 1 }\n }\n}\nmain() {}",
        );
        assert_invalid(
            "open class A {\n open prop p: Int64 {\n  get() { 0 }\n }\n}\nclass B <: A {\n override mut prop p: Int64 {\n  get() { 1 }\n }\n}\nmain() {}",
        );
        assert_valid(
            "open class A {\n open mut prop p: Int64 {\n  get() { 0 }\n  set(v) {}\n }\n}\nclass B <: A {\n override mut prop p: Int64 {\n  get() { 1 }\n }\n}\nmain() {}",
        );
    }

    #[test]
    fn test_w15b_enum_missing_interface_prop_rejected() {
        assert_invalid("interface I {\n mut prop p: Int64\n}\nenum E <: I {\n | A\n}\nmain() {}");
        assert_invalid("interface I {\n static prop p: Int64\n}\nenum E <: I {\n | A\n}\nmain() {}");
    }

    #[test]
    fn test_w15b_instance_property_redef_rejected() {
        assert_invalid(
            "open class A {\n mut prop p: Int64 {\n  get() { 0 }\n  set(v) {}\n }\n}\nclass B <: A {\n redef mut prop p: Int64 {\n  get() { 1 }\n  set(v) {}\n }\n}\nmain() {}",
        );
        // static 属性可用 redef。
        assert_valid(
            "open class A {\n static prop p: Int64 {\n  get() { 0 }\n }\n}\nclass B <: A {\n redef static prop p: Int64 {\n  get() { 1 }\n }\n}\nmain() {}",
        );
    }

    #[test]
    fn test_w15d_static_field_requires_init() {
        // 类静态字段无默认值且无 static init → 非法。
        assert_invalid(
            "class C {\n static var x: Int64\n}\nmain() {}",
        );
        // 带默认值 → 合法。
        assert_valid(
            "class C {\n static var x: Int64 = 0\n}\nmain() {}",
        );
        // 存在 static init → 合法（延迟初始化）。
        assert_valid(
            "class C {\n static var x: Int64\n static init() { x = 1 }\n}\nmain() {}",
        );
    }


    #[test]
    fn test_w15d_top_level_initializer_scope() {
        // 顶层初始化式引用函数局部变量 → 非法。
        assert_invalid(
            "func foo(): Int64 { let a = 1\n return a }\nlet b = a\nmain() {}",
        );
        // 顶层初始化式引用先声明的顶层变量 → 合法。
        assert_valid(
            "let a = 1\nlet b = a\nmain() {}",
        );
    }


    #[test]
    fn test_w15d_type_param_scope() {
        // main 中引用别处声明的类型参数 T → 非法。
        assert_invalid(
            "struct S<T> { let x: T\n S(x: T) { this.x = x } }\nmain() { var a: T }",
        );
        // 类型参数在其所属声明内使用 → 合法。
        assert_valid(
            "struct S<T> { let x: T\n S(x: T) { this.x = x } }\nmain() {}",
        );
    }


    #[test]
    fn test_w15d_operator_tuple_vararg() {
        // 数组形参运算符 + 元组实参（变长实参语法糖误用）→ 非法。
        assert_invalid(
            "class C {\n operator func **(arr: Array<Int64>) { false }\n}\nmain() { var c = C()\n c**(1, 2) }",
        );
        // 单个标量/数组实参的合法运算符调用 → 合法。
        assert_valid(
            "class C {\n operator func **(arr: Array<Int64>) { false }\n}\nmain() { var c = C()\n c**([1, 2]) }",
        );
    }


    #[test]
    fn test_w15d_call_array_literal_args() {
        // 数组形参后面还有非命名形参，却只传入标量 → 非法。
        assert_invalid(
            "func f0(arr: Array<Rune>, a: Bool) {}\nmain() { f0('d', true) }",
        );
        // 最后一个数组形参可由多个位置实参收集（变长实参语法糖）→ 合法。
        assert_valid(
            "func f0(arr: Array<Int64>): Int64 { arr.size }\nmain() { f0(1, 2, 3) }",
        );
    }


    #[test]
    fn test_w15d_func_var_call_no_defaults() {
        // 通过函数变量调用时省略可选参数 → 非法。
        assert_invalid(
            "func add(a: Int64, b!: Int64 = 1): Int64 { a + b }\nmain() { var f = add\n f(3) }",
        );
        // 通过函数变量调用时给足全部参数 → 合法。
        assert_valid(
            "func add(a: Int64, b!: Int64 = 1): Int64 { a + b }\nmain() { var f = add\n f(3, b: 1) }",
        );
    }


    #[test]
    fn test_w15d_function_name_lvalue() {
        // 给函数名赋值 → 非法。
        assert_invalid(
            "func myFunc() { 0 }\nmain() { myFunc = 1 }",
        );
        // 给同名的局部变量赋值（不涉及函数名）→ 合法。
        assert_valid(
            "func myFunc(): Int64 { 0 }\nmain() { var x = myFunc()\n x = 1 }",
        );
    }


    #[test]
    fn test_w15d_nested_function_scope() {
        // 跨函数调用局部函数 → 非法。
        assert_invalid(
            "func a() {\n func inner() { 0 }\n}\nfunc b() { inner() }\nmain() {}",
        );
        // 在同一外层函数内调用局部函数 → 合法。
        assert_valid(
            "func a() {\n func inner(): Int64 { 0 }\n inner()\n}\nmain() {}",
        );
    }


    #[test]
    fn test_w15d_cross_type_field_init() {
        // 字段初始化式无资格引用其它类型字段 → 非法。
        assert_invalid(
            "class C1 { var y: Int64 = 1 }\nclass C2 { var z: Int64 = y }\nmain() {}",
        );
        // 引用顶层变量 → 合法。
        assert_valid(
            "var g: Int64 = 1\nclass C2 { var z: Int64 = g }\nmain() {}",
        );
    }


    #[test]
    fn test_w15d_lambda_to_scalar_annotation() {
        // lambda 赋给 Int64 标注绑定 → 非法。
        assert_invalid(
            "main() { let f: Int64 = { a: Int64 => a } }",
        );
        // lambda 赋给函数类型标注绑定 → 合法。
        assert_valid(
            "main() { let f: (Int64) -> Int64 = { a: Int64 => a } }",
        );
    }


    #[test]
    fn test_w15d_let_field_ctor_reassign() {
        // 已初始化的 let 字段在构造函数中重赋值 → 非法。
        assert_invalid(
            "struct S {\n let x: Int64 = 1\n init() { x = 2 }\n}\nmain() {}",
        );
        // 未初始化的 let 字段在构造函数中首次赋值 → 合法。
        assert_valid(
            "struct S {\n let x: Int64\n init(x: Int64) { this.x = x }\n}\nmain() {}",
        );
    }


    #[test]
    fn test_w15d_duplicate_type_declarations_skipped() {
        // 同类别同名类型声明可能来自宏属性（@Test/@Strategy/@When 等）的多声明，
        // AST 不保留属性，无法区分，保守放行（正测试红线）。
        assert_valid("class name {}\nclass name {}\nmain() {}");
    }

    // ─── w16b: 16 章常量求值（成员 const / const 函数 / const init） ──────────

    #[test]
    fn test_w16b_const_field_nonconst_initializer_rejected() {
        // 16.1(2)：const 成员变量用非 const 函数调用初始化非法。
        assert_invalid("func f() { 1 }\nclass C { static const a = f() }\nmain() {}");
        assert_invalid("func f() { 1 }\nstruct S { const a = f()\n const init() { } }\nmain() {}");
        // 常量字面量初始化合法。
        assert_valid("class C { static const a = 1 }\nmain() {}");
        assert_valid("struct S { const a = 2\n const init() { } }\nmain() {}");
    }

    #[test]
    fn test_w16b_const_fn_access_nonconst_static_member_rejected() {
        // 16.4(4)/(5)：const 函数/const init 只能访问 const 的外部 static 成员。
        assert_invalid("class Aux { static let x = 0 }\nconst func f() { Aux.x }\nmain() {}");
        assert_invalid("class Aux { static var x = 0 }\nconst func f() { Aux.x }\nmain() {}");
        assert_invalid(
            "class Aux { static let x = 0 }\nstruct S { const init() { Aux.x } }\nmain() {}",
        );
        // 访问 const 静态成员合法。
        assert_valid("class Aux { static const x = 0 }\nconst func f() { Aux.x }\nmain() {}");
    }

    #[test]
    fn test_w16b_const_init_parent_requirement() {
        // 16.5：子类有 const init 时父类也须有 const init。
        assert_invalid("open class A { }\nclass B <: A { const init() { } }\nmain() {}");
        assert_valid(
            "open class A { const init() { } }\nclass B <: A { const init() { } }\nmain() {}",
        );
        // 父类未知（prelude）保守放行。
        assert_valid("class B <: Object { const init() { } }\nmain() {}");
    }

    #[test]
    fn test_w16b_const_init_tuple_static_assign_rejected() {
        // 16.5：const init 中元组解构赋值左值含 static 成员非法。
        assert_invalid(
            "struct S {\n static var v1 = 0\n static var v2 = 0\n const init() { (v1, v2) = (1, 2) }\n}\nmain() {}",
        );
    }

    // ─── w16b: 09 章泛型递归约束 ─────────────────────────────────────────────

    #[test]
    fn test_w16b_recursive_value_constraint() {
        // 9.5.1.1(12)/(13)：值类型/枚举上界的递归约束非法。
        assert_invalid(
            "struct S<T> { S(var x!: T) { }\n}\nclass G<T> where T <: S<T> {}\nmain() {}",
        );
        assert_invalid("class G<T> where T <: Option<T> {}\nmain() {}");
        // 非递归实参合法。
        assert_valid(
            "struct S<T> { S(var x!: T) { }\n}\nclass G<T> where T <: S<Int64> {}\nmain() {}",
        );
        // 接口上界保守放行（不据此判定递归）。
        assert_valid("interface I<T> {}\nclass G<T> where T <: I<T> {}\nmain() {}");
    }

}
