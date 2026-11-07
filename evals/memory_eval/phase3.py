"""第三阶段 CLI：默认只准备；收费执行须绑定审核后的计划和一次性账本。"""
import argparse
from collections import Counter
from decimal import Decimal
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time
import urllib.request

from .baselines import TfidfIndex
from .locked3 import load_cases,source_rows
from .models3 import BudgetStop,ChatClient,DenseIndex,Ledger,SemanticClient,TextTokenizer,build_context
from .protocol3 import JUDGE_SYSTEM,READER_SYSTEM,answer_scores,calibration_summary,digest,parse_judge,parse_prediction,stage_scores,summarize
from .runner import ROOT,TARGET,Sandbox,command,corpus,retrieve,write_json
from .metrics import mean,percentile

CONFIG=ROOT/'evals/configs'
LOCK=ROOT/'evals/datasets/longmemeval_phase3_lock.json'
CALIBRATION=ROOT/'evals/datasets/judge_calibration_v3.json'


def runtime_hashes():
    paths=[ROOT/'evals/memory_eval'/name for name in ['phase3.py','models3.py','protocol3.py','locked3.py','runner.py','baselines.py','metrics.py','longmemeval.py']]
    paths += [ROOT/'evals/harness/src/main.rs',ROOT/'crates/memory-postgres/src/repository.rs',ROOT/'evals/harness/Cargo.lock',ROOT/'tools/retrieval_models_server.py',ROOT/'evals/requirements-phase3-models.lock']
    paths += list((ROOT/'crates/memory-application/src').glob('*.rs'))
    return {str(p.relative_to(ROOT)):hashlib.sha256(p.read_bytes()).hexdigest() for p in paths}


def assets(args,out):
    _,models,_=load_configuration()
    for key in ['embedding','reranker','reader_tokenizer']:
        entry=models[key];folder=out/entry['model'].split('/')[-1];folder.mkdir()
        for filename,record in entry['files'].items():
            with urllib.request.urlopen(record['url'],timeout=45) as response:raw=response.read(32000001)
            if len(raw)!=record['bytes'] or hashlib.sha256(raw).hexdigest()!=record['sha256']:raise ValueError('public tokenizer artifact changed')
            (folder/filename).write_bytes(raw)
        write_json(folder/'manifest.json',entry)


def load_configuration():
    protocol=json.loads((CONFIG/'phase3_protocol.json').read_text());models=json.loads((CONFIG/'phase3_models.json').read_text())
    prices=json.loads((CONFIG/'phase3_prices.json').read_text());lock=json.loads(LOCK.read_text())
    if (not Decimal(0)<Decimal(protocol['max_cost_cny'])<=12 or protocol['retries']!=0 or protocol['max_requests']>130
        or not 0<protocol['timeout_seconds']<=45 or protocol['reader_input_cap']>16384 or protocol['judge_input_cap']>24576
        or protocol['reader_output_cap']>768 or protocol['judge_output_cap']>512 or protocol['reasoning_reserve']!=128):
        raise ValueError('protocol exceeds reviewed offline limits')
    expected={'reader':len(lock['cases'])*len(protocol['reader_methods']),
              'judge':len(lock['cases'])*len(protocol['reader_methods']),
              'calibration':protocol['calibration_cases'],'smoke':protocol['smoke_requests']}
    if (protocol['role_request_limits']!=expected or sum(expected.values())!=protocol['max_requests']
        or models['reader']['temperature']!=0 or models['reader']['enable_thinking'] is not False
        or models['reader']['thinking_budget']!=protocol['reasoning_reserve'] or models['reader']['max_tokens']!=protocol['reader_output_cap']
        or prices['model']!=models['reader']['model'] or prices['currency']!='CNY'
        or Decimal(prices['input_per_million'])!=Decimal(protocol['price_cny_per_million_input'])
        or Decimal(prices['output_per_million'])!=Decimal(protocol['price_cny_per_million_output'])):
        raise ValueError('role counts or price evidence differ from execution budget')
    for key in ['price_cny_per_million_input','price_cny_per_million_output']:
        if not Decimal(protocol[key]).is_finite() or Decimal(protocol[key])<=0:raise ValueError('invalid model price')
    return protocol,models,lock


def load_calibration():
    raw=CALIBRATION.read_bytes();value=json.loads(raw);cases=value['cases'];gate=value['gate']
    record=json.loads((CONFIG/'phase3_protocol.json').read_text())['calibration_dataset']
    # 校准集在入口核对一次；人工最终确认和收费授权仍由独立批准记录绑定封印。
    if (record!={'path':str(CALIBRATION.relative_to(ROOT)),'version':'judge-calibration-v3','sha256':hashlib.sha256(raw).hexdigest()}
        or value['version']!=record['version'] or len(cases)!=16 or len({c['id'] for c in cases})!=16
        or gate!={'expected_cases':16,'minimum_complete_case_matches':15,'match_fields':['semantic_accuracy','supported'],
                 'null_support_is_required_for_abstentions':True,'critical_error_tolerance':0}):
        raise ValueError('calibration version or reviewed gate differs')
    for c in cases:
        parse_prediction(c['prediction'],{'s'})
        parse_judge({'accurate':c['expected']['semantic_accuracy'],'supported':c['expected']['supported'],'reason':'label contract'},c['prediction']['abstained'])
        for check in c['critical_checks']:
            operator=check.get('operator','equals')
            if (check['field'] not in gate['match_fields'] or operator not in {'equals','not_null'}
                or (operator=='not_null' and (check['field']!='supported' or c['expected']['supported'] is not None))
                or (operator=='equals' and (type(check['forbidden_value']) is not bool or check['forbidden_value'] is c['expected'][check['field']]))):
                raise ValueError('critical error check contradicts expected labels')
    return value


def budget_key(lock,models):
    # 不含批准文件路径、输出目录或代码hash；复制文件、换目录、修代码都不能重置本实验额度。
    return digest({'experiment':'phase3-reader-judge-v1','test_lock':lock['seal'],
                   'provider':models['reader']['provider'],'reader_model':models['reader']['model']})


def claim_execution(plan,out,budget_root=None):
    directory=Path(budget_root) if budget_root is not None else ROOT/'evals/runs/paid-budget'
    directory.mkdir(parents=True,exist_ok=True);path=directory/(plan['budget_key']+'.json')
    try:
        with path.open('x',encoding='utf-8') as claim:
            json.dump({'budget_key':plan['budget_key'],'plan_seal':plan['seal'],'ledger_path':str((out/'calls.jsonl').resolve()),
                       'pid':os.getpid(),'restart_policy':'stop; never reopen a fresh budget'},claim)
            claim.flush();os.fsync(claim.fileno())
    except FileExistsError:
        raise BudgetStop('this experiment already started; concurrent or restarted paid execution refused') from None
    return path


def tokenizers(args,models):
    base=Path(args.assets)
    return {key:TextTokenizer(base/entry['model'].split('/')[-1]/'tokenizer.json',entry['files']['tokenizer.json']['sha256'],add_special_tokens=key=='embedding')
            for key,entry in models.items() if key in {'embedding','reader_tokenizer'}}


def prepare(args,out):
    protocol,models,lock=load_configuration();calibration=load_calibration();ts=tokenizers(args,models)
    cases=load_cases(source_rows(args.longmemeval),lock)
    inventory=[]
    for case in cases:
        inp=case['input'];docs=inp['documents'];params=models['embedding_parameters']
        doc_tokens=[ts['embedding'].count(d['content']) for d in docs]
        query_tokens=ts['embedding'].count(f"Instruct: {params['instruction']}\nQuery:{inp['query']}")
        inventory.append({'id':inp['id'],'stratum':'abstention' if case['labels']['unanswerable'] else case['labels']['question_type'],
                          'document_chunks':len(docs),'embedding_document_tokens':sum(doc_tokens),
                          'max_document_tokens':max(doc_tokens),'embedding_query_tokens':query_tokens,
                          'evidence_chunks':len(case['labels']['evidence_qrels']),
                          'reader_question_tokens':ts['reader_tokenizer'].count(inp['query'])})
    limits=protocol['role_request_limits'];reserve=protocol['reasoning_reserve']
    quote_rows=[{'role':'reader','requests':limits['reader'],'input_cap':protocol['reader_input_cap'],'output_cap':protocol['reader_output_cap']+reserve},
                {'role':'judge','requests':limits['judge']+limits['calibration'],'input_cap':protocol['judge_input_cap'],'output_cap':protocol['judge_output_cap']+reserve},
                {'role':'smoke','requests':limits['smoke'],'input_cap':protocol['reader_input_cap'],'output_cap':protocol['reader_output_cap']+reserve}]
    for row in quote_rows:
        row['cost_ceiling_cny']=str((Decimal(row['input_cap'])*Decimal(protocol['price_cny_per_million_input'])+
            Decimal(row['output_cap'])*Decimal(protocol['price_cny_per_million_output']))*row['requests']/1000000)
    total=sum(Decimal(r['cost_ceiling_cny']) for r in quote_rows)
    if total>Decimal(protocol['max_cost_cny']):raise ValueError('shrink experiment before authorization')
    if (sum(r['requests']*r['input_cap'] for r in quote_rows)!=protocol['max_input_tokens_total']
        or sum(r['requests']*r['output_cap'] for r in quote_rows)!=protocol['max_output_tokens_total']):
        raise ValueError('global token limits differ from the role allocation')
    chunks=sum(r['document_chunks'] for r in inventory)
    if chunks>12000 or max(r['max_document_tokens'] for r in inventory)>4096:raise ValueError('local embedding compute/truncation budget exceeded')
    plan={'kind':'offline_preparation_not_model_results','protocol':protocol,'models':models,'lock_seal':lock['seal'],'budget_key':budget_key(lock,models),
          'lock_sha256':hashlib.sha256(LOCK.read_bytes()).hexdigest(),'calibration_sha256':hashlib.sha256(CALIBRATION.read_bytes()).hexdigest(),
          'calibration_file':str(CALIBRATION.relative_to(ROOT)),'calibration_version':calibration['version'],'calibration_gate':calibration['gate'],
          'reader_prompt_sha256':digest(READER_SYSTEM),'judge_prompt_sha256':digest(JUDGE_SYSTEM),'runtime_sha256':runtime_hashes(),
          'price_evidence':json.loads((CONFIG/'phase3_prices.json').read_text()),'inventory':inventory,
          'strata':dict(Counter(r['stratum'] for r in inventory)),'total_chunks':chunks,
          'total_embedding_tokens':sum(r['embedding_document_tokens']+r['embedding_query_tokens'] for r in inventory),
          'max_embedding_http_requests':sum((r['document_chunks']+31)//32+1 for r in inventory),
          'quote':quote_rows,'total_cost_ceiling_cny':str(total),'api_requests_executed':0,'actual_cost_cny':'0',
          'gates':['explicit user approval','account model/non-thinking JSON availability smoke',
                   'human review of calibration labels','local semantic model readiness; no silent BM25-only fallback'],
          'official_benchmark_score':False}
    plan['seal']=digest(plan);write_json(out/'plan.json',plan)
    write_json(out/'manifest.json',{'status':'prepared_waiting_for_authorization','git_head':command(['git','rev-parse','HEAD']),
               'source_sha256':{str(p.relative_to(ROOT)):hashlib.sha256(p.read_bytes()).hexdigest() for p in
                  [*Path('evals/memory_eval').resolve().glob('*.py'),LOCK,*CONFIG.glob('phase3*.json'),CALIBRATION]},
               'plan_seal':plan['seal'],'api_calls':0,'cost_cny':'0'})
    text=['# 第三阶段离线准备清单','',f"锁定 {len(cases)} 题、{chunks} 块；开发/测试的题目和全部会话均无重叠。",
          f"Embedding 文本精确 Token 总量 {plan['total_embedding_tokens']}；最多 {plan['max_embedding_http_requests']} 次本地 Embedding HTTP 请求。",
          f"收费请求上限 {sum(r['requests'] for r in quote_rows)}，最坏预留费用 {total} 元，执行硬上限 {protocol['max_cost_cny']} 元。",
          '', '本计划尚未进行收费调用或公开锁定集Reader/Judge实验；免费工程集Embedding另报。以下输入上限包括消息封装预留；输出包括128个思维Token保险预留。',
          '', '| 角色 | 次数 | 每次输入/计费输出上限 | 最坏预留元 |','|---|---:|---|---:|']
    text.extend(f"| {r['role']} | {r['requests']} | {r['input_cap']}/{r['output_cap']} | {r['cost_ceiling_cny']} |" for r in quote_rows)
    text+=['','批准必须绑定 plan.json 的 seal；不得自动创建批准文件。实际模型版本仍以响应 model/fingerprint 和本地 health 为准。']
    (out/'report.md').write_text('\n'.join(text)+'\n')


def reader_messages(inp,context):
    return [{'role':'system','content':READER_SYSTEM},{'role':'user','content':json.dumps(
        {'question':inp['query'],'question_date':inp['question_date'],'evidence':context['text']},ensure_ascii=False)}]


def judge_messages(inp,labels,prediction,context):
    return [{'role':'system','content':JUDGE_SYSTEM},{'role':'user','content':json.dumps(
        {'question':inp['query'],'question_date':inp.get('question_date'),'reference_answers':labels['answers'],
         'question_type':labels.get('question_type','single-session-user'),
         'unanswerable':labels['unanswerable'],'response':{k:prediction[k] for k in ['answer','abstained','citations']},'evidence':context['text']},ensure_ascii=False)}]


def check_approval(path,plan,out):
    approval=json.loads(Path(path).read_text())
    unsigned={k:v for k,v in plan.items() if k!='seal'}
    protocol,models,lock=load_configuration()
    if (digest(unsigned)!=plan['seal'] or approval.get('approved') is not True or approval.get('plan_seal')!=plan['seal']
        or approval.get('calibration_labels_human_reviewed') is not True
        or not isinstance(approval.get('reviewer'),str) or not approval['reviewer'].strip()
        or protocol!=plan['protocol'] or models!=plan['models'] or lock['seal']!=plan['lock_seal']
        or plan.get('budget_key')!=budget_key(lock,models)
        or digest(READER_SYSTEM)!=plan['reader_prompt_sha256'] or digest(JUDGE_SYSTEM)!=plan['judge_prompt_sha256']
        or runtime_hashes()!=plan['runtime_sha256']
        or hashlib.sha256(CALIBRATION.read_bytes()).hexdigest()!=plan['calibration_sha256']):
        raise BudgetStop('approval/plan/protocol/labels differ; no request sent')


def run_models(args,out):
    plan=json.loads(Path(args.plan).read_text());check_approval(args.approval,plan,out)
    write_json(out/'plan.json',plan)
    protocol=plan['protocol'];models=plan['models'];ts=tokenizers(args,models)
    cases=load_cases(source_rows(args.longmemeval),json.loads(LOCK.read_text()));calibration=load_calibration()
    write_json(out/'labels.json',{c['input']['id']:c['labels'] for c in cases})
    # 所有免费就绪检查先完成，避免模型缺失时消耗一次性审批。
    semantic=SemanticClient(args.semantic_url,models,ts['embedding'],out/'embedding-cache')
    claim_path=claim_execution(plan,out)
    approval=json.loads(Path(args.approval).read_text())
    write_json(out/'approval-record.json',{'plan_seal':plan['seal'],'reviewer':approval['reviewer'],
               'calibration_labels_human_reviewed':True,'max_cost_cny':protocol['max_cost_cny']})
    ledger=Ledger(out/'calls.jsonl',protocol);chat=ChatClient(models['reader'],protocol,ledger,approved=True)
    rows=[];manifest={'status':'running','kind':'public_subset_not_official_full_benchmark','plan_seal':plan['seal'],
                      'git_head':command(['git','rev-parse','HEAD']),
                      'semantic_gateway':semantic.health,'budget_key':plan['budget_key'],'budget_claim':str(claim_path)}
    write_json(out/'manifest.json',manifest)
    try:
        # 先核实本地真实语义能力，再发任何收费请求；缺模型时整组停止。
        for _ in range(2):
            chat.call([{'role':'system','content':'Return a JSON object only.'},{'role':'user','content':'Return {"ok": true}.'}],'smoke')
        cal_rows=[]
        for c in calibration['cases']:
            inp={'id':c['id'],'query':c['question']};ctx={'text':'\nSOURCE s\n'+c['context']}
            result=chat.call(judge_messages(inp,c,c['prediction'],ctx),'calibration')
            judgment=parse_judge(result['json'],c['prediction']['abstained'])
            cal_rows.append({'id':c['id'],'judgment':judgment})
            cal_summary=calibration_summary(calibration,cal_rows)
            write_json(out/'judge-calibration.json',cal_summary['cases'])
            write_json(out/'judge-calibration-summary.json',cal_summary)
        if not cal_summary['passed']:raise BudgetStop('judge calibration requires 15/16 complete matches and zero critical errors; stop before locked test')
        subprocess.run(['cargo','build','--locked','--offline','--manifest-path',str(ROOT/'evals/harness/Cargo.toml')],env={**os.environ,'CARGO_TARGET_DIR':str(TARGET)},check=True,timeout=600)
        with (out/'bridge.log').open('w') as log,Sandbox() as sb:
            sb.start(log)
            for i,case in enumerate(cases):
                inp=case['input'];labels=case['labels'];tenant=f'phase3-{i}'
                index_started=time.perf_counter_ns()
                hits,docs,ingestion=corpus(sb,tenant,inp['documents'],True)
                ingest_ms=(time.perf_counter_ns()-index_started)/1e6;index_started=time.perf_counter_ns()
                tfidf=TfidfIndex(docs);dense=DenseIndex(docs,semantic)
                index_ms=(time.perf_counter_ns()-index_started)/1e6;query_started=time.perf_counter_ns()
                semantic.embed([inp['query']],'query')
                query_embedding_ms=(time.perf_counter_ns()-query_started)/1e6
                reverse={h['memory_version_id']:did for did,h in hits.items()};by_id={d['id']:d for d in docs}
                methods=sorted(['oracle',*protocol['retrieval_methods']],key=lambda m:digest([protocol['seed'],inp['id'],m]))
                for method in methods:
                    start=time.perf_counter_ns()
                    if method=='oracle':
                        selected=[{**hits[s],'provider_score':1.0} for s in labels['evidence_qrels'] if s in hits]
                        result=sb.call('rank',tenant,{'method':'bm25','query':inp['query'],'bm25':selected,'vector':[],'budget':20})
                    else:
                        alias={'tfidf':'vector_tfidf','semantic':'vector_tfidf','hybrid_semantic':'hybrid','bm25':'bm25'}[method]
                        result=retrieve(sb,tenant,inp['query'],dense if method in {'semantic','hybrid_semantic'} else tfidf,hits,alias,20)
                    if result.get('error'):
                        rows.append({'id':inp['id'],'method':method,'error':result['error']});write_json(out/'questions.json',rows)
                        raise RuntimeError('retrieval failed; no model call or silent fallback')
                    retrieval_done=time.perf_counter_ns()
                    ranked=[reverse[h['memory_version_id']] for h in result['hits']]
                    # 正文只取最终 canonical 授权结果，绝不从未授权输入对象直接拼 Context。
                    authorized={m['memory_version_id']:m['content'] for m in result['memories']}
                    selected=[{**by_id[reverse[h['memory_version_id']]],'content':authorized[h['memory_version_id']]} for h in result['hits']]
                    ctx=build_context(selected,ts['reader_tokenizer'],protocol['context'])
                    row={'id':inp['id'],'method':method,'unanswerable':labels['unanswerable'],'ranked':ranked,'context':ctx,
                         'ingestion':ingestion,'retrieval_ms':(retrieval_done-start)/1e6,
                         'context_ms':(time.perf_counter_ns()-retrieval_done)/1e6,'ingest_ms_shared':ingest_ms,'index_ms_shared':index_ms,
                         'query_embedding_ms_shared':query_embedding_ms if method in {'semantic','hybrid_semantic'} else 0,
                         **stage_scores(ranked,inp['documents'],labels,ctx)}
                    trace_dir=out/'traces'/inp['id'];trace_dir.mkdir(parents=True,exist_ok=True)
                    trace_path=trace_dir/(method+'.json');write_json(trace_path,row)
                    if method in protocol['reader_methods']:
                        messages=reader_messages(inp,ctx);row['reader_input']=messages;write_json(trace_path,row)
                        rr=chat.call(messages,'reader')
                        prediction=parse_prediction(rr['json'],{d['source_id'] for d in ctx['items']})
                        row.update(prediction=prediction,reader_record=rr);write_json(trace_path,row)
                        row['qa_latency_ms']=(time.perf_counter_ns()-start)/1e6
                        jj=chat.call(judge_messages(inp,labels,prediction,ctx),'judge')
                        judgment=parse_judge(jj['json'],prediction['abstained'])
                        row.update(prediction=prediction,judgment=judgment,reader_input=messages,reader_record=rr,judge_record=jj,
                                   usage=rr['usage'],judge_usage=jj['usage'],
                                   **answer_scores(prediction,labels['answers'],labels['unanswerable'],judgment))
                    row['evaluation_latency_ms']=(time.perf_counter_ns()-start)/1e6
                    row['latency_ms']=row.get('qa_latency_ms',row['retrieval_ms'])
                    write_json(trace_path,row)
                    rows.append(row);write_json(out/'questions.json',rows)
        manifest['status']='completed'
    except Exception as error:
        manifest.update(status='stopped',error_type=type(error).__name__,
                        reason=str(error) if isinstance(error,BudgetStop) else 'see per-call ledger and failure stage')
        write_json(out/'failure.json',{'question_id':inp.get('id') if 'inp' in locals() else None,
                   'method':method if 'method' in locals() else 'readiness_or_calibration',
                   'last_request':ledger.requests,'error_type':type(error).__name__})
        write_json(out/'summary.json',{'status':'stopped','completed_rows':len(rows),'expected_questions':len(cases),
                   'quality_metrics':None,'reason':'incomplete experiment; inspect traces and ledger'})
        (out/'report.md').write_text('# 实验已停止\n\n批次未完成，不计完整Benchmark成绩。见failure.json、traces和calls.jsonl；未自动重试或续费。\n')
        write_json(out/'manifest.json',manifest)
        raise
    finally:
        manifest.update(request_reservations=ledger.requests,api_dispatch_attempts=ledger.dispatches,
                        cost_reserved_or_estimated_cny=str(ledger.cost),embedding_calls=semantic.calls if 'semantic' in locals() else 0)
        write_json(out/'manifest.json',manifest);ledger.file.close()
    summary={}
    for method in ['oracle',*protocol['retrieval_methods']]:
        group=[r for r in rows if r['method']==method];pos=[r for r in group if not r['unanswerable']]
        summary[method]={**(summarize(group,len(cases)) if method in protocol['reader_methods'] else {}),
                         'session_recall_at_5':mean([r['session']['recall_at_5'] for r in pos]),
                         'evidence_chunk_recall_at_5':mean([r['evidence']['recall_at_5'] for r in pos]),
                         'context_evidence_char_coverage':mean([r['context_evidence_coverage'] for r in pos]),
                         'evidence_annotated_positive_n':sum(r['evidence']['recall_at_5'] is not None for r in pos),
                         'retrieval_p50_ms':percentile([r['retrieval_ms'] for r in group],.5),
                         'retrieval_p95_ms':percentile([r['retrieval_ms'] for r in group],.95),
                         'reader_judge_cost_cny':str(sum(Decimal(r[k]['cost_cny']) for r in group for k in ['reader_record','judge_record'] if k in r)),
                         'reader_input_tokens':sum(r['usage']['prompt_tokens'] for r in group if 'usage' in r),
                         'reader_output_tokens':sum(r['usage']['completion_tokens'] for r in group if 'usage' in r),
                         'judge_total_tokens':sum(r['judge_usage']['prompt_tokens']+r['judge_usage']['completion_tokens'] for r in group if 'judge_usage' in r)}
    write_json(out/'summary.json',summary)
    write_json(out/'labels.json',{c['input']['id']:c['labels'] for c in cases})
    lines=['# 真实模型公开子集结果','','非官方完整Benchmark或排行榜成绩。每组14题（12正例+2拒答），同一Reader/Context策略。',
           '', '| 方案 | Session R@5 | 标记块R@5 | Context字符覆盖 | 检索P50/P95 ms |','|---|---:|---:|---:|---|']
    for method,r in summary.items():lines.append(f"| {method} | {r['session_recall_at_5']} | {r['evidence_chunk_recall_at_5']} | {r['context_evidence_char_coverage']} | {r['retrieval_p50_ms']:.2f}/{r['retrieval_p95_ms']:.2f} |")
    lines += ['', '| Reader组 | 正例严格/规范化（12题） | Judge准确率（14题） | 拒答（2题） | 无依据回答率 | Reader输入/输出Token | Judge Token | 元 |',
              '|---|---|---:|---:|---:|---|---:|---:|']
    for method in protocol['reader_methods']:
        r=summary[method]
        lines.append(f"| {method} | {r['strict_answerable_accuracy']}/{r['normalized_answerable_accuracy']} | {r['semantic_accuracy_all']} | {r['abstention_accuracy']} | {r['unsupported_answer_rate']} | {r['reader_input_tokens']}/{r['reader_output_tokens']} | {r['judge_total_tokens']} | {r['reader_judge_cost_cny']} |")
    lines += ['',f"全批次按公示价和usage估算/预留费用 {ledger.cost} 元，包含smoke和校准，不等于供应商账单；各组表只计逐题Reader/Judge。建索引和共享query编码另见逐题记录。",
              '', '证据块是官方has_answer代理，非人工事实标注；Oracle覆盖不足时不是证据充分实验。TF-IDF未运行付费Reader。',
              '逐题证据、实际输入、Oracle、引用及失败见traces和questions.json；模型输出/usage/金额见calls.jsonl。']
    (out/'report.md').write_text('\n'.join(lines)+'\n')


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('mode',choices=['assets','prepare','models-run'])
    parser.add_argument('--output',required=True);parser.add_argument('--assets',default='/tmp/drb-phase3-assets')
    parser.add_argument('--longmemeval',default='/tmp/drb-eval-public/longmemeval_s_cleaned.json')
    parser.add_argument('--plan');parser.add_argument('--approval');parser.add_argument('--semantic-url',default='http://127.0.0.1:19882')
    args=parser.parse_args()
    if Path(args.output).exists():parser.error('output must not exist')
    if args.mode=='models-run' and (not args.approval or not args.plan):parser.error('explicit reviewed plan and approval required; no paid calls')
    out=Path(args.output);out.mkdir(parents=True)
    {'assets':assets,'prepare':prepare,'models-run':run_models}[args.mode](args,out)
    print(out.resolve())


if __name__=='__main__':main()
