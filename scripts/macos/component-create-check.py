#!/usr/bin/env python3
"""앱의 메뉴로 만든 두 파일과 CLI 프로젝트/폴더 조회의 ID·hash를 비교합니다."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import uuid


def main(app, cli):
    with tempfile.TemporaryDirectory(prefix="stellar-component-create-") as temporary:
        home = Path(temporary)
        env = dict(os.environ, HOME=str(home), GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull,
                   GIT_ALLOW_PROTOCOL="file", GIT_AUTHOR_NAME="Fixture", GIT_COMMITTER_NAME="Fixture",
                   GIT_AUTHOR_EMAIL="fixture@example.invalid", GIT_COMMITTER_EMAIL="fixture@example.invalid")
        subprocess.run([app, "--component-create-check", home], env=env, check=True, timeout=90)
        project = home / "Component Project"
        folder = project / "design/components"

        def result(*args):
            return json.loads(subprocess.check_output([cli, *args], env=env))["result"]

        entries = result("query", project, "components")
        standalone = result("component-source", "check", folder)["components"]
        assert entries["chunk_count"] == 0 and entries["revision"]["generation"] == 1
        assert len(standalone) == len(entries["entries"]) == 2
        expected_files = set()
        ids = set()
        for item, queried in zip(standalone, entries["entries"]):
            path = Path(item.pop("file"))
            assert item == queried and item["name"] == "버튼" and item["node_count"] == 1
            component = uuid.UUID(item["id"])
            root = uuid.UUID(item["root"])
            assert component.version == root.version == 7
            ids.update([component, root])
            suffix = hashlib.sha256(component.bytes).hexdigest()[:12]
            assert path == folder / f"버튼-{suffix}.stellar.yaml"
            assert item["source_hash"] == "sha256:" + hashlib.sha256(path.read_bytes()).hexdigest()
            expected_files.add(str(path.relative_to(project)))
        assert len(ids) == 4
        git = ["git", "-C", project]
        subprocess.run([*git, "diff", "--quiet", "HEAD", "--"], env=env, check=True)
        untracked = subprocess.check_output([*git, "ls-files", "--others", "--exclude-standard", "-z"], env=env)
        assert set(untracked.decode().split("\0")[:-1]) == expected_files
        print("component-create disk ok: CLI project and folder agree, only two untracked files, four unique UUIDv7 IDs")


if __name__ == "__main__":
    main(*(Path(arg).resolve() for arg in sys.argv[1:]))
