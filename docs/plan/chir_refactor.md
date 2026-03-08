# CHIR 层重构实施计划

## 目标

引入 CHIR (Cangjie High-level IR) 中间表示层，彻底解决类型推断问题，将架构从 **AST → WASM** 升级为 **AST → CHIR → WASM**。

---

## 架构设计

### CHIR 数据结构

```rust
// src/chir/mod.rs

/// CHIR 表达式
pub struct CHIRExpr {
    pub kind: CHIRExprKind,
    pub ty: Type,           // 完整的 AST 类型（单态化后）
    pub wasm_ty: ValType,   // WASM 类型
    pub span: Option<Span>, // 源码位置（用于错误报告）
}

pub enum CHIRExprKind {
    // 字面量
    Integer(i64),
    Float(f64),
    Bool(bool),
    String(String),

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
    },
}

/// CHIR 语句
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
}

/// CHIR 左值
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
pub struct CHIRBlock {
    pub stmts: Vec<CHIRStmt>,
    pub result: Option<Box<CHIRExpr>>, // 块表达式的结果
}

/// CHIR 函数
pub struct CHIRFunction {
    pub name: String,
    pub params: Vec<(String, Type, ValType)>, // (名称, AST类型, WASM类型)
    pub return_ty: Type,
    pub return_wasm_ty: ValType,
    pub locals: Vec<(String, Type, ValType)>, // 局部变量
    pub body: CHIRBlock,
}

/// CHIR 程序
pub struct CHIRProgram {
    pub functions: Vec<CHIRFunction>,
    pub structs: Vec<StructDef>,  // 从 AST 复制
    pub classes: Vec<ClassDef>,
    pub enums: Vec<EnumDef>,
    pub globals: Vec<(String, Type, CHIRExpr)>,
}
```

---

## 实施阶段

### 阶段 1: 基础设施（3 天）

#### 1.1 创建 CHIR 模块（1 天）

**文件**: `src/chir/mod.rs`, `src/chir/types.rs`, `src/chir/builder.rs`

**任务**:
- 定义 CHIR 数据结构（如上）
- 实现 `CHIRBuilder` 辅助构建 CHIR
- 添加 `Display` trait 用于调试输出

**验收标准**:
```rust
// 可以手动构建简单的 CHIR 函数
let mut builder = CHIRBuilder::new();
let expr = builder.int_const(42);
let func = builder.function("test", vec![], Type::Int64, vec![
    CHIRStmt::Return(Some(expr))
]);
println!("{}", func); // 输出可读的 CHIR
```

#### 1.2 实现类型推断器（2 天）

**文件**: `src/chir/type_inference.rs`

**任务**:
- 实现 `TypeInferenceContext` 结构体
- 遍历 AST，收集类型信息：
  - 局部变量类型
  - 表达式类型
  - 函数签名
- 处理类型约束和泛型替换

**核心逻辑**:
```rust
pub struct TypeInferenceContext {
    // 局部变量类型表
    locals: HashMap<String, Type>,

    // 函数签名表（单态化后）
    functions: HashMap<String, FunctionSignature>,

    // 结构体/类字段类型
    struct_fields: HashMap<String, HashMap<String, Type>>,

    // 当前函数返回类型
    current_return_ty: Option<Type>,
}

impl TypeInferenceContext {
    pub fn infer_expr(&mut self, expr: &Expr) -> Result<Type, String> {
        match expr {
            Expr::Var(name) => {
                self.locals.get(name)
                    .cloned()
                    .ok_or_else(|| format!("变量未定义: {}", name))
            }
            Expr::Integer(_) => Ok(Type::Int64),
            Expr::Binary { op, left, right } => {
                let left_ty = self.infer_expr(left)?;
                let right_ty = self.infer_expr(right)?;
                self.infer_binary_result(op, &left_ty, &right_ty)
            }
            Expr::Call { name, args, .. } => {
                let sig = self.functions.get(name)
                    .ok_or_else(|| format!("函数未定义: {}", name))?;
                Ok(sig.return_ty.clone())
            }
            // ... 其他表达式
        }
    }
}
```

**验收标准**:
- 能够推断所有 AST 表达式的类型
- 单元测试覆盖率 > 80%

---

### 阶段 2: AST → CHIR 转换（5 天）

#### 2.1 表达式转换（2 天）

**文件**: `src/chir/lower_expr.rs`

**任务**:
- 实现 `lower_expr(ast_expr, ctx) -> CHIRExpr`
- 处理所有表达式类型：
  - 字面量 → CHIRExprKind::Integer/Float/Bool/String
  - 变量 → CHIRExprKind::Local（查询局部变量索引）
  - 二元运算 → CHIRExprKind::Binary
  - 函数调用 → CHIRExprKind::Call（解析函数索引）
  - 方法调用 → CHIRExprKind::MethodCall（解析 vtable 偏移）
  - 字段访问 → CHIRExprKind::FieldGet（计算偏移）
  - 数组索引 → CHIRExprKind::ArrayGet

**核心逻辑**:
```rust
pub struct LoweringContext<'a> {
    type_ctx: &'a TypeInferenceContext,
    local_map: HashMap<String, u32>,  // 变量名 → 局部变量索引
    func_indices: &'a HashMap<String, u32>,
    next_local: u32,
}

impl<'a> LoweringContext<'a> {
    pub fn lower_expr(&mut self, expr: &Expr) -> Result<CHIRExpr, String> {
        // 先推断类型
        let ty = self.type_ctx.infer_expr(expr)?;
        let wasm_ty = ty.to_wasm();

        let kind = match expr {
            Expr::Var(name) => {
                let local_idx = self.local_map.get(name)
                    .ok_or_else(|| format!("变量未定义: {}", name))?;
                CHIRExprKind::Local(*local_idx)
            }
            Expr::Integer(n) => CHIRExprKind::Integer(*n),
            Expr::Binary { op, left, right } => {
                let left_chir = self.lower_expr(left)?;
                let right_chir = self.lower_expr(right)?;

                // 插入类型转换（如果需要）
                let left_chir = self.insert_cast_if_needed(left_chir, wasm_ty);
                let right_chir = self.insert_cast_if_needed(right_chir, wasm_ty);

                CHIRExprKind::Binary {
                    op: *op,
                    left: Box::new(left_chir),
                    right: Box::new(right_chir),
                }
            }
            Expr::Call { name, args, .. } => {
                let func_idx = self.func_indices.get(name)
                    .ok_or_else(|| format!("函数未定义: {}", name))?;
                let args_chir = args.iter()
                    .map(|a| self.lower_expr(a))
                    .collect::<Result<Vec<_>, _>>()?;
                CHIRExprKind::Call {
                    func_idx: *func_idx,
                    args: args_chir,
                }
            }
            Expr::Field { object, field } => {
                let obj_chir = self.lower_expr(object)?;
                let obj_ty = self.type_ctx.infer_expr(object)?;
                let offset = self.get_field_offset(&obj_ty, field)?;
                CHIRExprKind::FieldGet {
                    object: Box::new(obj_chir),
                    field_offset: offset,
                }
            }
            // ... 其他表达式
        };

        Ok(CHIRExpr {
            kind,
            ty,
            wasm_ty,
            span: None,
        })
    }

    fn insert_cast_if_needed(&self, expr: CHIRExpr, target_ty: ValType) -> CHIRExpr {
        if expr.wasm_ty == target_ty {
            return expr;
        }

        CHIRExpr {
            kind: CHIRExprKind::Cast {
                expr: Box::new(expr),
                from_ty: expr.wasm_ty,
                to_ty: target_ty,
            },
            ty: expr.ty.clone(),
            wasm_ty: target_ty,
            span: None,
        }
    }
}
```

**验收标准**:
- 所有表达式类型都能转换
- 类型转换自动插入
- 单元测试覆盖率 > 80%

#### 2.2 语句转换（1 天）

**文件**: `src/chir/lower_stmt.rs`

**任务**:
- 实现 `lower_stmt(ast_stmt, ctx) -> CHIRStmt`
- 处理：
  - Let/Var → CHIRStmt::Let（分配局部变量索引）
  - Assign → CHIRStmt::Assign（转换左值）
  - Return → CHIRStmt::Return（插入类型转换）
  - If/While/Match → 转换为 CHIRExprKind::If/Match

**验收标准**:
- 所有语句类型都能转换
- 控制流正确处理

#### 2.3 函数转换（1 天）

**文件**: `src/chir/lower_func.rs`

**任务**:
- 实现 `lower_function(ast_func) -> CHIRFunction`
- 收集参数和局部变量
- 转换函数体
- 处理返回类型

**验收标准**:
- 能够转换完整的函数定义
- 局部变量索引正确分配

#### 2.4 程序转换（1 天）

**文件**: `src/chir/lower.rs`

**任务**:
- 实现 `lower_program(ast_program) -> CHIRProgram`
- 转换所有函数
- 复制结构体/类/枚举定义
- 处理全局变量

**验收标准**:
- 能够转换完整的程序
- 集成测试：编译简单的 .cj 文件到 CHIR

---

### 阶段 3: CHIR → WASM 生成（4 天）

#### 3.1 表达式生成（2 天）

**文件**: `src/codegen/chir_codegen.rs`

**任务**:
- 实现 `emit_expr(chir_expr, func) -> ()`
- 为每种 CHIRExprKind 生成 WASM 指令
- 类型转换直接映射到 WASM 指令

**核心逻辑**:
```rust
pub struct CHIRCodeGen {
    func_indices: HashMap<String, u32>,
    // ... 其他上下文
}

impl CHIRCodeGen {
    pub fn emit_expr(&self, expr: &CHIRExpr, func: &mut WasmFunc) {
        match &expr.kind {
            CHIRExprKind::Integer(n) => {
                match expr.wasm_ty {
                    ValType::I32 => func.instruction(&Instruction::I32Const(*n as i32)),
                    ValType::I64 => func.instruction(&Instruction::I64Const(*n)),
                    _ => panic!("整数类型不匹配"),
                }
            }
            CHIRExprKind::Local(idx) => {
                func.instruction(&Instruction::LocalGet(*idx));
            }
            CHIRExprKind::Binary { op, left, right } => {
                self.emit_expr(left, func);
                self.emit_expr(right, func);
                self.emit_binary_op(op, expr.wasm_ty, func);
            }
            CHIRExprKind::Call { func_idx, args } => {
                for arg in args {
                    self.emit_expr(arg, func);
                }
                func.instruction(&Instruction::Call(*func_idx));
            }
            CHIRExprKind::Cast { expr, from_ty, to_ty } => {
                self.emit_expr(expr, func);
                self.emit_cast(*from_ty, *to_ty, func);
            }
            CHIRExprKind::FieldGet { object, field_offset } => {
                self.emit_expr(object, func);
                func.instruction(&Instruction::I32Const(*field_offset as i32));
                func.instruction(&Instruction::I32Add);
                self.emit_load(expr.wasm_ty, func);
            }
            // ... 其他表达式
        }
    }

    fn emit_cast(&self, from: ValType, to: ValType, func: &mut WasmFunc) {
        match (from, to) {
            (ValType::I64, ValType::I32) => {
                func.instruction(&Instruction::I32WrapI64);
            }
            (ValType::I32, ValType::I64) => {
                func.instruction(&Instruction::I64ExtendI32S);
            }
            (ValType::I64, ValType::F64) => {
                func.instruction(&Instruction::F64ConvertI64S);
            }
            // ... 其他转换
            (a, b) if a == b => {}, // 无需转换
            _ => panic!("不支持的类型转换: {:?} -> {:?}", from, to),
        }
    }
}
```

**验收标准**:
- 所有 CHIR 表达式都能生成正确的 WASM
- 类型转换指令正确插入
- 无多余的类型转换

#### 3.2 语句和控制流生成（1 天）

**文件**: `src/codegen/chir_codegen.rs`

**任务**:
- 实现 `emit_stmt(chir_stmt, func)`
- 实现 `emit_block(chir_block, func)`
- 处理 If/Match 的块类型

**验收标准**:
- 控制流正确生成
- 块类型匹配

#### 3.3 函数生成（1 天）

**文件**: `src/codegen/chir_codegen.rs`

**任务**:
- 实现 `emit_function(chir_func) -> wasm_encoder::Function`
- 生成函数签名
- 生成局部变量声明
- 生成函数体

**验收标准**:
- 完整的函数能够生成
- WASM 验证通过

---

### 阶段 4: 集成和迁移（3 天）

#### 4.1 管道集成（1 天）

**文件**: `src/pipeline.rs`

**任务**:
- 修改编译管道：
  ```rust
  // 旧: AST → monomorphize → codegen → WASM
  // 新: AST → monomorphize → CHIR lowering → CHIR codegen → WASM
  ```
- 添加 `--emit-chir` 选项输出 CHIR（用于调试）

**验收标准**:
- 编译管道正常工作
- 可以输出 CHIR 用于调试

#### 4.2 渐进式迁移（1 天）

**策略**: 保留旧的 AST → WASM 路径，通过环境变量切换

```rust
// src/pipeline.rs
pub fn compile_to_wasm(program: &Program) -> Vec<u8> {
    if std::env::var("USE_CHIR").is_ok() {
        // 新路径: AST → CHIR → WASM
        let chir = chir::lower_program(program)?;
        chir::codegen::generate_wasm(&chir)
    } else {
        // 旧路径: AST → WASM
        codegen::generate_wasm_legacy(program)
    }
}
```

**任务**:
- 实现双路径支持
- 添加测试对比两种路径的输出

**验收标准**:
- 两种路径都能工作
- 输出 WASM 功能等价

#### 4.3 测试和验证（1 天）

**任务**:
- 运行所有单元测试
- 运行 `./scripts/system_test.sh`
- 对比 WASM 验证错误数量
- 修复发现的问题

**验收标准**:
- 所有测试通过
- WASM 验证错误显著减少（预期 4288 → <500）

---

### 阶段 5: 优化和清理（2 天）

#### 5.1 性能优化（1 天）

**任务**:
- 分析编译性能
- 优化类型推断算法
- 减少不必要的类型转换

**验收标准**:
- 编译时间增加 < 20%

#### 5.2 删除旧代码（1 天）

**任务**:
- 删除旧的 AST → WASM 路径
- 清理 `src/codegen/expr.rs` 中的类型推断代码
- 更新文档

**验收标准**:
- 代码库更简洁
- 文档更新完成

---

## 时间表

| 阶段 | 任务 | 工作量 | 开始日期 | 结束日期 |
|------|------|--------|---------|---------|
| 1 | 基础设施 | 3 天 | Day 1 | Day 3 |
| 2 | AST → CHIR | 5 天 | Day 4 | Day 8 |
| 3 | CHIR → WASM | 4 天 | Day 9 | Day 12 |
| 4 | 集成迁移 | 3 天 | Day 13 | Day 15 |
| 5 | 优化清理 | 2 天 | Day 16 | Day 17 |
| **总计** | | **17 天** | | |

考虑到测试、调试和意外问题，预留 **3-5 天缓冲**，总工期约 **3-4 周**。

---

## 风险和缓解

### 风险 1: 类型推断复杂度超预期

**缓解**:
- 先实现基础类型推断，复杂特性（如高阶函数）后续迭代
- 参考 CJC 的实现

### 风险 2: WASM 生成出现新问题

**缓解**:
- 保留旧路径作为对比
- 逐步迁移，每个阶段都验证

### 风险 3: 性能下降

**缓解**:
- 使用 `cargo flamegraph` 分析性能瓶颈
- 优化热点代码

---

## 验收标准

### 功能性

- ✅ 所有 37 个示例编译通过
- ✅ WASM 验证错误 < 500（从 4288 降低 > 88%）
- ✅ 生成的 WASM 功能正确

### 性能

- ✅ 编译时间增加 < 20%
- ✅ 生成的 WASM 大小增加 < 5%

### 代码质量

- ✅ 单元测试覆盖率 > 80%
- ✅ 代码通过 `cargo clippy` 检查
- ✅ 文档完整

---

## 参考资料

### CJC 源码

- `third_party/cangjie_compiler/src/CHIR/` - CHIR 定义
- `third_party/cangjie_compiler/src/Sema/` - 类型推断
- `third_party/cangjie_compiler/src/CodeGen/` - 代码生成

### 相关文档

- `docs/plan/type_inference_improvement.md` - 类型推断改进方案
- `docs/plan/std_fix.md` - 当前错误分析
- `docs/codegen_compare.md` - CJWasm vs CJC 对比

---

## 后续工作

CHIR 层建立后，可以进一步实现：

1. **优化 Pass**: 常量折叠、死代码消除、内联
2. **更好的错误报告**: 利用 CHIR 的类型信息提供精确的错误位置
3. **调试信息**: 生成 DWARF 调试信息
4. **增量编译**: 基于 CHIR 实现增量编译
5. **多后端**: 除了 WASM，还可以生成 LLVM IR、C 代码等
