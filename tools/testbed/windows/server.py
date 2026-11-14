#!/usr/bin/env python3
"""Host side of the test VM agent: a queue of PowerShell jobs, their results, and files to fetch.

  POST /enqueue {script, timeout}  -> {id}        (from vmrun)
  GET  /wait/<id>                  -> result      (long-poll, from vmrun)
  GET  /next                       -> job or 204  (long-poll, from the VM)
  POST /result {id, code, output}                 (from the VM)
  GET  /files/<name>               -> a file from ./files
"""
import json, os, threading, time, uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

HERE = os.path.dirname(os.path.abspath(__file__))
FILES = os.environ.get("ORITHER_VM_FILES", os.path.join(HERE, "files"))
jobs, results = [], {}
lock = threading.Condition()
seen = {"last": 0.0}

class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def reply(self, code, body=b"", kind="application/json"):
        self.send_response(code)
        self.send_header("Content-Type", kind)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def body(self):
        return json.loads(self.rfile.read(int(self.headers.get("Content-Length", 0))) or b"{}")

    def do_GET(self):
        if self.path == "/next":
            seen["last"] = time.time()
            with lock:
                lock.wait_for(lambda: jobs, timeout=25)
                job = jobs.pop(0) if jobs else None
            return self.reply(200, json.dumps(job).encode()) if job else self.reply(204)
        if self.path == "/alive":
            return self.reply(200, json.dumps({"seconds_since_poll": time.time() - seen["last"]}).encode())
        if self.path.startswith("/wait/"):
            key = self.path[len("/wait/"):]
            with lock:
                lock.wait_for(lambda: key in results, timeout=600)
                result = results.pop(key, None)
            return self.reply(200, json.dumps(result).encode()) if result else self.reply(504)
        if self.path.startswith("/files/"):
            path = os.path.join(FILES, os.path.basename(self.path[len("/files/"):]))
            if not os.path.isfile(path):
                return self.reply(404)
            self.send_response(200)
            self.send_header("Content-Type", "application/octet-stream")
            self.send_header("Content-Length", str(os.path.getsize(path)))
            self.end_headers()
            with open(path, "rb") as f:
                while chunk := f.read(1 << 20):
                    self.wfile.write(chunk)
            return
        self.reply(404)

    def do_POST(self):
        data = self.body()
        if self.path == "/enqueue":
            job = {"id": uuid.uuid4().hex[:8], "script": data["script"], "timeout": data.get("timeout", 300)}
            with lock:
                jobs.append(job)
                lock.notify_all()
            return self.reply(200, json.dumps({"id": job["id"]}).encode())
        if self.path == "/result":
            with lock:
                results[data["id"]] = data
                lock.notify_all()
            return self.reply(200, b"{}")
        self.reply(404)

os.makedirs(FILES, exist_ok=True)
ThreadingHTTPServer(("127.0.0.1", 8800), Handler).serve_forever()
