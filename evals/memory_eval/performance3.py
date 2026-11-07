"""固定环境性能复核：PG重启首次请求与热池分开，非操作系统冷缓存。"""
import argparse
import json
import os
from pathlib import Path
import random
import subprocess
import time
import uuid

from .longmemeval import adapt
from .metrics import percentile
from .phase2 import stats
from .runner import ROOT,TARGET,Sandbox,command,corpus,write_json


class RestartableSandbox(Sandbox):
    def __enter__(self):
        # 匿名卷仅供本夹具；tmpfs 在容器重启后丢失，不能用于此冷启动对照。
        self.name='drb-eval-'+uuid.uuid4().hex[:12];self.database='drb_eval_'+uuid.uuid4().hex[:12];self.process=None
        self.image=command(['docker','image','inspect','postgres:18.4-bookworm','--format','{{.Id}}'])
        command(['docker','run','-d','--name',self.name,'--label','io.drb.eval=true','-v','/var/lib/postgresql',
                 '-p','127.0.0.1::5432','-e','POSTGRES_HOST_AUTH_METHOD=trust',self.image,'-c','shared_preload_libraries=pg_stat_statements'])
        return self


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output',required=True);parser.add_argument('--longmemeval',default='/tmp/drb-eval-public/longmemeval_s_cleaned.json')
    parser.add_argument('--startup-only',action='store_true')
    args=parser.parse_args();out=Path(args.output)
    if out.exists():parser.error('output must be new')
    out.mkdir(parents=True);rows=[];startups=[]
    manifest={'status':'running','kind':'local_performance_fixture_not_public_score','seed':20261108,'model_api_calls':0,
              'cold_definition':'first probe after PG/container and bridge restart; OS page cache uncontrolled; volume preserved',
              'warm_per_length':40,'cold_per_length':6,'timeout_ms':3000,'git_head':command(['git','rev-parse','HEAD']),
              'source_sha256':{str(p):__import__('hashlib').sha256(p.read_bytes()).hexdigest() for p in
                               [Path(__file__),ROOT/'evals/harness/src/main.rs',ROOT/'crates/memory-postgres/src/repository.rs']}}
    write_json(out/'manifest.json',manifest)
    data=adapt(args.longmemeval)
    inp=next(c['input'] for c in data['cases'] if c['input']['id']=='c6853660')
    subprocess.run(['cargo','build','--locked','--offline','--manifest-path',str(ROOT/'evals/harness/Cargo.toml')],
                   env={**os.environ,'CARGO_TARGET_DIR':str(TARGET)},check=True,timeout=600)
    if args.startup_only:
        manifest.update(kind='bridge_tokenizer_startup_attribution',cold_definition='bridge only restarted; same warm database')
        with (out/'bridge.log').open('w') as log,Sandbox() as sb:
            sb.start(log);corpus(sb,'startup',[{'id':'d','content':'coffee limit is two cups'}])
            for repeat in range(6):
                sb.process.stdin.close();sb.process.terminate();sb.process.wait(timeout=10)
                started=time.perf_counter_ns()
                sb.process=subprocess.Popen([str(TARGET/'debug/memory-eval-bridge')],stdin=subprocess.PIPE,stdout=subprocess.PIPE,
                    stderr=log,text=True,bufsize=1,env={'PATH':os.environ['PATH'],'EVAL_DATABASE_URL':sb.url})
                assert sb.receive()=={'ready':True}
                startup_ms=(time.perf_counter_ns()-started)/1e6
                for state in ['bridge_first','bridge_second']:
                    started=time.perf_counter_ns()
                    result=sb.call('sql_probe','startup',{'variant':'version_lengths','query':'coffee'})
                    rows.append({'repeat':repeat,'state':state,'bridge_connect_migrate_ms':startup_ms,
                                 'request_ms':(time.perf_counter_ns()-started)/1e6,**result})
            write_json(out/'measurements.json',rows)
        summary=[{'state':state,'n':len(g),'request_p50_ms':percentile([r['request_ms'] for r in g],.5),
                  'tokenize_p50_ms':percentile([r['tokenize_ms'] for r in g],.5),
                  'sql_p50_ms':percentile([r['sql_ms'] for r in g],.5)}
                 for state in ['bridge_first','bridge_second'] if (g:=[r for r in rows if r['state']==state])]
        write_json(out/'summary.json',summary);manifest['status']='completed';write_json(out/'manifest.json',manifest)
        print(out.resolve());return
    rng=random.Random(20261108)
    try:
        with (out/'bridge.log').open('w') as log:
            for length in [100,400,801]:
                with RestartableSandbox() as sb:
                    sb.start(log);manifest['postgres_image_id']=sb.image
                    corpus(sb,'phase3-perf',inp['documents'][:length],True)
                    command(['docker','exec',sb.name,'psql','-U','postgres','-d',sb.database,'-c','ANALYZE memory_search.retrieval_documents; ANALYZE memory_search.retrieval_terms'])
                    # 两种查询各预热一次，之后40次只测固定的最终SQL。
                    for query in ['coffee',inp['query']]:sb.call('sql_probe','phase3-perf',{'variant':'version_lengths','query':query})
                    warm=['coffee',inp['query']]*20;rng.shuffle(warm)
                    for state,queries in [('warm',warm),('pg_restart_first',['coffee',inp['query']]*3)]:
                        for repeat,query in enumerate(queries):
                            if state=='pg_restart_first':
                                begin=time.perf_counter_ns()
                                sb.process.stdin.close();sb.process.terminate();sb.process.wait(timeout=10)
                                command(['docker','restart',sb.name])
                                for _ in range(60):
                                    if subprocess.run(['docker','exec',sb.name,'pg_isready','-h','127.0.0.1','-U','postgres'],capture_output=True).returncode==0:break
                                    time.sleep(.2)
                                else:raise RuntimeError('restart readiness timeout')
                                # 随机映射的宿主端口可能随Docker重启改变，不能沿用旧连接地址。
                                port=command(['docker','port',sb.name,'5432/tcp']).rsplit(':',1)[1]
                                sb.url=f'postgres://drb_eval@127.0.0.1:{port}/{sb.database}'
                                sb.process=subprocess.Popen([str(TARGET/'debug/memory-eval-bridge')],stdin=subprocess.PIPE,stdout=subprocess.PIPE,
                                    stderr=log,text=True,bufsize=1,env={'PATH':os.environ['PATH'],'EVAL_DATABASE_URL':sb.url})
                                assert sb.receive()=={'ready':True}
                                startups.append({'length':length,'repeat':repeat,'restart_connect_migrate_ms':(time.perf_counter_ns()-begin)/1e6})
                            before=stats(sb);begin=time.perf_counter_ns()
                            result=sb.call('sql_probe','phase3-perf',{'variant':'version_lengths','query':query,'plan_mode':'auto'})
                            elapsed=(time.perf_counter_ns()-begin)/1e6;after=stats(sb)
                            rows.append({'length':length,'state':state,'repeat':repeat,'query':query,'request_ms':elapsed,
                                         'sql_calls':after['calls']-before['calls'],**result})
                            write_json(out/'measurements.json',rows);write_json(out/'startup.json',startups)
                    manifest.setdefault('resources',[]).append({'length':length,'memory_sample':command(['docker','stats','--no-stream',sb.name,'--format','{{.MemUsage}}'])})
        manifest['status']='completed'
    except Exception as e:
        manifest.update(status='failed',error_type=type(e).__name__);raise
    finally:write_json(out/'manifest.json',manifest)
    summary=[]
    for length in [100,400,801]:
        for state in ['warm','pg_restart_first']:
            group=[r for r in rows if r['length']==length and r['state']==state]
            summary.append({'length':length,'state':state,'n':len(group),'timeouts':sum(bool(r.get('error')) for r in group),
                            'p50_ms':percentile([r['request_ms'] for r in group],.5),'p95_ms':percentile([r['request_ms'] for r in group],.95),
                            'min_ms':min(r['request_ms'] for r in group),'max_ms':max(r['request_ms'] for r in group),
                            'sql_p50_ms':percentile([r['sql_ms'] for r in group],.5),'sql_p95_ms':percentile([r['sql_ms'] for r in group],.95),
                            'sql_calls':sorted({r['sql_calls'] for r in group})})
    write_json(out/'summary.json',summary)
    (out/'report.md').write_text('# 第三阶段性能复核\n\n重启首次与热池分组；不是OS物理冷盘或生产SLO。\n\n```json\n'+json.dumps(summary,indent=2)+'\n```\n')
    print(out.resolve())


if __name__=='__main__':main()
