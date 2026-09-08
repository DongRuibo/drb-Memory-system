"""Deterministic OpenAI-compatible test server; never a deployment dependency."""
from http.server import BaseHTTPRequestHandler, HTTPServer
import json


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        if self.path == '/v1/embeddings':
            inputs = body['input'] if isinstance(body['input'], list) else [body['input']]
            result = {'object': 'list', 'model': 'fixture-embedding',
                'data': [{'object': 'embedding', 'index': i, 'embedding': [1.0] + [0.0] * 15} for i in range(len(inputs))],
                'usage': {'prompt_tokens': 1, 'total_tokens': 1}}
        else:
            result = {'id': 'fixture', 'object': 'chat.completion', 'created': 1, 'model': 'fixture-chat',
                'choices': [{'index': 0, 'finish_reason': 'stop', 'message': {'role': 'assistant',
                    'content': json.dumps({'memory': [{'text': 'prefers brief answers'}]})}}],
                'usage': {'prompt_tokens': 1, 'completion_tokens': 1, 'total_tokens': 2}}
        data = json.dumps(result).encode()
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(data)))
        self.end_headers()
        self.wfile.write(data)


HTTPServer(('0.0.0.0', 8000), Handler).serve_forever()
