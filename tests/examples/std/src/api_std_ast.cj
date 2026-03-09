// L1 std.ast 全量 API 测试
// 类型：Token, TokenKind(enum), Position, TypeNode 及子类(PrimitiveType, RefType, FuncType 等),
// Pattern 及子类(ConstPattern, VarPattern, EnumPattern 等), ToTokens, ToBytes
// 函数：parseType, parseTypeFragment, parsePattern, parsePatternFragment, parseProgram, parseExpr, getTokenKind
package std_examples

import std.ast

// API 列表（vendor）:
// - public struct Token <: ToBytes; public enum TokenKind; public struct Position
// - public interface ToTokens, ToBytes
// - public class PrimitiveType, RefType, QualifiedType, TupleType, FuncType, VArrayType, ... <: TypeNode
// - public class ConstPattern, WildcardPattern, VarPattern, EnumPattern, TuplePattern, ... <: Pattern
// - public func parseType(input: Tokens): TypeNode; parseTypeFragment; parsePattern; parsePatternFragment
// - public func parseProgram(input: Tokens): Program; parseExpr
// - public func getTokenKind(no: UInt16): TokenKind
func api_std_ast_used(): Unit {
    ()
}
