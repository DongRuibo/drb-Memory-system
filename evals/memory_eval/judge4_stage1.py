"""V4第一阶段：封存及受授权的32次校准执行；不含正式Reader/检索入口。"""
import argparse
from datetime import datetime,timezone
from decimal import Decimal
import hashlib
import json
import os
from pathlib import Path

from . import judge4
from .models3 import BudgetStop,ChatClient,Ledger
from .phase3 import claim_execution
from .protocol3 import digest,parse_judge
from .runner import ROOT,write_json


CONFIG=ROOT/'evals/configs/judge_v4_stage1.json'
CLAIMS=ROOT/'evals/runs/paid-budget'
STAGE='judge-v4-stage1-calibration-and-controls'


def build_protocol():
    parent=json.loads(judge4.CONFIG.read_text());previous,receipt_sha=judge4.previous_cost()
    calibration=json.loads(judge4.CALIBRATION.read_text());controls=judge4.load_controls()
    if (digest(judge4.JUDGE_SYSTEM_V4)!=parent['judge_prompt_sha256']
        or hashlib.sha256(judge4.CALIBRATION.read_bytes()).hexdigest()!=parent['calibration']['sha256']
        or hashlib.sha256(judge4.CONTROLS.read_bytes()).hexdigest()!=parent['controls']['sha256']
        or receipt_sha!=parent['previous_receipt_sha256'] or str(previous)!=parent['previous_cost_cny']):
        raise BudgetStop('approved prompt, datasets or previous receipt changed')
    roles={r['role']:r for r in parent['role_budgets']};rows=[roles[k] for k in ['smoke','calibration','controls']]
    counts={r['role']:r['requests'] for r in rows};cost=sum(Decimal(r['cost_ceiling_cny']) for r in rows)
    if counts!={'smoke':2,'calibration':16,'controls':14} or cost!=Decimal('1.240064') or previous+cost>12:
        raise BudgetStop('stage scope or cumulative budget differs from reviewed limits')
    limits={'max_requests':32,'role_request_limits':counts,'max_cost_cny':str(cost),
            'max_input_tokens_total':sum(r['requests']*r['input_cap'] for r in rows),
            'max_output_tokens_total':sum(r['requests']*r['billable_output_cap'] for r in rows),
            'reader_input_cap':16384,'reader_output_cap':768,'judge_input_cap':8192,'judge_output_cap':512,
            'reasoning_reserve':128,'timeout_seconds':45,'retries':0,
            'price_cny_per_million_input':parent['prices_cny_per_million']['input'],
            'price_cny_per_million_output':parent['prices_cny_per_million']['output']}
    source_paths=[Path(__file__),ROOT/'evals/memory_eval/models3.py',ROOT/'evals/memory_eval/judge4.py',
                  ROOT/'evals/memory_eval/protocol3.py',ROOT/'evals/memory_eval/phase3.py',ROOT/'evals/memory_eval/runner.py']
    return {'version':'judge-v4-stage1-executor-v1','stage_id':STAGE,'paid_execution_authorized':False,
            'scope':'calibration_and_controls_only','parent_contract_sha256':hashlib.sha256(judge4.CONFIG.read_bytes()).hexdigest(),
            'budget_key':parent['proposed_stage_claim_keys']['calibration_and_controls'],'model':parent['judge_model'],
            'judge_prompt_sha256':parent['judge_prompt_sha256'],'calibration':parent['calibration'],
            'controls':{'path':parent['controls']['path'],'sha256':parent['controls']['sha256'],'gate':controls['gate']},
            'role_budgets':rows,'limits':limits,'global_cap_cny':'12.00','previous_cost_cny':str(previous),
            'previous_receipt_sha256':receipt_sha,'previous_budget_key':parent['previous_budget_key'],
            'previous_plan_seal':json.loads((judge4.PREVIOUS/'approved-plan.json').read_text())['seal'],
            'cumulative_stage1_worst_cny':str(previous+cost),
            'formal_stage':{'authorized':False,'requests':112,'separate_plan_required':True,
                            'separate_budget_key':parent['proposed_stage_claim_keys']['formal'],'ceiling_cny':parent['formal_stage_ceiling_cny']},
            'joint_judge_sees_reference_and_evidence':True,'support_only_view_used_for_calls':False,
            'gate_order':['calibration','controls'],'stop_when_gate_cannot_pass':True,
            'failure_reservation_policy':'transport, output, schema or usage failure retains maximum reservation; valid semantic disagreement is settled then stops gate',
            'restart_policy':'one canonical stage claim; no auto-resume, no output-directory reset; reconcile before any new authorization',
            'runtime_sha256':{str(p.relative_to(ROOT)):hashlib.sha256(p.read_bytes()).hexdigest() for p in source_paths}}


def input_plan(protocol):
    records=[];smoke=[{'role':'system','content':'Return a JSON object only.'},{'role':'user','content':'Return {"ok": true}.'}]
    for i in range(2):records.append({'role':'smoke','id':'smoke-'+str(i+1),'messages':smoke,'abstained':None})
    for role,path in [('calibration',judge4.CALIBRATION),('controls',judge4.CONTROLS)]:
        data=json.loads(path.read_text())
        for c in data['cases']:
            value=judge4.messages({'query':c['question'],'question_date':c.get('question_date')},c,c['prediction'],{'text':'\nSOURCE s\n'+c['context']})
            records.append({'role':role,'id':c['id'],'messages':value,'abstained':c['prediction']['abstained']})
    for r in records:
        smoke_role=r['role']=='smoke';limits=protocol['limits']
        r['input_byte_token_bound']=sum(len(m['content'].encode())+128 for m in r['messages'])+128
        if r['input_byte_token_bound']>(limits['reader_input_cap'] if smoke_role else limits['judge_input_cap']):
            raise BudgetStop('full request exceeds reviewed input limit; no truncation')
        r['payload']={'model':protocol['model']['model'],'messages':r['messages'],'temperature':0,'stream':False,
                      'max_tokens':768 if smoke_role else 512,'enable_thinking':False,'thinking_budget':128,
                      'response_format':{'type':'json_object'}}
        r['input_sha256']=digest(r['messages'])
    return records


def make_plan():
    protocol=build_protocol()
    if json.loads(CONFIG.read_text())!=protocol:raise BudgetStop('stage config/runtime changed; reseal required')
    records=input_plan(protocol)
    plan={'kind':'stage1_execution_plan_not_authorization','stage':protocol,'inputs':records,
          'config_sha256':hashlib.sha256(CONFIG.read_bytes()).hexdigest(),'budget_key':protocol['budget_key']}
    plan['seal']=digest(plan)
    return plan


def prepare(output):
    plan=make_plan();out=Path(output);out.mkdir(parents=True,exist_ok=False)
    write_json(out/'plan.json',plan);write_json(out/'protocol.json',plan['stage'])
    for role in ['smoke','calibration','controls']:write_json(out/(role+'-inputs.json'),[r for r in plan['inputs'] if r['role']==role])
    (out/'prompt-v4.txt').write_text(judge4.JUDGE_SYSTEM_V4+'\n')
    write_json(out/'manifest.json',{'status':'prepared_waiting_for_32_call_authorization','plan_seal':plan['seal'],
                                   'config_sha256':plan['config_sha256'],'paid_calls':0,'claim_created':False,
                                   'api_key_read':False,'formal_authorized':False})
    return plan


def check_approval(path,plan):
    approval=json.loads(Path(path).read_text());stage=plan['stage']
    if (plan!=make_plan() or approval.get('approved') is not True or approval.get('plan_seal')!=plan['seal']
        or approval.get('stage_id')!=STAGE or approval.get('scope')!='calibration_and_controls_only'
        or approval.get('calibration_labels_human_reviewed') is not True or approval.get('control_labels_human_reviewed') is not True
        or approval.get('formal_authorized') is not False or approval.get('max_requests')!=32
        or approval.get('role_request_limits')!=stage['limits']['role_request_limits']
        or approval.get('max_cost_cny')!=stage['limits']['max_cost_cny'] or approval.get('global_cap_cny')!='12.00'
        or approval.get('prices_rechecked') is not True or not isinstance(approval.get('reviewer'),str) or not approval['reviewer'].strip()):
        raise BudgetStop('new stage1 authorization/plan/labels/prices required; no key read or claim created')
    return approval


def verify_previous(stage,claim_root):
    path=Path(claim_root)/(stage['previous_budget_key']+'.json')
    if not path.is_file():raise BudgetStop('previous canonical claim missing; reconcile, do not reset')
    previous=json.loads(path.read_text());ledger=Path(previous['ledger_path'])
    if (previous.get('budget_key')!=stage['previous_budget_key'] or previous.get('plan_seal')!=stage['previous_plan_seal']
        or not ledger.is_file() or hashlib.sha256(ledger.read_bytes()).hexdigest()!=stage['previous_receipt_sha256']):
        raise BudgetStop('previous canonical ledger differs or is unsettled; stop for reconciliation')
    if Decimal(stage['previous_cost_cny'])+Decimal(stage['limits']['max_cost_cny'])>Decimal(stage['global_cap_cny']):
        raise BudgetStop('cumulative budget exceeds cap')


def _smoke_schema(value):
    if not isinstance(value,dict) or value.get('ok') is not True:raise ValueError('availability JSON contract failed')
    return value


def run_stage(plan_path,approval_path,output,*,transport=None,claim_root=None,previous_claim_root=None):
    # 测试注入仅用于隔离模拟；真实CLI不能换claim目录或运输层来重开预算。
    simulated=transport is not None
    if simulated:
        if claim_root is None or previous_claim_root is None or Path(claim_root).resolve()==CLAIMS.resolve():
            raise BudgetStop('simulation requires isolated claim directories')
    elif claim_root is not None or previous_claim_root is not None:
        raise BudgetStop('real execution cannot override canonical claim directories')
    plan=json.loads(Path(plan_path).read_text());approval=check_approval(approval_path,plan);stage=plan['stage']
    verify_previous(stage,previous_claim_root if simulated else CLAIMS)
    claims=Path(claim_root) if simulated else CLAIMS
    if (claims/(stage['formal_stage']['separate_budget_key']+'.json')).exists():
        raise BudgetStop('another V4 stage already claimed; reconcile cumulative budget first')
    out=Path(output)
    if out.exists():raise BudgetStop('output already exists; no overwrite or automatic resume')
    if not simulated and not os.environ.get('MEMORY_EVAL_API_KEY'):
        raise BudgetStop('MEMORY_EVAL_API_KEY required after authorization; no claim created')
    claim=claim_execution(plan,out,budget_root=claims)
    out.mkdir(parents=True);write_json(out/'plan.json',plan)
    write_json(out/'approval-record.json',{k:approval[k] for k in ['stage_id','plan_seal','scope','reviewer','max_requests','max_cost_cny','global_cap_cny']})
    ledger=Ledger(out/'calls.jsonl',stage['limits'])
    client=ChatClient(stage['model'],stage['limits'],ledger,approved=True,transport=transport)
    datasets={'calibration':json.loads(judge4.CALIBRATION.read_text()),'controls':judge4.load_controls()}
    cohort_rows={k:[] for k in datasets};attempted=[];current=None
    summary={'status':'running','mode':'simulation' if simulated else 'real','stage_id':STAGE,'formal_authorized':False,
             'formal_calls':0,'gates':{k:{'status':'not_run','result':None} for k in datasets},'quality_metrics':None}
    try:
        for record in plan['inputs']:
            current=record;request=ledger.requests+1;attempted.append((request,record))
            folder=out/'traces'/record['role'];folder.mkdir(parents=True,exist_ok=True)
            write_json(folder/(record['id']+'.json'),{'request':request,'case_id':record['id'],'role':record['role'],
                       'status':'prepared','payload':record['payload'],'input_sha256':record['input_sha256']})
            validator=_smoke_schema if record['role']=='smoke' else lambda value:parse_judge(value,record['abstained'])
            result=client.call(record['messages'],record['role'],validator=validator)
            if record['role']!='smoke':
                role=record['role'];cohort_rows[role].append({'id':record['id'],'judgment':result['json'],'request':result['request']})
                gate=judge4.evaluate(datasets[role],cohort_rows[role]);summary['gates'][role]={'status':'running','result':gate}
                write_json(out/(role+'-results.json'),cohort_rows[role]);write_json(out/(role+'-summary.json'),gate)
                possible=gate['complete_case_matches']+gate['expected_cases']-gate['completed_cases']
                if gate['critical_errors'] or possible<gate['minimum_complete_case_matches']:
                    raise BudgetStop(role+' gate cannot pass; stop immediately')
                if gate['completed_cases']==gate['expected_cases']:
                    if not gate['passed']:raise BudgetStop(role+' gate failed')
                    summary['gates'][role]['status']='passed'
            write_json(out/'summary.json',summary)
        summary['status']='completed_stage1'
    except Exception as error:
        summary.update(status='stopped',error_type=type(error).__name__,failed_role=current['role'] if current else None,
                       failed_case_id=current['id'] if current else None)
        if current and current['role'] in summary['gates']:summary['gates'][current['role']]['status']='stopped'
        write_json(out/'failure.json',{'error_type':type(error).__name__,'role':summary.get('failed_role'),
                                     'case_id':summary.get('failed_case_id'),'last_request':ledger.requests,'retry':0})
    finally:
        # 账本是崩溃后的权威记录；在正常/失败退出时按请求号导出完整Trace。
        ledger.file.close();events=[json.loads(s) for s in (out/'calls.jsonl').read_text().splitlines()]
        outputs={e['request']:e for e in events if e['event']=='model_output'}
        settled={e['request']:e for e in events if e['event']=='settled'}
        reserved={e['request']:e for e in events if e['event']=='reserved'}
        for request,record in attempted:
            wire=outputs.get(request);receipt=settled.get(request)
            trace={'request':request,'case_id':record['id'],'role':record['role'],'payload':record['payload'],
                   'input_sha256':record['input_sha256'],'status':'settled' if receipt else 'stopped',
                   'raw_response':wire.get('raw_response') if wire else None,'response_id':wire.get('response_id') if wire else None,
                   'usage':wire.get('usage') if wire else None,'latency_ms':wire.get('latency_ms') if wire else None,
                   'settlement':receipt,'reservation':reserved.get(request)}
            write_json(out/'traces'/record['role']/(record['id']+'.json'),trace)
        summary.update(requests_reserved=ledger.requests,api_dispatch_attempts=ledger.dispatches,
                       role_counts=ledger.role_counts,estimated_or_reserved_cost_cny=str(ledger.cost),
                       previous_cost_cny=stage['previous_cost_cny'],cumulative_spent_or_reserved_cny=str(Decimal(stage['previous_cost_cny'])+ledger.cost),
                       unsettled_reservations=len(reserved)-len(settled),budget_claim=str(claim))
        write_json(out/'summary.json',summary)
        if summary['status']=='completed_stage1':
            write_json(out/'handoff-receipt.json',{'kind':'stage1_evidence_not_formal_authorization','stage1_plan_seal':plan['seal'],
                       'stage1_budget_key':stage['budget_key'],'ledger_sha256':hashlib.sha256((out/'calls.jsonl').read_bytes()).hexdigest(),
                       'stage1_cost_cny':str(ledger.cost),'previous_cost_cny':stage['previous_cost_cny'],
                       'cumulative_cost_cny':summary['cumulative_spent_or_reserved_cny'],'gates_passed':['calibration','controls'],
                       'formal_authorized':False,'formal_requests_allowed':0,'formal_separate_budget_key':stage['formal_stage']['separate_budget_key']})
        (out/'report.md').write_text('# V4第一阶段执行\n\n模式：'+summary['mode']+'；状态：'+summary['status']+'。两个数据集独立汇总，见calibration-summary.json和controls-summary.json（未运行时见summary.json）。\n\n正式Reader/Judge调用0，交接凭据不构成112次授权。\n')
    return summary


def main():
    parser=argparse.ArgumentParser(description=__doc__);sub=parser.add_subparsers(dest='mode',required=True)
    prep=sub.add_parser('prepare');prep.add_argument('--output',required=True)
    run=sub.add_parser('run');run.add_argument('--plan',required=True);run.add_argument('--approval',required=True);run.add_argument('--output',required=True)
    args=parser.parse_args()
    if args.mode=='prepare':print(json.dumps({'plan_seal':prepare(args.output)['seal'],'paid_calls':0},ensure_ascii=False))
    else:
        result=run_stage(args.plan,args.approval,args.output)
        print(json.dumps(result,ensure_ascii=False))
        if result['status']!='completed_stage1':raise SystemExit(1)


if __name__=='__main__':main()
