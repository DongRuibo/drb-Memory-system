"""Judge V4离线合同：准备输入、费用提案和回放；没有收费执行模式。"""
import argparse
from collections import Counter
from decimal import Decimal
import difflib
import hashlib
import json
from pathlib import Path

from .phase3 import judge_messages
from .protocol3 import JUDGE_SYSTEM,READER_SYSTEM,calibration_summary,digest,parse_judge,parse_prediction
from .runner import ROOT,write_json


CONTROLS=ROOT/'evals/datasets/judge_contract_v4_controls.json'
CALIBRATION=ROOT/'evals/datasets/judge_calibration_v3.json'
CONFIG=ROOT/'evals/configs/judge_contract_v4.json'
PREVIOUS=ROOT/'evals/reports/phase3-real-models-20261107'

# 保留V3其余规则，仅把混排的评分段替换为Support先行步骤；V3原常量不改。
_OLD="""Accuracy: require all reference facts without contradiction. Equivalent complete sentences,
unambiguous number/date conversions and harmless explanations are allowed. A similar-looking
wrong fact, a subset of required facts, or invented details are incorrect. For an unanswerable
question only an explicit abstention is correct. Evaluate support separately: every material
claim in a non-abstaining answer must follow from the supplied evidence, including derivable
arithmetic/time relations. Matching the reference by guessing is accurate but unsupported."""
_NEW="""Apply these steps in order, without carrying a failure from one label into the other:
1. Identify only the material claims actually asserted in a non-abstaining response. Bind a
short answer to the question's subject and time. Do not add omitted reference facts to this
claim set, and do not treat omission itself as a factual claim.
2. Decide supported FIRST, using those asserted claims and the supplied evidence only.
This includes valid derivable arithmetic/time relations.
reference_answers, unanswerable, required answer length and completeness are NOT evidence
for support. If every asserted material claim follows from evidence, supported=true even
when the answer is incomplete. If any asserted material claim lacks support or contradicts
evidence, supported=false. A bare answer to a current-state question asserts the current
state; an obsolete fact does not support it. Explicitly dated historical claims may be supported.
3. Decide accurate INDEPENDENTLY: require all reference facts without contradiction.
Equivalent complete sentences, unambiguous number/date conversions and harmless explanations
are allowed. Wrong facts, a subset of required facts, or a material false addition are incorrect.
For an unanswerable question only an explicit abstention is correct. Matching the reference
by guessing is accurate but unsupported. Include brief, separate Support and Accuracy bases
in reason; do not use incompleteness alone as the explanation for supported=false.
Examples of the label distinction (not an exhaustive list):
- Asked for two cities; evidence says Paris and Lyon; answer Paris: accurate=false, supported=true.
- Same evidence; answer Paris and Rome: accurate=false, supported=false.
- Reference matches answer X but evidence gives no support for X: accurate=true, supported=false.
- A correct core answer plus an important contradicted extra fact: accurate=false, supported=false.
- An unanswerable question with a correct explicit abstention: accurate=true, supported=null."""
assert JUDGE_SYSTEM.count(_OLD)==1
JUDGE_SYSTEM_V4=JUDGE_SYSTEM.replace(_OLD,_NEW,1).replace(
    '{"accurate": boolean, "supported": boolean or null, "reason": "brief explanation"}',
    '{"supported": boolean or null, "accurate": boolean, "reason": "Support: brief basis; Accuracy: brief basis"}',1)


def messages(inp,labels,prediction,context):
    value=judge_messages(inp,labels,prediction,context)
    value[0]['content']=JUDGE_SYSTEM_V4
    return value


def support_only_view(inp,prediction,context):
    # 仅供两阶段方案的信息流审查，不调用独立Support模型，也不预测语义标签。
    return {'question':inp['query'],'question_date':inp.get('question_date'),
            'response':{k:prediction[k] for k in ['answer','abstained','citations']},'evidence':context['text']}


def load_controls():
    value=json.loads(CONTROLS.read_text());cases=value['cases']
    if (len(cases)!=14 or len({c['id'] for c in cases})!=14
        or Counter(c['family'] for c in cases)!=dict.fromkeys(['incomplete_supported','incomplete_unsupported',
             'complete_supported','complete_partly_false','entity_binding','temporal_update','abstention'],2)):
        raise ValueError('controls must contain two cases in each of seven families')
    for c in cases:
        parse_prediction(c['prediction'],{'s'})
        parse_judge({'accurate':c['expected']['semantic_accuracy'],'supported':c['expected']['supported'],'reason':'annotation'},c['prediction']['abstained'])
        if bool(c['annotated_asserted_claims'])==c['prediction']['abstained']:
            raise ValueError('claim annotations must be present only for non-abstentions')
    return value


def previous_cost():
    path=PREVIOUS/'calls.jsonl';events=[json.loads(s) for s in path.read_text().splitlines()]
    reserved_rows=[e for e in events if e['event']=='reserved'];settled_rows=[e for e in events if e['event']=='settled']
    reservations={e['request']:e for e in reserved_rows};settled={e['request']:e for e in settled_rows}
    if (len(reserved_rows)!=len(reservations) or len(settled_rows)!=len(settled)
        or len(reservations)!=18 or len(settled)!=18 or reservations.keys()!=settled.keys()):
        raise ValueError('previous experiment must be reconciled, with no unsettled reservations')
    cost=sum((Decimal(e['usage_list_price_cost_cny']) for e in settled.values()),Decimal(0))
    return cost,hashlib.sha256(path.read_bytes()).hexdigest()


def build_contract():
    previous,receipt_sha=previous_cost();old=json.loads((PREVIOUS/'approved-plan.json').read_text())
    # 校准/对照均为短输入，收紧到8K；prepare必须验证全输入字节保守界，不允许截断。
    rows=[{'role':role,'requests':n,'input_cap':cap,'visible_output_cap':out,'billable_output_cap':out+128}
          for role,n,cap,out in [('smoke',2,16384,768),('calibration',16,8192,512),('controls',14,8192,512),
                                 ('reader',56,16384,768),('judge',56,24576,512)]]
    prices=old['price_evidence'];inp=Decimal(prices['input_per_million']);out=Decimal(prices['output_per_million'])
    for row in rows:row['cost_ceiling_cny']=str(row['requests']*(row['input_cap']*inp+row['billable_output_cap']*out)/1000000)
    total=sum(Decimal(r['cost_ceiling_cny']) for r in rows)
    if previous+total>12:raise ValueError('cumulative ceiling exceeds 12 CNY; shrink plan, do not raise budget')
    trial='judge-contract-v4-trial-01'
    return {'version':'judge-evaluation-contract-v4','mode':'offline_preparation_only','authorized':False,
            'judge_model':{**old['models']['reader'],'max_tokens':512},'reader_model':old['models']['reader'],
            'output_schema':'accurate:boolean,supported:boolean|null,reason:string',
            'judge_prompt_sha256':digest(JUDGE_SYSTEM_V4),'v3_judge_prompt_sha256':digest(JUDGE_SYSTEM),'reader_prompt_sha256':digest(READER_SYSTEM),
            'calibration':{'path':str(CALIBRATION.relative_to(ROOT)),'sha256':hashlib.sha256(CALIBRATION.read_bytes()).hexdigest(),
                           'gate':json.loads(CALIBRATION.read_text())['gate']},
            'controls':{'path':str(CONTROLS.relative_to(ROOT)),'sha256':hashlib.sha256(CONTROLS.read_bytes()).hexdigest(),
                        'gate':load_controls()['gate'],'human_review_required':True,'additional_gate_is_proposed_not_authorized':True},
            'formal_protocol_preserved':{k:old['protocol'][k] for k in ['seed','candidate_budget','output_chunks','rrf','reader_methods','retrieval_methods','context']},
            'formal_lock_sha256':old['lock_sha256'],'trial_id':trial,'previous_budget_key':old['budget_key'],
            'proposed_budget_key':digest({'trial':trial,'previous_budget_key':old['budget_key']}),
            'proposed_stage_claim_keys':{stage:digest({'trial':trial,'stage':stage,'previous_budget_key':old['budget_key']})
                                         for stage in ['calibration_and_controls','formal']},
            'previous_receipt_sha256':receipt_sha,'previous_cost_cny':str(previous),'previous_cost_basis':'settled usage at public list prices; invoice not reconciled',
            'prices_cny_per_million':{'input':str(inp),'output':str(out),'cache_discount_assumed':False,'pricing_url':prices['pricing_url']},
            'global_cumulative_cap_cny':'12.00','proposed_new_round_hard_cap_cny':str(total),
            'cumulative_worst_cost_cny':str(previous+total),'role_budgets':rows,'proposed_max_requests':sum(r['requests'] for r in rows),
            'proposed_calibration_stage_requests':32,'proposed_calibration_stage_ceiling_cny':str(sum(Decimal(r['cost_ceiling_cny']) for r in rows[:3])),
            'formal_stage_requests':112,'formal_stage_ceiling_cny':str(sum(Decimal(r['cost_ceiling_cny']) for r in rows[3:])),
            'max_input_tokens':sum(r['requests']*r['input_cap'] for r in rows),'max_billable_output_tokens':sum(r['requests']*r['billable_output_cap'] for r in rows),
            'timeout_seconds':45,'retries':0,'restart_policy':'stable one-shot stage claims; no automatic resume or new output-directory budget',
            'stage_authorization_policy':'calibration/controls and formal require explicit scope; a 32-call grant never authorizes the 112-call formal stage',
            'cross_round_policy':'verify previous receipt hash and settled cost before reserving; cumulative previous+new spent/reserved <=12; preserve old claim',
            'runtime_sha256':{str(p.relative_to(ROOT)):hashlib.sha256(p.read_bytes()).hexdigest() for p in
                              [Path(__file__),ROOT/'evals/memory_eval/models3.py',ROOT/'evals/memory_eval/protocol3.py',ROOT/'evals/memory_eval/phase3.py']},
            'execution_gates':['new explicit authorization; previous unused allowance does not authorize V4',
                               'recheck real-time prices and model availability','16 calibration results, >=15 full matches, zero critical errors',
                               'proposed additional control gate: 14 full matches, zero critical errors; requires user confirmation',
                               'formal execution requires a separately sealed integration with this V4 adapter and existing budget client']}


def evaluate(dataset,rows):
    cases={c['id']:c for c in dataset['cases']}
    for row in rows:
        if row['id'] not in cases:raise ValueError('unknown case ID')
        parse_judge(row['judgment'],cases[row['id']]['prediction']['abstained'])
    return calibration_summary(dataset,rows)


def prepare(output):
    expected=build_contract();configured=json.loads(CONFIG.read_text())
    if configured!=expected:raise ValueError('V4 protocol/hash differs; review and reseal before use')
    output=Path(output);output.mkdir(parents=True,exist_ok=False)
    controls=load_controls();calibration=json.loads(CALIBRATION.read_text());inputs={}
    for name,data in [('calibration',calibration),('controls',controls)]:
        rows=[]
        for c in data['cases']:
            inp={'query':c['question'],'question_date':c.get('question_date')};ctx={'text':'\nSOURCE s\n'+c['context']}
            value=messages(inp,c,c['prediction'],ctx)
            bound=sum(len(m['content'].encode())+128 for m in value)+128
            if bound>8192:raise ValueError('full short-input bound exceeds 8192; no silent truncation')
            rows.append({'id':c['id'],'messages':value,'input_sha256':digest(value),'input_byte_token_bound':bound,
                         'support_only_view_for_design_review':support_only_view(inp,c['prediction'],ctx)})
        inputs[name]=rows;write_json(output/(name+'-inputs.json'),rows)
    write_json(output/'protocol.json',expected)
    (output/'prompt-v3.txt').write_text(JUDGE_SYSTEM+'\n');(output/'prompt-v4.txt').write_text(JUDGE_SYSTEM_V4+'\n')
    (output/'prompt.diff').write_text(''.join(difflib.unified_diff(JUDGE_SYSTEM.splitlines(True),JUDGE_SYSTEM_V4.splitlines(True),fromfile='Judge V3',tofile='Judge V4')))
    old_rows=json.loads((PREVIOUS/'judge-calibration.json').read_text())
    replay=evaluate(calibration,[{'id':r['id'],'judgment':r['judgment']} for r in old_rows])
    write_json(output/'v3-real-output-gate-replay.json',replay)
    manifest={'status':'offline_prepared_waiting_for_confirmation','contract_sha256':digest(expected),
              'config_file_sha256':hashlib.sha256(CONFIG.read_bytes()).hexdigest(),'paid_calls_this_round':0,'v4_real_model_results':None,
              'calibration_cases':16,'control_cases':14,'max_short_input_bound':max(r['input_byte_token_bound'] for rows in inputs.values() for r in rows),
              'old_failure_still_blocked':not replay['passed'],'new_control_annotations_human_review':'pending'}
    write_json(output/'manifest.json',manifest)
    return manifest


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    sub=parser.add_subparsers(dest='mode',required=True)
    prep=sub.add_parser('prepare');prep.add_argument('--output',required=True)
    replay=sub.add_parser('evaluate');replay.add_argument('--dataset',required=True);replay.add_argument('--responses',required=True);replay.add_argument('--output',required=True)
    args=parser.parse_args()
    if args.mode=='prepare':print(json.dumps(prepare(args.output),ensure_ascii=False))
    else:
        value=evaluate(json.loads(Path(args.dataset).read_text()),json.loads(Path(args.responses).read_text()))
        with Path(args.output).open('x') as file:json.dump(value,file,ensure_ascii=False,indent=2)


if __name__=='__main__':main()
