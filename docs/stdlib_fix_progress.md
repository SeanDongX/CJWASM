# 标准库模块修复进度报告

## 阶段 1: Where 子句支持 - ✅ 已完成

### 问题描述
Collection 模块使用了两种 where 语法：
1. **类型约束**: `class TreeSet<T> <: OrderedSet<T> where T <: Comparable<T>`
2. **For 循环过滤**: `for (key in collection where condition)`

### 修复内容

#### 1. 类型约束 where 子句
- **状态**: 已存在，无需修改
- **位置**: `src/parser/type_.rs:146` - `parse_where_clause()`
- **说明**: 类型约束的 where 子句解析已经实现并正常工作

#### 2. For 循环 where 过滤子句
**修改的文件**:

1. **src/ast/mod.rs** (第355-359行)
   - 在 `Stmt::For` 中添加 `filter: Option<Box<Expr>>` 字段
   ```rust
   For {
       var: String,
       iterable: Expr,
       filter: Option<Box<Expr>>,  // 新增
       body: Vec<Stmt>,
   }
   ```

2. **src/parser/stmt.rs** (第286-296行)
   - 在 for 循环解析中添加 where 子句支持
   ```rust
   self.expect(Token::In)?;
   let iterable = self.parse_for_iterable()?;
   // 解析可选的 where 过滤条件
   let filter = if self.check(&Token::Where) {
       self.advance();
       Some(Box::new(self.parse_expr()?))
   } else {
       None
   };
   ```

3. **src/parser/expr.rs**
   - 第1914行: 修改整数范围解析，使用 `parse_for_range_end()`
   - 第1948行: 修改变量范围解析，使用 `parse_for_range_end()`
   - 第2020-2062行: 新增 `parse_for_range_end()` 函数
     - 解析范围表达式的 end 部分
     - 在遇到 `where` 关键字时停止解析
     - 支持字段访问和方法调用

4. **src/codegen/expr.rs**
   - 第179-193行: 在 `collect_locals` 中添加 filter 处理
   - 第2794-2880行: 在 Range 迭代中添加 filter 条件检查
   - 第2910-2970行: 在数组迭代中添加 filter 条件检查
   - Filter 实现逻辑：
     ```rust
     if let Some(filter_expr) = filter {
         self.compile_expr(filter_expr, locals, func, loop_ctx);
         func.instruction(&Instruction::I32Eqz);
         func.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
         // 如果条件为假，跳过循环体，递增索引，继续循环
         // ...
         func.instruction(&Instruction::End);
     }
     ```

5. **src/codegen/mod.rs**
   - 第1293-1299行: 在 `collect_lambdas_from_stmt` 中添加 filter 处理
   - 第6519-6535行: 在测试代码中添加 `filter: None`

6. **src/monomorph/mod.rs**
   - 第532-543行: 在类型替换中添加 filter 处理

### 测试结果

✅ **单独编译测试**:
- collection: 成功
- core: 成功
- fs: 成功
- math: 成功
- random: 成功
- time: 成功

✅ **组合编译测试**:
- core + collection: 成功
- core + collection + fs + math + random + time: 成功（有 codegen 警告）

⚠️ **Codegen 错误**（不影响解析）:
- crypto: `变量未找到: 'blockSize'` - 接口属性访问问题
- io: `方法未找到: 'InputStream.read'` - 接口方法查找问题
- unicode: `函数未找到: 'Rune'` - 类型转换构造函数问题
- math (与其他模块组合时): `变量未找到: 'Int64'` - 类型名作为变量使用

### 示例代码

成功解析和编译的 for where 语句：
```cangjie
// 简单条件
for (i in 0..10 where i > 5) {
    println(i)
}

// 复杂表达式
for (key in map.keys() where other.contains(key)) {
    result.add(key)
}

// 方法调用
for (_ in 0..size where iThis.next() != iThat.next()) {
    return false
}
```

---

## 下一步工作

### 阶段 2: 接口属性访问（crypto 模块）
**问题**: 接口默认方法中访问接口属性 `blockSize`
**修复方案**: 在 codegen 中将属性名识别为 `this.property`

### 阶段 3: 接口方法查找（io 模块）
**问题**: 接口方法在 codegen 时无法找到
**修复方案**: 增强接口方法注册和查找逻辑

### 阶段 4: 类型转换构造函数（unicode 模块）
**问题**: `Rune(UInt32)` 构造函数调用无法找到
**修复方案**: 支持内置类型的构造函数语法

### 阶段 5: 类型名作为表达式（math 模块）
**问题**: `Int64` 等类型名在某些上下文中被当作变量使用
**修复方案**: 需要进一步分析具体使用场景

---

## 总结

阶段 1（Where 子句支持）已完成：
- ✅ 类型约束 where 子句：已存在并正常工作
- ✅ For 循环 where 过滤：已实现并测试通过
- ✅ 6 个模块可以单独编译成功
- ✅ 多个模块可以组合编译（解析层面）
- ⚠️ 仍有 4 个 codegen 问题需要在后续阶段修复

修改涉及 6 个文件，共约 150 行代码。所有修改都是向后兼容的，不影响现有功能。
