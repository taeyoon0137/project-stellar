#!/usr/bin/env python3
"""실제 CLI 저장 파일, macOS 패널 클릭, 시스템 Git index 재열기를 한 임시 홈에서 검증합니다."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile


def main(app, cli, fixture):
    node = "00000000-000b-7000-8000-00000000000b"
    with tempfile.TemporaryDirectory(prefix="stellar-git-stage-") as tmp:
        base = Path(tmp)
        work = base / "project"
        home = base / "home"
        home.mkdir()
        shutil.copytree(fixture, work)
        env = dict(os.environ, HOME=str(home), GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull,
                   GIT_TERMINAL_PROMPT="0", GIT_ALLOW_PROTOCOL="file")

        def git(*args, **options):
            return subprocess.run(["git", "-C", work, *args], env=env, check=True, capture_output=True, **options).stdout

        def stellar(*args, data=None):
            result = subprocess.run([cli, *args], env=env, text=True, input=data, capture_output=True)
            if result.returncode:
                raise RuntimeError(result.stderr or result.stdout)
            return json.loads(result.stdout)["result"]

        git("init", "-q")
        git("add", ".")
        git("-c", "user.name=Stellar Fixture", "-c", "user.email=fixture@example.invalid", "commit", "-qm", "baseline")
        project = stellar("query", work, "project")
        document = project["documents"][0]["id"]

        def edit(commands):
            revision = stellar("query", work, "project")["revision"]["token"]
            request = dict(format="stellar-apply", format_version=1, base_revision=revision,
                           document=document, commands=commands)
            stellar("apply", work, "-", data=json.dumps(request, ensure_ascii=False))

        def rename(name):
            return {"kind": "rename", "node": node, "name": name}

        original_name = stellar("query", work, "node", document, node)["node"]["name"]
        edit([rename("Canvas B")])
        current = work / "design/current"
        before = current.read_bytes()
        edit([rename("Canvas C"), {"kind": "set-locked", "node": node, "locked": True}])
        after = base / "next-generation"
        after.write_bytes(current.read_bytes())
        current.write_bytes(before)

        trust = home / "Library/Application Support/xyz.taeyoon.stellar/trust/trusted-projects.yaml"
        trust.parent.mkdir(parents=True)
        identity = project["project"]["id"]
        target = json.dumps(str(work.resolve()))
        trust.write_text(f"schema_version: 1\ntrusted:\n  - target: {target}\n    project_id: {identity}\n")

        def project_files():
            files = {}
            for path in work.rglob("*"):
                relative = path.relative_to(work)
                if relative.parts[0] != ".git" and path.is_file():
                    files[relative] = hashlib.sha256(path.read_bytes()).digest()
            return files

        expected_files = project_files()
        expected_files[current.relative_to(work)] = hashlib.sha256(after.read_bytes()).digest()
        subprocess.run([app, "--git-stage-check", work, after], env=env, check=True, timeout=75)
        assert project_files() == expected_files, "Stage/Unstage changed the saved project files"

        # Git 자신의 checkout-index 결과를 별도로 열고, working과 같은 노드의 이름·잠금을 비교합니다.
        def stored_node(directory, index_file):
            folder = base / directory
            folder.mkdir()
            subprocess.run(["git", "init", "-q", folder], env=env, check=True)
            subprocess.run(["git", "-C", work, "checkout-index", "-a", "--prefix=" + str(folder) + "/"],
                           env=dict(env, GIT_INDEX_FILE=str(index_file)), check=True)
            return stellar("query", folder, "node", document, node)["node"]

        staged = stored_node("staged", after.with_suffix(".staged-index"))
        unstaged = stored_node("unstaged", work / ".git/index")
        working = stellar("query", work, "node", document, node)["node"]
        assert staged["name"] == working["name"] == "Canvas C", (staged, working)
        assert unstaged["name"] == original_name, (unstaged, original_name)
        assert not staged["locked"] and not unstaged["locked"] and working["locked"]
        print("git-stage disk ok: name staged, then unstaged to HEAD; working name and lock preserved")


if __name__ == "__main__":
    main(*(Path(arg).resolve() for arg in sys.argv[1:]))
