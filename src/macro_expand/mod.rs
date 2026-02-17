//! 宏展开引擎：编译 macro func 为 WASM，编译期执行宏调用并替换 AST。
//!
//! 架构：
//! 1. 从 Program 中提取 MacroDef，编译为独立 WASM 模块
//! 2. 遍历 AST 找到 Stmt::MacroExpand / Expr::MacroCall
//! 3. 序列化参数为 JSON，通过 wasmtime 执行宏 WASM
//! 4. 反序列化结果为 AST 节点，替换宏调用处
//!
//! 当 `macro-system` feature 未启用时，使用纯 AST 层面的内建宏展开。

pub mod builtin;

#[cfg(feature = "macro-system")]
pub mod runtime;

use crate::ast::{Expr, MacroDef, Program, Stmt};
use std::collections::HashMap;

/// 宏展开器
pub struct MacroExpander {
    /// 已注册的宏定义（名称 → 宏定义）
    macro_defs: HashMap<String, MacroDef>,
    /// 编译后的 WASM 字节码（名称 → bytes）— 仅 macro-system feature
    #[cfg(feature = "macro-system")]
    wasm_modules: HashMap<String, Vec<u8>>,
}

/// 宏展开错误
#[derive(Debug)]
pub enum MacroError {
    /// 未找到宏定义
    UndefinedMacro(String),
    /// 宏编译失败
    CompileError(String),
    /// 宏执行失败
    RuntimeError(String),
    /// JSON 序列化/反序列化失败
    SerdeError(String),
}

impl std::fmt::Display for MacroError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MacroError::UndefinedMacro(name) => write!(f, "未定义的宏: @{}", name),
            MacroError::CompileError(msg) => write!(f, "宏编译错误: {}", msg),
            MacroError::RuntimeError(msg) => write!(f, "宏执行错误: {}", msg),
            MacroError::SerdeError(msg) => write!(f, "宏序列化错误: {}", msg),
        }
    }
}

impl MacroExpander {
    /// 从 Program 中提取宏定义并编译
    pub fn new(program: &Program) -> Self {
        let mut macro_defs = HashMap::new();
        for m in &program.macros {
            macro_defs.insert(m.name.clone(), m.clone());
        }

        #[cfg(feature = "macro-system")]
        let wasm_modules = {
            let mut modules = HashMap::new();
            for m in &program.macros {
                match runtime::compile_macro_to_wasm(m) {
                    Ok(bytes) => { modules.insert(m.name.clone(), bytes); }
                    Err(e) => {
                        eprintln!("警告: 宏 '{}' 编译失败: {}", m.name, e);
                    }
                }
            }
            modules
        };

        MacroExpander {
            macro_defs,
            #[cfg(feature = "macro-system")]
            wasm_modules,
        }
    }

    /// 展开 Program 中所有宏调用
    pub fn expand_program(&self, program: &mut Program) -> Result<(), Vec<MacroError>> {
        let mut errors = Vec::new();

        for func in &mut program.functions {
            if let Err(mut errs) = self.expand_stmts(&mut func.body) {
                errors.append(&mut errs);
            }
        }
        for class in &mut program.classes {
            if let Some(ref mut init) = class.init {
                if let Err(mut errs) = self.expand_stmts(&mut init.body) {
                    errors.append(&mut errs);
                }
            }
            for method in &mut class.methods {
                if let Err(mut errs) = self.expand_stmts(&mut method.func.body) {
                    errors.append(&mut errs);
                }
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// 展开一组语句中的宏调用
    fn expand_stmts(&self, stmts: &mut Vec<Stmt>) -> Result<(), Vec<MacroError>> {
        let mut errors = Vec::new();
        let mut i = 0;
        while i < stmts.len() {
            match &stmts[i] {
                Stmt::MacroExpand { name, args } => {
                    let macro_name = name.clone();
                    let macro_args = args.clone();
                    match self.invoke_macro(&macro_name, &macro_args) {
                        Ok(expanded_stmts) => {
                            stmts.remove(i);
                            for (j, s) in expanded_stmts.into_iter().enumerate() {
                                stmts.insert(i + j, s);
                            }
                            // 不增加 i，因为展开的语句可能还包含宏调用
                            continue;
                        }
                        Err(e) => {
                            errors.push(e);
                            i += 1;
                        }
                    }
                }
                _ => {
                    // 递归展开嵌套在语句内的宏
                    self.expand_stmt_inner(&mut stmts[i], &mut errors);
                    i += 1;
                }
            }
        }
        if errors.is_empty() { Ok(()) } else { Err(errors) }
    }

    /// 递归展开语句内部的宏调用
    fn expand_stmt_inner(&self, stmt: &mut Stmt, errors: &mut Vec<MacroError>) {
        match stmt {
            Stmt::While { body, .. } | Stmt::For { body, .. } | Stmt::Loop { body }
            | Stmt::DoWhile { body, .. } => {
                if let Err(mut errs) = self.expand_stmts(body) {
                    errors.append(&mut errs);
                }
            }
            Stmt::WhileLet { body, .. } => {
                if let Err(mut errs) = self.expand_stmts(body) {
                    errors.append(&mut errs);
                }
            }
            _ => {}
        }
    }

    /// 执行单个宏
    fn invoke_macro(&self, name: &str, args: &[Expr]) -> Result<Vec<Stmt>, MacroError> {
        // 首先尝试内建宏
        if let Some(expanded) = builtin::try_expand_builtin(name, args) {
            return Ok(expanded);
        }

        // 检查是否有用户定义的宏
        let macro_def = match self.macro_defs.get(name) {
            Some(def) => def,
            None => return Err(MacroError::UndefinedMacro(name.to_string())),
        };

        // 使用 wasmtime 执行（仅 macro-system feature）
        #[cfg(feature = "macro-system")]
        {
            if let Some(wasm_bytes) = self.wasm_modules.get(name) {
                let raw_stmts = runtime::execute_macro(name, wasm_bytes, args)?;
                // wasmtime 返回的是原始 quote body，需要进行参数替换
                return Ok(self.substitute_quote_body(&raw_stmts, &macro_def.params, args));
            }
        }

        // 回退：基于 AST 的简单宏展开（解释宏体中的 quote）
        self.interpret_macro(name, args)
    }

    /// 简易宏解释器：直接执行宏体，提取 quote 内容
    fn interpret_macro(&self, name: &str, args: &[Expr]) -> Result<Vec<Stmt>, MacroError> {
        let macro_def = self.macro_defs.get(name)
            .ok_or_else(|| MacroError::UndefinedMacro(name.to_string()))?;

        // 简单策略：遍历宏体，找到 return quote(...) 语句
        // 将 quote 内的语句作为展开结果
        for stmt in &macro_def.body {
            if let Stmt::Return(Some(expr)) = stmt {
                if let Expr::Quote { body, .. } = expr {
                    // 在展开结果中进行参数替换
                    let expanded = self.substitute_quote_body(body, &macro_def.params, args);
                    return Ok(expanded);
                }
            }
        }

        // 如果宏体中没有 return quote(...)，返回空展开
        Ok(vec![])
    }

    /// 在 quote body 中替换宏参数引用
    fn substitute_quote_body(
        &self,
        body: &[Stmt],
        params: &[crate::ast::Param],
        args: &[Expr],
    ) -> Vec<Stmt> {
        // 建立参数名到参数值的映射
        let mut param_map: HashMap<String, &Expr> = HashMap::new();
        for (i, param) in params.iter().enumerate() {
            if let Some(arg) = args.get(i) {
                param_map.insert(param.name.clone(), arg);
            }
        }
        // 深拷贝 body 并替换 Expr::Var 引用
        body.iter().map(|s| self.substitute_stmt(s, &param_map)).collect()
    }

    fn substitute_stmt(&self, stmt: &Stmt, params: &HashMap<String, &Expr>) -> Stmt {
        match stmt {
            Stmt::Expr(e) => Stmt::Expr(self.substitute_expr(e, params)),
            Stmt::Let { pattern, ty, value } => Stmt::Let {
                pattern: pattern.clone(),
                ty: ty.clone(),
                value: self.substitute_expr(value, params),
            },
            Stmt::Var { name, ty, value } => Stmt::Var {
                name: name.clone(),
                ty: ty.clone(),
                value: self.substitute_expr(value, params),
            },
            Stmt::Return(opt) => Stmt::Return(opt.as_ref().map(|e| self.substitute_expr(e, params))),
            other => other.clone(),
        }
    }

    fn substitute_expr(&self, expr: &Expr, params: &HashMap<String, &Expr>) -> Expr {
        match expr {
            Expr::Var(name) => {
                if let Some(replacement) = params.get(name.as_str()) {
                    (*replacement).clone()
                } else {
                    expr.clone()
                }
            }
            Expr::Call { name, type_args, args, named_args } => Expr::Call {
                name: name.clone(),
                type_args: type_args.clone(),
                args: args.iter().map(|a| self.substitute_expr(a, params)).collect(),
                named_args: named_args.clone(),
            },
            Expr::Binary { op, left, right } => Expr::Binary {
                op: op.clone(),
                left: Box::new(self.substitute_expr(left, params)),
                right: Box::new(self.substitute_expr(right, params)),
            },
            Expr::If { cond, then_branch, else_branch } => Expr::If {
                cond: Box::new(self.substitute_expr(cond, params)),
                then_branch: Box::new(self.substitute_expr(then_branch, params)),
                else_branch: else_branch.as_ref().map(|e| Box::new(self.substitute_expr(e, params))),
            },
            Expr::Block(stmts, tail) => Expr::Block(
                stmts.iter().map(|s| self.substitute_stmt(s, params)).collect(),
                tail.as_ref().map(|e| Box::new(self.substitute_expr(e, params))),
            ),
            _ => expr.clone(),
        }
    }
}

/// 判断 Program 是否包含宏（调用或定义）
pub fn program_has_macros(program: &Program) -> bool {
    if !program.macros.is_empty() {
        return true;
    }
    for func in &program.functions {
        if stmts_have_macro_calls(&func.body) {
            return true;
        }
    }
    false
}

fn stmts_have_macro_calls(stmts: &[Stmt]) -> bool {
    stmts.iter().any(|s| match s {
        Stmt::MacroExpand { .. } => true,
        Stmt::While { body, .. } | Stmt::For { body, .. } | Stmt::Loop { body }
        | Stmt::DoWhile { body, .. } => stmts_have_macro_calls(body),
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::*;

    #[test]
    fn test_expander_no_macros() {
        let mut program = Program {
            package_name: None,
            imports: vec![],
            structs: vec![],
            interfaces: vec![],
            classes: vec![],
            enums: vec![],
            functions: vec![Function {
                visibility: Visibility::default(),
                name: "main".to_string(),
                type_params: vec![],
                constraints: vec![],
                params: vec![],
                return_type: Some(Type::Int64),
                throws: None,
                body: vec![Stmt::Return(Some(Expr::Integer(0)))],
                extern_import: None,
            }],
            extends: vec![],
            type_aliases: vec![],
            macros: vec![],
        };
        let expander = MacroExpander::new(&program);
        assert!(expander.expand_program(&mut program).is_ok());
    }

    #[test]
    fn test_expander_simple_quote_macro() {
        let mut program = Program {
            package_name: None,
            imports: vec![],
            structs: vec![],
            interfaces: vec![],
            classes: vec![],
            enums: vec![],
            functions: vec![Function {
                visibility: Visibility::default(),
                name: "main".to_string(),
                type_params: vec![],
                constraints: vec![],
                params: vec![],
                return_type: Some(Type::Int64),
                throws: None,
                body: vec![
                    Stmt::MacroExpand {
                        name: "MyLog".to_string(),
                        args: vec![Expr::String("hello".to_string())],
                    },
                    Stmt::Return(Some(Expr::Integer(0))),
                ],
                extern_import: None,
            }],
            extends: vec![],
            type_aliases: vec![],
            macros: vec![MacroDef {
                visibility: Visibility::Public,
                name: "MyLog".to_string(),
                params: vec![Param {
                    name: "msg".to_string(),
                    ty: Type::String,
                    default: None,
                    variadic: false,
                    is_named: false,
                    is_inout: false,
                }],
                body: vec![Stmt::Return(Some(Expr::Quote {
                    body: vec![Stmt::Expr(Expr::Call {
                        name: "println".to_string(),
                        type_args: None,
                        args: vec![Expr::Var("msg".to_string())],
                        named_args: vec![],
                    })],
                    splices: vec![],
                }))],
            }],
        };

        let expander = MacroExpander::new(&program);
        let result = expander.expand_program(&mut program);
        assert!(result.is_ok());
        // 宏调用应该被展开为 println("hello")
        let body = &program.functions[0].body;
        assert_eq!(body.len(), 2); // expanded stmt + return
        match &body[0] {
            Stmt::Expr(Expr::Call { name, args, .. }) => {
                assert_eq!(name, "println");
                assert_eq!(args.len(), 1);
                match &args[0] {
                    Expr::String(s) => assert_eq!(s, "hello"),
                    _ => panic!("Expected string arg, got {:?}", args[0]),
                }
            }
            other => panic!("Expected Call, got {:?}", other),
        }
    }

    #[test]
    fn test_expander_undefined_macro() {
        let mut program = Program {
            package_name: None,
            imports: vec![],
            structs: vec![],
            interfaces: vec![],
            classes: vec![],
            enums: vec![],
            functions: vec![Function {
                visibility: Visibility::default(),
                name: "main".to_string(),
                type_params: vec![],
                constraints: vec![],
                params: vec![],
                return_type: Some(Type::Int64),
                throws: None,
                body: vec![
                    Stmt::MacroExpand {
                        name: "NonExistent".to_string(),
                        args: vec![],
                    },
                ],
                extern_import: None,
            }],
            extends: vec![],
            type_aliases: vec![],
            macros: vec![],
        };

        let expander = MacroExpander::new(&program);
        let result = expander.expand_program(&mut program);
        assert!(result.is_err());
    }

    #[test]
    fn test_program_has_macros_false() {
        let program = Program {
            package_name: None,
            imports: vec![],
            structs: vec![],
            interfaces: vec![],
            classes: vec![],
            enums: vec![],
            functions: vec![],
            extends: vec![],
            type_aliases: vec![],
            macros: vec![],
        };
        assert!(!program_has_macros(&program));
    }

    #[test]
    fn test_program_has_macros_with_def() {
        let program = Program {
            package_name: None,
            imports: vec![],
            structs: vec![],
            interfaces: vec![],
            classes: vec![],
            enums: vec![],
            functions: vec![],
            extends: vec![],
            type_aliases: vec![],
            macros: vec![MacroDef {
                visibility: Visibility::Public,
                name: "Test".to_string(),
                params: vec![],
                body: vec![],
            }],
        };
        assert!(program_has_macros(&program));
    }

    #[test]
    fn test_macro_serialization_roundtrip() {
        let args = vec![Expr::Integer(42), Expr::String("hello".to_string())];
        let json = serde_json::to_string(&args).unwrap();
        let restored: Vec<Expr> = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.len(), 2);
    }
}
