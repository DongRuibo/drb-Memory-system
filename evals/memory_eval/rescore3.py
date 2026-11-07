"""只重算既有开发诊断集的阶段指标，不预测锁定测试集，不调用模型。"""
import argparse
import json
from pathlib import Path

from .longmemeval import adapt
from .metrics import mean,percentile
from .models3 import TextTokenizer
from .protocol3 import answer_scores,stage_scores
from .runner import write_json


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--output',required=True)
    p.add_argument('--longmemeval',default='/tmp/drb-eval-public/longmemeval_s_cleaned.json')
    p.add_argument('--assets',default='/tmp/drb-phase3-assets');args=p.parse_args()
    out=Path(args.output)
    if out.exists():p.error('new output required')
    out.mkdir(parents=True)
    models=json.loads(Path('evals/configs/phase3_models.json').read_text());entry=models['reader_tokenizer']
    tokenizer=TextTokenizer(Path(args.assets)/'DeepSeek-V3.2/tokenizer.json',entry['files']['tokenizer.json']['sha256'])
    cases={c['input']['id']:c for c in adapt(args.longmemeval)['cases']}
    traces=json.loads(Path('evals/reports/phase2-longmemeval-final-v3/questions.json').read_text());rows=[]
    for r in traces:
        inp=cases[r['id']]['input']
        labels={'session_qrels':r['labels']['qrels'],'evidence_qrels':{} if r['labels']['unanswerable'] else {s:1 for s in r['official_evidence_chunk_ids']}}
        reverse={h['memory_version_id']:d['id'] for h,d in zip(r['retrieved_hits'],r['retrieved_documents'])}
        context={'items':[{'source_id':reverse[it['memory_version_id']],'text':it['text']} for sec in r['pack']['sections'] for it in sec['items']]}
        prediction={'answer':r['prediction']['text'],'abstained':r['prediction']['refused'],'citations':r['prediction']['citations']}
        rows.append({'id':r['id'],'method':r['method'],'unanswerable':r['labels']['unanswerable'],
                     'context_tokens_posthoc':tokenizer.count('\n'.join(it['text'] for it in context['items'])),
                     **stage_scores(r['retrieved_ids'],inp['documents'],labels,context),
                     **answer_scores(prediction,r['labels']['answers'],r['labels']['unanswerable']),
                     'latency_ms':r['retrieval_ms']})
    summary={}
    for method in ['bm25','vector_tfidf','hybrid','hybrid_rerank']:
        group=[r for r in rows if r['method']==method];pos=[r for r in group if not r['unanswerable']];neg=[r for r in group if r['unanswerable']]
        summary[method]={'n':len(group),'answerable_n':len(pos),'unanswerable_n':len(neg),
                         'session_recall_at_5':mean([r['session']['recall_at_5'] for r in pos]),
                         'evidence_chunk_recall_at_1':mean([r['evidence']['recall_at_1'] for r in pos]),
                         'evidence_chunk_recall_at_5':mean([r['evidence']['recall_at_5'] for r in pos]),
                         'evidence_chunk_recall_at_10':mean([min(len(set(t['retrieved_ids'][:10])&set(t['official_evidence_chunk_ids']))/len(t['official_evidence_chunk_ids']),1) for t in traces if t['method']==method and not t['labels']['unanswerable']]),
                         'context_evidence_char_coverage':mean([r['context_evidence_coverage'] for r in pos]),
                         'strict_answerable_accuracy':mean([r['strict_match'] for r in pos]),'normalized_answerable_accuracy':mean([r['normalized_match'] for r in pos]),
                         'abstention_accuracy':mean([r['abstention_correct'] for r in neg]),
                         'unsupported_answer_rate':None,'reason_unsupported_null':'No calibrated semantic support judge; not counted as zero',
                         'unanswerable_nonabstention_rate':mean([r['unanswerable_nonabstention'] for r in neg]),
                         'posthoc_context_tokens_p50':percentile([r['context_tokens_posthoc'] for r in group],.5)}
    write_json(out/'questions.json',rows);write_json(out/'summary.json',summary)
    write_json(out/'manifest.json',{'status':'completed','kind':'development_trace_rescore_not_new_test_or_real_model_results',
               'source':'evals/reports/phase2-longmemeval-final-v3/questions.json','tokenizer_sha256':tokenizer.sha,'api_calls':0,'cost_cny':0})
    (out/'report.md').write_text('# 开发诊断集分阶段复核\n\n官方has_answer块覆盖只是证据代理，不是人工事实召回。Token是事后计数，非当时模型usage。\n\n```json\n'+json.dumps(summary,indent=2)+'\n```\n')
    print(out.resolve())


if __name__=='__main__':main()
