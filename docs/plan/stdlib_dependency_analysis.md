# 标准库模块依赖分析报告

## 分析日期
2026-02-27

## 目标
分析标准库模块的依赖关系，找出可以独立编译的模块，确定实现标准库支持所需的基础设施。

## 分析方法

1. 逐个测试标准库模块的编译
2. 识别每个模块的依赖和阻塞因素
3. 分类模块的复杂度和可用性

## 模块分析结果

### 1. std.sort - 部分可用 ⚠️

**测试结果**:
- 简单导入：✅ 成功
- 基本使用：✅ 编译成功
- 完整功能：❌ 需要泛型约束

**依赖分析**:
```cangjie
// 简单排序可以工作
import std.sort
let arr = [3, 1, 4, 1, 5, 9, 2, 6]
sort(arr)  // ✅ 成功
```

**阻塞因素**:
- 泛型扩展方法：`extend<T> Array<T> where T <: Comparable<T>`
- 类型约束：`where T <: Comparable<T>`
- 方法未找到：`Array.compareSwap3`

**内部依赖**:
- `std.math.*` - 数学函数
- 泛型 Array 类型 - ✅ 已支持（Phase 2）
- 类型约束系统 - ❌ 未实现

**结论**: 基本功能可用，但高级特性需要完整的泛型约束系统。

### 2. std.ref - 可导入 ✅

**测试结果**:
- 模块导入：✅ 成功
- 基本使用：❌ 需要泛型类实例化

**依赖分析**:
```cangjie
import std.ref  // ✅ 导入成功

// WeakRef<T> 是泛型类
let weak = WeakRef<Counter>(obj, EAGER)  // ❌ 泛型类未单态化
```

**阻塞因素**:
- 泛型类：`WeakRef<T>`
- 需要泛型类的单态化支持

**内部依赖**:
- 枚举变体非限定引用：`EAGER`, `DEFERRED` - ✅ 已支持（Phase 3）
- 泛型类实例化 - ❌ 未实现

**结论**: 可以导入但无法实际使用，需要泛型类单态化。

### 3. std.overflow - 不可用 ❌

**测试结果**:
- 模块导入：✅ 成功
- 方法调用：❌ 方法未找到

**依赖分析**:
```cangjie
import std.overflow
let x: Int64 = 100
x.checkedAdd(200)  // ❌ 方法未找到
```

**阻塞因素**:
- `@Intrinsic` 内建函数：`func checkedAdd<T>(x: T, y: T): ?T`
- 扩展方法调用内建函数：`extend Int64 <: CheckedOp<Int64>`

**内部依赖**:
- 编译器内建函数支持 - ❌ 未实现
- 平台特定常量：`isNative64` - ✅ 已支持（Phase 3）

**结论**: 完全依赖编译器内建函数，无法使用。

### 4. std.io - 不可用 ❌

**测试结果**: 未直接测试（从错误信息推断）

**阻塞因素**:
- 方法未找到：`InputStream.read`
- 流接口和实现

**内部依赖**:
- I/O 流接口
- 文件系统操作
- 可能依赖 `@Intrinsic` 函数

**结论**: 需要完整的 I/O 基础设施。

### 5. std.crypto - 不可用 ❌

**测试结果**: 未测试

**推断**:
- 可能依赖 `@Intrinsic` 函数
- 需要加密算法实现

**结论**: 复杂度高，暂不可用。

### 6. std.deriving - 不可用 ❌

**依赖分析**:
```cangjie
import std.deriving.api.*
import std.ast.Token
import std.collection.*
```

**阻塞因素**:
- 依赖 `std.ast` - 复杂的 AST 操作
- 依赖 `std.collection` - 泛型集合类型
- 宏和元编程支持

**结论**: 依赖链太长，暂不可用。

### 7. 其他模块（已排除）

| 模块 | 原因 |
|------|------|
| std.unicode | 大型常量数组 `CASE_RANGES` |
| std.ast | 复杂的 AST 类型和操作 |
| std.argopt | 命令行参数解析，依赖复杂特性 |
| std.binary | 二进制数据处理 |
| std.console | 控制台交互 |
| std.database | 数据库接口 |
| std.net | 网络功能 |
| std.posix | POSIX 系统调用 |

## 阻塞因素分类

### 1. 编译器内建函数 (@Intrinsic)

**影响模块**: overflow, crypto, 可能还有 io

**示例**:
```cangjie
@Intrinsic
func checkedAdd<T>(x: T, y: T): ?T
```

**需要实现**:
- 识别 `@Intrinsic` 标记
- 为每个内建函数生成 WASM 代码
- 支持溢出检查、位操作等

**优先级**: 高（解锁 overflow 模块）

### 2. 泛型类单态化

**影响模块**: ref, collection, 大部分标准库

**示例**:
```cangjie
class WeakRef<T> { ... }
let weak = WeakRef<Counter>(obj, EAGER)
```

**需要实现**:
- 收集泛型类的实例化
- 生成单态化版本
- 类型参数替换

**优先级**: 高（解锁大量模块）

### 3. 泛型约束 (where 子句)

**影响模块**: sort, 高级泛型使用

**示例**:
```cangjie
extend<T> Array<T> where T <: Comparable<T> {
    func compareSwap3(...) { ... }
}
```

**需要实现**:
- 解析 where 子句
- 类型约束检查
- 约束满足验证

**优先级**: 中（sort 基本功能已可用）

### 4. 泛型扩展方法

**影响模块**: sort, collection

**示例**:
```cangjie
extend<T> Array<T> where T <: Comparable<T> {
    func method() { ... }
}
```

**需要实现**:
- 泛型扩展的单态化
- 方法名称解析
- 类型参数传播

**优先级**: 中

### 5. I/O 基础设施

**影响模块**: io, 间接影响其他模块

**需要实现**:
- InputStream/OutputStream 接口
- 文件操作
- 标准输入输出

**优先级**: 低（可以用外部函数替代）

### 6. 大型常量数组

**影响模块**: unicode

**示例**:
```cangjie
let CASE_RANGES: Array<CaseRange> = [
    CaseRange(0x0041, 0x005A, 0, 32, 0),
    // ... 数百个元素
]
```

**需要实现**:
- 数据段中的数组初始化
- 常量数组的内存布局

**优先级**: 低（unicode 不是核心功能）

## 当前可用功能总结

### 完全可用 ✅
- 基本语法和类型
- 类和接口
- 简单泛型（Array 构造）
- 模块级常量
- 枚举变体非限定引用

### 部分可用 ⚠️
- **std.sort**: 基本排序功能可用
  - ✅ `sort(arr)` 简单排序
  - ❌ 带约束的泛型方法

- **std.ref**: 可导入但无法使用
  - ✅ 模块导入
  - ❌ WeakRef 实例化

### 不可用 ❌
- std.overflow - 需要 @Intrinsic
- std.io - 需要 I/O 基础设施
- std.crypto - 需要 @Intrinsic
- std.deriving - 依赖链太长
- 其他复杂模块

## 实现路线图

### 短期目标（1-2周）

**目标**: 使 std.sort 完全可用

1. **泛型扩展方法支持**
   - 解析 `extend<T> Type<T>` 语法
   - 单态化泛型扩展
   - 方法名称解析

2. **基本类型约束**
   - 解析 `where T <: Interface` 语法
   - 简单的约束检查
   - 内建接口（Comparable, Equatable）

**预期成果**: std.sort 完全可用，examples/std 编译成功

### 中期目标（2-4周）

**目标**: 支持更多标准库模块

1. **泛型类单态化**
   - 收集泛型类实例化
   - 生成单态化版本
   - 类型参数替换

2. **编译器内建函数**
   - 识别 @Intrinsic 标记
   - 实现常用内建函数
   - overflow 检查操作

**预期成果**: std.ref, std.overflow 可用

### 长期目标（1-2月）

**目标**: 完整的标准库支持

1. **完整泛型系统**
   - 类型推断
   - 复杂约束
   - 关联类型

2. **I/O 基础设施**
   - 流接口
   - 文件操作
   - 标准输入输出

3. **高级特性**
   - 宏系统
   - 元编程
   - 反射

**预期成果**: 大部分标准库模块可用

## 建议的下一步行动

### 立即行动（本周）

1. **实现泛型扩展方法**
   - 这是解锁 std.sort 的关键
   - 相对独立，不依赖其他大型特性
   - 可以快速验证效果

2. **简化 examples/std**
   - 暂时只测试基本功能
   - 避免使用高级特性
   - 作为渐进式测试基准

### 近期行动（下周）

1. **基本类型约束**
   - 实现 where 子句解析
   - 支持 Comparable 接口
   - 验证 std.sort 完整功能

2. **泛型类单态化**
   - 收集实例化信息
   - 生成单态化版本
   - 解锁 std.ref

### 中期行动（本月）

1. **编译器内建函数**
   - 设计内建函数框架
   - 实现 overflow 操作
   - 解锁 std.overflow

2. **完善测试**
   - 为每个模块创建独立测试
   - 验证功能正确性
   - 建立回归测试套件

## 测试策略

### 独立模块测试

为每个标准库模块创建独立测试：

```
tests/
  std_sort_basic.cj       - 基本排序
  std_sort_advanced.cj    - 高级特性
  std_ref_basic.cj        - 弱引用基础
  std_overflow_basic.cj   - 溢出检查
  ...
```

### 渐进式集成

1. 从最简单的功能开始
2. 逐步添加复杂特性
3. 每个阶段都有可工作的版本

### 回归测试

- 保持 36/37 examples 通过
- 每次修改后运行完整测试套件
- 记录每个阶段的进展

## 结论

### 当前状态

经过 Phase 1-3 的实施，我们已经解决了：
- ✅ 接口虚表问题
- ✅ 模块级常量
- ✅ 枚举变体非限定引用
- ✅ Array 构造函数

但标准库仍然无法使用，主要原因是：
- ❌ 缺少泛型扩展方法支持
- ❌ 缺少类型约束系统
- ❌ 缺少编译器内建函数

### 关键发现

1. **std.sort 最接近可用**
   - 基本功能已可工作
   - 只需泛型扩展方法支持

2. **@Intrinsic 是主要障碍**
   - overflow, crypto 完全依赖
   - 需要系统性解决方案

3. **泛型系统是核心**
   - 几乎所有模块都需要
   - 应该优先实现

### 推荐路径

**优先级排序**:
1. 泛型扩展方法（解锁 sort）
2. 类型约束（完善 sort）
3. 泛型类单态化（解锁 ref）
4. 编译器内建函数（解锁 overflow）

**预期时间线**:
- 1周：std.sort 完全可用
- 2周：std.ref 可用
- 4周：std.overflow 可用
- 2月：大部分标准库可用

**成功标准**:
- examples/std 编译成功并运行
- 至少 3 个标准库模块完全可用
- 测试通过率达到 100% (37/37)
