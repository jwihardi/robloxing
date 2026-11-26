"""Exercise Workshop through the installed Gemini CLI with a local model endpoint."""
import argparse
from contextlib import closing, ExitStack
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import tempfile
import threading


class ModelEndpoint(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        if "generatecontent" not in self.path.lower():
            self.send_error(404)
            return
        names = [d["name"] for t in body.get("tools", []) for d in t.get("functionDeclarations", [])]
        self.server.tools = names
        contents = body.get("contents", [])
        last = contents[-1].get("parts", []) if contents else []
        text = " ".join(p.get("text", "") for p in last)
        delegate = next((n for n in names if n.endswith("workshop_delegate")), None)
        report = next((n for n in names if n.endswith("workshop_report_result")), None)
        generate = next((n for n in names if n.endswith("workshop_generate_asset")), None)
        if "HOLD_GEMINI_PROBE" in text:
            self.server.waiting.set()
            self.server.release.wait(timeout=10)
            parts = [{"text": "Original prompt."}]
        elif "STEER_GEMINI_PROBE" in text:
            parts = [{"text": "Direction received."}]
        elif "QUEUE_GEMINI_PROBE" in text:
            parts = [{"text": "Queued message received."}]
        elif any("functionResponse" in part for part in last):
            parts = [{"text": "Tool result received."}]
        elif "ASSET_PROBE" in text and generate:
            parts = [{"functionCall": {"name": generate, "args": {
                "kind": "texture", "material": "Moss", "seed": 17,
            }}}]
        elif "DELEGATE_PROBE" in text and delegate:
            parts = [{"functionCall": {"name": delegate, "args": {"tasks": [{
                "key": "review", "title": "Review project", "deliverable": "Review README",
                "acceptance": "Report what you found", "files": [], "depends_on": [],
            }]}}}]
        elif report:
            parts = [{"functionCall": {"name": report, "args": {
                "status": "complete", "summary": "Reviewed the project.",
            }}}]
        else:
            parts = [{"text": "Hello from Gemini."}]
        response = {"candidates": [{"content": {"role": "model", "parts": parts},
                                    "index": 0, "finishReason": "STOP"}],
                    "usageMetadata": {"promptTokenCount": 11, "candidatesTokenCount": 4,
                                      "totalTokenCount": 15}}
        is_stream = "streamgeneratecontent" in self.path.lower()
        data = json.dumps(response)
        data = (f"data: {data}\n\n" if is_stream else data).encode()
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream" if is_stream else "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        try:
            self.wfile.write(data)
        except (ConnectionResetError, ConnectionAbortedError, BrokenPipeError):
            pass  # The CLI closed the streamed request when the user steered it.


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", required=True, type=Path)
    cli = parser.parse_args().cli.resolve()
    with tempfile.TemporaryDirectory(prefix="workshop gemini ") as tmp, \
            ThreadingHTTPServer(("127.0.0.1", 0), ModelEndpoint) as endpoint, ExitStack() as cleanup:
        threading.Thread(target=endpoint.serve_forever, daemon=True).start()
        cleanup.callback(endpoint.shutdown)
        endpoint.waiting = threading.Event()
        endpoint.release = threading.Event()
        root = Path(tmp)
        project = root / "project"
        project.mkdir()
        home = root / "user"
        config = home / ".gemini"
        config.mkdir(parents=True)
        (config / "settings.json").write_text(json.dumps({
            "security": {"auth": {"selectedType": "gemini-api-key"}},
            "privacy": {"usageStatisticsEnabled": False},
        }))
        env = dict(os.environ, USERPROFILE=str(home), HOME=str(home), GEMINI_CLI_HOME=str(home),
                   WORKSHOP_HOME=str(root / "data"), NO_COLOR="1",
                   GOOGLE_GEMINI_BASE_URL=f"http://127.0.0.1:{endpoint.server_port}",
                   GEMINI_API_KEY="local-test-only", WORKSHOP_CODEX=str(root / "missing-codex"),
                   WORKSHOP_CLAUDE=str(root / "missing-claude"))

        def call(*args):
            r = subprocess.run([str(cli), "--project", str(project), *args], env=env,
                               capture_output=True, stdin=subprocess.DEVNULL, text=True,
                               encoding="utf-8", errors="replace", timeout=90)
            assert r.returncode == 0, (args, r.stderr[-1500:], r.stdout[-1500:])
            return r.stdout

        catalog = call("models", "--provider", "gemini").split()
        assert "gemini:default" in catalog
        model = next(m for m in catalog if m.startswith("gemini:gemini-"))
        call("open")
        assert "Hello from Gemini" in call("run", "--provider", "gemini", "--model",
                                           model, "STREAM_PROBE")
        db_path = next((root / "data").glob("*/workshop.sqlite"))
        with closing(sqlite3.connect(db_path)) as db:
            provider, saved, session, state = db.execute(
                "SELECT provider,model,session,state FROM attempts WHERE id=1").fetchone()
            assert (provider, saved, state) == ("gemini", model, "review") and session
        assert "Hello from Gemini" in call("resume", "1", "RESUME_PROBE")
        with closing(sqlite3.connect(db_path)) as db:
            assert db.execute("SELECT session FROM attempts WHERE id=1").fetchone() == (session,)
            assert db.execute("SELECT count(*) FROM events WHERE attempt=1 AND kind='agent/message'").fetchone() == (2,)
        call("gemini-trust", "true")
        output = call("run", "--provider", "gemini", "DELEGATE_PROBE")
        assert "Tool result received" in output, (endpoint.tools, output[-900:],
            (db_path.parent / "attempt-2/provider.stderr.log").read_text(errors="replace")[-2000:])
        with closing(sqlite3.connect(db_path)) as db:
            assert db.execute("SELECT state FROM workers").fetchall() == [("finished",)]
            assert db.execute("SELECT used,capacity FROM context_usage WHERE attempt=1").fetchone()[0] == 15
        assert "Tool result received" in call("resume", "2", "ASSET_PROBE")
        with closing(sqlite3.connect(db_path)) as db:
            row = db.execute("SELECT e.attempt,a.maker,i.path FROM event_assets a JOIN events e ON e.id=a.event JOIN event_images i ON i.event=e.id").fetchone()
            assert row[:2] == (2, "Local texture generator") and Path(row[2]).is_file()
        with subprocess.Popen([str(cli), "--project", str(project), "run", "--provider", "gemini",
                               "HOLD_GEMINI_PROBE"], env=env, stdin=subprocess.PIPE,
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE) as proc:
            try:
                assert endpoint.waiting.wait(30), "Gemini did not start the turn"
                proc.stdin.write(b"/queue QUEUE_GEMINI_PROBE\nSTEER_GEMINI_PROBE\n")
                proc.stdin.flush()
                endpoint.release.set()
                out, err = proc.communicate(timeout=60)
                assert proc.returncode == 0, err
                assert b"Direction received." in out and b"Queued message received." in out, out
            finally:
                endpoint.release.set()
                if proc.poll() is None:
                    proc.kill()
        signed_out = dict(env)
        for key in ("GEMINI_API_KEY", "GOOGLE_API_KEY", "GOOGLE_GEMINI_BASE_URL"):
            signed_out.pop(key, None)
        result = subprocess.run([str(cli), "--project", str(project), "run", "--provider", "gemini",
                                 "SIGNED_OUT_PROBE"], env=signed_out, stdin=subprocess.DEVNULL,
                                capture_output=True, timeout=40)
        error = result.stderr.lower()
        assert result.returncode and b"needs authentication" in error, result.stderr
        assert b"antigravity" in error and b"api key" in error, result.stderr
    print("Gemini discovery, streaming, resume, asset delivery, worker dispatch, usage, steering, queue, and signed-out handling passed.")


if __name__ == "__main__":
    main()
