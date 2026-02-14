# 未完成特性实施计划（基于 spec.md）

按 **spec.md** 各章节状态表整理，仅列出 `[ ]` 未完成特性，按**依赖关系与优先级**分阶段排列。

---

## 全局统计

从 spec.md 中提取所有 `[ ]` 状态的特性，共计约 **70 项**：

| 领域 | 未完成项数 | 复杂度 |
|------|-----------|--------|
| 类型系统（基础+复合+修饰符） | ~~15~~ 0 | ✅ |（Int8/16, UInt8/16/32/64, Rune, Tuple, IntNative/UIntNative/Float16/Nothing/VArray/This 已完成）
| 字面量（元组/Map） | ~~2~~ 0 | ✅ |（Tuple、Map 已完成）
| 表达式（??、三元） | ~~3~~ 1 | 低 |（?? 已完成，>>> 已移除 [cjc 不支持]，三元搁置）
| 函数（泛型函数、闭包、尾递归） | 3 | 中-高 |
| 类与继承（~~12 项全部未完成~~ 11 项已完成） | ~~12~~ 1 | ~~高~~ 低 |（仅 call_indirect 虚分派待完善）
| 泛型（~~约束、where、特化等~~ 8 项已完成） | ~~8~~ 0 | ~~高~~ ✅ |
| 接口/Trait（~~6 项全部~~ 8 项已完成，含闭包） | ~~6~~ 0 | ~~高~~ ✅ |
| 包系统（~~internal、包管理~~ 已完成，package/import 对齐 cjc） | ~~2~~ 0 | ~~中~~ ✅ |
| 错误处理（~~throws、finally、Error 类~~ 已完成，throws 已移除 [cjc 不支持]） | ~~3~~ 0 | ~~中~~ ✅ |
| 内存管理（~~RC/GC/手动~~ 已完成） | ~~3~~ 0 | ~~极高~~ ✅ |
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
Phase 5 (接口多态) ✅  Phase 8 (内存管理) ✅
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
v0.6.0 ──── Phase 8: 内存管理升级              ──── ✅ 已完成
         ── Phase 9: 其他补充（穿插）           ──── ✅ 已完成
v0.8.0 ──── Phase 10: cjc release/1.0 语法对齐 ──── ✅ 已完成
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
| 4 | **Rune (原 Char)** | 1.1 | 映射 i32（Unicode code point），需词法支持 `'a'` 字面量 | 低 | ✅ 已完成 |
| 5 | ~~**无符号右移 `>>>`**~~ | 3.4 | ~~对 i32 用 `i32.shr_u`，对 i64 用 `i64.shr_u`~~ cjc 不支持，已移除 | 低 | ❌ 已移除 |
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
| 12 | **析构函数 ~init** | 6.2 | 编译为 `__ClassName_deinit(this)` 手动清理函数（cjc: `~init` 替代 `deinit`） | 中 | ✅ 已完成 |
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
- **cjc 对齐**：继承使用 `<:` 语法（替代 `extends`），析构使用 `~init`（替代 `deinit`），类体内支持 `open`/`static` 修饰符

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
| 31 | **<: 接口实现 codegen** | 9 | 类声明 `<:` 接口（cjc: 替代 `implements`），extend 追加方法到函数表 | 高 | ✅ 已完成 |
| 32 | **扩展 extend** | 9 | `extend TypeName { func ... }` 为已有类型追加方法 | 中 | ✅ 已完成 |
| 33 | **接口继承** | 9 | `interface Child <: Parent` 父接口方法自动合并（cjc: 使用 `<:`） | 中 | ✅ 已完成 |
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
| 38 | ~~**throws 声明**~~ | 11 | ~~函数签名 `func f() throws ErrorType`~~ cjc 无此语法，已移除 | 低 | ❌ 已移除 |
| 39 | **finally** | 11 | `try-catch-finally`，codegen 确保 finally 块无论是否异常都执行 | 中 | ✅ 已完成 |
| 40 | **包管理** | 10 | 多文件编译、链接多个 .cj 文件的 WASM 模块、import 自动解析（cjc: `package` 替代 `module`，点分路径导入） | 高 | ✅ 已完成 |

### Phase 6 实现说明

#### Error 类 (#37)

1. **内置 Error 基类**：codegen 自动注册 `Error` 类（`message: String` 字段 + `init(message)` 构造函数）
2. **继承体系**：用户可定义自定义错误类 `class MyError <: Error`（cjc: `<:` 替代 `extends`），继承 Error 基类
3. **open 类**：Error 类标记为 `open`，允许被继承

#### ~~throws 声明 (#38)~~ 已移除（cjc 不支持）

> 原实现：函数签名 `func f() throws ErrorType` 语法，已在 v0.8.0 cjc 对齐中移除。
> cjc 不使用显式 `throws` 声明，throw 语句仍可在函数体内使用，配合 try-catch 捕获。

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
- 示例 `examples/error_handling.cj` 更新增加 finally 示例
- 多文件示例 `examples/multifile/module_main.cj` + `examples/multifile/module_lib.cj`（cjc: 使用 `package` 声明）

---

## Phase 7：WASI 与标准库（6-10 周）

**目标**：实现仓颉标准库 (std) 的 WASM/WASI 子集，使编译出的 WASM 可以独立运行常用程序。

**前置条件**：Phase 3（类）、Phase 5（接口）、Phase 6（错误处理），标准库依赖这些基础设施。

**参考来源**：仓颉标准库文档 (`libs/std/` in cangjiecorpus-mirror)

> **设计原则**：
> - 标准库函数在 codegen 中作为**内建运行时函数**实现（WASM 字节码），不需要用户显式 import
> - WASI 系统调用通过 `wasi_snapshot_preview1` 导入实现
> - 受 WASM 沙箱限制，不实现 `std.net`、`std.process`、`std.sync`、`std.reflect`、`std.posix` 等平台强依赖包
> - 优先实现使用频率最高、WASM 可行性最强的 API

---

### Phase 7.1：std.core 核心函数（1-2 周）✅ 已完成

已完成：`print`/`println`/`eprint`/`eprintln`（含多类型重载和 ToString 分发）、`readln()`（WASI fd_read）、字符串插值（含 struct toString）、`min`/`max`、基本类型转换

| # | 特性 | 仓颉原型 | 说明 | 复杂度 | 状态 |
|---|------|---------|------|--------|------|
| 41 | **print/println 多类型重载** | `print(Int64)`, `print(Float64)`, `print(Bool)`, `print(String)`, `print(Rune)`, `println(...)` | 对每种基础类型分发到对应的运行时输出函数；`print` 不换行、`println` 带换行 | 中 | ✅ 已完成 |
| 42 | **print\<T\>(T) where T <: ToString** | `print<T>(arg: T)` | 对实现 `toString()` 方法的 struct/class，自动调用 `toString()` 转为字符串后输出；字符串插值 `"${obj}"` 同样支持 | 中 | ✅ 已完成 |
| 43 | **eprint / eprintln** | `eprint(str: String)`, `eprintln(str: String)` | 输出到 stderr (fd=2)，支持 i64/str/bool 三种类型 | 低 | ✅ 已完成 |
| 44 | **readln()** | `readln(): String` | 基于 WASI `fd_read` 从 stdin (fd=0) 逐字节读取直到 `\n` 或 EOF，自动去除尾部 `\n`/`\r\n` | 中 | ✅ 已完成 |
| 45 | **println() 空行** | `println(): Unit` | 仅输出换行符；`eprintln()` 也支持空行 | 低 | ✅ 已完成 |
| 46 | **min / max** | `min<T>(a: T, b: T): T where T <: Comparable<T>` | i64 版本使用运行时函数，Float64 版本使用 WASM `f64.min`/`f64.max` 指令 | 低 | ✅ 已完成 |
| 47 | **sleep(Duration)** | `sleep(dur: Duration)` | WASI 无直接支持，可用 `poll_oneoff` 模拟或暂不实现 | 高 | 不实现 |

**实现要点**：
- I/O 运行时已重构为参数化 `emit_output_*(fd, newline)` 函数，共生成 12 个变体（println/print/eprintln/eprint × i64/str/bool）
- `print<T>(T) where T <: ToString`：编译期检查参数是否为 struct/class 类型且有 `ClassName.toString` 方法注册，如有则调用 `toString()` 获取字符串再调用 `__*_str`；字符串插值 `"${obj}"` 中同样支持 struct ToString
- `readln()`：WASI `fd_read` 作为第二个内置导入（`num_builtin_imports = 2`），运行时函数 `__readln` 逐字节读入临时缓冲区（偏移 128-1024），然后通过 `__alloc` 分配堆字符串对象并复制

---

### Phase 7.2：std.core 类型方法（1-2 周）

为基础类型添加实例方法和静态方法，使其与仓颉标准库一致。

| # | 特性 | 仓颉原型 | 说明 | 复杂度 |
|---|------|---------|------|--------|
| 48 | **Int64.toString()** | `func toString(): String` | 整数转字符串（已有 `__i64_to_str` 运行时） | 低 |
| 49 | **Float64.toString()** | `func toString(): String` | 浮点转字符串（已有 `__f64_to_str` 运行时） | 低 |
| 50 | **Bool.toString()** | `func toString(): String` | `true` / `false` 字符串 | 低 |
| 51 | **Int64.toFloat64() / Float64.toInt64()** | `func toFloat64(): Float64` 等 | 类型转换方法（补充 `as` 语法） | 低 |
| 52 | **String.size / String.isEmpty()** | `prop size: Int64`, `func isEmpty(): Bool` | 字符串长度和判空 | 低 |
| 53 | **String.toInt64() / String.toFloat64()** | `func toInt64(): Int64` | 字符串解析为数值（atoi/atof 运行时） | 中 |
| 54 | **Int64.abs() / Int64.compareTo()** | `static func abs(x: Int64): Int64`, `func compareTo(other: Int64): Ordering` | 绝对值和比较 | 低 |

**实现要点**：
- 在 codegen 中为基础类型注册 **内建方法表**（类似 vtable），当编译 `x.toString()` 时检查 x 是否为内建类型
- `String.toInt64()`：实现 `__str_to_i64` 运行时函数，逐字节解析十进制字符串
- `Ordering` 枚举（`LT`, `EQ`, `GT`）已在 AST 中支持，codegen 映射为 -1/0/1

---

### Phase 7.3：std.math 数学函数（1 周）✅ 已完成

| # | 特性 | 仓颉原型 | WASM 实现方式 | 复杂度 | 状态 |
|---|------|---------|-------------|--------|------|
| 55 | **sqrt** | `func sqrt(x: Float64): Float64` | WASM `f64.sqrt` 指令 | 低 | ✅ 已完成 |
| 56 | **floor / ceil / trunc / nearest** | `func floor(x: Float64): Float64` 等 | WASM `f64.floor` / `f64.ceil` / `f64.trunc` / `f64.nearest` | 低 | ✅ 已完成 |
| 57 | **abs (Float)** | `func abs(x: Float64): Float64` | WASM `f64.abs` 指令（用户未定义同名函数时） | 低 | ✅ 已完成 |
| 58 | **min / max (Float)** | `func fmin(a: Float64, b: Float64): Float64` | WASM `f64.min` / `f64.max`（`fmin`/`fmax` 内置名） | 低 | ✅ 已完成 |
| 59 | **copysign / neg** | `func copysign(x: Float64, y: Float64): Float64` | WASM `f64.copysign` / `f64.neg` | 低 | ✅ 已完成 |
| 60 | **sin / cos / tan** | `func sin(x: Float64): Float64` 等 | 泰勒级数（sin: 12 项 + 范围归约，cos/tan 基于 sin） | 高 | ✅ 已完成 |
| 61 | **exp / log / pow** | `func exp(x: Float64): Float64` 等 | exp: 20 项泰勒；log: 40 项 atanh 级数；pow = exp(exp·log(base)) | 高 | ✅ 已完成 |
| 62 | **数学常数** | `PI`, `E`, `TAU`, `INF`, `NAN`, `NEG_INF` | 编译期识别为内置标识符，直接嵌入 `f64.const` | 低 | ✅ 已完成 |

**实现要点**：
- WASM 原生指令函数（`sqrt` 等）在 `compile_expr` 中直接识别并发射对应指令，零额外运行时开销
- 泰勒级数函数（`sin`/`cos`/`tan`/`exp`/`log`/`pow`）作为 6 个运行时函数注册，共用 `(f64) → f64` 和 `(f64, f64) → f64` 类型签名
- 数学常数（`PI`/`E`/`TAU`/`INF`/`NAN`/`NEG_INF`）在 `Expr::Var` 中识别，仅在用户未定义同名局部变量时生效
- 所有 math 内置函数在类型推断中正确返回 `Float64`，确保 `let x = sqrt(16.0)` 推导正确
- 当用户自定义同名函数（如 `func abs(x: Int64)`）时，编译器优先使用用户函数，不冲突
- 示例：`examples/std_math.cj` 覆盖全部功能验证

---

### Phase 7.4：std.convert 格式化（1 周）

| # | 特性 | 仓颉原型 | 说明 | 复杂度 |
|---|------|---------|------|--------|
| 63 | **Formattable 接口** | `interface Formattable { func format(fmt: String): String }` | 定义统一格式化接口 | 中 |
| 64 | **Int64.format(spec)** | `func format(spec: String): String` | 支持宽度、对齐、进制（`"10x"`, `"#08b"`, `"+5"` 等） | 高 |
| 65 | **Float64.format(spec)** | `func format(spec: String): String` | 支持精度、科学计数法（`"10.2f"`, `"e"`, `"g"` 等） | 高 |
| 66 | **字符串插值格式化** | `"${x.format("10.2f")}"` | 在插值中调用 `.format()` 方法 | 中 |

**实现要点**：
- 格式化规范：`[flags][width][.precision][specifier]`
  - flags: `-`(左对齐), `+`(显示正号), `#`(进制前缀), `0`(零填充)
  - specifier: `b`(二进制), `o`(八进制), `x/X`(十六进制), `e/E`(科学), `g/G`(通用)
- 先实现最常用子集（宽度 + 精度 + `x`/`b`/`o` 进制），后续扩展完整格式规范

---

### Phase 7.5：std.collection 集合类型（2-3 周）

| # | 特性 | 仓颉原型 | 堆布局 | 复杂度 |
|---|------|---------|--------|--------|
| 67 | **ArrayList\<T\>** | `class ArrayList<T>` | `[len:i32][cap:i32][data_ptr:i32]`，data 指向 `T[]` 堆块 | 高 |
| 68 | **HashMap\<K,V\>** | `class HashMap<K,V> where K <: Hashable & Equatable<K>` | 开放寻址哈希表：`[size:i32][cap:i32][buckets_ptr:i32]` | 高 |
| 69 | **HashSet\<T\>** | `class HashSet<T> where T <: Hashable & Equatable<T>` | 基于 HashMap\<T, Unit\> 实现 | 中 |
| 70 | **ArrayStack\<T\>** | `class ArrayStack<T>` | 基于 ArrayList 实现 push/pop/peek | 中 |
| 71 | **LinkedList\<T\>** | `class LinkedList<T>` | 双向链表：节点 `[prev:i32][next:i32][value:T]` | 高 |
| 72 | **Iterable / Iterator 接口** | `interface Iterable<T> { func iterator(): Iterator<T> }` | 集合的迭代器协议 | 高 |
| 73 | **集合高阶函数** | `map`, `filter`, `fold`, `forEach`, `any`, `all`, `count` 等 | 接受 Lambda 参数的迭代操作 | 高 |

**实现要点**：
- `ArrayList` 是核心，HashMap/HashSet/ArrayStack 都基于它
- 需要支持动态扩容（grow：分配新块 → 复制 → 释放旧块），依赖已有的 `__alloc`/`__free`
- `Hashable` 接口需要为 Int64/String 等提供内建 `hashCode()` 方法
- 优先实现 ArrayList → ArrayStack → HashMap → HashSet 的顺序
- 迭代器和高阶函数需要 Lambda/闭包的完整支持（Phase 5 已完成）

---

### Phase 7.6：WASI 系统调用扩展（1-2 周）

| # | 特性 | WASI 函数 | 仓颉 API | 复杂度 |
|---|------|----------|---------|--------|
| 74 | **fd_read** | `wasi_snapshot_preview1.fd_read` | `readln(): String`、`std.io.InputStream` 基础 | 中 |
| 75 | **fd_close** | `wasi_snapshot_preview1.fd_close` | 文件关闭 | 低 |
| 76 | **args_get / args_sizes_get** | `wasi_snapshot_preview1.args_get` | `std.env.getArgs(): Array<String>` | 中 |
| 77 | **clock_time_get** | `wasi_snapshot_preview1.clock_time_get` | `std.time.DateTime.now()` 基础 | 中 |
| 78 | **random_get** | `wasi_snapshot_preview1.random_get` | `std.random.Random` 基础 | 低 |
| 79 | **environ_get / environ_sizes_get** | `wasi_snapshot_preview1.environ_get` | `std.env.getEnv(key: String): Option<String>` | 中 |
| 80 | **fd_prestat_get / path_open / fd_seek** | 多个 WASI 函数 | `std.fs.File` 基础 | 高 |
| 81 | **proc_exit** | `wasi_snapshot_preview1.proc_exit` | `std.env.exit(code: Int64)` | 低 |

**实现要点**：
- 每个 WASI 函数在 ImportSection 中注册，分配类型签名和函数索引
- `args_get`：先调用 `args_sizes_get` 获取参数数量和缓冲区大小，再分配堆内存调用 `args_get`
- `clock_time_get`：返回纳秒时间戳（i64），需要包装为 `Duration` 或 `DateTime`
- `random_get`：填充指定长度的随机字节到内存缓冲区
- 文件系统操作较复杂，作为可选项放在最后

---

### Phase 7.7：std.time / std.random / std.env（1 周）

| # | 特性 | 仓颉原型 | 说明 | 复杂度 |
|---|------|---------|------|--------|
| 82 | **Duration** | `struct Duration { ... }` | 时间间隔，内部存储纳秒 (i64) | 中 |
| 83 | **DateTime.now()** | `static func now(): DateTime` | 基于 WASI `clock_time_get` | 中 |
| 84 | **Random** | `class Random { func nextInt64(): Int64; func nextFloat64(): Float64 }` | 基于 WASI `random_get` + 线性同余生成器 | 中 |
| 85 | **getArgs()** | `func getArgs(): Array<String>` | 基于 WASI `args_get` 返回命令行参数数组 | 中 |
| 86 | **getEnv()** | `func getEnv(key: String): Option<String>` | 基于 WASI `environ_get` 查找环境变量 | 中 |
| 87 | **exit()** | `func exit(code: Int64): Nothing` | 基于 WASI `proc_exit` | 低 |

---

### Phase 7.8：std.sort / std.unicode（可选，1 周）

| # | 特性 | 仓颉原型 | 说明 | 复杂度 |
|---|------|---------|------|--------|
| 88 | **sort (Array)** | `func sort<T>(arr: Array<T>) where T <: Comparable<T>` | 快速排序/归并排序运行时 | 高 |
| 89 | **String.toArray() / String.split()** | `func toArray(): Array<Rune>`, `func split(sep: String): Array<String>` | 字符串操作 | 中 |
| 90 | **String.contains() / indexOf() / replace()** | 常见字符串搜索/替换 | 逐字节比较实现 | 中 |
| 91 | **Rune 属性** | `func isDigit(): Bool`, `func isLetter(): Bool` 等 | Unicode 字符分类查表 | 中 |

---

### 不实现的包（WASM 环境不适用）

| 包 | 原因 |
|---|------|
| **std.net** | WASM 沙箱无网络访问能力 |
| **std.process** | WASM 无法创建子进程 |
| **std.sync** | WASM 单线程，无并发原语（spawn/synchronized） |
| **std.reflect** | 运行时反射需要元数据，WASM 不保留类型信息 |
| **std.posix** | POSIX 系统调用不适用于 WASM |
| **std.database.sql** | 需要原生数据库驱动 |
| **std.crypto.\*** | 需要大量原生代码（可后续用纯算法实现） |
| **std.collection.concurrent** | 依赖 std.sync 多线程 |
| **std.ast** | 编译器自身功能，非运行时需要 |
| **std.unittest** | 测试框架，需要宏支持 |
| **std.objectpool** | 依赖并发 |

---

### 实施优先级与路线图

```
Phase 7.1  std.core 核心函数      ██████████  100% ✅ 已完成（print/println/eprint/eprintln/readln/插值/toString/min/max）
Phase 7.2  std.core 类型方法      ░░░░░░░░░░  待实现（toString/toInt/abs/compareTo）
Phase 7.3  std.math 数学函数      ██████████  100% ✅ 已完成（WASM 原生指令 + 泰勒级数 + 常数）
Phase 7.4  std.convert 格式化     ░░░░░░░░░░  待实现（format spec 解析）
Phase 7.5  std.collection 集合    ░░░░░░░░░░  待实现（ArrayList → HashMap）
Phase 7.6  WASI 系统调用扩展      ███░░░░░░░  30%（fd_write + fd_read 已完成）
Phase 7.7  std.time/random/env    ░░░░░░░░░░  待实现（依赖 Phase 7.6）
Phase 7.8  std.sort/unicode       ░░░░░░░░░░  可选
```

**推荐实施顺序**：7.1 → 7.3 → 7.2 → 7.6 → 7.7 → 7.4 → 7.5 → 7.8

理由：
1. **7.1 → 7.3**：数学函数大部分直接映射 WASM 指令，投入产出比最高
2. **7.2**：类型方法为后续集合类型提供基础（`toString`、`Comparable`）
3. **7.6 → 7.7**：WASI 扩展为 `readln`、`DateTime`、`Random` 等提供底层支持
4. **7.4**：格式化依赖类型方法
5. **7.5**：集合类型最复杂，但也是应用价值最高的部分
6. **7.8**：可选功能，按需实现

---

## Phase 8：内存管理升级 ✅ 已完成

**目标**：从 bump allocator 升级到可回收内存的方案。

| # | 特性 | spec 位置 | 说明 | 状态 |
|---|------|-----------|------|------|
| 54 | **引用计数 (RC/ARC)** | 12.2 | 对象头加引用计数字段，赋值/离开作用域时 inc/dec，归零释放 | ✅ 已完成 |
| 55 | **垃圾回收 (Mark-Sweep)** | 12.2 | 堆扫描、检测引用计数为零的对象、回收不可达内存 | ✅ 已完成 |
| 56 | **手动管理 (malloc/free)** | 12.2 | Free List Allocator 替代 bump allocator，支持内存复用 | ✅ 已完成 |

**实现要点**：

1. **Free List Allocator (`__alloc`/`__free`)**：
   - 所有堆对象带 8 字节头部：`[block_size: i32][refcount: i32][user_data...]`
   - `__alloc(size)` 优先从空闲链表分配，无可用块时 bump 分配
   - `__free(ptr)` 将块加入空闲链表头部供后续复用
   - 分配对齐到 8 字节
   - Global 0: heap_ptr, Global 1: free_list_head

2. **引用计数 (RC)**：
   - `__rc_inc(ptr)` 递增 `mem[ptr-4]`（仅堆指针有效）
   - `__rc_dec(ptr)` 递减，归零时自动调用 `__free`
   - 编译器自动在 `var` 赋值时对旧值 `rc_dec`
   - 函数退出时对所有堆类型局部变量 `rc_dec`（返回值除外）
   - 安全检查：null 和数据段指针不执行 RC 操作

3. **垃圾回收 (Mark-Sweep GC)**：
   - `__gc_collect()` 从 heap_start 扫描到 heap_ptr
   - 按 block_size 跳转遍历所有块
   - refcount == 0 的块被释放回空闲链表
   - 返回回收的总字节数
   - 安全保护：block_size <= 0 时中止扫描

4. **所有内存管理函数均已导出**，可从宿主环境调用。

5. **代码位置**：`src/memory.rs`（WASM 函数构建器），`src/codegen/mod.rs`（集成）。

---

## Phase 9：其他补充特性（穿插进行） ✅ 已完成

这些特性相对独立，可在各 Phase 间穿插完成：

| # | 特性 | spec 位置 | 说明 | 复杂度 | 状态 |
|---|------|-----------|------|--------|------|
| 57 | **Slice\<T\>** | 1.2 | 动态切片，引用数组子区间 `[ptr, len]`，堆布局 `[ptr:i32][len:i32]` | 中 | ✅ |
| 58 | **Map 字面量** | 2.3 | `Map<K,V>` 类型 + `Map { key => val }` 字面量语法，线性键值对堆布局 | 中 | ✅ |
| 59 | **类型修饰符 mut/ref/?/!** | 1.3 | `mut T`/`ref T` 关键字、`T?` → `Option<T>` 语法糖、`T!` 非空断言 | 高 | ✅ |
| 60 | **尾递归优化** | 5.1 | 检测尾调用位置（`return f(args)` 或末尾 `f(args)`），转为 `loop` + 参数重赋值 + `continue` | 中 | ✅ |
| 61 | **优化器扩展** | 15 | 死代码消除（return/break/continue 后不可达语句截断）、函数内联基础 | 中 | ✅ |

---

## 与 spec 章节对应关系

| spec 章节 | 已实现要点 | 未实现要点 |
|----------|------------|------------|
| 1 类型系统 | Int32/64, Float32/64, Float16, Bool, Unit, Rune, String, Array, Option, Result, Struct, Enum, **Function 类型**, **Slice<T>**, **Map<K,V>**, **mut/ref/?/!**, IntNative, UIntNative, Nothing, VArray, This | ✅ 全部完成 |
| 2 字面量 | 十进制/十六进制/八进制/二进制整数, 浮点, 数字分隔符, 科学计数法, 字符串(基本/转义/多行/原始/插值), 数组, **元组字面量**, **Map 字面量** | ✅ 全部完成 |
| 3 表达式 | 算术, 比较, 逻辑, 位运算, 赋值(**含 **=,&&=,\|\|=,&=,\|=,^=,<<=,>>=**), 幂运算, 类型转换, if 表达式, 块表达式, 方法调用, 枚举变体, 范围, ??, **++/--**, **\|>/~>**, **Slice 表达式** | 三元运算符 |
| 4 语句 | let/var, 类型注解, if/while/for/loop/match, return/break/continue, 解构绑定, if-let, while-let | ✅ 全部完成 |
| 5 函数 | 基本函数, 参数, 返回(`: Type`), 递归, 默认参数, 可变参数, Lambda, 函数重载, **泛型函数(约束)**, **闭包/Lambda编译**, **尾递归优化**, **main() 入口** | ✅ 全部完成 |
| 6 结构体与类 | 结构体(全部完成), 类(codegen: init/~init/<:继承/vtable/override/super/prop/abstract/sealed/open/static) | 运行时虚分派(call_indirect) |
| 7 枚举与匹配 | 简单枚举, 关联值, 枚举方法, 全部模式匹配, **泛型枚举(单态化)** | ✅ 全部完成 |
| 8 泛型 | 泛型函数/结构体(单态化), **类型约束 `<T:Bound>`**, **多重约束 `<T:A&B>`**, **where 子句**, **泛型类**, **泛型枚举**, **泛型特化**, **约束检查** | ✅ 全部完成 |
| 9 接口/Trait | 解析+codegen: **接口定义**, **默认实现**, **<: 实现**, **extend**, **接口继承(<:)**, **关联类型** | ✅ 全部完成 |
| 10 包系统 | package, import(点分路径), 通配符, public, protected, internal, private, **多文件编译**, **import 自动解析**, **-o 选项** | ✅ 全部完成 |
| 11 错误处理 | try-catch, throw, Result, ? 运算符, **finally**, **Error 基类(<:)**, **自定义错误继承** | ✅ 全部完成 |
| 12 内存管理 | **Free List Allocator**, **引用计数(RC)**, **Mark-Sweep GC**, **__alloc/__free/__rc_inc/__rc_dec/__gc_collect** | ✅ 全部完成 |
| 13 WASM 互操作 | @import, @export, foreign func, **WASI fd_write**, **print/println 多类型** | WASI(fd_read/fd_close/args_get/clock_time_get/random_get/environ_get/proc_exit) |
| 14 标准库 | 内置数学(min/max/abs), **print/println(所有基础类型)**, **字符串插值**, **eprint/eprintln**, **类型转换运行时** | std.core 类型方法, std.math, std.convert, std.collection, std.time, std.random, std.env, std.sort |

---

## Phase 10：cjc release/1.0 语法对齐 ✅ 已完成

**目标**：将 cjwasm 的语法规范与 cjc 编译器 release/1.0 分支严格对齐，确保源代码兼容。

**参考来源**：`https://gitcode.com/Cangjie/cangjie_compiler.git` release/1.0 分支

### 变更总览

| 类别 | 旧语法 (cjwasm) | 新语法 (cjc) | 影响范围 |
|------|-----------------|-------------|---------|
| 包声明 | `module math` | `package math` | lexer, parser, AST, codegen, pipeline |
| 导入 | `import foo from bar.baz` | `import bar.baz.foo` | parser |
| 类继承 | `class Dog extends Animal` | `class Dog <: Animal` | parser |
| 接口实现 | `class Foo implements Bar` | `class Foo <: Bar` | parser |
| 析构函数 | `deinit { }` | `~init { }` | lexer, parser |
| 外部函数 | `extern func` | `foreign func` | lexer, parser |
| 字符类型 | `Char` | `Rune` | lexer, AST, codegen, monomorph |
| 异常声明 | `func f() throws Error` | （已移除） | lexer, parser, AST |
| 无符号右移 | `>>>` | （已移除） | lexer, AST, codegen, optimizer |
| 默认可见性 | `private` | `internal` | AST |

### 新增 Token/关键字

`protected`, `const`, `static`, `redef`, `operator`, `unsafe`, `do`, `is`, `case`, `where`, `type`, `main`, `spawn`, `synchronized`, `macro`, `quote`, `inout`, `with`, `foreign`

### 新增类型

`IntNative`, `UIntNative`, `Float16`, `Nothing`, `VArray<T, N>`, `This`

### 新增运算符

`++`, `--`, `|>`, `~>`, `**=`, `&&=`, `||=`, `&=`, `|=`, `^=`, `<<=`, `>>=`, `#`, `@!`, `$`

### 其他改进

- `main()` 入口函数无需 `func` 关键字
- 类体内支持 `open`/`static` 方法修饰符
- 上下文相关关键字（`main`/`where`/`type`/`is`/`case`/`with`）可作为标识符使用
- `protected` 可见性修饰符（子类可见）

### 影响的文件

| 文件 | 变更类型 |
|------|---------|
| `src/lexer/mod.rs` | Token 定义：新增/移除/重命名关键字、类型、运算符 |
| `src/ast/mod.rs` | AST 节点：类型重命名、移除 UShr、新增 Protected、package_name |
| `src/parser/mod.rs` | 解析逻辑：import/class/interface/main/析构/extern 语法更新 |
| `src/codegen/mod.rs` | 代码生成：适配 Rune/package_name/移除 UShr |
| `src/monomorph/mod.rs` | 单态化：Rune/package_name |
| `src/optimizer/mod.rs` | 优化器：移除 UShr 常量折叠 |
| `src/pipeline.rs` | 编译管线：package_name |
| `tests/compile_test.rs` | 集成测试：全部更新为新语法 |
| `examples/*.cj` | 示例文件：全部更新为新语法 |

---

*文档版本: 10.3.0*
*最后更新: 2026-02-14（Phase 7.1 全部完成：新增 print\<T\> ToString 分发、readln()、WASI fd_read 导入；Phase 7.3 全部完成）*
