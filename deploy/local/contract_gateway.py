"""Loopback-published test ingress to the isolated Mem0 contract network."""

from http.client import HTTPConnection
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from threading import Thread


class Gateway(BaseHTTPRequestHandler):
    def forward(self):
        size = int(self.headers.get("Content-Length", "0"))
        if not 0 <= size <= 2 * 1024 * 1024:
            self.send_error(413)
            return
        target = ("mem0", 8000) if self.server.server_port == 8000 else ("retrieval-models-cpu", 18080)
        upstream = HTTPConnection(*target, timeout=30)
        try:
            headers = {key: self.headers[key] for key in ("Content-Type", "X-API-Key") if key in self.headers}
            upstream.request(self.command, self.path, self.rfile.read(size), headers)
            response = upstream.getresponse()
            body = response.read(2 * 1024 * 1024 + 1)
            if len(body) > 2 * 1024 * 1024:
                self.send_error(502)
                return
            self.send_response(response.status)
            self.send_header("Content-Type", response.getheader("Content-Type", "application/json"))
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        except OSError:
            self.send_error(502)
        finally:
            upstream.close()

    do_GET = forward
    do_POST = forward
    do_DELETE = forward

    def log_message(self, *_args):
        pass


if __name__ == "__main__":
    for port in (8000, 8001):
        server = ThreadingHTTPServer(("0.0.0.0", port), Gateway)
        Thread(target=server.serve_forever).start()
