package cjson_simple

import stdx.encoding.json.*

// 简单的 JSON 解析和生成演示
main() {
    // 测试 1: 解析 JSON 字符串
    let jsonStr = "{\"name\":\"Christmas\",\"value\":2024}"
    println("输入 JSON: " + jsonStr)
    
    let parsed = JsonValue.fromStr(jsonStr)
    let obj = parsed.asObject()
    
    let name = obj.get("name").getOrThrow().asString().getValue()
    let value = obj.get("value").getOrThrow().asInt().getValue()
    
    println("解析结果:")
    println("  name = " + name)
    println("  value = ${value}")
    
    // 测试 2: 构建 JSON 对象
    var newObj = JsonObject()
    newObj.put("greeting", JsonString("Hello"))
    newObj.put("count", JsonInt(42))
    newObj.put("pi", JsonFloat(3.14159))
    newObj.put("active", JsonBool(true))
    
    let output = newObj.toString()
    println("\n构建的 JSON: " + output)
    
    // 测试 3: JSON 数组
    var arr = JsonArray()
    arr.add(JsonInt(1))
    arr.add(JsonInt(2))
    arr.add(JsonInt(3))
    arr.add(JsonString("four"))
    
    println("\nJSON 数组: " + arr.toString())
    
    println("\n测试完成!")
}
