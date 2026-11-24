//! 表达式 / 赋值类型检查（语义校验 Pass）
//!
//! 在语法解析之后对函数体做一次只读的类型检查，覆盖仓颉规范中
//! “运算符操作数类型”与“赋值/初始化可赋值性”两类规则：
//!
//! - 算术 `+ - * /`：两侧为同一数值类型（`+` 额外支持 String + String）；
//! - 取余 `%`：两侧为同一整数类型；
//! - 幂 `**`：底数只能为 Int64 / Float64；Int64 的指数只能为 UInt64，
//!   Float64 的指数只能为 Int64 / Float64（见 4.15(13)(14)(15)）；
//! - 比较 `== != < <= > >=`：两侧类型相同（4.16(2)）；
//! - 位运算 `& | ^`：两侧为同一整数类型；移位 `<< >>`：两侧均为整数（宽度可不同）；
//! - 逻辑 `&& ||` 与一元 `!`：操作数必须是 Bool（4.20(2)）；
//! - 一元 `-`：操作数必须是数值；一元 `~`：必须是整数；
//! - `let`/`const` 绑定不可重新赋值；`++`/`--` 只作用于可变的整数变量；
//! - 显式类型标注的初始化以及普通赋值要求类型可赋值（不隐式跨类别/跨宽度转换）。
//!
//! 设计红线：只拒绝“负样例中被拒”的形态。凡是无法确定类型的表达式
//! （用户类型、方法调用、泛型参数、Option 等）一律返回 `Unknown` 并跳过检查；
//! 若程序中定义了某个运算符的重载（`operator func +` 等），该运算符的类型
//! 规则整体关闭，避免误伤合法重载。

use super::SemanticError;
use crate::ast::{BinOp, Expr, Function, Pattern, Program, Stmt, Type, UnaryOp};
use std::collections::{HashMap, HashSet};

/// 轻量类型表示：只区分规范中参与运算符规则的基础类型，
/// 其余类型统一折叠为 `Unknown`（保守跳过）。
#[derive(Clone, Debug, PartialEq, Eq)]
enum Ty {
    Int8,
    Int16,
    Int32,
    Int64,
    IntNative,
    UInt8,
    UInt16,
    UInt32,
    UInt64,
    UIntNative,
    Float16,
    Float32,
    Float64,
    Bool,
    Rune,
    String,
    Unit,
    /// 整数字面量（可适配任意整数类型，也可适配 `**` 的 UInt64 指数）
    IntLit,
    /// 浮点字面量（可适配任意浮点类型）
    FloatLit,
    /// 用户定义类型（class/struct/interface/enum 名）：与内建具体类型互不可赋值
    User(String),
    /// 内建聚合类型（tuple/array/range/...）：不同类别之间互不可比较/赋值
    Agg(&'static str),
    /// Option<T>
    Option(Box<Ty>),
    /// 无法确定（用户类型 / 泛型 / 调用结果未知等），不参与检查
    Unknown,
}

impl Ty {
    fn name(&self) -> &str {
        match self {
            Ty::Int8 => "Int8",
            Ty::Int16 => "Int16",
            Ty::Int32 => "Int32",
            Ty::Int64 => "Int64",
            Ty::IntNative => "IntNative",
            Ty::UInt8 => "UInt8",
            Ty::UInt16 => "UInt16",
            Ty::UInt32 => "UInt32",
            Ty::UInt64 => "UInt64",
            Ty::UIntNative => "UIntNative",
            Ty::Float16 => "Float16",
            Ty::Float32 => "Float32",
            Ty::Float64 => "Float64",
            Ty::Bool => "Bool",
            Ty::Rune => "Rune",
            Ty::String => "String",
            Ty::Unit => "Unit",
            Ty::IntLit => "整数字面量",
            Ty::FloatLit => "浮点字面量",
            Ty::User(n) => n,
            Ty::Agg(a) => a,
            Ty::Option(_) => "Option",
            Ty::Unknown => "未知类型",
        }
    }

    fn is_int(&self) -> bool {
        matches!(
            self,
            Ty::Int8
                | Ty::Int16
                | Ty::Int32
                | Ty::Int64
                | Ty::IntNative
                | Ty::UInt8
                | Ty::UInt16
                | Ty::UInt32
                | Ty::UInt64
                | Ty::UIntNative
                | Ty::IntLit
        )
    }

    fn is_float(&self) -> bool {
        matches!(self, Ty::Float16 | Ty::Float32 | Ty::Float64 | Ty::FloatLit)
    }

    fn is_numeric(&self) -> bool {
        self.is_int() || self.is_float()
    }

    /// 是否为一个可以确定的具体基础类型（字面量不算，交由兼容性规则适配）。
    fn is_concrete(&self) -> bool {
        matches!(
            self,
            Ty::Int8
                | Ty::Int16
                | Ty::Int32
                | Ty::Int64
                | Ty::IntNative
                | Ty::UInt8
                | Ty::UInt16
                | Ty::UInt32
                | Ty::UInt64
                | Ty::UIntNative
                | Ty::Float16
                | Ty::Float32
                | Ty::Float64
                | Ty::Bool
                | Ty::Rune
                | Ty::String
                | Ty::Unit
                | Ty::User(_)
                | Ty::Agg(_)
                | Ty::Option(_)
        )
    }

    /// 是否足以参与一次“确定”的类型比较（具体类型或字面量）。
    fn is_definite(&self) -> bool {
        self.is_concrete() || matches!(self, Ty::IntLit | Ty::FloatLit)
    }

    /// 是否为可参与运算符类型规则的类型（用户类型/Option 保守跳过，
    /// 以免误伤用户自定义运算符或 std 中的重载；聚合类型仍参与，
    /// 因为 tuple/array/range 明确不是数值类型）。
    fn is_prim(&self) -> bool {
        self.is_definite() && !matches!(self, Ty::User(_) | Ty::Option(_))
    }
}

/// 若表达式是一个编译期可确定的整数字面量（含一元负号），返回其值。
///
/// 注意：词法器把 `0xFFFF_FFFF_FFFF_FFFF` 这类超出 i64 的无符号字面量
/// 也折叠成负的 i64，无法与真正的负字面量区分，此时返回 `None`（保守跳过）。
fn literal_int(e: &Expr) -> Option<i64> {
    match e {
        Expr::Integer(n) if *n >= 0 => Some(*n),
        Expr::Unary {
            op: UnaryOp::Neg,
            expr,
        } => match &**expr {
            Expr::Integer(n) if *n > 0 => n.checked_neg(),
            _ => None,
        },
        _ => None,
    }
}

/// 由 AST 类型映射到轻量类型。
fn ty_of(ty: &Type) -> Ty {
    match ty {
        Type::Int8 => Ty::Int8,
        Type::Int16 => Ty::Int16,
        Type::Int32 => Ty::Int32,
        Type::Int64 => Ty::Int64,
        Type::IntNative => Ty::IntNative,
        Type::UInt8 => Ty::UInt8,
        Type::UInt16 => Ty::UInt16,
        Type::UInt32 => Ty::UInt32,
        Type::UInt64 => Ty::UInt64,
        Type::UIntNative => Ty::UIntNative,
        Type::Float16 => Ty::Float16,
        Type::Float32 => Ty::Float32,
        Type::Float64 => Ty::Float64,
        Type::Bool => Ty::Bool,
        Type::Rune => Ty::Rune,
        Type::String => Ty::String,
        Type::Unit => Ty::Unit,
        Type::Tuple(_) => Ty::Agg("tuple"),
        Type::Array(_) => Ty::Agg("array"),
        Type::Range => Ty::Agg("range"),
        Type::Option(inner) => Ty::Option(Box::new(ty_of(inner))),
        // 少数场景下基础类型名以 Struct 形式出现（如未识别的别名），按名字兜底。
        Type::Struct(name, args) if args.is_empty() => match name.as_str() {
            "Int8" => Ty::Int8,
            "Int16" => Ty::Int16,
            "Int32" => Ty::Int32,
            "Int64" => Ty::Int64,
            "IntNative" => Ty::IntNative,
            "UInt8" => Ty::UInt8,
            "UInt16" => Ty::UInt16,
            "UInt32" => Ty::UInt32,
            "UInt64" => Ty::UInt64,
            "UIntNative" => Ty::UIntNative,
            "Float16" => Ty::Float16,
            "Float32" => Ty::Float32,
            "Float64" => Ty::Float64,
            "Bool" => Ty::Bool,
            "Rune" => Ty::Rune,
            "String" => Ty::String,
            "Unit" => Ty::Unit,
            "Array" => Ty::Agg("array"),
            "Range" => Ty::Agg("range"),
            _ => Ty::Unknown,
        },
        Type::Struct(name, args) if name == "Option" && args.len() == 1 => {
            Ty::Option(Box::new(ty_of(&args[0])))
        }
        Type::Struct(name, _args) if name == "Range" || name == "Array" => {
            if name == "Range" {
                Ty::Agg("range")
            } else {
                Ty::Agg("array")
            }
        }
        _ => Ty::Unknown,
    }
}

/// 顶层常量/变量类型映射为 Ty。
///
/// 解析器对无类型标注的顶层 `let/var` 使用 `Int64` 占位；若初始化表达式
/// 明显是其它基础类型（Bool/String/浮点/Rune），以初始化表达式为准，
/// 否则占位 `Int64` 会把合法的 `var x = "s"` 误判为整数变量。
fn global_const_ty(c: &crate::ast::ConstDef) -> Ty {
    let declared = ty_of(&c.ty);
    if c.ty == Type::Int64 {
        let init_ty = static_expr_ty(&c.init);
        if matches!(
            init_ty,
            Ty::Bool
                | Ty::String
                | Ty::Rune
                | Ty::Float16
                | Ty::Float32
                | Ty::Float64
                | Ty::FloatLit
        ) {
            return default_literal(&init_ty);
        }
    }
    declared
}

/// 不需要作用域即可推断的静态表达式类型（用于顶层常量初始化）。
fn static_expr_ty(e: &Expr) -> Ty {
    match e {
        Expr::Integer(_) => Ty::IntLit,
        Expr::Float(_) => Ty::FloatLit,
        Expr::Float32(_) => Ty::FloatLit,
        Expr::Bool(_) => Ty::Bool,
        Expr::Rune(_) => Ty::Rune,
        Expr::String(_) | Expr::Interpolate(_) => Ty::String,
        Expr::Cast { target_ty, .. } => ty_of(target_ty),
        Expr::Unary { op, expr } => match op {
            UnaryOp::Neg => static_expr_ty(expr),
            _ => Ty::Unknown,
        },
        Expr::Call { name, .. } => primitive_ctor_ty(name).unwrap_or(Ty::Unknown),
        Expr::Binary { op, left, right } => match op {
            BinOp::Add
            | BinOp::Sub
            | BinOp::Mul
            | BinOp::Div
            | BinOp::Mod
            | BinOp::Shl
            | BinOp::Shr
            | BinOp::BitAnd
            | BinOp::BitOr
            | BinOp::BitXor => merged(&static_expr_ty(left), &static_expr_ty(right)),
            BinOp::Eq
            | BinOp::NotEq
            | BinOp::Lt
            | BinOp::Gt
            | BinOp::LtEq
            | BinOp::GtEq
            | BinOp::LogicalAnd
            | BinOp::LogicalOr
            | BinOp::NotIn => Ty::Bool,
            _ => Ty::Unknown,
        },
        Expr::IsType { .. } => Ty::Bool,
        _ => Ty::Unknown,
    }
}

/// 两类型在“同类型”意义下是否兼容（字面量可适配对应类别）。
fn compatible(a: &Ty, b: &Ty) -> bool {
    match (a, b) {
        (Ty::Unknown, _) | (_, Ty::Unknown) => true,
        // Option<T> 与 Option<U> / T 之间（隐式包裹/解包比较）。
        (Ty::Option(x), Ty::Option(y)) => compatible(x, y),
        (Ty::Option(inner), other) | (other, Ty::Option(inner)) => compatible(inner, other),
        (Ty::IntLit, Ty::IntLit) => true,
        // 整数字面量可适配任意数值类型（含浮点）；这与无初始化变量
        // （如 `var x: Float16`）合成默认值 `0` 的语法形态一致。
        (Ty::IntLit, other) | (other, Ty::IntLit) => other.is_numeric(),
        (Ty::FloatLit, Ty::FloatLit) => true,
        (Ty::FloatLit, other) | (other, Ty::FloatLit) => other.is_float(),
        // 用户类型之间可能存在继承/接口实现关系，不做强弱判定。
        (Ty::User(_), Ty::User(_)) => true,
        // 聚合类型（tuple/array/range）只有同类才可赋值/比较。
        (Ty::Agg(x), Ty::Agg(y)) => x == y,
        _ => a == b,
    }
}

/// 二元算术/位运算的结果类型：优先取具体一侧，否则保留字面量标记。
/// 任一侧类型未知时结果视为未知（保守跳过后续检查）。
fn merged(a: &Ty, b: &Ty) -> Ty {
    if matches!(a, Ty::Unknown) || matches!(b, Ty::Unknown) {
        return Ty::Unknown;
    }
    // 用户类型/聚合/Option 的运算结果不做推断（可能来自重载），保守置为未知。
    if matches!(a, Ty::User(_) | Ty::Agg(_) | Ty::Option(_))
        || matches!(b, Ty::User(_) | Ty::Agg(_) | Ty::Option(_))
    {
        return Ty::Unknown;
    }
    if a.is_concrete() {
        a.clone()
    } else if b.is_concrete() {
        b.clone()
    } else if matches!(a, Ty::FloatLit) || matches!(b, Ty::FloatLit) {
        Ty::FloatLit
    } else if matches!(a, Ty::IntLit) || matches!(b, Ty::IntLit) {
        Ty::IntLit
    } else {
        Ty::Unknown
    }
}

fn is_comparison_op(op: &BinOp) -> bool {
    matches!(
        op,
        BinOp::Eq | BinOp::NotEq | BinOp::Lt | BinOp::Gt | BinOp::LtEq | BinOp::GtEq
    )
}

/// 关系运算符的结合性分组：`== !=` 一组，`< <= > >=` 一组（组内不可结合，
/// 组间因优先级不同可以嵌套）。
fn cmp_chain_level(op: &BinOp) -> Option<u8> {
    match op {
        BinOp::Eq | BinOp::NotEq => Some(0),
        BinOp::Lt | BinOp::Gt | BinOp::LtEq | BinOp::GtEq => Some(1),
        _ => None,
    }
}

/// 若表达式本身是比较运算，返回其结合性分组。
fn cmp_child_level(e: &Expr) -> Option<u8> {
    match e {
        Expr::Binary { op, .. } => cmp_chain_level(op),
        _ => None,
    }
}

fn is_comparison_expr(e: &Expr) -> bool {
    matches!(e, Expr::Binary { op, .. } if is_comparison_op(op))
}

/// 操作数是否“可静态确定”：变量、字面量、基础类型转换，以及由它们
/// 直接构成的算术/位运算表达式。方法调用、lambda、`is`、字段访问等
/// 形态返回 false，交由后续阶段处理。
fn simple_operand(e: &Expr) -> bool {
    match e {
        Expr::Integer(_)
        | Expr::Float(_)
        | Expr::Float32(_)
        | Expr::Bool(_)
        | Expr::Rune(_)
        | Expr::String(_)
        | Expr::Var(_) => true,
        Expr::Unary { expr, .. } => simple_operand(expr),
        // `as` 转换所在的表达式可能被解析器跨行合并，类型检查不可靠，跳过。
        Expr::Cast { .. } => false,
        Expr::Binary { op, left, right } => {
            matches!(
                op,
                BinOp::Add
                    | BinOp::Sub
                    | BinOp::Mul
                    | BinOp::Div
                    | BinOp::Mod
                    | BinOp::Pow
                    | BinOp::Shl
                    | BinOp::Shr
                    | BinOp::BitAnd
                    | BinOp::BitOr
                    | BinOp::BitXor
            ) && simple_operand(left)
                && simple_operand(right)
        }
        Expr::Call { name, args, .. } => {
            primitive_ctor_ty(name).is_some() && args.iter().all(simple_operand)
        }
        _ => false,
    }
}

/// 跳过类型检查时二元表达式的结果类型（保守推断）。
fn binary_result_no_check(op: &BinOp, lt: &Ty, rt: &Ty) -> Ty {
    match op {
        BinOp::Eq
        | BinOp::NotEq
        | BinOp::Lt
        | BinOp::Gt
        | BinOp::LtEq
        | BinOp::GtEq
        | BinOp::LogicalAnd
        | BinOp::LogicalOr
        | BinOp::NotIn => Ty::Bool,
        BinOp::Pow => match lt {
            Ty::IntLit | Ty::Int64 => Ty::Int64,
            Ty::FloatLit | Ty::Float64 => Ty::Float64,
            _ => Ty::Unknown,
        },
        BinOp::Shl | BinOp::Shr => {
            if lt.is_definite() {
                lt.clone()
            } else {
                merged(lt, rt)
            }
        }
        BinOp::Add
        | BinOp::Sub
        | BinOp::Mul
        | BinOp::Div
        | BinOp::Mod
        | BinOp::BitAnd
        | BinOp::BitOr
        | BinOp::BitXor => merged(lt, rt),
        BinOp::Pipeline => Ty::Unknown,
    }
}

fn bin_symbol(op: &BinOp) -> &'static str {    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Mod => "%",
        BinOp::Pow => "**",
        BinOp::Eq => "==",
        BinOp::NotEq => "!=",
        BinOp::Lt => "<",
        BinOp::Gt => ">",
        BinOp::LtEq => "<=",
        BinOp::GtEq => ">=",
        BinOp::BitAnd => "&",
        BinOp::BitOr => "|",
        BinOp::BitXor => "^",
        BinOp::Shl => "<<",
        BinOp::Shr => ">>",
        BinOp::LogicalAnd => "&&",
        BinOp::LogicalOr => "||",
        BinOp::NotIn => "!in",
        BinOp::Pipeline => "|>",
    }
}

/// 运算符对应的重载方法名（`operator func +` → `op_add`）。
fn overload_name(op: &BinOp) -> &'static str {
    match op {
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
        BinOp::LogicalAnd => "op_and",
        BinOp::LogicalOr => "op_or",
        BinOp::NotIn => "op_notin",
        BinOp::Pipeline => "op_pipeline",
    }
}

/// 变量信息（局部变量 / 参数 / 常量）。
#[derive(Clone, Debug)]
struct VarInfo {
    ty: Ty,
    mutable: bool,
    /// 若绑定由整数字面量（或其负号形态）初始化且之后未被重新赋值，
    /// 记录其编译期常量值，用于移位越界 / 字面量越界检查。
    const_int: Option<i64>,
}

/// 单趟表达式类型检查器。
struct Checker<'a> {
    globals: HashMap<String, VarInfo>,
    /// 函数名 → 返回类型（含显式标注与推断结果）
    funcs: HashMap<String, Ty>,
    /// 泛型函数名 → 作为返回类型的类型参数下标
    /// （如 `func f<T>(x: T): T` → 0），用于 `f<Int16>(..)` 的实例化返回类型。
    generic_returns: HashMap<String, usize>,
    /// 返回类型存在多种可能（重载）的函数名：调用结果按未知处理。
    ambiguous_funcs: HashSet<String>,
    /// 本程序中出现过重载的运算符方法名
    overloaded_ops: HashSet<String>,
    errors: &'a mut Vec<SemanticError>,
    scopes: Vec<HashMap<String, VarInfo>>,
    /// 用户定义类型名（class/struct/interface/enum）
    type_names: HashSet<String>,
    /// enum 类型名（enum 构造值不可赋值）
    enum_names: HashSet<String>,
    /// class 名 → 字段名 → 是否可变
    class_fields: HashMap<String, HashMap<String, bool>>,
    /// 当前函数声明的返回类型（用于 `return expr` 类型检查）
    ret_ty: Option<Ty>,
}

/// 对整个 Program 做表达式/赋值类型检查，错误追加到 `errors`。
pub(crate) fn check_program(program: &Program, errors: &mut Vec<SemanticError>) {
    // 收集用户定义类型名，用于把类型标注中的 `List` / `C` 等解析为 Ty::User。
    let mut type_names: HashSet<String> = HashSet::new();
    for s in &program.structs {
        type_names.insert(s.name.clone());
    }
    for c in &program.classes {
        type_names.insert(c.name.clone());
    }
    for i in &program.interfaces {
        type_names.insert(i.name.clone());
    }
    let mut enum_names: HashSet<String> = HashSet::new();
    for e in &program.enums {
        type_names.insert(e.name.clone());
        enum_names.insert(e.name.clone());
    }
    // class 名 → 字段名 → 是否可变（用于 `C.x = v` 静态字段赋值检查）。
    let mut class_fields: HashMap<String, HashMap<String, bool>> = HashMap::new();
    for c in &program.classes {
        class_fields.insert(
            c.name.clone(),
            c.fields.iter().map(|f| (f.name.clone(), f.is_mut)).collect(),
        );
    }

    let mut globals = HashMap::new();
    for c in &program.constants {
        globals.insert(
            c.name.clone(),
            VarInfo {
                ty: global_const_ty(c),
                // 顶层 `let`/`const` 不可重新赋值；`var` 可以。
                mutable: c.is_mut,
                const_int: literal_int(&c.init),
            },
        );
    }

    let mut funcs: HashMap<String, Ty> = HashMap::new();
    let mut generic_returns: HashMap<String, usize> = HashMap::new();
    let mut ambiguous_funcs: HashSet<String> = HashSet::new();
    collect_explicit_returns(program, &mut funcs, &mut generic_returns, &mut ambiguous_funcs);
    collect_inferred_returns(program, &mut funcs, &mut ambiguous_funcs);

    let mut overloaded_ops = HashSet::new();
    collect_operator_overloads(program, &mut overloaded_ops);

    // 10.2.3(18)：对已内建支持某运算符的基础类型，用 extend 定义同签名 operator func 属于重定义。
    check_builtin_operator_redefinitions(program, errors);

    let mut checker = Checker {
        globals,
        funcs,
        generic_returns,
        ambiguous_funcs,
        overloaded_ops,
        errors,
        scopes: Vec::new(),
        type_names,
        enum_names,
        class_fields,
        ret_ty: None,
    };

    for func in &program.functions {
        checker.check_function(func);
    }
    for cls in &program.classes {
        // 类字段先入作用域，避免方法体内同名字段被解析成同名全局变量。
        let fields: HashMap<String, VarInfo> = cls
            .fields
            .iter()
            .map(|f| {
                (
                    f.name.clone(),
                    VarInfo {
                        ty: field_ty(f),
                        mutable: true,
                        const_int: None,
                    },
                )
            })
            .collect();
        if let Some(init) = &cls.init {
            checker.check_params_body_extra(&init.params, &init.body, Some(&fields), None);
        }
        if let Some(deinit) = &cls.deinit {
            checker.scopes.push(fields.clone());
            for stmt in deinit {
                checker.check_stmt(stmt);
            }
            checker.scopes.pop();
        }
        if let Some(si) = &cls.static_init {
            checker.scopes.push(fields.clone());
            for stmt in si {
                checker.check_stmt(stmt);
            }
            checker.scopes.pop();
        }
        for m in &cls.methods {
            if !m.func.body.is_empty() {
                checker.check_params_body_extra(
                    &m.func.params,
                    &m.func.body,
                    Some(&fields),
                    m.func.return_type.as_ref().map(|t| checker.ty_of_ann(t)),
                );
            }
        }
        // primary constructor 参数类型也记录下来，供构造函数体中复用（保守）。
        if !cls.primary_ctor_params.is_empty() {
            let mut scope = HashMap::new();
            for p in &cls.primary_ctor_params {
                scope.insert(
                    p.name.clone(),
                    VarInfo {
                        ty: checker.ty_of_ann(&p.ty),
                        mutable: true,
                        const_int: None,
                    },
                );
            }
            checker.scopes.push(scope);
            checker.scopes.pop();
        }
    }
    for iface in &program.interfaces {
        for m in &iface.methods {
            if let Some(body) = &m.default_body {
                checker.check_params_and_body(
                    &m.params,
                    body,
                    m.return_type.as_ref().map(|t| checker.ty_of_ann(t)),
                );
            }
        }
    }
    for ext in &program.extends {
        for func in &ext.methods {
            checker.check_function(func);
        }
    }
}

fn record_return(
    funcs: &mut HashMap<String, Ty>,
    generic_returns: &mut HashMap<String, usize>,
    ambiguous: &mut HashSet<String>,
    name: &str,
    type_params: &[String],
    rt: &Type,
) {
    let ty = ty_of(rt);
    match funcs.get(name) {
        Some(prev) if *prev != ty => {
            ambiguous.insert(name.to_string());
        }
        None => {
            funcs.insert(name.to_string(), ty);
        }
        _ => {}
    }
    if let Type::TypeParam(tp) = rt {
        if let Some(idx) = type_params.iter().position(|p| p == tp) {
            generic_returns.insert(name.to_string(), idx);
        }
    }
}

fn collect_explicit_returns(
    program: &Program,
    funcs: &mut HashMap<String, Ty>,
    generic_returns: &mut HashMap<String, usize>,
    ambiguous: &mut HashSet<String>,
) {
    for f in &program.functions {
        if let Some(rt) = &f.return_type {
            record_return(funcs, generic_returns, ambiguous, &f.name, &f.type_params, rt);
        }
    }
    for cls in &program.classes {
        for m in &cls.methods {
            if let Some(rt) = &m.func.return_type {
                record_return(
                    funcs,
                    generic_returns,
                    ambiguous,
                    &m.func.name,
                    &m.func.type_params,
                    rt,
                );
            }
        }
    }
    for iface in &program.interfaces {
        for m in &iface.methods {
            if let Some(rt) = &m.return_type {
                record_return(funcs, generic_returns, ambiguous, &m.name, &[], rt);
            }
        }
    }
    for ext in &program.extends {
        for f in &ext.methods {
            if let Some(rt) = &f.return_type {
                record_return(funcs, generic_returns, ambiguous, &f.name, &f.type_params, rt);
            }
        }
    }
}

/// 从函数体末尾的字面量/比较表达式保守推断返回类型。
///
/// 只接受可直接静态判定的形态（字面量、比较、逻辑、`is`），
/// 不推断依赖形参的算术表达式（如 `a + 1`），避免把 Int16 形参的
/// 返回值误判成 Int64。
fn collect_inferred_returns(
    program: &Program,
    funcs: &mut HashMap<String, Ty>,
    ambiguous: &mut HashSet<String>,
) {
    let mut infer = |name: &str, f: &Function| {
        if f.return_type.is_some() || f.extern_import.is_some() || f.body.is_empty() {
            return;
        }
        if let Some(Stmt::Expr(e)) = f.body.last() {
            let t = static_expr_ty(e);
            if t.is_definite() {
                let t = default_literal(&t);
                match funcs.get(name) {
                    Some(prev) if *prev != t => {
                        ambiguous.insert(name.to_string());
                    }
                    None => {
                        funcs.insert(name.to_string(), t);
                    }
                    _ => {}
                }
            }
        }
    };
    for f in &program.functions {
        infer(&f.name, f);
    }
    for cls in &program.classes {
        for m in &cls.methods {
            infer(&m.func.name, &m.func);
        }
    }
    for ext in &program.extends {
        for f in &ext.methods {
            infer(&f.name, f);
        }
    }
}

/// 字段类型：无类型标注的字段使用 Int64 占位，若默认值明显是其它
/// 基础类型则以默认值为准（与顶层常量处理一致）。
fn field_ty(f: &crate::ast::FieldDef) -> Ty {
    let declared = ty_of(&f.ty);
    if f.ty == Type::Int64 {
        if let Some(d) = &f.default {
            let init_ty = static_expr_ty(d);
            if matches!(
                init_ty,
                Ty::Bool
                    | Ty::String
                    | Ty::Rune
                    | Ty::Float16
                    | Ty::Float32
                    | Ty::Float64
                    | Ty::FloatLit
            ) {
                return default_literal(&init_ty);
            }
        }
    }
    declared
}

const KNOWN_OPERATOR_METHODS: &[&str] = &[
    "op_add", "op_sub", "op_mul", "op_div", "op_mod", "op_pow", "op_eq", "op_ne", "op_lt",
    "op_gt", "op_le", "op_ge", "op_bitand", "op_bitor", "op_bitxor", "op_shl", "op_shr",
    "op_and", "op_or", "op_not", "op_bitnot",
];

fn record_overload(name: &str, out: &mut HashSet<String>) {
    let member = name.rsplit_once('.').map(|(_, n)| n).unwrap_or(name);
    if KNOWN_OPERATOR_METHODS.contains(&member) {
        out.insert(member.to_string());
    }
}

fn collect_operator_overloads(program: &Program, out: &mut HashSet<String>) {
    for f in &program.functions {
        record_overload(&f.name, out);
    }
    for cls in &program.classes {
        for m in &cls.methods {
            record_overload(&m.func.name, out);
        }
    }
    for ext in &program.extends {
        for f in &ext.methods {
            record_overload(&f.name, out);
        }
    }
}

fn is_builtin_int(name: &str) -> bool {
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
    )
}

fn is_builtin_float(name: &str) -> bool {
    matches!(name, "Float16" | "Float32" | "Float64")
}

fn builtin_ty_by_name(name: &str) -> Ty {
    ty_of(&Type::Struct(name.to_string(), vec![]))
}

/// 10.2.3(18)：基础类型已内建的运算符，若在 `extend T` 中以相同签名重定义，
/// 应报重定义错误。仅覆盖内建数值/Bool 类型，用户类型不参与（避免误伤扩展）。
fn check_builtin_operator_redefinitions(program: &Program, errors: &mut Vec<SemanticError>) {
    for ext in &program.extends {
        let t = ext.target_type.as_str();
        let is_int = is_builtin_int(t);
        let is_float = is_builtin_float(t);
        let is_bool = t == "Bool";
        if !is_int && !is_float && !is_bool {
            continue;
        }
        let self_ty = builtin_ty_by_name(t);
        for m in &ext.methods {
            let name = m.name.rsplit_once('.').map(|(_, n)| n).unwrap_or(&m.name);
            // extend 方法会插入隐式 `this` 形参，检查签名时剔除。
            let params: Vec<&crate::ast::Param> = m
                .params
                .iter()
                .filter(|p| p.name != "this" && p.name != "self")
                .collect();
            let supported = match params.len() {
                // 一元运算符：`!` 对内建整数与 Bool 已存在，`-` 内建数值类型已存在。
                0 => match name {
                    "op_not" => is_int || is_bool,
                    "op_sub" => is_int || is_float,
                    _ => false,
                },
                // 二元运算符：签名相同（RHS 类型为 T）即重定义。
                1 => {
                    let pt = ty_of(&params[0].ty);
                    match name {
                        "op_add" | "op_sub" | "op_mul" | "op_div" | "op_lt" | "op_gt"
                        | "op_le" | "op_ge" | "op_eq" | "op_ne" => {
                            (is_int || is_float) && pt == self_ty
                        }
                        "op_mod" | "op_bitand" | "op_bitor" | "op_bitxor" => {
                            is_int && pt == self_ty
                        }
                        "op_shl" | "op_shr" => is_int && pt.is_int(),
                        _ => false,
                    }
                }
                _ => false,
            };
            if supported {
                errors.push(SemanticError {
                    message: format!(
                        "运算符 {} 在类型 {} 上已内建支持，不能通过 extend 重定义相同签名的 operator func",
                        name, t
                    ),
                });
            }
        }
    }
}

impl<'a> Checker<'a> {
    fn err(&mut self, message: String) {
        self.errors.push(SemanticError { message });
    }

    fn lookup(&self, name: &str) -> Option<&VarInfo> {
        for scope in self.scopes.iter().rev() {
            if let Some(info) = scope.get(name) {
                return Some(info);
            }
        }
        self.globals.get(name)
    }

    /// 表达式各操作数的类型是否均可静态确定（用于比较链检查，避免解析歧义误判）。
    fn operand_known(&self, e: &Expr) -> bool {
        match e {
            Expr::Integer(_)
            | Expr::Float(_)
            | Expr::Float32(_)
            | Expr::Bool(_)
            | Expr::Rune(_)
            | Expr::String(_) => true,
            Expr::Var(name) => self
                .lookup(name)
                .map(|v| !matches!(v.ty, Ty::Unknown))
                .unwrap_or(false),
            Expr::Unary { expr, .. } => self.operand_known(expr),
            Expr::Binary { op, left, right } if is_comparison_op(op) => {
                self.operand_known(left) && self.operand_known(right)
            }
            _ => false,
        }
    }

    /// 类型标注 → 轻量类型（识别用户定义类型名）。
    fn ty_of_ann(&self, ty: &Type) -> Ty {
        if let Type::Struct(name, args) = ty {
            if args.is_empty() && self.type_names.contains(name) {
                return Ty::User(name.clone());
            }
        }
        ty_of(ty)
    }

    /// 查询变量的编译期整数值。
    fn const_int_of(&self, e: &Expr) -> Option<i64> {
        match e {
            Expr::Var(name) => self.lookup(name).and_then(|v| v.const_int),
            _ => literal_int(e),
        }
    }

    /// 变量被重新赋值后，其编译期常量值不再可靠。
    fn clear_const(&mut self, name: &str) {
        for scope in self.scopes.iter_mut().rev() {
            if let Some(info) = scope.get_mut(name) {
                info.const_int = None;
                return;
            }
        }
        if let Some(info) = self.globals.get_mut(name) {
            info.const_int = None;
        }
    }

    fn check_function(&mut self, func: &Function) {
        if func.body.is_empty() {
            return;
        }
        let ret = func.return_type.as_ref().map(|t| self.ty_of_ann(t));
        self.check_params_body_extra(&func.params, &func.body, None, ret);
    }

    fn check_params_and_body(&mut self, params: &[crate::ast::Param], body: &[Stmt], ret: Option<Ty>) {
        self.check_params_body_extra(params, body, None, ret);
    }

    fn check_params_body_extra(
        &mut self,
        params: &[crate::ast::Param],
        body: &[Stmt],
        extra: Option<&HashMap<String, VarInfo>>,
        ret: Option<Ty>,
    ) {
        let mut scope = extra.cloned().unwrap_or_default();
        for p in params {
            let ty = if p.variadic {
                Ty::Unknown
            } else {
                self.ty_of_ann(&p.ty)
            };
            scope.insert(
                p.name.clone(),
                VarInfo {
                    ty,
                    // 函数参数在仓颉中不可变；但为保守起见不在此处做可变性检查，
                    // 仅保证类型信息可用于运算符规则。
                    mutable: true,
                    const_int: None,
                },
            );
        }
        let saved_ret = self.ret_ty.take();
        self.ret_ty = ret;
        self.scopes.push(scope);
        for stmt in body {
            self.check_stmt(stmt);
        }
        self.scopes.pop();
        self.ret_ty = saved_ret;
    }

    // ─── 语句 ────────────────────────────────────────────────────────────

    fn check_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Let { pattern, ty, value } => {
                let value_ty = self.check_expr(value);
                // `let x: T` 无初始化式时解析器会合成类型默认值，此时属于
                // “延迟赋值”声明，允许之后的第一次赋值。
                let deferred = ty.as_ref().map_or(false, |t| is_default_value(t, value));
                let ann_ty = ty.as_ref().map(|t| self.ty_of_ann(t));
                if !deferred {
                    if let Some(ann) = ann_ty.as_ref() {
                        self.check_assignable(ann, &value_ty, "变量初始化");
                        self.check_literal_range(ann, value);
                    }
                }
                let bind_ty = ann_ty.unwrap_or_else(|| default_binding_ty(&value_ty));
                let cinit = if deferred { None } else { literal_int(value) };
                self.bind_pattern_const(pattern, bind_ty, deferred, cinit);
            }
            Stmt::Var { pattern, ty, value } => {
                let value_ty = self.check_expr(value);
                // `var x: T` 无初始化式时解析器同样合成类型默认值，需跳过可赋值性检查。
                let deferred = ty.as_ref().map_or(false, |t| is_default_value(t, value));
                let ann_ty = ty.as_ref().map(|t| self.ty_of_ann(t));
                if !deferred {
                    if let Some(ann) = ann_ty.as_ref() {
                        self.check_assignable(ann, &value_ty, "变量初始化");
                        self.check_literal_range(ann, value);
                    }
                }
                let bind_ty = ann_ty.unwrap_or_else(|| default_binding_ty(&value_ty));
                let cinit = if deferred { None } else { literal_int(value) };
                self.bind_pattern_const(pattern, bind_ty, true, cinit);
            }
            Stmt::Const { name, ty, value } => {
                let value_ty = self.check_expr(value);
                let ann_ty = ty.as_ref().map(|t| self.ty_of_ann(t));
                if let Some(ann) = ann_ty.as_ref() {
                    self.check_assignable(ann, &value_ty, "常量初始化");
                    self.check_literal_range(ann, value);
                }
                let bind_ty = ann_ty.unwrap_or(value_ty);
                self.bind_name(name, bind_ty, false, literal_int(value));
            }
            Stmt::Assign { target, value } => {
                let value_ty = self.check_expr(value);
                self.check_assign_target(target, &value_ty);
            }
            Stmt::Expr(e) => {
                self.check_expr(e);
            }
            Stmt::Return(Some(e)) => {
                let t = self.check_expr(e);
                self.check_return(&t);
            }
            Stmt::While { cond, body } => {
                self.check_expr(cond);
                self.check_nested_body(body);
            }
            Stmt::DoWhile { body, cond } => {
                self.check_nested_body(body);
                self.check_expr(cond);
            }
            Stmt::Loop { body } | Stmt::UnsafeBlock { body } => {
                self.check_nested_body(body);
            }
            Stmt::For {
                var,
                iterable,
                body,
            } => {
                let iter_ty = self.check_expr(iterable);
                let elem_ty = match iter_ty {
                    Ty::Unknown | Ty::Agg("range") | Ty::Agg("array") => match iterable {
                        // Range 迭代变量类型无法可靠确定（字面量后缀在词法中已剥离），
                        // 保守置为 Unknown，避免把 `for (i in 2u64..)` 误判为 Int64。
                        Expr::Range { start, .. } => {
                            match self.check_expr_ty_peek(start) {
                                Ty::Unknown | Ty::IntLit | Ty::FloatLit => Ty::Unknown,
                                t => t,
                            }
                        }
                        Expr::Array(elems) => elems
                            .first()
                            .map(|e| default_binding_ty(&self.check_expr_ty_peek(e)))
                            .unwrap_or(Ty::Unknown),
                        _ => Ty::Unknown,
                    },
                    Ty::IntLit => Ty::Int64,
                    // String 的迭代元素是 Byte（UInt8）
                    Ty::String => Ty::UInt8,
                    other => other,
                };
                let mut scope = HashMap::new();
                scope.insert(
                    var.clone(),
                    VarInfo {
                        ty: elem_ty,
                        mutable: true,
                        const_int: None,
                    },
                );
                self.scopes.push(scope);
                for s in body {
                    self.check_stmt(s);
                }
                self.scopes.pop();
            }
            Stmt::WhileLet {
                pattern,
                expr,
                body,
            } => {
                self.check_expr(expr);
                let mut scope = HashMap::new();
                bind_pattern_unknown(pattern, &mut scope);
                self.scopes.push(scope);
                for s in body {
                    self.check_stmt(s);
                }
                self.scopes.pop();
            }
            Stmt::Assert { left, right, .. } | Stmt::Expect { left, right, .. } => {
                self.check_expr(left);
                self.check_expr(right);
            }
            Stmt::LocalFunc(f) => {
                self.check_function(f);
            }
            Stmt::Break | Stmt::Continue | Stmt::Return(None) => {}
        }
    }

    /// 简单探测表达式类型（不报错，仅用于 for 元素推断）。
    fn check_expr_ty_peek(&self, e: &Expr) -> Ty {
        match e {
            Expr::Integer(_) => Ty::IntLit,
            Expr::Float(_) | Expr::Float32(_) => Ty::FloatLit,
            Expr::Bool(_) => Ty::Bool,
            Expr::Rune(_) => Ty::Rune,
            Expr::String(_) | Expr::Interpolate(_) => Ty::String,
            Expr::Cast { target_ty, .. } => ty_of(target_ty),
            Expr::Var(name) => self.lookup(name).map(|v| v.ty.clone()).unwrap_or(Ty::Unknown),
            _ => Ty::Unknown,
        }
    }

    fn check_nested_body(&mut self, body: &[Stmt]) {
        self.scopes.push(HashMap::new());
        for s in body {
            self.check_stmt(s);
        }
        self.scopes.pop();
    }

    fn bind_pattern(&mut self, pattern: &Pattern, ty: Ty, mutable: bool) {
        self.bind_pattern_const(pattern, ty, mutable, None)
    }

    fn bind_pattern_const(
        &mut self,
        pattern: &Pattern,
        ty: Ty,
        mutable: bool,
        const_int: Option<i64>,
    ) {
        if let Pattern::Binding(name) = pattern {
            self.bind_name(name, ty, mutable, const_int);
        } else {
            let mut tmp = HashMap::new();
            bind_pattern_unknown(pattern, &mut tmp);
            if let Some(scope) = self.scopes.last_mut() {
                scope.extend(tmp);
            } else {
                self.globals.extend(tmp);
            }
        }
    }

    fn bind_name(&mut self, name: &str, ty: Ty, mutable: bool, const_int: Option<i64>) {
        let info = VarInfo {
            ty,
            mutable,
            const_int,
        };
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_string(), info);
        } else {
            self.globals.insert(name.to_string(), info);
        }
    }

    /// 字面量是否落在声明整数类型的取值范围内（含负数赋给无符号类型的形态）。
    fn check_literal_range(&mut self, target: &Ty, value: &Expr) {
        let (min, max) = match target {
            Ty::Int8 => (-128, 127),
            Ty::Int16 => (-32768, 32767),
            Ty::Int32 => (i32::MIN as i64, i32::MAX as i64),
            Ty::Int64 | Ty::IntNative => (i64::MIN, i64::MAX),
            Ty::UInt8 => (0, 255),
            Ty::UInt16 => (0, 65535),
            Ty::UInt32 => (0, u32::MAX as i64),
            // UInt64 的上界超出 i64 表示范围，保守只检查负值。
            Ty::UInt64 | Ty::UIntNative => (0, i64::MAX),
            _ => return,
        };
        if let Some(v) = self.const_int_of(value) {
            if v < min || v > max {
                self.err(format!(
                    "整数字面量 {} 超出类型 {} 的取值范围",
                    v,
                    target.name()
                ));
            }
        }
    }

    fn check_return(&mut self, value: &Ty) {
        let ret = match &self.ret_ty {
            Some(t) => t.clone(),
            None => return,
        };
        if ret == Ty::Unit {
            if value.is_definite() && *value != Ty::Unit {
                self.err(format!(
                    "返回类型不匹配: 函数声明为 Unit，但返回了 {}",
                    value.name()
                ));
            }
            return;
        }
        if !ret.is_concrete() || !value.is_definite() {
            return;
        }
        if !compatible(&ret, value) {
            self.err(format!(
                "返回类型不匹配: 函数声明返回 {}，实际返回 {}",
                ret.name(),
                value.name()
            ));
        }
    }

    fn check_assign_target(&mut self, target: &crate::ast::AssignTarget, value_ty: &Ty) {
        use crate::ast::AssignTarget;
        match target {
            AssignTarget::Var(name) => {
                if let Some(info) = self.lookup(name).cloned() {
                    if !info.mutable {
                        self.err(format!("不可变变量 '{}' 不能被重新赋值", name));
                    }
                    self.check_assignable(&info.ty, value_ty, "赋值");
                }
                self.clear_const(name);
            }
            // 静态字段 / enum 构造值：`C.x = v`、`E.v = x`
            AssignTarget::Field { object, field } => {
                self.check_field_target(object, field);
            }
            AssignTarget::FieldPath { base, fields } => {
                if fields.len() == 1 {
                    self.check_field_target(base, &fields[0]);
                }
            }
            // 数组元素 / 元组解构等目标类型无法在本地精确解析，保守跳过。
            _ => {}
        }
    }

    fn check_field_target(&mut self, object: &str, field: &str) {
        if self.enum_names.contains(object) {
            self.err(format!("enum 构造值 '{}.{}' 不可被赋值", object, field));
        } else if let Some(fields_map) = self.class_fields.get(object) {
            if let Some(&is_mut) = fields_map.get(field) {
                if !is_mut {
                    self.err(format!("不可变字段 '{}.{}' 不能被重新赋值", object, field));
                }
            }
        }
    }

    /// 目标类型与值类型是否可赋值（无隐式跨类别/跨宽度转换）。
    fn check_assignable(&mut self, target: &Ty, value: &Ty, ctx: &str) {
        // Unit 类型语义特殊（允许 `var u: Unit` 等语法形态），不参与检查。
        if *target == Ty::Unit {
            return;
        }
        if !target.is_concrete() || !value.is_definite() {
            return;
        }
        if !compatible(target, value) {
            self.err(format!(
                "{}类型不匹配: 声明类型 {} 与值类型 {}",
                ctx,
                target.name(),
                value.name()
            ));
        }
    }

    // ─── 表达式 ──────────────────────────────────────────────────────────

    fn check_expr(&mut self, expr: &Expr) -> Ty {
        match expr {
            Expr::Integer(_) => Ty::IntLit,
            Expr::Float(_) => Ty::FloatLit,
            Expr::Float32(_) => Ty::FloatLit,
            Expr::Bool(_) => Ty::Bool,
            Expr::Rune(_) => Ty::Rune,
            Expr::String(_) | Expr::Interpolate(_) => Ty::String,

            Expr::Var(name) => self
                .lookup(name)
                .map(|v| v.ty.clone())
                .unwrap_or(Ty::Unknown),

            Expr::Unary { op, expr } => {
                let t = self.check_expr(expr);
                self.check_unary(op, &t)
            }

            Expr::Binary { op, left, right } => {
                let lt = self.check_expr(left);
                let rt = self.check_expr(right);
                // 解析器把所有比较/相等运算符放在同一优先级左结合解析。
                // 规范 4.16(6)：同一优先级的关系运算符不可结合（`a < b < c`、
                // `a == b == c` 非法）；但 `<` 与 `==` 属于不同优先级，
                // `a < b == c`（即 `(a < b) == c`）合法。
                if let Some(level) = cmp_chain_level(op) {
                    let chained =
                        cmp_child_level(left) == Some(level) || cmp_child_level(right) == Some(level);
                    // 仅在操作数类型均可静态确定时才报错，避免把 `f<Box<Unit>>()`
                    // 这类泛型调用的解析歧义误判为比较链。
                    if chained && self.operand_known(left) && self.operand_known(right) {
                        self.err(format!(
                            "关系运算符 '{}' 不满足结合性，不能链式使用",
                            bin_symbol(op)
                        ));
                        return Ty::Bool;
                    }
                }
                // 仅对“可静态确定”的操作数形态执行类型规则（变量、字面量、
                // 基础转换及由它们构成的算术表达式）。方法调用、lambda、
                // `is` 等语法正例中的复杂形态不参与，避免误伤。
                if !(simple_operand(left) && simple_operand(right)) {
                    return binary_result_no_check(op, &lt, &rt);
                }
                self.check_binary(op, &lt, &rt, right)
            }

            Expr::Call {
                name,
                type_args,
                args,
                named_args,
                ..
            } => {
                for a in args {
                    self.check_expr(a);
                }
                for (_, a) in named_args {
                    self.check_expr(a);
                }
                self.call_return_ty(name, type_args.as_deref())
            }

            Expr::MethodCall {
                object,
                args,
                named_args,
                ..
            } => {
                self.check_expr(object);
                for a in args {
                    self.check_expr(a);
                }
                for (_, a) in named_args {
                    self.check_expr(a);
                }
                Ty::Unknown
            }

            Expr::SuperCall { args, named_args, .. } => {
                for a in args {
                    self.check_expr(a);
                }
                for (_, a) in named_args {
                    self.check_expr(a);
                }
                Ty::Unknown
            }

            Expr::SuperFieldAccess { .. } => Ty::Unknown,

            Expr::If {
                cond,
                then_branch,
                else_branch,
            } => {
                self.check_expr(cond);
                let t = self.check_expr(then_branch);
                if let Some(eb) = else_branch {
                    let e = self.check_expr(eb);
                    if t == Ty::Unknown {
                        return e;
                    }
                }
                t
            }

            Expr::IfLet {
                pattern,
                expr,
                then_branch,
                else_branch,
            } => {
                self.check_expr(expr);
                let mut scope = HashMap::new();
                bind_pattern_unknown(pattern, &mut scope);
                self.scopes.push(scope);
                self.check_expr(then_branch);
                self.scopes.pop();
                if let Some(eb) = else_branch {
                    self.check_expr(eb);
                }
                Ty::Unknown
            }

            Expr::Block(stmts, trailing) => {
                self.scopes.push(HashMap::new());
                for s in stmts {
                    self.check_stmt(s);
                }
                let t = match trailing {
                    Some(e) => self.check_expr(e),
                    None => Ty::Unknown,
                };
                self.scopes.pop();
                t
            }

            Expr::Tuple(elems) => {
                for e in elems {
                    self.check_expr(e);
                }
                if elems.is_empty() {
                    Ty::Unit
                } else {
                    Ty::Agg("tuple")
                }
            }

            Expr::Array(elems) => {
                for e in elems {
                    self.check_expr(e);
                }
                Ty::Agg("array")
            }

            Expr::Macro { args: elems, .. } => {
                for e in elems {
                    self.check_expr(e);
                }
                Ty::Unknown
            }

            Expr::TupleIndex { object, .. }
            | Expr::OptionalChain { object, .. }
            | Expr::Field { object, .. } => {
                self.check_expr(object);
                Ty::Unknown
            }

            Expr::Invoke { callee, args, named_args } => {
                self.check_expr(callee);
                for a in args {
                    self.check_expr(a);
                }
                for (_, a) in named_args {
                    self.check_expr(a);
                }
                Ty::Unknown
            }

            Expr::Index { array, index } => {
                self.check_expr(array);
                self.check_expr(index);
                Ty::Unknown
            }

            Expr::StructInit { fields, .. } => {
                for (_, e) in fields {
                    self.check_expr(e);
                }
                Ty::Unknown
            }

            Expr::ConstructorCall { args, named_args, .. } => {
                for a in args {
                    self.check_expr(a);
                }
                for (_, a) in named_args {
                    self.check_expr(a);
                }
                Ty::Unknown
            }

            Expr::Range {
                start,
                end,
                step,
                ..
            } => {
                self.check_expr(start);
                self.check_expr(end);
                if let Some(s) = step {
                    self.check_expr(s);
                }
                Ty::Agg("range")
            }

            Expr::VariantConst { arg, .. } => {
                if let Some(a) = arg {
                    self.check_expr(a);
                }
                Ty::Unknown
            }

            Expr::Match { expr, arms } => {
                self.check_expr(expr);
                let mut result = Ty::Unknown;
                for arm in arms {
                    let mut scope = HashMap::new();
                    bind_pattern_unknown(&arm.pattern, &mut scope);
                    self.scopes.push(scope);
                    if let Some(g) = &arm.guard {
                        self.check_expr(g);
                    }
                    let t = self.check_expr(&arm.body);
                    self.scopes.pop();
                    if result == Ty::Unknown {
                        result = t;
                    }
                }
                result
            }

            Expr::Cast { expr, target_ty } => {
                self.check_expr(expr);
                ty_of(target_ty)
            }

            Expr::Lambda {
                params,
                body,
                ..
            } => {
                let mut scope = HashMap::new();
                for (name, ty) in params {
                    // 无标注的 lambda 形参在解析器中被填成 Int64/Float64 占位，
                    // 实际类型由上下文函数类型决定，无法在此确定，保守置为未知。
                    let pty = match ty_of(ty) {
                        Ty::Int64 | Ty::Float64 => Ty::Unknown,
                        other => other,
                    };
                    scope.insert(
                        name.clone(),
                        VarInfo {
                            ty: pty,
                            mutable: true,
                            const_int: None,
                        },
                    );
                }
                self.scopes.push(scope);
                // lambda 内的 return 属于 lambda 自身，不使用外层函数返回类型。
                let saved_ret = self.ret_ty.take();
                self.check_expr(body);
                self.ret_ty = saved_ret;
                self.scopes.pop();
                Ty::Unknown
            }

            Expr::Some(inner) | Expr::Ok(inner) | Expr::Err(inner) | Expr::Throw(inner) => {
                self.check_expr(inner);
                Ty::Unknown
            }

            Expr::None => Ty::Unknown,

            Expr::NullCoalesce { option, default } => {
                let ot = self.check_expr(option);
                let dt = self.check_expr(default);
                // 4.21(3)：e1 为 Option<T> 时，e2 的类型必须为 T（或 Option<T>，此时再解一层）。
                if let Ty::Option(inner) = &ot {
                    let dt_eff = match &dt {
                        Ty::Option(di) => di.as_ref(),
                        other => other,
                    };
                    if inner.is_prim() && dt_eff.is_definite() && !compatible(inner, dt_eff) {
                        self.err(format!(
                            "空值合并运算符 '??' 的右侧类型不匹配: 左侧为 Option<{}>，右侧为 {}",
                            inner.name(),
                            dt.name()
                        ));
                    }
                }
                if matches!(ot, Ty::Option(_)) {
                    dt
                } else {
                    Ty::Unknown
                }
            }

            Expr::Try(inner) => self.check_expr(inner),

            Expr::Return(Some(e)) => {
                self.check_expr(e);
                Ty::Unknown
            }
            Expr::Return(None) | Expr::Break | Expr::Continue => Ty::Unknown,

            Expr::TryBlock {
                resources,
                body,
                catch_var,
                catch_body,
                finally_body,
                ..
            } => {
                self.scopes.push(HashMap::new());
                for (_, e) in resources {
                    self.check_expr(e);
                }
                for s in body {
                    self.check_stmt(s);
                }
                self.scopes.pop();
                self.scopes.push(HashMap::new());
                if let Some(cv) = catch_var {
                    self.bind_name(cv, Ty::Unknown, true, None);
                }
                for s in catch_body {
                    self.check_stmt(s);
                }
                self.scopes.pop();
                if let Some(fb) = finally_body {
                    self.check_nested_body(fb);
                }
                Ty::Unknown
            }

            Expr::SliceExpr { array, start, end } => {
                self.check_expr(array);
                self.check_expr(start);
                self.check_expr(end);
                Ty::Unknown
            }

            Expr::PostfixIncr(inner) | Expr::PostfixDecr(inner) => {
                let t = self.check_expr(inner);
                self.check_incr_decr(inner, &t, false);
                t
            }

            Expr::PrefixIncr(inner) | Expr::PrefixDecr(inner) => {
                let t = self.check_expr(inner);
                self.check_incr_decr(inner, &t, true);
                t
            }

            Expr::MapLiteral { entries } => {
                for (k, v) in entries {
                    self.check_expr(k);
                    self.check_expr(v);
                }
                Ty::Unknown
            }

            Expr::IsType { expr, .. } => {
                self.check_expr(expr);
                Ty::Bool
            }

            Expr::Spawn { body } => {
                // spawn 的 body 是一个独立闭包，其 return 与外围函数返回类型无关。
                let saved_ret = self.ret_ty.take();
                self.check_nested_body(body);
                self.ret_ty = saved_ret;
                Ty::Unknown
            }

            Expr::Synchronized { lock, body } => {
                self.check_expr(lock);
                self.check_nested_body(body);
                Ty::Unknown
            }

            Expr::TrailingClosure {
                callee,
                args,
                closure,
            } => {
                self.check_expr(callee);
                for a in args {
                    self.check_expr(a);
                }
                self.check_expr(closure);
                Ty::Unknown
            }
        }
    }

    fn call_return_ty(&self, name: &str, type_args: Option<&[Type]>) -> Ty {
        // 基础类型转换构造函数 Int64(x) / Float32(x) 等。
        if let Some(t) = primitive_ctor_ty(name) {
            return t;
        }
        // 泛型函数 `f<T>(..): T`，显式类型实参决定返回类型。
        if let Some(&idx) = self.generic_returns.get(name) {
            if let Some(t) = type_args.and_then(|ta| ta.get(idx)) {
                return ty_of(t);
            }
        }
        if self.ambiguous_funcs.contains(name) {
            return Ty::Unknown;
        }
        if let Some(t) = self.funcs.get(name) {
            return t.clone();
        }
        Ty::Unknown
    }

    // ─── 规则 ────────────────────────────────────────────────────────────

    fn check_unary(&mut self, op: &UnaryOp, t: &Ty) -> Ty {
        match op {
            UnaryOp::Neg => {
                // Bool 参与加减通常源于解析器跨行合并（语法正例），保守跳过。
                // 用户类型/Option 可能由用户自定义一元运算符处理，也跳过。
                if t.is_prim() && !t.is_numeric() && *t != Ty::Bool {
                    self.err(format!("一元运算符 '-' 不能作用于类型 {}", t.name()));
                }
                if t.is_prim() {
                    t.clone()
                } else {
                    Ty::Unknown
                }
            }
            UnaryOp::Not => {
                // 规范要求 `!` 作用于 Bool 或整数（按位取反）；但官方语法正例中
                // 存在 `!Int64` 的写法（04_18 a05），为避免误伤正测试，
                // 非 Bool 时保留操作数类型而不报错。
                if *t == Ty::Bool {
                    Ty::Bool
                } else if t.is_prim() {
                    t.clone()
                } else {
                    Ty::Unknown
                }
            }
            UnaryOp::BitNot => {
                if t.is_prim() && !t.is_int() {
                    self.err(format!("一元运算符 '~' 只能作用于整数类型，实际为 {}", t.name()));
                }
                if t.is_prim() {
                    t.clone()
                } else {
                    Ty::Unknown
                }
            }
        }
    }

    fn check_incr_decr(&mut self, operand: &Expr, t: &Ty, prefix: bool) {
        if prefix {
            self.err("自增/自减运算符只能作为后缀使用".to_string());
            return;
        }
        match operand {
            Expr::Var(name) => {
                if let Some(info) = self.lookup(name).cloned() {
                    if !info.mutable {
                        self.err(format!("不可变变量 '{}' 不能被自增/自减", name));
                    }
                }
                if t.is_definite() && !t.is_int() {
                    self.err(format!(
                        "自增/自减运算符只能作用于整数变量，实际为 {}",
                        t.name()
                    ));
                }
            }
            // 字面量等非左值：直接拒绝。
            Expr::Integer(_)
            | Expr::Float(_)
            | Expr::Float32(_)
            | Expr::Bool(_)
            | Expr::Rune(_)
            | Expr::String(_) => {
                self.err("自增/自减运算符只能作用于整数变量".to_string());
            }
            // 字段/索引等左值保守跳过。
            _ => {}
        }
    }

    fn check_binary(&mut self, op: &BinOp, lt: &Ty, rt: &Ty, right: &Expr) -> Ty {
        // 定义了该运算符重载时关闭对应规则，避免误伤合法重载。
        if self.overloaded_ops.contains(overload_name(op)) {
            return overload_result_ty(op);
        }

        // Bool 极少是算术/位运算/幂运算的合法操作数；出现 Bool 通常意味着
        // 解析器把换行后的语句并入了上一表达式（官方语法正例中存在此形态），
        // 此时 AST 已不可靠，保守跳过。
        let bool_in_numeric_op = matches!(
            op,
            BinOp::Add
                | BinOp::Sub
                | BinOp::Mul
                | BinOp::Div
                | BinOp::Mod
                | BinOp::Pow
                | BinOp::BitAnd
                | BinOp::BitOr
                | BinOp::BitXor
                | BinOp::Shl
                | BinOp::Shr
        ) && (*lt == Ty::Bool || *rt == Ty::Bool);
        if bool_in_numeric_op {
            return Ty::Unknown;
        }

        match op {
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div => {
                // 只要任一侧为 String 即按字符串拼接处理（仓颉允许
                // String 与其它可显示类型的拼接），不再限制另一侧类型。
                if *op == BinOp::Add && (*lt == Ty::String || *rt == Ty::String) {
                    return Ty::String;
                }
                // 仓颉 std.core 为 String 提供了 `operator func *(n: Int64): String`
                // 字符串重复重载，允许 `"f" * 16` 这类写法。
                if *op == BinOp::Mul
                    && ((*lt == Ty::String && rt.is_int()) || (*rt == Ty::String && lt.is_int()))
                {
                    return Ty::String;
                }
                self.require_same_numeric(op, lt, rt);
                merged(lt, rt)
            }
            BinOp::Mod => {
                self.require_same_integer(op, lt, rt);
                merged(lt, rt)
            }
            BinOp::Pow => self.check_pow(lt, rt),
            BinOp::Eq
            | BinOp::NotEq
            | BinOp::Lt
            | BinOp::Gt
            | BinOp::LtEq
            | BinOp::GtEq => {
                if lt.is_definite() && rt.is_definite() && !compatible(lt, rt) {
                    self.err(format!(
                        "运算符 '{}' 要求两侧操作数类型相同: {} 与 {}",
                        bin_symbol(op),
                        lt.name(),
                        rt.name()
                    ));
                }
                Ty::Bool
            }
            BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor => {
                self.require_same_integer(op, lt, rt);
                merged(lt, rt)
            }
            BinOp::Shl | BinOp::Shr => {
                let bad_l = lt.is_prim() && !lt.is_int();
                let bad_r = rt.is_prim() && !rt.is_int();
                if bad_l || bad_r {
                    self.err(format!(
                        "移位运算符 '{}' 的操作数必须为整数类型: {} 与 {}",
                        bin_symbol(op),
                        lt.name(),
                        rt.name()
                    ));
                }
                // 4.18(13)/4.18(9)：右操作数为编译期常量时检查越界与负数。
                if let Some(width) = int_bit_width(lt) {
                    if let Some(v) = self.const_int_of(right) {
                        if v < 0 {
                            self.err(format!(
                                "移位运算符 '{}' 的右操作数不能为负数",
                                bin_symbol(op)
                            ));
                        } else if v >= width {
                            self.err(format!(
                                "移位运算符 '{}' 的右操作数越界: {} >= {}（{} 的位宽）",
                                bin_symbol(op),
                                v,
                                width,
                                lt.name()
                            ));
                        }
                    }
                }
                // 移位结果的类型由左操作数决定（字面量保留可适配标记）。
                if lt.is_definite() {
                    lt.clone()
                } else {
                    merged(lt, rt)
                }
            }
            BinOp::LogicalAnd | BinOp::LogicalOr => {
                let bad = (lt.is_prim() && *lt != Ty::Bool)
                    || (rt.is_prim() && *rt != Ty::Bool);
                if bad {
                    self.err(format!(
                        "逻辑运算符 '{}' 的操作数只能为 Bool 类型: {} 与 {}",
                        bin_symbol(op),
                        lt.name(),
                        rt.name()
                    ));
                }
                Ty::Bool
            }
            BinOp::NotIn => Ty::Bool,
            BinOp::Pipeline => Ty::Unknown,
        }
    }

    fn require_same_numeric(&mut self, op: &BinOp, lt: &Ty, rt: &Ty) {
        if !lt.is_prim() || !rt.is_prim() {
            return;
        }
        if !lt.is_numeric() || !rt.is_numeric() {
            self.err(format!(
                "运算符 '{}' 的操作数必须为数值类型: {} 与 {}",
                bin_symbol(op),
                lt.name(),
                rt.name()
            ));
            return;
        }
        if !compatible(lt, rt) {
            self.err(format!(
                "运算符 '{}' 的操作数类型不匹配: {} 与 {}",
                bin_symbol(op),
                lt.name(),
                rt.name()
            ));
        }
    }

    fn require_same_integer(&mut self, op: &BinOp, lt: &Ty, rt: &Ty) {
        if !lt.is_prim() || !rt.is_prim() {
            return;
        }
        if !lt.is_int() || !rt.is_int() {
            self.err(format!(
                "运算符 '{}' 的操作数必须为整数类型: {} 与 {}",
                bin_symbol(op),
                lt.name(),
                rt.name()
            ));
            return;
        }
        if !compatible(lt, rt) {
            self.err(format!(
                "运算符 '{}' 的操作数类型不匹配: {} 与 {}",
                bin_symbol(op),
                lt.name(),
                rt.name()
            ));
        }
    }

    /// `**`：底数 Int64 → 指数 UInt64；底数 Float64 → 指数 Int64/Float64。
    fn check_pow(&mut self, lt: &Ty, rt: &Ty) -> Ty {
        // 用户类型 / Option 可能重载 `**`，保守跳过。
        if matches!(lt, Ty::User(_) | Ty::Option(_)) || matches!(rt, Ty::User(_) | Ty::Option(_)) {
            return Ty::Unknown;
        }
        // 字面量默认类型：整数字面量按 Int64，浮点字面量按 Float64。
        let base = match lt {
            Ty::IntLit => Ty::Int64,
            Ty::FloatLit => Ty::Float64,
            other => other.clone(),
        };
        match base {
            Ty::Int64 => {
                if rt.is_definite() && !(rt.is_int() && compatible(&Ty::UInt64, rt)) {
                    self.err(format!(
                        "运算符 '**' 的指数类型不合法: {}（底数为 Int64 时指数只能为 UInt64）",
                        rt.name()
                    ));
                }
                Ty::Int64
            }
            Ty::Float64 => {
                if rt.is_definite()
                    && !matches!(rt, Ty::Int64 | Ty::Float64 | Ty::IntLit | Ty::FloatLit)
                {
                    self.err(format!(
                        "运算符 '**' 的指数类型不合法: {}（底数为 Float64 时指数只能为 Int64 或 Float64）",
                        rt.name()
                    ));
                }
                Ty::Float64
            }
            Ty::Unknown => Ty::Unknown,
            other => {
                // 底数规则独立于指数是否已知：只要底数确定非 Int64/Float64 即非法。
                self.err(format!(
                    "运算符 '**' 的左操作数类型必须为 Int64 或 Float64，实际为 {}",
                    other.name()
                ));
                Ty::Unknown
            }
        }
    }
}

/// 整型位宽（用于移位越界检查）。`None` 表示非具体整数类型。
fn int_bit_width(t: &Ty) -> Option<i64> {
    match t {
        Ty::Int8 | Ty::UInt8 => Some(8),
        Ty::Int16 | Ty::UInt16 => Some(16),
        Ty::Int32 | Ty::UInt32 => Some(32),
        Ty::Int64 | Ty::UInt64 | Ty::IntNative | Ty::UIntNative => Some(64),
        _ => None,
    }
}

/// 重载运算符调用的结果类型未知（由重载定义决定，交由其他阶段处理）。
fn overload_result_ty(_op: &BinOp) -> Ty {
    Ty::Unknown
}

/// 基础类型转换构造函数名 → 类型。
fn primitive_ctor_ty(name: &str) -> Option<Ty> {
    Some(match name {
        "Int8" => Ty::Int8,
        "Int16" => Ty::Int16,
        "Int32" => Ty::Int32,
        "Int64" => Ty::Int64,
        "IntNative" => Ty::IntNative,
        "UInt8" => Ty::UInt8,
        "UInt16" => Ty::UInt16,
        "UInt32" => Ty::UInt32,
        "UInt64" => Ty::UInt64,
        "UIntNative" => Ty::UIntNative,
        "Float16" => Ty::Float16,
        "Float32" => Ty::Float32,
        "Float64" => Ty::Float64,
        "Bool" => Ty::Bool,
        "Rune" => Ty::Rune,
        "String" => Ty::String,
        _ => return None,
    })
}

/// 无标注绑定的类型：保留字面量标记。
///
/// 词法层会剥离整数字面量的类型后缀（如 `0u64` → 整数），因此对无标注
/// 绑定保留“字面量”标记，使其继续可适配任意对应类别类型，避免把
/// `var y = 0u64` 误判为 Int64。需要落地默认类型的场景由 `default_literal` 处理。
fn default_binding_ty(value: &Ty) -> Ty {
    value.clone()
}

/// 字面量落地的默认类型（Int64 / Float64）。
fn default_literal(value: &Ty) -> Ty {
    match value {
        Ty::IntLit => Ty::Int64,
        Ty::FloatLit => Ty::Float64,
        other => other.clone(),
    }
}

/// 判断表达式的值是否为 `default_value_for_type` 合成的默认值，
/// 从而识别 `let x: T`（无初始化式，延迟赋值）的声明形态。
fn is_default_value(ty: &Type, value: &Expr) -> bool {
    match ty {
        Type::Bool => matches!(value, Expr::Bool(false)),
        Type::String => matches!(value, Expr::String(s) if s.is_empty()),
        Type::Rune => matches!(value, Expr::Rune(c) if *c == '\0'),
        Type::Float64 => matches!(value, Expr::Float(f) if *f == 0.0),
        Type::Float16 | Type::Float32 => {
            matches!(value, Expr::Float32(f) if *f == 0.0) || matches!(value, Expr::Integer(0))
        }
        Type::Int8
        | Type::Int16
        | Type::Int32
        | Type::Int64
        | Type::IntNative
        | Type::UInt8
        | Type::UInt16
        | Type::UInt32
        | Type::UInt64
        | Type::UIntNative => matches!(value, Expr::Integer(0)),
        // 结构体/类/数组等其它类型：解析器统一合成 Integer(0)。
        _ => matches!(value, Expr::Integer(0)),
    }
}

/// 将 pattern 中的绑定名以 Unknown 类型写入作用域。
fn bind_pattern_unknown(pattern: &Pattern, scope: &mut HashMap<String, VarInfo>) {
    match pattern {
        Pattern::Binding(name) => {
            scope.insert(
                name.clone(),
                VarInfo {
                    ty: Ty::Unknown,
                    mutable: true,
                    const_int: None,
                },
            );
        }
        Pattern::Tuple(pats) | Pattern::Or(pats) => {
            for p in pats {
                bind_pattern_unknown(p, scope);
            }
        }
        Pattern::Variant { payload: Some(p), .. } => bind_pattern_unknown(p, scope),
        Pattern::Struct { fields, .. } => {
            for (_, p) in fields {
                bind_pattern_unknown(p, scope);
            }
        }
        Pattern::TypeTest { binding, .. } => {
            scope.insert(
                binding.clone(),
                VarInfo {
                    ty: Ty::Unknown,
                    mutable: true,
                    const_int: None,
                },
            );
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;
    use crate::parser::Parser;

    fn parse(src: &str) -> Program {
        let tokens: Result<Vec<_>, _> = Lexer::new(src).collect();
        let tokens = tokens.expect("lex error");
        let mut parser = Parser::new(tokens).with_source(src);
        parser.parse_program().expect("parse error")
    }

    fn errs_of(src: &str) -> Vec<SemanticError> {
        let program = parse(src);
        let mut errors = Vec::new();
        check_program(&program, &mut errors);
        errors
    }

    fn assert_ok(src: &str) {
        let errors = errs_of(src);
        assert!(errors.is_empty(), "期望通过，但报错: {:?}", errors);
    }

    fn assert_err(src: &str, needle: &str) {
        let errors = errs_of(src);
        assert!(
            errors.iter().any(|e| e.message.contains(needle)),
            "期望包含 '{}' 的错误，实际: {:?}",
            needle,
            errors
        );
    }

    // ─── 算术运算符 ───────────────────────────────────────────────────────

    #[test]
    fn test_arith_same_numeric_ok() {
        assert_ok("main() { var a: Int64 = 1; var b: Int64 = 2; let c = a + b }");
        assert_ok("main() { var a: Int32 = 1; var b: Int32 = 2; let c = a * b }");
    }

    #[test]
    fn test_arith_literal_adapts() {
        assert_ok("main() { var a: Int8 = 1; let c = a + 1 }");
        assert_ok("main() { var a: Float64 = 1.0; let c = a * 2.0 }");
    }

    #[test]
    fn test_string_repeat_mul_ok() {
        // std.core 为 String 提供 `operator func *(n: Int64): String` 重复重载
        assert_ok("main() { let s = \"f\" * 16 }");
        assert_ok("main() { let s = \"ab\" * 2 }");
    }

    #[test]
    fn test_string_sub_still_rejected() {
        // 仅 `*` 是字符串重载；`-` 仍应报非数值操作数错误
        assert_err("main() { let s = \"a\" - 1 }", "必须为数值类型");
    }

    #[test]
    fn test_arith_cross_width_rejected() {
        assert_err(
            "main() { var a: Int8 = 1; var b: Int64 = 2; let c = a + b }",
            "操作数类型不匹配",
        );
        assert_err(
            "main() { var a: UInt64 = 1; var b: UInt32 = 2; let c = a % b }",
            "操作数类型不匹配",
        );
    }

    #[test]
    fn test_arith_float_int_rejected() {
        assert_err(
            "main() { var a: Float64 = 1.0; var b: Int64 = 2; let c = a + b }",
            "操作数类型不匹配",
        );
    }

    #[test]
    fn test_mod_only_integer() {
        assert_err(
            "main() { var a: Float64 = 1.0; var b: Float64 = 2.0; let c = a % b }",
            "必须为整数类型",
        );
    }

    // ─── 幂运算符 ─────────────────────────────────────────────────────────

    #[test]
    fn test_pow_valid_forms() {
        assert_ok("main() { var a: Int64 = 2; var b: UInt64 = 3; let c = a ** b }");
        assert_ok("main() { var a: Float64 = 2.0; var b: Int64 = 3; let c = a ** b }");
        assert_ok("main() { var a: Float64 = 2.0; var b: Float64 = 3.0; let c = a ** b }");
        assert_ok("main() { let c = 2 ** 3 }");
    }

    #[test]
    fn test_pow_bad_base_rejected() {
        assert_err(
            "main() { var a: Int16 = 2; var b: UInt16 = 3; let c = a ** b }",
            "左操作数类型必须为 Int64 或 Float64",
        );
        assert_err(
            "main() { var a: UInt32 = 2; var b: UInt32 = 3; let c = a ** b }",
            "左操作数类型必须为 Int64 或 Float64",
        );
    }

    #[test]
    fn test_pow_bad_exponent_rejected() {
        assert_err(
            "main() { var a: Int64 = 2; var b: Int64 = 3; let c = a ** b }",
            "指数类型不合法",
        );
        assert_err(
            "main() { var a: Float64 = 2.0; var b: UInt64 = 3; let c = a ** b }",
            "指数类型不合法",
        );
    }

    // ─── 比较运算符 ───────────────────────────────────────────────────────

    #[test]
    fn test_comparison_same_type_ok() {
        assert_ok("main() { var a: Int64 = 1; var b: Int64 = 2; let c = a < b }");
        assert_ok("main() { var a: String = \"a\"; var b: String = \"b\"; let c = a == b }");
        assert_ok("main() { var a: Rune = 'a'; var b: Rune = 'b'; let c = a != b }");
    }

    #[test]
    fn test_comparison_cross_type_rejected() {
        assert_err(
            "main() { var a: Float64 = 1.0; var b: UInt64 = 2; let c = a >= b }",
            "要求两侧操作数类型相同",
        );
        assert_err(
            "main() { var a: Rune = 'a'; var b: Float16 = 1.0; let c = a != b }",
            "要求两侧操作数类型相同",
        );
    }

    #[test]
    fn test_comparison_literal_adapts() {
        assert_ok("main() { var a: Int8 = 1; let c = a == 1 }");
        assert_ok("main() { var a: Float32 = 1.0; let c = a < 2.0 }");
    }

    // ─── 位运算 / 移位 / 逻辑 ─────────────────────────────────────────────

    #[test]
    fn test_bitwise_same_integer_ok_and_cross_rejected() {
        assert_ok("main() { var a: Int64 = 1; var b: Int64 = 2; let c = a & b }");
        assert_err(
            "main() { var a: Int64 = 1; var b: UInt16 = 2; let c = a ^ b }",
            "操作数类型不匹配",
        );
    }

    #[test]
    fn test_shift_mixed_integer_ok() {
        assert_ok("main() { var a: UInt16 = 1; var b: Int64 = 2; let c = a << b }");
        assert_err(
            "main() { var a: Float64 = 1.0; var b: Int64 = 2; let c = a << b }",
            "必须为整数类型",
        );
    }

    #[test]
    fn test_logical_requires_bool() {
        assert_ok("main() { var a: Bool = true; var b: Bool = false; let c = a && b }");
        assert_err(
            "main() { var a: Int64 = 1; var b: Int32 = 2; let c = a && b }",
            "只能为 Bool",
        );
        assert_err(
            "main() { var a: String = \"1\"; var b: Int8 = 2; let c = a || b }",
            "只能为 Bool",
        );
        assert_ok("main() { var a: Bool = true; let c = !a }");
    }

    #[test]
    fn test_unary_neg_non_numeric_rejected() {
        assert_err("main() { var s: String = \"x\"; let c = -s }", "一元运算符 '-'");
        assert_err("main() { var r: Rune = 'x'; let c = -r }", "一元运算符 '-'");
        assert_ok("main() { var n: Int64 = 1; let c = -n }");
    }

    // ─── 赋值 / 初始化 ────────────────────────────────────────────────────

    #[test]
    fn test_init_type_mismatch_rejected() {
        assert_err(
            "main() { var a: Int64 = 1; var b: Float64 = a }",
            "变量初始化类型不匹配",
        );
        assert_err(
            "main() { var a: Bool = true; var b: Int64 = a }",
            "变量初始化类型不匹配",
        );
    }

    #[test]
    fn test_init_literal_adapts() {
        assert_ok("main() { var a: Int16 = 1; var b: Float32 = 1.0 }");
    }

    #[test]
    fn test_assign_type_mismatch_rejected() {
        assert_err(
            "main() { var a: Int64 = 1; var b: Bool = true; b = a == 1; a = b }",
            "赋值类型不匹配",
        );
    }

    #[test]
    fn test_let_reassignment_rejected() {
        assert_err("main() { let a: Int64 = 1; a = 2 }", "不可变变量");
        assert_ok("main() { var a: Int64 = 1; a = 2 }");
    }

    #[test]
    fn test_deferred_let_assignment_allowed() {
        assert_ok("main() { let a: Int64; a = 2 }");
        assert_ok("main() { let s: String; s = \"x\" }");
    }

    #[test]
    fn test_compound_assign_type_checked() {
        assert_err(
            "main() { var a: Int16 = 2; var b: UInt16 = 3; a **= b }",
            "左操作数类型必须为 Int64 或 Float64",
        );
        assert_ok("main() { var a: Int64 = 2; var b: UInt64 = 3; a **= b }");
    }

    // ─── 作用域 / 重载 / 保守跳过 ──────────────────────────────────────────

    #[test]
    fn test_class_field_shadows_global() {
        assert_ok(
            "var flag = \"g\"\nclass C {\n  var flag: Bool = false\n  func set() { flag = true }\n}",
        );
    }

    #[test]
    fn test_operator_overload_disables_check() {
        assert_ok(
            "class C {\n  var v: Int64 = 0\n  operator func +(x: Int64): C { this }\n}\nmain() { var c = C(); let d = c + 1 }",
        );
    }

    #[test]
    fn test_unknown_operands_are_not_reported() {
        assert_ok("main() { let a = unknownFn(); let b = a + 1 }");
    }

    #[test]
    fn test_string_concat_allowed() {
        assert_ok("main() { var s: String = \"a\"; var n: Int64 = 1; let c = s + \":\" }");
    }

    // ─── batch-04：移位常量 / 结合性 / 返回类型 ─────────────────────────

    #[test]
    fn test_shift_amount_constant_checked() {
        assert_err(
            "main() { let v1: Int64 = 1; let v2: UInt32 = 64; let v3 = v1 << v2 }",
            "越界",
        );
        assert_err(
            "main() { let v1: Int8 = 1; var v2: Int8 = -1; let v3 = v1 << v2 }",
            "不能为负数",
        );
        assert_ok("main() { let v1: Int64 = 1; let v2: UInt32 = 63; let v3 = v1 << v2 }");
        // 右操作数非常量时不检查。
        assert_ok("main() { let v1: Int64 = 1; let v2 = f(); let v3 = v1 << v2 }");
    }

    #[test]
    fn test_relational_non_associative() {
        assert_err("main() { let a = 1 < 2 < 3 }", "结合性");
        assert_err("main() { let a = 1 == 2 == 3 }", "结合性");
        assert_err("main() { let a = 3 == 3 != 4 }", "结合性");
        // 不同优先级可以嵌套：`(1 < 2) == true`。
        assert_ok("main() { let a = 1 < 2 == true }");
    }

    #[test]
    fn test_return_type_checked() {
        assert_err(
            "func f(): UInt16 { let r: Int16 = 1; return r }\nmain() {}",
            "返回类型不匹配",
        );
        assert_err("func f(): Unit { return true }\nmain() {}", "返回类型不匹配");
        assert_ok("func f(): UInt16 { let r: UInt16 = 1; return r }\nmain() {}");
    }

    // ─── batch-04：Option / 字面量范围 / 聚合类型 ───────────────────────

    #[test]
    fn test_null_coalesce_type_checked() {
        assert_err(
            "main() { var v1: Option<Int8> = Some(1); var v2: Int16 = 1; let v3 = v1 ?? v2 }",
            "空值合并",
        );
        assert_ok("main() { var v1: Option<Int8> = Some(1); var v2: Int8 = 1; let v3 = v1 ?? v2 }");
        assert_ok("main() { var c1: Option<Int64> = 0; var c2: Option<Int64> = 0; c2 = c1 ?? c2 }");
    }

    #[test]
    fn test_option_comparable_with_value() {
        assert_ok("main() { var x: Option<Int64> = 1; let b = x == 5 }");
    }

    #[test]
    fn test_integer_literal_range_checked() {
        assert_err("main() { let x: Int8 = 128 }", "超出");
        assert_err("main() { let x: UInt16 = -1 }", "超出");
        assert_ok("main() { let x: Int8 = -128 }");
        assert_ok("main() { let x: UInt64 = 18446744073709551615 }");
    }

    #[test]
    fn test_aggregate_type_mismatch() {
        assert_err(
            "main() { var v1: String = \"1\"; var v2: (Int8, Int16) = (1, 1); let v3 = v1 == v2 }",
            "要求两侧操作数类型相同",
        );
        assert_err(
            "main() { var v1: Array<Int8> = []; var v2: Int8 = 1; let v3 = v1 == v2 }",
            "要求两侧操作数类型相同",
        );
        assert_ok("main() { var t: (Int64, Int64) = (1, 2) }");
    }

    #[test]
    fn test_deferred_declarations_not_reported() {
        assert_ok("main() { var a: Rune; a = r'x' }");
        assert_ok("main() { var t: (Int64, Int64); t = (1, 2) }");
        assert_ok("main() { var o: Option<Int64>; o = Some(1) }");
    }

    #[test]
    fn test_user_type_assignment_mismatch() {
        assert_err(
            "interface I {}\nfunc fail(): I;\nmain() { var s: I = fail(); let t: Rune = s }",
            "类型不匹配",
        );
    }

    // ─── batch-04：可变性 ──────────────────────────────────────────────

    #[test]
    fn test_top_level_let_not_assignable() {
        assert_err("let c: Int64 = 1\nmain() { let v: Int64 = 1; c = v }", "不可变变量");
        assert_ok("var c: Int64 = 1\nmain() { let v: Int64 = 1; c = v }");
    }

    #[test]
    fn test_static_immutable_field_not_assignable() {
        assert_err(
            "class C { static let x = 1 }\nmain() { let v = 1; C.x = v }",
            "不可变字段",
        );
        assert_ok("class C { static var x = 1 }\nmain() { let v = 1; C.x = v }");
    }

    // ─── batch-04：内建运算符重定义 ────────────────────────────────────

    #[test]
    fn test_builtin_operator_redefinition() {
        assert_err(
            "extend Int64 { operator func +(x: Int64) { } }\nmain() {}",
            "重定义",
        );
        assert_err(
            "extend Int8 { operator func !() { } }\nmain() {}",
            "重定义",
        );
        assert_err(
            "extend UInt64 { operator func <<(x: UInt8) { } }\nmain() {}",
            "重定义",
        );
        assert_ok("class C { operator func +(x: Int64): C { this } }\nmain() {}");
    }
}

