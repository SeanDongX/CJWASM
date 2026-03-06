//! 标准库元数据模块 (P0)
//!
//! 提供仓颉标准库类型的静态元数据，用于改进类型推断。
//! 覆盖 `infer_ast_type` 无法通过本地符号表解析的第三方/标准库类型。

use crate::ast::Type;

/// 查询标准库类型方法的返回类型。
///
/// `type_name`  - 对象的结构体名（如 "ArrayList", "HashMap"）
/// `type_args`  - 泛型参数（如 ArrayList<Int64> → [Int64]）
/// `method`     - 方法名
pub fn stdlib_method_return_type(type_name: &str, type_args: &[Type], method: &str) -> Option<Type> {
    let elem = || type_args.first().cloned().unwrap_or(Type::Int64);
    let ktype = || type_args.first().cloned().unwrap_or(Type::Int64);
    let vtype = || type_args.get(1).cloned().unwrap_or(Type::Int64);

    match (type_name, method) {
        // ── ArrayList / LinkedList / ArrayStack ──────────────────────────────
        // 底层 WASM 实现均直接返回元素值（i64），不包装为 Option
        ("ArrayList" | "LinkedList" | "ArrayStack", "get" | "first" | "last" | "pop" | "remove") => {
            Some(elem())
        }
        // Unit 返回值不暴露，避免 .to_wasm() panic
        (
            "ArrayList" | "LinkedList" | "ArrayStack",
            "add" | "push" | "append" | "prepend" | "insert" | "clear" | "sort" | "sortBy"
            | "reverse" | "set",
        ) => None,
        ("ArrayList" | "LinkedList" | "ArrayStack", "size") => Some(Type::Int64),
        ("ArrayList" | "LinkedList" | "ArrayStack", "isEmpty") => Some(Type::Bool),
        ("ArrayList" | "LinkedList" | "ArrayStack", "contains") => Some(Type::Bool),
        ("ArrayList" | "LinkedList" | "ArrayStack", "indexOf" | "lastIndexOf") => {
            Some(Type::Int64)
        }
        ("ArrayList" | "LinkedList" | "ArrayStack", "toArray" | "clone" | "slice") => {
            Some(Type::Array(Box::new(elem())))
        }
        ("ArrayList" | "LinkedList" | "ArrayStack", "iterator") => {
            Some(Type::Struct("Iterator".to_string(), type_args.to_vec()))
        }
        ("ArrayList" | "LinkedList" | "ArrayStack", "toString") => Some(Type::String),

        // ── HashMap ──────────────────────────────────────────────────────────
        ("HashMap", "get") => Some(Type::Option(Box::new(vtype()))),
        ("HashMap", "getOrDefault" | "getOrThrow") => Some(vtype()),
        ("HashMap", "remove") => Some(Type::Option(Box::new(vtype()))),
        ("HashMap", "put" | "putAll" | "clear") => None,
        ("HashMap", "containsKey" | "containsValue" | "isEmpty") => Some(Type::Bool),
        ("HashMap", "size") => Some(Type::Int64),
        ("HashMap", "keys") => Some(Type::Array(Box::new(ktype()))),
        ("HashMap", "values") => Some(Type::Array(Box::new(vtype()))),
        ("HashMap", "entries") => {
            Some(Type::Array(Box::new(Type::Tuple(vec![ktype(), vtype()]))))
        }
        ("HashMap", "toString") => Some(Type::String),

        // ── HashSet ──────────────────────────────────────────────────────────
        ("HashSet", "add" | "remove" | "clear") => None,
        ("HashSet", "contains") => Some(Type::Bool),
        ("HashSet", "size") => Some(Type::Int64),
        ("HashSet", "isEmpty") => Some(Type::Bool),
        ("HashSet", "toArray") => Some(Type::Array(Box::new(elem()))),
        ("HashSet", "iterator") => {
            Some(Type::Struct("Iterator".to_string(), type_args.to_vec()))
        }

        // ── StringBuilder ────────────────────────────────────────────────────
        ("StringBuilder", "append" | "prepend" | "insert" | "deleteCharAt" | "clear") => {
            Some(Type::Struct("StringBuilder".to_string(), vec![]))
        }
        ("StringBuilder", "toString" | "build") => Some(Type::String),
        ("StringBuilder", "length" | "size") => Some(Type::Int64),
        ("StringBuilder", "isEmpty") => Some(Type::Bool),

        // ── Path ─────────────────────────────────────────────────────────────
        ("Path", "join" | "resolve" | "normalize") => {
            Some(Type::Struct("Path".to_string(), vec![]))
        }
        ("Path", "toString" | "fileName" | "extension" | "stem") => Some(Type::String),
        ("Path", "parent") => Some(Type::Option(Box::new(Type::Struct(
            "Path".to_string(),
            vec![],
        )))),
        ("Path", "exists" | "isFile" | "isDirectory" | "isAbsolute" | "isRelative") => {
            Some(Type::Bool)
        }
        ("Path", "components") => Some(Type::Array(Box::new(Type::String))),

        // ── Duration ─────────────────────────────────────────────────────────
        (
            "Duration",
            "toNanoseconds" | "toMicroseconds" | "toMilliseconds" | "toSeconds" | "toMinutes"
            | "toHours",
        ) => Some(Type::Int64),
        ("Duration", "toString") => Some(Type::String),
        ("Duration", "add" | "sub" | "mul") => Some(Type::Struct("Duration".to_string(), vec![])),

        // ── DateTime / Instant ───────────────────────────────────────────────
        ("DateTime" | "Instant", "timestamp" | "toEpochMilli" | "toEpochNano") => {
            Some(Type::Int64)
        }
        ("DateTime" | "Instant", "format" | "toString") => Some(Type::String),
        ("DateTime" | "Instant", "add" | "sub") => {
            Some(Type::Struct(type_name.to_string(), vec![]))
        }
        ("DateTime" | "Instant", "isBefore" | "isAfter" | "equals") => Some(Type::Bool),
        (
            "DateTime",
            "year" | "month" | "day" | "hour" | "minute" | "second" | "nanosecond",
        ) => Some(Type::Int64),

        // ── Thread ───────────────────────────────────────────────────────────
        ("Thread", "join" | "start" | "sleep") => None,
        ("Thread", "id") => Some(Type::Int64),
        ("Thread", "name") => Some(Type::String),
        ("Thread", "isAlive" | "isDaemon") => Some(Type::Bool),

        // ── Channel ──────────────────────────────────────────────────────────
        ("Channel", "send" | "close") => None,
        ("Channel", "receive") => Some(elem()),
        ("Channel", "tryReceive") => Some(Type::Option(Box::new(elem()))),
        ("Channel", "isClosed" | "isEmpty") => Some(Type::Bool),
        ("Channel", "size" | "capacity") => Some(Type::Int64),

        // ── File / IO ────────────────────────────────────────────────────────
        (
            "File" | "FileWriter" | "FileReader" | "BufferedReader" | "BufferedWriter"
            | "OutputStream" | "InputStream",
            "readToString",
        ) => Some(Type::String),
        (
            "File" | "FileReader" | "BufferedReader" | "InputStream",
            "readLine" | "readLines",
        ) => Some(Type::Option(Box::new(Type::String))),
        (
            "File" | "FileWriter" | "FileReader" | "BufferedReader" | "BufferedWriter"
            | "OutputStream" | "InputStream",
            "write" | "writeLine" | "flush" | "close",
        ) => None,
        (
            "File" | "FileWriter" | "FileReader" | "BufferedReader" | "BufferedWriter",
            "size" | "length",
        ) => Some(Type::Int64),
        (
            "File",
            "exists" | "isFile" | "isDirectory" | "createNewFile" | "mkdirs" | "delete",
        ) => Some(Type::Bool),
        ("File", "listFiles") => {
            Some(Type::Array(Box::new(Type::Struct("File".to_string(), vec![]))))
        }
        ("File", "name" | "absolutePath" | "canonicalPath" | "parent") => Some(Type::String),
        ("File", "openRead") => Some(Type::Struct("FileReader".to_string(), vec![])),
        ("File", "openWrite" | "openAppend") => Some(Type::Struct("FileWriter".to_string(), vec![])),

        // ── Random ───────────────────────────────────────────────────────────
        ("Random", "nextInt64" | "nextInt32" | "nextInt" | "nextLong") => Some(Type::Int64),
        ("Random", "nextFloat64" | "nextDouble" | "nextFloat") => Some(Type::Float64),
        ("Random", "nextBool") => Some(Type::Bool),
        ("Random", "nextBytes") => Some(Type::Array(Box::new(Type::UInt8))),

        // ── Regex ────────────────────────────────────────────────────────────
        ("Regex", "matches" | "containsMatchIn" | "matchEntire") => Some(Type::Bool),
        ("Regex", "find") => Some(Type::Option(Box::new(Type::String))),
        ("Regex", "findAll") => Some(Type::Array(Box::new(Type::String))),
        ("Regex", "replace" | "replaceAll" | "replaceFirst") => Some(Type::String),
        ("Regex", "split") => Some(Type::Array(Box::new(Type::String))),

        // ── Queue / Deque ────────────────────────────────────────────────────
        ("Queue" | "Deque" | "PriorityQueue", "enqueue" | "offer" | "push") => None,
        ("Queue" | "Deque" | "PriorityQueue", "dequeue" | "poll" | "pop" | "peek") => {
            Some(Type::Option(Box::new(elem())))
        }
        ("Queue" | "Deque" | "PriorityQueue", "size") => Some(Type::Int64),
        ("Queue" | "Deque" | "PriorityQueue", "isEmpty") => Some(Type::Bool),

        // ── Stack ────────────────────────────────────────────────────────────
        ("Stack", "push") => None,
        ("Stack", "pop" | "peek" | "top") => Some(Type::Option(Box::new(elem()))),
        ("Stack", "size") => Some(Type::Int64),
        ("Stack", "isEmpty") => Some(Type::Bool),

        // ── Iterator ─────────────────────────────────────────────────────────
        ("Iterator", "next") => Some(Type::Option(Box::new(elem()))),
        ("Iterator", "hasNext") => Some(Type::Bool),
        ("Iterator", "toArray" | "collect") => Some(Type::Array(Box::new(elem()))),
        ("Iterator", "count" | "size") => Some(Type::Int64),
        ("Iterator", "map" | "filter" | "flatMap") => {
            Some(Type::Struct("Iterator".to_string(), type_args.to_vec()))
        }

        // ── TreeMap / TreeSet ────────────────────────────────────────────────
        ("TreeMap", "get") => Some(Type::Option(Box::new(vtype()))),
        ("TreeMap", "put" | "remove" | "clear") => None,
        ("TreeMap", "containsKey" | "isEmpty") => Some(Type::Bool),
        ("TreeMap", "size") => Some(Type::Int64),
        ("TreeMap", "keys") => Some(Type::Array(Box::new(ktype()))),
        ("TreeMap", "values") => Some(Type::Array(Box::new(vtype()))),
        ("TreeSet", "add" | "remove" | "clear") => None,
        ("TreeSet", "contains" | "isEmpty") => Some(Type::Bool),
        ("TreeSet", "size") => Some(Type::Int64),

        _ => None,
    }
}

/// 查询标准库类型字段的类型。
///
/// `type_name`  - 结构体名
/// `type_args`  - 泛型参数
/// `field`      - 字段名
pub fn stdlib_field_type(type_name: &str, type_args: &[Type], field: &str) -> Option<Type> {
    match (type_name, field) {
        // Duration 字段
        (
            "Duration",
            "nanoseconds" | "microseconds" | "milliseconds" | "seconds" | "minutes" | "hours",
        ) => Some(Type::Int64),

        // DateTime / Instant 字段
        (
            "DateTime" | "Instant",
            "year" | "month" | "day" | "hour" | "minute" | "second" | "nanosecond" | "timestamp",
        ) => Some(Type::Int64),
        ("DateTime" | "Instant", "date") => Some(Type::Struct("DateTime".to_string(), vec![])),

        // Thread 字段
        ("Thread", "id") => Some(Type::Int64),
        ("Thread", "name") => Some(Type::String),
        ("Thread", "isAlive" | "isDaemon") => Some(Type::Bool),

        // Channel 字段
        ("Channel", "capacity" | "size") => Some(Type::Int64),
        ("Channel", "isClosed") => Some(Type::Bool),

        // File 字段
        ("File", "name" | "absolutePath" | "path" | "parent") => Some(Type::String),
        ("File", "exists" | "isFile" | "isDirectory") => Some(Type::Bool),
        ("File", "length" | "size") => Some(Type::Int64),

        // Path 字段
        ("Path", "fileName" | "extension" | "stem") => Some(Type::String),

        // 异常/错误字段（通用 - 所有类型均可能有 message 字段）
        (_, "message" | "msg" | "detail") => Some(Type::String),
        (_, "cause") => Some(Type::Option(Box::new(Type::String))),
        (_, "code" | "errorCode" | "status") => Some(Type::Int64),

        _ => None,
    }
}

/// 查询标准库构造函数/工厂方法的返回类型。
///
/// 用于 `Call` 表达式中，当名字不在 `func_return_types` 里时作为兜底。
pub fn stdlib_constructor_type(name: &str, type_args: &[Type]) -> Option<Type> {
    let elem = || type_args.first().cloned().unwrap_or(Type::Int64);
    let ktype = || type_args.first().cloned().unwrap_or(Type::Int64);
    let vtype = || type_args.get(1).cloned().unwrap_or(Type::Int64);

    match name {
        "StringBuilder" => Some(Type::Struct("StringBuilder".to_string(), vec![])),
        "Path" => Some(Type::Struct("Path".to_string(), vec![])),
        "Duration" => Some(Type::Struct("Duration".to_string(), vec![])),
        "DateTime" | "Instant" => Some(Type::Struct(name.to_string(), vec![])),
        "Thread" => Some(Type::Struct("Thread".to_string(), vec![])),
        "Random" => Some(Type::Struct("Random".to_string(), vec![])),
        "Regex" => Some(Type::Struct("Regex".to_string(), vec![])),
        "File" => Some(Type::Struct("File".to_string(), vec![])),
        "FileWriter" => Some(Type::Struct("FileWriter".to_string(), vec![])),
        "FileReader" => Some(Type::Struct("FileReader".to_string(), vec![])),
        "BufferedReader" => Some(Type::Struct("BufferedReader".to_string(), vec![])),
        "BufferedWriter" => Some(Type::Struct("BufferedWriter".to_string(), vec![])),
        "Queue" | "Deque" | "PriorityQueue" => Some(Type::Struct(name.to_string(), type_args.to_vec())),
        "Stack" => Some(Type::Struct("Stack".to_string(), type_args.to_vec())),
        "TreeMap" => Some(Type::Map(Box::new(ktype()), Box::new(vtype()))),
        "TreeSet" => Some(Type::Map(Box::new(elem()), Box::new(Type::Int64))),
        "Channel" => Some(Type::Struct("Channel".to_string(), type_args.to_vec())),
        _ => None,
    }
}
