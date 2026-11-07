"""LongMemEval 独立 Adapter：标签与写入/查询对象分离。"""
import hashlib
import json
import random
import re
from datetime import datetime, timezone
from pathlib import Path


def timestamp(value):
    # 官方日期没有时区；适配统一按 UTC 保存，并在 provenance 显式记录假设。
    value=re.sub(r"\s+\([^)]*\)","",value)
    return datetime.strptime(value,"%Y/%m/%d %H:%M").replace(tzinfo=timezone.utc).isoformat()


def adapt(path, limit=12, seed=20261106, variant="s"):
    raw = Path(path).read_bytes()
    rows = json.loads(raw)
    required = {"question_id", "question_type", "question", "answer", "question_date",
                "haystack_dates", "haystack_session_ids", "haystack_sessions", "answer_session_ids"}
    if not isinstance(rows, list) or not rows or any(not required <= row.keys() for row in rows):
        raise ValueError("invalid official LongMemEval array schema")
    if len({r["question_id"] for r in rows}) != len(rows):
        raise ValueError("duplicate question_id")
    if not 1 <= limit <= len(rows) or variant not in {"s", "m", "oracle"}:
        raise ValueError("invalid sample limit or variant")
    # 按题型轮转，先确定样本，绝不按成功/失败筛选。
    rng = random.Random(seed)
    groups = {}
    for row in sorted(rows, key=lambda r: r["question_id"]):
        group = "abstention" if "_abs" in row["question_id"] else row["question_type"]
        groups.setdefault(group, []).append(row)
    for group in groups.values():
        rng.shuffle(group)
    selected = []
    while len(selected) < limit:
        for group in sorted(groups):
            if groups[group] and len(selected) < limit:
                selected.append(groups[group].pop())
    cases = []
    for row in selected:
        ids, dates, sessions = row["haystack_session_ids"], row["haystack_dates"], row["haystack_sessions"]
        if len(ids) != len(sessions) or len(ids) != len(dates) or len(ids) != len(set(ids)):
            raise ValueError("unaligned or duplicate sessions")
        if not set(row["answer_session_ids"]) <= set(ids):
            raise ValueError("unknown evidence session")
        documents = []
        # 保留全部角色及干扰会话，不看 has_answer；按时间输入。
        for date, sid, session in sorted(zip(dates, ids, sessions)):
            for turn_no, turn in enumerate(session):
                if turn.get("role") not in {"user", "assistant"} or not isinstance(turn.get("content"), str):
                    raise ValueError("invalid history turn")
                content = re.sub(r"\s+", " ", turn["content"]).strip()
                for chunk_no, start in enumerate(range(0, len(content), 1000)):
                    chunk = content[start:start + 1000].strip()
                    if not chunk:
                        continue
                    documents.append({"id": f"{sid}:{turn_no}:{chunk_no}", "session_id": sid,
                                      "occurred_at":timestamp(date),
                                      "content": f"[{date}] {turn['role']}: {chunk}"})
        cases.append({"input": {"id": row["question_id"], "query": row["question"],
                                 "question_date": row["question_date"], "documents": documents},
                      "labels": {"qrels": {} if "_abs" in row["question_id"] else {s: 1 for s in row["answer_session_ids"]},
                                 "reference_session_ids": row["answer_session_ids"],
                                 "answers": [str(row["answer"])],
                                 "unanswerable": "_abs" in row["question_id"],
                                 "question_type": row["question_type"]}})
    return {"provenance": {"kind": "public_subset", "benchmark": "LongMemEval", "variant": variant,
                           "source_sha256": hashlib.sha256(raw).hexdigest(), "source_rows": len(rows),
                           "selected_ids": [c["input"]["id"] for c in cases], "seed": seed,
                           "selection": "seeded question-type round-robin including abstention",
                           "official_score": False, "chunk_chars": 1000,
                           "timezone_assumption":"naive upstream timestamps interpreted as UTC"}, "cases": cases}
