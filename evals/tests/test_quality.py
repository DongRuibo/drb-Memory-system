"""新质量协议的离线及隔离PG测试；不接触模型API或真实业务库。"""
from copy import deepcopy
import csv
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from evals.memory_eval import quality
from evals.memory_eval.baselines import TfidfIndex
from evals.memory_eval.models3 import TextTokenizer
from evals.memory_eval.runner import Sandbox,corpus,governance


class CounterFixture:
    # 仅用于输入隔离单测；精确Token测试另用真实固定Hash tokenizer。
    def count(self,text):return 10 if text else 0


class QualityTests(unittest.TestCase):
    def test_drafts_have_48_queries_24_scenarios_and_pending_labels(self):
        config,data,gov,docs=quality.load_inputs()
        self.assertEqual(len(data['queries']),48);self.assertEqual(len(gov['cases']),24);self.assertEqual(len(docs),240)
        self.assertEqual(sum(q['unanswerable'] for q in data['queries']),6)
        self.assertTrue(all(q['human_review']=='pending' for q in data['queries']))
        self.assertTrue(all(c['human_review']=='pending' for c in gov['cases']))
        self.assertEqual(config['paid_api_calls_limit'],0)

    def test_known_gaps_are_ability_failures_not_expected_passes(self):
        _,_,gov,_=quality.load_inputs();cases={c['id']:c for c in gov['cases']}
        self.assertTrue(cases['g03']['labels']['duplicate']);self.assertFalse(cases['g06']['labels']['conflict'])
        self.assertEqual(cases['g03']['expected']['head_state'],'active')

    def test_multiturn_is_explicit_concat_without_answer_labels(self):
        _,data,_,_=quality.load_inputs();q=deepcopy(data['queries'][42]);q['reference_fact']='GOLD_SENTINEL'
        text=quality.query_text(q)
        self.assertIn(q['history'][0]['content'],text);self.assertIn(q['query'],text);self.assertNotIn('GOLD_SENTINEL',text)
        self.assertEqual(quality.query_text(data['queries'][0]),data['queries'][0]['query'])

    def test_real_tokenizer_counts_rendered_payload_not_characters(self):
        config,_,_,_=quality.load_inputs();entry=config['tokenizer']
        tokenizer=TextTokenizer('/tmp/drb-phase3-assets/DeepSeek-V3.2/tokenizer.json',entry['sha256'])
        full=[{'memory_version_id':'v1','content':'用户在珠海办公。'},{'memory_version_id':'v2','content':'The meeting starts at 09:30 UTC.'}]
        pack=[{'memory_version_id':'v1','text':'用户在珠海办公。'}]
        value=quality.token_comparison(full,pack,tokenizer)
        self.assertEqual(value['full_tokens'],tokenizer.count(quality.render_memories(full)))
        self.assertEqual(value['pack_tokens'],tokenizer.count(quality.render_memories(pack)))
        self.assertNotEqual(value['full_tokens'],len(value['full_rendered']))
        self.assertAlmostEqual(value['token_saving_ratio'],1-value['pack_tokens']/value['full_tokens'])
        self.assertEqual(quality.render_memories(full[:1]),quality.render_memories(pack))

    def test_empty_baseline_has_null_saving_and_no_fabricated_ratio(self):
        value=quality.token_comparison([],[],CounterFixture());self.assertIsNone(value['token_saving_ratio'])
        self.assertEqual(value['full_tokens'],0)

    def test_stage_first_loss_distinguishes_source_rrf_auth_and_rerank(self):
        for name in ['source_union','rrf','authorization','rerank','context']:
            names=['source_union','rrf','authorization','rerank','context'];index=names.index(name)
            stages={n:['gold'] if i<index else ['noise'] for i,n in enumerate(names)}
            result=quality.stage_failures(stages,{'gold'})
            self.assertEqual(result['first_loss'],name)
        self.assertIsNone(quality.stage_failures({'source_union':[]},set())['first_loss'])

    def test_multihop_partial_is_not_complete_and_nonempty_is_not_evidence(self):
        q={'required_evidence':[{'document_id':'a','spans':['甲事实']},{'document_id':'b','spans':['乙事实']}]}
        one=quality.evidence_retention(q,[{'memory_version_id':'va','text':'甲事实'}],{'va':'a'})
        self.assertEqual(one['evidence_recall'],.5);self.assertFalse(one['all_evidence_delivered']);self.assertFalse(one['effective_context_empty'])
        none=quality.evidence_retention(q,[{'memory_version_id':'vx','text':'不相关'}],{'vx':'x'})
        self.assertTrue(none['effective_context_empty'])
        self.assertIsNone(quality.evidence_retention({'required_evidence':[]},[],{})['effective_context_empty'])

    def test_span_must_really_be_delivered_not_just_its_document_id(self):
        q={'required_evidence':[{'document_id':'a','spans':['2025年7月14日']} ]}
        value=quality.evidence_retention(q,[{'memory_version_id':'va','text':'2025年7月'}],{'va':'a'})
        self.assertEqual(value['evidence_units_delivered'],0)

    def test_prepare_never_creates_sandbox_or_reads_model_credentials(self):
        with (tempfile.TemporaryDirectory() as d,patch.object(quality,'Sandbox',side_effect=AssertionError('no DB in prepare')),
              patch('os.environ.get',side_effect=AssertionError('no key read'))):
            out=Path(d)/'review';quality.prepare(out)
            with (out/'retrieval-review.csv').open() as f:self.assertEqual(len(list(csv.DictReader(f))),48)
            with (out/'governance-review.csv').open() as f:self.assertEqual(len(list(csv.DictReader(f))),24)
            self.assertIn('q48',(out/'retrieval-review.md').read_text());self.assertIn('g24',(out/'governance-review.md').read_text())
            with self.assertRaises(FileExistsError):quality.prepare(out)

    def test_predictions_do_not_receive_gold_even_when_annotations_change(self):
        hit={'memory_version_id':'v','memory_record_id':'r','provider_score':1.0}
        mem={'memory_version_id':'v','content':'一条事实'}
        class SB:
            def __init__(self):self.calls=[]
            def call(self,action,tenant,data,**kwargs):
                self.calls.append((action,tenant,deepcopy(data)))
                if action=='search':return [hit]
                if action=='rank':return {'hits':[hit],'memories':[mem],'trace':{'fused':[hit],'preauthorized':[mem],'reranked':[hit],'postauthorized':[mem]}}
                return {'sections':[{'items':[{'memory_version_id':'v','text':'一条事实'}]}]}
        class Index:
            def search(self,*args):return [('d',1)]
        q={'id':'x','category':'c','query':'问题','history':[],'input_policy':'query_only','unanswerable':False,
           'qrels':{'d':1},'required_evidence':[{'document_id':'d','spans':['一条事实']}],'reference_fact':'GOLD_SENTINEL'}
        config={'candidate_budget':20,'context_char_budget':2048};sb=SB()
        quality.retrieval_case(sb,'temp',q,Index(),{'d':hit},'hybrid',config,CounterFixture(),[mem]);original=sb.calls
        q['reference_fact']='DIFFERENT_GOLD';q['qrels']={'other':1};q['required_evidence']=[];sb=SB()
        quality.retrieval_case(sb,'temp',q,Index(),{'d':hit},'hybrid',config,CounterFixture(),[mem])
        self.assertEqual(original,sb.calls);self.assertNotIn('GOLD_SENTINEL',json.dumps(original))

    def test_timeout_is_quality_failure_not_correct_unanswerability(self):
        class SB:
            def call(self,*args,**kwargs):return {'error':'statement timeout'}
        q={'id':'x','category':'c','query':'x','history':[],'input_policy':'query_only','unanswerable':False,'qrels':{'d':1}}
        r=quality.retrieval_case(SB(),'t',q,None,{},'bm25',{'candidate_budget':20},None,[])
        self.assertEqual(r['recall_at_5'],0);self.assertIsNone(r['token_metrics']);self.assertEqual(r['first_loss'],'request_failure')

    def test_csv_keeps_failed_rows_and_null_measurements(self):
        with tempfile.TemporaryDirectory() as d:
            p=Path(d)/'rows.csv';quality.write_csv(p,[{'id':'ok','error':None},{'id':'bad','error':'timeout','tokens':None}])
            with p.open() as f:rows=list(csv.DictReader(f))
            self.assertEqual(len(rows),2);self.assertEqual(rows[1]['id'],'bad');self.assertEqual(rows[1]['tokens'],'')

    def test_missing_and_duplicate_results_cannot_produce_completed_aggregate(self):
        row={'id':'a','method':'bm25','unanswerable':False}
        with self.assertRaises(ValueError):quality.summarize([row],[],['bm25'],expected_n=2)
        with self.assertRaises(ValueError):quality.summarize([row,row],[],['bm25'],expected_n=2)

    def test_positive_empty_denominator_excludes_no_answer_and_keeps_timeout(self):
        common={'method':'bm25','category':'c','retrieval_ms':1,'recall_at_1':0,'recall_at_5':0,'mrr_at_10':0,'ndcg_at_10':0,
                'stage_hits':{'source_union':True,'context_pack':False},'first_loss':'context_pack','token_metrics':None}
        rows=[{**common,'id':'a','unanswerable':False,'error':None,'literal_context_empty':False,'effective_context_empty':True,'all_evidence_delivered':False},
              {**common,'id':'b','unanswerable':False,'error':'timeout'},
              {**common,'id':'c','unanswerable':True,'error':None,'negative_any_hit':True,
               'recall_at_1':None,'recall_at_5':None,'mrr_at_10':None,'ndcg_at_10':None}]
        r=quality.summarize(rows,[],['bm25'],expected_n=3)['methods']['bm25']
        self.assertEqual(r['positive_n'],2);self.assertEqual(r['negative_n'],1)
        self.assertEqual(r['literal_empty_positive_rate'],0);self.assertEqual(r['literal_empty_measured_n'],1)
        self.assertEqual(r['literal_or_request_failure_rate'],.5);self.assertEqual(r['effective_empty_positive_rate'],1)
        self.assertEqual(r['errors'],1);self.assertIsNone(r['token_saving_micro'])
        self.assertEqual(r['stage_losses']['context_pack']['conditional_denominator'],1)

    def test_identification_confusion_is_separate_from_database_contract_match(self):
        records=[{'id':'a','family':'重复识别','passed':False,'checks':{'duplicate':False},'expected_labels':{'duplicate':True},'observed_duplicate':False},
                 {'id':'b','family':'重复识别','passed':True,'checks':{'duplicate':True},'expected_labels':{'duplicate':False},'observed_duplicate':False}]
        r=quality.summarize([],records,[],expected_n=0)['governance']['重复识别']
        self.assertEqual(r['label_accuracy'],.5);self.assertEqual(r['confusion'],{'tp':0,'tn':1,'fp':0,'fn':1})

    def test_correct_duplicate_label_with_source_failure_does_not_lose_label_credit(self):
        records=[{'id':'a','family':'重复识别','passed':False,'checks':{'duplicate':True,'source_count':False},
                  'expected_labels':{'duplicate':True},'observed_duplicate':True}]
        r=quality.summarize([],records,[],expected_n=0)['governance']['重复识别']
        self.assertEqual(r['accuracy'],0);self.assertEqual(r['label_accuracy'],1);self.assertEqual(r['confusion']['tp'],1)


@unittest.skipUnless(os.environ.get('MEMORY_QUALITY_PG_TEST')=='1','requires explicitly enabled isolated Docker PostgreSQL')
class QualityPGTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.folder=tempfile.TemporaryDirectory();cls.log=(Path(cls.folder.name)/'bridge.log').open('w')
        cls.sb=Sandbox();cls.sb.__enter__()
        try:cls.sb.start(cls.log)
        except Exception:
            cls.sb.__exit__(None,None,None);cls.log.close();cls.folder.cleanup();raise
        _,_,gov,_=quality.load_inputs();cls.cases={c['id']:c for c in gov['cases']}
        value=os.environ.get('MEMORY_QUALITY_TEST_ARTIFACTS');cls.artifacts=Path(value) if value else None
        if cls.artifacts:cls.artifacts.mkdir(parents=True,exist_ok=True)

    @classmethod
    def tearDownClass(cls):
        cls.sb.__exit__(None,None,None);cls.log.close();cls.folder.cleanup()

    def run_case(self,qid):
        value=quality.governance_case(self.sb,self.cases[qid])
        if self.artifacts:quality.write_json(self.artifacts/(qid+'.json'),value)
        return value

    def test_added_trace_preserves_default_rank_and_full_authorization_not_top10(self):
        docs=[{'id':str(i),'content':f'星河组件{i}使用PostgreSQL。'} for i in range(12)]
        h,_,_=corpus(self.sb,'quality-test-trace',docs)
        raw=self.sb.call('search','quality-test-trace',{'query':'星河组件PostgreSQL','budget':20})
        arg={'method':'bm25','query':'星河组件PostgreSQL','bm25':raw,'vector':[],'budget':20}
        plain=self.sb.call('rank','quality-test-trace',arg);traced=self.sb.call('rank','quality-test-trace',{**arg,'trace':True})
        self.assertEqual(plain['hits'],traced['hits']);self.assertNotIn('trace',plain)
        self.assertEqual(len(traced['trace']['fused']),12);self.assertEqual(len(plain['hits']),10)
        full=self.sb.call('authorize','quality-test-trace',{'hits':list(h.values())});self.assertEqual(len(full),12)

    def test_source_event_candidate_version_export_has_real_links(self):
        for qid in ['g13','g14','g15']:
            with self.subTest(qid=qid):
                r=self.run_case(qid);self.assertTrue(r['passed'],r['checks'])
                self.assertTrue(r['checks']['source_chain'])

    def test_old_candidate_and_precompiled_pack_are_both_denied(self):
        for qid in ['g22','g23','g24']:
            with self.subTest(qid=qid):
                r=self.run_case(qid);self.assertTrue(r['passed'],r['checks'])
                self.assertEqual(r['cached_authorized'],[]);self.assertIn('error',r['old_pack_consume'])

    def test_scope_denials_require_owner_positive_control(self):
        for qid in ['g19','g20','g21']:
            with self.subTest(qid=qid):
                r=self.run_case(qid);self.assertTrue(r['passed'],r['checks'])
                self.assertTrue(r['owner_visible']);self.assertFalse(r['scope_allowed'])

    def test_dedup_and_conflict_known_gaps_are_not_silently_relabelled(self):
        for qid,check in [('g03','duplicate'),('g06','conflict')]:
            with self.subTest(qid=qid):
                r=self.run_case(qid);self.assertFalse(r['passed']);self.assertFalse(r['checks'][check])

    def test_review_reject_and_selective_forget_use_existing_rules(self):
        for qid in ['g11','g17','g18']:
            with self.subTest(qid=qid):
                r=self.run_case(qid);self.assertTrue(r['passed'],r['checks'])

    def test_end_to_end_fixture_real_pg_context_and_exact_tokens(self):
        config,_,_,_=quality.load_inputs();entry=config['tokenizer']
        tokenizer=TextTokenizer('/tmp/drb-phase3-assets/DeepSeek-V3.2/tokenizer.json',entry['sha256'])
        docs=[{'id':'home','content':'我住在海口。'},{'id':'hobby','content':'我偏好散步。'}]
        h,actual,_=corpus(self.sb,'quality-test-token',docs);index=TfidfIndex(actual)
        full=self.sb.call('authorize','quality-test-token',{'hits':list(h.values())})
        q={'id':'fixture','category':'测试夹具','query':'我住在海口。','history':[],'input_policy':'query_only',
           'qrels':{'home':1},'unanswerable':False,'required_evidence':[{'document_id':'home','spans':['我住在海口。']}]}
        for method in config['methods']:
            r=quality.retrieval_case(self.sb,'quality-test-token',q,index,h,method,config,tokenizer,full)
            self.assertIsNone(r['error']);self.assertTrue(r['all_evidence_delivered'])
            self.assertEqual(r['token_metrics']['full_memory_count'],2)
            self.assertEqual(r['token_metrics']['pack_tokens'],tokenizer.count(r['token_metrics']['pack_rendered']))
            if self.artifacts:quality.write_json(self.artifacts/('retrieval-fixture-'+method+'.json'),r)

    def test_existing_governance_runner_remains_compatible(self):
        checks,qa=governance(self.sb)
        self.assertEqual(len(checks),11);self.assertTrue(all(c['passed'] for c in checks));self.assertEqual(len(qa),2)
        self.assertEqual(next(c['status'] for c in checks if c['name']=='semantic_paraphrase_dedup_gap'),'known_limitation')


if __name__=='__main__':unittest.main()
