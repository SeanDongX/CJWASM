//! CHIR 类型定义

use crate::ast::{BinOp, UnaryOp, Type, StructDef, ClassDef, EnumDef};
use wasm_encoder::ValType;

/// 源码位置（用于错误报告）
#[derive(Debug, Clone)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

/// CHIR 表达式
#[derive(Debug, Clone)]
pub struct CHIRExpr {
    pub kind: CHIRExprKind,
    pub ty: Type,           // 完整的 AST 类型（单态化后）
    pub wasm_ty: ValType,   // WASM 类型
    pub span: Option<Span>, // 源码位置
}

/// CHIR 表达式类型
#[derive(Debug, Clone)]
pub enum CHIRExprKind {
    // 字面量
    Integer(i64),
    Float(f64),
    Float32(f32),
    Bool(bool),
    String(String),
    Rune(char),

    // 变量和引用
    Local(u32),              // 局部变量索引
    Global(String),          // 全局变量名

    // 运算
    Binary {
        op: BinOp,
        left: Box<CHIRExpr>,
        right: Box<CHIRExpr>,
    },
    Unary {
        op: UnaryOp,
        expr: Box<CHIRExpr>,
    },

    // 函数调用
    Call {
        func_idx: u32,       // 函数索引（已解析）
        args: Vec<CHIRExpr>,
    },
    MethodCall {
        vtable_offset: Option<u32>, // vtable 偏移（虚方法）
        func_idx: Option<u32>,       // 函数索引（静态方法）
        receiver: Box<CHIRExpr>,
        args: Vec<CHIRExpr>,
    },

    // 内存访问
    Load {
        ptr: Box<CHIRExpr>,
        offset: u32,
        align: u32,
    },
    Store {
        ptr: Box<CHIRExpr>,
        value: Box<CHIRExpr>,
        offset: u32,
        align: u32,
    },

    // 控制流
    If {
        cond: Box<CHIRExpr>,
        then_block: CHIRBlock,
        else_block: Option<CHIRBlock>,
    },
    Match {
        subject: Box<CHIRExpr>,
        arms: Vec<CHIRMatchArm>,
    },
    Block(CHIRBlock),

    // 类型转换（显式）
    Cast {
        expr: Box<CHIRExpr>,
        from_ty: ValType,
        to_ty: ValType,
    },

    // 数组/元组
    ArrayNew {
        len: Box<CHIRExpr>,
        init: Box<CHIRExpr>,
    },
    ArrayGet {
        array: Box<CHIRExpr>,
        index: Box<CHIRExpr>,
    },
    ArraySet {
        array: Box<CHIRExpr>,
        index: Box<CHIRExpr>,
        value: Box<CHIRExpr>,
    },
    TupleNew {
        elements: Vec<CHIRExpr>,
    },
    TupleGet {
        tuple: Box<CHIRExpr>,
        index: usize,
    },

    // 结构体/类
    StructNew {
        struct_name: String,
        fields: Vec<(String, CHIRExpr)>,
    },
    FieldGet {
        object: Box<CHIRExpr>,
        field_offset: u32,
        field_ty: Type,
    },
    FieldSet {
        object: Box<CHIRExpr>,
        field_offset: u32,
        value: Box<CHIRExpr>,
    },

    // I/O 输出
    /// println / print / eprintln / eprint 内置输出
    Print {
        /// 输出参数（None 表示空行）
        arg: Option<Box<CHIRExpr>>,
        /// true = 输出后加 '\n'
        newline: bool,
        /// 文件描述符（1=stdout, 2=stderr）
        fd: i32,
    },

    // 特殊
    Nop,                     // 无操作
    Unreachable,             // 不可达代码
}

/// CHIR 语句
#[derive(Debug, Clone)]
pub enum CHIRStmt {
    Let {
        local_idx: u32,
        value: CHIRExpr,
    },
    Assign {
        target: CHIRLValue,
        value: CHIRExpr,
    },
    Expr(CHIRExpr),
    Return(Option<CHIRExpr>),
    Break,
    Continue,
    While {
        cond: CHIRExpr,
        body: CHIRBlock,
    },
    Loop {
        body: CHIRBlock,
    },
}

/// CHIR 左值
#[derive(Debug, Clone)]
pub enum CHIRLValue {
    Local(u32),
    Field {
        object: Box<CHIRExpr>,
        offset: u32,
    },
    Index {
        array: Box<CHIRExpr>,
        index: Box<CHIRExpr>,
    },
}

/// CHIR 基本块
#[derive(Debug, Clone)]
pub struct CHIRBlock {
    pub stmts: Vec<CHIRStmt>,
    pub result: Option<Box<CHIRExpr>>, // 块表达式的结果
}

/// CHIR Match 分支
#[derive(Debug, Clone)]
pub struct CHIRMatchArm {
    pub pattern: CHIRPattern,
    pub guard: Option<CHIRExpr>,
    pub body: CHIRBlock,
}

/// CHIR 模式（简化版，用于 codegen）
#[derive(Debug, Clone)]
pub enum CHIRPattern {
    Wildcard,
    Binding(u32),            // 局部变量索引
    Literal(CHIRLiteral),
    Variant {
        discriminant: i32,   // 枚举判别值
        payload_binding: Option<u32>,
    },
    Range {
        start: i64,
        end: i64,
        inclusive: bool,
    },
}

/// CHIR 字面量
#[derive(Debug, Clone)]
pub enum CHIRLiteral {
    Integer(i64),
    Bool(bool),
    String(String),
}

/// CHIR 函数
#[derive(Debug, Clone)]
pub struct CHIRFunction {
    pub name: String,
    pub params: Vec<CHIRParam>,
    pub return_ty: Type,
    pub return_wasm_ty: ValType,
    pub locals: Vec<CHIRLocal>,
    pub body: CHIRBlock,
}

/// CHIR 参数
#[derive(Debug, Clone)]
pub struct CHIRParam {
    pub name: String,
    pub ty: Type,
    pub wasm_ty: ValType,
    pub local_idx: u32,
}

/// CHIR 局部变量
#[derive(Debug, Clone)]
pub struct CHIRLocal {
    pub name: String,
    pub ty: Type,
    pub wasm_ty: ValType,
    pub local_idx: u32,
}

/// CHIR 程序
#[derive(Debug, Clone)]
pub struct CHIRProgram {
    pub functions: Vec<CHIRFunction>,
    pub structs: Vec<StructDef>,
    pub classes: Vec<ClassDef>,
    pub enums: Vec<EnumDef>,
    pub globals: Vec<CHIRGlobal>,
}

/// CHIR 全局变量
#[derive(Debug, Clone)]
pub struct CHIRGlobal {
    pub name: String,
    pub ty: Type,
    pub wasm_ty: ValType,
    pub init: CHIRExpr,
}

impl CHIRExpr {
    /// 创建一个简单的表达式
    pub fn new(kind: CHIRExprKind, ty: Type, wasm_ty: ValType) -> Self {
        CHIRExpr {
            kind,
            ty,
            wasm_ty,
            span: None,
        }
    }

    /// 创建整数常量
    pub fn int_const(value: i64, ty: Type) -> Self {
        let wasm_ty = ty.to_wasm();
        CHIRExpr::new(CHIRExprKind::Integer(value), ty, wasm_ty)
    }

    /// 创建布尔常量
    pub fn bool_const(value: bool) -> Self {
        CHIRExpr::new(
            CHIRExprKind::Bool(value),
            Type::Bool,
            ValType::I32,
        )
    }
}

impl CHIRBlock {
    /// 创建空块
    pub fn empty() -> Self {
        CHIRBlock {
            stmts: Vec::new(),
            result: None,
        }
    }

    /// 创建只有一个表达式的块
    pub fn from_expr(expr: CHIRExpr) -> Self {
        CHIRBlock {
            stmts: Vec::new(),
            result: Some(Box::new(expr)),
        }
    }
}
