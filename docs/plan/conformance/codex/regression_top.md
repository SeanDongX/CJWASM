# Conformance 回归聚类报告（PASSED -> FAILED）

## 基本信息

- 数据批次：`target/conformance/20260318_110040`
- 总测试数：`29060`
- 关键回归：`PASSED -> FAILED = 11355`
- 反向改善：`FAILED -> PASSED = 198`

## 结果迁移概览（Top）

- `PASSED -> FAILED`: `11355`
- `INCOMPLETE -> FAILED`: `1199`
- `FAILED -> INCOMPLETE`: `1123`
- `FAILED -> PASSED`: `198`
- `PASSED -> ERRORED`: `9`
- `INCOMPLETE -> ERRORED`: `6`
- `FAILED -> ERRORED`: `6`
- `ERRORED -> INCOMPLETE`: `1`

## PASSED -> FAILED 模块 Top 20

| Rank | Module | Count | Share |
|---:|---|---:|---:|
| 1 | `src/tests/04_expressions` | 4734 | 41.69% |
| 2 | `src/tests/06_class_and_interface` | 3096 | 27.27% |
| 3 | `src/tests/02_types` | 1307 | 11.51% |
| 4 | `src/tests/11_packages_and_module_management` | 453 | 3.99% |
| 5 | `src/tests/03_names_scopes_variables_and_modifiers` | 440 | 3.87% |
| 6 | `src/tests/05_function` | 349 | 3.07% |
| 7 | `src/tests/01_lexical_structure` | 343 | 3.02% |
| 8 | `src/tests/13_multi_language_interoperability` | 122 | 1.07% |
| 9 | `src/tests/08_extension` | 118 | 1.04% |
| 10 | `src/tests/16_constant_evaluation` | 111 | 0.98% |
| 11 | `src/tests/10_overloading` | 99 | 0.87% |
| 12 | `src/tests/09_generics` | 51 | 0.45% |
| 13 | `src/tests/12_exceptions` | 41 | 0.36% |
| 14 | `src/tests/07_property` | 36 | 0.32% |
| 15 | `src/regression` | 20 | 0.18% |
| 16 | `src/tests/17_annotation` | 12 | 0.11% |
| 17 | `src/tests/14_metaprogramming` | 10 | 0.09% |
| 18 | `src/tests/a_cangjie_grammar_summary` | 7 | 0.06% |
| 19 | `src/tests/15_concurrency` | 6 | 0.05% |

## Top 10 模块示例用例（每模块最多 5 条）

### 1. `src/tests/04_expressions` (4734)
- `/Users/sean/workspace/cangjie_oss/CJWasm2/third_party/cangjie_test/Conformance/Compiler/testsuite/src/tests/04_expressions/01_literals/a01/test_a01_18.cj`
- `/Users/sean/workspace/cangjie_oss/CJWasm2/third_party/cangjie_test/Conformance/Compiler/testsuite/src/tests/04_expressions/01_literals/a01/test_a01_19.cj`
- `/Users/sean/workspace/cangjie_oss/CJWasm2/third_party/cangjie_test/Conformance/Compiler/testsuite/src/tests/04_expressions/01_literals/a01/test_a01_20.cj`
- `/Users/sean/workspace/cangjie_oss/CJWasm2/third_party/cangjie_test/Conformance/Compiler/testsuite/src/tests/04_expressions/01_literals/a01/test_a01_21.cj`
- `/Users/sean/workspace/cangjie_oss/CJWasm2/third_party/cangjie_test/Conformance/Compiler/testsuite/src/tests/04_expressions/01_literals/a01/test_a01_23.cj`

### 2. `src/tests/06_class_and_interface` (3096)
- `/Users/sean/workspace/cangjie_oss/CJWasm2/third_party/cangjie_test/Conformance/Compiler/testsuite/src/tests/06_class_and_interface/01_class/01_class_definition/01_class_modifiers/a06/test_a06_02.cj`
- `/Users/sean/workspace/cangjie_oss/CJWasm2/third_party/cangjie_test/Conformance/Compiler/testsuite/src/tests/06_class_and_interface/01_class/01_class_definition/01_class_modifiers/a06/test_a06_03.cj`
- `/Users/sean/workspace/cangjie_oss/CJWasm2/third_party/cangjie_test/Conformance/Compiler/testsuite/src/tests/06_class_and_interface/01_class/01_class_definition/01_class_modifiers/a06/test_a06_04.cj`
- `/Users/sean/workspace/cangjie_oss/CJWasm2/third_party/cangjie_test/Conformance/Compiler/testsuite/src/tests/06_class_and_interface/01_class/01_class_definition/01_class_modifiers/a06/test_a06_05.cj`
- `/Users/sean/workspace/cangjie_oss/CJWasm2/third_party/cangjie_test/Conformance/Compiler/testsuite/src/tests/06_class_and_interface/01_class/01_class_definition/01_class_modifiers/a06/test_a06_06.cj`

### 3. `src/tests/02_types` (1307)
- `/Users/sean/workspace/cangjie_oss/CJWasm2/third_party/cangjie_test/Conformance/Compiler/testsuite/src/tests/02_types/01_value_types/01_numeric_types/01_numeric_literals/a03/test_a03_03.cj`
- `/Users/sean/workspace/cangjie_oss/CJWasm2/third_party/cangjie_test/Conformance/Compiler/testsuite/src/tests/02_types/01_value_types/01_numeric_types/01_numeric_literals/a03/test_a03_04.cj`
- `/Users/sean/workspace/cangjie_oss/CJWasm2/third_party/cangjie_test/Conformance/Compiler/testsuite/src/tests/02_types/01_value_types/01_numeric_types/01_numeric_literals/a03/test_a03_05.cj`
- `/Users/sean/workspace/cangjie_oss/CJWasm2/third_party/cangjie_test/Conformance/Compiler/testsuite/src/tests/02_types/01_value_types/01_numeric_types/01_numeric_literals/a04/test_a04_02.cj`
- `/Users/sean/workspace/cangjie_oss/CJWasm2/third_party/cangjie_test/Conformance/Compiler/testsuite/src/tests/02_types/01_value_types/01_numeric_types/01_numeric_literals/a04/test_a04_03.cj`

### 4. `src/tests/11_packages_and_module_management` (453)
- `/Users/sean/workspace/cangjie_oss/CJWasm2/third_party/cangjie_test/Conformance/Compiler/testsuite/src/tests/11_packages_and_module_management/01_packages/01_package_declaration/01_package_names/a03/test_a03_02.cj`
- `/Users/sean/workspace/cangjie_oss/CJWasm2/third_party/cangjie_test/Conformance/Compiler/testsuite/src/tests/11_packages_and_module_management/01_packages/01_package_declaration/a08/test_a08_01.cj`
- `/Users/sean/workspace/cangjie_oss/CJWasm2/third_party/cangjie_test/Conformance/Compiler/testsuite/src/tests/11_packages_and_module_management/01_packages/01_package_declaration/a11/test_a11_01.cj`
- `/Users/sean/workspace/cangjie_oss/CJWasm2/third_party/cangjie_test/Conformance/Compiler/testsuite/src/tests/11_packages_and_module_management/01_packages/02_package_members/a01/test_a01_12.cj`
- `/Users/sean/workspace/cangjie_oss/CJWasm2/third_party/cangjie_test/Conformance/Compiler/testsuite/src/tests/11_packages_and_module_management/01_packages/02_package_members/a01/test_a01_16.cj`

### 5. `src/tests/03_names_scopes_variables_and_modifiers` (440)
- `/Users/sean/workspace/cangjie_oss/CJWasm2/third_party/cangjie_test/Conformance/Compiler/testsuite/src/tests/03_names_scopes_variables_and_modifiers/01_names/a01/test_a01_11.cj`
- `/Users/sean/workspace/cangjie_oss/CJWasm2/third_party/cangjie_test/Conformance/Compiler/testsuite/src/tests/03_names_scopes_variables_and_modifiers/01_names/a02/test_a02_005.cj`
- `/Users/sean/workspace/cangjie_oss/CJWasm2/third_party/cangjie_test/Conformance/Compiler/testsuite/src/tests/03_names_scopes_variables_and_modifiers/01_names/a02/test_a02_028.cj`
- `/Users/sean/workspace/cangjie_oss/CJWasm2/third_party/cangjie_test/Conformance/Compiler/testsuite/src/tests/03_names_scopes_variables_and_modifiers/01_names/a02/test_a02_038.cj`
- `/Users/sean/workspace/cangjie_oss/CJWasm2/third_party/cangjie_test/Conformance/Compiler/testsuite/src/tests/03_names_scopes_variables_and_modifiers/01_names/a02/test_a02_054.cj`

### 6. `src/tests/05_function` (349)
- `/Users/sean/workspace/cangjie_oss/CJWasm2/third_party/cangjie_test/Conformance/Compiler/testsuite/src/tests/05_function/01_function_definition/01_function_modifiers/01_modifier_of_global_functions/a02/test_a02_03.cj`
- `/Users/sean/workspace/cangjie_oss/CJWasm2/third_party/cangjie_test/Conformance/Compiler/testsuite/src/tests/05_function/01_function_definition/01_function_modifiers/01_modifier_of_global_functions/a03/test_a03_02.cj`
- `/Users/sean/workspace/cangjie_oss/CJWasm2/third_party/cangjie_test/Conformance/Compiler/testsuite/src/tests/05_function/01_function_definition/01_function_modifiers/01_modifier_of_global_functions/a04/test_a04_02.cj`
- `/Users/sean/workspace/cangjie_oss/CJWasm2/third_party/cangjie_test/Conformance/Compiler/testsuite/src/tests/05_function/01_function_definition/01_function_modifiers/02_modifier_of_local_functions/a01/test_a01_02.cj`
- `/Users/sean/workspace/cangjie_oss/CJWasm2/third_party/cangjie_test/Conformance/Compiler/testsuite/src/tests/05_function/01_function_definition/01_function_modifiers/03_modifier_of_member_functions/a01/test_a01_02.cj`

### 7. `src/tests/01_lexical_structure` (343)
- `/Users/sean/workspace/cangjie_oss/CJWasm2/third_party/cangjie_test/Conformance/Compiler/testsuite/src/tests/01_lexical_structure/01_identifiers_and_keywords/a06/test_a06_10.cj`
- `/Users/sean/workspace/cangjie_oss/CJWasm2/third_party/cangjie_test/Conformance/Compiler/testsuite/src/tests/01_lexical_structure/01_identifiers_and_keywords/a06/test_a06_24.cj`
- `/Users/sean/workspace/cangjie_oss/CJWasm2/third_party/cangjie_test/Conformance/Compiler/testsuite/src/tests/01_lexical_structure/02_semicolons_and_newline_characters/a01/test_a01_03.cj`
- `/Users/sean/workspace/cangjie_oss/CJWasm2/third_party/cangjie_test/Conformance/Compiler/testsuite/src/tests/01_lexical_structure/02_semicolons_and_newline_characters/a01/test_a01_04.cj`
- `/Users/sean/workspace/cangjie_oss/CJWasm2/third_party/cangjie_test/Conformance/Compiler/testsuite/src/tests/01_lexical_structure/02_semicolons_and_newline_characters/a01/test_a01_05.cj`

### 8. `src/tests/13_multi_language_interoperability` (122)
- `/Users/sean/workspace/cangjie_oss/CJWasm2/third_party/cangjie_test/Conformance/Compiler/testsuite/src/tests/13_multi_language_interoperability/01_c_interoperability/01_unsafe_context/a01/test_a01_01.cj`
- `/Users/sean/workspace/cangjie_oss/CJWasm2/third_party/cangjie_test/Conformance/Compiler/testsuite/src/tests/13_multi_language_interoperability/01_c_interoperability/01_unsafe_context/a02/test_a02_01.cj`
- `/Users/sean/workspace/cangjie_oss/CJWasm2/third_party/cangjie_test/Conformance/Compiler/testsuite/src/tests/13_multi_language_interoperability/01_c_interoperability/01_unsafe_context/a02/test_a02_02.cj`
- `/Users/sean/workspace/cangjie_oss/CJWasm2/third_party/cangjie_test/Conformance/Compiler/testsuite/src/tests/13_multi_language_interoperability/01_c_interoperability/01_unsafe_context/a02/test_a02_19.cj`
- `/Users/sean/workspace/cangjie_oss/CJWasm2/third_party/cangjie_test/Conformance/Compiler/testsuite/src/tests/13_multi_language_interoperability/01_c_interoperability/01_unsafe_context/a03/test_a03_02.cj`

### 9. `src/tests/08_extension` (118)
- `/Users/sean/workspace/cangjie_oss/CJWasm2/third_party/cangjie_test/Conformance/Compiler/testsuite/src/tests/08_extension/01_extension_syntax/02_interface_extensions/a02/test_a02_02.cj`
- `/Users/sean/workspace/cangjie_oss/CJWasm2/third_party/cangjie_test/Conformance/Compiler/testsuite/src/tests/08_extension/01_extension_syntax/02_interface_extensions/a02/test_a02_03.cj`
- `/Users/sean/workspace/cangjie_oss/CJWasm2/third_party/cangjie_test/Conformance/Compiler/testsuite/src/tests/08_extension/01_extension_syntax/02_interface_extensions/a02/test_a02_04.cj`
- `/Users/sean/workspace/cangjie_oss/CJWasm2/third_party/cangjie_test/Conformance/Compiler/testsuite/src/tests/08_extension/01_extension_syntax/02_interface_extensions/a04/test_a04_02.cj`
- `/Users/sean/workspace/cangjie_oss/CJWasm2/third_party/cangjie_test/Conformance/Compiler/testsuite/src/tests/08_extension/01_extension_syntax/02_interface_extensions/a08/test_a08_01.cj`

### 10. `src/tests/16_constant_evaluation` (111)
- `/Users/sean/workspace/cangjie_oss/CJWasm2/third_party/cangjie_test/Conformance/Compiler/testsuite/src/tests/16_constant_evaluation/01_const_variables/a01/test_a01_02.cj`
- `/Users/sean/workspace/cangjie_oss/CJWasm2/third_party/cangjie_test/Conformance/Compiler/testsuite/src/tests/16_constant_evaluation/01_const_variables/a01/test_a01_03.cj`
- `/Users/sean/workspace/cangjie_oss/CJWasm2/third_party/cangjie_test/Conformance/Compiler/testsuite/src/tests/16_constant_evaluation/01_const_variables/a01/test_a01_04.cj`
- `/Users/sean/workspace/cangjie_oss/CJWasm2/third_party/cangjie_test/Conformance/Compiler/testsuite/src/tests/16_constant_evaluation/01_const_variables/a02/test_a02_04.cj`
- `/Users/sean/workspace/cangjie_oss/CJWasm2/third_party/cangjie_test/Conformance/Compiler/testsuite/src/tests/16_constant_evaluation/01_const_variables/a02/test_a02_05.cj`

## FAILED -> PASSED 模块 Top 10（改善参考）

| Rank | Module | Count | Share |
|---:|---|---:|---:|
| 1 | `src/tests/02_types` | 33 | 16.67% |
| 2 | `src/tests/01_lexical_structure` | 30 | 15.15% |
| 3 | `src/tests/11_packages_and_module_management` | 29 | 14.65% |
| 4 | `src/tests/04_expressions` | 24 | 12.12% |
| 5 | `src/tests/07_property` | 19 | 9.60% |
| 6 | `src/tests/13_multi_language_interoperability` | 12 | 6.06% |
| 7 | `src/tests/03_names_scopes_variables_and_modifiers` | 10 | 5.05% |
| 8 | `src/tests/14_metaprogramming` | 9 | 4.55% |
| 9 | `src/tests/05_function` | 7 | 3.54% |
| 10 | `src/tests/08_extension` | 6 | 3.03% |

---
完整回归明细见：`docs/plan/comformance/passed_to_failed_20260318_110040.csv`
