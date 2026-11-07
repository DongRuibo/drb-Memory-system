"""收费前安全回归：只用临时文件、子进程和本机HTTP夹具，收费请求为0。"""
from copy import deepcopy
from decimal import Decimal
from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest.mock import patch

from evals.memory_eval.models3 import BudgetStop,ChatClient,Ledger,json_http
from evals.memory_eval import phase3


class BudgetAuditTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.addCleanup(self.temp.cleanup);self.root=Path(self.temp.name)
        self.protocol,self.models,self.lock=phase3.load_configuration()

    def ledger(self):
        ledger=Ledger(self.root/'calls.jsonl',self.protocol);self.addCleanup(ledger.file.close);return ledger

    def test_role_limit_does_not_reset_after_settlement(self):
        p=deepcopy(self.protocol);p['role_request_limits']['reader']=1
        ledger=Ledger(self.root/'calls.jsonl',p);self.addCleanup(ledger.file.close)
        r=ledger.reserve('reader',100,100);ledger.settle(r,{'prompt_tokens':1,'completion_tokens':1},{})
        with self.assertRaises(BudgetStop):ledger.reserve('reader',100,100)
        self.assertEqual(ledger.requests,1)

    def test_duplicate_settlement_cannot_release_budget_twice(self):
        ledger=self.ledger();r=ledger.reserve('reader',100,100)
        ledger.settle(r,{'prompt_tokens':10,'completion_tokens':10},{})
        balance=ledger.cost
        with self.assertRaises(BudgetStop):ledger.settle(r,{'prompt_tokens':0,'completion_tokens':0},{})
        self.assertEqual(ledger.cost,balance)

    def test_second_inflight_request_is_refused(self):
        ledger=self.ledger();ledger.reserve('reader',100,100)
        with self.assertRaises(BudgetStop):ledger.reserve('judge',100,100)
        self.assertEqual(ledger.requests,1)

    def test_thread_cannot_dispatch_from_another_threads_ledger(self):
        ledger=self.ledger();calls=[];errors=[]
        client=ChatClient(self.models['reader'],self.protocol,ledger,transport=lambda *a:calls.append(a))
        def attempt():
            try:client.call([{'role':'user','content':'fixture'}],'reader')
            except BudgetStop as e:errors.append(str(e))
        thread=threading.Thread(target=attempt);thread.start();thread.join(timeout=5)
        self.assertFalse(thread.is_alive());self.assertEqual(len(errors),1)
        self.assertEqual(calls,[]);self.assertEqual(ledger.requests,0)

    @unittest.skipUnless(hasattr(os,'fork'),'POSIX fork guard')
    def test_fork_child_cannot_reuse_parent_ledger(self):
        ledger=self.ledger();read,write=os.pipe();pid=os.fork()
        if pid==0:
            os.close(read)
            try:ledger.reserve('reader',100,100)
            except BudgetStop:os.write(write,b'blocked')
            finally:os._exit(0)
        os.close(write);result=os.read(read,100);os.close(read);os.waitpid(pid,0)
        self.assertEqual(result,b'blocked');self.assertEqual(ledger.requests,0)

    def test_reservation_disk_failure_sends_nothing(self):
        ledger=self.ledger();calls=[]
        client=ChatClient(self.models['reader'],self.protocol,ledger,transport=lambda *a:calls.append(a))
        with patch('evals.memory_eval.models3.os.fsync',side_effect=OSError('fixture disk failure')):
            with self.assertRaises(BudgetStop):client.call([{'role':'user','content':'fixture'}],'reader')
        self.assertEqual(calls,[]);self.assertTrue(ledger.halted)

    def test_settlement_disk_failure_keeps_worst_reservation(self):
        ledger=self.ledger();r=ledger.reserve('reader',1000,100);balance=ledger.cost
        with patch('evals.memory_eval.models3.os.fsync',side_effect=OSError('fixture disk failure')):
            with self.assertRaises(BudgetStop):ledger.settle(r,{'prompt_tokens':1,'completion_tokens':1},{})
        self.assertEqual(ledger.cost,balance);self.assertEqual(ledger.pending,r);self.assertTrue(ledger.halted)

    def test_process_race_has_one_winner_even_with_different_output_paths(self):
        script="""import sys,time
from pathlib import Path
from evals.memory_eval.phase3 import claim_execution
from evals.memory_eval.models3 import BudgetStop
base=Path(sys.argv[1]);out=base/sys.argv[2]
while not (base/'start').exists():time.sleep(.01)
try:claim_execution({'budget_key':'a'*64,'seal':sys.argv[2]},out,base/'claims')
except BudgetStop:sys.exit(4)
"""
        processes=[subprocess.Popen([sys.executable,'-c',script,str(self.root),name],stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True) for name in ['one','two']]
        (self.root/'start').touch()
        results=[p.communicate(timeout=15) for p in processes]
        self.assertEqual(sorted(p.returncode for p in processes),[0,4],results)
        with self.assertRaises(BudgetStop):phase3.claim_execution({'budget_key':'a'*64,'seal':'changed-code-plan'},self.root/'three',self.root/'claims')

    def test_partial_claim_after_crash_is_not_treated_as_free(self):
        claims=self.root/'claims';claims.mkdir();(claims/('b'*64+'.json')).touch()
        with self.assertRaises(BudgetStop):phase3.claim_execution({'budget_key':'b'*64,'seal':'fixture'},self.root/'new-out',claims)

    def test_process_crash_reservation_is_durable_and_ledger_cannot_restart(self):
        script="""import json,os,sys
from evals.memory_eval.models3 import Ledger
l=Ledger(sys.argv[1],json.loads(sys.argv[2]));l.reserve('reader',16384,896);os._exit(9)
"""
        path=self.root/'crash.jsonl'
        result=subprocess.run([sys.executable,'-c',script,str(path),json.dumps(self.protocol)],capture_output=True,text=True,timeout=10)
        self.assertEqual(result.returncode,9,result.stderr)
        events=[json.loads(s) for s in path.read_text().splitlines()]
        self.assertEqual(events[-1]['event'],'reserved');self.assertEqual(events[-1]['cost_reserved_cny'],'0.070912')
        with self.assertRaises(FileExistsError):Ledger(path,self.protocol)

    def test_budget_identity_ignores_approval_and_code_paths(self):
        same=phase3.budget_key(self.lock,self.models)
        model=deepcopy(self.models);model['reader']['revision']='same hosted model, metadata refresh'
        self.assertEqual(same,phase3.budget_key(self.lock,model))

    def test_quote_uses_protocol_prices_instead_of_literal_4_6(self):
        protocol=deepcopy(self.protocol);protocol.update(price_cny_per_million_input='1',price_cny_per_million_output='2')
        class Counter:
            def count(self,text):return len(text)
        cases=[{'input':{'id':str(i),'query':'fixture','documents':[{'content':'note'}]},
                'labels':{'unanswerable':False,'question_type':'fixture','evidence_qrels':{}}} for i in range(14)]
        out=self.root/'prepared';out.mkdir()
        class Args:longmemeval='unused';assets='unused'
        with patch.object(phase3,'load_configuration',return_value=(protocol,self.models,self.lock)),patch.object(phase3,'tokenizers',return_value={'embedding':Counter(),'reader_tokenizer':Counter()}),patch.object(phase3,'source_rows',return_value=[]),patch.object(phase3,'load_cases',return_value=cases):
            phase3.prepare(Args(),out)
        plan=json.loads((out/'plan.json').read_text())
        self.assertEqual(Decimal(plan['total_cost_ceiling_cny']),Decimal('2.915840'))

    def test_current_quote_role_and_token_allocation(self):
        r=self.protocol['role_request_limits'];self.assertEqual(r,{'reader':56,'judge':56,'calibration':16,'smoke':2})
        self.assertEqual(sum(r.values()),130)
        self.assertEqual(58*16384+72*24576,self.protocol['max_input_tokens_total'])
        self.assertEqual(58*896+72*640,self.protocol['max_output_tokens_total'])

    def test_hidden_reasoning_usage_stops_even_without_reasoning_text(self):
        ledger=self.ledger()
        def response(payload,timeout):
            return {'model':self.models['reader']['model'],'choices':[{'finish_reason':'stop','message':{'content':'{}'}}],
                    'usage':{'prompt_tokens':100,'completion_tokens':20,'completion_tokens_details':{'reasoning_tokens':10}}}
        with self.assertRaises(BudgetStop):ChatClient(self.models['reader'],self.protocol,ledger,transport=response).call([{'role':'user','content':'fixture'}],'reader')
        self.assertTrue(ledger.halted);self.assertEqual(ledger.requests,1)

    def test_price_evidence_mismatch_and_budget_above_12_rejected_at_entry(self):
        config=self.root/'config';config.mkdir()
        for p in phase3.CONFIG.glob('phase3*.json'):(config/p.name).write_bytes(p.read_bytes())
        p=config/'phase3_prices.json';v=json.loads(p.read_text());v['input_per_million']='5';p.write_text(json.dumps(v))
        with patch.object(phase3,'CONFIG',config):
            with self.assertRaises(ValueError):phase3.load_configuration()
        v['input_per_million']='4';p.write_text(json.dumps(v))
        p=config/'phase3_protocol.json';v=json.loads(p.read_text());v['max_cost_cny']='12.01';p.write_text(json.dumps(v))
        with patch.object(phase3,'CONFIG',config):
            with self.assertRaises(ValueError):phase3.load_configuration()

    @unittest.skipUnless(hasattr(signal,'setitimer'),'POSIX wall-clock HTTP deadline')
    def test_slow_drip_http_does_not_extend_total_deadline(self):
        class Drip(BaseHTTPRequestHandler):
            def do_POST(self):
                self.send_response(200);self.send_header('Content-Length','1000');self.end_headers()
                try:
                    for _ in range(120):self.wfile.write(b' ');self.wfile.flush();time.sleep(.03)
                except OSError:pass
            def log_message(self,*args):pass
        server=ThreadingHTTPServer(('127.0.0.1',0),Drip)
        thread=threading.Thread(target=server.serve_forever,kwargs={'poll_interval':.01},daemon=True);thread.start()
        started=time.monotonic()
        try:
            with self.assertRaises((TimeoutError, __import__('urllib.error',fromlist=['URLError']).URLError)):
                json_http(f'http://127.0.0.1:{server.server_port}/fixture',{},.2)
            self.assertLess(time.monotonic()-started,1)
        finally:server.shutdown();server.server_close();thread.join(timeout=2)
        self.assertEqual(signal.getitimer(signal.ITIMER_REAL)[0],0)


if __name__=='__main__':unittest.main()
