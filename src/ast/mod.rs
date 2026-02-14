use wasm_encoder::ValType;

/// 仓颉语言类型
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Type {
    Int8,
    Int16,
    Int32,
    Int64,
    UInt8,
    UInt16,
    UInt32,
    UInt64,
    Float32,
    Float64,
    Bool,
    Char,
    Unit,
    /// 字符串类型 (指针, 在内存中存储)
    String,
    /// 数组类型 Array<T>
    Array(Box<Type>),
    /// 结构体类型 (可选类型实参，如 Point 或 Pair<Int64,String>)
    Struct(String, Vec<Type>),
    /// 元组类型 (T1, T2, ...)
    Tuple(Vec<Type>),
    /// 范围类型 (start..end 或 start..=end)
    /// 内存布局: [start: i64][end: i64][inclusive: i32] = 20 bytes
    Range,
    /// 函数类型 (param_types) -> return_type，用于 Lambda
    /// 在 WASM 中以 i32 函数表索引表示
    Function {
        params: Vec<Type>,
        ret: Box<Option<Type>>,
    },
    /// Option<T> 类型
    Option(Box<Type>),
    /// Result<T, E> 类型
    Result(Box<Type>, Box<Type>),
    /// 切片类型 Slice<T>，引用数组子区间 [ptr, len]
    Slice(Box<Type>),
    /// Map 类型 Map<K, V>
    Map(Box<Type>, Box<Type>),
    /// 泛型类型参数 (如 T)，仅在泛型定义体内使用，单态化时替换为具体类型
    TypeParam(String),
}

impl Type {
    /// 转换为 WASM 值类型
    pub fn to_wasm(&self) -> ValType {
        match self {
            Type::Int8 => ValType::I32,   // 小整数映射为 i32
            Type::Int16 => ValType::I32,
            Type::Int32 => ValType::I32,
            Type::Int64 => ValType::I64,
            Type::UInt8 => ValType::I32,   // 无符号小整数映射为 i32
            Type::UInt16 => ValType::I32,
            Type::UInt32 => ValType::I32,
            Type::UInt64 => ValType::I64,  // UInt64 映射为 i64
            Type::Float32 => ValType::F32,
            Type::Float64 => ValType::F64,
            Type::Bool => ValType::I32,
            Type::Char => ValType::I32,    // Unicode code point 映射为 i32
            Type::Unit => panic!("Unit 类型不能转换为 WASM 值类型"),
            // 复合类型都用 i32 指针表示
            Type::String => ValType::I32,
            Type::Array(_) => ValType::I32,
            Type::Tuple(_) => ValType::I32,  // 堆指针
            Type::Struct(..) => ValType::I32,
            Type::Range => ValType::I32,    // 指针
            Type::Function { .. } => ValType::I32, // 函数表索引
            Type::Option(_) => ValType::I32,      // 指针
            Type::Result(_, _) => ValType::I32,   // 指针
            Type::Slice(_) => ValType::I32,        // 指针
            Type::Map(_, _) => ValType::I32,       // 指针
            Type::TypeParam(_) => panic!("TypeParam 不能直接转换为 WASM，需先单态化"),
        }
    }

    /// 获取类型在内存中的大小 (字节)
    pub fn size(&self) -> u32 {
        match self {
            Type::Int8 | Type::UInt8 => 1,
            Type::Int16 | Type::UInt16 => 2,
            Type::Int32 | Type::UInt32 | Type::Bool | Type::Char => 4,
            Type::Int64 | Type::UInt64 => 8,
            Type::Float32 => 4,
            Type::Float64 => 8,
            Type::Unit => 0,
            Type::String => 4,      // 指针大小
            Type::Array(_) => 4,    // 指针大小
            Type::Tuple(_) => 4,    // 指针大小
            Type::Struct(..) => 4,   // 指针大小
            Type::Range => 4,       // 指针大小
            Type::Function { .. } => 4, // 函数表索引大小
            Type::Option(_) => 4,   // 指针大小
            Type::Result(_, _) => 4, // 指针大小
            Type::Slice(_) => 4,     // 指针大小
            Type::Map(_, _) => 4,    // 指针大小
            Type::TypeParam(_) => panic!("TypeParam 不能直接计算 size，需先单态化"),
        }
    }

    /// 获取 Range 类型在堆上的实际大小
    pub fn range_heap_size() -> u32 {
        // [start: i64][end: i64][inclusive: i32] = 8 + 8 + 4 = 20 bytes
        20
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_encoder::ValType;

    #[test]
    fn test_to_wasm_all_types() {
        assert_eq!(Type::Int8.to_wasm(), ValType::I32);
        assert_eq!(Type::Int16.to_wasm(), ValType::I32);
        assert_eq!(Type::Int32.to_wasm(), ValType::I32);
        assert_eq!(Type::Int64.to_wasm(), ValType::I64);
        assert_eq!(Type::UInt8.to_wasm(), ValType::I32);
        assert_eq!(Type::UInt16.to_wasm(), ValType::I32);
        assert_eq!(Type::UInt32.to_wasm(), ValType::I32);
        assert_eq!(Type::UInt64.to_wasm(), ValType::I64);
        assert_eq!(Type::Float32.to_wasm(), ValType::F32);
        assert_eq!(Type::Float64.to_wasm(), ValType::F64);
        assert_eq!(Type::Bool.to_wasm(), ValType::I32);
        assert_eq!(Type::Char.to_wasm(), ValType::I32);
        assert_eq!(Type::String.to_wasm(), ValType::I32);
        assert_eq!(Type::Array(Box::new(Type::Int64)).to_wasm(), ValType::I32);
        assert_eq!(Type::Tuple(vec![Type::Int64, Type::Int64]).to_wasm(), ValType::I32);
        assert_eq!(Type::Struct("Foo".to_string(), vec![]).to_wasm(), ValType::I32);
        assert_eq!(Type::Range.to_wasm(), ValType::I32);
        assert_eq!(Type::Function { params: vec![], ret: Box::new(Some(Type::Int64)) }.to_wasm(), ValType::I32);
        assert_eq!(Type::Option(Box::new(Type::Int64)).to_wasm(), ValType::I32);
        assert_eq!(Type::Result(Box::new(Type::Int64), Box::new(Type::String)).to_wasm(), ValType::I32);
        assert_eq!(Type::Slice(Box::new(Type::Int64)).to_wasm(), ValType::I32);
        assert_eq!(Type::Map(Box::new(Type::String), Box::new(Type::Int64)).to_wasm(), ValType::I32);
    }

    #[test]
    #[should_panic(expected = "Unit 类型不能转换为 WASM")]
    fn test_to_wasm_unit_panic() {
        Type::Unit.to_wasm();
    }

    #[test]
    #[should_panic(expected = "TypeParam 不能直接转换")]
    fn test_to_wasm_typeparam_panic() {
        Type::TypeParam("T".to_string()).to_wasm();
    }

    #[test]
    fn test_size_all_types() {
        assert_eq!(Type::Int8.size(), 1);
        assert_eq!(Type::UInt8.size(), 1);
        assert_eq!(Type::Int16.size(), 2);
        assert_eq!(Type::UInt16.size(), 2);
        assert_eq!(Type::Int32.size(), 4);
        assert_eq!(Type::UInt32.size(), 4);
        assert_eq!(Type::Bool.size(), 4);
        assert_eq!(Type::Char.size(), 4);
        assert_eq!(Type::Int64.size(), 8);
        assert_eq!(Type::UInt64.size(), 8);
        assert_eq!(Type::Float32.size(), 4);
        assert_eq!(Type::Float64.size(), 8);
        assert_eq!(Type::Unit.size(), 0);
        assert_eq!(Type::String.size(), 4);
        assert_eq!(Type::Array(Box::new(Type::Int64)).size(), 4);
        assert_eq!(Type::Tuple(vec![]).size(), 4);
        assert_eq!(Type::Struct("S".to_string(), vec![]).size(), 4);
        assert_eq!(Type::Range.size(), 4);
        assert_eq!(Type::Function { params: vec![], ret: Box::new(Some(Type::Int64)) }.size(), 4);
        assert_eq!(Type::Option(Box::new(Type::Int64)).size(), 4);
        assert_eq!(Type::Result(Box::new(Type::Int64), Box::new(Type::String)).size(), 4);
        assert_eq!(Type::Slice(Box::new(Type::Int64)).size(), 4);
        assert_eq!(Type::Map(Box::new(Type::String), Box::new(Type::Int64)).size(), 4);
    }

    #[test]
    #[should_panic(expected = "TypeParam 不能直接计算 size")]
    fn test_size_typeparam_panic() {
        Type::TypeParam("T".to_string()).size();
    }

    #[test]
    fn test_range_heap_size() {
        assert_eq!(Type::range_heap_size(), 20);
    }
}

/// 一元运算符
#[derive(Debug, Clone, PartialEq)]
pub enum UnaryOp {
    Not,     // !
    Neg,     // - 负号
    BitNot,  // ~ 按位取反
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
    Pow,         // ** 幂运算
    BitAnd,      // &
    BitOr,       // |
    BitXor,      // ^
    Shl,         // <<
    Shr,         // >>
    UShr,        // >>>
}

/// 字符串插值的部分
#[derive(Debug, Clone)]
pub enum InterpolatePart {
    /// 字面量文本
    Literal(String),
    /// 插值表达式
    Expr(Box<Expr>),
}

/// 表达式
#[derive(Debug, Clone)]
pub enum Expr {
    /// 整数字面量
    Integer(i64),
    /// 浮点数字面量 (Float64)
    Float(f64),
    /// Float32 字面量 (后缀 f)
    Float32(f32),
    /// 布尔字面量
    Bool(bool),
    /// 字符字面量 (Unicode code point)
    Char(char),
    /// 字符串字面量
    String(String),
    /// 字符串插值 "Hello, ${name}!"
    /// 各部分依次为字面量文本或表达式
    Interpolate(Vec<InterpolatePart>),
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
    /// 函数调用 (可选显式类型实参，如 identity<Int64>(42))
    Call {
        name: String,
        type_args: Option<Vec<Type>>,
        args: Vec<Expr>,
    },
    /// 方法调用 (obj.method(args))
    MethodCall {
        object: Box<Expr>,
        method: String,
        args: Vec<Expr>,
    },
    /// super 调用：super.method(args) 或 super(args) 调用父类
    SuperCall {
        method: String,
        args: Vec<Expr>,
    },
    /// if 表达式
    If {
        cond: Box<Expr>,
        then_branch: Box<Expr>,
        else_branch: Option<Box<Expr>>,
    },
    /// if-let 表达式：若 pattern 匹配 expr 则求值 then_branch，否则 else_branch
    IfLet {
        pattern: Pattern,
        expr: Box<Expr>,
        then_branch: Box<Expr>,
        else_branch: Option<Box<Expr>>,
    },
    /// 代码块
    Block(Vec<Stmt>, Option<Box<Expr>>),
    /// 元组字面量 (a, b, c)
    Tuple(Vec<Expr>),
    /// 元组索引访问 tuple.0, tuple.1
    TupleIndex {
        object: Box<Expr>,
        index: u32,
    },
    /// 数组字面量 [1, 2, 3]
    Array(Vec<Expr>),
    /// 数组访问 arr[index]
    Index {
        array: Box<Expr>,
        index: Box<Expr>,
    },
    /// 结构体实例化 Point { x: 1, y: 2 } 或 Pair<Int64,String> { first: 1, second: "hi" }
    StructInit {
        name: String,
        type_args: Option<Vec<Type>>,
        fields: Vec<(String, Expr)>,
    },
    /// 构造函数调用 Point(1, 2) 或 Pair<Int64,String>(1, "hi")，在 codegen 中转换为 StructInit
    ConstructorCall {
        name: String,
        type_args: Option<Vec<Type>>,
        args: Vec<Expr>,
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
    /// 枚举变体构造 Color.Red 或 Result.Ok(42)（无关联值时值为 i32 判别式，有关联值为堆指针）
    VariantConst {
        enum_name: String,
        variant_name: String,
        /// 关联值，如 Ok(42) 的 42
        arg: Option<Box<Expr>>,
    },
    /// match 表达式
    Match {
        expr: Box<Expr>,
        arms: Vec<MatchArm>,
    },
    /// 类型转换 expr as Type
    Cast {
        expr: Box<Expr>,
        target_ty: Type,
    },
    /// Lambda 表达式 (x: Int64) -> Int64 { x * 2 } 或 { x: Int64 => x * 2 }
    Lambda {
        params: Vec<(String, Type)>,
        return_type: Option<Type>,
        body: Box<Expr>,
    },
    /// Some(value) 表达式
    Some(Box<Expr>),
    /// None 表达式
    None,
    /// Ok(value) 表达式
    Ok(Box<Expr>),
    /// Err(value) 表达式
    Err(Box<Expr>),
    /// 空值合并: a ?? b，若 a 为 Some(v) 则返回 v，否则返回 b
    NullCoalesce {
        option: Box<Expr>,
        default: Box<Expr>,
    },
    /// ? 运算符 (expr?)，若为 Err/None 则提前返回
    Try(Box<Expr>),
    /// throw 表达式
    Throw(Box<Expr>),
    /// try 块表达式（支持 try-catch-finally）
    TryBlock {
        body: Vec<Stmt>,
        catch_var: Option<String>,
        catch_body: Vec<Stmt>,
        /// finally 块（无论是否异常都执行）
        finally_body: Option<Vec<Stmt>>,
    },
    /// 切片表达式 arr[start..end]
    SliceExpr {
        array: Box<Expr>,
        start: Box<Expr>,
        end: Box<Expr>,
    },
    /// Map 字面量 Map<K, V> { key1 => val1, key2 => val2 }
    MapLiteral {
        entries: Vec<(Expr, Expr)>,
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
    /// 枚举变体模式 Color.Red 或 Result.Ok(v)（匹配时用，binding 为关联值绑定名）
    Variant {
        enum_name: String,
        variant_name: String,
        /// 关联值绑定名，如 Ok(v) 的 v
        binding: Option<String>,
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
    /// let 绑定 (不可变)，pattern 为 Binding(name) 或 Struct 解构
    Let {
        pattern: Pattern,
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
    /// while-let 循环：当 expr 匹配 pattern 时执行 body，否则退出
    WhileLet {
        pattern: Pattern,
        expr: Box<Expr>,
        body: Vec<Stmt>,
    },
    /// for 循环 (for i in 0..10 { ... })
    For {
        var: String,
        iterable: Expr,
        body: Vec<Stmt>,
    },
    /// loop 无限循环 loop { ... }
    Loop { body: Vec<Stmt> },
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
    /// cjc 兼容: 字段默认值 (如 `var x: Int64 = 0`)
    pub default: Option<Expr>,
}

/// 结构体定义
#[derive(Debug, Clone)]
pub struct StructDef {
    pub visibility: Visibility,
    pub name: String,
    /// 泛型类型参数，如 struct Pair<T,U> 的 ["T","U"]
    pub type_params: Vec<String>,
    /// 类型约束
    pub constraints: Vec<TypeConstraint>,
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

/// 函数参数（可选默认值、可变参数）
#[derive(Debug, Clone)]
pub struct Param {
    pub name: String,
    pub ty: Type,
    /// 默认值，如 power(base, exp = 2) 的 2
    pub default: Option<Expr>,
    /// 可变参数，如 sum(args: Int64...)，调用时 args 展开为数组
    pub variadic: bool,
}

/// 函数定义
/// 外部函数导入：WASM 从宿主导入 (module, name)
#[derive(Debug, Clone, Default)]
pub struct ExternImport {
    /// 模块名，如 "env"、"wasi_snapshot_preview1"
    pub module: String,
    /// 导入名，如 "print"、"fd_write"
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct Function {
    pub visibility: Visibility,
    pub name: String,
    /// 泛型类型参数，如 func identity<T>(x: T) 的 ["T"]
    pub type_params: Vec<String>,
    /// 类型约束（来自 `<T: Bound>` 或 `where T: Bound`）
    pub constraints: Vec<TypeConstraint>,
    pub params: Vec<Param>,
    pub return_type: Option<Type>,
    /// throws 声明的异常类型（如 func f() throws MyError -> Int64）
    pub throws: Option<String>,
    pub body: Vec<Stmt>,
    /// 若为 Some，则为 extern 函数，从 (module, name) 导入；无 body
    pub extern_import: Option<ExternImport>,
}

/// 枚举变体（可选关联类型）
#[derive(Debug, Clone)]
pub struct EnumVariant {
    pub name: String,
    /// 关联值类型，如 Ok(Int64) 的 Int64
    pub payload: Option<Type>,
}

/// 接口方法签名（可选默认实现）
#[derive(Debug, Clone)]
pub struct InterfaceMethod {
    pub name: String,
    pub params: Vec<Param>,
    pub return_type: Option<Type>,
    /// 默认实现体（若有则为 Some）
    pub default_body: Option<Vec<Stmt>>,
}

/// 接口关联类型定义（如 type Element）
#[derive(Debug, Clone)]
pub struct AssocTypeDef {
    pub name: String,
}

/// 接口定义
#[derive(Debug, Clone)]
pub struct InterfaceDef {
    pub visibility: Visibility,
    pub name: String,
    /// 父接口列表（接口继承）
    pub parents: Vec<String>,
    pub methods: Vec<InterfaceMethod>,
    /// 关联类型列表
    pub assoc_types: Vec<AssocTypeDef>,
}

/// 扩展定义（为已有类型追加方法/实现接口）
/// extend TypeName: InterfaceName { ... }
#[derive(Debug, Clone)]
pub struct ExtendDef {
    /// 被扩展的类型名
    pub target_type: String,
    /// 实现的接口（可选）
    pub interface: Option<String>,
    /// 关联类型绑定（如 type Element = Int64）
    pub assoc_type_bindings: Vec<(String, Type)>,
    /// 扩展的方法
    pub methods: Vec<Function>,
}

/// 类定义（支持继承、init、override、super、abstract、sealed）
#[derive(Debug, Clone)]
pub struct ClassDef {
    pub visibility: Visibility,
    pub name: String,
    /// 泛型类型参数
    pub type_params: Vec<String>,
    /// 类型约束
    pub constraints: Vec<TypeConstraint>,
    /// 是否为 abstract 类（不能直接实例化）
    pub is_abstract: bool,
    /// 是否为 sealed 类（不能被继承）
    pub is_sealed: bool,
    /// 是否为 open 类（允许被继承，仓颉中类默认不可继承）
    pub is_open: bool,
    /// 继承的父类
    pub extends: Option<String>,
    /// 实现的接口列表
    pub implements: Vec<String>,
    pub fields: Vec<FieldDef>,
    /// init 构造函数（无方法名，参数列表 + body）
    pub init: Option<InitDef>,
    /// deinit 析构函数
    pub deinit: Option<Vec<Stmt>>,
    /// 方法（含 override 标记）
    pub methods: Vec<ClassMethod>,
}

/// 构造函数定义
#[derive(Debug, Clone)]
pub struct InitDef {
    pub params: Vec<Param>,
    pub body: Vec<Stmt>,
}

/// 类方法（含 override 标记）
#[derive(Debug, Clone)]
pub struct ClassMethod {
    pub override_: bool,
    pub func: Function,
}

/// 属性定义（getter/setter）
#[derive(Debug, Clone)]
pub struct PropDef {
    pub name: String,
    pub ty: Type,
    pub getter: Option<Vec<Stmt>>,
    pub setter: Option<(String, Vec<Stmt>)>, // (参数名, body)
}

/// 类型参数约束：<T: Comparable & Hashable>
#[derive(Debug, Clone)]
pub struct TypeConstraint {
    /// 类型参数名（如 "T"）
    pub param: String,
    /// 约束的接口名列表（如 ["Comparable", "Hashable"]）
    pub bounds: Vec<String>,
}

/// 枚举定义（支持无关联值或单关联值变体，支持泛型）
#[derive(Debug, Clone)]
pub struct EnumDef {
    pub visibility: Visibility,
    pub name: String,
    /// 泛型类型参数，如 enum Result<T, E> 的 ["T", "E"]
    pub type_params: Vec<String>,
    /// 类型约束
    pub constraints: Vec<TypeConstraint>,
    pub variants: Vec<EnumVariant>,
}

impl EnumDef {
    pub fn variant_index(&self, name: &str) -> Option<u32> {
        self.variants.iter().position(|v| v.name == name).map(|i| i as u32)
    }

    pub fn variant_payload(&self, name: &str) -> Option<&Type> {
        self.variants.iter().find(|v| v.name == name).and_then(|v| v.payload.as_ref())
    }

    /// 是否有任意变体带关联值
    pub fn has_payload(&self) -> bool {
        self.variants.iter().any(|v| v.payload.is_some())
    }

    /// 所有变体 payload 类型的最大尺寸（字节），用于堆布局
    pub fn payload_size(&self) -> u32 {
        self.variants
            .iter()
            .filter_map(|v| v.payload.as_ref())
            .map(|t| t.size())
            .max()
            .unwrap_or(0)
    }
}

/// 可见性修饰符
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Visibility {
    #[default]
    Private,
    Public,
    /// 模块内可见
    Internal,
}

/// 导入项
#[derive(Debug, Clone)]
pub struct Import {
    /// 导入的模块路径，如 "std.io"
    pub module_path: Vec<String>,
    /// 导入的具体项，None 表示 import * from module
    pub items: Option<Vec<String>>,
    /// 别名，如 import foo as bar
    pub alias: Option<String>,
}

/// 程序 (模块)
#[derive(Debug, Clone)]
pub struct Program {
    /// 模块名称，None 表示主模块
    pub module_name: Option<String>,
    /// 导入列表
    pub imports: Vec<Import>,
    pub structs: Vec<StructDef>,
    pub interfaces: Vec<InterfaceDef>,
    pub classes: Vec<ClassDef>,
    pub enums: Vec<EnumDef>,
    pub functions: Vec<Function>,
    /// 扩展定义
    pub extends: Vec<ExtendDef>,
}
