"""Runner 自建临时 PG；没有连接既有服务或在线模型的入口。"""
import hashlib
import json
import os
from pathlib import Path
import platform
import random
import selectors
import subprocess
import sys
import time
import uuid

from .baselines import TfidfIndex, extract_rules, read_extractive
from .longmemeval import adapt, timestamp
from .metrics import answer_metrics, formation_counts, mean, percentile, prf, retrieval_metrics

ROOT = Path(__file__).resolve().parents[2]
METHODS = ("bm25", "vector_tfidf", "hybrid", "hybrid_rerank")
TARGET = Path("/tmp/drb-memory-eval-target")


def command(args, **kwargs):
    return subprocess.check_output(args, text=True, timeout=120, **kwargs).strip()


def write_json(path, value):
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2, allow_nan=False) + "\n")


def validate_dataset(d):
    if d.get("kind") != "self_constructed" or not d.get("version"):
        raise ValueError("dataset requires kind=self_constructed and version")
    ids = [doc["id"] for doc in d["documents"]]
    if len(ids) != len(set(ids)) or not ids:
        raise ValueError("empty or duplicate document IDs")
    for doc in d["documents"]:
        if not isinstance(doc["content"], str) or not 1 <= len(doc["content"].encode()) <= 8192:
            raise ValueError("invalid document content")
    for section in ("queries", "formation", "qa"):
        if not d[section] or len({q["id"] for q in d[section]}) != len(d[section]):
            raise ValueError("empty or duplicate case IDs")
    for q in d["queries"] + d["qa"]:
        if not isinstance(q["query"], str) or not q["query"].strip() or not set(q["qrels"]) <= set(ids):
            raise ValueError("invalid query or unknown qrel document")
        if any(type(g) is not int or not 0 <= g <= 3 for g in q["qrels"].values()):
            raise ValueError("relevance grade must be an integer in [0,3]")
    if any(not any(g > 0 for g in q["qrels"].values()) for q in d["queries"]):
        raise ValueError("positive retrieval query requires relevant documents")
    for q in d["qa"]:
        if type(q["unanswerable"]) is not bool or not isinstance(q["answers"], list):
            raise ValueError("invalid QA labels")
        if not q["unanswerable"] and not q["answers"]:
            raise ValueError("answerable QA requires aliases")
    for case in d["formation"]:
        if not isinstance(case["text"], str) or not isinstance(case["facts"], list) or any(
                not aliases or any(not isinstance(a, str) for a in aliases) for aliases in case["facts"]):
            raise ValueError("invalid formation fact aliases")


class Sandbox:
    def __init__(self,statement_timeout_ms=3000):
        self.statement_timeout_ms=statement_timeout_ms

    def __enter__(self):
        self.name = "drb-eval-" + uuid.uuid4().hex[:12]
        self.database = "drb_eval_" + uuid.uuid4().hex[:12]
        self.process = None
        self.image = command(["docker", "image", "inspect", "postgres:18.4-bookworm", "--format", "{{.Id}}"])
        command(["docker", "run", "-d", "--name", self.name, "--label", "io.drb.eval=true",
                 "--tmpfs", "/var/lib/postgresql", "-p", "127.0.0.1::5432",
                 "-e", "POSTGRES_HOST_AUTH_METHOD=trust", self.image,
                 "-c","shared_preload_libraries=pg_stat_statements"])
        return self

    def start(self, log):
        for _ in range(60):
            # 镜像初始化会短暂启动仅 Unix socket 可用的临时服务器；等待最终 TCP 监听。
            probe = subprocess.run(["docker", "exec", self.name, "pg_isready", "-h", "127.0.0.1", "-U", "postgres"], capture_output=True)
            if probe.returncode == 0:
                break
            time.sleep(0.5)
        else:
            raise RuntimeError("temporary PostgreSQL startup timeout")
        command(["docker", "exec", self.name, "psql", "-U", "postgres", "-v", "ON_ERROR_STOP=1",
                 "-c", "CREATE ROLE drb_eval LOGIN NOSUPERUSER NOBYPASSRLS;"])
        command(["docker", "exec", self.name, "psql", "-U", "postgres", "-v", "ON_ERROR_STOP=1",
                 "-c", f"ALTER ROLE drb_eval SET statement_timeout='{self.statement_timeout_ms}ms';"])
        command(["docker", "exec", self.name, "createdb", "-U", "postgres", "-O", "drb_eval", self.database])
        port = command(["docker", "port", self.name, "5432/tcp"]).rsplit(":", 1)[1]
        self.url=f"postgres://drb_eval@127.0.0.1:{port}/{self.database}"
        command(["docker","exec",self.name,"psql","-U","postgres","-d",self.database,
                 "-c","CREATE EXTENSION pg_stat_statements;"])
        self.process = subprocess.Popen([str(TARGET / "debug/memory-eval-bridge")], stdin=subprocess.PIPE,
            stdout=subprocess.PIPE, stderr=log, text=True, bufsize=1,
            env={"PATH": os.environ["PATH"], "EVAL_DATABASE_URL": self.url})
        if self.receive() != {"ready": True}:
            raise RuntimeError("evaluation bridge did not become ready")

    def receive(self):
        with selectors.DefaultSelector() as selector:
            selector.register(self.process.stdout, selectors.EVENT_READ)
            if not selector.select(timeout=120):
                raise TimeoutError("evaluation bridge response timeout")
        line = self.process.stdout.readline()
        if not line:
            raise RuntimeError("evaluation bridge exited; see bridge.log")
        return json.loads(line)

    def call(self, action, tenant, data=None, user="evaluator", projects=(), allow_error=False):
        self.process.stdin.write(json.dumps({"action": action, "tenant": tenant, "user": user,
                                            "projects": list(projects), "data": data or {}}) + "\n")
        self.process.stdin.flush()
        response = self.receive()
        if "error" in response:
            if allow_error:
                return response
            raise RuntimeError(response["error"])
        return response["ok"]

    def __exit__(self, *exc):
        try:
            if self.process:
                self.process.stdin.close()
                if self.process.poll() is None:
                    self.process.terminate()
                    self.process.wait(timeout=10)
        finally:
            command(["docker", "rm", "-f", "-v", self.name], stderr=subprocess.DEVNULL)


def ingest(sb, tenant, doc, projects=(), allow_error=False, **kwargs):
    key = hashlib.sha256(f"{tenant}:{doc['id']}".encode()).hexdigest()
    return sb.call("ingest", tenant, {"key": key, "content": doc["content"],
        "source":doc.get("source",doc["content"]),
        **({"occurred_at":doc["occurred_at"]} if "occurred_at" in doc else {}),
        "semantic_key": "eval." + hashlib.sha256(doc["id"].encode()).hexdigest(), **kwargs}, projects=projects, allow_error=allow_error)


def corpus(sb, tenant, documents, tolerate_rejections=False):
    operations={}
    rejected=[]
    for doc in documents:
        operation=ingest(sb, tenant, doc, allow_error=tolerate_rejections)
        if "error" in operation:
            rejected.append({"id":doc["id"],"error":operation["error"]})
        else:
            operations[doc["id"]]=operation
    sb.call("drain", tenant)
    state = sb.call("state", tenant)
    by_version = {row["memory_version_id"]: row for row in state["retrieval_documents"]}
    hits = {}
    effective=[]
    originals={doc["id"]:doc["content"] for doc in documents}
    for doc in documents:
        if doc["id"] not in operations:
            continue
        row = by_version.get(operations[doc["id"]]["memory_version_id"])
        if row is None:
            raise RuntimeError("projection missing a dataset document")
        hits[doc["id"]] = {k: row[k] for k in ("memory_record_id", "memory_version_id", "head_revision", "generation", "content_hash")}
        hits[doc["id"]].update(provider_object_id="eval-tfidf:" + doc["id"], provider_score=0.0)
        effective.append({**doc,"content":row["content"]})
    return hits,effective,{"input_documents":len(documents),"indexed_documents":len(effective),
                           "redacted_documents":sum(d["content"] != originals[d["id"]] for d in effective),
                           "rejected":rejected}


def retrieve(sb, tenant, query, index, hit_map, method, budget):
    started = time.perf_counter_ns()
    depth = budget // 2 if method.startswith("hybrid") else budget
    bm25 = sb.call("search", tenant, {"query": query, "budget": depth},allow_error=True) if method != "vector_tfidf" else []
    if isinstance(bm25,dict):
        if "statement timeout" not in bm25["error"]:
            raise RuntimeError(bm25["error"])
        return {"hits":[],"memories":[],"latency_ms":(time.perf_counter_ns()-started)/1e6,
                "source_candidates":0,"error":"db_statement_timeout"}
    vector = [{**hit_map[doc], "provider_score": score} for doc, score in index.search(query, depth)] if method != "bm25" else []
    result = sb.call("rank", tenant, {"query": query, "bm25": bm25, "vector": vector,
                                     "method": method, "budget": budget})
    result["latency_ms"] = (time.perf_counter_ns() - started) / 1e6
    result["source_candidates"] = len(bm25) + len(vector)
    result["rerank_budget"] = budget if method == "hybrid_rerank" else 0
    return result


def qa_case(sb, tenant, query_input, labels, index, hits, method, budget):
    result = retrieve(sb, tenant, query_input["query"], index, hits, method, budget)
    # Context Pack 的阈值针对相关性分数；原始 cosine/BM25/RRF 不能直接当成该分数。
    started=time.perf_counter_ns()
    context_ranked=result
    if method != "hybrid_rerank" and result["hits"]:
        context_ranked=sb.call("rank",tenant,{"method":"bm25","query":query_input["query"],
                              "bm25":result["hits"],"vector":[],"budget":budget,"rerank_for_context":True})
    relevance_ms=(time.perf_counter_ns()-started)/1e6
    started=time.perf_counter_ns()
    pack = sb.call("context", tenant, {"query": query_input["query"], "hits": context_ranked["hits"],
                   **({"reference_time":timestamp(query_input["question_date"])} if "question_date" in query_input else {})})
    context_ms=(time.perf_counter_ns()-started)/1e6
    items = [item for section in pack["sections"] for item in section["items"]]
    started=time.perf_counter_ns()
    prediction = read_extractive(query_input["query"], items)
    reader_ms=(time.perf_counter_ns()-started)/1e6
    metrics = answer_metrics(prediction["text"], labels["answers"], labels["unanswerable"],
                             prediction["refused"], labels.get("stale_facts", []))
    if result.get("error"):
        metrics.update(answer_correct=False,refusal_correct=False if labels["unanswerable"] else None)
    reverse = {h["memory_version_id"]: doc for doc, h in hits.items()}
    doc_sessions = {doc["id"]: doc.get("session_id", doc["id"]) for doc in index.documents}
    retrieved = list(dict.fromkeys(doc_sessions[reverse[h["memory_version_id"]]] for h in result["hits"]))
    return {"id": query_input["id"], "method": method, "query": query_input["query"],
            "prediction": prediction, "labels": labels, "retrieved": retrieved,
            "retrieval_hit": bool(set(retrieved) & set(labels["qrels"])),
            "all_evidence_hit": bool(labels["qrels"]) and set(labels["qrels"]) <= set(retrieved),
            "context_item_count": len(items), "context_estimated_tokens": pack["estimated_tokens"],
            "raw_hits":result["hits"],"context_ranked_hits":context_ranked["hits"],"context_items":items,
            "omission_summary":pack.get("omission_summary",{}),"relevance_ms":relevance_ms,"context_ms":context_ms,"reader_ms":reader_ms,
            "retrieval_ms": result["latency_ms"],"error":result.get("error"), **metrics}


def governance(sb):
    tenant = "eval-governance"
    checks = []
    qa = []
    def record(name, ok, evidence, status="tested"):
        checks.append({"name": name, "passed": bool(ok), "status": status, "evidence": evidence})
    first = ingest(sb, tenant, {"id":"home", "content":"我住在成都。"})
    sb.call("drain", tenant)
    cached = sb.call("search", tenant, {"query":"成都", "budget":20})
    duplicate = ingest(sb, tenant, {"id":"home-copy", "content":"我住在成都。"}, semantic_key="eval."+hashlib.sha256(b"home").hexdigest())
    state = sb.call("state", tenant)
    record("exact_duplicate_and_source_trace", first["memory_record_id"] == duplicate["memory_record_id"]
           and len(state["memory_versions"]) == 1 and len(state["memory_version_sources"]) == 2, state)
    new = sb.call("correct", tenant, {"record":first["memory_record_id"], "revision":0,
                 "key":"eval-home-correction-v1", "content":"我住在深圳。"})
    # 使用旧投影缓存，在异步删除执行之前验证最终授权闸门。
    guarded = sb.call("rank", tenant, {"method":"bm25", "query":"成都", "bm25":cached, "vector":[], "budget":20})
    record("stale_head_rejected_before_projection", not guarded["hits"],
           {"with_head_guard":len(guarded["hits"]), "without_head_guard_ablation":len(cached)})
    sb.call("drain", tenant)
    current = sb.call("inspect", tenant, {"record":first["memory_record_id"]})
    fresh = sb.call("search", tenant, {"query":"深圳", "budget":20})
    record("version_switch", len(current["versions"]) == 2 and current["revision"] == 1
           and current["current_version_id"] == new["memory_version_id"] and bool(fresh), current)
    pack = sb.call("context", tenant, {"query":"我住在哪", "hits":fresh})
    texts = [i["text"] for s in pack["sections"] for i in s["items"]]
    record("context_uses_current_fact", "我住在深圳。" in texts and "我住在成都。" not in texts, texts)
    update_docs=[{"id":"home", "content":"我住在深圳。"}]
    update_hits={"home":fresh[0]}
    qa.append(qa_case(sb,tenant,{"id":"updated-home","query":"我住在哪个城市？"},
                     {"answers":["我住在深圳。"],"unanswerable":False,"qrels":{"home":1},
                      "stale_facts":["我住在成都"]},TfidfIndex(update_docs),update_hits,"hybrid_rerank",20))
    conflict = ingest(sb,tenant,{"id":"home-conflict","content":"我住在北京。"},
                      semantic_key="eval."+hashlib.sha256(b"home").hexdigest())
    contested = sb.call("inspect",tenant,{"record":first["memory_record_id"]})
    guarded = sb.call("rank",tenant,{"method":"bm25","query":"深圳","bm25":fresh,"vector":[],"budget":20})
    record("conflict_quarantine",contested["head_state"] == "contested" and not guarded["hits"],contested)
    accepted = sb.call("review",tenant,{"candidate":conflict["candidate_id"],"key":"eval-conflict-accept-v1"})
    sb.call("drain",tenant)
    state=sb.call("state",tenant)
    record("conflict_review_version", accepted["memory_version_id"] != new["memory_version_id"]
           and len(state["memory_versions"]) == 3, state)
    beijing=sb.call("search",tenant,{"query":"北京","budget":20})
    other_user=sb.call("rank",tenant,{"method":"bm25","query":"北京","bm25":beijing,"vector":[],"budget":20},user="other")
    other_tenant=sb.call("state","eval-other-tenant")
    record("user_and_tenant_scope",not other_user["hits"] and not other_tenant["memory_records"],other_tenant)
    project=ingest(sb,tenant,{"id":"alpha","content":"Alpha 项目使用 PostgreSQL。"},
                   scope="project",scope_id="Alpha",memory_type="semantic_fact",projects=["Alpha"])
    sb.call("drain",tenant)
    denied=sb.call("search",tenant,{"query":"PostgreSQL","scope":"project","scope_id":"Alpha","budget":20},allow_error=True)
    allowed=sb.call("search",tenant,{"query":"PostgreSQL","scope":"project","scope_id":"Alpha","budget":20},projects=["Alpha"])
    record("project_scope", "error" in denied and bool(allowed), {"denied":denied,"allowed":allowed})
    paraphrase=ingest(sb,tenant,{"id":"hobby","content":"我喜欢游泳。"})
    equivalent=ingest(sb,tenant,{"id":"hobby-paraphrase","content":"游泳是我的爱好。"},
                      semantic_key="eval."+hashlib.sha256(b"hobby").hexdigest())
    para=sb.call("inspect",tenant,{"record":paraphrase["memory_record_id"]})
    record("semantic_paraphrase_dedup_gap",para["head_state"] == "contested",para,"known_limitation")
    latest=sb.call("inspect",tenant,{"record":first["memory_record_id"]})
    sb.call("forget",tenant,{"record":first["memory_record_id"],"revision":latest["revision"],"key":"eval-home-forget-v1"})
    sb.call("drain",tenant)
    forgotten=sb.call("inspect",tenant,{"record":first["memory_record_id"]})
    raw=sb.call("search",tenant,{"query":"北京","budget":20})
    record("forget_retrieval", forgotten["head_state"] == "forgotten" and
           all(h["memory_record_id"] != first["memory_record_id"] for h in raw),forgotten)
    qa.append(qa_case(sb,tenant,{"id":"forgotten-home","query":"我住在哪个城市？"},
                     {"answers":[],"unanswerable":True,"qrels":{},"stale_facts":["我住在成都","我住在深圳","我住在北京"]},
                     TfidfIndex(update_docs),update_hits,"hybrid_rerank",20))
    sb.call("erase",tenant,{"record":first["memory_record_id"],"key":"eval-home-erasure-v1"})
    sb.call("drain",tenant)
    final=sb.call("state",tenant)
    versions=[v for v in final["memory_versions"] if v["memory_record_id"] == first["memory_record_id"]]
    record("logical_erasure", bool(versions) and all(v["content"] is None for v in versions),versions)
    for item in qa:
        item["benchmark"]="self_governance_dynamic"
    return checks,qa


def manifest(args):
    paths = sorted(set(list((ROOT / "evals/memory_eval").glob("*.py")) +
                       list((ROOT / "evals/harness").rglob("*.rs")) +
                       list((ROOT / "crates").rglob("*.rs")) + list((ROOT / "migrations").glob("*.sql")) +
                       list(ROOT.glob("Cargo.*")) + list((ROOT / "evals/harness").glob("Cargo.*"))))
    hashes = {str(p.relative_to(ROOT)):hashlib.sha256(p.read_bytes()).hexdigest() for p in paths}
    return {"schema_version":1,"status":"running","seed":args.seed,"git_head":command(["git","rev-parse","HEAD"]),
            "branch":command(["git","branch","--show-current"]),
            "git_status":command(["git","status","--short"]),"source_files_sha256":hashes,
            "dataset_sha256":hashlib.sha256(Path(args.dataset).read_bytes()).hexdigest(),
            "python":platform.python_version(),"platform":platform.platform(),
            "rust":command(["rustc","--version"]),"budget":args.budget,"repeats":args.repeats,
            "statement_timeout_ms":args.statement_timeout_ms,
            "qa_context_policy":"phase2: raw similarities pass existing TransparentReranker before context relevance gate; retrieval metrics stay raw",
            "models":{"vector":"corpus-only TF-IDF cosine; NOT semantic embedding",
                      "rerank":"production TransparentReranker; NOT neural reranker",
                      "formation":"eval rule baseline; NOT production LLM extractor",
                      "reader":"extractive sentence baseline; NOT production generator"},
            "api_budget":{"online_calls_limit":0,"usd_limit":0,"actual_online_calls":0,"actual_usd":0},
            "projection":"local ack through production Repository, no external Mem0/ES",
            "scope":"governed Repository ports, not mem0_primary HTTP deployment",
            "latency":"self: one warmup then repeated sequential queries; public: one pass per query/method; includes IPC, query vectorization, PG search, fusion, auth and rerank; excludes ingest/context/reader",
            "candidate_budget":"20 total source slots by default: single=20, hybrid=10+10; fused<=20, output<=10",
            "started_at_utc":time.strftime("%Y-%m-%dT%H:%M:%SZ",time.gmtime())}


def run(args):
    dataset = json.loads(Path(args.dataset).read_text())
    validate_dataset(dataset)
    public = adapt(args.longmemeval,args.limit,args.seed,args.variant) if args.longmemeval else None
    out=Path(args.output)
    out.mkdir(parents=True,exist_ok=False)
    meta=manifest(args)
    write_json(out / "manifest.json",meta)
    rows=[]
    qa=[]
    checks=[]
    formation=[]
    ingestion={}
    rng=random.Random(args.seed)
    subprocess.run(["cargo","build","--locked","--offline","--manifest-path",str(ROOT / "evals/harness/Cargo.toml")],
                   env={**os.environ,"CARGO_TARGET_DIR":str(TARGET)},check=True,timeout=600)
    with (out / "bridge.log").open("w") as log, Sandbox(args.statement_timeout_ms) as sb:
        sb.start(log)
        meta["postgres_image_id"]=sb.image
        meta["database_role"]="NOSUPERUSER NOBYPASSRLS; FORCE RLS tables"
        tenant="eval-retrieval"
        hits,docs,ingestion["self_constructed"]=corpus(sb,tenant,dataset["documents"])
        index=TfidfIndex(docs)
        reverse={h["memory_version_id"]:doc for doc,h in hits.items()}
        # 每组一次预热；各轮固定种子打乱查询与方法顺序，不筛选成绩。
        for method in METHODS:
            retrieve(sb,tenant,dataset["queries"][0]["query"],index,hits,method,args.budget)
        for repeat in range(args.repeats):
            queries=list(dataset["queries"]); rng.shuffle(queries)
            for q in queries:
                methods=list(METHODS);rng.shuffle(methods)
                for method in methods:
                    result=retrieve(sb,tenant,q["query"],index,hits,method,args.budget)
                    ranked=[reverse[h["memory_version_id"]] for h in result["hits"]]
                    rows.append({"id":q["id"],"query":q["query"],"method":method,"repeat":repeat,
                                 "ranked":ranked,"qrels":q["qrels"],"latency_ms":result["latency_ms"],
                                 "source_candidates":result["source_candidates"],"error":result.get("error"),**retrieval_metrics(ranked,q["qrels"])})
        for case in dataset["formation"]:
            prediction=extract_rules(case["text"])
            formation.append({"id":case["id"],"text":case["text"],"predictions":prediction,
                              "gold":case["facts"],**formation_counts(prediction,case["facts"])})
        # 另起 namespace，确实从自然语言提取后写入；不是复用预灌的事实库。
        formed=[]
        for doc in dataset["documents"]:
            formed.extend({"id":doc["id"]+f".{i}","content":fact,"session_id":doc["id"],"source":doc["content"]}
                          for i,fact in enumerate(extract_rules(doc["content"])))
        qa_hits,qa_docs,ingestion["self_qa_formation"]=corpus(sb,"eval-qa",formed)
        qa_index=TfidfIndex(qa_docs)
        for q in dataset["qa"]:
            qa.append(qa_case(sb,"eval-qa",q,q,qa_index,qa_hits,"hybrid_rerank",args.budget))
        write_json(out / "retrieval.json",rows)
        write_json(out / "formation.json",formation)
        write_json(out / "qa.json",qa)
        checks,dynamic_qa=governance(sb)
        qa.extend(dynamic_qa)
        write_json(out / "governance.json",checks)
        if public:
            write_json(out / "longmemeval-selection.json",public["provenance"])
            for i,case in enumerate(public["cases"]):
                print(f"LongMemEval {i+1}/{len(public['cases'])}: {case['input']['id']}",flush=True)
                inp,labels=case["input"],case["labels"]
                t=f"eval-longmemeval-{i}"
                h,docs,ingestion[inp["id"]]=corpus(sb,t,inp["documents"],tolerate_rejections=True)
                idx=TfidfIndex(docs)
                write_json(out / "ingestion.json",ingestion)
                # 公开实验是保留全部角色的原文分块对照；没有宣称执行 LLM 事实提取。
                for method in METHODS:
                    item=qa_case(sb,t,inp,labels,idx,h,method,args.budget)
                    item["benchmark"]="LongMemEval-"+args.variant+" public subset"
                    qa.append(item)
                    rows.append({"id":inp["id"],"method":method,"repeat":0,"benchmark":item["benchmark"],
                                 "question_type":labels["question_type"],"unanswerable":labels["unanswerable"],
                                 "ranked":item["retrieved"],"qrels":labels["qrels"],
                                 "latency_ms":item["retrieval_ms"],
                                 "error":item.get("error"),
                                 **retrieval_metrics(item["retrieved"],labels["qrels"])})
                write_json(out / "qa.json",qa)
                write_json(out / "retrieval.json",rows)
    meta["status"]="completed"
    meta["completed_at_utc"]=time.strftime("%Y-%m-%dT%H:%M:%SZ",time.gmtime())
    write_json(out / "manifest.json",meta)
    write_json(out / "ingestion.json",ingestion)
    write_json(out / "retrieval.json",rows)
    write_json(out / "formation.json",formation)
    write_json(out / "qa.json",qa)
    failures=[{"category":"retrieval",**r} for r in rows if r.get("error") or r["recall_at_5"] is not None and r["recall_at_5"] < 1]
    failures += [{"category":"formation",**r} for r in formation if r["fp"] or r["fn"]]
    failures += [{"category":"qa",**r} for r in qa if not r["answer_correct"] or r["stale_fact_error"]]
    write_json(out / "failures.json",failures)
    report(out,meta,rows,formation,qa,checks)
    if any(not c["passed"] for c in checks):
        raise RuntimeError("governance assertion failed; inspect governance.json")
    print(out.resolve())


def report(out,meta,rows,formation,qa,checks):
    retrieval={}
    for dataset in sorted({r.get("benchmark","self_constructed") for r in rows}):
        retrieval[dataset]={}
        for method in METHODS:
            group=[r for r in rows if r.get("benchmark","self_constructed")==dataset and r["method"]==method]
            retrieval[dataset][method]={"questions":len({r["id"] for r in group}),
                "scored_questions":len({r["id"] for r in group if r["recall_at_1"] is not None}),"latency_samples":len(group),
                "error_count":sum(bool(r.get("error")) for r in group),
                **{k:mean([r[k] for r in group]) for k in ("recall_at_1","recall_at_5","mrr_at_10","ndcg_at_10")},
                "p50_ms":percentile([r["latency_ms"] for r in group],.5),
                "p95_ms":percentile([r["latency_ms"] for r in group],.95),
                "p95_success_ms":percentile([r["latency_ms"] for r in group if not r.get("error")],.95),"online_usd":0}
    tp,fp,fn=(sum(r[k] for r in formation) for k in ("tp","fp","fn"))
    qa_summary={}
    for dataset in sorted({r.get("benchmark","self_constructed") for r in qa}):
        for method in sorted({r["method"] for r in qa if r.get("benchmark","self_constructed")==dataset}):
            group=[r for r in qa if r.get("benchmark","self_constructed")==dataset and r["method"]==method]
            answerable=[r for r in group if not r["labels"]["unanswerable"]]
            qa_summary[dataset+"/"+method]={"n":len(group),"answer_accuracy":mean([r["answer_correct"] for r in group]),
                "answerable_accuracy":mean([r["answer_correct"] for r in answerable]),
                "refusal_accuracy":mean([r["refusal_correct"] for r in group]),
                "refusal_n":sum(r["labels"]["unanswerable"] for r in group),
                "error_count":sum(bool(r.get("error")) for r in group),
                "retrieval_hit_rate_answerable":mean([r["retrieval_hit"] for r in answerable]),
                "stale_fact_error_rate":mean([r["stale_fact_error"] for r in group if r["labels"].get("stale_facts")]),
                "stale_fact_cases":sum(bool(r["labels"].get("stale_facts")) for r in group)}
    summary={"retrieval":retrieval,"formation":{"n":len(formation),"tp":tp,"fp":fp,"fn":fn,**prf(tp,fp,fn),
                "empty_cases":sum(not r["gold"] for r in formation),
                "empty_correct":sum(not r["gold"] and not r["predictions"] for r in formation)},
             "qa":qa_summary,"governance":{"checks":len(checks),"passed":sum(c["passed"] for c in checks),
                 "limitations":[c["name"] for c in checks if c["status"]=="known_limitation"]},
             "api_budget":meta["api_budget"],"official_benchmark_score":False}
    write_json(out / "summary.json",summary)
    lines=["# Memory 评测报告", "",f"代码基点 `{meta['git_head']}`；分支 `{meta['branch']}`。实际工作区哈希见 manifest.json。",
           "", "所有在线调用与费用均为 0。Vector 是 TF-IDF cosine；Rerank 是现有透明词法规则；不是语义模型对比。",
           "公开子集使用原文分块，Formation/自建 QA 使用本地规则。均不是官方完整 Benchmark 或排行榜成绩。",""]
    lines += [f"数据库每条语句硬上限 {meta['statement_timeout_ms']} ms。超时保留在质量分母中，回答与拒答均不能因超时算对；延迟含返回失败的耗时，成功查询延迟另见 JSON。", ""]
    for name,methods in retrieval.items():
        lines += ["## "+name,"", "| 方法 | N / 正例 | 错误 | R@1 | R@5 | MRR@10 | nDCG@10 | P50 ms | P95 ms |", "|---|---:|---:|---:|---:|---:|---:|---:|---:|"]
        for method,m in methods.items():
            lines.append(f"| {method} | {m['questions']} / {m['scored_questions']} | {m['error_count']} | "+" | ".join(f"{m[k]:.4f}" if m[k] is not None else "N/A" for k in
                ("recall_at_1","recall_at_5","mrr_at_10","ndcg_at_10","p50_ms","p95_ms"))+" |")
        lines.append("")
    lines += ["## Formation（规则基线，严格事实别名匹配）", "",json.dumps(summary["formation"],ensure_ascii=False),"",
              "## QA（抽取式句子回答，严格 normalized exact match）", "", "```json",json.dumps(qa_summary,ensure_ascii=False,indent=2),"```", "",
              "## 治理", "",f"自动断言 {summary['governance']['passed']}/{len(checks)}；语义改写去重为已知缺口，探针通过不表示能力已实现。", "",
              "同一旧候选集的 Head 闸门消融见 governance.json。遗忘与逻辑擦除不代表备份/WAL/外部索引物理擦除。", "",
              "逐题结果：retrieval.json / formation.json / qa.json；失败案例：failures.json；版本与参数：manifest.json。", "",
              "小规模合成集没有独立盲测；公开子集有抽样误差。延迟为本机热启动串行测量，不是生产并发 SLO。"]
    (out / "report.md").write_text("\n".join(lines)+"\n")
