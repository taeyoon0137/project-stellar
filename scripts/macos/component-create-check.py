#!/usr/bin/env python3
"""앱의 메뉴로 만든 두 파일과 CLI 프로젝트/폴더 조회의 ID·hash를 비교합니다."""
import hashlib
import json
import os
import select
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

        subprocess.run([*git, "add", "design/components"], env=env, check=True)
        subprocess.run([*git, "commit", "-qm", "two library components"], env=env, check=True)
        original_revision = subprocess.check_output([*git, "rev-parse", "HEAD"], env=env, text=True).strip()
        subprocess.run([*git, "switch", "-qc", "third"], env=env, check=True)
        result("component-source", "create", folder, "Third", "0195c530-3acd-7bc1-8414-c213f61cb965",
               "0195c530-3acd-7bc1-8414-c213f61cb966")
        subprocess.run([*git, "add", "design/components"], env=env, check=True)
        subprocess.run([*git, "commit", "-qm", "third component"], env=env, check=True)
        subprocess.run([*git, "switch", "-q", "--detach", original_revision], env=env, check=True)

        with subprocess.Popen([app, "--component-library-check", home], stdin=subprocess.PIPE,
                              stdout=subprocess.PIPE, text=True, env=env) as process:
            def read_panel(action="refresh"):
                process.stdin.write(action + "\n")
                process.stdin.flush()
                if not select.select([process.stdout], [], [], 25)[0]:
                    raise TimeoutError("Component Library did not return its visible rows")
                return json.loads(process.stdout.readline())

            def compare(action="refresh"):
                visible = read_panel(action)
                expected = result("query", project, "components")["entries"]
                actual = visible["entries"]
                assert len(actual) == len(expected)
                sources = result("component-source", "check", folder)["components"] if folder.exists() else []
                filenames = {row["id"]: Path(row["file"]).name for row in sources}
                for row, expected_row in zip(actual, expected):
                    assert row.pop("filename") == filenames[expected_row["id"]]
                    assert row == dict(expected_row, width=float(expected_row["width"]), height=float(expected_row["height"]))
                return visible

            try:
                assert len(compare("initial")["entries"]) == 2
                subprocess.run([*git, "switch", "-q", "third"], env=env, check=True)
                assert len(compare()["entries"]) == 3
                broken = folder / "broken.stellar.yaml"
                broken.write_text("broken")
                failure = read_panel()
                assert failure["entries"] == [] and failure["message"]
                broken.unlink()
                assert len(compare()["entries"]) == 3
                subprocess.run([*git, "switch", "-q", "--detach", original_revision + "^"], env=env, check=True)
                assert compare()["message"]  # empty state
                subprocess.run([*git, "switch", "-q", "third"], env=env, check=True)
                assert len(compare()["entries"]) == 3
                assert read_panel("revoke")["entries"] == []
                process.stdin.write("done\n")
                process.stdin.flush()
                assert process.wait(timeout=15) == 0
            finally:
                if process.poll() is None:
                    process.kill()
        print("component-library ok: matches CLI after checkout, empty state, malformed file and recovery, trust revocation clears panel")



if __name__ == "__main__":
    main(*(Path(arg).resolve() for arg in sys.argv[1:]))
