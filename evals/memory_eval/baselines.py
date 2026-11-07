"""无模型、无网络的透明对照；不宣称具备语义理解能力。"""
import math
import re
from collections import Counter

from .metrics import normalize

STOPWORDS = set("a an the i my me you your is are was were do did does what which when where how to of in for on and it with have has this that can please".split())


def tokens(text):
    words = re.findall(r"[a-z0-9_]+|[\u4e00-\u9fff]+", text.casefold())
    result = []
    for word in words:
        if "\u4e00" <= word[0] <= "\u9fff":
            result.extend(word[i:i + 2] for i in range(max(1, len(word) - 1)))
        elif word not in STOPWORDS:
            result.append(word)
    return result


class TfidfIndex:
    def __init__(self, documents):
        self.documents = documents
        counts = [Counter(tokens(d["content"])) for d in documents]
        df = Counter(t for c in counts for t in c)
        self.idf = {t: math.log((1 + len(documents)) / (1 + n)) + 1 for t, n in df.items()}
        self.vectors = [self.vector(c) for c in counts]

    def vector(self, counts):
        weighted = {t: (1 + math.log(n)) * self.idf[t] for t, n in counts.items() if t in self.idf}
        norm = math.sqrt(sum(v * v for v in weighted.values()))
        return {t: v / norm for t, v in weighted.items()} if norm else {}

    def search(self, query, limit):
        q = self.vector(Counter(tokens(query)))
        scores = [(sum(q.get(t, 0) * v for t, v in vector.items()), doc["id"])
                  for doc, vector in zip(self.documents, self.vectors)]
        return [(doc, score) for score, doc in sorted(scores, key=lambda x: (-x[0], x[1]))[:limit] if score > 0]


def extract_rules(text):
    predictions = []
    for sentence in re.split(r"[。！？!?；;\n]", text):
        sentence = sentence.strip().rstrip(".")
        if not sentence or any(t in sentence for t in ("如果", "假如", "可能", "他说", "她说", "比如", "不要记", "请忘", "吗", "是否")):
            continue
        if re.match(r"^(我(?:喜欢|不喜欢|偏好|住在|在|是|已经|昨天|今天|的|使用)|I (?:like|live|work|prefer|am|use)\b)", sentence, re.I):
            predictions.append(sentence)
    return list({normalize(s): s for s in predictions}.values())


def read_extractive(query, items):
    # 只读 Context Pack；没有答案、相关性标注、has_answer 或原始全历史。
    q = set(tokens(query))
    candidates = []
    for item in items:
        for sentence in re.split(r"(?<=[.!?。！？])\s+|[\n]", item["text"]):
            sentence = sentence.strip()
            if sentence:
                overlap = len(q & set(tokens(sentence))) / max(1, len(q))
                candidates.append((overlap, sentence, item["memory_version_id"]))
    if not candidates:
        return {"text": "信息不足，无法回答。", "refused": True, "citations": []}
    best = max(candidates, key=lambda x: x[0])
    if best[0] < 0.2:
        return {"text": "信息不足，无法回答。", "refused": True, "citations": []}
    return {"text": best[1], "refused": False, "citations": [best[2]]}
