# 执行证据索引

真实Judge校准未通过，已停止，正式14题未运行。

- report.md：完整报告和16例实际判定
- summary.json：汇总（正式质量为null）
- calls.jsonl：18次原始输出、usage、费用
- traces/：完整请求与响应，输入SHA与账本一致
- judge-calibration-summary.json：15/16整例匹配，1个关键错误
- validation.json：冻结协议、原有32文件及费用保护核对
- authorization.json、approved-plan.json、price-verification.json：授权、参数及实时价格
- offline-tests.log、build.log：收费前免费检查证据

不删除原claim或账本，不自动重跑。
