use wasm_encoder::ValType;

/// 仓颉语言类型
#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    Int32,
    Int64,
    Float64,
    Bool,
    Unit,
    /// 字符串类型 (指针, 在内存中存储)
    String,
    /// 数组类型 Array<T>
    Array(Box<Type>),
    /// 结构体类型
    Struct(String),
}

impl Type {
    /// 转换为 WASM 值类型
    pub fn to_wasm(&self) -> ValType {
        match self {
            Type::Int32 => ValType::I32,
            Type::Int64 => ValType::I64,
            Type::Float64 => ValType::F64,
            Type::Bool => ValType::I32,
            Type::Unit => panic!("Unit 类型不能转换为 WASM 值类型"),
            // 复合类型都用 i32 指针表示
            Type::String => ValType::I32,
            Type::Array(_) => ValType::I32,
            Type::Struct(_) => ValType::I32,
        }
    }

    /// 获取类型在内存中的大小 (字节)
    pub fn size(&self) -> u32 {
        match self {
            Type::Int32 | Type::Bool => 4,
            Type::Int64 => 8,
            Type::Float64 => 8,
            Type::Unit => 0,
            Type::String => 4,      // 指针大小
            Type::Array(_) => 4,    // 指针大小
            Type::Struct(_) => 4,   // 指针大小
        }
    }
}

/// 一元运算符
#[derive(Debug, Clone, PartialEq)]
pub enum UnaryOp {
    Not,  // !
    Neg,  // - 负号
}

/// 二元运算符
#[derive(Debug, Clone, PartialEq)]
pub enum BinOp {
    Add,    // +
    Sub,    // -
    Mul,    // *
    Div,    // /
    Mod,    // %
    Eq,     // ==
    NotEq,  // !=
    Lt,     // <
    Gt,     // >
    LtEq,   // <=
    GtEq,   // >=
    LogicalAnd,  // &&
    LogicalOr,   // ||
}

/// 表达式
#[derive(Debug, Clone)]
pub enum Expr {
    /// 整数字面量
    Integer(i64),
    /// 浮点数字面量
    Float(f64),
    /// 布尔字面量
    Bool(bool),
    /// 字符串字面量
    String(String),
    /// 变量引用
    Var(String),
    /// 一元运算 (! 等)
    Unary {
        op: UnaryOp,
        expr: Box<Expr>,
    },
    /// 二元运算
    Binary {
        op: BinOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    /// 函数调用
    Call {
        name: String,
        args: Vec<Expr>,
    },
    /// 方法调用 (obj.method(args))
    MethodCall {
        object: Box<Expr>,
        method: String,
        args: Vec<Expr>,
    },
    /// if 表达式
    If {
        cond: Box<Expr>,
        then_branch: Box<Expr>,
        else_branch: Option<Box<Expr>>,
    },
    /// 代码块
    Block(Vec<Stmt>, Option<Box<Expr>>),
    /// 数组字面量 [1, 2, 3]
    Array(Vec<Expr>),
    /// 数组访问 arr[index]
    Index {
        array: Box<Expr>,
        index: Box<Expr>,
    },
    /// 结构体实例化 Point { x: 1, y: 2 }
    StructInit {
        name: String,
        fields: Vec<(String, Expr)>,
    },
    /// 字段访问 point.x
    Field {
        object: Box<Expr>,
        field: String,
    },
    /// 范围表达式 0..10 或 0..=10
    Range {
        start: Box<Expr>,
        end: Box<Expr>,
        inclusive: bool,  // true 表示 ..=
    },
    /// 枚举变体构造 Color.Red（值为 i32 判别式）
    VariantConst {
        enum_name: String,
        variant_name: String,
    },
    /// match 表达式
    Match {
        expr: Box<Expr>,
        arms: Vec<MatchArm>,
    },
}

/// match 分支
#[derive(Debug, Clone)]
pub struct MatchArm {
    pub pattern: Pattern,
    pub guard: Option<Box<Expr>>,  // if 守卫条件
    pub body: Box<Expr>,
}

/// 模式
#[derive(Debug, Clone)]
pub enum Pattern {
    /// 通配符 _
    Wildcard,
    /// 字面量模式 1, "hello", true
    Literal(Literal),
    /// 变量绑定 x
    Binding(String),
    /// 范围模式 1..10, 'a'..='z'
    Range {
        start: Literal,
        end: Literal,
        inclusive: bool,
    },
    /// 多模式 1 | 2 | 3
    Or(Vec<Pattern>),
    /// 结构体解构 Point { x, y }
    Struct {
        name: String,
        fields: Vec<(String, Pattern)>,
    },
    /// 元组解构 (a, b)
    Tuple(Vec<Pattern>),
    /// 枚举变体模式 Color.Red 或 Red（匹配时用）
    Variant {
        enum_name: String,
        variant_name: String,
    },
}

/// 字面量值 (用于模式匹配)
#[derive(Debug, Clone)]
pub enum Literal {
    Integer(i64),
    Float(f64),
    Bool(bool),
    String(String),
    Char(char),
}

/// 语句
#[derive(Debug, Clone)]
pub enum Stmt {
    /// let 绑定 (不可变)
    Let {
        name: String,
        ty: Option<Type>,
        value: Expr,
    },
    /// var 绑定 (可变)
    Var {
        name: String,
        ty: Option<Type>,
        value: Expr,
    },
    /// 赋值
    Assign {
        target: AssignTarget,
        value: Expr,
    },
    /// 表达式语句
    Expr(Expr),
    /// return 语句
    Return(Option<Expr>),
    /// while 循环
    While {
        cond: Expr,
        body: Vec<Stmt>,
    },
    /// for 循环 (for i in 0..10 { ... })
    For {
        var: String,
        iterable: Expr,
        body: Vec<Stmt>,
    },
    /// break（跳出当前循环）
    Break,
    /// continue（跳到当前循环下一轮）
    Continue,
}

/// 赋值目标
#[derive(Debug, Clone)]
pub enum AssignTarget {
    /// 变量
    Var(String),
    /// 数组元素 arr[i]
    Index { array: String, index: Box<Expr> },
    /// 结构体字段 obj.field
    Field { object: String, field: String },
}

/// 结构体字段定义
#[derive(Debug, Clone)]
pub struct FieldDef {
    pub name: String,
    pub ty: Type,
}

/// 结构体定义
#[derive(Debug, Clone)]
pub struct StructDef {
    pub name: String,
    pub fields: Vec<FieldDef>,
}

impl StructDef {
    /// 计算结构体总大小
    pub fn size(&self) -> u32 {
        self.fields.iter().map(|f| f.ty.size()).sum()
    }

    /// 获取字段偏移量
    pub fn field_offset(&self, field_name: &str) -> Option<u32> {
        let mut offset = 0;
        for f in &self.fields {
            if f.name == field_name {
                return Some(offset);
            }
            offset += f.ty.size();
        }
        None
    }

    /// 获取字段类型
    pub fn field_type(&self, field_name: &str) -> Option<&Type> {
        self.fields.iter().find(|f| f.name == field_name).map(|f| &f.ty)
    }
}

/// 函数参数
#[derive(Debug, Clone)]
pub struct Param {
    pub name: String,
    pub ty: Type,
}

/// 函数定义
#[derive(Debug, Clone)]
pub struct Function {
    pub name: String,
    pub params: Vec<Param>,
    pub return_type: Option<Type>,
    pub body: Vec<Stmt>,
}

/// 枚举定义（简单枚举，无关联值）
#[derive(Debug, Clone)]
pub struct EnumDef {
    pub name: String,
    pub variants: Vec<String>,
}

impl EnumDef {
    pub fn variant_index(&self, name: &str) -> Option<u32> {
        self.variants.iter().position(|v| v == name).map(|i| i as u32)
    }
}

/// 程序 (模块)
#[derive(Debug)]
pub struct Program {
    pub structs: Vec<StructDef>,
    pub enums: Vec<EnumDef>,
    pub functions: Vec<Function>,
}
