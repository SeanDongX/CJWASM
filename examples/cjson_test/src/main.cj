package cjson_test

// ============================================
// CJson-style Demo for cjwasm
// JSON Serialization Example
// ============================================

struct Event {
    var name: String
    var year: Int64
}

struct Person {
    var name: String
    var age: Int64
}

// JSON serialization functions (CJson-style)
func eventToJson(e: Event): String {
    return "{\"name\":\"" + e.name + "\",\"year\":" + e.year.toString() + "}"
}

func personToJson(p: Person): String {
    return "{\"name\":\"" + p.name + "\",\"age\":" + p.age.toString() + "}"
}

func eventsToJsonArray(e1: Event, e2: Event, e3: Event): String {
    return "[" + eventToJson(e1) + "," + eventToJson(e2) + "," + eventToJson(e3) + "]"
}

main() {
    println("=== CJson Test Demo ===")
    println("")

    // Test single object serialization
    println("[Test 1] Event -> JSON")
    let christmas = Event { name: "Christmas", year: 2024 }
    println(eventToJson(christmas))

    println("")
    println("[Test 2] Person -> JSON")
    let alice = Person { name: "Alice", age: 28 }
    println(personToJson(alice))

    println("")
    println("[Test 3] Array -> JSON")
    let e1 = Event { name: "NewYear", year: 2025 }
    let e2 = Event { name: "Valentine", year: 2025 }
    let e3 = Event { name: "Easter", year: 2025 }
    println(eventsToJsonArray(e1, e2, e3))

    println("")
    println("=== Test Complete ===")
}
