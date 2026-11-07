"""评分只在预测结束后读取标签；空分母返回 None。"""
import math
import re
import unicodedata


def normalize(text):
    return "".join(c for c in unicodedata.normalize("NFKC", str(text)).casefold() if c.isalnum())


def retrieval_metrics(ranked, qrels):
    # 重复结果不占据新的名次；不同会话的相关性标签不能重复计数。
    ranked = list(dict.fromkeys(ranked))
    relevant = {k for k, grade in qrels.items() if grade > 0}
    if not relevant:
        return {k: None for k in ("recall_at_1", "recall_at_5", "mrr_at_10", "ndcg_at_10")}
    dcg = sum((2 ** qrels.get(doc, 0) - 1) / math.log2(i + 2) for i, doc in enumerate(ranked[:10]))
    ideal = sorted(qrels.values(), reverse=True)[:10]
    idcg = sum((2 ** grade - 1) / math.log2(i + 2) for i, grade in enumerate(ideal))
    return {
        "recall_at_1": len(set(ranked[:1]) & relevant) / len(relevant),
        "recall_at_5": len(set(ranked[:5]) & relevant) / len(relevant),
        "mrr_at_10": next((1 / (i + 1) for i, doc in enumerate(ranked[:10]) if doc in relevant), 0.0),
        "ndcg_at_10": dcg / idcg,
    }


def formation_counts(predictions, gold):
    # 最大二分匹配：一个候选只能命中一个事实，重复候选计入 FP。
    edges = [[i for i, aliases in enumerate(gold) if normalize(p) in {normalize(a) for a in aliases}]
             for p in predictions]
    matched = {}

    def augment(p, visited):
        for g in edges[p]:
            if g not in visited:
                visited.add(g)
                if g not in matched or augment(matched[g], visited):
                    matched[g] = p
                    return True
        return False

    tp = sum(augment(i, set()) for i in range(len(predictions)))
    return {"tp": tp, "fp": len(predictions) - tp, "fn": len(gold) - tp}


def prf(tp, fp, fn):
    return {"precision": tp / (tp + fp) if tp + fp else None,
            "recall": tp / (tp + fn) if tp + fn else None,
            "f1": 2 * tp / (2 * tp + fp + fn) if 2 * tp + fp + fn else None}


def mean(values):
    values = [v for v in values if v is not None]
    return sum(values) / len(values) if values else None


def percentile(values, p):
    if not values:
        return None
    values = sorted(values)
    position = (len(values) - 1) * p
    lo, hi = math.floor(position), math.ceil(position)
    return values[lo] + (values[hi] - values[lo]) * (position - lo)


def answer_metrics(prediction, aliases, unanswerable, refused, stale_facts=()):
    return {"answer_correct": (refused if unanswerable else
                               not refused and normalize(prediction) in {normalize(a) for a in aliases}),
            "refusal_correct": refused if unanswerable else None,
            "stale_fact_error": any(normalize(s) in normalize(prediction) for s in stale_facts)}
