# 类型推断系统改进方案

## 问题分析

### 当前架构缺陷

CJWasm2 采用 **AST → WASM** 直接编译，缺少中间类型推断层（CHIR），导致：

1. **TypeParam 类型信息丢失**: 单态化后，`Type::TypeParam("T")` 被替换为具体类型（如 `Type::Int32`），但在 codegen 阶段，局部变量和表达式的类型推断仍然依赖 AST 节点，无法获取单态化后的实际类型。

2. **类型推断时机错误**:
   - 单态化在 `monomorphize_program` 中完成，生成新的函数/结构体定义
   - Codegen 在 `compile_function` 中进行，此时 AST 中的 `TypeParam` 已被替换
   - 但 `infer_type_with_locals` 无法区分"原始 TypeParam"和"单态化后的具体类型"

3. **局部变量类型表不完整**: `LocalsBuilder` 只记录变量名到 WASM 类型的映射，不记录 AST 类型，导致：
   ```rust
   // 单态化前: func sort<T>(arr: Array<T>)
   // 单态化后: func sort$Int32(arr: Array<Int32>)
   // LocalsBuilder: arr → ValType::I32 (数组指针)
   // 但无法知道 arr 的元素类型是 Int32
   ```

### 具体表现

**错误 1: local.set expected [i64] but got [i32]** (376 个)
```rust
// 单态化前
func process<T>(item: T) { ... }

// 单态化后
func process$Int32(item: Int32) { ... }

// Codegen 时
// LocalsBuilder 记录: item → ValType::I64 (因为 TypeParam.to_wasm() = I64)
// 但实际传入的是 Int32 → ValType::I32
// 导致 local.set 类型不匹配
```

**错误 2: call expected [i32, i64] but got [i32, i32]** (544 个)
```rust
// 方法签名: Box<T>.get(): T
// 单态化后: Box$Int32.get(): Int32
// Codegen 推断返回类型时，看到 TypeParam → 返回 I64
// 但实际应该返回 I32
```

---

## 解决方案对比

### 方案 A: 添加 CHIR 中间层（彻底方案）

**架构**: AST → CHIR (类型推断) → WASM

**实现**:
1. 定义 CHIR 结构体，类似 CJC 的设计：
   ```rust
   struct CHIRExpr {
       kind: CHIRExprKind,
       ty: Type,  // 完整的 AST 类型（单态化后）
       wasm_ty: ValType,  // WASM 类型
   }
   ```

2. 在单态化后，遍历 AST 构建 CHIR，进行类型推断：
   - 变量声明: 记录完整类型
   - 表达式: 递归推断类型
   - 函数调用: 解析返回类型

3. Codegen 从 CHIR 生成 WASM，类型信息完整

**优点**:
- 彻底解决类型推断问题
- 类型信息完整，便于优化
- 架构清晰，易于维护

**缺点**:
- 工作量大（~2000 行代码）
- 需要重构整个 codegen 流程
- 开发周期长（2-3 周）

---

### 方案 B: 增强 LocalsBuilder（渐进方案）

**架构**: 保持 AST → WASM，增强局部变量类型表

**实现**:
1. 扩展 `LocalsBuilder` 记录 AST 类型：
   ```rust
   pub struct LocalsBuilder {
       locals: HashMap<String, u32>,
       types: HashMap<String, ValType>,
       ast_types: HashMap<String, Type>,  // 新增：AST 类型
   }
   ```

2. 在 `compile_function` 开头，遍历参数和局部变量，记录单态化后的 AST 类型：
   ```rust
   for param in &func.params {
       let ast_ty = param.ty.clone();  // 单态化后的类型
       locals.set_ast_type(&param.name, ast_ty);
   }
   ```

3. 修改 `infer_type_with_locals` 优先使用 AST 类型：
   ```rust
   fn infer_type_with_locals(&self, expr: &Expr, locals: &LocalsBuilder) -> ValType {
       if let Expr::Var(name) = expr {
           if let Some(ast_ty) = locals.get_ast_type(name) {
               return ast_ty.to_wasm();  // 使用单态化后的类型
           }
       }
       // 原有逻辑...
   }
   ```

4. 在关键点添加类型协调：
   - `Stmt::Let/Var`: 使用 AST 类型推断
   - `Stmt::Assign`: 检查目标类型并协调
   - 方法调用: 查询单态化后的方法签名

**优点**:
- 工作量适中（~500 行代码）
- 不破坏现有架构
- 可以渐进式实施
- 开发周期短（3-5 天）

**缺点**:
- 类型信息仍不完整（只有局部变量）
- 表达式类型推断仍依赖启发式
- 无法完全消除所有类型错误

---

### 方案 C: 局部修补（临时方案）

**架构**: 保持现状，在错误高发点添加特殊处理

**实现**:
1. 针对 376 个 `local.set` 错误：
   - 在 `Stmt::Let/Var/Assign` 中，检测 TypeParam 局部变量
   - 如果值类型是 I32 但局部变量是 I64，插入 I64ExtendI32S
   - 如果值类型是 I64 但局部变量是 I32，插入 I32WrapI64

2. 针对 544 个 `call` 错误：
   - 在方法调用参数推入前，查询方法签名
   - 如果参数类型不匹配，插入类型转换指令

3. 针对 257 个 `i64.store` 错误：
   - 在存储指令前，检查值类型
   - 如果不匹配，插入类型转换

**优点**:
- 工作量最小（~200 行代码）
- 快速见效（1-2 天）
- 不改变架构

**缺点**:
- 治标不治本
- 代码质量下降（充满特殊处理）
- 难以维护
- 可能引入新的错误

---

## 推荐方案

### 短期（1-2 周）: 方案 B（增强 LocalsBuilder）

**理由**:
1. 平衡了工作量和效果
2. 可以解决大部分类型错误（预计消除 60-70%）
3. 不破坏现有架构，风险可控
4. 为长期方案打基础

**实施步骤**:

#### 第 1 步: 扩展 LocalsBuilder（1 天）
```rust
// src/codegen/mod.rs
impl LocalsBuilder {
    pub fn set_ast_type(&mut self, name: &str, ty: Type) {
        self.ast_types.insert(name.to_string(), ty);
    }

    pub fn get_ast_type(&self, name: &str) -> Option<&Type> {
        self.ast_types.get(name)
    }
}
```

#### 第 2 步: 在 compile_function 中记录类型（1 天）
```rust
// src/codegen/expr.rs
fn compile_function(&mut self, func: &Function) {
    let mut locals = LocalsBuilder::new();

    // 记录参数类型（单态化后）
    for param in &func.params {
        locals.add(&param.name, param.ty.to_wasm());
        locals.set_ast_type(&param.name, param.ty.clone());
    }

    // 遍历函数体，记录局部变量类型
    self.collect_local_types(&func.body, &mut locals);

    // 原有编译逻辑...
}

fn collect_local_types(&self, stmts: &[Stmt], locals: &mut LocalsBuilder) {
    for stmt in stmts {
        match stmt {
            Stmt::Let { pattern, ty, value } | Stmt::Var { pattern, ty, value } => {
                if let Pattern::Binding(name) = pattern {
                    if let Some(ty) = ty {
                        locals.set_ast_type(name, ty.clone());
                    } else {
                        // 从 value 推断类型
                        if let Some(inferred) = self.infer_ast_type(value) {
                            locals.set_ast_type(name, inferred);
                        }
                    }
                }
            }
            // 递归处理嵌套语句...
            _ => {}
        }
    }
}
```

#### 第 3 步: 改进 infer_type_with_locals（1 天）
```rust
fn infer_type_with_locals(&self, expr: &Expr, locals: &LocalsBuilder) -> ValType {
    // 优先使用 AST 类型
    if let Expr::Var(name) = expr {
        if let Some(ast_ty) = locals.get_ast_type(name) {
            return ast_ty.to_wasm();
        }
    }

    // 原有逻辑...
    if let Some(ast_ty) = self.infer_ast_type_with_locals(expr, locals) {
        return ast_ty.to_wasm();
    }

    // Fallback
    self.infer_type(expr)
}
```

#### 第 4 步: 添加类型协调（2 天）
```rust
// Stmt::Let/Var
Stmt::Let { pattern, value, .. } => {
    self.compile_expr(value, locals, func, loop_ctx);
    if let Pattern::Binding(name) = pattern {
        let val_ty = self.infer_type_with_locals(value, locals);
        let local_ty = locals.get_ast_type(name)
            .map(|t| t.to_wasm())
            .unwrap_or(val_ty);
        self.emit_type_coercion(func, val_ty, local_ty);
        // ...
    }
}

// 方法调用参数
fn compile_method_call(...) {
    // 查询方法签名
    let method_sig = self.lookup_method_signature(object_ty, method);

    // 推入参数并协调类型
    for (arg, param_ty) in args.iter().zip(method_sig.params.iter()) {
        self.compile_expr(arg, locals, func, loop_ctx);
        let arg_ty = self.infer_type_with_locals(arg, locals);
        let expected_ty = param_ty.to_wasm();
        self.emit_type_coercion(func, arg_ty, expected_ty);
    }
}
```

#### 第 5 步: 测试和验证（1 天）
- 运行 `cargo test`
- 运行 `./scripts/system_test.sh`
- 检查 WASM 验证错误数量变化
- 预期: 4288 → ~1500 (-65%)

---

### 长期（1-2 月）: 方案 A（CHIR 层）

在方案 B 稳定后，逐步迁移到 CHIR 架构：

1. 定义 CHIR 数据结构
2. 实现 AST → CHIR 转换
3. 实现 CHIR → WASM 生成
4. 逐步迁移现有 codegen 逻辑
5. 删除旧的 AST → WASM 路径

---

## 预期效果

### 方案 B 实施后

| 错误类型 | 当前 | 预期 | 改善 |
|---------|------|------|------|
| `local.set expected [i64] but got [i32]` | 376 | ~50 | -87% |
| `call expected [i32, i64] but got [i32, i32]` | 544 | ~150 | -72% |
| `i64.store expected [i32, i64] but got [i32, i32]` | 257 | ~80 | -69% |
| `i32.wrap_i64 expected [i64] but got [... i32]` | 650 | ~400 | -38% |
| **总计** | **4288** | **~1500** | **-65%** |

### 方案 A 实施后

预期消除 95%+ 的类型错误，达到接近 CJC 的类型安全水平。

---

## 参考资料

- CJC 源码: `third_party/cangjie_compiler/src/CodeGen/`
- CHIR 定义: `third_party/cangjie_compiler/src/CHIR/`
- 类型推断: `third_party/cangjie_compiler/src/Sema/`
