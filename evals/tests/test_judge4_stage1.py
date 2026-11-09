"""V4第一阶段完整模拟、失败停止与预算回归；永不调用真实模型或读取Key。"""
from copy import deepcopy
from decimal import Decimal
from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import unittest
from unittest.mock import patch

from evals.memory_eval import judge4,judge4_stage1 as stage
from evals.memory_eval.models3 import BudgetStop,ChatClient,Ledger,json_http
from evals.memory_eval.protocol3 import digest,JUDGE_SYSTEM,READER_SYSTEM


def approval_fixture(plan):
    return {'approved':True,'stage_id':stage.STAGE,'plan_seal':plan['seal'],'scope':'calibration_and_controls_only',
            'calibration_labels_human_reviewed':True,'control_labels_human_reviewed':True,'formal_authorized':False,
            'max_requests':32,'role_request_limits':{'smoke':2,'calibration':16,'controls':14},
            'max_cost_cny':'1.240064','global_cap_cny':'12.00','reviewer':'offline-test-fixture','prices_rechecked':True}


def prior_fixture(folder,plan):
    folder.mkdir();ledger=folder/'previous.jsonl';ledger.write_bytes((judge4.PREVIOUS/'calls.jsonl').read_bytes())
    old=plan['stage'];claim=folder/(old['previous_budget_key']+'.json')
    claim.write_text(json.dumps({'budget_key':old['previous_budget_key'],'plan_seal':old['previous_plan_seal'],'ledger_path':str(ledger)}))
    return ledger


class ScriptedAPI:
    """按预定用例顺序返回手工标签夹具；不根据文本关键词预测语义。"""
    def __init__(self,plan,mutations=None,failure=None,usage_at_cap=False):
        self.plan=plan;self.calls=[];self.mutations=mutations or {};self.failure=failure;self.usage_at_cap=usage_at_cap
        data=[json.loads(judge4.CALIBRATION.read_text()),judge4.load_controls()]
        self.expected={c['id']:c['expected'] for d in data for c in d['cases']}

    def __call__(self,payload,timeout):
        index=len(self.calls);record=self.plan['inputs'][index];self.calls.append(payload)
        if timeout!=45:raise AssertionError('total request timeout changed')
        if self.failure and record['id']==self.failure[0] and self.failure[1]=='timeout':raise TimeoutError('mock timeout')
        if record['role']=='smoke':value={'ok':True}
        else:
            expected=self.expected[record['id']];value={'accurate':expected['semantic_accuracy'],'supported':expected['supported'],
                                                      'reason':'scripted fixture; not real Judge inference'}
            value.update(self.mutations.get(record['id'],{}))
        usage={'prompt_tokens':(16384 if record['role']=='smoke' else 8192) if self.usage_at_cap else 100,
               'completion_tokens':payload['max_tokens'] if self.usage_at_cap else 20,
               'completion_tokens_details':{'reasoning_tokens':0}}
        response={'id':'mock-response-'+str(index+1),'model':payload['model'],'system_fingerprint':'mock-only',
                  'choices':[{'finish_reason':'stop','message':{'role':'assistant','content':json.dumps(value)}}],'usage':usage}
        if self.failure and record['id']==self.failure[0]:
            mode=self.failure[1]
            if mode=='invalid_json':response['choices'][0]['message']['content']='not json'
            elif mode=='missing_usage':response.pop('usage')
            elif mode=='reasoning':response['usage']['completion_tokens_details']['reasoning_tokens']=1
            elif mode=='truncated':response['choices'][0]['finish_reason']='length'
            elif mode=='http_error':return {'http_status':429,'http_error_body':'mock rate limit','id':'mock-http-trace'}
            elif mode=='over_usage':response['usage']['prompt_tokens']=999999
        return response


class JudgeStage1Tests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.addCleanup(self.temp.cleanup);self.root=Path(self.temp.name)
        self.plan=stage.prepare(self.root/'sealed');self.plan_path=self.root/'sealed/plan.json'
        self.approval=self.root/'approval.json';self.approval.write_text(json.dumps(approval_fixture(self.plan)))
        self.prior=self.root/'prior';self.old_ledger=prior_fixture(self.prior,self.plan);self.claims=self.root/'simulation-claims'

    def run_mock(self,api=None,out='result'):
        api=api or ScriptedAPI(self.plan)
        with patch('evals.memory_eval.models3.os.environ.get',side_effect=AssertionError('must not read API Key in simulation')):
            result=stage.run_stage(self.plan_path,self.approval,self.root/out,transport=api,claim_root=self.claims,previous_claim_root=self.prior)
        return result,api

    def test_prepare_creates_no_claim_or_authorization_and_has_32_inputs(self):
        self.assertFalse(self.claims.exists());self.assertEqual(len(self.plan['inputs']),32)
        self.assertEqual(self.plan['stage']['limits']['role_request_limits'],{'smoke':2,'calibration':16,'controls':14})
        self.assertFalse(self.plan['stage']['paid_execution_authorized'])
        self.assertEqual(self.plan['stage']['limits']['max_cost_cny'],'1.240064')
        self.assertEqual(self.plan['stage']['cumulative_stage1_worst_cny'],'1.275632')

    def test_complete_32_call_simulation_keeps_gates_separate_and_never_starts_formal(self):
        with patch('evals.memory_eval.phase3.run_models',side_effect=AssertionError('formal runner forbidden')):
            result,api=self.run_mock()
        self.assertEqual(len(api.calls),32);self.assertEqual(result['status'],'completed_stage1')
        self.assertEqual(result['role_counts'],{'smoke':2,'calibration':16,'controls':14})
        self.assertEqual(result['gates']['calibration']['result']['complete_case_matches'],16)
        self.assertEqual(result['gates']['controls']['result']['complete_case_matches'],14)
        self.assertFalse(result['formal_authorized']);self.assertEqual(result['formal_calls'],0)
        handoff=json.loads((self.root/'result/handoff-receipt.json').read_text())
        self.assertFalse(handoff['formal_authorized']);self.assertEqual(handoff['formal_requests_allowed'],0)
        self.assertEqual(Decimal(result['estimated_or_reserved_cost_cny']),Decimal('0.01664'))

    def test_wire_payloads_use_v4_reference_and_evidence_but_no_private_annotations(self):
        result,api=self.run_mock()
        for record,payload in zip(self.plan['inputs'],api.calls):
            self.assertEqual(payload,record['payload'])
            if record['role']=='smoke':continue
            self.assertEqual(digest(payload['messages'][0]['content']),self.plan['stage']['judge_prompt_sha256'])
            value=json.loads(payload['messages'][1]['content'])
            self.assertIn('reference_answers',value);self.assertIn('evidence',value)
            for key in ['expected','annotated_asserted_claims','critical_checks','omitted_required_items']:
                self.assertNotIn(key,value)
        self.assertEqual(result['formal_calls'],0)

    def test_all_trace_ids_raw_outputs_usage_and_input_hashes_are_preserved(self):
        self.run_mock();out=self.root/'result';paths=list((out/'traces').rglob('*.json'));self.assertEqual(len(paths),32)
        events=[json.loads(s) for s in (out/'calls.jsonl').read_text().splitlines()]
        inputs={e['request']:e for e in events if e['event']=='model_input'}
        for path in paths:
            trace=json.loads(path.read_text());request=trace['request']
            self.assertEqual(trace['response_id'],'mock-response-'+str(request))
            self.assertEqual(trace['usage']['prompt_tokens'],100)
            self.assertEqual(digest(trace['payload']['messages']),trace['input_sha256'])
            self.assertEqual(inputs[request]['payload'],trace['payload'])
            self.assertIsNotNone(trace['raw_response']);self.assertIsNotNone(trace['settlement'])

    def test_first_gate_critical_error_stops_before_any_controls(self):
        result,api=self.run_mock(ScriptedAPI(self.plan,{'partial_answer':{'supported':False}}))
        self.assertEqual(result['status'],'stopped');self.assertEqual(result['failed_case_id'],'partial_answer')
        self.assertEqual(result['role_counts']['controls'],0);self.assertEqual(result['gates']['controls']['status'],'not_run')
        self.assertEqual(result['gates']['calibration']['result']['critical_errors'][0]['code'],'partial_answer_marked_unsupported')
        self.assertEqual(len(api.calls),16);self.assertFalse((self.root/'result/handoff-receipt.json').exists())

    def test_first_gate_one_noncritical_mismatch_still_allows_15_of_16(self):
        result,api=self.run_mock(ScriptedAPI(self.plan,{'number_format':{'accurate':False}}))
        self.assertEqual(result['status'],'completed_stage1');self.assertEqual(len(api.calls),32)
        self.assertEqual(result['gates']['calibration']['result']['complete_case_matches'],15)

    def test_first_gate_two_noncritical_mismatches_stop_when_threshold_impossible(self):
        result,api=self.run_mock(ScriptedAPI(self.plan,{'full_sentence':{'accurate':False},'number_format':{'accurate':False}}))
        self.assertEqual(result['status'],'stopped');self.assertLess(len(api.calls),18)
        self.assertEqual(result['role_counts']['controls'],0)

    def test_second_gate_c01_failure_stops_at_first_new_control(self):
        result,api=self.run_mock(ScriptedAPI(self.plan,{'c01':{'supported':False}}))
        self.assertEqual(len(api.calls),19);self.assertEqual(result['status'],'stopped')
        self.assertEqual(result['gates']['calibration']['status'],'passed')
        self.assertEqual(result['gates']['controls']['result']['expected_cases'],14)
        self.assertEqual(result['gates']['controls']['result']['completed_cases'],1)
        self.assertFalse((self.root/'result/handoff-receipt.json').exists())

    def test_second_gate_c02_failure_does_not_change_prompt_or_continue(self):
        result,api=self.run_mock(ScriptedAPI(self.plan,{'c02':{'supported':False}}))
        self.assertEqual(len(api.calls),20);self.assertEqual(result['failed_case_id'],'c02')
        self.assertEqual(result['role_counts']['controls'],2);self.assertEqual(result['formal_calls'],0)

    def test_transport_failure_keeps_reservation_and_does_not_retry(self):
        result,api=self.run_mock(ScriptedAPI(self.plan,failure=('preference_rubric_positive','timeout')))
        self.assertEqual(len(api.calls),3);self.assertEqual(result['unsettled_reservations'],1)
        self.assertEqual(Decimal(result['estimated_or_reserved_cost_cny']),Decimal('0.00052')*2+Decimal('0.036608'))
        trace=json.loads((self.root/'result/traces/calibration/preference_rubric_positive.json').read_text())
        self.assertIsNone(trace['raw_response']);self.assertIsNotNone(trace['reservation']);self.assertIsNone(trace['settlement'])

    def test_invalid_output_usage_or_http_response_never_releases_reservation(self):
        for index,mode in enumerate(['invalid_json','missing_usage','reasoning','truncated','http_error','over_usage']):
            with self.subTest(mode):
                claims=self.root/('claims-'+str(index));api=ScriptedAPI(self.plan,failure=('preference_rubric_positive',mode))
                result=stage.run_stage(self.plan_path,self.approval,self.root/('bad-'+str(index)),transport=api,claim_root=claims,previous_claim_root=self.prior)
                self.assertEqual(len(api.calls),3);self.assertEqual(result['unsettled_reservations'],1)
                self.assertEqual(Decimal(result['estimated_or_reserved_cost_cny']),Decimal('0.037648'))
                trace=json.loads((self.root/('bad-'+str(index))/'traces/calibration/preference_rubric_positive.json').read_text())
                self.assertIsNotNone(trace['raw_response']);self.assertIsNone(trace['settlement'])

    def test_invalid_judge_schema_including_refusal_nonnull_is_not_settled(self):
        result,api=self.run_mock(ScriptedAPI(self.plan,{'correct_abstention':{'supported':False}}))
        self.assertEqual(result['status'],'stopped');self.assertEqual(result['unsettled_reservations'],1)
        self.assertEqual(result['role_counts']['controls'],0)
        trace=json.loads((self.root/'result/traces/calibration/correct_abstention.json').read_text())
        self.assertFalse(json.loads(trace['raw_response']['choices'][0]['message']['content'])['supported'])

    def test_maximum_valid_usage_stays_below_stage_and_cumulative_caps(self):
        result,api=self.run_mock(ScriptedAPI(self.plan,usage_at_cap=True))
        self.assertEqual(len(api.calls),32);self.assertEqual(result['status'],'completed_stage1')
        self.assertLessEqual(Decimal(result['estimated_or_reserved_cost_cny']),Decimal('1.240064'))
        self.assertLessEqual(Decimal(result['cumulative_spent_or_reserved_cny']),Decimal('12'))

    def test_role_request_token_and_cost_caps_reject_extra_reservations(self):
        limits=self.plan['stage']['limits']
        for role,n in limits['role_request_limits'].items():
            with self.subTest(role):
                ledger=Ledger(self.root/(role+'.jsonl'),limits)
                try:
                    for _ in range(n):
                        r=ledger.reserve(role,100,10);ledger.settle(r,{'prompt_tokens':0,'completion_tokens':0},{})
                    with self.assertRaises(BudgetStop):ledger.reserve(role,100,10)
                finally:ledger.file.close()
        for key,value in [('max_input_tokens_total',1),('max_output_tokens_total',1),('max_cost_cny','0.000001')]:
            ledger=Ledger(self.root/(key+'.jsonl'),{**limits,key:value})
            try:
                with self.assertRaises(BudgetStop):ledger.reserve('calibration',8192,640)
                self.assertEqual(ledger.requests,0)
            finally:ledger.file.close()
        ledger=Ledger(self.root/'forbidden.jsonl',limits)
        try:
            with self.assertRaises(BudgetStop):ledger.reserve('reader',100,10)
        finally:ledger.file.close()

    def test_no_approval_or_broadened_scope_never_reads_key_or_creates_claim(self):
        original=approval_fixture(self.plan)
        for key,value in [('approved',False),('approved','yes'),('control_labels_human_reviewed',False),
                          ('formal_authorized',True),('max_requests',144),('max_cost_cny','12'),('prices_rechecked',False),('stage_id','formal')]:
            with self.subTest(key=key,value=value):
                changed={**original,key:value};self.approval.write_text(json.dumps(changed))
                with patch('evals.memory_eval.judge4_stage1.os.environ.get',side_effect=AssertionError('key read before approval')):
                    with self.assertRaises(BudgetStop):stage.run_stage(self.plan_path,self.approval,self.root/'unauthorized')
        self.assertFalse((self.root/'unauthorized').exists())

    def test_changed_plan_data_scope_rejected_even_if_new_seal_is_supplied(self):
        plan=deepcopy(self.plan);plan['stage']['limits']['role_request_limits']['reader']=56
        plan.pop('seal');plan['seal']=digest(plan)
        p=self.root/'tampered.json';p.write_text(json.dumps(plan));approval=approval_fixture(plan);approval['role_request_limits']=plan['stage']['limits']['role_request_limits']
        self.approval.write_text(json.dumps(approval))
        with self.assertRaises(BudgetStop):stage.run_stage(p,self.approval,self.root/'broadened',transport=ScriptedAPI(plan),claim_root=self.claims,previous_claim_root=self.prior)
        self.assertFalse(self.claims.exists())

    def test_previous_unsettled_history_stops_before_new_claim(self):
        self.old_ledger.write_text(self.old_ledger.read_text()+'{"event":"reserved","request":19}\n')
        api=ScriptedAPI(self.plan)
        with self.assertRaises(BudgetStop):self.run_mock(api)
        self.assertEqual(api.calls,[]);self.assertFalse(self.claims.exists())

    def test_new_output_or_copied_approval_cannot_restart_completed_stage(self):
        self.run_mock();old_claim=(self.claims/(self.plan['budget_key']+'.json')).read_bytes()
        copied=self.root/'copied.json';copied.write_bytes(self.approval.read_bytes());api=ScriptedAPI(self.plan)
        with self.assertRaises(BudgetStop):stage.run_stage(self.plan_path,copied,self.root/'new-output',transport=api,claim_root=self.claims,previous_claim_root=self.prior)
        self.assertEqual(api.calls,[]);self.assertFalse((self.root/'new-output').exists())
        self.assertEqual((self.claims/(self.plan['budget_key']+'.json')).read_bytes(),old_claim)

    def test_partial_claim_is_not_automatically_deleted(self):
        self.claims.mkdir();(self.claims/(self.plan['budget_key']+'.json')).touch()
        api=ScriptedAPI(self.plan)
        with self.assertRaises(BudgetStop):self.run_mock(api)
        self.assertEqual(api.calls,[]);self.assertEqual((self.claims/(self.plan['budget_key']+'.json')).read_bytes(),b'')

    def test_formal_claim_or_real_directory_override_is_rejected(self):
        self.claims.mkdir();(self.claims/(self.plan['stage']['formal_stage']['separate_budget_key']+'.json')).touch()
        with self.assertRaises(BudgetStop):self.run_mock()
        with self.assertRaises(BudgetStop):stage.run_stage(self.plan_path,self.approval,self.root/'override',claim_root=self.claims)

    def test_two_processes_different_outputs_have_one_simulation_winner(self):
        script="""import sys,time
from pathlib import Path
from evals.memory_eval import judge4_stage1 as stage
from evals.memory_eval.models3 import BudgetStop
from evals.tests.test_judge4_stage1 import ScriptedAPI
import json
base=Path(sys.argv[1]);plan=json.loads((base/'sealed/plan.json').read_text())
while not (base/'start').exists():time.sleep(.01)
try:
    result=stage.run_stage(base/'sealed/plan.json',base/'approval.json',base/sys.argv[2],transport=ScriptedAPI(plan),claim_root=base/'simulation-claims',previous_claim_root=base/'prior')
except BudgetStop:sys.exit(4)
sys.exit(0 if result['status']=='completed_stage1' else 8)
"""
        children=[subprocess.Popen([sys.executable,'-c',script,str(self.root),out],stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True) for out in ['one','two']]
        (self.root/'start').touch();results=[p.communicate(timeout=30) for p in children]
        self.assertEqual(sorted(p.returncode for p in children),[0,4],results)
        self.assertEqual(sum((self.root/p).exists() for p in ['one','two']),1)

    def test_process_crash_after_reservation_cannot_open_a_fresh_budget(self):
        script="""import json,os,sys
from pathlib import Path
from evals.memory_eval import judge4_stage1 as stage
base=Path(sys.argv[1])
def crash(*args):os._exit(9)
stage.run_stage(base/'sealed/plan.json',base/'approval.json',base/'crashed',transport=crash,claim_root=base/'simulation-claims',previous_claim_root=base/'prior')
"""
        child=subprocess.run([sys.executable,'-c',script,str(self.root)],capture_output=True,text=True,timeout=15)
        self.assertEqual(child.returncode,9,child.stderr)
        path=self.root/'crashed/calls.jsonl';events=[json.loads(s) for s in path.read_text().splitlines()]
        self.assertEqual(events[-1]['event'],'model_input');self.assertTrue(any(e['event']=='reserved' for e in events))
        old=path.read_bytes()
        with self.assertRaises(BudgetStop):self.run_mock(out='restart')
        self.assertEqual(path.read_bytes(),old)

    def test_simulation_cannot_write_canonical_paid_claim_directory(self):
        with self.assertRaises(BudgetStop):stage.run_stage(self.plan_path,self.approval,self.root/'bad',transport=ScriptedAPI(self.plan),claim_root=stage.CLAIMS,previous_claim_root=self.prior)

    def test_prompts_datasets_and_two_stage_design_are_preserved(self):
        old=json.loads((judge4.PREVIOUS/'approved-plan.json').read_text())
        self.assertEqual(digest(JUDGE_SYSTEM),old['judge_prompt_sha256']);self.assertEqual(digest(READER_SYSTEM),old['reader_prompt_sha256'])
        self.assertEqual(digest(judge4.JUDGE_SYSTEM_V4),'ba7ab1acc3aec7b379313b7b20ca61a2226c05feb40418c6fd43022f4eed6d05')
        self.assertTrue(self.plan['stage']['joint_judge_sees_reference_and_evidence'])
        self.assertFalse(self.plan['stage']['support_only_view_used_for_calls'])

    def test_http_error_body_and_supplier_trace_preserved_under_same_deadline(self):
        class Handler(BaseHTTPRequestHandler):
            def do_POST(self):
                body=b'{"error":"mock quota"}';self.send_response(429);self.send_header('Content-Length',str(len(body)))
                self.send_header('x-siliconcloud-trace-id','mock-429');self.end_headers();self.wfile.write(body)
            def log_message(self,*args):pass
        server=ThreadingHTTPServer(('127.0.0.1',0),Handler);thread=threading.Thread(target=server.serve_forever,daemon=True);thread.start()
        try:
            response=json_http('http://127.0.0.1:'+str(server.server_port)+'/mock',{},1,capture_http_errors=True)
            self.assertEqual(response['http_status'],429);self.assertEqual(response['id'],'mock-429')
            self.assertEqual(json.loads(response['http_error_body']),{'error':'mock quota'})
        finally:server.shutdown();server.server_close();thread.join(timeout=3)


if __name__=='__main__':unittest.main()
