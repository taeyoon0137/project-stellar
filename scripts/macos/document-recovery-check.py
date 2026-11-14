#!/usr/bin/env python3
"""강제 종료 → 새 앱 프로세스 → 복구 시트 → 일반 저장/별도 복구 문서."""
import hashlib
import json
import os
from pathlib import Path
import select
import subprocess
import sys
import tempfile
import time

app, cli = map(Path, sys.argv[1:])


def until(action):
    end = time.monotonic() + 15
    while not action():
        if time.monotonic() >= end:
            raise TimeoutError("checkpoint did not become durable")
        time.sleep(.025)


def snapshot(root):
    return {p.relative_to(root): hashlib.sha256(p.read_bytes()).hexdigest() for p in root.rglob("*")
            if p.is_file() and ".git" not in p.relative_to(root).parts}


for mode in ("restore", "export"):
    with tempfile.TemporaryDirectory(prefix="stellar-recovery-") as tmp:
        home = Path(tmp).resolve()
        env = dict(os.environ, HOME=str(home), GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull,
                   GIT_ALLOW_PROTOCOL="file", GIT_AUTHOR_NAME="Fixture", GIT_COMMITTER_NAME="Fixture",
                   GIT_AUTHOR_EMAIL="fixture@example.invalid", GIT_COMMITTER_EMAIL="fixture@example.invalid")
        root = home / "Crash Project"
        recovery = home / "Library/Application Support/xyz.taeyoon.stellar/recovery"

        def native(action):
            return [app, "--document-recovery-check", home, action]

        def query(project, what, *args):
            return json.loads(subprocess.check_output([cli, "query", project, what, *args], env=env))["result"]

        with subprocess.Popen(native("edit"), stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, env=env) as writer:
            try:
                assert select.select([writer.stdout], [], [], 30)[0]
                node = writer.stdout.readline().strip()
                assert len(node) == 36, (node, writer.poll())
                until(lambda: bool(list(recovery.rglob("*.starrecovery"))))
                assert not subprocess.check_output(["git", "-C", root, "status", "--porcelain"], env=env)
                # A second running process must never offer this live writer's checkpoint.
                live = subprocess.run(native("live"), capture_output=True, text=True, env=env, timeout=30)
                assert live.returncode == 0, live.stderr
                writer.kill()
            finally:
                if writer.poll() is None:
                    writer.kill()
                writer.wait(timeout=10)
        project = query(root, "project")
        document = project["documents"][0]["id"]
        baseline = query(root, "tree", document)["nodes"]
        assert node not in {n["id"] for n in baseline}
        if mode == "export":
            git = ["git", "-C", root]
            subprocess.run([*git, "switch", "-qc", "independent-edit"], env=env, check=True)
            tx = {"format": "stellar-apply", "format_version": 1, "base_revision": project["revision"]["token"],
                  "document": document, "commands": [{"kind": "rename", "node": baseline[0]["id"], "name": "External Frame"}]}
            subprocess.run([cli, "apply", root, "-"], input=json.dumps(tx), text=True, check=True, env=env, stdout=subprocess.DEVNULL)
            subprocess.run([*git, "add", "design"], env=env, check=True)
            subprocess.run([*git, "commit", "-qm", "separate external edit"], env=env, check=True)
            subprocess.run([*git, "switch", "-q", "main"], env=env, check=True)
            subprocess.run([*git, "switch", "-q", "independent-edit"], env=env, check=True)
        before = snapshot(root)
        p = subprocess.run(native(mode), capture_output=True, text=True, env=env, timeout=50)
        assert p.returncode == 0, (mode, p.returncode, p.stdout, p.stderr)
        if mode == "restore":
            assert p.stdout.strip() == node
            target = root
            assert query(root, "project")["revision"]["generation"] == 2
        else:
            assert p.stdout.strip() == "exported" and snapshot(root) == before
            assert not subprocess.check_output(["git", "-C", root, "status", "--porcelain"], env=env)
            assert query(root, "tree", document)["nodes"][0]["name"] == "External Frame"
            target = home / "Recovered"
        nodes = query(target, "tree", document)["nodes"]
        assert node in {n["id"] for n in nodes} and len(nodes) == len(baseline) + 1
        until(lambda: not list(recovery.rglob("*.starrecovery")))
        print(f"{mode}: real SIGKILL, native relaunch prompt, recovered Frame {node}, CLI verified", flush=True)
