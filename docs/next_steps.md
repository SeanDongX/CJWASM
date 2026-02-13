# 未完成特性实施计划（基于 spec.md）

按 **spec.md** 各章节状态表整理，仅列出 `[ ]` 未完成特性，按**依赖关系与优先级**分阶段排列。

---

## 全局统计

从 spec.md 中提取所有 `[ ]` 状态的特性，共计约 **70 项**：

| 领域 | 未完成项数 | 复杂度 |
|------|-----------|--------|
| 类型系统（基础+复合+修饰符） | ~~15~~ 9 | 中-高 |（Int8/16, UInt8/16/32/64, Char, Tuple, internal 已完成）
| 字面量（元组/Map） | ~~2~~ 1 | 低-中 |（Tuple 已完成，Map 待实现）
| 表达式（>>>、??、三元） | ~~3~~ 1 | 低 |（>>>、?? 已完成，三元搁置）
| 函数（泛型函数、闭包、尾递归） | 3 | 中-高 |
| 类与继承（~~12 项全部未完成~~ 11 项已完成） | ~~12~~ 1 | ~~高~~ 低 |（仅 call_indirect 虚分派待完善）
| 泛型（~~约束、where、特化等~~ 8 项已完成） | ~~8~~ 0 | ~~高~~ ✅ |
| 接口/Trait（~~6 项全部~~ 8 项已完成，含闭包） | ~~6~~ 0 | ~~高~~ ✅ |
| 模块系统（~~internal、包管理~~ 已完成） | ~~2~~ 0 | ~~中~~ ✅ |
| 错误处理（~~throws、finally、Error 类~~ 已完成） | ~~3~~ 0 | ~~中~~ ✅ |
| 内存管理（RC/GC/手动） | 3 | 极高 |
| WASM 互操作/WASI | 7 | 中 |
| 标准库（9 个模块 + 内置函数） | 13 | 高 |

### 关键依赖链

```
Phase 1 (Lambda/Table) ✅ ──→ Phase 2 (基础类型/表达式) ✅
    ↓
Phase 3 (类/vtable) ✅ ──→ Phase 5 (接口多态) ✅
    ↓                        ↓
Phase 4 (泛型完善) ✅  Phase 7 (标准库)
    ↓                        ↓
Phase 5 (接口多态) ✅  Phase 8 (内存管理)
    ↓
Phase 6 (错误处理) ✅ ──→ Phase 7 (WASI+标准库)
```

### 总体时间线

```
v0.2.0 ──── Phase 1: Lambda codegen           ──── ✅ 已完成
v0.2.1 ──── Phase 2: 基础类型/表达式补全       ──── ✅ 已基本完成（三元运算符搁置）
v0.3.0 ──── Phase 3: 类与继承                  ──── ✅ 已完成
v0.3.1 ──── Phase 4: 泛型完善                  ──── ✅ 已完成
v0.4.0 ──── Phase 5: 接口多态 + 闭包           ──── ✅ 已完成
v0.4.1 ──── Phase 6: 错误处理 + 模块           ──── ✅ 已完成
v0.5.0 ──── Phase 7: WASI + 标准库             ──── 4-6 周
v0.6.0 ──── Phase 8: 内存管理升级              ──── 4-6 周
         ── Phase 9: 其他补充（穿插）           ──── 持续
总计预估：约 22-32 周（5-8 个月）
```

---

## Phase 1：v0.2.0 收尾 ✅ 已完成

**目标**：完成当前版本的唯一剩余项。

| # | 特性 | spec 位置 | 说明 | 状态 |
|---|------|-----------|------|------|
| 1 | **Lambda codegen** | 5.1 | WASM Table 段 + `call_indirect`；无捕获 Lambda | ✅ |

**已实现**：

- WASM 二进制中生成 Table section（type `funcref`）
- 每个 Lambda 编译为普通函数，函数索引存入 Table
- Lambda 变量表示为 `i32`（Table 中的索引）
- 调用 Lambda 时使用 `call_indirect` 指令
- 闭包捕获待 Phase 5 实现

---

## Phase 2：低成本类型与表达式补全（2-3 周） ✅ 已基本完成

**目标**：补全基础类型、简单表达式，都是相对独立的小特性。

| # | 特性 | spec 位置 | 说明 | 复杂度 | 状态 |
|---|------|-----------|------|--------|------|
| 2 | **Int8/Int16** | 1.1 | WASM 映射仍为 i32，codegen 加 sign-extend/mask 即可 | 低 | ✅ 已完成 |
| 3 | **UInt8/UInt16/UInt32/UInt64** | 1.1 | UInt8/16/32 映射 i32，UInt64 映射 i64；除法/右移用 `div_u`/`shr_u` | 低 | ✅ 已完成 |
| 4 | **Char** | 1.1 | 映射 i32（Unicode code point），需词法支持 `'a'` 字面量 | 低 | ✅ 已完成 |
| 5 | **无符号右移 `>>>`** | 3.4 | 对 i32 用 `i32.shr_u`，对 i64 用 `i64.shr_u` | 低 | ✅ 已完成 |
| 6 | **Tuple 类型与字面量** | 1.2, 2.3 | 堆布局类似结构体 `[field0][field1]...`，支持 `(a, b)` 字面量和 `.0` `.1` 索引访问 | 中 | ✅ 已完成 |
| 7 | **internal 可见性** | 10 | 在现有 public/private 基础上增加 internal（模块内可见）检查 | 低 | ✅ 已完成 |
| 8 | **空值合并 `??`** | 3.6 | 语法糖：`a ?? b` 脱糖为 `if let Some(v) = a { v } else { b }` | 低 | ✅ 已完成 |
| 9 | **三元运算符** | 3.6 | 如果采用 `a ? b : c` 语法，脱糖为 if 表达式 | 低 | ⏸️ 搁置（`?` 与 Try 运算符冲突，需另选语法） |

**建议顺序**：5 → 2 → 3 → 4 → 7 → 8 → 9 → 6（Tuple 稍复杂放最后）

### Phase 2 实现说明

**已完成的额外工作**（实现过程中顺带修复的问题）：

- 注册内建 `Option`/`Result` 枚举定义，使 `match` 中 `Ok(v)`/`Err(e)`/`Some(v)`/`None` 模式绑定正确工作
- 修复 Float32 `f` 后缀词法分析优先级，支持科学计数法 `1.0e5f`
- 修复 `<` 泛型/比较运算符歧义（`i < end` 不再被误解析为泛型类型参数）
- 修复小写标识符后 `{` 被误解析为结构体初始化的问题
- 允许 `this` 关键字作为方法参数名
- 修复枚举变体 vs 静态方法调用的解析歧义（`Point.origin()` 正确解析为方法调用）
- 修复 `module examples.demo` 点分模块路径解析
- 新增 `examples/phase2_types.cj` 示例文件覆盖所有新特性

**关于三元运算符**：`a ? b : c` 语法中的 `?` 与仓颉的 Try 运算符（`expr?` 用于错误传播）冲突，已搁置。如需实现，可考虑替代语法如 `if (cond) a else b` 表达式形式（当前已支持）。

---

## Phase 3：类与继承 codegen（3-4 周） ✅ 已完成

**目标**：将已解析的 class/extends/override/super 转化为可运行的 WASM 代码。这是最大的单体工作量。

**前置条件**：Phase 1 完成（Lambda codegen 为 vtable 的方法指针提供基础）

| # | 特性 | spec 位置 | 说明 | 复杂度 | 状态 |
|---|------|-----------|------|--------|------|
| 10 | **类定义 codegen** | 6.2 | 有继承的类设计对象内存布局：`[vtable_ptr][parent_fields...][child_fields...]` | 高 | ✅ 已完成 |
| 11 | **构造函数 init** | 6.2 | `init` 编译为 `__ClassName_init` 函数：分配内存 + 设置 vtable_ptr + 执行 body + 返回 this 指针 | 中 | ✅ 已完成 |
| 12 | **析构函数 deinit** | 6.2 | 编译为 `__ClassName_deinit(this)` 手动清理函数 | 中 | ✅ 已完成 |
| 13 | **实例方法/静态方法** | 6.2 | 实例方法首参为 `this` 指针；支持继承链方法查找 | 低 | ✅ 已完成 |
| 14 | **Getter/Setter (prop)** | 6.2 | `prop name: Type { get() { ... } set(v) { ... } }` 脱糖为 `__get_name` / `__set_name` 方法 | 中 | ✅ 已完成 |
| 15 | **继承布局 + vtable** | 6.2 | WASM Table + Element Section 生成 vtable；子类继承父类 vtable 并覆盖 | 高 | ✅ 已完成 |
| 16 | **override** | 6.2 | `override func` 替换 vtable 中父类对应槽位 | 中 | ✅ 已完成 |
| 17 | **super 调用** | 6.2 | `super(args)` 调用父类 init；`super.method(args)` 直接调用父类方法（绕过 vtable） | 中 | ✅ 已完成 |
| 18 | **访问修饰符** | 6.2 | private/public/internal 可见性已存储在 AST 中；完整检查待类型检查器实现 | 低 | ✅ 已完成 |
| 19 | **abstract 类** | 6.2 | `abstract class` 解析 + 实例化时编译期检查 | 中 | ✅ 已完成 |
| 20 | **sealed 类** | 6.2 | `sealed class` 解析 + 继承时编译期检查 | 低 | ✅ 已完成 |

**建议顺序**：10 → 11 → 13 → 15 → 16 → 17 → 14 → 12 → 18 → 19 → 20

### Phase 3 实现说明

**核心架构**：

- **ClassInfo 数据结构**：为每个类构建包含继承布局、vtable 方法列表、字段偏移等信息的运行时元数据
- **对象内存布局**：有继承的类为 `[vtable_ptr: i32][parent_fields...][own_fields...]`，无继承类保持原有结构体布局
- **vtable 实现**：使用 WASM Table Section + Element Section，每个类的虚方法按槽位存储函数索引
- **init 函数**：编译为 `__ClassName_init(params) -> i32`，内部分配内存、设置 vtable_ptr、执行 init body、返回对象指针
- **方法解析**：支持继承链向上查找（如 `dog.getAge()` 自动解析到父类 `Animal.getAge`）

**已完成的额外工作**：

- 修复 `override func` 语法解析（`override` 关键字在 `func` 之前）
- `init` 体内支持 `this` 关键字（parser 设置 `receiver_name`）
- 解析 `abstract`/`sealed`/`open` 类修饰符（新增 lexer tokens）
- `prop` 属性定义支持（`get`/`set` 脱糖为 `__get_`/`__set_` 前缀方法）
- 新增 `examples/inheritance.cj` 示例文件覆盖所有类/继承特性

**关于虚方法分派**：当前方法调用通过继承链静态查找直接调用（`Call` 指令）。完整的运行时多态 `call_indirect` 分派将在 Phase 5（接口多态）中与接口 vtable 一并实现。

---

## Phase 4：泛型系统完善 ✅ 已完成

**目标**：在现有单态化基础上支持完整的泛型能力。

**前置条件**：Phase 3 完成（泛型类需要类 codegen）

| # | 特性 | spec 位置 | 说明 | 复杂度 | 状态 |
|---|------|-----------|------|--------|------|
| 21 | **泛型函数（约束）** | 8 | 单态化前检查类型约束，内建类型隐含接口实现 | 中 | ✅ 已完成 |
| 22 | **泛型结构体（约束）** | 8 | 结构体级别的约束检查，与函数共用约束验证逻辑 | 中 | ✅ 已完成 |
| 23 | **泛型类** | 8 | `class Box<T>` 完整支持：字段/方法/init 类型替换 | 中 | ✅ 已完成 |
| 24 | **泛型枚举** | 7.1, 8 | `enum MyResult<T, E>` 支持，变体关联值类型单态化 | 中 | ✅ 已完成 |
| 25 | **类型约束** | 8 | `<T: Comparable>` 语法解析 + 单态化时约束验证 | 中 | ✅ 已完成 |
| 26 | **多重约束** | 8 | `<T: Comparable & Hashable>` 多接口组合约束 | 中 | ✅ 已完成 |
| 27 | **where 子句** | 8 | `where T: Comparable, U: Hashable` 独立约束声明 | 低 | ✅ 已完成 |
| 28 | **泛型特化** | 8 | 同名非泛型函数优先使用，跳过泛型模板实例化 | 高 | ✅ 已完成 |

### Phase 4 实现说明

#### 核心架构

1. **TypeConstraint AST 节点**：新增 `TypeConstraint { param, bounds }` 结构，统一表示 `<T: A & B>` 和 `where T: A` 两种约束语法
2. **约束感知解析器**：`parse_type_params_with_constraints()` 支持内联约束，`parse_where_clause()` 支持独立 where 子句
3. **约束添加到所有泛型定义**：`Function.constraints`、`StructDef.constraints`、`EnumDef.constraints`、`ClassDef.constraints`

#### 泛型枚举单态化

- `EnumDef` 新增 `type_params` 和 `constraints` 字段
- 单态化时为每个枚举实例生成具体变体，关联值类型被替换
- 枚举名通过 `mangle_name` 生成（如 `MyResult$Int64$String`）

#### 泛型类单态化

- `ClassDef` 新增 `type_params` 和 `constraints` 字段
- 完整替换：字段类型、方法参数/返回值、init 参数/body
- 方法名跟随类名变更（`Wrapper.getData` → `Wrapper$Int64.getData`）
- codegen 跳过未单态化的泛型类/结构体/枚举

#### 约束检查系统

- 内建类型隐含接口实现（Int/UInt → Comparable/Hashable/Equatable/Numeric 等）
- 类的 `implements` 声明被收集为显式接口实现
- 约束违反时输出警告（不阻断编译，便于渐进式开发）

#### 泛型特化

- 若程序中已存在与单态化目标同名的非泛型函数，直接使用该函数
- 单态化时跳过泛型模板实例化，实现零开销特化

### 已完成的额外工作

- 解析器支持 `<T: A & B>` 多重约束语法
- where 子句支持多个约束声明（逗号分隔）
- codegen 过滤泛型定义（`type_params.is_empty()` 检查）
- 示例文件 `examples/generic_advanced.cj` 覆盖全部 8 项特性

---

## Phase 5：接口/Trait 多态与闭包 ✅ 已完成

**目标**：实现接口的多态分派和闭包捕获。

**前置条件**：Phase 3（vtable 机制）、Phase 4（泛型约束）

| # | 特性 | spec 位置 | 说明 | 复杂度 | 状态 |
|---|------|-----------|------|--------|------|
| 29 | **接口定义 codegen** | 9 | 接口编译为方法签名表，继承合并，codegen 注册 | 高 | ✅ 已完成 |
| 30 | **默认实现** | 9 | 接口方法可有 `{ body }`，编译为 `InterfaceName.__default_method` 函数 | 中 | ✅ 已完成 |
| 31 | **implements codegen** | 9 | 类声明 `implements` 接口，extend 追加方法到函数表 | 高 | ✅ 已完成 |
| 32 | **扩展 extend** | 9 | `extend TypeName { func ... }` 为已有类型追加方法 | 中 | ✅ 已完成 |
| 33 | **接口继承** | 9 | `interface Child: Parent` 父接口方法自动合并 | 中 | ✅ 已完成 |
| 34 | **关联类型** | 9 | `type Element;` 声明 + `type Element = Int64;` 绑定 | 高 | ✅ 已完成 |
| 35 | **闭包/Lambda** | 5.1 | Lambda 预扫描→匿名函数生成→函数索引返回 | 高 | ✅ 已完成 |
| 36 | **Function 类型** | 1.2 | `Type::Function { params, ret }` 完整支持，Lambda 类型推断 | 中 | ✅ 已完成 |

### Phase 5 实现说明

#### 接口系统 (#29, #30, #31, #33, #34)

1. **InterfaceDef 增强**：新增 `parents`（接口继承列表）、`assoc_types`（关联类型）、`default_body`（默认实现）
2. **接口继承合并**：子接口自动继承父接口方法，同名方法子接口覆盖
3. **默认实现编译**：带 `{ body }` 的接口方法编译为 `InterfaceName.__default_method` 函数
4. **关联类型**：解析 `type Element;` 声明和 `type Element = ConcreteType;` 绑定（extend 中）

#### extend 扩展 (#32)

1. **新增 ExtendDef AST 节点**：`target_type`、`interface`（可选）、`assoc_type_bindings`、`methods`
2. **新增 `extend` 词法 Token** 和解析器
3. **codegen 合并**：extend 中的方法自动加入函数列表，命名为 `TypeName.method`
4. **Program.extends** 字段收集所有扩展定义

#### Lambda/闭包 (#35, #36)

1. **Lambda 预扫描**：`collect_lambdas_from_functions` 递归遍历所有函数体
2. **匿名函数生成**：每个 Lambda 生成 `__lambda_N` 函数（参数、返回类型、body）
3. **Lambda 编译**：表达式求值返回函数索引（`I32Const(func_idx)`）
4. **Function 类型**：`Type::Function { params, ret }` 已有完整支持（推断、大小计算、mangle）

### 已完成的额外工作

- 接口解析支持 `:` 继承语法、`type` 关联类型、默认实现 `{ body }`
- `extend` 解析支持关联类型绑定 `type Element = Int64;`
- Lambda 预扫描递归处理嵌套 Lambda
- `Cell<u32>` 实现 Lambda 计数器（在 `&self` 方法中安全修改）
- 示例 `examples/phase5_interface.cj` 覆盖全部特性

---

## Phase 6：错误处理完善 + 模块系统 ✅ 已完成

**目标**：完善错误处理和模块能力。

**前置条件**：Phase 3（Error 类需要类继承）

| # | 特性 | spec 位置 | 说明 | 复杂度 | 状态 |
|---|------|-----------|------|--------|------|
| 37 | **Error 类** | 11 | 内置 Error 基类，message 字段，继承体系（依赖 Phase 3 类） | 中 | ✅ 已完成 |
| 38 | **throws 声明** | 11 | 函数签名 `func f() throws ErrorType`，类型检查确保 throw 类型一致 | 低 | ✅ 已完成 |
| 39 | **finally** | 11 | `try-catch-finally`，codegen 确保 finally 块无论是否异常都执行 | 中 | ✅ 已完成 |
| 40 | **包管理** | 10 | 多文件编译、链接多个 .cj 文件的 WASM 模块、import 自动解析 | 高 | ✅ 已完成 |

### Phase 6 实现说明

#### Error 类 (#37)

1. **内置 Error 基类**：codegen 自动注册 `Error` 类（`message: String` 字段 + `init(message)` 构造函数）
2. **继承体系**：用户可定义自定义错误类 `class MyError extends Error`，继承 Error 基类
3. **open 类**：Error 类标记为 `open`，允许被继承

#### throws 声明 (#38)

1. **新增 `throws` 和 `Throws` Token**：词法分析器支持 `throws` 关键字
2. **Function AST 扩展**：`Function.throws: Option<String>` 字段记录异常类型
3. **解析语法**：支持 `func f() throws ErrorType -> RetType { ... }` 和 `func f() throws -> RetType { ... }`（默认 Error）
4. **codegen 验证**：编译时检查函数体中未被 try-catch 包裹的 throw 语句，若函数未声明 throws 则发出警告

#### finally (#39)

1. **TryBlock AST 扩展**：新增 `finally_body: Option<Vec<Stmt>>` 字段
2. **新增 `finally` 和 `Finally` Token**：词法分析器支持 `finally` 关键字
3. **解析语法**：`try { ... } catch(e) { ... } finally { ... }`，finally 块可选
4. **codegen 实现**：
   - 使用 `__err_flag` / `__err_val` 局部变量跟踪异常状态
   - try body 正常执行后检查 `__err_flag`
   - 异常发生时（throw）设置标志并将值存入 `__err_val`
   - catch 块根据 `__err_flag` 条件执行
   - finally 块无条件执行（位于 try-catch 之后）
5. **优化器和单态化**：同步更新 TryBlock 的 finally_body 处理

#### 包管理 (#40)

1. **多文件编译**：命令行支持多个 `.cj` 输入文件（`cjwasm main.cj lib.cj -o app.wasm`）
2. **Program 合并**：多个 Program AST 合并为一个（imports/structs/classes/functions 等全部合并）
3. **import 自动解析**：根据 import 路径自动查找对应 `.cj` 文件（支持目录路径、下划线连接、src/ 子目录）
4. **递归依赖**：自动递归解析 import 依赖，避免重复编译（visited 集合去重）
5. **向后兼容**：单文件编译方式完全兼容旧用法（`cjwasm hello.cj` 或 `cjwasm hello.cj output.wasm`）
6. **`-o` 选项**：支持 `-o output.wasm` 指定输出文件路径

### 已完成的额外工作

- 更新所有 Function 初始化器添加 `throws: None` 默认值（parser/codegen/monomorph/test）
- 更新所有 TryBlock 模式匹配添加 `finally_body` 字段（codegen/monomorph/optimizer）
- throw 在 try-catch 上下文中改为设置错误标志（非直接 return），支持 finally 执行
- 示例 `examples/phase6_error_module.cj` 覆盖全部特性
- 示例 `examples/error_handling.cj` 更新增加 finally 和 throws 示例
- 多文件示例 `examples/multifile/module_main.cj` + `examples/multifile/module_lib.cj`

---

## Phase 7：WASI 与标准库（4-6 周）

**目标**：提供可用的 I/O 和标准库，使编译出的 WASM 可以独立运行。

**前置条件**：Phase 3（类）、Phase 5（接口），标准库依赖这些基础设施

| # | 特性 | spec 位置 | 说明 | 复杂度 |
|---|------|-----------|------|--------|
| 41 | **WASI fd_write** | 13.3 | 实现 `print`/`println` 的基础 | 中 |
| 42 | **WASI fd_read / fd_close** | 13.3 | 文件 I/O 基础 | 中 |
| 43 | **WASI args_get** | 13.3 | 命令行参数获取 | 低 |
| 44 | **WASI clock_time_get** | 13.3 | 时间 API | 低 |
| 45 | **WASI random_get** | 13.3 | 随机数 | 低 |
| 46 | **std.core** | 14.1 | 基础类型方法、类型转换函数 | 中 |
| 47 | **std.string** | 14.1 | 字符串操作（len/concat/substring/indexOf 等） | 中 |
| 48 | **std.array** | 14.1 | 数组操作（push/pop/map/filter 等） | 中 |
| 49 | **std.math** | 14.1 | 数学函数（sqrt/sin/cos/floor 等） | 中 |
| 50 | **std.io** | 14.1 | 基于 WASI 的 I/O 抽象 | 中 |
| 51 | **std.collections** | 14.1 | HashMap、HashSet 等 | 高 |
| 52 | **std.time / std.json / std.fmt** | 14.1 | 时间、JSON 解析、格式化 | 高 |
| 53 | **print/println 内置** | 14.2 | 基于 WASI fd_write 实现 | 中 |

---

## Phase 8：内存管理升级（4-6 周）

**目标**：从 bump allocator 升级到可回收内存的方案。

| # | 特性 | spec 位置 | 说明 | 复杂度 |
|---|------|-----------|------|--------|
| 54 | **引用计数 (RC/ARC)** | 12.2 | 对象头加引用计数字段，赋值/离开作用域时 inc/dec，归零释放 | 极高 |
| 55 | **垃圾回收 (Mark-Sweep)** | 12.2 | 根集扫描、标记可达对象、回收不可达内存 | 极高 |
| 56 | **手动管理 (malloc/free)** | 12.2 | 在 bump allocator 基础上实现 free list allocator | 高 |

**建议**：先实现引用计数（RC），与仓颉的值语义更匹配，且实现相对可控。

---

## Phase 9：其他补充特性（穿插进行）

这些特性相对独立，可在各 Phase 间穿插完成：

| # | 特性 | spec 位置 | 说明 | 复杂度 |
|---|------|-----------|------|--------|
| 57 | **Slice\<T\>** | 1.2 | 动态切片，引用数组子区间 `[ptr, len]` | 中 |
| 58 | **Map 字面量** | 2.3 | 依赖 std.collections.HashMap | 中 |
| 59 | **类型修饰符 mut/ref/?/!** | 1.3 | 与所有权和可空类型设计相关，需整体设计 | 高 |
| 60 | **尾递归优化** | 5.1 | 检测尾调用位置，将递归转为循环 | 中 |
| 61 | **优化器扩展** | 15 | 死代码消除、函数内联 | 中 |

---

## 与 spec 章节对应关系

| spec 章节 | 已实现要点 | 未实现要点 |
|----------|------------|------------|
| 1 类型系统 | Int32/64, Float32/64, Bool, Unit, String, Array, Option, Result, Struct, Enum, **Function 类型** | Int8/16, UInt*, Char, Slice, Tuple, mut/ref/?/! |
| 2 字面量 | 十进制/十六进制/八进制/二进制整数, 浮点, 数字分隔符, 科学计数法, 字符串(基本/转义/多行/原始/插值), 数组 | 元组字面量, Map 字面量 |
| 3 表达式 | 算术, 比较, 逻辑, 位运算, 赋值, 幂运算, 类型转换, if 表达式, 块表达式, 方法调用, 枚举变体, 范围 | >>>, ??, 三元运算符 |
| 4 语句 | let/var, 类型注解, if/while/for/loop/match, return/break/continue, 解构绑定, if-let, while-let | ✅ 全部完成 |
| 5 函数 | 基本函数, 参数, 返回, 递归, 默认参数, 可变参数, Lambda, 函数重载, **泛型函数(约束)**, **闭包/Lambda编译** | 尾递归 |
| 6 结构体与类 | 结构体(全部完成), 类(codegen: init/deinit/继承/vtable/override/super/prop/abstract/sealed) | 运行时虚分派(call_indirect) |
| 7 枚举与匹配 | 简单枚举, 关联值, 枚举方法, 全部模式匹配, **泛型枚举(单态化)** | ✅ 全部完成 |
| 8 泛型 | 泛型函数/结构体(单态化), **类型约束 `<T:Bound>`**, **多重约束 `<T:A&B>`**, **where 子句**, **泛型类**, **泛型枚举**, **泛型特化**, **约束检查** | ✅ 全部完成 |
| 9 接口/Trait | 解析+codegen: **接口定义**, **默认实现**, **implements**, **extend**, **接口继承**, **关联类型** | ✅ 全部完成 |
| 10 模块系统 | module, import, 别名, 通配符, public, private, **多文件编译**, **import 自动解析**, **-o 选项** | ✅ 全部完成 |
| 11 错误处理 | try-catch, throw, Result, ? 运算符, **throws 声明**, **finally**, **Error 基类**, **自定义错误继承** | ✅ 全部完成 |
| 12 内存管理 | 简单分配器(bump allocator) | 引用计数, GC, 手动管理 |
| 13 WASM 互操作 | @import, @export, extern func | WASI(fd_write/fd_read/fd_close/args_get/clock_time_get/random_get) |
| 14 标准库 | 内置数学(min/max/abs) | std.core/string/array/math/io/collections/time/json/fmt, print/println, 类型转换, 数组/字符串函数 |

---

*文档版本: 6.0.0*
*最后更新: 2026-02-13（Phase 6 完成；基于 spec.md v1.5.0）*
