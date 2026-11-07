"""锁定官方子集；预测输入与参考答案/has_answer 只在入口拆分一次。"""
import hashlib
import json
from pathlib import Path
import re

from .longmemeval import timestamp
from .protocol3 import digest

SOURCE_SHA='d6f21ea9d60a0d56f34a05b609c79c88a451d2ae03597821ea3d5a9678c3a442'


def source_rows(path):
    raw=Path(path).read_bytes()
    if hashlib.sha256(raw).hexdigest()!=SOURCE_SHA:
        raise ValueError('source SHA differs from preregistered LongMemEval-S')
    return json.loads(raw)


def make_lock(rows,dev_ids,seed,per_stratum):
    dev=[r for r in rows if r['question_id'] in dev_ids]
    dev_evidence={s for r in dev for s in r['answer_session_ids']}
    dev_sessions={s for r in dev for s in r['haystack_session_ids']}
    groups={}
    for row in rows:
        if row['question_id'] in dev_ids or set(row['haystack_session_ids'])&dev_sessions:continue
        group='abstention' if '_abs' in row['question_id'] else row['question_type']
        groups.setdefault(group,[]).append(row)
    selected=[]
    for group,items in sorted(groups.items()):
        if len(items)<per_stratum:raise ValueError('not enough disjoint cases')
        ordered=sorted(items,key=lambda r:digest([seed,r['question_id']]))[:per_stratum]
        selected.extend({'id':r['question_id'],'stratum':group,'question_type':r['question_type']} for r in ordered)
    ids={r['id'] for r in selected}
    held=[r for r in rows if r['question_id'] in ids]
    value={'version':'longmemeval-s-heldout-phase3-v1','kind':'public_subset_not_official_full_benchmark',
           'source_sha256':SOURCE_SHA,'source_url':'https://huggingface.co/datasets/xiaowu0162/longmemeval-cleaned/resolve/main/longmemeval_s_cleaned.json',
           'seed':seed,'per_stratum':per_stratum,'dev_ids':sorted(dev_ids),'cases':selected,
           'id_overlap':len(ids&set(dev_ids)),
           'answer_session_overlap':len({s for r in held for s in r['answer_session_ids']}&dev_evidence),
           'all_history_session_overlap':len({s for r in held for s in r['haystack_session_ids']}&dev_sessions),
           'selection':'SHA256(seed,question_id) within strata; exclude dev IDs and ALL dev history sessions; no answer/success filtering',
           'preprocessing':'all roles, chronological; whitespace collapse, 1000-char no-overlap chunks; UTC assumption',
           'evidence_annotation':'upstream has_answer; chunk-level proxy; manual fact review pending'}
    return {**value,'seal':digest(value)}


def load_cases(rows,lock):
    unsigned={k:v for k,v in lock.items() if k!='seal'}
    if (digest(unsigned)!=lock['seal'] or lock['source_sha256']!=SOURCE_SHA or lock['id_overlap']
        or lock['answer_session_overlap'] or lock['all_history_session_overlap']):
        raise ValueError('invalid or overlapping lock')
    lookup={r['question_id']:r for r in rows};cases=[]
    for entry in lock['cases']:
        r=lookup[entry['id']];docs=[];evidence={}
        for date,sid,turns in sorted(zip(r['haystack_dates'],r['haystack_session_ids'],r['haystack_sessions'])):
            for t,turn in enumerate(turns):
                text=re.sub(r'\s+',' ',turn['content']).strip()
                for c,start in enumerate(range(0,len(text),1000)):
                    body=text[start:start+1000].strip()
                    if not body:continue
                    did=f'{sid}:{t}:{c}'
                    docs.append({'id':did,'session_id':sid,'content':f"[{date}] {turn['role']}: {body}",'occurred_at':timestamp(date)})
                    if turn.get('has_answer') and '_abs' not in r['question_id']:evidence[did]=1
        negative='_abs' in r['question_id']
        cases.append({'input':{'id':r['question_id'],'query':r['question'],'question_date':r['question_date'],'documents':docs},
                      'labels':{'answers':[str(r['answer'])],'unanswerable':negative,'question_type':r['question_type'],
                                'session_qrels':{} if negative else {s:1 for s in r['answer_session_ids']},'evidence_qrels':evidence}})
    return cases
