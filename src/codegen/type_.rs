//! 类型与布局：类型名字修饰、函数类型索引、类型别名解析等。

use crate::ast::Type;
use wasm_encoder::ValType;

use super::CodeGen;

impl CodeGen {
    /// 重载名字修饰：name$Type1$Type2 用于多态解析
    pub(crate) fn type_mangle_suffix(ty: &Type) -> String {
        match ty {
            Type::Int8 => "Int8".to_string(),
            Type::Int16 => "Int16".to_string(),
            Type::Int32 => "Int32".to_string(),
            Type::Int64 => "Int64".to_string(),
            Type::UInt8 => "UInt8".to_string(),
            Type::UInt16 => "UInt16".to_string(),
            Type::UInt32 => "UInt32".to_string(),
            Type::UInt64 => "UInt64".to_string(),
            Type::Float32 => "Float32".to_string(),
            Type::Float64 => "Float64".to_string(),
            Type::Bool => "Bool".to_string(),
            Type::Rune => "Rune".to_string(),
            Type::IntNative => "IntNative".to_string(),
            Type::UIntNative => "UIntNative".to_string(),
            Type::Float16 => "Float16".to_string(),
            Type::Nothing => "Nothing".to_string(),
            Type::Unit => "Unit".to_string(),
            Type::String => "String".to_string(),
            Type::Array(inner) => format!("Array_{}", Self::type_mangle_suffix(inner)),
            Type::Tuple(types) => format!(
                "Tuple_{}",
                types
                    .iter()
                    .map(Self::type_mangle_suffix)
                    .collect::<Vec<_>>()
                    .join("_")
            ),
            Type::Struct(s, args) => {
                if args.is_empty() {
                    s.clone()
                } else {
                    format!(
                        "{}_{}",
                        s,
                        args.iter()
                            .map(Self::type_mangle_suffix)
                            .collect::<Vec<_>>()
                            .join("_")
                    )
                }
            }
            Type::Range => "Range".to_string(),
            Type::Function { params, ret } => {
                let params_str = params
                    .iter()
                    .map(Self::type_mangle_suffix)
                    .collect::<Vec<_>>()
                    .join("_");
                let ret_str = ret
                    .as_ref()
                    .as_ref()
                    .map(Self::type_mangle_suffix)
                    .unwrap_or_else(|| "Unit".to_string());
                format!("Fn_{}_{}", params_str, ret_str)
            }
            Type::Option(inner) => format!("Option_{}", Self::type_mangle_suffix(inner)),
            Type::Result(ok, err) => format!(
                "Result_{}_{}",
                Self::type_mangle_suffix(ok),
                Self::type_mangle_suffix(err)
            ),
            Type::TypeParam(name) => name.clone(), // 单态化前用于名字修饰的占位
            Type::Slice(inner) => format!("Slice_{}", Self::type_mangle_suffix(inner)),
            Type::Map(k, v) => format!(
                "Map_{}_{}",
                Self::type_mangle_suffix(k),
                Self::type_mangle_suffix(v)
            ),
            Type::This => "This".to_string(), // P2: This 类型
            Type::Qualified(path) => path.join("_"), // P1: 限定类型，如 pkg.Module.Type
        }
    }

    /// 函数名 + 参数类型列表 → 修饰后的 key（用于重载解析）
    pub(crate) fn mangle_key(name: &str, param_tys: &[Type]) -> String {
        if param_tys.is_empty() {
            format!("{}$_", name)
        } else {
            format!(
                "{}${}",
                name,
                param_tys
                    .iter()
                    .map(Self::type_mangle_suffix)
                    .collect::<Vec<_>>()
                    .join("$")
            )
        }
    }

    /// P2.3: 查找匹配的函数类型索引（用于 call_indirect）
    pub(crate) fn find_or_create_func_type_idx(
        &self,
        params: &[ValType],
        results: &[ValType],
    ) -> u32 {
        let sig = (params.to_vec(), results.to_vec());
        if let Some(&type_idx) = self.func_type_by_sig.get(&sig) {
            return type_idx;
        }
        // fallback: 返回 0
        0
    }

    /// P2.2: 解析类型时展开类型别名
    pub(crate) fn resolve_type(&self, ty: &Type) -> Type {
        match ty {
            Type::Struct(name, args) if args.is_empty() => {
                if let Some(actual) = self.type_aliases.get(name) {
                    self.resolve_type(actual)
                } else {
                    ty.clone()
                }
            }
            Type::Option(inner) => Type::Option(Box::new(self.resolve_type(inner))),
            Type::Array(inner) => Type::Array(Box::new(self.resolve_type(inner))),
            _ => ty.clone(),
        }
    }
}
