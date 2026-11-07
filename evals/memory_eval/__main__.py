import argparse
import json
from pathlib import Path
from .runner import run

parser = argparse.ArgumentParser(description="Memory 离线评测：仅自建临时 PostgreSQL，在线调用/费用上限均为零")
parser.add_argument("--dataset", default="evals/datasets/memory_v1.json")
parser.add_argument("--output", required=True, help="必须是不存在的新目录")
parser.add_argument("--seed", type=int, default=20261106)
parser.add_argument("--budget", type=int, default=20)
parser.add_argument("--repeats", type=int, default=3)
parser.add_argument("--longmemeval", help="官方完整 JSON 文件路径；只选择固定种子子集")
parser.add_argument("--variant", choices=["s", "m", "oracle"], default="s")
parser.add_argument("--limit", type=int, default=12)
parser.add_argument("--statement-timeout-ms",type=int,default=3000)
args = parser.parse_args()
if args.budget < 10 or args.budget > 50 or args.budget % 2 or not 1 <= args.repeats <= 10 or not 1 <= args.limit <= 500:
    parser.error("budget 必须为 10..50 偶数；repeats 为 1..10；limit 为 1..500")
if not 1000 <= args.statement_timeout_ms <= 10000:
    parser.error("statement-timeout-ms 必须为 1000..10000")
if Path(args.output).exists():
    parser.error("output 已存在；请选择新目录，避免覆盖既有报告")
try:
    run(args)
except Exception as error:
    # 基础设施失败也保留已完成题目，不把失败运行展示为完整成绩。
    manifest=Path(args.output)/"manifest.json"
    if manifest.exists():
        data=json.loads(manifest.read_text())
        data.update(status="failed",error_type=type(error).__name__,error=str(error))
        manifest.write_text(json.dumps(data,ensure_ascii=False,indent=2)+"\n")
    raise
