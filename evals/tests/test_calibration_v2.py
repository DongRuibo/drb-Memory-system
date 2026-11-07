"""v2人工标签合同和整例校准门禁；所有Judge结果均为离线夹具。"""
from copy import deepcopy
import hashlib
import json
from pathlib import Path
import unittest

from evals.memory_eval.phase3 import CALIBRATION,LOCK,load_calibration,load_configuration
from evals.memory_eval.protocol3 import READER_SYSTEM,answer_scores,calibration_summary,digest,parse_judge


ROOT=Path(__file__).resolve().parents[2]


class CalibrationV2Tests(unittest.TestCase):
    def setUp(self):
        self.data=json.loads((ROOT/'evals/datasets/judge_calibration_v2.json').read_text())
        self.rows=[{'id':c['id'],'judgment':{'accurate':c['expected']['semantic_accuracy'],
                   'supported':c['expected']['supported'],'reason':'离线标签夹具，不是模型判断'}} for c in self.data['cases']]

    def test_v1_and_locked_test_are_unchanged(self):
        v1=ROOT/'evals/datasets/judge_calibration_v1.json'
        self.assertEqual(hashlib.sha256(v1.read_bytes()).hexdigest(),'93ab2e268dfa40a86a4fc38dee1f2c84789b34bed51b14b61bce598e97d560a1')
        self.assertEqual(hashlib.sha256(LOCK.read_bytes()).hexdigest(),'7c50ca5ed56dc660773e32352c163a9b7edd6d243bed35cd4855510a82c9d417')
        self.assertEqual(self.data['derived_from']['sha256'],hashlib.sha256(v1.read_bytes()).hexdigest())

    def test_only_three_cases_change_content_and_no_expected_label_changes(self):
        v1=json.loads((ROOT/'evals/datasets/judge_calibration_v1.json').read_text())
        self.assertEqual([c['id'] for c in v1['cases']],[c['id'] for c in self.data['cases']])
        modified=[]
        for old,new,record in zip(v1['cases'],self.data['cases'],self.data['changes']):
            self.assertEqual(old['expected'],new['expected'])
            changed={k:{'before':old[k],'after':new[k]} for k in ['question','answers','context','prediction'] if old[k]!=new[k]}
            self.assertEqual(changed,record['changed_fields'])
            if changed:modified.append(new['id'])
        self.assertEqual(set(modified),{'synonym','invented_explanation','arithmetic'})
        cases={c['id']:c for c in self.data['cases']}
        self.assertEqual(cases['synonym']['answers'],['sparkling water'])
        self.assertEqual(cases['synonym']['prediction']['answer'],'carbonated water')
        self.assertIn('did not overlap',cases['arithmetic']['context'])
        self.assertIn('did not live there before 2020',cases['invented_explanation']['context'])
        self.assertEqual(self.data['human_review_status'],'pending_final_confirmation')
        self.assertIs(self.data['paid_api_authorized'],False)

    def test_all_16_strict_and_normalized_contracts_remain_separate(self):
        for c in self.data['cases']:
            with self.subTest(c['id']):
                result=answer_scores(c['prediction'],c['answers'],c['unanswerable'])
                self.assertEqual(result['strict_match'],c['expected']['strict_match'])
                self.assertEqual(result['normalized_match'],c['expected']['normalized_match'])
                self.assertIsNone(result['semantic_accuracy'])

    def test_all_16_complete_matches_pass(self):
        result=calibration_summary(self.data,self.rows)
        self.assertTrue(result['passed']);self.assertEqual(result['complete_case_matches'],16)
        self.assertEqual(result['agreement'],1);self.assertEqual(result['critical_errors'],[])

    def test_15_complete_matches_with_only_noncritical_error_pass(self):
        next(r for r in self.rows if r['id']=='number_format')['judgment']['accurate']=False
        result=calibration_summary(self.data,self.rows)
        self.assertTrue(result['passed']);self.assertEqual(result['complete_case_matches'],15)
        self.assertEqual(result['agreement'],15/16)

    def test_two_wrong_labels_in_one_case_count_as_one_failed_case(self):
        r=next(r for r in self.rows if r['id']=='number_format')
        r['judgment'].update(accurate=False,supported=False)
        result=calibration_summary(self.data,self.rows)
        self.assertEqual(result['complete_case_matches'],15)
        failed=next(c for c in result['cases'] if c['id']=='number_format')
        self.assertEqual(set(failed['mismatches']),{'semantic_accuracy','supported'})

    def test_14_complete_matches_do_not_pass(self):
        for r in self.rows:
            if r['id'] in {'number_format','date_format'}:r['judgment']['accurate']=False
        result=calibration_summary(self.data,self.rows)
        self.assertEqual(result['complete_case_matches'],14);self.assertFalse(result['passed'])

    def test_each_critical_error_blocks_even_with_15_complete_matches(self):
        checked=[]
        for c in self.data['cases']:
            for check in c['critical_checks']:
                with self.subTest(check['code']):
                    rows=deepcopy(self.rows);row=next(r for r in rows if r['id']==c['id'])
                    field='accurate' if check['field']=='semantic_accuracy' else 'supported'
                    row['judgment'][field]=check['forbidden_value']
                    result=calibration_summary(self.data,rows)
                    self.assertEqual(result['complete_case_matches'],15);self.assertFalse(result['passed'])
                    self.assertEqual([e['code'] for e in result['critical_errors']],[check['code']])
                    checked.append(c['id'])
        self.assertEqual(set(checked),{'unsupported_guess','lucky_guess','entity_swap','correct_abstention','unnecessary_abstention','invented_explanation'})

    def test_15_valid_results_without_last_result_do_not_pass(self):
        result=calibration_summary(self.data,self.rows[:-1])
        self.assertEqual(result['complete_case_matches'],15);self.assertFalse(result['passed'])
        self.assertEqual(result['expected_cases'],16);self.assertEqual(result['completed_cases'],15)

    def test_duplicate_and_unknown_case_ids_are_rejected(self):
        with self.assertRaises(ValueError):calibration_summary(self.data,self.rows+[self.rows[0]])
        rows=deepcopy(self.rows);rows[0]['id']='unknown'
        with self.assertRaises(ValueError):calibration_summary(self.data,rows)

    def test_abstention_null_is_required_and_not_a_no_hallucination_label(self):
        result=calibration_summary(self.data,self.rows)
        by_id={c['id']:c for c in result['cases']}
        self.assertTrue(by_id['correct_abstention']['abstention_correct'])
        self.assertFalse(by_id['unnecessary_abstention']['abstention_correct'])
        for cid in ['correct_abstention','unnecessary_abstention']:
            self.assertIsNone(by_id[cid]['judgment']['supported'])
        with self.assertRaises(ValueError):parse_judge({'accurate':True,'supported':False,'reason':'x'},True)
        with self.assertRaises(ValueError):parse_judge({'accurate':True,'supported':None,'reason':'x'},False)
        with self.assertRaises(ValueError):parse_judge({'accurate':True,'reason':'x'},True)

    def test_reader_retrieval_and_budget_unchanged(self):
        old=json.loads((ROOT/'evals/reports/phase3-protocol-review-v2/plan.json').read_text())
        protocol,models,_=load_configuration()
        self.assertEqual({k:v for k,v in protocol.items() if k not in {'version','calibration_dataset'}},{k:v for k,v in old['protocol'].items() if k!='version'})
        self.assertEqual(models,old['models'])
        self.assertEqual(digest(READER_SYSTEM),old['reader_prompt_sha256'])
        self.assertEqual(CALIBRATION.name,'judge_calibration_v3.json')


if __name__=='__main__':unittest.main()
