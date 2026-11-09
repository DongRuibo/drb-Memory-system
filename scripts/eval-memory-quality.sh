#!/usr/bin/env bash
# 只用隔离临时PG及本地Tokenizer；没有收费模型入口，不加载.env。
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
quality_mode="${1:-prepare}"
quality_output="${2:-evals/runs/memory-quality-$(date -u +%Y%m%dT%H%M%SZ)}"
export PYTHONDONTWRITEBYTECODE=1
if [[ -d /tmp/drb-phase3-deps ]]; then
  export PYTHONPATH="/tmp/drb-phase3-deps${PYTHONPATH:+:$PYTHONPATH}"
fi
case "$quality_mode" in
  prepare)
    python3 -m evals.memory_eval.quality prepare --output "$quality_output"
    ;;
  test)
    MEMORY_QUALITY_PG_TEST=1 python3 -m unittest discover -s evals/tests -v
    ;;
  run)
    # 未确认标签时结果仍明确标为待复核，不冒充人工确认成绩。
    python3 -m evals.memory_eval.quality run --output "$quality_output" --assets "${QUALITY_TOKENIZER_ASSETS:-/tmp/drb-phase3-assets}"
    ;;
  *)
    echo '用法：scripts/eval-memory-quality.sh prepare|test|run [全新输出目录]' >&2
    exit 2
    ;;
esac
