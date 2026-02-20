// CJson 官方 example 的 WASM 可运行副本
// 对应: https://gitcode.com/Cangjie-TPC/CJson (cjc_1.1.0) src/example/demo/
// 原示例依赖 @JsonSerializable 宏、DateTime、HashMap、stdx；此处用手动 toJson/fromJson 实现相同输入输出。

// ---------- JSON 解析辅助 ----------
func jsonStr(json: String, key: String): String {
    let p = "\"" + key + "\":\""
    let i = json.indexOf(p)
    if (i < 0) { return "" }
    let rest = json.replace(p, "||SEP||")
    let parts = rest.split("||SEP||")
    if (parts.size < 2) { return "" }
    let end = parts[1].split("\"")
    return end[0]
}

func jsonInt(json: String, key: String): Int64 {
    let p = "\"" + key + "\":"
    let i = json.indexOf(p)
    if (i < 0) { return 0 }
    let rest = json.replace(p, "||SEP||")
    let parts = rest.split("||SEP||")
    if (parts.size < 2) { return 0 }
    let numPart = parts[1].split(",")[0].replace("}", "").replace(" ", "").trim()
    return numPart.toInt64()
}


// ---------- ExampleOne (对应 @JsonSerializable class ExampleOne) ----------
struct ExampleOne {
    var name: String
    var time: String
}

func ExampleOne_fromJson(json: String): ExampleOne {
    let name = jsonStr(json, "name")
    let time = jsonStr(json, "time")
    return ExampleOne { name: name, time: time }
}

func ExampleOne_toJson(self: ExampleOne): String {
    return "{\"name\":\"" + self.name + "\",\"time\":\"" + self.time + "\"}"
}

// ---------- ClassWithOption ----------
struct ClassWithOption {
    var optStrNull: String
    var optStrVal: String
    var optIntNull: String
    var optIntVal: Int64
}

func ClassWithOption_fromJson(json: String): ClassWithOption {
    let s1 = "null"
    let s2 = jsonStr(json, "optionalStringWithValue")
    let s3 = "{}"
    let i4 = jsonInt(json, "optionalIntWithValue")
    return ClassWithOption { optStrNull: s1, optStrVal: s2, optIntNull: s3, optIntVal: i4 }
}

func ClassWithOption_toJson(self: ClassWithOption): String {
    return "{\"optionalStringWithoutValue\": null, \"optionalStringWithValue\": \"" + self.optStrVal + "\", \"optionalIntWithoutValue\": {}, \"optionalIntWithValue\": " + self.optIntVal.toString() + "}"
}

// ---------- ExampleOneStruct ----------
struct ExampleOneStruct {
    var name: String
    var time: String
}

func ExampleOneStruct_fromJson(json: String): ExampleOneStruct {
    let name = jsonStr(json, "name")
    let time = jsonStr(json, "time")
    return ExampleOneStruct { name: name, time: time }
}

func ExampleOneStruct_toJson(self: ExampleOneStruct): String {
    return "{\"name\":\"" + self.name + "\",\"time\":\"" + self.time + "\"}"
}

// ---------- ExampleOneOptional (One/Two/Three/Four) ----------
struct ExampleOneOptional {
    var one: String
    var two: String
    var three: Int64
    var four: String
}

func ExampleOneOptional_fromJson(json: String): ExampleOneOptional {
    let three = jsonInt(json, "Three")
    let four = jsonStr(json, "Four")
    return ExampleOneOptional { one: "null", two: "null", three: three, four: four }
}

func ExampleOneOptional_toJson(self: ExampleOneOptional): String {
    return "{\"One\": null, \"Two\": null, \"Three\": " + self.three.toString() + ", \"Four\": \"" + self.four + "\"}"
}

// ---------- ExampleTwo (name + CustomTime with value) ----------
struct CustomTime {
    var value: String
}

struct ExampleTwo {
    var name: String
    var time: CustomTime
}

func CustomTime_fromJson(json: String): CustomTime {
    let v = jsonObjectValueStr(json, "value")
    return CustomTime { value: v }
}

func CustomTime_toJson(self: CustomTime): String {
    return "{\"value\":\"" + self.value + "\"}"
}

func ExampleTwo_fromJson(json: String): ExampleTwo {
    let name = jsonStr(json, "name")
    let timeVal = jsonStr(json, "value")
    return ExampleTwo { name: name, time: CustomTime { value: timeVal } }
}

func ExampleTwo_toJson(self: ExampleTwo): String {
    let t = CustomTime_toJson(self.time)
    return "{\"name\":\"" + self.name + "\",\"time\":" + t + "}"
}

// ---------- main（与 CJson 官方 example main.cj 一致）----------
main(): Int64 {
    println(ExampleOne_toJson(ExampleOne_fromJson("{\"name\":\"Chrsitmas\",\"time\":\"2022-12-25T00:00:00+01:00\"}")))
    println(ClassWithOption_toJson(ClassWithOption_fromJson("{\"optionalStringWithoutValue\": null, \"optionalStringWithValue\": \"StringValue\", \"optionalIntWithoutValue\": {}, \"optionalIntWithValue\": 199}")))
    println(ExampleOneStruct_toJson(ExampleOneStruct_fromJson("{\"name\":\"Labor Day\",\"time\":\"2023-05-01T00:00:00+10:00\"}")))
    println(ExampleOneOptional_toJson(ExampleOneOptional_fromJson("{\"One\": null, \"Two\": null, \"Three\": 64, \"Four\": \"\"}")))
    println(ExampleTwo_toJson(ExampleTwo_fromJson("{\"name\":\"New Year\",\"time\":{\"value\":\"CUST_2023-01-01T00:00:00+08:00\"}}")))
    return 0
}
