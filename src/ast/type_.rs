//! 仓颉类型 (与 cjc release/1.0 对齐)

use wasm_encoder::ValType;

/// 仓颉语言类型 (与 cjc release/1.0 对齐)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Type {
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
    Rune,
    Bool,
    Nothing,
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
    /// P2: This 类型 - 表示当前类的类型，用于链式调用
    /// 在类方法中，This 会被解析为当前类的类型
    This,
    /// P1: 限定类型 - 模块化类型引用，如 pkg.Module.Type
    Qualified(Vec<String>), // 路径段，如 ["pkg", "Module", "Type"]
}

impl Type {
    /// 转换为 WASM 值类型
    pub fn to_wasm(&self) -> ValType {
        match self {
            Type::Int8 => ValType::I32,
            Type::Int16 => ValType::I32,
            Type::Int32 => ValType::I32,
            Type::Int64 => ValType::I64,
            Type::IntNative => ValType::I64,
            Type::UInt8 => ValType::I32,
            Type::UInt16 => ValType::I32,
            Type::UInt32 => ValType::I32,
            Type::UInt64 => ValType::I64,
            Type::UIntNative => ValType::I64,
            Type::Float16 => ValType::F32,
            Type::Float32 => ValType::F32,
            Type::Float64 => ValType::F64,
            Type::Rune => ValType::I32,
            Type::Bool => ValType::I32,
            Type::Nothing => panic!("Nothing 类型不能转换为 WASM 值类型"),
            Type::Unit => panic!("Unit 类型不能转换为 WASM 值类型"),
            Type::String => ValType::I32,
            Type::Array(_) => ValType::I32,
            Type::Tuple(_) => ValType::I32,
            Type::Struct(..) => ValType::I32,
            Type::Range => ValType::I32,
            Type::Function { .. } => ValType::I32,
            Type::Option(_) => ValType::I32,
            Type::Result(_, _) => ValType::I32,
            Type::Slice(_) => ValType::I32,
            Type::Map(_, _) => ValType::I32,
            Type::TypeParam(_) => {
                eprintln!("警告: TypeParam 转换为 i64（需要单态化）");
                ValType::I64
            }
            Type::This => ValType::I32, // This 类型表示类对象，使用指针
            Type::Qualified(_) => ValType::I32, // 限定类型通常是类或结构体
        }
    }

    /// 获取类型在内存中的大小 (字节)
    pub fn size(&self) -> u32 {
        match self {
            Type::Int8 | Type::UInt8 => 1,
            Type::Int16 | Type::UInt16 => 2,
            Type::Int32 | Type::UInt32 | Type::Bool | Type::Rune => 4,
            Type::Int64 | Type::UInt64 | Type::IntNative | Type::UIntNative => 8,
            Type::Float16 => 2,
            Type::Float32 => 4,
            Type::Float64 => 8,
            Type::Nothing => 0,
            Type::Unit => 0,
            Type::String => 4,
            Type::Array(_) => 4,
            Type::Tuple(_) => 4,
            Type::Struct(..) => 4,
            Type::Range => 4,
            Type::Function { .. } => 4,
            Type::Option(_) => 4,
            Type::Result(_, _) => 4,
            Type::Slice(_) => 4,
            Type::Map(_, _) => 4,
            Type::TypeParam(_) => 8, // 默认指针大小（单态化前使用）
            Type::This => 4,         // This 类型表示类对象指针
            Type::Qualified(_) => 4, // 限定类型通常是类或结构体指针
        }
    }

    /// 获取 Range 类型在堆上的实际大小
    pub fn range_heap_size() -> u32 {
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
        assert_eq!(Type::Int64.to_wasm(), ValType::I64);
        assert_eq!(Type::Float32.to_wasm(), ValType::F32);
        assert_eq!(Type::Float64.to_wasm(), ValType::F64);
        assert_eq!(Type::String.to_wasm(), ValType::I32);
        assert_eq!(Type::Array(Box::new(Type::Int64)).to_wasm(), ValType::I32);
        assert_eq!(Type::Option(Box::new(Type::Int64)).to_wasm(), ValType::I32);
    }

    #[test]
    #[should_panic(expected = "Nothing 类型不能转换为 WASM")]
    fn test_to_wasm_nothing_panic() {
        Type::Nothing.to_wasm();
    }

    #[test]
    #[should_panic(expected = "Unit 类型不能转换为 WASM")]
    fn test_to_wasm_unit_panic() {
        Type::Unit.to_wasm();
    }

    #[test]
    fn test_to_wasm_typeparam() {
        assert_eq!(Type::TypeParam("T".to_string()).to_wasm(), ValType::I64);
    }

    #[test]
    fn test_size_all_types() {
        assert_eq!(Type::Int8.size(), 1);
        assert_eq!(Type::Int64.size(), 8);
        assert_eq!(Type::Float64.size(), 8);
        assert_eq!(Type::Unit.size(), 0);
        assert_eq!(Type::String.size(), 4);
    }

    #[test]
    fn test_size_typeparam_default() {
        // TypeParam returns default pointer size (8) without panicking
        assert_eq!(Type::TypeParam("T".to_string()).size(), 8);
    }

    #[test]
    fn test_range_heap_size() {
        assert_eq!(Type::range_heap_size(), 20);
    }
}
