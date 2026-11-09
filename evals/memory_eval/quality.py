"""长期记忆质量评测：只用临时PG和本地Tokenizer，不含收费模型入口。"""
import argparse
from collections import Counter
import csv
import hashlib
import json
import os
from pathlib import Path
import random
import subprocess
import time

from .baselines import TfidfIndex
from .metrics import mean,percentile,retrieval_metrics
from .models3 import TextTokenizer
from .runner import ROOT,TARGET,Sandbox,corpus,ingest,write_json

CONFIG=ROOT/'evals/configs/memory_quality_v1.json'
RETRIEVAL=ROOT/'evals/datasets/context_quality_v1.json'
GOVERNANCE=ROOT/'evals/datasets/governance_quality_v1.json'


def load_inputs():
    config=json.loads(CONFIG.read_text());data=json.loads(RETRIEVAL.read_text());governance=json.loads(GOVERNANCE.read_text())
    for path in [RETRIEVAL,GOVERNANCE,ROOT/data['base_dataset']['path']]:
        if hashlib.sha256(path.read_bytes()).hexdigest()!=config['dataset_sha256'][str(path.relative_to(ROOT))]:
            raise ValueError('dataset hash differs from reviewed draft; reseal explicitly')
    base=json.loads((ROOT/data['base_dataset']['path']).read_text())
    if hashlib.sha256((ROOT/data['base_dataset']['path']).read_bytes()).hexdigest()!=data['base_dataset']['sha256']:
        raise ValueError('base dataset changed')
    docs=base['documents']+data['documents'];by_id={d['id']:d for d in docs}
    if len(by_id)!=len(docs) or len(data['queries'])!=48 or len({q['id'] for q in data['queries']})!=48:
        raise ValueError('duplicate document/query ID or wrong query count')
    if len(Counter(q['category'] for q in data['queries']))!=8 or set(Counter(q['category'] for q in data['queries']).values())!={6}:
        raise ValueError('expected eight query categories, six each')
    old_queries={q['query'] for q in base['queries']}
    for q in data['queries']:
        if q['query'] in old_queries or not q['query'].strip() or type(q['unanswerable']) is not bool:
            raise ValueError('invalid or reused query')
        if q['unanswerable']!= (not q['qrels']) or set(q['qrels'])!={e['document_id'] for e in q['required_evidence']}:
            raise ValueError('answerability/qrels/evidence inconsistent')
        if any(type(v) is not int or not 1<=v<=3 for v in q['qrels'].values()):raise ValueError('invalid relevance grade')
        for e in q['required_evidence']:
            if e['document_id'] not in by_id or not e['spans'] or any(not s or s not in by_id[e['document_id']]['content'] for s in e['spans']):
                raise ValueError('evidence span missing from declared document')
        if q['input_policy'] not in {'query_only','explicit_history_concat'} or any(h['role'] not in {'user','assistant'} for h in q['history']):
            raise ValueError('invalid history policy')
    cases=governance['cases']
    if (len(cases)!=24 or len({c['id'] for c in cases})!=24 or len(Counter(c['family'] for c in cases))!=8
        or set(Counter(c['family'] for c in cases).values())!={3}):
        raise ValueError('expected 24 unique governance cases in eight families')
    for c in cases:
        if c['human_review']!='pending' or not c['steps'] or any(s['op'] not in {'ingest','correct','review','cache','forget'} for s in c['steps']):
            raise ValueError('invalid governance draft')
    return config,data,governance,docs


def query_text(q):
    # 显式拼接历史是已声明的Adapter策略，不做指代改写，也不接触答案。
    if q['input_policy']=='query_only':return q['query']
    return '\n'.join(h['role']+': '+h['content'] for h in q['history'])+'\n当前问题: '+q['query']


def render_memories(items):
    # 两种注入方案使用同一格式；计数包括来源版本标识和分隔符，不含Reader提示词。
    return '\n'.join('<memory version="'+i['memory_version_id']+'">\n'+i.get('text',i.get('content',''))+'\n</memory>' for i in items)


def token_comparison(full,pack,tokenizer):
    a=render_memories(full);b=render_memories(pack);at=tokenizer.count(a);bt=tokenizer.count(b)
    return {'full_memory_count':len(full),'pack_memory_count':len(pack),'full_tokens':at,'pack_tokens':bt,
            'token_saving_ratio':(at-bt)/at if at else None,'full_rendered':a,'pack_rendered':b,
            'measurement':'exact_local_tokenizer_on_rendered_memory_payload_not_provider_usage'}


def evidence_retention(q,items,reverse):
    delivered={reverse[i['memory_version_id']]:i.get('text',i.get('content','')) for i in items if i['memory_version_id'] in reverse}
    flags=[all(s in delivered.get(e['document_id'],'') for s in e['spans']) for e in q['required_evidence']]
    return {'evidence_units':len(flags),'evidence_units_delivered':sum(flags),'evidence_recall':mean(flags),
            'all_evidence_delivered':all(flags) if flags else None,'effective_context_empty':not any(flags) if flags else None}


def stage_failures(stages,gold):
    # 只对存在可用金证据的题归因；正确拒绝越权/旧版本在治理组另报。
    if not gold:return {'first_loss':None,'stage_hits':{},'stage_complete':{}}
    flags={name:bool(set(ids)&gold) for name,ids in stages.items()}
    complete={name:gold<=set(ids) for name,ids in stages.items()}
    first=next((name for name,hit in flags.items() if not hit),None)
    return {'first_loss':first,'stage_hits':flags,'stage_complete':complete}


def retrieval_case(sb,tenant,q,index,hits,method,config,tokenizer,full):
    query=query_text(q);budget=config['candidate_budget'];depth=budget//2 if method.startswith('hybrid') else budget
    start=time.perf_counter_ns()
    bm=sb.call('search',tenant,{'query':query,'budget':depth},allow_error=True) if method!='vector_tfidf' else []
    if isinstance(bm,dict):
        return {'id':q['id'],'category':q['category'],'method':method,'unanswerable':q['unanswerable'],
                'error':bm['error'],'retrieval_ms':(time.perf_counter_ns()-start)/1e6,'qrels':q['qrels'],
                **retrieval_metrics([],q['qrels']),'first_loss':'request_failure','token_metrics':None}
    vector=[{**hits[d],'provider_score':score} for d,score in index.search(query,depth)] if method!='bm25' else []
    result=sb.call('rank',tenant,{'method':method,'query':query,'bm25':bm,'vector':vector,'budget':budget,'trace':True})
    elapsed=(time.perf_counter_ns()-start)/1e6;reverse={h['memory_version_id']:d for d,h in hits.items()}
    trace=result['trace'];ids=lambda xs:[reverse[x['memory_version_id']] for x in xs if x['memory_version_id'] in reverse]
    # 复用既有QA策略：相关性重排后才交给生产Context的0.4闸门，纯检索成绩仍取原排名。
    context_rank=result if method=='hybrid_rerank' else sb.call('rank',tenant,{'method':'bm25','query':query,
        'bm25':result['hits'],'vector':[],'budget':budget,'rerank_for_context':True,'trace':True})
    context_start=time.perf_counter_ns()
    pack=sb.call('context',tenant,{'query':query,'hits':context_rank['hits'],'token_budget':config['context_char_budget']})
    context_ms=(time.perf_counter_ns()-context_start)/1e6
    items=[i for section in pack['sections'] for i in section['items']]
    stages={'source_union':ids(bm+vector),'rrf':ids(trace['fused']),'preauthorization':ids(trace['preauthorized']),
            'rerank':ids(trace['reranked']),'output_limit':ids(result['hits']),'postauthorization':ids(result['memories']),
            'context_relevance':ids(context_rank['memories']),'context_pack':ids(items)}
    return {'id':q['id'],'category':q['category'],'method':method,'unanswerable':q['unanswerable'],
            'input_query':query,'input_policy':q['input_policy'],'qrels':q['qrels'],'ranked':ids(result['hits']),
            'database_relevant_exists':set(q['qrels'])<=set(hits) if q['qrels'] else None,
            'retrieval_ms':elapsed,'context_ms':context_ms,'error':None,
            'source_bm25':bm,'source_vector':vector,'retrieval_trace':trace,'context_rank_trace':context_rank['trace'],
            'stages':stages,**stage_failures(stages,set(q['qrels'])),**retrieval_metrics(ids(result['hits']),q['qrels']),
            'literal_context_empty':not items,**evidence_retention(q,items,reverse),'context_pack':pack,
            'full_evidence_retention':evidence_retention(q,full,reverse),
            'token_metrics':token_comparison(full,items,tokenizer),'negative_any_hit':bool(result['hits']) if q['unanswerable'] else None}


def governance_case(sb,c):
    tenant='quality-'+c['id'];refs={};cached=[];compiled=None;steps=[]
    projects=next((s['projects'] for s in c['steps'] if 'projects' in s),[])
    for n,s in enumerate(c['steps']):
        op=s['op']
        if op=='ingest':
            value=ingest(sb,tenant,{'id':str(n),'content':s['text']},semantic_key='quality.'+s['semantic_key'],
                         scope=s.get('scope','user'),scope_id=s.get('scope_id'),projects=s.get('projects',[]),
                         memory_type=s.get('memory_type','episodic_summary'))
            refs[s['as']]=value
        elif op in {'correct','forget'}:
            record=refs[s['target']]['memory_record_id'];before=sb.call('inspect',tenant,{'record':record},projects=projects)
            value=sb.call(op,tenant,{'record':record,'revision':before['revision'],'key':'quality-'+c['id']+'-'+str(n),
                         **({'content':s['text']} if op=='correct' else {})},projects=projects)
            if op=='correct':refs[s['as']]=value
        elif op=='review':
            value=sb.call('review',tenant,{'candidate':refs[s['target']]['candidate_id'],
                         'key':hashlib.sha256((c['id']+':review:'+str(n)).encode()).hexdigest(),'outcome':s['outcome']},projects=projects)
        else:
            sb.call('drain',tenant);row=refs[s['target']]
            state=sb.call('state',tenant);v=next(x for x in state['retrieval_documents'] if x['memory_version_id']==row['memory_version_id'])
            cached=[{k:v[k] for k in ['memory_record_id','memory_version_id','head_revision','generation','content_hash']}]
            cached[0].update(provider_object_id='quality-cached',provider_score=1.0)
            compiled=sb.call('context_compile',tenant,{'query':'我住在哪里？','hits':cached},projects=projects);value=compiled
        steps.append({'input':s,'output':value})
        if s.get('drain',True):sb.call('drain',tenant)
    state=sb.call('state',tenant);head=state['memory_heads'][0];e=c['expected'];versions=state['memory_versions']
    current=next((v['content'] for v in versions if v['id']==head['current_version_id']),None)
    allowed=sb.call('authorize',tenant,{'hits':cached},projects=projects) if cached else []
    consume=None
    if compiled:
        consume=sb.call('context_consume',tenant,{k:compiled[k] for k in ['context_pack_id','task_id','attempt_id']}|{'key':c['id']+'-consume'},projects=projects,allow_error=True)
    # 用所有现存投影候选验证可见性，避免把词法未命中误当成遗忘或权限成功。
    allhits=[{k:v[k] for k in ['memory_record_id','memory_version_id','head_revision','generation','content_hash']}|
             {'provider_object_id':'quality-state','provider_score':1.0} for v in state['retrieval_documents']]
    visible=sb.call('authorize',tenant,{'hits':allhits},projects=projects)
    scope_allowed=None;owner_visible=None
    if 'probe' in c['labels']:
        probe=c['labels']['probe'];owner_visible=bool(visible)
        scope_allowed=bool(sb.call('authorize',tenant+'-other' if probe.get('other_tenant') else tenant,{'hits':allhits},
                          user=probe.get('user','evaluator'),projects=probe.get('projects',[])))
    source_ids={x['id'] for x in state['source_events']};candidate_ids={x['id'] for x in state['memory_candidates']}
    candidate_links=state['candidate_sources'];version_links=state['memory_version_sources']
    valid_chain=(bool(version_links) and all(x['source_event_id'] in source_ids for x in version_links)
                 and source_ids=={x['source_event_id'] for x in version_links}
                 and all(x['candidate_id'] in candidate_ids and x['source_event_id'] in source_ids for x in candidate_links)
                 and all(any(x['candidate_id']==cid for x in candidate_links) for cid in candidate_ids)
                 and all(any(x['memory_version_id']==v['id'] for x in version_links) for v in versions))
    # 重复识别与来源完整性分别计分，不能因来源边缺失把已复用的版本判成未识别。
    a=refs.get('a',{});b=refs.get('b',{})
    observed_duplicate=(bool(a) and bool(b) and a.get('memory_record_id')==b.get('memory_record_id')
                        and a.get('memory_version_id')==b.get('memory_version_id') and head['state']=='active')
    checks={'version_count':len(versions)==e['versions'],'record_count':len(state['memory_records'])==e.get('records',1)}
    if len(state['memory_records'])==1:checks.update(head_state=head['state']==e['head_state'],current_text=current==e['current_text'])
    for key in ['duplicate','conflict']:
        if key in c['labels']:checks[key]=(observed_duplicate if key=='duplicate' else any(h['state']=='contested' for h in state['memory_heads'])) is c['labels'][key]
    if 'sources' in e:checks['source_count']=len(source_ids)==e['sources'] and len({x['source_event_id'] for x in version_links})==e['sources']
    if e.get('source_chain'):checks['source_chain']=valid_chain
    if 'stale_allowed' in e:checks['stale_candidate']=bool(allowed) is e['stale_allowed']
    if 'stale_pack_allowed' in e:checks['stale_pack']=('error' not in consume) is e['stale_pack_allowed']
    if 'visible' in e:checks['visible']=bool(visible) is e['visible']
    if 'scope_allowed' in e:checks['scope_isolation']=scope_allowed is e['scope_allowed'] and owner_visible
    if 'forgotten_records' in e:checks['forgotten_count']=sum(h['state']=='forgotten' for h in state['memory_heads'])==e['forgotten_records']
    if 'remaining_text' in e:checks['remaining_visible']=e['remaining_text'] in [m['content'] for m in visible]
    if 'retained_history_text' in e:checks['history_retained']=e['retained_history_text'] in [v['content'] for v in versions]
    # 活跃单事实必须真实编译/消费当前版本，而不是只检查Head字段。
    current_pack=None
    if len(state['memory_records'])==1 and head['state']=='active':
        current_pack=sb.call('context',tenant,{'query':'当前事实是什么？','hits':allhits},projects=projects)
        texts=[i['text'] for sec in current_pack['sections'] for i in sec['items']]
        checks['current_context']=texts==[e['current_text']]
    return {'id':c['id'],'family':c['family'],'human_review':'pending','passed':all(checks.values()),'checks':checks,
            'expected_labels':c['labels'],'expected_state':e,
            'observed_duplicate':observed_duplicate,'observed_conflict':any(h['state']=='contested' for h in state['memory_heads']),
            'state':state,'steps':steps,'cached_hits':cached,'cached_authorized':allowed,'old_pack_consume':consume,
            'visible':visible,'scope_allowed':scope_allowed,'owner_visible':owner_visible,'current_pack':current_pack}


def summarize(rows,governance,methods,expected_n=48):
    result={'review_status':'pending_not_final_human_confirmed_scores','methods':{},'governance':{}}
    for method in methods:
        group=[r for r in rows if r['method']==method];positive=[r for r in group if not r['unanswerable']];negative=[r for r in group if r['unanswerable']]
        if len(group)!=expected_n or len({r['id'] for r in group})!=expected_n:
            raise ValueError('missing or duplicate results; no completed aggregate')
        successful=[r for r in positive if not r['error']];tokens=[r['token_metrics'] for r in group if r.get('token_metrics')]
        values={k:mean([r[k] for r in group if k in r]) for k in ['recall_at_1','recall_at_5','mrr_at_10','ndcg_at_10']}
        stages=list(successful[0]['stage_hits']) if successful else []
        losses={}
        for i,name in enumerate(stages):
            eligible=successful if i==0 else [r for r in successful if r['stage_hits'][stages[i-1]]]
            lost=sum(not r['stage_hits'][name] for r in eligible)
            losses[name]={'lost_queries':lost,'conditional_denominator':len(eligible),'conditional_rate':lost/len(eligible) if eligible else None,
                          'first_loss_queries':sum(r['first_loss']==name for r in successful)}
        result['methods'][method]={'n':len(group),'positive_n':len(positive),'negative_n':len(negative),'errors':sum(bool(r['error']) for r in group),**values,
            'literal_empty_measured_n':len(successful),
            'literal_empty_positive_rate':mean([r['literal_context_empty'] for r in successful]),
            'literal_or_request_failure_rate':sum(r.get('literal_context_empty',True) for r in positive)/len(positive) if positive else None,
            'positive_request_failure_rate':sum(bool(r['error']) for r in positive)/len(positive) if positive else None,
            'retrieval_miss_including_failure_rate':sum(bool(r['error']) or not r['stage_hits']['source_union'] for r in positive)/len(positive) if positive else None,
            'authorization_stage_loss_rate':sum(r['first_loss'] in {'preauthorization','postauthorization'} for r in positive)/len(positive) if positive else None,
            'effective_empty_positive_rate':sum(r.get('effective_context_empty',True) for r in positive)/len(positive) if positive else None,
            'all_evidence_delivery_rate':sum(r.get('all_evidence_delivered',False) for r in positive)/len(positive) if positive else None,
            'negative_any_hit_rate':mean([r.get('negative_any_hit') for r in negative]),'stage_losses':losses,
            'retrieval_p50_ms':percentile([r['retrieval_ms'] for r in group],.5),'retrieval_p95_ms':percentile([r['retrieval_ms'] for r in group],.95),
            'token_successful_queries':len(tokens),'token_failed_queries':len(group)-len(tokens),
            'full_memory_count_mean':mean([t['full_memory_count'] for t in tokens]),'pack_memory_count_mean':mean([t['pack_memory_count'] for t in tokens]),
            'full_tokens_mean':mean([t['full_tokens'] for t in tokens]),'pack_tokens_mean':mean([t['pack_tokens'] for t in tokens]),
            'full_tokens_p50':percentile([t['full_tokens'] for t in tokens],.5),'full_tokens_p95':percentile([t['full_tokens'] for t in tokens],.95),
            'pack_tokens_p50':percentile([t['pack_tokens'] for t in tokens],.5),'pack_tokens_p95':percentile([t['pack_tokens'] for t in tokens],.95),
            'token_saving_micro':1-sum(t['pack_tokens'] for t in tokens)/sum(t['full_tokens'] for t in tokens) if sum(t['full_tokens'] for t in tokens) else None,
            'token_saving_macro':mean([t['token_saving_ratio'] for t in tokens]),
            'by_category':{cat:{'n':len(part),'positive_n':sum(not r['unanswerable'] for r in part),
                **{k:mean([r[k] for r in part]) for k in values}} for cat in sorted({r['category'] for r in group})
                for part in [[r for r in group if r['category']==cat]]}}
    for family in sorted({r['family'] for r in governance}):
        group=[r for r in governance if r['family']==family]
        result['governance'][family]={'n':len(group),'passed':sum(r['passed'] for r in group),'accuracy':mean([r['passed'] for r in group])}
    for label,family in [('duplicate','重复识别'),('conflict','冲突识别')]:
        group=[r for r in governance if r['family']==family]
        if group:
            result['governance'][family]['label_accuracy']=mean([r['checks'][label] for r in group])
            result['governance'][family]['confusion']={
                name:sum(r['expected_labels'][label] is gold and r['observed_'+label] is actual for r in group)
                for name,gold,actual in [('tp',True,True),('tn',False,False),('fp',False,True),('fn',True,False)]}
    return result


def write_csv(path,rows):
    if not rows:return
    # 嵌套数据保留JSON，CSV只导出适合逐题筛选的列，失败行不会删除。
    keys=list(dict.fromkeys(k for r in rows for k,v in r.items() if not isinstance(v,(dict,list))))
    with path.open('w',newline='',encoding='utf-8') as f:
        writer=csv.DictWriter(f,fieldnames=keys);writer.writeheader()
        writer.writerows({k:r.get(k) for k in keys} for r in rows)


def prepare(out):
    config,data,gov,docs=load_inputs();out.mkdir(parents=True,exist_ok=False)
    write_json(out/'protocol.json',config)
    rows=['# 新增48题人工标注复核清单','','状态：全部待复核；非公开Benchmark，不宣布人工通过。多轮题按显式历史拼接单列。','',
          '| ID | 类别 | 实际查询（含历史） | 参考事实 | 必要证据及原文片段 |','|---|---|---|---|---|']
    clean=lambda s:s.replace('|','\\|').replace('\n','<br>')
    for q in data['queries']:
        rows.append('| '+' | '.join(clean(str(x)) for x in [q['id'],q['category'],query_text(q),q['reference_fact'],
            '; '.join(e['document_id']+': '+ '；'.join(e['spans']) for e in q['required_evidence']) or '无答案：全量合成语料未记载'])+' |')
    (out/'retrieval-review.md').write_text('\n'.join(rows)+'\n')
    rows=['# 24个治理场景人工标注复核清单','','状态：全部待复核；直接提供Candidate/Correction，不评估LLM理解。g03/g06保留能力预期，当前已知可能失败。','',
          '| ID | 类别/场景 | 操作输入 | 预期标签和最终状态 | 理由 |','|---|---|---|---|---|']
    for c in gov['cases']:
        rows.append('| '+' | '.join(clean(str(x)) for x in [c['id'],c['family']+' / '+c['title'],
            json.dumps(c['steps'],ensure_ascii=False),json.dumps({'labels':c['labels'],'state':c['expected']},ensure_ascii=False),c['reason']])+' |')
    (out/'governance-review.md').write_text('\n'.join(rows)+'\n')
    write_csv(out/'retrieval-review.csv',[{'id':q['id'],'category':q['category'],'query':query_text(q),'reference_fact':q['reference_fact'],
              'required_evidence':json.dumps(q['required_evidence'],ensure_ascii=False),'human_review':'pending'} for q in data['queries']])
    write_csv(out/'governance-review.csv',[{'id':c['id'],'family':c['family'],'title':c['title'],'steps':json.dumps(c['steps'],ensure_ascii=False),
              'expected':json.dumps(c['expected'],ensure_ascii=False),'labels':json.dumps(c['labels'],ensure_ascii=False),'reason':c['reason'],'human_review':'pending'} for c in gov['cases']])
    write_json(out/'manifest.json',{'status':'prepared_for_human_review','query_count':48,'governance_count':24,'documents':len(docs),
               'human_review':'pending','paid_api_calls':0,'dataset_sha256':config['dataset_sha256']})


def run(out,assets):
    config,data,gov,docs=load_inputs();out.mkdir(parents=True,exist_ok=False)
    entry=config['tokenizer'];tokenizer=TextTokenizer(Path(assets)/'DeepSeek-V3.2/tokenizer.json',entry['sha256'])
    sources=[Path(__file__),ROOT/'evals/harness/src/main.rs',ROOT/'evals/memory_eval/runner.py',ROOT/'evals/memory_eval/models3.py',ROOT/'crates/memory-postgres/src/repository.rs']
    manifest={'status':'running','human_review':'pending','interpretation':'provisional_engineering_run_not_final_reviewed_scores','paid_api_calls':0,
              'git_head':subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip(),'dataset_sha256':config['dataset_sha256'],
              'tokenizer':entry,'source_sha256':{str(p.relative_to(ROOT)):hashlib.sha256(p.read_bytes()).hexdigest() for p in sources},
              'semantic_embedding':'not_run_no_tfidf_substitution','production_http':'not_run_Repository_ports_only','protocol':config}
    rows=[];governance=[];write_json(out/'manifest.json',manifest)
    try:
        subprocess.run(['cargo','build','--locked','--offline','--manifest-path',str(ROOT/'evals/harness/Cargo.toml')],
                       env={**os.environ,'CARGO_TARGET_DIR':str(TARGET)},check=True,timeout=600)
        with (out/'bridge.log').open('w') as log,Sandbox() as sb:
            sb.start(log);h,effective,ingestion=corpus(sb,'quality-retrieval',docs);index=TfidfIndex(effective)
            full=sb.call('authorize','quality-retrieval',{'hits':list(h.values())})
            write_json(out/'authorized-full-memory.json',full);manifest['ingestion']=ingestion;manifest['postgres_image_id']=sb.image
            queries=list(data['queries']);rng=random.Random(config['seed']);rng.shuffle(queries)
            for q in queries:
                methods=list(config['methods']);rng.shuffle(methods)
                for method in methods:
                    rows.append(retrieval_case(sb,'quality-retrieval',q,index,h,method,config,tokenizer,full))
                    write_json(out/'retrieval.json',rows)
            for c in gov['cases']:
                governance.append(governance_case(sb,c));write_json(out/'governance.json',governance)
        manifest['status']='completed'
        result=summarize(rows,governance,config['methods']);write_json(out/'summary.json',result)
        report_fields={
            'context-token':['n','token_successful_queries','token_failed_queries','full_memory_count_mean','pack_memory_count_mean',
                'full_tokens_mean','pack_tokens_mean','full_tokens_p50','full_tokens_p95','pack_tokens_p50','pack_tokens_p95',
                'token_saving_micro','token_saving_macro','all_evidence_delivery_rate'],
            'empty-context':['n','positive_n','negative_n','errors','literal_empty_measured_n','literal_empty_positive_rate',
                'literal_or_request_failure_rate','positive_request_failure_rate','retrieval_miss_including_failure_rate',
                'authorization_stage_loss_rate','effective_empty_positive_rate','all_evidence_delivery_rate','stage_losses'],
            'retrieval':['n','positive_n','negative_n','errors','recall_at_1','recall_at_5','mrr_at_10','ndcg_at_10',
                'negative_any_hit_rate','retrieval_p50_ms','retrieval_p95_ms','by_category']}
        for key in ['context-token','empty-context','retrieval','governance']:
            payload=result['governance'] if key=='governance' else {method:{k:value[k] for k in report_fields[key]} for method,value in result['methods'].items()}
            write_json(out/(key+'-summary.json'),payload)
            (out/(key+'-report.md')).write_text('# '+key+'（待人工确认的工程结果）\n\n非公开Benchmark、非生产HTTP成绩；收费API为0。分母、计数范围及方法见protocol.json。\n\n```json\n'+json.dumps(payload,ensure_ascii=False,indent=2)+'\n```\n')
    except Exception as e:
        manifest.update(status='failed',error_type=type(e).__name__);raise
    finally:
        write_json(out/'manifest.json',manifest);write_json(out/'failures.json',[r for r in rows if r['error']]+[r for r in governance if not r['passed']])
        write_csv(out/'retrieval.csv',[{**{k:v for k,v in r.items() if not isinstance(v,(dict,list))},**(r.get('token_metrics') or {})} for r in rows])
        write_csv(out/'governance.csv',governance)
    return result


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('mode',choices=['prepare','run']);p.add_argument('--output',required=True)
    p.add_argument('--assets',default='/tmp/drb-phase3-assets');args=p.parse_args()
    if args.mode=='prepare':prepare(Path(args.output))
    else:run(Path(args.output),args.assets)
    print(Path(args.output).resolve())


if __name__=='__main__':main()
