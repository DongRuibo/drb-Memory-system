"""评分合同、无授权零请求、预算、截断和真实向量接口的离线反例。"""
from copy import deepcopy
from decimal import Decimal
import hashlib
import json
from pathlib import Path
import tempfile
import unittest

from evals.memory_eval.locked3 import make_lock,load_cases
from evals.memory_eval.models3 import BudgetStop,ChatClient,DenseIndex,Ledger,SemanticClient,TextTokenizer,build_context
from evals.memory_eval.phase3 import CALIBRATION,budget_key,check_approval,load_configuration,runtime_hashes
from evals.memory_eval.protocol3 import JUDGE_SYSTEM,READER_SYSTEM,answer_scores,digest,normalized_answer,parse_judge,parse_prediction,stage_scores,summarize

ROOT=Path(__file__).resolve().parents[2]


class CharacterTokenizer:
    sha='fixture-not-a-model'
    add_special_tokens=True
    def count(self,text):return len(text)


class Phase3Tests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.addCleanup(self.temp.cleanup);self.path=Path(self.temp.name)
        self.protocol,self.models,self.lock=load_configuration()

    def ledger(self,**limits):
        value=Ledger(self.path/'ledger.jsonl',{**self.protocol,**limits});self.addCleanup(value.file.close)
        return value

    def response(self,payload,timeout):
        return {'id':'fixture','model':self.models['reader']['model'],'choices':[{'finish_reason':'stop','message':{'content':'{"answer":"Paris","abstained":false,"citations":["s"]}'}}],
                'usage':{'prompt_tokens':100,'completion_tokens':20}}

    def test_all_calibration_strict_and_normalized_contracts(self):
        data=json.loads((ROOT/'evals/datasets/judge_calibration_v1.json').read_text())
        self.assertEqual(data['human_review_status'],'pending')
        self.assertEqual(len(data['cases']),16)
        for c in data['cases']:
            with self.subTest(c['id']):
                score=answer_scores(c['prediction'],c['answers'],c['unanswerable'])
                self.assertEqual(score['strict_match'],c['expected']['strict_match'])
                self.assertEqual(score['normalized_match'],c['expected']['normalized_match'])
                self.assertIsNone(score['semantic_accuracy'])

    def test_normalization_never_merges_wrong_decimal_or_dates(self):
        from evals.memory_eval.metrics import normalize
        self.assertEqual(normalize('1.5'),normalize('15'))
        self.assertNotEqual(normalized_answer('1.5'),normalized_answer('15'))
        self.assertNotEqual(normalized_answer('2024-01-02'),normalized_answer('2024-02-01'))
        self.assertNotEqual(normalized_answer('Paris'),normalized_answer('Ada lives in Paris.'))

    def test_unapproved_reads_no_key_and_sends_no_request(self):
        ledger=self.ledger();client=ChatClient(self.models['reader'],self.protocol,ledger)
        with self.assertRaises(BudgetStop):client.call([{'role':'user','content':'hello'}],'reader')
        self.assertEqual(ledger.requests,0)

    def test_request_limit_stops_before_next_transport(self):
        ledger=self.ledger(max_requests=1);calls=[]
        def transport(p,t):calls.append(p);return self.response(p,t)
        client=ChatClient(self.models['reader'],self.protocol,ledger,transport=transport)
        client.call([{'role':'user','content':'hello'}],'reader')
        with self.assertRaises(BudgetStop):client.call([{'role':'user','content':'hello'}],'reader')
        self.assertEqual(len(calls),1)
        self.assertEqual(ledger.cost,Decimal('0.00052'))

    def test_cost_limit_reserves_worst_before_transport(self):
        ledger=self.ledger(max_cost_cny='0.001');client=ChatClient(self.models['reader'],self.protocol,ledger,transport=self.response)
        with self.assertRaises(BudgetStop):client.call([{'role':'user','content':'small'}],'reader')
        self.assertEqual(ledger.requests,0)

    def test_total_token_limits_stop_without_request(self):
        for field in ['max_input_tokens_total','max_output_tokens_total']:
            with self.subTest(field),tempfile.TemporaryDirectory() as t:
                ledger=Ledger(Path(t)/'calls',{**self.protocol,field:1})
                try:
                    with self.assertRaises(BudgetStop):ledger.reserve('reader',100,100)
                    self.assertEqual(ledger.requests,0)
                finally:ledger.file.close()

    def test_input_cap_no_silent_truncation(self):
        ledger=self.ledger();client=ChatClient(self.models['reader'],self.protocol,ledger,transport=self.response)
        with self.assertRaises(BudgetStop):client.call([{'role':'user','content':'中'*20000}],'reader')
        self.assertEqual(ledger.requests,0)

    def test_timeout_failure_retains_reservation_and_retry_zero(self):
        ledger=self.ledger();calls=[]
        def transport(p,t):calls.append(t);raise TimeoutError('secret-like error text never logged')
        client=ChatClient(self.models['reader'],self.protocol,ledger,transport=transport)
        with self.assertRaises(BudgetStop):client.call([{'role':'user','content':'hi'}],'reader')
        with self.assertRaises(BudgetStop):client.call([{'role':'user','content':'hi'}],'reader')
        self.assertEqual(calls,[45]);self.assertGreater(ledger.cost,0)
        self.assertNotIn('secret-like', (self.path/'ledger.jsonl').read_text())

    def test_ledger_cannot_be_reset_on_restart(self):
        self.ledger()
        with self.assertRaises(FileExistsError):Ledger(self.path/'ledger.jsonl',self.protocol)

    def test_missing_usage_halts_and_retains_reservation(self):
        ledger=self.ledger();r=ledger.reserve('reader',1000,100)
        with self.assertRaises(BudgetStop):ledger.settle(r,{}, {})
        self.assertTrue(ledger.halted);self.assertGreater(ledger.cost,0)

    def test_provider_usage_over_reservation_halts(self):
        ledger=self.ledger();r=ledger.reserve('reader',1000,100)
        with self.assertRaises(BudgetStop):ledger.settle(r,{'prompt_tokens':1001,'completion_tokens':1},{})

    def test_nonthinking_and_output_budget_sent_to_provider(self):
        ledger=self.ledger();calls=[]
        def transport(p,t):calls.append(p);return self.response(p,t)
        ChatClient(self.models['reader'],self.protocol,ledger,transport=transport).call([{'role':'user','content':'hi'}],'reader')
        self.assertFalse(calls[0]['enable_thinking']);self.assertEqual(calls[0]['thinking_budget'],128)
        self.assertEqual(calls[0]['max_tokens'],768)

    def test_unexpected_thinking_or_truncation_stops_campaign(self):
        ledger=self.ledger()
        def transport(p,t):
            r=self.response(p,t);r['choices'][0]['message']['reasoning_content']='unexpected';return r
        with self.assertRaises(BudgetStop):ChatClient(self.models['reader'],self.protocol,ledger,transport=transport).call([{'role':'user','content':'hi'}],'reader')
        self.assertTrue(ledger.halted)

    def test_context_unicode_budget_prefix_and_no_answer_access(self):
        docs=[{'id':'a','content':'早期事实 北京🏠。'*10},{'id':'b','content':'later fact'}]
        pack=build_context(docs,CharacterTokenizer(),{'max_tokens':50,'max_utf8_bytes':75})
        self.assertLessEqual(pack['utf8_bytes'],75);self.assertLessEqual(pack['content_tokens'],50)
        self.assertTrue(pack['items'][0]['truncated']);self.assertEqual(pack['items'][0]['text'],docs[0]['content'][:pack['items'][0]['source_char_end']])
        self.assertNotIn('b',[i['source_id'] for i in pack['items']])

    def test_session_hit_does_not_imply_evidence_recall(self):
        docs=[{'id':'wrong','session_id':'s','content':'irrelevant'},{'id':'gold','session_id':'s','content':'answer'}]
        scores=stage_scores(['wrong'],docs,{'session_qrels':{'s':1},'evidence_qrels':{'gold':1}},{'items':[]})
        self.assertEqual(scores['session']['recall_at_5'],1);self.assertEqual(scores['evidence']['recall_at_5'],0)
        self.assertEqual(scores['context_evidence_coverage'],0)

    def test_context_counts_partial_delivery_and_redaction(self):
        docs=[{'id':'s','session_id':'session','content':'abcdef'}];labels={'session_qrels':{'session':1},'evidence_qrels':{'s':1}}
        result=stage_scores(['s'],docs,labels,{'items':[{'source_id':'s','text':'abc'}]})
        self.assertEqual(result['context_evidence_coverage'],.5)
        result=stage_scores(['s'],docs,labels,{'items':[{'source_id':'s','text':'[REDACTED]'}]})
        self.assertEqual(result['context_evidence_coverage'],0)

    def test_unsupported_correct_guess_separate_from_accuracy(self):
        prediction={'answer':'Paris','abstained':False,'citations':[]}
        result=answer_scores(prediction,['Paris'],False,{'accurate':True,'supported':False})
        self.assertTrue(result['strict_match']);self.assertTrue(result['unsupported_answer'])

    def test_unknown_citations_recorded_not_hidden(self):
        prediction=parse_prediction({'answer':'Paris','abstained':False,'citations':['invented']},{'s'})
        self.assertEqual(prediction['unknown_citations'],['invented'])
        with self.assertRaises(ValueError):parse_judge({'accurate':'yes','supported':True,'reason':'x'},False)

    def test_claiming_abstention_does_not_make_a_guess_correct(self):
        prediction={'answer':'Paris','abstained':True,'citations':[]}
        result=answer_scores(prediction,['unknown'],True,{'accurate':False,'supported':None})
        self.assertTrue(result['declared_abstention']);self.assertFalse(result['abstention_correct'])
        self.assertIsNone(answer_scores(prediction,['unknown'],True)['abstention_correct'])

    def test_failures_stay_in_accuracy_denominator_and_no_judge_is_null(self):
        summary=summarize([{'unanswerable':False,'strict_match':True,'normalized_match':True,'latency_ms':1}],2)
        self.assertEqual(summary['strict_accuracy_all'],.5);self.assertIsNone(summary['semantic_accuracy_all'])

    def test_actual_tokenizer_and_cache_manifest(self):
        entry=self.models['reader_tokenizer'];p=Path('/tmp/drb-phase3-assets/DeepSeek-V3.2/tokenizer.json')
        if not p.exists():self.skipTest('downloaded public tokenizer required; not a model call')
        tokenizer=TextTokenizer(p,entry['files']['tokenizer.json']['sha256'])
        pack=build_context([{'id':'s','content':'日期2024-01-02，剂量1.5mg。'*100}],tokenizer,{'max_tokens':64,'max_utf8_bytes':500})
        self.assertLessEqual(tokenizer.count(pack['text']),64)
        with self.assertRaises(ValueError):TextTokenizer(p,'0'*64)

    def test_embedding_counter_includes_real_eos(self):
        entry=self.models['embedding'];p=Path('/tmp/drb-phase3-assets/Qwen3-Embedding-0.6B/tokenizer.json')
        if not p.exists():self.skipTest('public tokenizer asset required')
        sha=entry['files']['tokenizer.json']['sha256']
        plain=TextTokenizer(p,sha);model=TextTokenizer(p,sha,add_special_tokens=True)
        self.assertEqual(model.count('hello'),plain.count('hello')+1)

    def test_semantic_vectors_are_not_tfidf_and_cache_reused(self):
        manifest=deepcopy(self.models);manifest['embedding_parameters']['dimensions']=2;calls=[]
        def transport(url,payload,timeout):
            if url.endswith('/health'):return {'embedding':{'model':manifest['embedding']['model'],'revision':manifest['embedding']['revision'],'dimensions':2,'normalization':'l2'},
                'max_length':4096,'instruction':manifest['embedding_parameters']['instruction'],
                'device':manifest['embedding_parameters']['device'],'embedding_dtype':manifest['embedding_parameters']['dtype'],
                **{k:manifest['embedding_parameters'][k] for k in ['torch_version','transformers_version','tokenizers_version']}}
            calls.append(payload)
            return {'model':manifest['embedding']['model'],'data':[{'index':i,'embedding':[1.,0.] if t=='harbor' else [0.,1.]} for i,t in enumerate(payload['input'])]}
        client=SemanticClient('http://127.0.0.1:19882',manifest,CharacterTokenizer(),self.path/'cache',transport)
        index=DenseIndex([{'id':'a','content':'harbor'},{'id':'b','content':'orchard'}],client)
        self.assertEqual(index.search('harbor',1)[0][0],'a');self.assertEqual(index.search('harbor',1)[0][0],'a')
        self.assertEqual(len(calls),2)
        with self.assertRaises(ValueError):SemanticClient('https://example.org',manifest,CharacterTokenizer(),self.path/'cache',transport)

    def test_lock_has_no_dev_ids_or_any_history_overlap_and_is_sealed(self):
        self.assertEqual(len(self.lock['cases']),14)
        for key in ['id_overlap','answer_session_overlap','all_history_session_overlap']:self.assertEqual(self.lock[key],0)
        broken=deepcopy(self.lock);broken['cases'][0]['id']='changed'
        with self.assertRaises(ValueError):load_cases([],broken)

    def test_reference_and_has_answer_never_enter_prediction_input(self):
        r={'question_id':'q','question_type':'single-session-user','question':'Where?','answer':'GOLD_SENTINEL','question_date':'2024/01/01 00:00',
           'haystack_dates':['2023/01/01 00:00'],'haystack_session_ids':['s'],'answer_session_ids':['s'],
           'haystack_sessions':[[{'role':'user','content':'ordinary text','has_answer':True}]]}
        lock=make_lock([r],[],1,1);case=load_cases([r],lock)[0]
        self.assertNotIn('GOLD_SENTINEL',json.dumps(case['input']));self.assertNotIn('has_answer',json.dumps(case['input']))
        self.assertEqual(case['labels']['answers'],['GOLD_SENTINEL'])

    def test_no_approval_or_changed_plan_rejected(self):
        p=self.path/'approval.json';p.write_text('{"approved":false}')
        with self.assertRaises(BudgetStop):check_approval(p,{'seal':'wrong'},self.path)

    def test_approval_binds_actual_runtime_and_human_review(self):
        plan={'protocol':self.protocol,'models':self.models,'lock_seal':self.lock['seal'],'budget_key':budget_key(self.lock,self.models),
              'reader_prompt_sha256':digest(READER_SYSTEM),'judge_prompt_sha256':digest(JUDGE_SYSTEM),
              'runtime_sha256':runtime_hashes(),'calibration_sha256':hashlib.sha256(CALIBRATION.read_bytes()).hexdigest()}
        plan['seal']=digest(plan)
        approval={'approved':True,'calibration_labels_human_reviewed':True,'reviewer':'fixture','plan_seal':plan['seal']}
        p=self.path/'approval.json';p.write_text(json.dumps(approval));check_approval(p,plan,self.path)
        for pending in [False,'pending',1]:
            approval['calibration_labels_human_reviewed']=pending;p.write_text(json.dumps(approval))
            with self.assertRaises(BudgetStop):check_approval(p,plan,self.path)
        approval['calibration_labels_human_reviewed']=True
        plan['runtime_sha256']['evals/memory_eval/models3.py']='changed';plan.pop('seal');plan['seal']=digest(plan)
        approval['plan_seal']=plan['seal'];p.write_text(json.dumps(approval))
        with self.assertRaises(BudgetStop):check_approval(p,plan,self.path)

    def test_visible_output_usage_exceeding_cap_stops_within_reservation(self):
        ledger=self.ledger()
        def transport(p,t):
            r=self.response(p,t);r['usage']['completion_tokens']=800;return r
        with self.assertRaises(BudgetStop):ChatClient(self.models['reader'],self.protocol,ledger,transport=transport).call([{'role':'user','content':'hi'}],'reader')
        self.assertTrue(ledger.halted);self.assertLess(ledger.cost,Decimal(self.protocol['max_cost_cny']))


if __name__=='__main__':unittest.main()
