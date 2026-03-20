#!/usr/bin/env python3
"""Post-process conformance reports and print warning/shim-focused stats."""

from __future__ import annotations

import argparse
import json
import re
from collections import Counter
from pathlib import Path
from typing import Dict, Iterable, List, Tuple


WARNING_RE = re.compile(r"warning:", re.IGNORECASE)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Analyze warning/staticlib/macro related conformance differences."
    )
    parser.add_argument("cjc_report", help="Path to cjc harness json report")
    parser.add_argument("cjwasm_report", help="Path to cjwasm harness json report")
    parser.add_argument(
        "--max-list",
        type=int,
        default=30,
        help="Maximum listed test paths per category (default: 30)",
    )
    return parser.parse_args()


def normalize_path(path: str) -> str:
    p = (path or "").replace("\\", "/")
    if p.startswith("./"):
        p = p[2:]
    return p


def load_tests(path: Path) -> Dict[Tuple[str, str], dict]:
    raw = json.loads(path.read_text(encoding="utf-8"))
    tests: Dict[Tuple[str, str], dict] = {}
    for item in raw:
        if not isinstance(item, dict):
            continue
        if "name" not in item or "result" not in item or "test_path" not in item:
            continue
        key = (str(item.get("name", "")), normalize_path(str(item.get("test_path", ""))))
        tests[key] = item
    return tests


def has_warning(test_item: dict) -> bool:
    compile_log = str(test_item.get("compile_log", ""))
    execute_log = str(test_item.get("execute_log", ""))
    return bool(WARNING_RE.search(compile_log) or WARNING_RE.search(execute_log))


def compile_warning_expect(test_item: dict) -> str:
    return str(test_item.get("compile_warning", "")).strip().lower()


def is_aux_or_shim_related(test_item: dict) -> bool:
    test_path = normalize_path(str(test_item.get("test_path", ""))).lower()
    compile_log = str(test_item.get("compile_log", "")).lower()

    if "/aux_" in test_path or test_path.endswith("/aux.cj"):
        return True

    deps = test_item.get("dependencies")
    if isinstance(deps, list) and len(deps) > 0:
        return True

    if bool(test_item.get("macrolib")):
        return True

    if "--output-type=staticlib" in compile_log or "--compile-macro" in compile_log:
        return True

    return False


def format_list(title: str, items: Iterable[str], max_items: int) -> List[str]:
    items = list(items)
    lines = [f"{title}: {len(items)}"]
    for path in items[:max_items]:
        lines.append(f"  - {path}")
    if len(items) > max_items:
        lines.append(f"  ... ({len(items) - max_items} more)")
    return lines


def main() -> int:
    args = parse_args()
    cjc_report = Path(args.cjc_report)
    cjwasm_report = Path(args.cjwasm_report)

    cjc_tests = load_tests(cjc_report)
    cjwasm_tests = load_tests(cjwasm_report)

    cjc_keys = set(cjc_tests.keys())
    cjwasm_keys = set(cjwasm_tests.keys())
    common_keys = cjc_keys & cjwasm_keys

    result_diff_keys = []
    result_transitions: Counter[str] = Counter()

    warning_expect_yes_diffs: List[str] = []
    warning_missing_on_cjwasm: List[str] = []
    warning_unexpected_on_cjwasm: List[str] = []
    warning_only_like_diffs: List[str] = []
    aux_or_shim_related_diffs: List[str] = []

    for key in common_keys:
        cjc_item = cjc_tests[key]
        cjwasm_item = cjwasm_tests[key]

        cjc_res = str(cjc_item.get("result", ""))
        cjwasm_res = str(cjwasm_item.get("result", ""))
        if cjc_res == cjwasm_res:
            continue

        test_path = normalize_path(str(cjc_item.get("test_path", key[1])))
        result_diff_keys.append(test_path)
        result_transitions[f"{cjc_res} -> {cjwasm_res}"] += 1

        expected = compile_warning_expect(cjc_item)
        cjc_has_warn = has_warning(cjc_item)
        cjwasm_has_warn = has_warning(cjwasm_item)

        if expected == "yes":
            warning_expect_yes_diffs.append(test_path)
            if cjc_has_warn and not cjwasm_has_warn:
                warning_missing_on_cjwasm.append(test_path)
            if (
                cjc_has_warn
                and not cjwasm_has_warn
                and cjc_res == "FAILED"
                and cjwasm_res in {"PASSED", "INCOMPLETE"}
            ):
                warning_only_like_diffs.append(test_path)

        if expected == "no" and (not cjc_has_warn) and cjwasm_has_warn:
            warning_unexpected_on_cjwasm.append(test_path)

        if is_aux_or_shim_related(cjc_item) or is_aux_or_shim_related(cjwasm_item):
            aux_or_shim_related_diffs.append(test_path)

    print("Conformance Post Stats")
    print("======================")
    print(f"cjc tests: {len(cjc_tests)}")
    print(f"cjwasm tests: {len(cjwasm_tests)}")
    print(f"common tests: {len(common_keys)}")
    print(f"added/removed tests: {len(cjwasm_keys - cjc_keys)}/{len(cjc_keys - cjwasm_keys)}")
    print(f"result diffs: {len(result_diff_keys)}")
    print()

    if result_transitions:
        print("Result Transition Top:")
        for transition, count in result_transitions.most_common(10):
            print(f"  {transition}: {count}")
        print()

    for line in format_list(
        "diffs where expected compile_warning=yes", warning_expect_yes_diffs, args.max_list
    ):
        print(line)
    print()

    for line in format_list(
        "warning missing on cjwasm (cjc has warning, cjwasm no warning)",
        warning_missing_on_cjwasm,
        args.max_list,
    ):
        print(line)
    print()

    for line in format_list(
        "warning-only-like diffs (FAILED -> PASSED/INCOMPLETE due warning gap)",
        warning_only_like_diffs,
        args.max_list,
    ):
        print(line)
    print()

    for line in format_list(
        "unexpected warnings on cjwasm for compile_warning=no",
        warning_unexpected_on_cjwasm,
        args.max_list,
    ):
        print(line)
    print()

    for line in format_list(
        "aux/staticlib/macro related diffs (heuristic)", aux_or_shim_related_diffs, args.max_list
    ):
        print(line)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
