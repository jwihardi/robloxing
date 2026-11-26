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


def wait_for(predicate, message, timeout=45):
    deadline = time.monotonic() + timeout
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
                        WORKSHOP_CODEX=str(fixture), WORKSHOP_CLAUDE=str(fixture), WORKSHOP_PROBE_DIR=str(root))
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


def chat_goals(case):
    case.env["WORKSHOP_GEMINI"] = case.env["WORKSHOP_ANTIGRAVITY"] = case.env["WORKSHOP_CODEX"]
    marker = case.root / "goal-mode"
    marker.touch()
    objective = "Write and check both fixture files"
    for index, provider in enumerate(("codex", "claude", "gemini", "antigravity"), start=1):
        case.call("run", "--provider", provider, "/goal " + objective)
        with sqlite3.connect(case.db) as db:
            row = db.execute("SELECT t.prompt,g.objective,g.status,g.detail FROM attempts a "
                             "JOIN tasks t ON t.id=a.task JOIN chat_goals g ON g.attempt=a.id WHERE a.id=?",
                             (index,)).fetchone()
            assert row == (objective, objective, "complete", "Verified both fixture files."), (provider, row)
            assert db.execute("SELECT count(*) FROM events WHERE attempt=? AND kind='agent/message'",
                              (index,)).fetchone()[0] == 2
        assert "Verified both fixture files." in case.call("resume", str(index), "/goal")
        work = case.db.parent / ("attempt-" + str(index)) / "work"
        assert (work / "first.txt").read_text() == "one" and (work / "second.txt").read_text() == "two"

    case.call("team", "--check-args", json.dumps([case.env["WORKSHOP_CODEX"], "check"]))
    (case.root / "fail-check").touch()
    case.call("run", "/goal Meet the project check", ok=False)
    assert "paused" in case.call("resume", "5", "/goal")
    (case.root / "fail-check").unlink()
    case.call("resume", "5", "/goal Recheck the project")
    assert "complete" in case.call("resume", "5", "/goal")
    marker.unlink()
    (case.root / "small-talk").touch()
    case.call("run", "/goal Needs a report")
    assert "paused" in case.call("resume", "6", "/goal")
    case.call("run", "An ordinary conversation")
    assert "No goal set" in case.call("resume", "7", "/goal")
    with sqlite3.connect(case.db) as db:
        assert db.execute("SELECT count(*) FROM events WHERE attempt=6 AND kind='agent/message'").fetchone() == (2,)
        assert db.execute("SELECT count(*) FROM events WHERE attempt=7 AND kind='agent/message'").fetchone() == (1,)


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


def stop_preparing_worker(case):
    case.call("team", "--parallelism", "1", "--worker-model", "fixture-fast")
    case.hold("catalog")
    proc = case.run("run", "delegate")
    try:
        wait_for(lambda: (case.root / "catalog.ready").exists(), "worker never requested models")
        proc.stdin.write(b"/steer-worker 1 Keep the lobby entrance\n/stop-worker 1\n")
        proc.stdin.flush()
        wait_for(lambda: case.workers()[0][1] == "blocked",
                 "worker ignored stop during model discovery", timeout=10)
        with sqlite3.connect(case.db) as db:
            assert db.execute("SELECT run FROM workers WHERE id=1").fetchone() == (None,)
            notices = db.execute("SELECT summary FROM events WHERE kind='worker/delivery_error'").fetchall()
            assert len(notices) == 1 and "Keep the lobby entrance" in notices[0][0], notices
    finally:
        case.release("catalog")
        proc.communicate(timeout=60)
    assert proc.returncode != 0
    assert not (case.root / "left.ready").exists()
    assert [w[1] for w in case.workers()] == ["blocked", "finished"]


def read_only_models(case):
    folder = case.root / "ordinary-folder"
    folder.mkdir()
    (folder / "notes.txt").write_text("unfinished work")
    home = case.root / "unused-home"
    result = subprocess.run([case.cmd[0], "--project", str(folder), "models", "--provider", "codex"],
                            env=dict(case.env, WORKSHOP_HOME=str(home)), capture_output=True,
                            text=True, check=True, timeout=30)
    assert "fixture-fast" in result.stdout
    assert list(folder.iterdir()) == [folder / "notes.txt"], "model lookup modified the folder"
    assert not home.exists(), "model lookup created Workshop storage"
    case.hold("left")
    proc = case.run("run", "delegate")
    try:
        case.started("left")
        assert "fixture-fast" in case.call("models", "--provider", "codex")
    finally:
        case.release("left")
        finish(proc, True)


def steer_preparing_worker(case):
    case.call("team", "--parallelism", "1", "--worker-model", "fixture-fast")
    (case.root / "record-inference").touch()
    case.hold("catalog")
    proc = case.run("run", "delegate")
    try:
        wait_for(lambda: (case.root / "catalog.ready").exists(), "worker did not start preparation")
        proc.stdin.write(b"/steer-worker 1 Preserve the original proportions\n")
        proc.stdin.flush()
        def routed():
            with sqlite3.connect(case.db) as db:
                return db.execute("SELECT 1 FROM events WHERE kind='worker/direction'").fetchone()
        wait_for(routed, "direction was not routed during preparation")
    finally:
        case.release("catalog")
        finish(proc, True)
    requests = [json.loads(path.read_text())["input"][0]["text"]
                for path in case.root.glob("worker-*.json")]
    assert len(requests) == 2
    for request in requests:
        assert ("Creator direction: Preserve the original proportions" in request) == request.startswith("Write left\n")
    with sqlite3.connect(case.db) as db:
        assert not db.execute("SELECT 1 FROM pending_worker_directions").fetchone()
        assert not db.execute("SELECT 1 FROM events WHERE kind='worker/delivery_error'").fetchone()


def interrupted_worker_directions(case):
    case.call("team", "--parallelism", "1", "--worker-model", "fixture-fast")
    for crash in (False, True):
        case.hold("catalog")
        proc = case.run("run", "delegate")
        marker = case.root / "catalog.ready"
        try:
            wait_for(marker.exists, "worker did not start model discovery")
            with sqlite3.connect(case.db) as db:
                worker, lead = db.execute("SELECT id, lead FROM workers WHERE state='running'").fetchone()
            proc.stdin.write(("/steer-worker %d Preserve the original proportions\n" % worker).encode())
            proc.stdin.flush()
            def pending():
                with sqlite3.connect(case.db) as db:
                    return db.execute("SELECT 1 FROM pending_worker_directions WHERE worker=?", (worker,)).fetchone()
            wait_for(pending, "preparing worker direction was not saved")
            if crash:
                proc.kill()
            else:
                proc.stdin.write(b"/cancel\n")
                proc.stdin.flush()
            proc.communicate(timeout=15)
        finally:
            if proc.poll() is None:
                proc.kill()
                proc.communicate(timeout=10)
            case.release("catalog")
        marker.unlink()
        case.call("open")
        case.call("open")
        assert not pending()
        with sqlite3.connect(case.db) as db:
            notices = db.execute("SELECT summary FROM events WHERE attempt=? AND kind='worker/delivery_error'", (lead,)).fetchall()
            assert len(notices) == 1 and "Preserve the original proportions" in notices[0][0], notices
            assert "Queued for Write left" in notices[0][0], notices


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


def foreign_git_environment(case):
    caller = case.root / "caller"
    subprocess.run(["git", "init", "--quiet", str(caller)], check=True)
    notes = caller / "notes.txt"
    notes.write_text("staged work")
    subprocess.run(["git", "-C", str(caller), "add", "notes.txt"], check=True)
    index = caller / ".git/index"
    staged = index.read_bytes()
    notes.write_text("unfinished changes")
    case.env.update(GIT_DIR=str(caller / ".git"), GIT_INDEX_FILE=str(index),
                    GIT_WORK_TREE=str(caller))
    (case.root / "check-git-context").touch()
    direct_repair(case)
    assert notes.read_text() == "unfinished changes" and index.read_bytes() == staged
    assert not (caller / "answer.txt").exists()


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

    (case.root / "direct-repair").unlink()
    marker.touch()
    case.call("run", "Confirm the release destination.", ok=False)
    marker.unlink()
    (case.root / "small-talk").touch()
    check = [case.env["WORKSHOP_CODEX"], "check"]
    case.call("team", "--check-args", json.dumps(check))
    (case.root / "fail-check").touch()
    case.call("resume", "2", "Use the playtest destination.", ok=False)
    case.call("accept", "2", ok=False)
    (case.root / "fail-check").unlink()
    case.call("resume", "2", "Confirmed. The checks are ready.")
    case.call("accept", "2")
    with sqlite3.connect(case.db) as db:
        assert db.execute("SELECT count(*) FROM events WHERE attempt=2 AND kind='workshop/blocker_resolved'").fetchone() == (1,)


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


def steer_during_check(case):
    (case.root / "direct-repair").touch()
    (case.root / "delivery-mode").touch()
    case.hold("check")
    case.call("team", "--check-args", json.dumps([case.env["WORKSHOP_CODEX"], "content-check"]))
    proc = case.run("run", "Update the answer.")
    marker = case.root / "check.ready"
    wait_for(lambda: marker.exists() and marker.stat().st_size, "check never started")
    pid = int(marker.read_text())
    proc.stdin.write(b"/queue Follow up after verification\n")
    proc.stdin.flush()
    def queued():
        with sqlite3.connect(case.db) as db:
            return db.execute("SELECT count(*) FROM queued_messages").fetchone() == (1,)
    wait_for(queued, "queue should be available during checks")
    case.release("check")
    proc.stdin.write(b"Please ensure answer.txt must contain correct\n")
    proc.stdin.flush()
    finish(proc, True)
    assert exited(pid)
    assert (case.root / "delivery.log").read_text().splitlines() == [
        "Update the answer.", "Please ensure answer.txt must contain correct", "Follow up after verification"]
    case.call("accept", "1")
    assert (case.project / "answer.txt").read_text().strip() == "correct"


def model_effort(case):
    (case.root / "record-inference").touch()
    global_policy = {"models": ["claude:default"], "max_effort": "medium"}
    with sqlite3.connect(case.home / "preferences.sqlite") as db:
        db.execute("INSERT INTO worker_defaults(id, policy) VALUES(1, ?)", (json.dumps(global_policy),))
    other = case.root / "second-project"
    other.mkdir()
    def other_policy():
        data = subprocess.check_output([case.cmd[0], "--project", str(other), "team"], env=case.env)
        return json.loads(data)["worker_policy"]
    assert json.loads(case.call("team"))["worker_policy"] == global_policy == other_policy()
    assert json.loads(case.call("team", "--worker-model", "fixture-fast"))["worker_policy"] == {
        "models": ["fixture-fast"], "max_effort": "medium"}
    case.call("team", "--lead-model", "fixture-deep", "--lead-effort", "high",
              "--worker-effort", "low")
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
    assert other_policy() == global_policy
    assert json.loads(case.call("team", "--inherit-workers"))["worker_policy"] == global_policy
    assert json.loads(case.call("team", "--worker-effort", "low"))["worker_policy"] == {
        "models": ["claude:default"], "max_effort": "low"}
    assert other_policy() == global_policy


def claude_worker_stop_and_resume(case):
    case.call("team", "--lead-model", "claude:default", "--worker-model", "claude:default")
    stop_one(case)
    with sqlite3.connect(case.db) as db:
        assert db.execute("SELECT provider,state FROM attempts").fetchall() == [("claude","accepted")]


def claude_without_codex(case):
    case.env["WORKSHOP_CODEX"] = str(case.root / "no-codex.exe")
    assert "claude:default" in case.call("models")
    case.call("run", "--provider", "claude", "delegate")
    with sqlite3.connect(case.db) as db:
        assert db.execute("SELECT provider,state FROM attempts").fetchone() == ("claude", "review")
        assert db.execute("SELECT model,state FROM workers ORDER BY id").fetchall() == [
            ("claude:default", "finished"), ("claude:default", "finished")]
    case.call("accept", "1")


def mixed_provider_conflict(case):
    case.call("team", "--lead-model", "claude:default", "--worker-model", "fixture-fast")
    conflict(case)


def delivery(case, provider):
    (case.root / "delivery-mode").touch()
    case.hold("right")
    proc = case.run("run", "--provider", provider, "delegate")
    def send(text):
        proc.stdin.write((text + "\n").encode())
        proc.stdin.flush()
    try:
        wait_for(lambda: (case.root / "waiting-worker").exists(), "worker not ready for direction")
        case.started("right")
        send("/queue Follow up one")
        send("/queue Follow up two")
        def pending():
            with sqlite3.connect(case.db) as db:
                return [r[0] for r in db.execute("SELECT body FROM queued_messages ORDER BY id")]
        wait_for(lambda: pending() == ["Follow up one", "Follow up two"], "queue did not retain FIFO order")
        send("/steer-worker 1 Use the blue version")
        wait_for(lambda: case.workers()[0][1] == "review", "worker did not act on direction")
        assert case.workers()[1][1] == "running"
        assert (case.root / "delivery.log").read_text().splitlines() == ["delegate"]
    finally:
        case.release("right")
        if not (case.root / "waiting-worker").exists() or case.workers()[0][1] == "running":
            send("/cancel")
        finish(proc, True)
    assert "To Write left" in (case.root / "lead-feedback").read_text()
    assert "Use the blue version" in (case.root / "lead-feedback").read_text()
    assert (case.root / "delivery.log").read_text().splitlines() == ["delegate", "Follow up one", "Follow up two"]
    with sqlite3.connect(case.db) as db:
        assert db.execute("SELECT summary FROM events WHERE kind='user/followup' ORDER BY id").fetchall() == [
            ("Follow up one",), ("Follow up two",)]
        assert not pending()
    case.call("accept", "1")
    assert (case.project / "left.txt").read_text().strip() == "blue"
    assert (case.project / "right.txt").read_text().strip() == "right"
    proc = case.run("run", "--provider", provider, "Wait for a correction")
    wait_for(lambda: "Wait for a correction" in (case.root / "delivery.log").read_text(), "lead did not start")
    send("Actually use green")
    finish(proc, True)
    assert (case.root / "delivery.log").read_text().splitlines()[-1] == "Actually use green"
    with sqlite3.connect(case.db) as db:
        assert db.execute("SELECT kind,summary FROM events WHERE attempt=2 AND kind LIKE 'steer/%'").fetchall() == [
            ("steer/sent", "Actually use green")]
    proc = case.run("run", "--provider", provider, "Wait for a correction")
    wait_for(lambda: (case.root / "delivery.log").read_text().count("Wait for a correction") == 2,
             "second lead did not start")
    send("/queue Follow up after restart")
    wait_for(lambda: pending() == ["Follow up after restart"], "missing pending message")
    send("/cancel")
    finish(proc, True)
    assert pending() == ["Follow up after restart"]
    assert "Follow up after restart" not in (case.root / "delivery.log").read_text()
    case.call("send-next", "3")
    assert not pending()
    assert (case.root / "delivery.log").read_text().splitlines().count("Follow up after restart") == 1


def interrupted_directions(case):
    case.env["WORKSHOP_GEMINI"] = case.env["WORKSHOP_ANTIGRAVITY"] = case.env["WORKSHOP_CODEX"]
    (case.root / "record-queue").touch()
    (case.root / "delivery-mode").touch()
    original = "Follow up with the lobby plan"
    direction = "Change the lobby entrance"
    followup = "Follow up with a review"
    original_file = case.root / "brief.txt"
    queued_file = case.root / "reference.txt"
    original_file.write_text("original attachment marker")
    queued_file.write_text("queued attachment marker")
    wire = case.root / "queue-wire.jsonl"
    for provider in ("codex", "claude", "gemini", "antigravity"):
        case.hold("init")
        proc = case.run("run", "--provider", provider, original)
        try:
            wait_for(lambda: (case.root / "init.ready").exists(), provider + " never reached initialization")
            with sqlite3.connect(case.db) as db:
                attempt = db.execute("SELECT max(id) FROM attempts").fetchone()[0]
            proc.stdin.write((direction + "\n/queue " + followup + "\n").encode())
            proc.stdin.flush()
            def pending():
                with sqlite3.connect(case.db) as db:
                    return db.execute("SELECT body FROM queued_messages WHERE attempt=? ORDER BY id", (attempt,)).fetchall()
            wait_for(lambda: pending() == [(direction,), (followup,)], provider + " did not retain the inputs")
            with sqlite3.connect(case.db) as db:
                assert not db.execute("SELECT 1 FROM events WHERE attempt=? AND kind='steer/sent'", (attempt,)).fetchone()
            if provider == "gemini":
                proc.kill()
                proc.communicate(timeout=10)
            else:
                proc.stdin.write(b"/cancel\n")
                proc.stdin.flush()
                finish(proc, True)
        finally:
            if proc.poll() is None:
                proc.kill()
                proc.communicate(timeout=10)
            case.release("init")
        (case.root / "init.ready").unlink()
        case.call("open")
        case.call("open")
        assert pending() == [(followup,)]
        with sqlite3.connect(case.db) as db:
            notices = db.execute("SELECT kind,summary FROM events WHERE attempt=? AND kind LIKE 'steer/%'", (attempt,)).fetchall()
            assert len(notices) == 1 and notices[0][0] == "steer/unavailable" and direction in notices[0][1], (provider, notices)
            assert db.execute("SELECT session FROM attempts WHERE id=?", (attempt,)).fetchone() == ("",)
            # A desktop sender can attach files to its original and queued messages.
            db.execute("INSERT INTO task_attachments(task,path) SELECT task,? FROM attempts WHERE id=?",
                       (str(original_file), attempt))
            db.execute("UPDATE queued_messages SET files=? WHERE attempt=?", (json.dumps([str(queued_file)]), attempt))
        count = len(wire.read_text().splitlines()) if wire.is_file() else 0
        retry = provider == "claude"
        case.call("retry" if retry else "send-next", str(attempt))
        frames = wire.read_text().splitlines()[count:]
        assert len(frames) == 2, (provider, frames)
        assert original in frames[0] and "original attachment marker" in frames[0] and "queued attachment marker" not in frames[0]
        assert followup in frames[1] and "queued attachment marker" in frames[1] and "original attachment marker" not in frames[1]
        with sqlite3.connect(case.db) as db:
            current = db.execute("SELECT max(id) FROM attempts").fetchone()[0]
            assert (current != attempt) == retry, (provider, current, attempt)
            assert not db.execute("SELECT 1 FROM queued_messages").fetchone()
            assert db.execute("SELECT summary FROM events WHERE attempt=? AND kind='user/followup'",
                              (current,)).fetchall() == [(followup,)]


def rejected_codex_direction(case):
    (case.root / "reject-direction").touch()
    (case.root / "delivery-mode").touch()
    proc = case.run("run", "Wait for a correction")
    try:
        wait_for(lambda: (case.root / "delivery.log").exists(), "lead did not start")
        proc.stdin.write(b"Actually use blue\n")
        proc.stdin.flush()
        def rejected():
            with sqlite3.connect(case.db) as db:
                return db.execute("SELECT kind,summary FROM events WHERE kind LIKE 'steer/%'").fetchall()
        wait_for(lambda: rejected(), "missing provider receipt")
        assert rejected() == [("steer/unavailable", "Turn no longer accepts directions\nActually use blue")]
    finally:
        proc.stdin.write(b"/cancel\n")
        proc.stdin.flush()
        finish(proc, True)


def late_codex_receipt(case):
    (case.root / "late-receipt").touch()
    (case.root / "delivery-mode").touch()
    proc = case.run("run", "Wait for a correction")
    wait_for(lambda: (case.root / "delivery.log").exists(), "lead did not start")
    proc.stdin.write(b"/queue Follow up on the same chat\nActually use green\n")
    proc.stdin.flush()
    finish(proc, True)
    assert (case.root / "delivery.log").read_text().splitlines() == ["Wait for a correction", "Follow up on the same chat"]
    with sqlite3.connect(case.db) as db:
        receipts = db.execute("SELECT kind,summary FROM events WHERE kind LIKE 'steer/%' OR kind='user/followup' ORDER BY id").fetchall()
        assert [kind for kind, _ in receipts] == ["steer/unavailable", "user/followup"], receipts
        assert "Actually use green" in receipts[0][1]


def shared_blender_connection(case):
    fixture = case.env["WORKSHOP_CODEX"]
    case.env.update(WORKSHOP_GEMINI=fixture, WORKSHOP_ANTIGRAVITY=fixture)
    (case.root / "capture-mcp").touch()
    case.call("gemini-trust", "true")
    assert "1 tool." in case.call("blender-mcp", "connect")
    assert "1 tool." in case.call("blender-mcp", "test")
    for source in ("codex", "claude", "antigravity", "gemini"):
        case.call("run", "--provider", source, "hi")
        payload = json.loads((case.root / ("mcp-" + source + ".json")).read_text())
        if source == "codex":
            server = payload["mcp_servers.blender"]
        elif source == "claude":
            server = payload["mcpServers"]["blender"]
        else:
            assert any(s["name"] == "workshop" for s in payload)
            server = next(s for s in payload if s["name"] == "blender")
        assert Path(server["command"]) == Path(fixture) and server["args"] == ["--fixture-blender"]
    case.call("blender-mcp", "disconnect")
    case.call("run", "--provider", "codex", "hi")
    assert "mcp_servers.blender" not in json.loads((case.root / "mcp-codex.json").read_text())


def interrupted_goal(case):
    (case.root / "goal-mode").touch()
    case.hold("goal")
    proc = case.run("run", "/goal Finish both fixture files")
    def has_session():
        with sqlite3.connect(case.db) as db:
            return db.execute("SELECT 1 FROM attempts WHERE id=1 AND state='running' AND session<>''").fetchone()
    try:
        wait_for(has_session, "goal did not connect")
        assert json.loads(case.call("status", "--json"))[0]["status"] == "In progress"
        work = case.db.parent / "attempt-1/work"
        (work / "first.txt").write_text("one")
    finally:
        proc.kill()
        proc.communicate(timeout=10)
        case.release("goal")
    case.call("recover-goal", "1")
    with sqlite3.connect(case.db) as db:
        assert db.execute("SELECT state,status FROM attempts JOIN chat_goals ON attempt=id").fetchall() == [("review","complete")]
    assert (work / "second.txt").read_text() == "two" and not (case.project / "second.txt").exists()
    assert "No interrupted goal" in case.call("recover-goal")


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
        for test in (shared_blender_connection, interrupted_goal, interrupted_directions, rejected_codex_direction, late_codex_receipt, autonomous_release, direct_repair, foreign_git_environment, unavailable_runner,
                     small_talk, chat_goals, manual_recheck, external_blocker, mixed_plan, conflict,
                     stop_one, stop_preparing_worker, steer_preparing_worker, interrupted_worker_directions, cancel_check, steer_during_check, overlap,
                     repeated_check, model_effort, read_only_models, claude_worker_stop_and_resume,
                     mixed_provider_conflict, claude_without_codex):
            test(Case(base / test.__name__, args.cli.resolve(), fixture))
            print(test.__name__ + " passed", flush=True)
        for provider in ("codex", "claude"):
            delivery(Case(base / ("delivery-" + provider), args.cli.resolve(), fixture), provider)
            print(provider + " queue and steering passed", flush=True)


if __name__ == "__main__":
    main()
