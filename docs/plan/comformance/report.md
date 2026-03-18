# Conformance 对比报告（2026-03-18 11:00:40）

## 运行信息

- 对比批次目录：`target/conformance/20260318_110040`
- 原始差异报告：`docs/plan/comformance/diff_20260318_110040.txt`
- 基线（cjc）日志：`target/conformance/20260318_110040/cjc.log`
- 候选（cjwasm）日志：`target/conformance/20260318_110040/cjwasm.log`

## 汇总结果

总测试数：`29060`

- `cjc`：Passed `23080`，Failed `2672`，Errored `2`，Skipped `2`，Incomplete `3304`
- `cjwasm`：Passed `11914`，Failed `13899`，Errored `22`，Skipped `2`，Incomplete `3223`

## 差异概览（来自 run_diff）

- different/same results：`13897 / 15163`
- FAILED -> PASSED：`198`
- PASSED -> FAILED：`11355`
- FAILED -> INCOMPLETE：`1123`
- INCOMPLETE -> FAILED：`1199`
- PASSED -> ERRORED：`9`
- INCOMPLETE -> ERRORED：`6`
- FAILED -> ERRORED：`6`
- ERRORED -> INCOMPLETE：`1`
- different/same logs：`29047 / 13`

## 复现命令

```bash
./scripts/conformance_diff.sh
```
