"""免费本地语义模型的工程留出集对照；不读取锁定的公开测试题。"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import random
import subprocess
import time

from .baselines import TfidfIndex
from .metrics import mean,percentile,retrieval_metrics
from .models3 import DenseIndex,SemanticClient,TextTokenizer
from .phase3 import load_configuration
from .runner import ROOT,TARGET,Sandbox,command,corpus,retrieve,write_json


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--output',required=True)
    p.add_argument('--assets',default='/tmp/drb-phase3-assets');p.add_argument('--url',default='http://127.0.0.1:19882')
    args=p.parse_args();out=Path(args.output)
    if out.exists():p.error('new output required')
    out.mkdir(parents=True);protocol,models,_=load_configuration()
    entry=models['embedding'];tokenizer=TextTokenizer(Path(args.assets)/'Qwen3-Embedding-0.6B/tokenizer.json',entry['files']['tokenizer.json']['sha256'],add_special_tokens=True)
    data=json.loads(Path('evals/datasets/holdout_v2.json').read_text());rows=[]
    manifest={'status':'running','kind':'self_constructed_engineering_holdout_real_embedding_not_public_benchmark',
              'dataset_sha256':hashlib.sha256(Path('evals/datasets/holdout_v2.json').read_bytes()).hexdigest(),
              'paid_api_calls':0,'actual_cost_cny':0,'reader':'not run','semantic_model':models['embedding'],'parameters':models['embedding_parameters']}
    write_json(out/'manifest.json',manifest)
    try:
        client=SemanticClient(args.url,models,tokenizer,out/'embedding-cache');manifest['health']=client.health
        subprocess.run(['cargo','build','--locked','--offline','--manifest-path',str(ROOT/'evals/harness/Cargo.toml')],
                       env={**os.environ,'CARGO_TARGET_DIR':str(TARGET)},check=True,timeout=600)
        with (out/'bridge.log').open('w') as log,Sandbox() as sb:
            sb.start(log);hits,docs,ingestion=corpus(sb,'semantic3',data['documents'])
            index_start=time.perf_counter_ns();dense=DenseIndex(docs,client)
            manifest['semantic_index_ms']=(time.perf_counter_ns()-index_start)/1e6
            tf=TfidfIndex(docs);reverse={h['memory_version_id']:d for d,h in hits.items()};rng=random.Random(protocol['seed'])
            for query in data['queries']:
                start=time.perf_counter_ns();client.embed([query['query']],'query');query_ms=(time.perf_counter_ns()-start)/1e6
                methods=['bm25','tfidf','semantic','hybrid_semantic'];rng.shuffle(methods)
                for method in methods:
                    alias={'tfidf':'vector_tfidf','semantic':'vector_tfidf','hybrid_semantic':'hybrid','bm25':'bm25'}[method]
                    result=retrieve(sb,'semantic3',query['query'],dense if method in {'semantic','hybrid_semantic'} else tf,hits,alias,20)
                    ranked=[reverse[h['memory_version_id']] for h in result['hits']]
                    rows.append({'id':query['id'],'query':query['query'],'tag':query['tag'],'method':method,'ranked':ranked,
                                 'qrels':query['qrels'],'error':result.get('error'),'retrieval_ms_excluding_embedding':result['latency_ms'],
                                 'query_embedding_ms_shared':query_ms if method in {'semantic','hybrid_semantic'} else 0,
                                 'negative_any_hit':not query['qrels'] and bool(ranked),**retrieval_metrics(ranked,query['qrels'])})
                    write_json(out/'questions.json',rows)
            manifest.update(status='completed',ingestion=ingestion,local_embedding_http_requests=client.calls,
                            input_texts_encoded=client.texts,postgres_image_id=sb.image)
    except Exception as e:
        manifest.update(status='failed',error_type=type(e).__name__);raise
    finally:write_json(out/'manifest.json',manifest)
    summary={}
    for method in ['bm25','tfidf','semantic','hybrid_semantic']:
        group=[r for r in rows if r['method']==method]
        summary[method]={**{k:mean([r[k] for r in group]) for k in ['recall_at_1','recall_at_5','mrr_at_10','ndcg_at_10']},
                         'positive_n':20,'negative_n':4,'errors':sum(bool(r['error']) for r in group),
                         'retrieval_p50_ms_excluding_embedding':percentile([r['retrieval_ms_excluding_embedding'] for r in group],.5),
                         'retrieval_p95_ms_excluding_embedding':percentile([r['retrieval_ms_excluding_embedding'] for r in group],.95),
                         'negative_any_hit_rate':mean([r['negative_any_hit'] for r in group if not r['qrels']]),
                         'query_embedding_p50_ms_shared':percentile([r['query_embedding_ms_shared'] for r in group],.5)}
    write_json(out/'summary.json',summary)
    (out/'report.md').write_text('# 本地真实语义检索工程留出集\n\n195文档、20正例+4负例；已用过的开发留出集，非独立公开测试集。未运行真实Reader、Reranker或Judge。\n\n```json\n'+json.dumps(summary,indent=2)+'\n```\n')
    print(out.resolve())


if __name__=='__main__':main()
