import json
import math
from pathlib import Path
import tempfile
import unittest
import subprocess
import sys
from unittest.mock import patch

from evals.memory_eval.baselines import TfidfIndex
from evals.memory_eval.longmemeval import adapt
from evals.memory_eval.metrics import answer_metrics, formation_counts, percentile, retrieval_metrics
from evals.memory_eval.runner import qa_case, validate_dataset
from evals.memory_eval.phase2 import reference_consistent


class MetricTests(unittest.TestCase):
    def test_graded_ndcg_uses_all_relevant_and_normalizes(self):
        result = retrieval_metrics(["a", "noise", "b"], {"a": 2, "b": 1})
        expected = (3 + 1 / math.log2(4)) / (3 + 1 / math.log2(3))
        self.assertAlmostEqual(result["ndcg_at_10"], expected)
        self.assertEqual(result["recall_at_1"], .5)
        self.assertEqual(result["recall_at_5"], 1)
        self.assertEqual(retrieval_metrics(["a", "b"], {"a": 2, "b": 1})["ndcg_at_10"], 1)

    def test_cutoffs_duplicates_and_empty(self):
        self.assertEqual(retrieval_metrics([str(i) for i in range(11)], {"10": 1})["mrr_at_10"], 0)
        self.assertEqual(retrieval_metrics(["x", "x", "a", "a"], {"a": 1})["mrr_at_10"], .5)
        self.assertEqual(retrieval_metrics([], {"a": 1})["ndcg_at_10"], 0)
        self.assertIsNone(retrieval_metrics(["x"], {})["ndcg_at_10"])

    def test_fact_matching_is_one_to_one_and_maximum(self):
        self.assertEqual(formation_counts(["A", "A", "wrong"], [["A"]]), {"tp":1,"fp":2,"fn":0})
        self.assertEqual(formation_counts(["A", "B"], [["A", "B"], ["A"]]), {"tp":2,"fp":0,"fn":0})
        self.assertEqual(formation_counts(["不应写入"], []), {"tp":0,"fp":1,"fn":0})

    def test_percentiles_interpolate(self):
        self.assertEqual(percentile([0,10], .95), 9.5)
        self.assertIsNone(percentile([], .5))

    def test_math_reference_tolerates_only_numerically_tied_order(self):
        reference=[{"memory_version_id":"a","bm25_score":17.03080333775922},
                   {"memory_version_id":"b","bm25_score":17.03080333775922}]
        self.assertTrue(reference_consistent([{**reference[1],"bm25_score":17.030803337759224},reference[0]],reference))
        self.assertFalse(reference_consistent([{**reference[1],"bm25_score":18},reference[0]],reference))

    def test_retrieval_hit_does_not_make_answer_correct(self):
        r=answer_metrics("我住在成都",["我住在深圳"],False,False,["我住在成都"])
        self.assertFalse(r["answer_correct"])
        self.assertTrue(r["stale_fact_error"])
        self.assertTrue(answer_metrics("不知道",[],True,True)["refusal_correct"])

    def test_tfidf_only_fits_corpus(self):
        index=TfidfIndex([{"id":"a","content":"known words"}])
        before=dict(index.idf)
        self.assertEqual(index.search("unseen",10),[])
        self.assertEqual(index.idf,before)

    def test_dataset_rejects_bad_labels(self):
        dataset=json.loads(Path("evals/datasets/memory_v1.json").read_text())
        validate_dataset(dataset)
        dataset["queries"][0]["qrels"]={"absent":1}
        with self.assertRaises(ValueError):
            validate_dataset(dataset)

    def test_existing_report_is_never_overwritten(self):
        with tempfile.TemporaryDirectory() as folder:
            manifest=Path(folder)/"manifest.json"
            manifest.write_text('{"status":"completed"}')
            result=subprocess.run([sys.executable,"-m","evals.memory_eval","--output",folder],capture_output=True)
            self.assertEqual(result.returncode,2)
            self.assertEqual(manifest.read_text(),'{"status":"completed"}')

    def test_timeout_is_not_a_correct_abstention(self):
        class FakeSandbox:
            def call(self,*args,**kwargs):
                return {"sections":[],"estimated_tokens":0}
        index=TfidfIndex([])
        with patch("evals.memory_eval.runner.retrieve",return_value={"hits":[],"latency_ms":3000,"error":"db_statement_timeout"}):
            result=qa_case(FakeSandbox(),"t",{"id":"q","query":"unknown"},
                           {"answers":[],"unanswerable":True,"qrels":{}},index,{},"bm25",20)
        self.assertFalse(result["answer_correct"])
        self.assertFalse(result["refusal_correct"])

    def test_context_receives_relevance_score_without_changing_raw_retrieval(self):
        raw={"memory_version_id":"v","provider_score":.2}
        class FakeSandbox:
            def call(self,action,tenant,data):
                if action=="rank":
                    self.ranked=data
                    return {"hits":[{**raw,"provider_score":.8}]}
                self.context=data
                return {"sections":[],"estimated_tokens":0}
        sb=FakeSandbox();index=TfidfIndex([{"id":"d","content":"fact"}])
        with patch("evals.memory_eval.runner.retrieve",return_value={"hits":[raw],"latency_ms":1}):
            result=qa_case(sb,"t",{"id":"q","query":"fact"},{"answers":["fact"],"unanswerable":False,"qrels":{"d":1}},index,{"d":raw},"vector_tfidf",20)
        self.assertTrue(sb.ranked["rerank_for_context"])
        self.assertEqual(sb.context["hits"][0]["provider_score"],.8)
        self.assertEqual(result["raw_hits"][0]["provider_score"],.2)
        self.assertEqual(result["retrieved"],["d"])

    def test_adapter_never_imports_gold_or_filters_evidence(self):
        row={"question_id":"q_abs","question_type":"single-session-user","question":"Where?",
             "answer":"GOLD_NEVER_INGEST","question_date":"2024/01/03 (Wed) 10:00",
             "haystack_dates":["2024/01/02 (Tue) 10:00","2024/01/01 (Mon) 10:00"],
             "haystack_session_ids":["distractor","evidence"],"answer_session_ids":["evidence"],
             "haystack_sessions":[[{"role":"assistant","content":"distractor content","has_answer":False}],
                                  [{"role":"user","content":"evidence content","has_answer":True}]]}
        with tempfile.TemporaryDirectory() as folder:
            path=Path(folder)/"data.json";path.write_text(json.dumps([row]))
            result=adapt(path,1)
            imported=json.dumps(result["cases"][0]["input"])
            self.assertNotIn("GOLD_NEVER_INGEST",imported)
            self.assertNotIn("has_answer",imported)
            self.assertIn("distractor content",imported)
            self.assertEqual(result["cases"][0]["input"]["documents"][0]["session_id"],"evidence")
            self.assertTrue(result["cases"][0]["labels"]["unanswerable"])
            self.assertEqual(result,adapt(path,1))
            row["answer_session_ids"]=["unknown"]
            path.write_text(json.dumps([row]))
            with self.assertRaises(ValueError):
                adapt(path,1)

    def test_adapter_chunk_boundary_is_valid_for_repository(self):
        row={"question_id":"q","question_type":"single-session-user","question":"q","answer":"a",
             "question_date":"2024/01/01 00:00","haystack_dates":["2023/01/01 00:00"],"haystack_session_ids":["s"],
             "haystack_sessions":[[{"role":"user","content":"x"*999+" "+"y"*10}]],"answer_session_ids":["s"]}
        with tempfile.TemporaryDirectory() as folder:
            path=Path(folder)/"d.json";path.write_text(json.dumps([row]))
            docs=adapt(path,1)["cases"][0]["input"]["documents"]
            self.assertEqual(len(docs),2)
            self.assertTrue(all(d["content"] == d["content"].strip() for d in docs))


if __name__ == "__main__":
    unittest.main()
