# 标准库模块修复计划

## 当前状态总结

经过测试，9个标准库模块的状态如下：

### ✅ 完全成功的模块 (6个)
- **math**: 已修复所有7个解析错误，编译成功
- **random**: 编译成功
- **time**: 编译成功
- **core**: 编译成功
- **fs**: 编译成功
- **collection**: 单独编译成功，但与其他模块一起编译时有 where 子句冲突

### ⚠️ 解析成功但代码生成失败的模块 (3个)
- **crypto**: 解析成功，codegen 错误
- **io**: 解析成功，codegen 错误
- **unicode**: 解析成功，codegen 错误

### 🔧 需要解析器增强的问题 (1个)
- **where 子句**: collection 模块在类定义中使用 where 约束

---

## 详细修复计划

### 1. Where 子句支持 (高优先级)

**问题描述:**
```cangjie
public class TreeSet<T> <: OrderedSet<T> where T <: Comparable<T> {
```

**错误信息:**
```
语法错误: 意外的 token: Where, 期望: RParen (字节偏移 3255-3260)
```

**影响范围:**
- `collection/tree_set.cj` 第16行
- `collection/tree_set.cj` 第375行

**修复方案:**
1. 在 `src/parser/decl.rs` 的 `parse_class_with_visibility()` 中添加 where 子句解析
2. 在类定义的 `<: Interface` 之后检查 `Token::Where`
3. 解析 where 约束：`where T <: Constraint`
4. 将约束存储到 AST 的 `ClassDef` 结构中
5. 在 codegen 阶段验证约束（可选，初期可忽略）

**实现位置:**
- `src/parser/decl.rs`: `parse_class_with_visibility()` 函数
- `src/ast/mod.rs`: 扩展 `ClassDef` 结构添加 `where_clauses` 字段

**预计难度:** 中等

---

### 2. Crypto 模块 - 接口属性访问 (中优先级)

**问题描述:**
在接口的默认方法实现中访问接口属性 `blockSize`

**错误信息:**
```
thread 'main' panicked at src/codegen/expr.rs:3306:29:
变量未找到: 'blockSize'
```

**代码位置:**
```cangjie
// crypto/cipher/cipher.cj:16-17
public interface BlockCipher {
    prop blockSize: Int64

    func encrypt(input: Array<Byte>): Array<Byte> {
        let buf = Array<Byte>(blockSize, repeat: 0)  // 这里访问 blockSize
        this.encrypt(input, to: buf)
        return buf
    }
}
```

**修复方案:**
1. 在接口方法的 codegen 中，识别接口属性访问
2. 将 `blockSize` 识别为 `this.blockSize` 的简写
3. 在 `compile_expr` 的 `Expr::Var` 分支中：
   - 检查当前是否在接口方法中
   - 检查变量名是否是接口的属性
   - 如果是，转换为 `this.field` 访问

**实现位置:**
- `src/codegen/expr.rs`: `compile_expr()` 中的 `Expr::Var` 分支（约3306行）
- 需要在 `CodeGen` 结构中跟踪当前接口上下文

**预计难度:** 中等

---

### 3. IO 模块 - 接口方法查找 (中优先级)

**问题描述:**
接口方法在 codegen 时无法找到

**错误信息:**
```
thread 'main' panicked at src/codegen/decl.rs:90:9:
方法未找到: 'InputStream.read'
```

**修复方案:**
1. 在 `src/codegen/decl.rs` 中增强接口方法的注册逻辑
2. 确保接口方法被正确添加到方法索引表
3. 在方法调用时，如果对象类型是接口，使用接口方法查找

**实现位置:**
- `src/codegen/decl.rs`: 接口方法注册逻辑（约90行）
- `src/codegen/mod.rs`: 方法索引表管理

**预计难度:** 中等

---

### 4. Unicode 模块 - Rune 构造函数 (中优先级)

**问题描述:**
`Rune(UInt32)` 构造函数调用无法找到

**错误信息:**
```
thread 'main' panicked at src/codegen/expr.rs:4048:25:
函数未找到: 'Rune' (key: 'Rune')
```

**代码位置:**
```cangjie
// unicode 模块中多处使用
Rune(UInt32(start + ((codePosition - start) & !1)))
```

**修复方案:**
1. `Rune` 是内置类型，需要支持类型转换构造函数
2. 在 `compile_expr` 的 `Expr::Call` 分支中：
   - 检查函数名是否是内置类型（Rune, Int64, UInt32 等）
   - 如果是，生成类型转换代码而不是函数调用
3. 对于 `Rune(UInt32(x))`，实际上是 `UInt32 -> Rune` 的转换

**实现位置:**
- `src/codegen/expr.rs`: `compile_expr()` 中的 `Expr::Call` 分支（约4048行）
- 可能需要在 `src/codegen/mod.rs` 中添加类型转换辅助函数

**预计难度:** 中等

---

## 实施顺序建议

### 阶段 1: Where 子句支持（立即）
优先级最高，因为它阻止了 collection 模块与其他模块一起编译。

**步骤:**
1. 扩展 AST 定义添加 where 子句
2. 在 class 和 interface 解析中添加 where 子句解析
3. 测试 collection 模块

**预计时间:** 2-3小时

### 阶段 2: 接口属性访问（crypto）
修复接口默认方法中的属性访问问题。

**步骤:**
1. 在 codegen 中跟踪接口上下文
2. 修改 Expr::Var 处理逻辑
3. 测试 crypto 模块

**预计时间:** 2-3小时

### 阶段 3: 接口方法查找（io）
完善接口方法的注册和查找机制。

**步骤:**
1. 修复接口方法注册
2. 增强方法查找逻辑
3. 测试 io 模块

**预计时间:** 2-3小时

### 阶段 4: 类型转换构造函数（unicode）
支持内置类型的构造函数语法。

**步骤:**
1. 识别类型转换调用
2. 生成适当的转换代码
3. 测试 unicode 模块

**预计时间:** 2-3小时

---

## 测试策略

### 单元测试
每个修复后，单独测试该模块：
```bash
# 修改 src/pipeline.rs 中的 L1_STD_TOP
const L1_STD_TOP: &[&str] = &["module_name"];
cargo run -- build -p examples/std
```

### 集成测试
所有修复完成后，测试所有模块一起编译：
```bash
const L1_STD_TOP: &[&str] = &[
    "core", "collection", "crypto", "fs", "io",
    "math", "random", "time", "unicode"
];
cargo run -- build -p examples/std
```

### 回归测试
确保之前修复的 math 模块功能仍然正常：
```bash
cargo test
```

---

## 成功标准

1. ✅ 所有 9 个标准库模块能够单独编译成功
2. ✅ 所有 9 个模块能够一起编译成功
3. ✅ 生成的 WASM 文件大小合理（预计 50KB-200KB）
4. ✅ 没有 panic 或 codegen 错误
5. ✅ 编译警告数量可接受（主要是未使用变量）

---

## 风险和注意事项

1. **Where 子句语义**: 初期实现可以只解析不验证，避免复杂的类型约束检查
2. **接口方法调用**: 需要考虑虚函数表和动态分发，可能需要较大的架构改动
3. **类型转换**: 需要明确 Rune/UInt32 等类型的内存表示和转换规则
4. **向后兼容**: 确保新增功能不破坏现有的测试用例

---

## 已完成的 Math 模块修复回顾

作为参考，math 模块修复了以下问题：
1. ✅ @Intrinsic 函数 EOF 检测
2. ✅ 接口中的 operator 修饰符
3. ✅ 运算符方法名支持
4. ✅ 接口泛型参数
5. ✅ Match guard 表达式
6. ✅ Guard 二元运算符
7. ✅ Match subject 二元运算符
8. ✅ 枚举多接口继承

这些修复为后续工作提供了良好的基础。
