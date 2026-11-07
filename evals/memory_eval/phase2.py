"""第二阶段诊断：零模型调用、固定数据、隔离数据库、保留失败。"""
import argparse
from collections import Counter, defaultdict
import hashlib
import json
import math
import os
from pathlib import Path
import random
import subprocess
import time

from .baselines import TfidfIndex, read_extractive, tokens
from .longmemeval import adapt, timestamp
from .metrics import answer_metrics, mean, normalize, percentile, retrieval_metrics
from .runner import ROOT, TARGET, METHODS, Sandbox, command, corpus, retrieve, write_json


def stats(sb):
    sql="SELECT json_build_object('calls',COALESCE(sum(calls),0),'sql_ms',COALESCE(sum(total_exec_time),0)) FROM pg_stat_statements WHERE userid=(SELECT oid FROM pg_roles WHERE rolname='drb_eval')"
    return json.loads(command(["docker","exec",sb.name,"psql","-U","postgres","-d",sb.database,"-Atc",sql]))


def reference_bm25(term_rows, query_terms, hit_map):
    # 用真实 Jieba 词项做独立 Python 数学参考，不复用 SQL 的聚合/排序实现。
    by_version=defaultdict(dict)
    for row in term_rows:
        by_version[row["memory_version_id"]][row["term"]]=row
    n=len(hit_map)
    avgdl=mean([next(iter(ts.values()))["document_length"] for ts in by_version.values()]) or 1
    df=Counter(t for ts in by_version.values() for t in ts)
    records={h["memory_version_id"]:h["memory_record_id"] for h in hit_map.values()}
    scores=[]
    for version,ts in by_version.items():
        score=0
        for term in set(query_terms):
            if term in ts:
                row=ts[term];tf=row["term_frequency"];dl=row["document_length"]
                score+=math.log(1+(n-df[term]+.5)/(df[term]+.5))*tf*2.2/(tf+1.2*(.25+.75*dl/avgdl))
        if score>0:
            scores.append({"memory_version_id":version,"memory_record_id":records[version],"bm25_score":score})
    return sorted(scores,key=lambda r:(-r["bm25_score"],r["memory_record_id"],r["memory_version_id"]))[:20]


def reference_consistent(actual,reference):
    # 仅校验数学参考的浮点误差，不改真实排序或任何检索/答案评分。
    if {r["memory_version_id"] for r in actual}!={r["memory_version_id"] for r in reference}:
        return False
    expected={r["memory_version_id"]:r["bm25_score"] for r in reference}
    return all(math.isclose(r["bm25_score"],expected[r["memory_version_id"]],rel_tol=1e-12,abs_tol=1e-12)
               and math.isclose(r["bm25_score"],e["bm25_score"],rel_tol=1e-12,abs_tol=1e-12)
               for r,e in zip(actual,reference))


def residual(sb,args,out):
    data=adapt(args.longmemeval);rows=[];plans=[]
    write_json(out/"longmemeval-selection.json",data["provenance"])
    for i in range(12):
        corpus(sb,f"residual-small-{i}",[{"id":"small","content":f"Small tenant {i} note."}])
    for case in [c for c in data["cases"] if c["input"]["id"] in {"c6853660","35a27287"}]:
        inp=case["input"];tenant="residual-"+inp["id"]
        h,docs,ingestion=corpus(sb,tenant,inp["documents"],True)
        term_rows=sb.call("terms",tenant)
        for repeat in range(8):
            for variant in ["materialized","materialized_uncached"]:
                print("residual",inp["id"],repeat,variant,flush=True)
                start=time.perf_counter_ns()
                result=sb.call("sql_probe",tenant,{"query":inp["query"],"variant":variant,"plan_mode":"auto"})
                elapsed=(time.perf_counter_ns()-start)/1e6
                ref=reference_bm25(term_rows,result["terms"],h)
                if not result.get("error"):
                    assert reference_consistent(result["rows"],ref)
                rows.append({"id":inp["id"],"repeat":repeat,"variant":variant,"request_ms":elapsed,**result})
                write_json(out/"questions.json",rows)
        for mode in ["force_generic_plan","force_custom_plan"]:
            result=sb.call("sql_probe",tenant,{"query":inp["query"],"variant":"materialized","plan_mode":mode,"explain":True},allow_error=True)
            plans.append({"id":inp["id"],"plan_mode":mode,**result})
            write_json(out/"plans.json",plans)
    write_json(out/"summary.json",[{"variant":v,"n":len(g),"timeouts":sum(bool(r.get("error")) for r in g),
                                    "p50_ms":percentile([r["sql_ms"] for r in g],.5),"p95_ms":percentile([r["sql_ms"] for r in g],.95)}
                                   for v in ["materialized","materialized_uncached"] if (g:=[r for r in rows if r["variant"]==v])])


def cardinality(sb,args,out):
    # 这是规划器压力夹具，不作为额外 Benchmark 题或官方分数；所有副本仅在新临时库。
    data=adapt(args.longmemeval)
    write_json(out/"longmemeval-selection.json",data["provenance"])
    case=next(c for c in data["cases"] if c["input"]["id"]=="35a27287")
    inp=case["input"];tenant="cardinality-target"
    h,docs,ingestion=corpus(sb,tenant,inp["documents"],True)
    def sql(statement):
        return command(["docker","exec",sb.name,"psql","-U","postgres","-d",sb.database,"-v","ON_ERROR_STOP=1","-Atc",statement])
    tables=["memory_core.memory_records","memory_core.memory_decisions","memory_core.memory_versions",
            "memory_search.retrieval_documents","memory_search.retrieval_terms"]
    for i in range(11):
        clone=f"cardinality-copy-{i}"
        if i==10:
            sql("ANALYZE memory_search.retrieval_documents; ANALYZE memory_search.retrieval_terms")
            clone="cardinality-unseen"
        clone_namespace=sb.call("namespace",clone)
        for table in tables:
            cols=sql(f"SELECT string_agg(attname,',' ORDER BY attnum) FROM pg_attribute WHERE attrelid='{table}'::regclass AND attnum>0 AND NOT attisdropped AND attgenerated=''").split(',')
            changes=[f"'tenant_id','{clone}'"]
            for col in cols:
                if col in {"id","memory_record_id","memory_version_id","decision_id"}:
                    changes.append(f"'{col}',md5('{clone}'||t.{col}::text)::uuid")
            if table.endswith("memory_decisions"):changes.append("'candidate_id',NULL")
            if table.endswith("retrieval_documents"):changes.append(f"'namespace_key','{clone_namespace}'")
            fields=','.join(f"(j.v).{c}" for c in cols)
            sql(f"INSERT INTO {table} ({','.join(cols)}) SELECT {fields} FROM (SELECT jsonb_populate_record(NULL::{table},to_jsonb(t)||jsonb_build_object({','.join(changes)})) AS v FROM {table} t WHERE t.tenant_id='{tenant}') j")
    tenant="cardinality-unseen"
    plans=[];rows=[]
    for mode in ["force_custom_plan","force_generic_plan"]:
        for variant in ["materialized","version_lengths"]:
            for analyze in [False,True]:
                result=sb.call("sql_probe",tenant,{"query":inp["query"],"variant":variant,"plan_mode":mode,"explain":True,"analyze":analyze,"plan_timeout_ms":10000},allow_error=True)
                plans.append({"plan_mode":mode,"variant":variant,"analyze":analyze,"plan_timeout_ms":10000,**result});write_json(out/"plans.json",plans)
    for repeat in range(3):
        for variant in ["before","materialized","materialized_uncached","lateral","version_lengths"]:
            result=sb.call("sql_probe",tenant,{"query":inp["query"],"variant":variant,"plan_mode":"auto"})
            rows.append({"repeat":repeat,"variant":variant,**result});write_json(out/"questions.json",rows)
    write_json(out/"summary.json",{"kind":"synthetic multi-tenant planner fixture, not benchmark score","source_question":inp["id"],
               "target_documents":len(h),"copies":11,"total_projection_documents":int(sql("SELECT count(*) FROM memory_search.retrieval_documents")),
               "input_ingestion":ingestion,"memory_sample":command(["docker","stats","--no-stream",sb.name,"--format","{{.MemUsage}}"]),
               "variants":{v:{"timeouts":sum(bool(r.get("error")) for r in rows if r["variant"]==v),
                               "sql_p50_ms":percentile([r["sql_ms"] for r in rows if r["variant"]==v],.5)}
                           for v in ["before","materialized","materialized_uncached","lateral","version_lengths"]}})


def performance(sb,args,out):
    data=adapt(args.longmemeval)
    write_json(out/"longmemeval-selection.json",data["provenance"])
    docs=next(c["input"]["documents"] for c in data["cases"] if c["input"]["id"]=="c6853660")
    query=next(c["input"]["query"] for c in data["cases"] if c["input"]["id"]=="c6853660")
    rows=[];plans=[];resources=[];rng=random.Random(args.seed)
    # 小租户与长历史租户并存，复现 tenant/namespace 相关列的基数估算问题。
    for i in range(12):
        corpus(sb,f"phase2-small-{i}",[{"id":"small","content":f"Small tenant {i} has a separate note."}])
    for length in [100,400,len(docs)]:
        tenant=f"phase2-perf-{length}"
        h,effective,ingestion=corpus(sb,tenant,docs[:length],True)
        terms=sb.call("terms",tenant)
        command(["docker","exec",sb.name,"psql","-U","postgres","-d",sb.database,"-c",
                 "ANALYZE memory_search.retrieval_documents; ANALYZE memory_search.retrieval_terms;"])
        resources.append({"length":length,"ingestion":ingestion,"docker_sample":command(["docker","stats","--no-stream",sb.name,"--format","{{.MemUsage}}"]),
                          "work_mem":command(["docker","exec",sb.name,"psql","-U","postgres","-Atc","SHOW work_mem"]),
                          "term_rows":len(terms)})
        for mode in ["auto","force_generic_plan"]:
            for variant in ["before","materialized","version_lengths"]:
                sb.call("sql_probe",tenant,{"variant":variant,"query":"coffee","plan_mode":mode})
            for repeat in range(args.repeats):
                for q in ["coffee",query]:
                    variants=["before","materialized","version_lengths"];rng.shuffle(variants)
                    for variant in variants:
                        print(f"perf {length} {mode} repeat={repeat} {variant}",flush=True)
                        before=stats(sb);start=time.perf_counter_ns()
                        result=sb.call("sql_probe",tenant,{"variant":variant,"query":q,"plan_mode":mode})
                        elapsed=(time.perf_counter_ns()-start)/1e6;after=stats(sb)
                        reference=reference_bm25(terms,result["terms"],h)
                        equal=None
                        if not result.get("error"):
                            equal=[r["memory_version_id"] for r in result["rows"]]==[r["memory_version_id"] for r in reference]
                            assert equal,(length,variant,q,"rank differs from independent reference")
                            assert all(math.isclose(a["bm25_score"],b["bm25_score"],rel_tol=1e-12,abs_tol=1e-12) for a,b in zip(result["rows"],reference))
                        rows.append({"length":length,"variant":variant,"query":q,"plan_mode":mode,"repeat":repeat,
                                     "request_ms":elapsed,"sql_calls":after["calls"]-before["calls"],
                                     "all_sql_ms":after["sql_ms"]-before["sql_ms"],"reference_equal":equal,**result})
                        write_json(out/"performance.json",rows)
            # 高频单词的完成计划用于验证聚合 loops，完整问题的超时计划保留为错误。
            for variant in ["before","materialized","version_lengths"]:
                for q in ["coffee",query]:
                    result=sb.call("sql_probe",tenant,{"variant":variant,"query":q,"plan_mode":mode,"explain":True},allow_error=True)
                    plans.append({"length":length,"variant":variant,"query":q,"plan_mode":mode,**result})
                    write_json(out/"plans.json",plans)
        write_json(out/"resources.json",resources)
    summary=[]
    for length in [100,400,len(docs)]:
        for mode in ["auto","force_generic_plan"]:
            for variant in ["before","materialized","version_lengths"]:
                group=[r for r in rows if (r["length"],r["plan_mode"],r["variant"])==(length,mode,variant)]
                summary.append({"length":length,"plan_mode":mode,"variant":variant,"n":len(group),
                                "timeouts":sum(bool(r.get("error")) for r in group),
                                "p50_ms":percentile([r["request_ms"] for r in group],.5),
                                "p95_ms":percentile([r["request_ms"] for r in group],.95),
                                "sql_p50_ms":percentile([r["sql_ms"] for r in group],.5),
                                "sql_calls":sorted(set(r["sql_calls"] for r in group)),
                                "completed_rank_checks":sum(r["reference_equal"] is True for r in group)})
    write_json(out/"summary.json",summary)
    edge_docs=[{"id":"empty-terms","content":"的 和 我"},{"id":"a","content":"lighthouse lighthouse"},{"id":"b","content":"lighthouse archive"}]
    h,_,_=corpus(sb,"phase2-empty-terms",edge_docs)
    terms=sb.call("terms","phase2-empty-terms")
    edges=[]
    for variant in ["before","version_lengths"]:
        result=sb.call("sql_probe","phase2-empty-terms",{"variant":variant,"query":"lighthouse"})
        ref=reference_bm25(terms,result["terms"],h)
        assert reference_consistent(result["rows"],ref)
        edges.append({"variant":variant,**result})
    write_json(out/"empty-terms-edge.json",edges)


def diagnosis(sb,args,out):
    data=adapt(args.longmemeval)
    write_json(out/"longmemeval-selection.json",data["provenance"])
    raw={r["question_id"]:r for r in json.loads(Path(args.longmemeval).read_text())}
    reviews=json.loads((ROOT/"evals/datasets/longmemeval_review_v1.json").read_text())["cases"]
    previous=json.loads((ROOT/"evals/reports/2026-11-06-local-v3/qa.json").read_text())
    results=[];records=[]
    for i,case in enumerate(data["cases"]):
        inp,labels=case["input"],case["labels"];qid=inp["id"];tenant=f"phase2-qa-{i}"
        print(f"diagnose {i+1}/12 {qid}",flush=True)
        h,docs,ingestion=corpus(sb,tenant,inp["documents"],True)
        idx=TfidfIndex(docs);reverse={v["memory_version_id"]:k for k,v in h.items()};by_id={d["id"]:d for d in docs}
        official=raw[qid]
        oracle=[];evidence_ids=[]
        for sid,session in zip(official["haystack_session_ids"],official["haystack_sessions"]):
            for turn_no,turn in enumerate(session):
                if turn.get("has_answer"):
                    oracle.append({"text":turn["content"],"memory_version_id":f"oracle:{sid}:{turn_no}"})
                    evidence_ids.extend(d["id"] for d in docs if d["id"].startswith(f"{sid}:{turn_no}:"))
        oracle_prediction=read_extractive(inp["query"],oracle)
        oracle_score=answer_metrics(oracle_prediction["text"],labels["answers"],labels["unanswerable"],oracle_prediction["refused"])
        pinned=[h[d]["memory_version_id"] for d in evidence_ids]
        oracle_pack=sb.call("context",tenant,{"query":inp["query"],"hits":[],"pinned_versions":pinned,"reference_time":timestamp(inp["question_date"])})
        oracle_context=[it for sec in oracle_pack["sections"] for it in sec["items"]]
        oracle_context_prediction=read_extractive(inp["query"],oracle_context)
        oracle_record={"id":qid,"labels":labels,"evidence_turns":oracle,"oracle_prediction":oracle_prediction,"oracle_metrics":oracle_score,
                       "oracle_context":oracle_pack,"oracle_context_prediction":oracle_context_prediction,
                       "identity_scorer_control":answer_metrics(labels["answers"][0],labels["answers"],False,False)["answer_correct"] if not labels["unanswerable"] else None,
                       "ingestion":ingestion,"review":reviews[qid]}
        for method in METHODS:
            before_sql=stats(sb)
            result=retrieve(sb,tenant,inp["query"],idx,h,method,20)
            after_sql=stats(sb)
            retrieved_ids=[reverse[x["memory_version_id"]] for x in result["hits"]]
            legacy_pack=sb.call("context",tenant,{"query":inp["query"],"hits":result["hits"],"reference_time":timestamp(inp["question_date"])})
            # 与生产链相同：原始相似度先经过现有相关性重排，再交给 Context Pack；不降低任何阈值。
            ranked=result if method=="hybrid_rerank" else sb.call("rank",tenant,{"method":"bm25","query":inp["query"],"bm25":result["hits"],"vector":[],"budget":20,"rerank_for_context":True})
            pack=sb.call("context",tenant,{"query":inp["query"],"hits":ranked["hits"],"reference_time":timestamp(inp["question_date"])})
            items=[it for sec in pack["sections"] for it in sec["items"]]
            prediction=read_extractive(inp["query"],items)
            score=answer_metrics(prediction["text"],labels["answers"],labels["unanswerable"],prediction["refused"])
            if result.get("error"):
                score.update(answer_correct=False,refusal_correct=False if labels["unanswerable"] else None)
            retrieved_text=" ".join(by_id[d]["content"] for d in retrieved_ids)
            context_text=" ".join(it["text"] for it in items)
            facts=reviews[qid]["facts"]
            fact_retrieved=[normalize(f) in normalize(retrieved_text) for f in facts]
            fact_context=[normalize(f) in normalize(context_text) for f in facts]
            categories=[]
            if not labels["unanswerable"]:
                if not all(fact_retrieved): categories.append("Retrieval Failure")
                if any(a and not b for a,b in zip(fact_retrieved,fact_context)):categories.append("Context Construction Failure")
                if all(fact_context) and not score["answer_correct"]:categories.append("Answer Generation Failure")
                if len(legacy_pack["sections"])!=len(pack["sections"]) or "preference" in labels["question_type"] or qid=="1b9b7252":
                    categories.append("Evaluator / Adapter Failure")
            row={"id":qid,"method":method,"query":inp["query"],"labels":labels,"retrieved_ids":retrieved_ids,
                 "retrieved_hits":result["hits"],"retrieved_documents":[by_id[d] for d in retrieved_ids],
                 "official_evidence_chunk_ids":evidence_ids,"has_answer_turn_hit":bool(set(evidence_ids)&set(retrieved_ids)),
                 "session_hit":bool({by_id[d]["session_id"] for d in retrieved_ids}&set(labels["qrels"])),
                 "facts_retrieved":fact_retrieved,"facts_in_context":fact_context,"legacy_pack":legacy_pack,
                 "reranked_hits":ranked["hits"],"pack":pack,"prediction":prediction,"categories":categories,
                 "retrieval_ms":result["latency_ms"],"error":result.get("error"),**score}
            row["retrieval_sql_calls"]=after_sql["calls"]-before_sql["calls"]
            row["retrieval_sql_exec_ms"]=after_sql["sql_ms"]-before_sql["sql_ms"]
            results.append(row)
        oracle_record["previous_tfidf"]=next(r for r in previous if r["id"]==qid and r["method"]=="vector_tfidf")
        records.append(oracle_record)
        write_json(out/"questions.json",results);write_json(out/"oracle.json",records)
    summary={}
    for method in METHODS:
        group=[r for r in results if r["method"]==method];positive=[r for r in group if not r["labels"]["unanswerable"]]
        summary[method]={"n":len(group),"answerable_n":len(positive),"strict_answerable_correct":sum(r["answer_correct"] for r in positive),
                         "session_hits":sum(r["session_hit"] for r in positive),"has_answer_turn_hits":sum(r["has_answer_turn_hit"] for r in positive),
                         "all_fact_spans_retrieved":sum(all(r["facts_retrieved"]) for r in positive),
                         "all_fact_spans_in_context":sum(all(r["facts_in_context"]) for r in positive),
                         "empty_legacy_contexts":sum(not r["legacy_pack"]["sections"] for r in positive),
                         "empty_corrected_contexts":sum(not r["pack"]["sections"] for r in positive),
                         "timeouts":sum(bool(r["error"]) for r in group)}
    summary["oracle"]={"answerable_n":10,"direct_reader_strict_correct":sum(r["oracle_metrics"]["answer_correct"] for r in records if not r["labels"]["unanswerable"]),
                       "scorer_identity_correct":sum(r["identity_scorer_control"] is True for r in records)}
    write_json(out/"summary.json",summary)


def holdout(sb,args,out):
    data=json.loads((ROOT/"evals/datasets/holdout_v2.json").read_text())
    h,docs,ingestion=corpus(sb,"phase2-holdout",data["documents"])
    idx=TfidfIndex(docs);reverse={v["memory_version_id"]:k for k,v in h.items()};rows=[];sql_checks=[]
    term_rows=sb.call("terms","phase2-holdout")
    for q in data["queries"]:
        before=sb.call("sql_probe","phase2-holdout",{"variant":"before","query":q["query"]})
        after=sb.call("sql_probe","phase2-holdout",{"variant":"version_lengths","query":q["query"]})
        ref=reference_bm25(term_rows,after["terms"],h)
        for name,result in [("before",before),("version_lengths",after)]:
            if not result.get("error"):
                if not reference_consistent(result["rows"],ref):
                    write_json(out/"reference-mismatch.json",{"id":q["id"],"variant":name,"result":result["rows"],"reference":ref})
                    raise AssertionError("independent reference rank mismatch; inspect reference-mismatch.json")
        sql_checks.append({"id":q["id"],"before_error":before.get("error"),"after_error":after.get("error"),
                           "reference_consistent":reference_consistent(after["rows"],ref),
                           "equal_rank":None if before.get("error") else [r["memory_version_id"] for r in before["rows"]]==[r["memory_version_id"] for r in after["rows"]]})
        for method in METHODS:
            result=retrieve(sb,"phase2-holdout",q["query"],idx,h,method,20)
            ranked=[reverse[r["memory_version_id"]] for r in result["hits"]]
            rows.append({"id":q["id"],"tag":q["tag"],"query":q["query"],"qrels":q["qrels"],"method":method,"ranked":ranked,
                         "error":result.get("error"),"latency_ms":result["latency_ms"],"false_positive":not q["qrels"] and bool(ranked),
                         **retrieval_metrics(ranked,q["qrels"])})
    summary={};rng=random.Random(args.seed)
    for method in METHODS:
        group=[r for r in rows if r["method"]==method]
        summary[method]={**{k:mean([r[k] for r in group]) for k in ["recall_at_1","recall_at_5","mrr_at_10","ndcg_at_10"]},
                         "positive_n":sum(bool(r["qrels"]) for r in group),"negative_n":sum(not r["qrels"] for r in group),
                         "negative_any_hit_rate":mean([r["false_positive"] for r in group if not r["qrels"]])}
        values=[r["recall_at_5"] for r in group if r["qrels"]]
        boot=[mean(rng.choices(values,k=len(values))) for _ in range(1000)]
        summary[method]["recall_at_5_query_bootstrap_95_ci"]=[percentile(boot,.025),percentile(boot,.975)]
    write_json(out/"questions.json",rows);write_json(out/"summary.json",summary);write_json(out/"ingestion.json",ingestion)
    write_json(out/"sql-equivalence.json",sql_checks)
    old=json.loads((ROOT/"evals/datasets/memory_v1.json").read_text())
    audit={"exact_document_overlap":len({normalize(d["content"]) for d in old["documents"]}&{normalize(d["content"]) for d in data["documents"]}),
           "exact_query_overlap":len({normalize(q["query"]) for q in old["queries"]}&{normalize(q["query"]) for q in data["queries"]}),
           "holdout_tags":dict(Counter(q["tag"] for q in data["queries"]))}
    for name,ds in [("old",old),("holdout",data)]:
        by_id={d["id"]:set(tokens(d["content"])) for d in ds["documents"]}
        overlaps=[]
        for q in ds["queries"]:
            qrel=q.get("qrels",{});qt=set(tokens(q["query"]))
            overlaps.extend(len(qt&by_id[d])/max(1,len(qt|by_id[d])) for d in qrel if qrel[d]>0)
        audit[name]={"documents":len(ds["documents"]),"queries":len(ds["queries"]),
                     "mean_relevant_token_jaccard":mean(overlaps),"max_relevant_token_jaccard":max(overlaps)}
    write_json(out/"dataset-audit.json",audit)


def pg_tests(sb,args,out):
    # 不取消 #[ignore]，只在明确指定的一次性库上主动运行全部旧 PG 测试。
    env={**os.environ,"CARGO_TARGET_DIR":str(TARGET),"TEST_DATABASE_URL":sb.url}
    with (out/"cargo-tests.log").open("w") as log:
        result=subprocess.run(["cargo","test","--locked","--offline","-p","memory-postgres","--lib","--","--ignored","--test-threads=1"],
                              env=env,stdout=log,stderr=subprocess.STDOUT,timeout=600)
    write_json(out/"summary.json",{"exit_code":result.returncode,"test_log":"cargo-tests.log","role":"NOSUPERUSER NOBYPASSRLS"})
    if result.returncode: raise RuntimeError("PG integration tests failed; inspect cargo-tests.log")


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode",choices=["performance","residual","cardinality","diagnosis","holdout","pg-tests"])
    parser.add_argument("--output",required=True)
    parser.add_argument("--longmemeval",default="/tmp/drb-eval-public/longmemeval_s_cleaned.json")
    parser.add_argument("--seed",type=int,default=20261107)
    parser.add_argument("--repeats",type=int,default=3)
    args=parser.parse_args()
    if Path(args.output).exists() or not 1<=args.repeats<=5:parser.error("new output directory required; repeats=1..5")
    out=Path(args.output);out.mkdir(parents=True)
    manifest={"phase":2,"mode":args.mode,"seed":args.seed,"repeats":args.repeats,"status":"running",
              "git_head":command(["git","rev-parse","HEAD"]),"git_status":command(["git","status","--short"]),
              "statement_timeout_ms":3000,"model_api_calls":0,"model_api_usd":0,
              "source_sha256":{str(p):hashlib.sha256(p.read_bytes()).hexdigest() for p in
                  [*Path("evals/memory_eval").glob("*.py"),Path("evals/harness/src/main.rs"),Path("crates/memory-postgres/src/repository.rs"),
                   *Path("evals/evidence/phase2").glob("*.sql"),*Path("evals/datasets").glob("*.json")]}}
    write_json(out/"manifest.json",manifest)
    try:
        subprocess.run(["cargo","build","--locked","--offline","--manifest-path",str(ROOT/"evals/harness/Cargo.toml")],
                       env={**os.environ,"CARGO_TARGET_DIR":str(TARGET)},check=True,timeout=600)
        with (out/"bridge.log").open("w") as log,Sandbox() as sb:
            sb.start(log);manifest["postgres_image_id"]=sb.image
            {"performance":performance,"residual":residual,"cardinality":cardinality,"diagnosis":diagnosis,"holdout":holdout,"pg-tests":pg_tests}[args.mode](sb,args,out)
    except Exception as error:
        manifest.update(status="failed",error=str(error));write_json(out/"manifest.json",manifest)
        raise
    manifest["status"]="completed";write_json(out/"manifest.json",manifest)
    print(out.resolve())


if __name__=="__main__":
    main()
