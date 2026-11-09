"""本次运行的离线复核与报告重建；不发模型请求、不写原始预测或标签。"""
from pathlib import Path
from collections import Counter
import csv
import hashlib
import json
import math
import shutil
import tokenizers

ROOT=Path.cwd();OUT=Path(__file__).resolve().parent
rows=json.loads((OUT/'retrieval.json').read_text());gov=json.loads((OUT/'governance.json').read_text())
summary=json.loads((OUT/'summary.json').read_text());manifest=json.loads((OUT/'manifest.json').read_text())
data=json.loads((ROOT/'evals/datasets/context_quality_v1.json').read_text());queries={q['id']:q for q in data['queries']}
methods=['bm25','vector_tfidf','hybrid','hybrid_rerank']
labels={'bm25':'BM25','vector_tfidf':'TF-IDF','hybrid':'Hybrid（TF-IDF）','hybrid_rerank':'Hybrid＋透明重排'}
assert manifest['status']=='completed' and len(rows)==192 and len(gov)==24
assert {r['id'] for r in gov}=={f'g{i:02}' for i in range(1,25)}
assert tokenizers.__version__=='0.22.1'
raw=ROOT/'evals/configs/memory_quality_v1.json';protocol=json.loads(raw.read_text())
assert protocol==manifest['protocol']
for p,h in manifest['dataset_sha256'].items():assert hashlib.sha256((ROOT/p).read_bytes()).hexdigest()==h
for p,h in manifest['source_sha256'].items():assert hashlib.sha256((ROOT/p).read_bytes()).hexdigest()==h
sealed=json.loads((ROOT/'evals/reports/memory-quality-v1-review-final/hashes.json').read_text())
assert all(hashlib.sha256((ROOT/p).read_bytes()).hexdigest()==h for p,h in sealed.items())
asset=Path('/tmp/drb-phase3-assets/DeepSeek-V3.2/tokenizer.json')
assert hashlib.sha256(asset.read_bytes()).hexdigest()==protocol['tokenizer']['sha256']
tok=tokenizers.Tokenizer.from_file(str(asset));counts={}
for r in rows:
 assert not r['error']
 for key in ['full','pack']:
  text=r['token_metrics'][key+'_rendered']
  if text not in counts:counts[text]=len(tok.encode(text,add_special_tokens=False).ids)
  assert counts[text]==r['token_metrics'][key+'_tokens']
for method in methods:
 group=[r for r in rows if r['method']==method];positive=[r for r in group if not r['unanswerable']]
 assert len(group)==48 and {r['id'] for r in group}==set(queries)
 recalculated=[]
 for r in positive:
  ranked=list(dict.fromkeys(r['ranked']));qrels=r['qrels'];gold={k for k,v in qrels.items() if v>0}
  ideal=sorted(qrels.values(),reverse=True)[:10]
  dcg=sum((2**qrels.get(doc,0)-1)/math.log2(i+2) for i,doc in enumerate(ranked[:10]))
  idcg=sum((2**g-1)/math.log2(i+2) for i,g in enumerate(ideal))
  recalculated.append({'recall_at_1':len(set(ranked[:1])&gold)/len(gold),'recall_at_5':len(set(ranked[:5])&gold)/len(gold),
      'mrr_at_10':next((1/(i+1) for i,d in enumerate(ranked[:10]) if d in gold),0),'ndcg_at_10':dcg/idcg})
 for key in recalculated[0]:assert math.isclose(sum(x[key] for x in recalculated)/42,summary['methods'][method][key],abs_tol=1e-12)
 assert all(r['full_evidence_retention']['all_evidence_delivered'] for r in positive)
assert sum(r['passed'] for r in gov)==22
assert {r['id'] for r in gov if not r['passed']}=={'g03','g06'}
validation={'status':'passed','retrieval_records':192,'governance_records':24,'request_errors':0,
 'independent_retrieval_metric_recalculation':True,'exact_token_recount_queries':192,'unique_payload_tokenizations':len(counts),
 'full_baseline_all_evidence_verified':'42/42 for each method','reviewed_hashes_match':True,
 'raw_human_review_metadata_preserved':'pending','paid_api_calls':0}
(OUT/'validation.json').write_text(json.dumps(validation,ensure_ascii=False,indent=2)+'\n')
# 原始运行遗漏单独协议文件，但manifest已含完整协议；这里只补齐报告引用，不改结果或代码。
(OUT/'protocol.json').write_bytes(raw.read_bytes())
frozen=OUT/'frozen-inputs';frozen.mkdir(exist_ok=True)
for p in manifest['dataset_sha256']:shutil.copyfile(ROOT/p,frozen/Path(p).name)
(OUT/'execution-note.json').write_text(json.dumps({'date':'2026-11-09','user_instruction':'执行',
 'scope':'run current frozen 48-query four-method and 24-governance free evaluation',
 'annotation_review':'execution authorized against current labels; pending source metadata retained; no invented item-by-item review record',
 'paid_api_calls':0,'paid_claim_created':False,'production_database_used':False},ensure_ascii=False,indent=2)+'\n')
category=[];token_cohorts=[];diagnostics=[]
for method in methods:
 s=summary['methods'][method]
 for cat,c in s['by_category'].items():category.append({'method':method,'category':cat,**c})
 for cohort in ['all_48','answerable_42']:
  group=[r for r in rows if r['method']==method and (cohort=='all_48' or not r['unanswerable'])]
  a=sum(r['token_metrics']['full_tokens'] for r in group);b=sum(r['token_metrics']['pack_tokens'] for r in group)
  token_cohorts.append({'method':method,'cohort':cohort,'n':len(group),
   'full_memory_count_mean':sum(r['token_metrics']['full_memory_count'] for r in group)/len(group),
   'pack_memory_count_mean':sum(r['token_metrics']['pack_memory_count'] for r in group)/len(group),
   'full_tokens_mean':a/len(group),'pack_tokens_mean':b/len(group),'saving_ratio':1-b/a})
 for r in rows:
  if r['method']==method and not r['unanswerable'] and not r['all_evidence_delivered']:
   diagnostics.append({'method':method,'id':r['id'],'query':r['input_query'],'first_loss':r['first_loss'],
    'literal_empty':r['literal_context_empty'],'evidence_delivered':r['evidence_units_delivered'],'evidence_total':r['evidence_units'],
    'raw_row_index':rows.index(r),'category':r['category']})
for name,values in [('category-metrics.csv',category),('token-cohorts.csv',token_cohorts),('diagnostic-failures.csv',diagnostics)]:
 with (OUT/name).open('w',newline='',encoding='utf-8') as f:
  writer=csv.DictWriter(f,fieldnames=list(values[0]));writer.writeheader();writer.writerows(values)
(OUT/'diagnostic-failures.json').write_text(json.dumps(diagnostics,ensure_ascii=False,indent=2)+'\n')
# 对已保存候选做事后公式核对，绝不改变排名、Prompt、标签或阈值。
def relevance(query,content,score):
 q=[c for c in query.lower() if c.isalnum()];d=[c for c in content.lower() if c.isalnum()]
 qg={tuple(q[i:i+2]) for i in range(len(q)-1)};dg={tuple(d[i:i+2]) for i in range(len(d)-1)}
 overlap=len(qg&dg)/min(len(qg),len(dg)) if qg and dg else 0
 recall=len(set(q)&set(d))/len(set(q)) if q else 0
 exact=float(bool(q) and ''.join(q) in ''.join(d))
 lexical=.55*overlap+.3*recall+.15*exact
 return {'lexical_relevance':lexical,'input_retrieval_score':score,'transparent_relevance':.65*lexical+.35*max(0,min(score,1))}
case_dir=OUT/'failure-cases';case_dir.mkdir(exist_ok=True);score_checks=[]
base=json.loads((ROOT/data['base_dataset']['path']).read_text())
by_content={d['content']:d['id'] for d in base['documents']+data['documents']}
for qid in ['q07','q14','q20']:
 for method in methods:
  r=next(x for x in rows if x['id']==qid and x['method']==method)
  input_stage='preauthorized' if method=='hybrid_rerank' else 'postauthorized'
  for mem in r['retrieval_trace'][input_stage]:
   # 采用实际调用透明重排之前的分数，不能再次把已重排分数当作原始RRF分数。
   document=by_content[mem['content']]
   if document not in r['qrels']:continue
   score_checks.append({'id':qid,'method':method,'document':document,'query':r['input_query'],'content':mem['content'],
     **relevance(r['input_query'],mem['content'],mem['score']),'score_input_stage':input_stage,
     'retained_for_context':document in r['stages']['context_relevance'],
     'note':'posthoc implementation-formula audit; not rerun or relabel; categorical applicability exceptions not modeled'})
  (case_dir/(method+'-'+qid+'.json')).write_text(json.dumps({'input':queries[qid],'actual_trace':r},ensure_ascii=False,indent=2)+'\n')
assert all((x['transparent_relevance']>=.4)==x['retained_for_context'] for x in score_checks)
(OUT/'relevance-score-audit.json').write_text(json.dumps(score_checks,ensure_ascii=False,indent=2)+'\n')
lines=['# 长期记忆质量评测实测报告（2026-11-09）','',
 '用户授权按当前固定标注执行；原始pending字段保持不变，不捏造逐项人工审核记录。全部为自建工程集，不是公开Benchmark或生产环境成绩。',
 '', '240条记忆（原195＋新增45）、48道查询（42正例＋6无答案），四组共192条检索记录；24个治理场景。请求错误0、收费API调用0、未运行Reader/真实语义模型/神经Reranker。','',
 '## 检索与上下文','',
 '| 方法 | Recall@1 | Recall@5 | MRR@10 | nDCG@10 | P50/P95 ms | 字面空包 | 有效证据为空 | 必要证据全部送达 |',
 '|---|---:|---:|---:|---:|---:|---:|---:|---:|']
for method in methods:
 s=summary['methods'][method];group=[r for r in rows if r['method']==method and not r['unanswerable']]
 lines.append(f"| {labels[method]} | {s['recall_at_1']:.4f} | {s['recall_at_5']:.4f} | {s['mrr_at_10']:.4f} | {s['ndcg_at_10']:.4f} | {s['retrieval_p50_ms']:.2f}/{s['retrieval_p95_ms']:.2f} | {sum(r['literal_context_empty'] for r in group)}/42 | {sum(r['effective_context_empty'] for r in group)}/42 | {sum(r['all_evidence_delivered'] for r in group)}/42 |")
lines += ['', '排名指标分母为42正例；延迟每组48个单次样本，包括负例，未做重复测量。延迟包含IPC、查询向量化（TF-IDF）、PG搜索、融合/授权；Hybrid+透明重排的检索计时含重排，其他组进入Context前的附加重排不在该计时内。因此不能从这张表推导完整Context链路延迟或总Rerank增量成本。',
 '', '所有正例在源候选中都至少命中一个有效证据，RRF和授权阶段整题证据丢失0/42。BM25在context_relevance丢失1/42；TF-IDF丢失26/42，另有1题只保留部分多跳证据。Hybrid两组均42/42送达全部标注证据。本轮没有发生预算阶段新增的整题证据丢失；不能据此证明长文/紧预算总能保留证据。',
 '', '## Context Pack Token效率','',
 '固定DeepSeek-V3.2 tokenizer revision a7e62ac04ecb2c0a54d736dc46601c5606cf10a6，tokenizers==0.22.1，add_special_tokens=false。精确计数包含统一记忆封装和版本标识，不是实际模型usage或完整请求Token。',
 '', '| 方法 | 全量平均条数→按需平均条数 | 全量平均Token→按需平均Token | 按需P50/P95 Token | 节省比例 | 正例全证据送达 |',
 '|---|---:|---:|---:|---:|---:|']
for method in methods:
 s=summary['methods'][method]
 lines.append(f"| {labels[method]} | {s['full_memory_count_mean']:.0f}→{s['pack_memory_count_mean']:.2f} | {s['full_tokens_mean']:.0f}→{s['pack_tokens_mean']:.2f} | {s['pack_tokens_p50']:.1f}/{s['pack_tokens_p95']:.1f} | {s['token_saving_micro']:.2%} | {s['all_evidence_delivery_rate']:.2%} |")
lines += ['', '本表Token统计分母为48题，证据覆盖分母42正例；全量Token的P50/P95均为12489。`token-cohorts.csv`另存同一42正例口径：Hybrid平均240→3.76条、12489→179.95 Token，节省98.56%，必要证据42/42全部送达。',
 '', '**不能把TF-IDF的99.49%节省解释为最好**：26/42正例空包造成大量零Token。BM25也有非空却没有有效证据的q14。节省率必须和证据覆盖一起解释。语料中含大量人工运维干扰记录，此比例不能外推生产收益。',
 '', '## 失败定位','',
 '- q07“山雀在什么年月日正式上线”：TF-IDF将正确date-a排第1，最终Context为空；原cosine=0.302544，透明相关性分数=0.284640，低于0.4。BM25授权后的检索分数=1，透明分数=0.528750，保留。',
 '- q14“谁上班时不方便听提示音，需要用字来通知”：BM25正确para-b排第7，透明分数0.163073，Context只保留另一人的hearing记录，字面非空但有效证据为空。Hybrid把para-b排第6，归一化融合分数0.910448令透明分数为0.404686，进入Context前重排后保留；Hybrid+透明重排的检索排名为第2。',
 '- q20“雾松负责人的办公分机”：TF-IDF已召回负责人关系和分机两个证据，但Context只保留分机相关块，缺少项目→负责人的关联。这说明“有一条相关记忆”不代表多跳证据完整。',
 '', '诊断定位到现有透明重排/相关性分数合同：BM25原分值在canonical授权时clamp到1，TF-IDF cosine往往显著小于1，RRF则是归一化融合分值。相同0.65词法＋0.35检索分数公式以及0.4阈值，对不同来源产生不同筛选结果。这里报告代码与Trace证据，不将其冒充真实语义模型结果，也不在本次测试集上调阈值、权重或规则。',
 '', '## 记忆治理','', '| 类别 | 符合预期场景 | 准确率 |', '|---|---:|---:|']
for family,s in summary['governance'].items():lines.append(f"| {family} | {s['passed']}/{s['n']} | {s['accuracy']:.2%} |")
lines += ['', '共22/24场景符合预期。重复识别标签2/3（TP1/TN1/FN1），冲突识别标签2/3（TP1/TN1/FP1）；显式纠正与审核版本替代合计6/6，遗忘3/3、Scope3/3、来源追溯3/3、旧候选/旧Pack拦截3/3。每类仅3例，不能宣传普遍100%正确。',
 '', '- g03：“我喜欢骑自行车”与“骑自行车是我的爱好”未被视为重复，Head进入contested。',
 '- g06：“我偏好文字通知”与“通知请用文字形式”被误判为冲突。',
 '', '这两项是当前基于语义键/正文Hash治理的语义等价能力缺口；fail-closed隔离按现有规则工作，但不等于去重/冲突识别质量通过。没有修改标签、治理规则或生产代码。直接提交Candidate/Correction不证明自然语言Formation/纠错识别能力；逻辑遗忘不代表WAL/备份/外部副本物理擦除。',
 '', '## 无答案、限制和下一步','',
 '四组对6/6无答案题都返回检索候选。BM25/Hybrid两组的6/6还注入了非空Context，TF-IDF为1/6；不等于已生成幻觉，因为本轮没有Reader，也不能宣称拒答成功。真实Embedding和神经Reranker新集对照尚未执行；历史195文档Qwen3成绩必须另表。',
 '', '本轮为同作者自建集，非独立团队盲测；多轮是显式历史拼接；时间冲突检索为raw-history；只有governed Repository与本地投影回执，没有Mem0/ES HTTP或生产ANN。单次查询延迟无统计显著性保证。',
 '', '下一步建议先在独立开发数据上审查不同检索分数与相关性闸门的合同，再另行设计对照；不要利用本次48题调参后继续称其独立测试。语义去重应独立评估规范化/等价识别，保留来源和人工审核路径。本轮只报告，不修改生产实现。',
 '', '## 复现与证据','',
 '```bash', 'scripts/eval-memory-quality.sh run evals/runs/memory-quality-reproduce-new',
 '# 结果目录保留逐题retrieval/governance JSON和CSV；模型API调用0。',
 '# 本次派生报告可在仓库根目录重新构建：',
 'PYTHONPATH=/tmp/drb-phase3-deps python3 -B '+str(OUT.relative_to(ROOT))+'/build_report.py','```','',
 '原始文件：manifest.json、summary.json、retrieval.json/csv、governance.json/csv、failures.json、四类自动报告与JSON汇总、bridge.log。扩展文件：category-metrics.csv、token-cohorts.csv、diagnostic-failures.json/csv、failure-cases、relevance-score-audit.json、validation.json、frozen-inputs、protocol.json。原始预测、标注和pending元数据未改。',
 '', '上轮离线回归143项通过；本轮未改评测源码，完整运行后独立重算192条检索指标和Token，验证全部记录齐全。原有32个文件、历史资产、锁定集和收费账本的本轮保护核验另见protection-check.json。',
 '', '## 可用简历表述','',
 '1. 在Rust/PostgreSQL governed长期记忆模块构建自建240文档/48查询的分阶段评测；使用固定Tokenizer测得Hybrid在42正例中保留全部必要证据，平均记忆payload由12489降至179.95 Token，减少98.56%（非生产usage）。',
 '2. 在同一42正例上对比BM25、TF-IDF、固定RRF与透明重排：Hybrid+透明重排Recall@5=100%、MRR@10=0.9246；通过逐题Trace发现TF-IDF虽Recall@5=97.62%，仍有26/42题在相关性筛选阶段丢失全部有效证据。',
 '3. 构建24个版本/冲突/来源/遗忘/Scope场景，22例符合预期；明确纠正与版本替代6/6、旧候选及旧Pack拦截3/3，保留语义改写去重与误报冲突失败，未将其包装为全部治理通过。',
 '', '以上均须保留“自建工程集/本地隔离PG/小样本”边界，不写成公开Benchmark、生产SLO或真实LLM问答准确率。']
report='\n'.join(lines)+'\n'
(OUT/'report.md').write_text(report)
(ROOT/'docs/EVAL_MEMORY_QUALITY_RESULTS.md').write_text(report+'\n本次原始证据目录：[完整运行](../'+str(OUT.relative_to(ROOT))+'/report.md)。\n')
print(json.dumps(validation,ensure_ascii=False,indent=2))
