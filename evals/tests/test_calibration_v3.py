"""v3小范围修正：独立模拟指定关键错误，绝不调用模型API。"""
from copy import deepcopy
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from evals.memory_eval import phase3
from evals.memory_eval.protocol3 import JUDGE_SYSTEM,READER_SYSTEM,answer_scores,calibration_summary,digest,parse_judge


ROOT=Path(__file__).resolve().parents[2]


class CalibrationV3Tests(unittest.TestCase):
    def setUp(self):
        self.data=phase3.load_calibration()
        self.rows=[{'id':c['id'],'judgment':{'accurate':c['expected']['semantic_accuracy'],
                   'supported':c['expected']['supported'],'reason':'离线模拟Judge输出'}} for c in self.data['cases']]

    def test_v1_v2_and_locked_14_hashes_preserved(self):
        expected={'evals/datasets/judge_calibration_v1.json':'93ab2e268dfa40a86a4fc38dee1f2c84789b34bed51b14b61bce598e97d560a1',
                  'evals/datasets/judge_calibration_v2.json':'e6ba50bd599e149c2258c7aeb5055ab60607e6b598c1d71f23959d237aa03e0d',
                  'evals/datasets/longmemeval_phase3_lock.json':'7c50ca5ed56dc660773e32352c163a9b7edd6d243bed35cd4855510a82c9d417'}
        for path,sha in expected.items():self.assertEqual(hashlib.sha256((ROOT/path).read_bytes()).hexdigest(),sha)
        self.assertEqual(self.data['derived_from'],{'path':'evals/datasets/judge_calibration_v2.json','sha256':expected['evals/datasets/judge_calibration_v2.json']})

    def test_only_positive_preference_fact_changes_and_references_labels_unchanged(self):
        v2=json.loads((ROOT/'evals/datasets/judge_calibration_v2.json').read_text())
        changed=[]
        for old,new,record in zip(v2['cases'],self.data['cases'],self.data['changes']):
            self.assertEqual(old['id'],new['id'])
            for key in ['question','answers','prediction','expected','question_type','unanswerable']:
                self.assertEqual(old[key],new[key])
            if old['context']!=new['context']:changed.append(new['id'])
            expected_diff={k:{'before':old[k],'after':new[k]} for k in ['context','critical_checks'] if old[k]!=new[k]}
            self.assertEqual(record['changed_fields'],expected_diff)
        self.assertEqual(changed,['preference_rubric_positive'])
        positive=next(c for c in self.data['cases'] if c['id']=='preference_rubric_positive')
        self.assertEqual(positive['context'],'Ada enjoys chess, prefers quiet indoor activities, and wants to avoid traffic noise and crowds.')
        self.assertIs(positive['expected']['semantic_accuracy'],True);self.assertIs(positive['expected']['supported'],True)

    def test_all_requested_critical_errors_block_15_of_16(self):
        # 模拟输出按用户指定方向独立列举，不从数据中的门禁定义生成，以发现漏配或配反。
        errors=[('decimal_error','accurate',True),('decimal_error','supported',True),
                ('unsupported_guess','accurate',True),('unsupported_guess','supported',True),
                ('lucky_guess','accurate',False),('lucky_guess','supported',True),
                ('entity_swap','accurate',True),('entity_swap','supported',True),
                ('partial_answer','accurate',True),('partial_answer','supported',False),
                ('invented_explanation','accurate',True),('invented_explanation','supported',True),
                ('correct_abstention','accurate',False),('correct_abstention','supported',True),('correct_abstention','supported',False),
                ('unnecessary_abstention','accurate',True),('unnecessary_abstention','supported',True),('unnecessary_abstention','supported',False),
                ('preference_rubric_negative','accurate',True),('preference_rubric_negative','supported',True)]
        for cid,field,value in errors:
            with self.subTest(case_id=cid,field=field,value=value):
                rows=deepcopy(self.rows);row=next(r for r in rows if r['id']==cid);row['judgment'][field]=value
                summary=calibration_summary(self.data,rows)
                self.assertEqual(summary['complete_case_matches'],15);self.assertFalse(summary['passed'])
                self.assertEqual(len(summary['critical_errors']),1);self.assertEqual(summary['critical_errors'][0]['case_id'],cid)
                if cid in {'correct_abstention','unnecessary_abstention'} and field=='supported':
                    with self.assertRaises(ValueError):parse_judge(row['judgment'],True)
        self.assertEqual(len(errors),20)

    def test_complete_case_threshold_keeps_one_vote_per_case(self):
        self.assertTrue(calibration_summary(self.data,self.rows)['passed'])
        row=next(r for r in self.rows if r['id']=='number_format');row['judgment'].update(accurate=False,supported=False)
        summary=calibration_summary(self.data,self.rows)
        self.assertTrue(summary['passed']);self.assertEqual(summary['complete_case_matches'],15);self.assertEqual(summary['agreement'],15/16)
        next(r for r in self.rows if r['id']=='date_format')['judgment']['accurate']=False
        self.assertFalse(calibration_summary(self.data,self.rows)['passed'])

    def test_missing_duplicate_unknown_results_never_pass(self):
        for rows in [[],self.rows[:-1]]:
            summary=calibration_summary(self.data,rows)
            self.assertFalse(summary['passed']);self.assertEqual(summary['expected_cases'],16)
        with self.assertRaises(ValueError):calibration_summary(self.data,self.rows+[self.rows[0]])
        rows=deepcopy(self.rows);rows[0]['id']='unknown'
        with self.assertRaises(ValueError):calibration_summary(self.data,rows)

    def test_refusals_require_explicit_null_at_entry(self):
        for accurate in [True,False]:
            parse_judge({'accurate':accurate,'supported':None,'reason':'fixture'},True)
            for supported in [True,False,'null',0]:
                with self.assertRaises(ValueError):parse_judge({'accurate':accurate,'supported':supported,'reason':'fixture'},True)
            with self.assertRaises(ValueError):parse_judge({'accurate':accurate,'reason':'fixture'},True)

    def test_config_pins_v3_hash_and_rejects_wrong_hash(self):
        protocol,_,_=phase3.load_configuration();record=protocol['calibration_dataset']
        self.assertEqual(record,{'path':'evals/datasets/judge_calibration_v3.json','version':'judge-calibration-v3',
                                'sha256':hashlib.sha256(phase3.CALIBRATION.read_bytes()).hexdigest()})
        with tempfile.TemporaryDirectory() as temp:
            config=Path(temp);protocol['calibration_dataset']['sha256']='0'*64
            (config/'phase3_protocol.json').write_text(json.dumps(protocol))
            with patch.object(phase3,'CONFIG',config):
                with self.assertRaises(ValueError):phase3.load_calibration()

    def test_prompts_retrieval_models_and_budget_identity_unchanged(self):
        old=json.loads((ROOT/'evals/reports/phase3-calibration-v2/plan.json').read_text())
        protocol,models,lock=phase3.load_configuration()
        self.assertEqual({k:v for k,v in protocol.items() if k not in {'version','calibration_dataset'}},
                         {k:v for k,v in old['protocol'].items() if k!='version'})
        self.assertEqual(models,old['models']);self.assertEqual(phase3.budget_key(lock,models),old['budget_key'])
        self.assertEqual(digest(READER_SYSTEM),old['reader_prompt_sha256']);self.assertEqual(digest(JUDGE_SYSTEM),old['judge_prompt_sha256'])

    def test_strict_normalized_contracts_unchanged(self):
        for c in self.data['cases']:
            result=answer_scores(c['prediction'],c['answers'],c['unanswerable'])
            for field in ['strict_match','normalized_match']:self.assertEqual(result[field],c['expected'][field])
            self.assertIsNone(result['semantic_accuracy'])


if __name__=='__main__':unittest.main()
