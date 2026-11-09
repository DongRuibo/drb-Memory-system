"""有持久预算预留的顺序模型客户端；默认不获准访问收费端点。"""
from decimal import Decimal
import hashlib
import json
import math
import os
from pathlib import Path
import signal
import threading
import time
import urllib.error
import urllib.request
from urllib.parse import urlparse

from .protocol3 import digest


class BudgetStop(RuntimeError):
    pass


class Ledger:
    def __init__(self,path,limits):
        self.limits=limits;self.requests=0;self.dispatches=0;self.input=0;self.output=0;self.cost=Decimal(0);self.halted=False
        self.owner=(os.getpid(),threading.get_ident());self.pending=None
        self.role_counts={role:0 for role in limits['role_request_limits']}
        # 不自动重开或恢复含未结算请求的账本，避免进程重启后清空累计费用。
        self.file=Path(path).open('x',encoding='utf-8')
        self.log({'event':'budget_opened','limits':limits,'owner_pid':self.owner[0]})

    def check_owner(self):
        # 当前Runner只支持顺序执行；线程和fork子进程不能共享同一内存预算发请求。
        if (os.getpid(),threading.get_ident())!=self.owner:
            raise BudgetStop('parallel/thread/fork budget use is disabled; no request sent')

    def log(self,event):
        self.check_owner()
        try:
            self.file.write(json.dumps(event,ensure_ascii=False,allow_nan=False)+'\n');self.file.flush();os.fsync(self.file.fileno())
        except OSError:
            self.halted=True
            raise BudgetStop('budget persistence failed; stop without releasing reservations') from None

    def reserve(self,role,input_cap,output_cap):
        self.check_owner()
        c=(Decimal(input_cap)*Decimal(self.limits['price_cny_per_million_input'])+Decimal(output_cap)*Decimal(self.limits['price_cny_per_million_output']))/1000000
        if (self.halted or self.pending is not None or role not in self.role_counts
            or self.role_counts.get(role,0)>=self.limits['role_request_limits'].get(role,0)
            or self.requests>=self.limits['max_requests'] or self.input+input_cap>self.limits['max_input_tokens_total']
            or self.output+output_cap>self.limits['max_output_tokens_total'] or self.cost+c>Decimal(self.limits['max_cost_cny'])):
            self.halted=True
            raise BudgetStop('request/token/cost limit reached; no request sent')
        reservation={'request':self.requests+1,'role':role,'input_reserved':input_cap,'output_reserved':output_cap,'cost_reserved_cny':str(c)}
        # 预留先落盘再允许发送；磁盘失败不能消耗一笔未记录的调用。
        self.log({'event':'reserved',**reservation,'cumulative_reserved_or_spent_cny':str(self.cost+c)})
        self.requests+=1;self.input+=input_cap;self.output+=output_cap;self.cost+=c
        self.role_counts[role]+=1;self.pending=reservation
        return reservation

    def settle(self,reservation,usage,metadata):
        self.check_owner()
        if self.pending!=reservation:
            self.halted=True
            raise BudgetStop('duplicate or unknown settlement; no budget released')
        prompt=usage.get('prompt_tokens');completion=usage.get('completion_tokens')
        if (type(prompt) is not int or type(completion) is not int or prompt<0 or completion<0
            or prompt>reservation['input_reserved'] or completion>reservation['output_reserved']):
            self.halted=True;self.log({'event':'invalid_usage_stop','request':reservation['request'],'usage':usage})
            raise BudgetStop('usage missing or exceeds provider contract; reservation retained, stop')
        actual=(Decimal(prompt)*Decimal(self.limits['price_cny_per_million_input'])+Decimal(completion)*Decimal(self.limits['price_cny_per_million_output']))/1000000
        remaining=self.cost-Decimal(reservation['cost_reserved_cny'])+actual
        self.log({'event':'settled','request':reservation['request'],'usage':usage,'usage_list_price_cost_cny':str(actual),
                  'cumulative_reserved_or_spent_cny':str(remaining),**metadata})
        # 结算记录持久化之后才释放额度；崩溃/写失败时仍按最坏预留处理。
        self.input-=reservation['input_reserved']-prompt;self.output-=reservation['output_reserved']-completion
        self.cost=remaining;self.pending=None
        return actual


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self,*args,**kwargs):
        return None


def json_http(url,payload,timeout,key=None,capture_http_errors=False):
    # 顺序POSIX CLI用总时限中断慢速响应；urllib的socket timeout本身只限制单次阻塞。
    if threading.current_thread() is not threading.main_thread() or not hasattr(signal,'setitimer'):
        raise BudgetStop('real model HTTP requires the main POSIX thread; no request sent')
    if signal.getitimer(signal.ITIMER_REAL)[0]>0:
        raise BudgetStop('nested HTTP deadline is unsupported; no request sent')
    def expired(_signum,_frame):raise TimeoutError('model HTTP total deadline exceeded')
    headers={'Content-Type':'application/json'}
    if key:headers['Authorization']='Bearer '+key
    request=urllib.request.Request(url,data=json.dumps(payload).encode() if payload is not None else None,headers=headers)
    previous=signal.signal(signal.SIGALRM,expired);signal.setitimer(signal.ITIMER_REAL,timeout)
    try:
        try:response=urllib.request.build_opener(NoRedirect).open(request,timeout=timeout)
        except urllib.error.HTTPError as error:
            if not capture_http_errors:raise
            response=error
        with response:
            body=response.read(32000001)
        if len(body)>32000000:raise ValueError('model response too large')
        if capture_http_errors and response.code>=300:
            # 在同一个总时限内收集错误正文和供应商追踪ID，不记录任何认证请求头。
            return {'http_status':response.code,'http_error_body':body.decode('utf-8',errors='replace'),
                    'id':response.headers.get('x-siliconcloud-trace-id')}
        return json.loads(body)
    finally:
        signal.setitimer(signal.ITIMER_REAL,0);signal.signal(signal.SIGALRM,previous)


class ChatClient:
    def __init__(self,model,protocol,ledger,approved=False,transport=None):
        self.model=model;self.protocol=protocol;self.ledger=ledger;self.approved=approved
        self.transport=transport

    def call(self,messages,role,validator=None):
        self.ledger.check_owner()
        if not self.approved and self.transport is None:
            raise BudgetStop('paid execution requires explicit user approval; no credentials read')
        cap=self.protocol['reader_input_cap'] if role in {'reader','smoke'} else self.protocol['judge_input_cap']
        output=self.protocol['reader_output_cap'] if role in {'reader','smoke'} else self.protocol['judge_output_cap']
        # UTF-8 字节数是对该 byte-BPE 的保守界，另留每条消息128个封装Token；真实usage另记。
        bound=sum(len(m['content'].encode())+128 for m in messages)+128
        if bound>cap:raise BudgetStop('input bound exceeds fixed cap; no request sent')
        key=None
        if self.transport is None:
            if self.model['base_url']!='https://api.siliconflow.cn/v1':raise BudgetStop('unreviewed provider; no request sent')
            key=os.environ.get('MEMORY_EVAL_API_KEY')
            if not key:raise BudgetStop('MEMORY_EVAL_API_KEY missing after approval; no request sent')
        r=self.ledger.reserve(role,cap,output+self.protocol['reasoning_reserve'])
        payload={'model':self.model['model'],'messages':messages,'temperature':0,'stream':False,'max_tokens':output,
                 'enable_thinking':False,'thinking_budget':128,'response_format':{'type':'json_object'}}
        # 完整正文先落盘，不保存Authorization；即使解析或传输失败也能追溯实际输入。
        self.ledger.log({'event':'model_input','request':r['request'],'payload':payload,'input_sha256':digest(messages)})
        start=time.perf_counter_ns()
        try:
            if self.transport is None:
                self.ledger.dispatches+=1
                response=json_http(self.model['base_url']+'/chat/completions',payload,self.protocol['timeout_seconds'],key,capture_http_errors=True)
            else:
                self.ledger.dispatches+=1;response=self.transport(payload,self.protocol['timeout_seconds'])
        except Exception as error:
            # 传输失败仍可能已计费，保留最坏预留并停止；不记录可能含凭据的异常正文。
            self.ledger.halted=True;self.ledger.log({'event':'transport_failed_stop','request':r['request'],'error_type':type(error).__name__})
            raise BudgetStop('transport failed; reservation retained; retry=0') from None
        elapsed=(time.perf_counter_ns()-start)/1e6
        raw=response if isinstance(response,dict) else {}
        self.ledger.log({'event':'model_output','request':r['request'],'raw_response':response,'choices':raw.get('choices',[]),
                         'usage':raw.get('usage',{}),'latency_ms':elapsed,'response_id':raw.get('id'),
                         'response_model':raw.get('model'),'system_fingerprint':raw.get('system_fingerprint')})
        try:
            choice=raw['choices'][0];message=choice['message'];usage=raw.get('usage',{})
            if (raw.get('model')!=self.model['model'] or choice.get('finish_reason')!='stop' or message.get('reasoning_content')
                or usage.get('completion_tokens_details',{}).get('reasoning_tokens',0)!=0
                or (type(usage.get('completion_tokens')) is int and usage['completion_tokens']>output)):
                raise ValueError('model, truncation or non-thinking contract failed')
            value=json.loads(message['content'])
            if validator is not None:value=validator(value)
        except Exception as error:
            # 先校验再结算：无效输出或schema不释放预留，也不重试修答案。
            self.ledger.halted=True
            self.ledger.log({'event':'invalid_output_stop','request':r['request'],'error_type':type(error).__name__})
            raise BudgetStop('model response/JSON/schema contract failed; reservation retained; retry=0') from None
        cost=self.ledger.settle(r,usage,{'latency_ms':elapsed,'response_id':raw.get('id'),
                           'response_model':raw.get('model'),'system_fingerprint':raw.get('system_fingerprint'),
                           'input_sha256':digest(messages)})
        return {'json':value,'usage':usage,'latency_ms':elapsed,'input_byte_token_bound':bound,
                'request':r['request'],'response_id':raw.get('id'),'cost_cny':str(cost),'raw_response':response,
                'response_model':raw.get('model'),'system_fingerprint':raw.get('system_fingerprint')}


class TextTokenizer:
    def __init__(self,path,expected_sha,add_special_tokens=False):
        raw=Path(path).read_bytes()
        if hashlib.sha256(raw).hexdigest()!=expected_sha:raise ValueError('tokenizer SHA mismatch')
        from tokenizers import Tokenizer
        import tokenizers
        if tokenizers.__version__!='0.22.1':raise ValueError('use the locked tokenizers==0.22.1 runtime')
        self.tokenizer=Tokenizer.from_str(raw.decode());self.sha=expected_sha;self.add_special_tokens=add_special_tokens

    def count(self,text):
        return len(self.tokenizer.encode(text,add_special_tokens=self.add_special_tokens).ids)


def build_context(documents,tokenizer,policy):
    items=[];omissions=[];remaining=policy['max_tokens'];bytes_left=policy['max_utf8_bytes'];stopped=False
    for doc in documents:
        text=doc['content'];header=f"\nSOURCE {doc['id']}\n";rendered=header+text
        n=tokenizer.count(rendered);b=len(rendered.encode())
        if stopped or remaining<=0 or bytes_left<=len(header.encode()):
            omissions.append({'source_id':doc['id'],'reason':'budget','original_tokens':n});continue
        end=len(text)
        if n>remaining or b>bytes_left:
            lo=0;hi=len(text)
            while lo<hi:
                mid=(lo+hi+1)//2;candidate=header+text[:mid]
                if tokenizer.count(candidate)<=remaining and len(candidate.encode())<=bytes_left:lo=mid
                else:hi=mid-1
            end=lo;rendered=header+text[:end];n=tokenizer.count(rendered);b=len(rendered.encode());stopped=True
        if end:
            items.append({'source_id':doc['id'],'text':text[:end],'source_char_end':end,'original_chars':len(text),
                          'tokens':n,'truncated':end<len(text),'rendered':rendered})
            remaining-=n;bytes_left-=b
        if end<len(text):omissions.append({'source_id':doc['id'],'reason':'prefix_truncated','omitted_chars':len(text)-end})
    joined=''.join(i['rendered'] for i in items);actual_tokens=tokenizer.count(joined)
    if actual_tokens>policy['max_tokens']:raise ValueError('joined context token budget exceeded; no Reader call')
    return {'items':items,'text':joined,'content_tokens':actual_tokens,
            'utf8_bytes':policy['max_utf8_bytes']-bytes_left,'omissions':omissions,'tokenizer_sha256':tokenizer.sha,'policy':policy}


class SemanticClient:
    def __init__(self,url,manifest,tokenizer,cache,transport=json_http):
        parsed=urlparse(url)
        if parsed.scheme!='http' or parsed.hostname!='127.0.0.1' or parsed.username or parsed.query or parsed.fragment:
            raise ValueError('semantic evaluation uses a dedicated local gateway')
        self.url=url;self.manifest=manifest;self.tokenizer=tokenizer;self.cache=Path(cache);self.cache.mkdir(parents=True,exist_ok=True)
        self.transport=transport;self.calls=0;self.texts=0;self.started=time.monotonic()
        if tokenizer.add_special_tokens is not manifest['embedding_parameters']['add_special_tokens']:
            raise ValueError('embedding tokenizer must include the gateway EOS postprocessor')
        health=transport(url+'/health',None,5)
        self.health=health
        expected=manifest['embedding'];params=manifest['embedding_parameters'];actual=health.get('embedding',{})
        if (actual.get('model')!=expected['model'] or actual.get('revision')!=expected['revision']
            or actual.get('dimensions')!=params['dimensions'] or actual.get('normalization')!='l2'
            or health.get('max_length')!=params['max_length'] or health.get('instruction')!=params['instruction']
            or health.get('device')!=params['device'] or health.get('embedding_dtype')!=params['dtype']
            or any(health.get(k)!=params[k] for k in ['torch_version','transformers_version','tokenizers_version'])):
            raise ValueError('gateway health differs from frozen model parameters')

    def embed(self,texts,input_type):
        params=self.manifest['embedding_parameters'];result=[]
        for start in range(0,len(texts),32):
            batch=texts[start:start+32]
            for text in batch:
                formatted=f"Instruct: {params['instruction']}\nQuery:{text}" if input_type=='query' else text
                if self.tokenizer.count(formatted)>params['max_length']:raise ValueError('embedding would truncate; experiment stopped')
            key=digest({'texts':batch,'input_type':input_type,'model':self.manifest['embedding'],'parameters':params})
            path=self.cache/(key+'.json')
            if path.exists():response=json.loads(path.read_text())
            else:
                if self.calls>=400 or self.texts+len(batch)>12014 or time.monotonic()-self.started>3600:
                    raise BudgetStop('local semantic compute budget reached')
                self.calls+=1;self.texts+=len(batch)
                response=self.transport(self.url+'/v1/embeddings',{'model':self.manifest['embedding']['model'],
                    'input':batch,'input_type':input_type,'dimensions':params['dimensions'],'instruction':params['instruction']},45)
                path.write_text(json.dumps(response,allow_nan=False))
            if response.get('model')!=self.manifest['embedding']['model'] or len(response['data'])!=len(batch):raise ValueError('embedding count/model mismatch')
            for i,entry in enumerate(response['data']):
                v=entry['embedding']
                if (entry['index']!=i or len(v)!=params['dimensions'] or not all(math.isfinite(x) for x in v)
                    or abs(math.sqrt(sum(x*x for x in v))-1)>.01):raise ValueError('embedding dimensions/index/L2 mismatch')
                result.append(v)
        return result


class DenseIndex:
    def __init__(self,documents,client):
        self.documents=documents;self.client=client;self.vectors=client.embed([d['content'] for d in documents],'document')

    def search(self,query,limit):
        vector=self.client.embed([query],'query')[0]
        ranked=[(d['id'],sum(a*b for a,b in zip(v,vector))) for d,v in zip(self.documents,self.vectors)]
        return sorted(ranked,key=lambda r:(-r[1],r[0]))[:limit]
