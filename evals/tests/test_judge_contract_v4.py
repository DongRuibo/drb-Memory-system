"""V4输入隔离、冻结资产和既有门禁回归；不模拟自然语言语义评分器。"""
from copy import deepcopy
from decimal import Decimal
import hashlib
import json
import subprocess
import sys
import tempfile
from pathlib import Path
import unittest
from unittest.mock import patch

from evals.memory_eval import judge4
from evals.memory_eval.phase3 import judge_messages
from evals.memory_eval.protocol3 import JUDGE_SYSTEM,READER_SYSTEM,digest


ROOT=Path(__file__).resolve().parents[2]


class JudgeContractV4Tests(unittest.TestCase):
    def setUp(self):
        self.cal=json.loads(judge4.CALIBRATION.read_text());self.controls=judge4.load_controls()
        self.rows=[{'id':c['id'],'judgment':{'accurate':c['expected']['semantic_accuracy'],
                    'supported':c['expected']['supported'],'reason':'人工标签夹具，不是模型结果'}} for c in self.cal['cases']]

    def test_v3_prompt_calibration_and_gate_are_frozen(self):
        old=json.loads((judge4.PREVIOUS/'approved-plan.json').read_text())
        self.assertEqual(digest(JUDGE_SYSTEM),old['judge_prompt_sha256'])
        self.assertEqual(digest(READER_SYSTEM),old['reader_prompt_sha256'])
        self.assertEqual(hashlib.sha256(judge4.CALIBRATION.read_bytes()).hexdigest(),old['calibration_sha256'])
        self.assertEqual(self.cal['gate'],old['calibration_gate'])
        self.assertIn('arithmetic/time relations',judge4.JUDGE_SYSTEM_V4)

    def test_v4_adapter_changes_only_judge_system_message(self):
        c=next(c for c in self.cal['cases'] if c['id']=='partial_answer');inp={'query':c['question']};ctx={'text':c['context']}
        old=judge_messages(inp,c,c['prediction'],ctx);new=judge4.messages(inp,c,c['prediction'],ctx)
        self.assertNotEqual(old[0]['content'],new[0]['content']);self.assertEqual(old[1:],new[1:])
        self.assertEqual(new[0]['content'],judge4.JUDGE_SYSTEM_V4)
        self.assertLess(new[0]['content'].index('Decide supported FIRST'),new[0]['content'].index('Decide accurate INDEPENDENTLY'))
        self.assertLess(new[0]['content'].index('"supported"'),new[0]['content'].index('"accurate"'))

    def test_control_coverage_annotations_are_not_model_scores(self):
        self.assertEqual(len(self.controls['cases']),14)
        self.assertEqual(len({c['family'] for c in self.controls['cases']}),7)
        self.assertEqual(self.controls['human_review_status'],'pending');self.assertIs(self.controls['real_model_results'],False)
        for c in self.controls['cases']:
            self.assertNotIn(c['question'],judge4.JUDGE_SYSTEM_V4)
            for claim in c['annotated_asserted_claims']:self.assertNotIn(claim,judge4.JUDGE_SYSTEM_V4)
            payload=json.loads(judge4.messages({'query':c['question']},c,c['prediction'],{'text':c['context']})[1]['content'])
            for private_annotation in ['expected','annotated_asserted_claims','omitted_required_items','critical_checks']:
                self.assertNotIn(private_annotation,payload)
        self.assertFalse(any(c['id']=='partial_answer' for c in self.controls['cases']))

    def test_annotated_label_dimensions_remain_distinct(self):
        by_id={c['id']:c for c in self.controls['cases']}
        for cid in ['c01','c02']:self.assertEqual(by_id[cid]['expected'],{'semantic_accuracy':False,'supported':True})
        for cid in ['c03','c04','c07','c08','c09','c10','c11']:
            self.assertEqual(by_id[cid]['expected'],{'semantic_accuracy':False,'supported':False})
        for cid in ['c05','c06','c12']:self.assertEqual(by_id[cid]['expected'],{'semantic_accuracy':True,'supported':True})
        self.assertEqual(by_id['c13']['expected'],{'semantic_accuracy':True,'supported':None})
        self.assertEqual(by_id['c14']['expected'],{'semantic_accuracy':False,'supported':None})

    def test_support_only_alternative_excludes_reference_metadata(self):
        c=self.controls['cases'][0];inp={'query':c['question']};ctx={'text':c['context']}
        label=deepcopy(c);label['answers']=['GOLD_SENTINEL'];label['unanswerable']=True
        full=judge4.messages(inp,label,c['prediction'],ctx)
        view=judge4.support_only_view(inp,c['prediction'],ctx)
        self.assertIn('GOLD_SENTINEL',json.dumps(full))
        self.assertNotIn('GOLD_SENTINEL',json.dumps(view));self.assertNotIn('reference_answers',view);self.assertNotIn('unanswerable',view)
        # 这是信息流检查，不声称联合Prompt能在模型内部屏蔽reference。
        self.assertEqual(view['response']['answer'],c['prediction']['answer'])

    def test_real_v3_failure_replayed_without_relabeling_still_fails(self):
        trace=json.loads((judge4.PREVIOUS/'traces/calibration/partial_answer.json').read_text())
        self.assertEqual(digest(trace['payload']['messages']),trace['input_sha256'])
        raw=json.loads(trace['raw_choices'][0]['message']['content'])
        self.assertEqual(raw['supported'],False)
        rows=json.loads((judge4.PREVIOUS/'judge-calibration.json').read_text())
        result=judge4.evaluate(self.cal,[{'id':r['id'],'judgment':r['judgment']} for r in rows])
        self.assertEqual(result['complete_case_matches'],15);self.assertFalse(result['passed'])
        self.assertEqual(result['critical_errors'][0]['code'],'partial_answer_marked_unsupported')

    def test_primary_gate_keeps_16_denominator_and_15_threshold(self):
        self.assertTrue(judge4.evaluate(self.cal,self.rows)['passed'])
        next(r for r in self.rows if r['id']=='number_format')['judgment']['accurate']=False
        self.assertTrue(judge4.evaluate(self.cal,self.rows)['passed'])
        next(r for r in self.rows if r['id']=='date_format')['judgment']['accurate']=False
        self.assertFalse(judge4.evaluate(self.cal,self.rows)['passed'])

    def test_partial_answer_critical_error_is_not_downgraded(self):
        next(r for r in self.rows if r['id']=='partial_answer')['judgment']['supported']=False
        result=judge4.evaluate(self.cal,self.rows)
        self.assertEqual(result['complete_case_matches'],15);self.assertFalse(result['passed'])

    def test_missing_duplicate_unknown_and_invalid_null_results_rejected(self):
        result=judge4.evaluate(self.cal,self.rows[:-1]);self.assertFalse(result['passed']);self.assertEqual(result['expected_cases'],16)
        with self.assertRaises(ValueError):judge4.evaluate(self.cal,self.rows+[self.rows[0]])
        unknown=deepcopy(self.rows);unknown[0]['id']='unknown'
        with self.assertRaises(ValueError):judge4.evaluate(self.cal,unknown)
        invalid=deepcopy(self.rows);next(r for r in invalid if r['id']=='correct_abstention')['judgment']['supported']=False
        with self.assertRaises(ValueError):judge4.evaluate(self.cal,invalid)

    def test_controls_are_a_separate_proposed_gate_not_merged_into_16(self):
        rows=[{'id':c['id'],'judgment':{'accurate':c['expected']['semantic_accuracy'],'supported':c['expected']['supported'],'reason':'fixture'}} for c in self.controls['cases']]
        result=judge4.evaluate(self.controls,rows);self.assertEqual(result['expected_cases'],14);self.assertTrue(result['passed'])
        rows[0]['judgment']['supported']=False
        self.assertFalse(judge4.evaluate(self.controls,rows)['passed'])
        self.assertEqual(self.cal['gate']['expected_cases'],16)

    def test_budget_includes_first_run_and_all_144_new_requests(self):
        contract=judge4.build_contract()
        self.assertEqual(contract['proposed_max_requests'],144)
        self.assertEqual(Decimal(contract['previous_cost_cny']),Decimal('0.035568'))
        self.assertEqual(Decimal(contract['proposed_calibration_stage_ceiling_cny']),Decimal('1.240064'))
        self.assertEqual(Decimal(contract['formal_stage_ceiling_cny']),Decimal('9.691136'))
        self.assertEqual(Decimal(contract['proposed_new_round_hard_cap_cny']),Decimal('10.931200'))
        self.assertEqual(Decimal(contract['cumulative_worst_cost_cny']),Decimal('10.966768'))
        self.assertFalse(contract['authorized'])

    def test_over_12_cost_is_rejected_instead_of_raising_cap(self):
        with patch.object(judge4,'previous_cost',return_value=(Decimal('2.00'),'fixture')):
            with self.assertRaises(ValueError):judge4.build_contract()

    def test_unsettled_or_duplicate_history_requires_reconciliation(self):
        events=[json.loads(s) for s in (judge4.PREVIOUS/'calls.jsonl').read_text().splitlines()]
        with tempfile.TemporaryDirectory() as temp:
            p=Path(temp);target=p/'calls.jsonl'
            for changed in [[e for e in events if not(e['event']=='settled' and e['request']==18)],events+[next(e for e in events if e['event']=='settled')]]:
                target.write_text('\n'.join(json.dumps(e) for e in changed)+'\n')
                with patch.object(judge4,'PREVIOUS',p):
                    with self.assertRaises(ValueError):judge4.previous_cost()

    def test_trial_identity_is_new_but_not_reset_by_prompt_edits(self):
        contract=judge4.build_contract();self.assertNotEqual(contract['proposed_budget_key'],contract['previous_budget_key'])
        with patch.object(judge4,'JUDGE_SYSTEM_V4',judge4.JUDGE_SYSTEM_V4+' hypothetical revision'):
            revised=judge4.build_contract()
            self.assertEqual(revised['proposed_budget_key'],contract['proposed_budget_key'])
            self.assertEqual(revised['proposed_stage_claim_keys'],contract['proposed_stage_claim_keys'])
        self.assertEqual(len(set(contract['proposed_stage_claim_keys'].values())),2)
        self.assertIn('no automatic resume',contract['restart_policy'])

    def test_prepare_is_offline_and_all_30_inputs_fit_8k_without_truncation(self):
        with tempfile.TemporaryDirectory() as temp,patch('urllib.request.OpenerDirector.open',side_effect=AssertionError('network forbidden')):
            out=Path(temp)/'prepared';manifest=judge4.prepare(out)
            self.assertEqual(manifest['paid_calls_this_round'],0);self.assertIsNone(manifest['v4_real_model_results'])
            self.assertLessEqual(manifest['max_short_input_bound'],8192)
            self.assertEqual(len(json.loads((out/'calibration-inputs.json').read_text())),16)
            self.assertEqual(len(json.loads((out/'controls-inputs.json').read_text())),14)
            self.assertTrue(manifest['old_failure_still_blocked'])

    def test_cli_has_no_paid_execution_mode(self):
        p=subprocess.run([sys.executable,'-m','evals.memory_eval.judge4','models-run'],cwd=ROOT,capture_output=True,text=True)
        self.assertEqual(p.returncode,2);self.assertIn('invalid choice',p.stderr)


if __name__=='__main__':unittest.main()
