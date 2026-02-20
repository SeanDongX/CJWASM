package cjson_demo

// CJson Demo for cjwasm

struct Event {
    var name: String
    var year: Int64
}

struct Person {
    var name: String
    var age: Int64
}

func eventToJson(e: Event): String {
    return "{\"name\":\"" + e.name + "\",\"year\":" + e.year.toString() + "}"
}

func personToJson(p: Person): String {
    return "{\"name\":\"" + p.name + "\",\"age\":" + p.age.toString() + "}"
}

// Fixed array serialization
func eventsToJsonArray(e1: Event, e2: Event, e3: Event): String {
    return "[" + eventToJson(e1) + "," + eventToJson(e2) + "," + eventToJson(e3) + "]"
}

func personsToJsonArray(p1: Person, p2: Person, p3: Person): String {
    return "[" + personToJson(p1) + "," + personToJson(p2) + "," + personToJson(p3) + "]"
}

main() {
    println("============================================")
    println("     CJson Demo - cjwasm Compilation")
    println("============================================")
    println("")

    // Test 1: Single Event
    println("[Test 1] Single Event Serialization")
    let christmas = Event { name: "Christmas", year: 2024 }
    println("  -> " + eventToJson(christmas))

    // Test 2: Single Person  
    println("")
    println("[Test 2] Single Person Serialization")
    let alice = Person { name: "Alice", age: 28 }
    println("  -> " + personToJson(alice))

    // Test 3: Multiple Events
    println("")
    println("[Test 3] Event Array Serialization")
    let e1 = Event { name: "NewYear", year: 2025 }
    let e2 = Event { name: "Valentine", year: 2025 }
    let e3 = Event { name: "Easter", year: 2025 }
    println("  -> " + eventsToJsonArray(e1, e2, e3))

    // Test 4: Multiple Persons
    println("")
    println("[Test 4] Person Array Serialization")
    let p1 = Person { name: "Bob", age: 35 }
    let p2 = Person { name: "Carol", age: 29 }
    let p3 = Person { name: "Dave", age: 42 }
    println("  -> " + personsToJsonArray(p1, p2, p3))

    // Test 5: Loop serialization
    println("")
    println("[Test 5] Loop Serialization")
    let events = [
        Event { name: "Jan", year: 2025 },
        Event { name: "Feb", year: 2025 },
        Event { name: "Mar", year: 2025 }
    ]
    for i in 0..3 {
        println("  [" + i.toString() + "] " + eventToJson(events[i]))
    }

    println("")
    println("============================================")
    println("          Demo Complete!")
    println("============================================")
}
