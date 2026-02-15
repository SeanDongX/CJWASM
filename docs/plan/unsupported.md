# 未支持特性执行计划

基于 [Cangjie-TPC/scientific](https://gitcode.com/Cangjie-TPC/scientific.git)、[Cangjie-TPC/matrix4cj](https://gitcode.com/Cangjie-TPC/matrix4cj.git)、[Cangjie-TPC/quartz4cj](https://gitcode.com/Cangjie-TPC/quartz4cj.git) 和 [Cangjie-TPC/activemq4cj](https://gitcode.com/Cangjie-TPC/activemq4cj.git) 四个仓颉三方库的实际使用场景，整理 cjwasm 尚未支持的语言特性，并按优先级和依赖关系制定实施路线。

---

## 特性来源标记

| 标记 | 含义 |
|------|------|
| `[S]` | scientific 库使用 (数值计算) |
| `[M]` | matrix4cj 库使用 (矩阵运算) |
| `[Q]` | quartz4cj 库使用 (任务调度) |
| `[A]` | activemq4cj 库使用 (消息队列) |
| `[S+M]` | scientific + matrix4cj 使用 |
| `[ALL]` | 所有库均使用 |

---

## 一、特性总览

从四个库的编译失败和适配过程中，共识别出 **55 项**未支持特性和 **6 项** Bug（✅ 已全部修复）：

### A. 解析器缺失（Parser 层）

| # | 特性 | 来源 | 重要度 | 复杂度 | 阶段 |
|---|------|------|--------|--------|------|
| 1 | `else if` 链式语法 | [ALL] | ★★★★★ | 低 | P1 |
| 2 | `let _ = expr` 通配符赋值 | [Q] | ★★★★☆ | 低 | P1 |
| 3 | `?T` Option 类型 (`?Int64`, `?String`, `?Array<Byte>`) | [Q+A] | ★★★★★ | 高 | P2 |
| 4 | `type` 别名 (`type Func = (T) -> U`) | [Q+A] | ★★★★☆ | 中 | P2 |
| 5 | 函数类型参数 `(Int64) -> Int64` | [Q+A] | ★★★★★ | 高 | P2 |
| 6 | `spawn` 协程 | [Q+A] | ★★★★★ | 极高 | P5 |
| 7 | `synchronized` 同步块 | [Q+A] | ★★★★★ | 极高 | P5 |
| 8 | `for-in` 带步长 `for(i in 0..=10 : 2)` | [Q] | ★★★☆☆ | 低 | P2 |
| 9 | 抽象方法（无 body 的 `func` 声明） | [Q+A] | ★★★★☆ | 低 | P2 |
| 10 | 命名参数 `name!: Type = default` | [ALL] | ★★★★☆ | 中 | P2 |
| 11 | `import ... as` 别名 | [S+M+A] | ★★★☆☆ | 低 | P2 |
| 41 | `package` 包声明 | [A] | ★★★★★ | 中 | P2 |
| 42 | `internal` / `public` import 可见性 | [A] | ★★★★☆ | 低 | P2 |
| 43 | 多目标 import `import pkg.{A, B, C}` | [A] | ★★★☆☆ | 低 | P2 |
| 44 | `mut prop` 可变属性 (getter + setter) | [A] | ★★★★★ | 中 | P2 |
| 45 | `static init()` 静态初始化块 | [A] | ★★★★☆ | 中 | P3 |
| 46 | `static var` 可变静态字段 | [A] | ★★★★☆ | 中 | P2 |
| 47 | `override` 方法重写关键字 | [A] | ★★★★☆ | 低 | P2 |
| 48 | `@OverflowWrapping` 等注解 | [A] | ★★☆☆☆ | 中 | P5 |

### B. 代码生成缺失（Codegen 层）

| # | 特性 | 来源 | 重要度 | 复杂度 | 阶段 |
|---|------|------|--------|--------|------|
| 12 | void 函数调用语句（drop 修复） | [ALL] | ★★★★★ | 低 | P1 |
| 13 | 动态 Array 构造 `Array<T>(size, init)` | [ALL] | ★★★★☆ | 中 | P2 |
| 14 | Array 实例方法 (`clone/flatten/slice/copyTo/isEmpty`) | [S+M+A] | ★★★★☆ | 中 | P2 |
| 15 | 运算符重载 `operator func +/-/*/[]` | [M+Q+A] | ★★★★★ | 高 | P3 |
| 16 | 嵌套数组 `Array<Array<T>>` | [M] | ★★★☆☆ | 中 | P3 |
| 17 | `extend` 内建类型实现接口 | [ALL] | ★★★★☆ | 高 | P3 |
| 18 | 复杂泛型约束方法分派 | [S+A] | ★★★☆☆ | 高 | P4 |
| 19 | Range 属性/方法 (`.step/.start/.iterator()`) | [M+Q] | ★★★☆☆ | 中 | P4 |
| 20 | `for (i in range)` Range 对象迭代 | [M] | ★★★☆☆ | 中 | P4 |
| 21 | `static` 类成员方法 (`static func`) | [Q+A] | ★★★★★ | 中 | P2 |
| 22 | `is` / `as` 运行时类型检查与转换 | [Q+A] | ★★★★★ | 高 | P3 |
| 23 | `if let Some(x) <- y` 模式匹配 | [Q+A] | ★★★★★ | 高 | P3 |
| 24 | `??` 空合并运算符（含控制流 `?? return/break/continue`） | [Q+A] | ★★★★★ | 高 | P3 |
| 25 | `match` type pattern (`case x: SomeType =>`) | [Q+A] | ★★★★☆ | 高 | P3 |
| 26 | `match` `where` 守卫 | [Q] | ★★★☆☆ | 中 | P3 |
| 27 | 元组类型与元组解构 (`(Int64, Int64)`, `for((k,v) in map)`) | [Q+A] | ★★★★☆ | 中 | P3 |
| 28 | Lambda/闭包表达式 (`{ x => expr }`) | [Q+A] | ★★★★★ | 高 | P3 |
| 29 | 方法重载（同名不同参数） | [Q+A] | ★★★★★ | 中 | P3 |
| 49 | `Any` 类型 (动态类型、类型擦除) | [A] | ★★★★★ | 极高 | P4 |
| 50 | `try-finally`（无 catch 的 finally 块） | [A] | ★★★★☆ | 中 | P2 |
| 51 | 完整数值类型 (`Int8/Int16/Int32/UInt16/UInt32/Byte/Float32`) | [A] | ★★★★☆ | 中 | P2 |

### C. 标准库缺失

| # | 特性 | 来源 | 重要度 | 复杂度 | 阶段 |
|---|------|------|--------|--------|------|
| 30 | `std.collection.*` (HashMap, ArrayList, HashSet, TreeMap) | [Q+A] | ★★★★★ | 极高 | P4 |
| 31 | `std.time.*` (DateTime, TimeZone, Duration, Calendars) | [Q+A] | ★★★★★ | 极高 | P5 |
| 32 | `std.sync.*` (Monitor, AtomicBool, AtomicInt64) | [Q+A] | ★★★★★ | 极高 | P5 |
| 33 | `std.log.*` (Logger, LoggerFactory) | [Q+A] | ★★★★☆ | 中 | P5 |
| 34 | `std.regex.*` (Regex) | [Q] | ★★★☆☆ | 高 | P5 |
| 35 | `std.unicode.*` (Rune, toRuneArray) | [Q+A] | ★★★☆☆ | 中 | P5 |
| 36 | `std.fs.*` (File, Path) | [Q] | ★★☆☆☆ | 中 | P5 |
| 37 | `std.convert.Parsable` (Int32.parse, Bool.parse) | [Q] | ★★★☆☆ | 中 | P5 |
| 38 | `std.unittest` 宏 (`@Test/@TestCase`) | [S] | ★★☆☆☆ | 高 | P5 |
| 39 | `std.argopt` / `std.convert` 完整模块 | [S] | ★★☆☆☆ | 中 | P5 |
| 40 | String 方法 (`trim/split/indexOf/startsWith/endsWith/contains/replace`) | [Q+A] | ★★★★★ | 中 | P2 |
| 52 | `std.collection.concurrent.*` (ConcurrentHashMap, ArrayBlockingQueue) | [A] | ★★★★★ | 极高 | P5 |
| 53 | `std.sync.*` 扩展 (Mutex, ReentrantMutex, ReentrantReadWriteMutex, Condition) | [A] | ★★★★★ | 极高 | P5 |
| 54 | `std.net.*` (ClientTcpEndpoint, TcpSocket, TLS) | [A] | ★★★★★ | 极高 | P6 |
| 55 | `ByteBuffer` 字节缓冲区操作 | [A] | ★★★★★ | 高 | P5 |
| 56 | `ThreadPool` / `ThreadPoolFactory` / `ThreadLocal` | [A] | ★★★★★ | 极高 | P5 |
| 57 | `std.time.Timer` 定时器 | [A] | ★★★★☆ | 高 | P5 |
| 58 | `std.reflect.TypeInfo` 反射 | [A] | ★★★☆☆ | 极高 | P6 |
| 59 | `stdx.encoding.url.URL` URL解析 | [A] | ★★★★☆ | 中 | P5 |
| 60 | `stdx.serialization.*` 序列化框架 | [A] | ★★★☆☆ | 高 | P6 |

### D. 构建系统缺失

| # | 特性 | 来源 | 重要度 | 复杂度 | 阶段 |
|---|------|------|--------|--------|------|
| 61 | 外部 git 依赖 (`dependencies` in cjpm.toml) | [A] | ★★★★★ | 高 | P4 |
| 62 | 多模块/多包编译 (`package` + 跨包 import) | [A] | ★★★★★ | 高 | P4 |
| 63 | `stdx` 扩展库路径 (`CANGJIE_STDX_PATH`) | [A] | ★★★☆☆ | 中 | P5 |

### D. 已发现的 Bug（已实现但行为不正确） ✅ 全部已修复

| # | Bug 描述 | 影响 | 严重度 | 状态 |
|---|----------|------|--------|------|
| B1 | **enum match 变体区分失败**：所有变体匹配第一个 case | 所有使用 enum match 的代码 | ★★★★★ | ✅ 已修复 |
| B2 | **prop getter body 编译不正确**：返回值错误 | 所有使用 prop 的代码 | ★★★★☆ | ✅ 已修复 |
| B3 | **super() 构造调用参数丢失**：父类字段未初始化 | 类继承代码 | ★★★★☆ | ✅ 已修复 |
| B4 | **String `+` 拼接生成空字符串** | 字符串拼接代码 | ★★★★☆ | ✅ 已修复 |
| B5 | **方法链式调用返回类型不追踪**：`a.foo().bar()` 中 `bar()` 找不到 | Builder 模式代码 | ★★★★☆ | ✅ 已修复 |
| B6 | **try-catch codegen 生成无效 WASM**：i64/i32 类型错误 | 异常处理代码 | ★★★★★ | ✅ 已修复 |

**修复详情（2026-02-15）**：
- **B1**：`Pattern::Binding` 在 match 中检查是否为未限定的枚举变体名（如 `case RED`），自动转为变体比较
- **B2**：实现隐式 `this` 字段访问（`Expr::Var`/`Stmt::Assign` 回退到 `this.field`），prop getter 调用（`Expr::Field` 检查 `__get_` 方法），prop 返回类型推断
- **B3**：为每个类 init 额外生成 `__ClassName_init_body(this, params...)` 函数，super() 调用使用 init_body（不分配新对象）
- **B4**：`BinOp::Add` 检测 String 类型时调用 `__str_concat` 而非 `I32Add`，新增 `emit_to_string` 辅助方法
- **B5**：`get_object_type` 添加 `Expr::MethodCall` 分支，通过 `infer_ast_type_with_locals` 追踪方法返回类型
- **B6**：TryBlock 表达式模式：try/catch 最后的表达式值存入 `__try_result` 临时变量，修改 `expr_produces_value` 正确判断

### E. Float64 数组支持 ✅ 已完成

在之前的会话中已修复：
- 数组字面量创建：Float64 元素使用 `F64Store`
- 数组索引读取：Float64 数组使用 `F64Load`
- 数组赋值：Float64 值使用 `F64Store`，并修正 `+4` 偏移
- `Array.size` 属性支持数组类型（原仅支持 String）

---

## 二、分阶段实施计划

### P1：关键 Bug 修复 + 基础语法（预计 3-5 天）

解决直接导致编译/运行失败的问题。

#### 1.1 `else if` 链式语法 [ALL]

**现状**：`else if (cond) { ... }` 报错 "期望: LBrace"，必须写成 `else { if (cond) { ... } }`。

**影响范围**：几乎所有实际仓颉代码都使用 `else if`，这是最高频的语法缺失。

**实现方案**：
```
parser/mod.rs → parse_if_stmt()
  当 `else` 后面遇到 `Token::If` 时，递归调用 parse_if_stmt()
  将结果包装为单语句的 else 分支
```

**预计工作量**：0.5 天

---

#### 1.2 void 函数调用 Drop 修复 [ALL]

**现状**：返回 Unit（无返回值）的函数作为语句调用时，`expr_produces_value()` 返回 true，导致对空栈执行 `drop` 指令，WASM 校验失败。

**实现方案**：
```rust
Expr::Call { name, .. } => {
    match self.func_return_types.get(name) {
        Some(Type::Unit) | None => false,
        _ => true,
    }
}
```

**预计工作量**：0.5 天

---

#### 1.3 `let _ = expr` 通配符赋值 [Q]

**现状**：`let _ = someFunc()` 报错 "意外的 token: Underscore, 期望: 变量名或类型名"。

**使用场景**（quartz4cj）：
```cangjie
let _ = trig.trigger()     // 丢弃返回值
for (_ in 0..5) { ... }   // 忽略循环变量
```

**实现方案**：解析器中识别 `_` 为特殊通配符变量名，codegen 对通配符赋值生成 `drop` 指令。

**预计工作量**：0.5 天

---

#### 1.4-1.9 Bug 修复 [B1-B6] ✅ 全部已完成

所有 6 个 Bug 已于 2026-02-15 修复并通过测试（166 个现有测试无回归）。详见 D 节的修复详情。

---

### P2：核心语法扩展（预计 3-4 周）

补全三个库最常用的语法特性。

#### 2.1 `?T` Option 类型 + `Some` / `None` [Q]

**现状**：`?Int64` 报错 "意外的 token: Question, 期望: 类型"。

**使用场景**（quartz4cj，高频使用）：
```cangjie
func getEndTime(): ?DateTime                      // 返回可空值
private var cfgOpt: ?PropertiesParser = None       // 可空字段
if (let Some(nd) <- nextTime) { ... }             // if let 解包
let tw = twOpt ?? return;                         // ?? 空合并
let date = date0 ?? DateTime.now()                // ?? 默认值
jobClass.getOrThrow()                             // 强制解包
```

**实现方案**：

1. **类型系统**：`?T` = `Option<T>`，内部表示为 tagged union `(tag: i32, value: T)`
   - 内存布局：`[tag:i32][padding:...][value:T_size]`
   - `None` → tag=0, `Some(v)` → tag=1, value=v
2. **解析器**：`?` 前缀识别为 Option 类型包装
3. **AST**：新增 `Expr::Some(expr)`, `Expr::None`, `Expr::IfLet`, `Expr::NullCoalesce`
4. **Codegen**：
   - `Some(x)` → 分配 Option 结构，设 tag=1, 写入 x
   - `None` → 分配 Option 结构，设 tag=0
   - `if let Some(v) <- opt` → 检查 tag==1，取出 value 绑定到 v
   - `opt ?? default` → 检查 tag==0，为 0 则取 default

**预计工作量**：5-7 天

**涉及文件**：`src/parser/mod.rs`, `src/ast/mod.rs`, `src/codegen/mod.rs`

**依赖项**：是 #23, #24 的前置条件

---

#### 2.2 `type` 别名 [Q]

**现状**：`type JobFunction = (Int64) -> Int64` 报错 "意外的 token: TypeAlias"。

**使用场景**：
```cangjie
public type JobFunction = (JobExecutionContext) -> Unit
public type Supplier = () -> Any
public type Runnable = () -> Unit
```

**实现方案**：
1. 解析 `type Name = Type` 为类型别名定义
2. AST 新增 `TypeAlias` 节点
3. 在类型解析阶段将别名展开为实际类型

**预计工作量**：1-2 天

---

#### 2.3 函数类型参数 + Lambda/闭包 [Q]

**现状**：`func apply(f: (Int64) -> Int64, x: Int64)` 中 `(Int64) -> Int64` 类型报错 "意外的 token: LParen"。

**使用场景**（quartz4cj 大量使用）：
```cangjie
// 函数类型参数
public func ifPresent(f: (T) -> Unit): Unit { ... }

// Lambda 表达式
listeners.removeIf({ l => l.getName() == listenerName })
let creator = { => JobImplForJobFunction(jobFunc) }

// 集合操作
for (e in elements) { ... }
```

**实现方案**：
1. **解析器**：识别 `(T1, T2, ...) -> R` 为函数类型
2. **Lambda**：`{ args => body }` 解析为匿名函数，编译为 WASM `table.indirect_call`
3. **Codegen**：函数类型参数通过 WASM Table 间接调用

**预计工作量**：7-10 天

**涉及文件**：`src/parser/mod.rs`, `src/ast/mod.rs`, `src/codegen/mod.rs`

---

#### 2.4 `static` 类成员 [Q]

**现状**：`static func create(): Counter` 可解析，但 `Counter.create()` 调用时返回类型未追踪，导致后续方法调用崩溃 "方法未找到"。

**使用场景**：
```cangjie
class AndMatcher<T> {
    public static func and(left: Matcher<T>, right: Matcher<T>): AndMatcher<T> { ... }
}
public static func newTrigger(): TriggerBuilder<T> { ... }
private static let log = LoggerFactory.getLogger("org.quartz.RAMJobStore")
```

**实现方案**：
1. `static func` → 编译为全局函数 `ClassName__methodName`
2. `static let` → 编译为全局变量
3. `ClassName.method()` 调用 → 解析为全局函数调用
4. 修复返回类型追踪

**预计工作量**：3-4 天

---

#### 2.5 抽象方法（无 body 的 func 声明） [Q]

**现状**：`func sound(): Int64;` 在 class body 中报错 "期望: LBrace"，必须提供函数体。

**使用场景**：
```cangjie
public abstract class AbstractTrigger<T> <: OperableTrigger {
    public func getStartTime(): DateTime;     // 抽象方法
    public func setStartTime(startTime: DateTime): Unit;
}
```

**实现方案**：解析器中允许 class 方法以 `;` 结尾（当有 `abstract` 修饰符时）。

**预计工作量**：0.5 天

---

#### 2.6 `for-in` 带步长 [Q]

**现状**：`for (i in 0..=10 : 2)` 报错 "意外的 token: Colon, 期望: RParen"。

**使用场景**：
```cangjie
for (i in startAt..=stopAt : incr) { ... }   // cronexpression.cj 中的 cron 字段遍历
```

**实现方案**：在 for-in range 语法中支持可选的 `:step` 后缀，codegen 中生成对应步长的循环。

**预计工作量**：1 天

---

#### 2.7 动态 Array 构造 `Array<T>(size, init)` [ALL]

**现状**：仅支持字面量 `[1, 2, 3]`，不支持运行时大小的数组构造。

**使用场景**：
```cangjie
var u = Array<Float64>(N, { i => Float64(1) })       // scientific
_data = Array<Float64>(m * n, repeat: value)          // matrix4cj
let newBuf = Array<Rune>(newBufLen, repeat: r' ')     // quartz4cj
```

**预计工作量**：3-4 天

---

#### 2.8 Array 实例方法 [S+M]

**现状**：Array 仅支持 `arr[i]`（索引访问）、`arr.size`（长度）和 `sort(arr)`（全局排序）。

**需要新增**：`clone()`, `isEmpty()`, `flatten()`, `slice()`, `copyTo()`

**预计工作量**：3-4 天

---

#### 2.9 命名参数 `name!: Type = default` [ALL]

**使用场景**：
```cangjie
public init(m: Int64, n: Int64, value!: Float64 = 0.0)             // matrix4cj
JobDetailImpl(jobClass, key, description!: "", durability!: false)  // quartz4cj
public QzThread(name!: String = "")                                 // quartz4cj
```

**预计工作量**：3-4 天

---

#### 2.10 String 方法 [Q]

**现状**：cjwasm 仅支持 `String.size`、`println`、`${}`字符串插值。缺少大量常用方法。

**需要新增的方法**（quartz4cj 使用频率由高到低）：

| 方法 | 签名 | 使用频率 |
|------|------|----------|
| `trim()` | `func trim(): String` | ★★★★★ |
| `startsWith(s)` | `func startsWith(s: String): Bool` | ★★★★★ |
| `endsWith(s)` | `func endsWith(s: String): Bool` | ★★★★☆ |
| `contains(s)` | `func contains(s: String): Bool` | ★★★★★ |
| `indexOf(s)` | `func indexOf(s: String): ?Int64` | ★★★★★ |
| `indexOf(s, from)` | `func indexOf(s: String, from: Int64): ?Int64` | ★★★★☆ |
| `split(delim)` | `func split(delim: String): Array<String>` | ★★★★☆ |
| `replace(old, new)` | `func replace(old: String, new: String): String` | ★★★★☆ |
| `isEmpty()` | `func isEmpty(): Bool` | ★★★★☆ |
| `isBlank()` | `func isBlank(): Bool` | ★★★☆☆ |
| `toRuneArray()` | `func toRuneArray(): Array<Rune>` | ★★★☆☆ |

**实现方案**：在 `compile_builtin_method()` 中为 String 类型添加方法分发。

**预计工作量**：5-7 天

---

#### 2.11 `import ... as` 别名 [S+M+A]

**预计工作量**：1 天

---

#### 2.12 `package` 声明 + 多模块 import [A]

**现状**：cjwasm 仅支持单文件编译，不支持 `package` 声明和跨包 import。

**使用场景**（activemq4cj 所有文件均使用）：
```cangjie
package activemq4cj.cjms                               // 包声明
internal import std.collection.LinkedList               // 可见性修饰
internal import activemq4cj.cjms.Message as CJMSMessage // 别名
internal import activemq4cj.cjms.{Destination, Queue}   // 多目标
```

**实现方案**：
1. 解析 `package name.space` 声明
2. 支持 `import path.{A, B, C}` 多目标导入
3. 支持 `internal` / `public` import 可见性（可暂时忽略可见性差异）

**预计工作量**：5-7 天

---

#### 2.13 `mut prop` 可变属性 [A]

**现状**：`prop` 的 getter 可解析但有 Bug（B2），`mut prop` 的 setter 未支持。

**使用场景**：
```cangjie
// activemq4cj: 消息属性
mut prop text: ?String { get() { ... } set(value) { ... } }
mut prop exceptionListener: ?ExceptionListener
mut prop deliveryMode: DeliveryMode
mut prop disableMessageID: Bool
```

**实现方案**：扩展 prop 解析支持 `mut` 修饰符和 `set(value)` 块，codegen 生成对应的 setter 方法。

**预计工作量**：2-3 天（含修复 Bug B2）

---

#### 2.14 `override` 关键字 + `static var` [A]

**使用场景**：
```cangjie
public open override func hashCode(): Int64 { ... }    // override
private static var instanceCount: AtomicInt32           // static var
```

**预计工作量**：1-2 天

---

#### 2.15 `try-finally`（无 catch）[A]

**使用场景**：
```cangjie
try {
    // 临界区操作
} finally {
    mutex.readMutex.unlock()
}
```

**预计工作量**：1 天（扩展现有 try-catch 实现）

---

#### 2.16 完整数值类型 [A]

**现状**：cjwasm 仅支持 `Int64`、`Float64`、`Bool`。

**需要新增**：`Int8`, `Int16`, `Int32`, `UInt16`, `UInt32`, `Byte`, `Float32`, `Rune`

**使用场景**（activemq4cj 的 OpenWire 协议大量使用）：
```cangjie
func readInt8(): Int8
func writeInt32(value: Int32): Unit
func readUInt16(): UInt16
let QUEUE_TYPE: Byte = 0x01
```

**实现方案**：
- `Int32`/`Int16`/`Int8`/`Byte` → WASM `i32` + 截断/扩展
- `UInt16`/`UInt32` → WASM `i32` + 无符号扩展
- `Float32` → WASM `f32`

**预计工作量**：5-7 天

---

### P3：面向对象 + 类型系统扩展（预计 4-6 周）

quartz4cj 是一个完整的面向对象框架，需要丰富的 OOP 支持。

#### 3.1 运算符重载 `operator func` [M+Q]

**使用场景**：
```cangjie
// quartz4cj: enum 运算符
public operator func ==(other: TriggerState): Bool { ... }
public operator func <(o: T): Bool { ... }

// matrix4cj: 矩阵运算符
public operator func *(other: Matrix): Matrix { ... }
```

**预计工作量**：5-7 天

---

#### 3.2 `if let Some(x) <- y` 模式匹配 [Q]

**使用场景**（quartz4cj 高频使用）：
```cangjie
if (let Some(nd) <- nextTime) {
    difference = (nd - lastDate).toMilliseconds()
}
if (let Some(initException) <- initExceptionOpt) {
    throw initException
}
if (let Some(grpMap) <- grpMapOpt) {
    for (tw in grpMap.values()) { ... }
}
```

**实现方案**：
1. 解析 `if (let Some(name) <- expr)` 为 IfLet 语句
2. Codegen：对 expr（Option 类型）检查 tag，tag==1 时绑定 value 到变量 name

**预计工作量**：3-4 天

**依赖**：#2.1 Option 类型

---

#### 3.3 `??` 空合并运算符 [Q]

**使用场景**（quartz4cj 极高频使用）：
```cangjie
let tw = twOpt ?? return;                              // ?? + return
let nft = tw.trigger.getNextFireTime() ?? continue;    // ?? + continue
let tw = twOpt ?? break;                               // ?? + break
let date = date0 ?? DateTime.now()                     // ?? + 默认值
let obj = opt ?? throw SchedulerException("...")        // ?? + throw
scheduleBuilder = scheduleBuilder ?? SimpleScheduleBuilder.simpleSchedule()
```

**实现方案**：
1. 解析 `expr ?? fallback` 为二元运算
2. fallback 可以是：表达式、`return`、`break`、`continue`、`throw expr`
3. Codegen：检查 expr 的 Option tag，为 None 时执行 fallback

**预计工作量**：3-4 天

**依赖**：#2.1 Option 类型

---

#### 3.4 `is` / `as` 类型检查与转换 [Q]

**使用场景**：
```cangjie
@Assert(true, jobInst is Job)                        // 类型检查
let trig = (trig as T).getOrThrow()                  // 类型转换
match (obj) { case other: AndMatcher<T> => ... }     // match 类型匹配
```

**实现方案**：
1. `is` → 检查对象的 class_id 是否与目标类型匹配（含继承链）
2. `as` → 返回 `Option<T>`，根据 `is` 结果包装

**预计工作量**：4-5 天

---

#### 3.5 `match` type pattern + `where` 守卫 [Q]

**使用场景**：
```cangjie
match (obj) {
    case other: AndMatcher<T> =>                     // 类型模式
        return this.leftOperand == other.leftOperand
    case _ => false
}

match (types) {
    case v where v == SECOND => max = 60             // where 守卫
    case v where v == MINUTE => max = 60
    case _ => ()
}
```

**预计工作量**：3-4 天

---

#### 3.6 Lambda/闭包表达式 [Q]

**使用场景**：
```cangjie
listeners.removeIf({ l => l.getName() == listenerName })
let creator = { => JobImplForJobFunction(jobFunc) }
```

**注意**：与 #2.3 函数类型参数紧密相关。如果 #2.3 已实现函数类型，Lambda 需额外支持闭包变量捕获。

**预计工作量**：5-7 天（含闭包捕获）

---

#### 3.7 元组类型与解构 [Q]

**使用场景**：
```cangjie
func getValue(s: Array<Rune>, i: Int64): (Int64, Int64) {
    return (value, pos)
}
let vs = getValue(s, i)
val = vs[0]
i = vs[1]

match (map.firstEntry()) {
    case Some((k, v)) => k     // 元组解构
}
```

**预计工作量**：3-4 天

---

#### 3.8 方法重载 [Q]

**使用场景**：
```cangjie
func shutdown(): Unit { ... }
func shutdown(waitForJobsToComplete: Bool): Unit { ... }
func scheduleJob(jobDetail: JobDetail, trigger: Trigger): DateTime { ... }
func scheduleJob(trigger: Trigger): DateTime { ... }
```

**预计工作量**：3-4 天（需增强名字修饰机制）

---

#### 3.9 嵌套数组 `Array<Array<T>>` [M]

**预计工作量**：3-4 天

---

#### 3.10 `Any` 类型（动态类型、类型擦除）[A]

**现状**：cjwasm 无 `Any` 类型支持。

**使用场景**（activemq4cj 高频使用）：
```cangjie
// 动态类型 HashMap
properties: ?HashMap<String, Any>
func getObjectProperty(name: String): Any
func setObjectProperty(name: String, value: Any): Unit

// 类型断言
(v as String)
(v as Bool)
(v as Int64)

// 反射参数
func setProperties(target: Any, props: HashMap<String, Any>): Unit
func narrow(target: TypeInfo): Any
```

**实现方案**：
1. `Any` → 运行时 tagged union `(type_id: i32, data: *)`
2. 装箱/拆箱：基本类型存入 `Any` 时装箱到堆上，取出时拆箱
3. `as` 转换：检查 type_id 匹配后拆箱

**预计工作量**：7-10 天

---

#### 3.11 `static init()` 静态初始化块 [A]

**使用场景**：
```cangjie
static init() {
    marshallerFactoryRegistry.add(12, OpenWireFormatFactory())
}
```

**预计工作量**：2-3 天

---

#### 3.12 `extend` 内建类型实现接口 [ALL]

**使用场景**（quartz4cj 新增）：
```cangjie
extend Int64 {
    public static prop MAX_VALUE: Int64 { get() { return 0x7fffffffffffffff } }
}
extend DateTime {
    public static func currentTimeMillis(): Int64 { ... }
}
extend<T> Option<T> {
    public func isPresent(): Bool { ... }
    public func ifPresent(f: (T) -> Unit): Unit { ... }
}
extend<E> ArrayList<E> where E <: Equatable<E> {
    public func find(o: E): Bool { ... }
    public func removeByValue(o: E): Bool { ... }
}
```

**预计工作量**：4-5 天

---

### P4：集合框架 + 高级特性（预计 4-6 周）

quartz4cj 的核心依赖。

#### 4.1 `std.collection.*` 集合框架 [Q]

**现状**：`import std.collection.*` 可通过解析，但 HashMap/ArrayList/HashSet/TreeMap 没有实际实现，codegen 生成无效 WASM。

**quartz4cj 使用的集合类型**：

| 类型 | 使用场景 | 优先级 |
|------|----------|--------|
| `HashMap<K,V>` | 存储 Job/Trigger 映射，cron 字段解析 | ★★★★★ |
| `ArrayList<T>` | 监听器列表，排除日期列表 | ★★★★★ |
| `HashSet<T>` | 去重存储 | ★★★★☆ |
| `TreeMap<K,V>` | 有序映射（TreeSet 内部使用） | ★★★☆☆ |

**实现方案**：

1. **HashMap**：开放寻址或拉链法，基于 WASM 线性内存
   - 需要 Hashable 接口的 `hashCode()` 方法
   - 方法：`put`, `get`, `remove`, `containsKey`, `containsValue`, `size`, `keys`, `values`, `entries`
2. **ArrayList**：基于可增长数组
   - 方法：`add`, `get`, `set`, `remove`, `removeIf`, `size`, `isEmpty`, `contains`, `find`
3. **HashSet**：基于 HashMap 实现
4. **TreeMap**：红黑树实现

**预计工作量**：15-20 天

---

#### 4.2 复杂泛型约束方法分派 [S]

**预计工作量**：5-7 天

---

#### 4.3 Range 属性与迭代 [M+Q]

**预计工作量**：3-4 天

---

### P5：并发 + 完整生态（预计 6-8 周）

quartz4cj 的核心是并发调度，这些特性是完整运行它的前提。

#### 5.1 `spawn` 协程 [Q]

**现状**：`spawn` 关键字不被解析器识别。

**使用场景**：
```cangjie
spawn {
    try {
        run()
    } catch(e: Exception) {
        log.error("exception ${e.message}", e)
    }
    ended()
}
```

**WASM 限制**：WebAssembly 本身不支持线程和并发。可能的实现路径：
- **WASM Threads 提案**：使用 SharedArrayBuffer + Atomics
- **Cooperative scheduling**：将 spawn 编译为状态机，在单线程中交替执行
- **WASI Threads**：利用 WASI 线程扩展（wasmtime 支持）

**预计工作量**：15-20 天

---

#### 5.2 `synchronized` + `Monitor` [Q]

**使用场景**：
```cangjie
synchronized(lock) {
    if (!started) { return }
    lock.wait()
}
lock.notifyAll()
```

**依赖**：#5.1 spawn（无并发则 synchronized 可简化为无操作）

**预计工作量**：5-7 天

---

#### 5.3 `std.time.*` [Q]

**使用场景**：
```cangjie
DateTime.now()
DateTime.fromUnixTimeStamp()
DateTime.of(year: 2020, month: 1, dayOfMonth: 1, ...)
TimeZone.Local
Duration.millisecond
```

**实现方案**：基于 WASI `clock_time_get` 实现基本时间功能。

**预计工作量**：7-10 天

---

#### 5.4 `std.sync.AtomicBool/AtomicInt64` [Q]

**使用场景**：
```cangjie
private let halted = AtomicBool(false)
private static let seq: AtomicInt64 = AtomicInt64(1)
halted.store(true)
seq.fetchAdd(1)
```

**预计工作量**：3-5 天

---

#### 5.5 `Mutex` / `ReentrantMutex` / `Condition` [A]

**现状**：quartz4cj 使用 Monitor，activemq4cj 使用更底层的 Mutex/Condition 原语。

**使用场景**：
```cangjie
let sessionsMutex = ReentrantMutex()
let rwMutex = ReentrantReadWriteMutex()
rwMutex.readMutex.lock()
rwMutex.readMutex.unlock()

let condition = Condition(mutex)
condition.wait(timeout: Duration.second * 5)
condition.notify()
condition.notifyAll()
```

**依赖**：#5.1 spawn

**预计工作量**：5-7 天

---

#### 5.6 `ConcurrentHashMap` / `ArrayBlockingQueue` [A]

**使用场景**：
```cangjie
dispatchers: ConcurrentHashMap<ConsumerId, ActiveMQDispatcher>
```

**依赖**：#4.1 集合框架 + #5.5 Mutex

**预计工作量**：5-7 天

---

#### 5.7 `ThreadPool` / `ThreadLocal` / `Timer` [A]

**使用场景**：
```cangjie
ThreadPoolFactory.createThreadPool(...)
let tls = ThreadLocal<TlsClientConfig>()
let timer: ?Timer = None
```

**预计工作量**：7-10 天

---

#### 5.8 `ByteBuffer` 字节缓冲区 [A]

**使用场景**（OpenWire 协议序列化核心）：
```cangjie
let buf = ByteBuffer.allocate(8192)
buf.putInt32(commandType)
buf.putInt8(flags)
let value = buf.getInt32()
buf.putByteArray(content)
```

**实现方案**：基于 WASM 线性内存实现 ByteBuffer，支持 put/get 各数值类型。

**预计工作量**：5-7 天

---

#### 5.9 `stdx.encoding.url.URL` [A]

**使用场景**：
```cangjie
let url = URL.parse(brokerURL)
url.hostName
url.port
url.queryForm.get("wireFormat.tcpNoDelayEnabled")
```

**预计工作量**：3-5 天

---

#### 5.10 `std.log.*` / `std.regex.*` / `std.fs.*` / `std.unicode.*` [Q+A]

这些标准库模块的优先级较低，可通过纯仓颉代码在用户层部分替代。

**预计工作量**：总计 10-15 天

---

#### 5.11 `std.unittest` 宏 [S]

**预计工作量**：5-7 天

---

### P6：网络 I/O + 反射（远期，预计 8-12 周）

activemq4cj 的核心传输层依赖。

#### 6.1 `std.net.*` TCP/TLS 网络 [A]

**使用场景**：
```cangjie
let endpoint = ClientTcpEndpoint(config, threadPool)
config.host = location.hostName
config.port = location.port
config.noDelay = true
let tls = TlsContext.getCurrent()
```

**WASM 限制**：标准 WASM 不支持原生 socket。可能的实现路径：
- **WASI socket 提案**：使用 `wasi:sockets` 接口
- **代理模式**：通过 host function 桥接到宿主环境的网络栈

**预计工作量**：15-20 天

---

#### 6.2 `std.reflect.TypeInfo` 反射 [A]

**使用场景**：
```cangjie
let info = TypeInfo.of(target)
let prop = info.getInstanceProperty(name)
prop.setValue(target, convertedValue)
```

**实现方案**：
- 编译期生成类型元数据表
- TypeInfo 通过类型 ID 查找元数据
- 属性访问通过 offset + getter/setter 函数指针

**预计工作量**：15-20 天

---

#### 6.3 `stdx.serialization.*` 序列化框架 [A]

**使用场景**：
```cangjie
import stdx.serialization.serialization.DataModel
import stdx.serialization.serialization.Serializable
```

**预计工作量**：10-15 天

---

### 不实现的特性

| 特性 | 原因 |
|------|------|
| `foreign func` + `CPointer<Unit>` + `unsafe` | WASM 沙箱环境无法链接原生 C 库 |
| `std.io.InputStream/OutputStream` 完整版 | WASM 不支持完整的 I/O 流抽象 |
| 原生线程/进程管理 | WASM 沙箱限制 |
| 真实 TCP/TLS 网络连接 | WASM 沙箱限制（可通过 WASI socket 提案部分支持） |

---

## 三、实施路线图

```
P1 (3-5天)     █████  else if + wildcard + 6 个 Bug 修复
                │
P2 (3-4周)     █████████████████  Option + type alias + 函数类型 + static
                │                  + String方法 + 命名参数 + Array构造
                │                  + package + mut prop + override + try-finally
                │
P3 (4-6周)     █████████████████████████  运算符重载 + if let/?? + is/as
                │                          + Lambda + 元组 + 方法重载 + Any
                │
P4 (4-6周)     █████████████████████████  集合框架 + 多模块编译
                │                          + 外部依赖 + 泛型分派
                │
P5 (6-8周)     █████████████████████████████████  spawn + synchronized
                │                                    + std.time/log/sync
                │                                    + Mutex/Condition/Atomic
                │                                    + ByteBuffer/Timer/URL
                │
P6 (远期)      ████████████████████████████████████████  网络I/O + 反射
                                                          + 序列化 + TLS
```

### 里程碑目标

| 阶段 | 版本 | 目标 | 可编译范围 |
|------|------|------|-----------|
| P1 完成 | v0.9.0 | Bug 修复 + 基础语法 | 避免 enum match / try-catch / prop 等 Bug；支持 `let _ =` |
| P2 完成 | v0.9.5 | 核心语法补全 | Option 类型、String 方法、static 成员、函数类型、package 声明 |
| P3 完成 | v1.0.0 | 面向对象支持 | **matrix4cj 核心模块 + quartz4cj 数据模型可编译** |
| P4 完成 | v1.1.0 | 集合 + 多模块 | **quartz4cj 调度逻辑 + activemq4cj 接口层可编译** |
| P5 完成 | v1.2.0 | 并发 + 生态 | **quartz4cj 完整运行** |
| P6 完成 | v2.0.0 | 网络 + 反射 | **activemq4cj 完整运行** |

### P1 完成后的预期效果

修复所有已知 Bug，以下代码模式可正常编译运行：
- enum + match 正确区分变体
- prop getter 返回正确值
- 类继承中 super() 参数传递
- 字符串 `+` 拼接
- 方法链式调用 (Builder 模式)
- try-catch 异常处理
- `let _ = expr` 丢弃返回值

### P3 完成后的预期效果

matrix4cj 的以下核心文件可直接编译运行：
- `src/matrix.cj` — Matrix 类（创建/访问/运算）
- `src/maths.cj` — 数学辅助函数
- `src/matrix_exception.cj` — 异常类

quartz4cj 的以下数据模型文件可编译：
- `src/org/quartz/jobkey.cj` — JobKey
- `src/org/quartz/triggerkey.cj` — TriggerKey
- `src/org/quartz/timeofday.cj` — TimeOfDay
- `src/org/quartz/jobdatamap.cj` — JobDataMap
- `src/org/quartz/impl_jobdetailimpl.cj` — JobDetail 实现
- `src/org/quartz/impl_matchers_*.cj` — Matcher 系列

activemq4cj 的以下接口定义文件可编译：
- `src/cjms/message.cj` — Message 接口
- `src/cjms/destination.cj` — Destination 接口
- `src/cjms/delivery_mode.cj` — DeliveryMode 枚举
- `src/cjms/cjms_exception.cj` — 异常类层次

### P4 完成后的预期效果

quartz4cj 的核心调度逻辑（去除并发部分）可编译：
- `src/org/quartz/cronexpression.cj` — Cron 表达式解析
- `src/org/quartz/datebuilder.cj` — 日期构建器
- `src/org/quartz/simpl_ramjobstore.cj` — 内存 Job 存储
- `src/org/quartz/impl_triggers_*.cj` — 触发器实现

activemq4cj 的数据模型层可编译（需多模块支持）：
- `src/client/command/*.cj` — 命令/消息数据结构
- `src/client/api/*.cj` — API 接口定义

### P5 完成后的预期效果

quartz4cj 可完整运行（190 个文件）

activemq4cj 的核心逻辑（去除网络部分）可编译：
- `src/client/activemq_session.cj` — 会话管理
- `src/client/state/*.cj` — 连接状态追踪
- `src/client/openwire/*.cj` — 协议序列化

### P6 完成后的预期效果

activemq4cj 可完整运行（236 个文件）：
- `src/client/transport/tcp_transport.cj` — TCP 传输
- `src/client/transport/failover_transport.cj` — 故障转移
- `src/client/activemq_connection.cj` — 连接管理

---

## 四、四个库的特性使用热力图

```
                          scientific  matrix4cj  quartz4cj  activemq4cj
P1:
  else if                    ████        ████        ████        ████
  void 函数语句              ████        ████        ████        ████
  let _ = expr                                       ████
  [BUG] enum match                                   ████        ████
  [BUG] prop getter                      ████        ████        ████
  [BUG] super() 参数                     ████        ████        ████
  [BUG] String +                         ██          ████        ████
  [BUG] 链式调用                                     ████        ████
  [BUG] try-catch                        ██          ████        ████

P2:
  ?T Option 类型                                     ████        ████
  type 别名                                          ████        ████
  函数类型参数                                        ████        ████
  static 成员                                        ████        ████
  package 包声明                                                  ████
  internal import                                                 ████
  mut prop 属性                                      ████        ████
  override 关键字                                    ████        ████
  static var                                                      ████
  try-finally                                                     ████
  完整数值类型                                                    ████
  for-in 步长                                        ████
  抽象方法                                           ████        ████
  动态 Array 构造            ████        ████        ████        ████
  命名参数                   ████        ████        ████        ████
  Array 方法                 ██          ████                    ████
  String 方法                                        ████        ████
  import as                  ██          ██                      ████

P3:
  运算符重载                 ██          ████        ████        ████
  if let / ??                                        ████        ████
  is / as 类型检查                                   ████        ████
  Lambda/闭包                                        ████        ████
  元组 / for((k,v) in m)                             ████        ████
  方法重载                                           ████        ████
  match type/where                                   ████        ████
  Any 类型                                                        ████
  static init()                                                   ████
  嵌套数组                                ████
  extend 内建类型            ████        ████        ████

P4:
  HashMap / ArrayList                                ████        ████
  HashSet / TreeMap                                  ████        ████
  外部依赖 / 多模块                                               ████
  复杂泛型分派               ████                                 ████
  Range 属性                              ████       ████

P5:
  spawn 协程                                         ████        ████
  synchronized                                       ████        ████
  Mutex / ReentrantMutex                                          ████
  Condition (wait/notify)                                         ████
  ConcurrentHashMap                                               ████
  Monitor / Atomic                                   ████        ████
  ThreadPool/ThreadLocal                                          ████
  ByteBuffer                                                      ████
  Timer / Duration                                   ████        ████
  URL 解析                                                        ████
  std.time.*                                         ████        ████
  std.log.*                                          ████        ████
  std.regex.*                                        ████
  std.unittest               ████

P6 (远期):
  TCP/TLS 网络                                                    ████
  反射 (TypeInfo)                                                 ████
  序列化框架                                                      ████
  FFI (不实现)               ████                                 ████
```

---

## 五、quartz4cj 适配编译测试结果

### 测试环境

创建了 cjwasm 兼容的简化版本 `/tmp/quartz4cj_wasm/`，移植了 quartz4cj 的核心概念：

- enum 定义与 match（IntervalUnit）
- class + interface 实现（SimpleTrigger <: Trigger）
- Builder 模式（ScheduleBuilder 链式调用）
- 对象方法调用与字段访问
- 类方法中的 Array 操作（CronField.contains）
- TimeOfDay 比较逻辑

### 测试结果：18/20 通过

| 测试 | 结果 | 说明 |
|------|------|------|
| enum + match (MINUTE) | ❌ | Bug B1: 返回 1 而非 60 |
| enum + match (HOUR) | ❌ | Bug B1: 返回 1 而非 3600 |
| SimpleTrigger 创建 | ✅ | |
| SimpleTrigger 方法 | ✅ | |
| trigger() 状态更新 | ✅ | |
| getNextFireTime() | ✅ | |
| canFireAgain() | ✅ | |
| CronField.contains(存在) | ✅ | |
| CronField.contains(不存在) | ✅ | |
| TimeOfDay.compare() | ✅ | |
| TimeOfDay.toMillis() | ✅ | |
| TimeOfDay.equals() | ✅ | |
| Builder 模式 (interval) | ✅ | 需拆分链式调用为多步 |
| Builder 模式 (repeatCount) | ✅ | |
| shouldFireNow() | ✅ | |
| parseCronField (all) | ✅ | |
| parseCronField (single) | ✅ | |

### 已确认可用的特性

- enum 定义（`| VARIANT` 语法）
- class 定义 + interface 实现（`<:` 语法）
- class 继承 + 方法重写
- `open` class 修饰符
- `prop` 定义（解析通过，但有 Bug B2）
- 对象构造、字段访问、方法调用
- Array<Int64> 在 class 字段中的使用
- 字符串相等比较 `==`
- `match` 基本语法（有 Bug B1）

### 需要 workaround 的特性

| 原始写法 | Workaround |
|----------|------------|
| `builder.a().b().c()` | `let t1 = builder.a(); let t2 = t1.b(); let t3 = t2.c()` |
| `let _ = func()` | `let _unused = func()` |
| `else if (cond) {}` | `else { if (cond) {} }` |
| `void 函数语句` | 改为返回 `Int64`, `return 0` |

---

## 六、activemq4cj 分析摘要

### 项目概况

| 属性 | 值 |
|------|-----|
| 仓库 | https://gitcode.com/Cangjie-TPC/activemq4cj.git |
| 版本 | 1.0.0 |
| 源文件 | 236 个 .cj 文件 |
| 描述 | ActiveMQ 消息队列仓颉 SDK 实现 |
| 外部依赖 | `hyperion` (网络框架, git 依赖) |
| 编译结果 | 立即失败（中文注释触发词法错误 + 大量不支持特性） |

### 项目特点

activemq4cj 是四个库中**复杂度最高**的企业级项目：

1. **多模块架构**：使用 `package` 声明和跨包 import，包含 `cjms`（JMS 接口层）、`client`（ActiveMQ 实现）、`command`（协议命令）、`transport`（网络传输）、`openwire`（序列化协议）等多个模块
2. **重度网络 I/O**：TCP 连接、TLS 加密、故障转移传输
3. **企业级并发**：Mutex/ReentrantMutex/Condition/ConcurrentHashMap/ThreadPool/AtomicBool
4. **二进制协议**：ByteBuffer 操作、marshal/unmarshal 序列化
5. **反射机制**：TypeInfo 实现属性内省和动态设置
6. **完整类型系统**：Any 动态类型、大量 Option 类型、接口继承链

### activemq4cj 引入的全新特性（之前三个库未使用）

| # | 特性 | 说明 | 代码示例 |
|---|------|------|----------|
| 1 | `package` 声明 | 多包架构 | `package activemq4cj.cjms` |
| 2 | `internal import` | import 可见性控制 | `internal import std.collection.LinkedList` |
| 3 | 多目标 import | 从同一包导入多个项 | `import activemq4cj.cjms.{Destination, Queue, Topic}` |
| 4 | `mut prop` | 可变属性 (getter+setter) | `mut prop text: ?String { get(){...} set(v){...} }` |
| 5 | `static init()` | 静态初始化块 | `static init() { registry.add(12, ...) }` |
| 6 | `static var` | 可变静态字段 | `private static var instanceCount: AtomicInt32` |
| 7 | `override` 关键字 | 方法重写标记 | `public open override func hashCode(): Int64` |
| 8 | `@OverflowWrapping` | 算术溢出注解 | `@OverflowWrapping func hashCode()` |
| 9 | `Any` 类型 | 动态类型+类型擦除 | `HashMap<String, Any>`, `(v as String)` |
| 10 | `try-finally` | 无 catch 的 finally | `try { ... } finally { mutex.unlock() }` |
| 11 | `Mutex`/`ReentrantMutex` | 互斥锁(非 Monitor) | `ReentrantMutex()`, `mutex.lock()/unlock()` |
| 12 | `ReentrantReadWriteMutex` | 读写锁 | `mutex.readMutex.lock()`, `mutex.writeMutex.lock()` |
| 13 | `Condition` | 条件变量 | `condition.wait(timeout:)`, `condition.notify()` |
| 14 | `ConcurrentHashMap` | 线程安全 Map | `ConcurrentHashMap<ConsumerId, Dispatcher>` |
| 15 | `ArrayBlockingQueue` | 阻塞队列 | `import std.collection.concurrent.ArrayBlockingQueue` |
| 16 | `SyncCounter` | 同步计数器 | `SyncCounter(1)`, `waitUntilZero(timeout:)` |
| 17 | `ThreadPool` | 线程池 | `ThreadPoolFactory.createThreadPool(...)` |
| 18 | `ThreadLocal` | 线程局部存储 | `ThreadLocal<TlsClientConfig>()` |
| 19 | `Timer` | 定时器 | `optimizedAckTimer: ?Timer = None` |
| 20 | `Duration` 算术 | 时间运算 | `Duration.second * 5`, `Duration.millisecond * 10` |
| 21 | `ByteBuffer` | 字节缓冲操作 | `ByteBuffer.allocate(8192)`, `putInt32()`, `getInt32()` |
| 22 | `ClientTcpEndpoint` | TCP 客户端 | `ClientTcpEndpoint(config, threadPool)` |
| 23 | `TlsContext` | TLS 加密 | `TlsContext.getCurrent()`, `config.tlsEnabled` |
| 24 | `URL.parse()` | URL 解析 | `URL.parse(brokerURL)`, `location.hostName` |
| 25 | `TypeInfo` 反射 | 类型内省 | `TypeInfo.of(target)`, `getInstanceProperty(name)` |
| 26 | `Int8/Int16/Int32/UInt16/UInt32/Byte/Float32` | 完整数值类型 | `func readInt16(): Int16`, `Byte = 0x01` |
| 27 | 外部 git 依赖 | 包管理 | `hyperion = {git = "...", branch = "master"}` |
| 28 | `Resource` 接口 | 自动关闭 | `interface Connection <: Resource` |
| 29 | `None<T>` 类型化空值 | Option 语法细节 | `linkedExceptionVal: ?Exception = None<Exception>` |
| 30 | `Serializable<T>` | 序列化接口 | `import stdx.serialization.serialization.Serializable` |

### 与前三个库的对比

| 维度 | scientific | matrix4cj | quartz4cj | activemq4cj |
|------|-----------|-----------|-----------|-------------|
| 文件数 | ~30 | ~20 | 190 | **236** |
| 复杂度 | 数值计算 | 矩阵运算 | 任务调度 | **消息队列** |
| 核心特性 | FFI + 泛型 | OOP + 运算符 | 并发 + 集合 | **网络 + 反射 + 序列化** |
| 外部依赖 | C 库(FFI) | 无 | 无 | **hyperion (git)** |
| 多模块 | 否 | 否 | 否 | **是 (cjms + client + ...)** |
| 并发需求 | 无 | 无 | 高 | **极高** |
| 网络需求 | 无 | 无 | 无 | **TCP + TLS** |
| 反射需求 | 无 | 无 | 无 | **TypeInfo** |
| 序列化需求 | 无 | 无 | 无 | **OpenWire 协议** |

### 编译可行性评估

activemq4cj 是目前分析的四个库中**距离 cjwasm 可编译最远**的项目。完整编译需要：

1. **P1-P3 全部完成**：基础语法 + OOP + 类型系统
2. **P4 完成**：集合框架 + 多模块编译
3. **P5 完成**：并发原语 + 标准库
4. **P6（新增阶段）完成**：网络 I/O + 反射 + 序列化

预计需要 **P6 阶段全部完成**后才能尝试编译 activemq4cj 的核心逻辑。

---

*文档版本: 3.1.0*
*更新日期: 2026-02-15*
*变更: 全部 6 个 Bug (B1-B6) 已修复*
*基于库版本: scientific v0.1.0, matrix4cj v1.0.4, quartz4cj v2.4.0, activemq4cj v1.0.0*
