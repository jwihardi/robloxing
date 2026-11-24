"""Exercise the real Workshop binaries with a deterministic Windows stdio provider."""
import argparse
import ctypes
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import tempfile
import time


def wait_for(predicate, message):
    deadline = time.monotonic() + 45
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(0.05)
    raise AssertionError(message)


def exited(pid):
    kernel = ctypes.windll.kernel32
    handle = kernel.OpenProcess(0x100000, False, pid)
    if not handle:
        return True
    try:
        return kernel.WaitForSingleObject(handle, 0) == 0
    finally:
        kernel.CloseHandle(handle)


class Case:
    def __init__(self, root, cli, fixture):
        self.root = root
        root.mkdir()
        self.project = root / "project"
        self.project.mkdir()
        self.home = root / "data"
        self.env = dict(os.environ, WORKSHOP_HOME=str(self.home),
                        WORKSHOP_CODEX=str(fixture), WORKSHOP_PROBE_DIR=str(root))
        self.cmd = [str(cli), "--project", str(self.project)]
        self.call("open")
        self.db = next(self.home.glob("*/workshop.sqlite"))

    def call(self, *args, ok=True):
        result = subprocess.run(self.cmd + list(args), env=self.env, capture_output=True,
                                text=True, encoding="utf-8", errors="replace", timeout=60)
        assert (result.returncode == 0) == ok, (args, result.stderr)
        return result.stdout

    def run(self, *args):
        return subprocess.Popen(self.cmd + list(args), env=self.env, stdin=subprocess.PIPE,
                                stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)

    def hold(self, key):
        (self.root / (key + ".hold")).touch()

    def release(self, key):
        (self.root / (key + ".hold")).unlink()

    def started(self, key):
        marker = self.root / (key + ".ready")
        wait_for(lambda: marker.exists() and "," in marker.read_text(), key + " failed to start")
        return list(map(int, marker.read_text().split(",")))

    def workers(self):
        with sqlite3.connect(self.db.as_uri() + "?mode=ro", uri=True) as db:
            return db.execute("SELECT name,state,tries,repeats FROM workers ORDER BY id").fetchall()


def finish(proc, ok):
    _, error = proc.communicate(timeout=60)
    assert (proc.returncode == 0) == ok, error.decode(errors="replace")


def conflict(case):
    case.call("team", "--parallelism", "1")
    case.hold("left")
    case.hold("right")
    proc = case.run("run", "delegate")
    case.started("left")
    assert case.workers()[1][1:3] == ("queued", 0)
    assert not (case.root / "right.ready").exists()
    case.release("left")
    case.started("right")
    integration = case.db.parent / "attempt-1/work"
    assert (integration / "left.txt").read_text() == "left\n"
    (integration / "right.txt").write_text("concurrent lead edit\n")
    case.release("right")
    # The lead receives the integration conflict and repairs it on the same user request.
    finish(proc, True)
    assert not (case.project / "left.txt").exists()
    assert [w[2] for w in case.workers()] == [1, 2]
    case.call("accept", "1")
    case.call("accept", "1")
    assert [(case.project / (key + ".txt")).read_text() for key in ("left", "right")] == ["left\n", "right\n"]


def stop_one(case):
    for key in ("left", "right"):
        case.hold(key)
    proc = case.run("run", "delegate")
    left_pids = case.started("left")
    right_pids = case.started("right")
    proc.stdin.write(b"/stop-worker 1\n")
    proc.stdin.flush()
    wait_for(lambda: all(exited(pid) for pid in left_pids), "stopped worker still running")
    assert all(not exited(pid) for pid in right_pids)
    case.release("right")
    finish(proc, False)
    assert [w[1] for w in case.workers()] == ["blocked", "finished"]
    case.release("left")
    case.call("resume", "1", "retry left")
    assert [w[2] for w in case.workers()] == [2, 1]
    case.call("accept", "1")


def overlap(case):
    case.call("team", "--parallelism", "2")
    (case.root / "overlap").touch()
    case.hold("left")
    proc = case.run("run", "delegate")
    case.started("left")
    assert case.workers()[1][1:3] == ("queued", 0)
    case.release("left")
    finish(proc, True)
    assert all(row[1] == "finished" for row in case.workers())


def mixed_plan(case):
    case.call("team", "--check-args", json.dumps([case.env["WORKSHOP_CODEX"], "joined-check"]))
    (case.root / "mixed").touch()
    for key in ("left", "right", "guide"):
        case.hold(key)
    proc = case.run("run", "mixed plan")
    try:
        case.started("left")
        right_pids = case.started("right")
        assert [w[1] for w in case.workers()] == ["running", "running", "queued", "queued"]
        case.release("left")
        case.started("guide")
        assert all(not exited(pid) for pid in right_pids)
        case.release("guide")
        wait_for(lambda: case.workers()[2][1] == "finished", "guide did not integrate")
        assert case.workers()[3][1:3] == ("queued", 0)
        case.release("right")
        finish(proc, True)
    finally:
        if proc.poll() is None:
            proc.kill()
            proc.wait()
    case.call("accept", "1")
    assert (case.root / "joins.log").read_text().splitlines() == ["check"]
    for key in ("left", "right", "guide", "summary"):
        assert (case.project / (key + ".txt")).read_text() == key + "\n"


def repeated_check(case):
    case.call("team", "--check-args", json.dumps([case.env["WORKSHOP_CODEX"], "check"]))
    (case.root / "fail-check").touch()
    case.call("run", "delegate", ok=False)
    case.call("resume", "1", "retry", ok=False)
    case.call("resume", "1", "retry again", ok=False)
    assert all(w[1:] == ("blocked", 2, 2) for w in case.workers()), case.workers()
    case.call("accept", "1", ok=False)


def autonomous_release(case):
    (case.root / "autonomous").touch()
    case.call("run", "Deliver a release bundle with two outputs and a checked release note.")
    by_key = {key: (state, tries) for key, state, tries, _ in case.workers()}
    assert by_key == {"left": ("finished", 1), "right": ("finished", 1), "audit": ("finished", 2),
                      "fix-right": ("finished", 1), "publish": ("finished", 1)}, by_key
    with sqlite3.connect(case.db) as db:
        repairs = db.execute("SELECT summary FROM events WHERE kind='workshop/repair'").fetchall()
        assert len(repairs) == 1 and "rigth" in repairs[0][0], repairs
        assert not db.execute("SELECT 1 FROM events WHERE kind='user/followup'").fetchone()
    assert not (case.project / "right.txt").exists()
    case.call("accept", "1")
    assert (case.project / "right.txt").read_text().strip() == "right"
    assert (case.project / "publish.txt").read_text().strip() == "release ready"


def direct_repair(case):
    (case.root / "direct-repair").touch()
    case.call("team", "--check-args", json.dumps([case.env["WORKSHOP_CODEX"], "content-check"]))
    case.call("run", "Update the answer.")
    assert case.workers() == []
    case.call("accept", "1")
    assert (case.project / "answer.txt").read_text() == "correct"


def unavailable_runner(case):
    (case.root / "runner-error").touch()
    case.call("run", "Update the project.", ok=False)
    with sqlite3.connect(case.db) as db:
        state, action = db.execute("SELECT state,action FROM attempts WHERE id=1").fetchone()
        assert state == "failed" and "test command runner" in action
        assert db.execute("SELECT count(*) FROM events WHERE kind='workshop/repair'").fetchone() == (1,)
    case.call("accept", "1", ok=False)


def small_talk(case):
    for flag in ("small-talk", "fail-check", "check-isolation", "record-inference"):
        (case.root / flag).touch()
    case.env["CODEX_WINDOWS_REGISTERED_CORE"] = "inherited-test-marker"
    case.call("team", "--check-args", json.dumps([case.env["WORKSHOP_CODEX"], "check"]))
    case.call("run", "hi")
    assert not case.workers()
    assert len(list(case.root.glob("lead-*.json"))) == 1
    assert not list(case.home.rglob("checks.log"))


def manual_recheck(case):
    (case.root / "direct-repair").touch()
    case.call("run", "answer.txt must contain correct")
    command = [case.env["WORKSHOP_CODEX"], "check"]
    case.call("team", "--check-args", json.dumps(command))
    case.call("accept", "1", ok=False)
    case.call("check", "1", "--", *command)
    marker = case.root / "fail-check"
    marker.touch()
    case.call("check", "1", "--", *command, ok=False)
    case.call("accept", "1", ok=False)
    marker.unlink()
    case.call("check", "1", "--", *command)
    case.call("accept", "1")


def external_blocker(case):
    marker = case.root / "blocker"
    marker.touch()
    case.call("run", "Prepare the release.", ok=False)
    case.call("accept", "1", ok=False)
    assert not (case.project / "answer.txt").exists()
    marker.unlink()
    (case.root / "direct-repair").touch()
    case.call("resume", "1", "The release input is here. answer.txt must contain correct.")
    case.call("accept", "1")
    assert (case.project / "answer.txt").read_text() == "correct"


def cancel_check(case):
    (case.root / "direct-repair").touch()
    case.hold("check")
    case.call("team", "--check-args", json.dumps([case.env["WORKSHOP_CODEX"], "content-check"]))
    proc = case.run("run", "Update the answer.")
    wait_for(lambda: (case.root / "check.ready").exists() and (case.root / "check.ready").stat().st_size, "check not running")
    check_pid = int((case.root / "check.ready").read_text())
    proc.stdin.write(b"/cancel\n")
    proc.stdin.flush()
    finish(proc, True)
    wait_for(lambda: exited(check_pid), "cancelled check still running")
    case.call("accept", "1", ok=False)
    with sqlite3.connect(case.db) as db:
        assert db.execute("SELECT state FROM attempts WHERE id=1").fetchone() == ("cancelled",)
        assert not db.execute("SELECT 1 FROM events WHERE kind='workshop/repair'").fetchone()


def model_effort(case):
    (case.root / "record-inference").touch()
    case.call("team", "--lead-model", "fixture-deep", "--lead-effort", "high",
              "--worker-model", "fixture-fast", "--worker-effort", "low")
    case.call("run", "delegate")
    case.call("team", "--auto-delegate", "false")
    case.call("resume", "1", "Use workers for the follow-up.")
    for role, expected in (("lead", ("fixture-deep", "high")), ("worker", ("fixture-fast", "low"))):
        requests = [json.loads(p.read_text()) for p in case.root.glob(role + "-*.json")]
        assert len(requests) == 2, (role, requests)
        assert all((r["model"], r["effort"]) == expected for r in requests)
        hints = [item["text"] for r in requests for item in r["input"]
                 if item.get("text", "").startswith("[Workshop project setting:")]
        if role == "worker":
            assert not hints, hints
        else:
            assert len(hints) == 2
            assert any("automatic delegation]" in hint for hint in hints)
            assert any("delegation by request]" in hint for hint in hints)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--cli", type=Path, default=Path("target/debug/workshop-cli.exe"))
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="ws-h2-") as tmp:
        base = Path(tmp)
        fixture = base / "provider.exe"
        compiler = Path(os.environ["WINDIR"]) / "Microsoft.NET/Framework64/v4.0.30319/csc.exe"
        subprocess.run([str(compiler), "/nologo", "/r:System.Web.Extensions.dll",
                        "/out:" + str(fixture), str(Path(__file__).with_name("FixtureProvider.cs"))], check=True)
        for test in (autonomous_release, direct_repair, unavailable_runner, small_talk, manual_recheck, external_blocker,
                     mixed_plan, conflict, stop_one, cancel_check, overlap, repeated_check, model_effort):
            test(Case(base / test.__name__, args.cli.resolve(), fixture))
            print(test.__name__ + " passed", flush=True)


if __name__ == "__main__":
    main()
