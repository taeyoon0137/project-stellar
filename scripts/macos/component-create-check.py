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
        fixture = home / "selected"
        fixture.mkdir()
        app_env = dict(env, HOME=str(fixture))
        subprocess.run([app, "--selected-component-check", fixture], env=app_env, check=True, timeout=90)
        source = fixture / "Component Project"
        rows = json.loads(subprocess.check_output([cli, "query", source, "components"], env=app_env))["result"]
        assert rows["revision"]["generation"] == 1 and rows["chunk_count"] == 0
        [card] = rows["entries"]
        assert (card["name"], card["node_count"], float(card["width"]), float(card["height"])) == ("Card", 3, 230, 97)
        git = ["git", "-C", source]
        [path] = (source / "design/components").glob("*.stellar.yaml")
        assert subprocess.check_output([*git, "status", "--porcelain", "--untracked-files=all"], env=app_env).decode().splitlines() == [
            "?? " + path.relative_to(source).as_posix()]
        subprocess.run([*git, "add", "--", path], env=app_env, check=True)
        subprocess.run([*git, "commit", "-qm", "selection component"], env=app_env, check=True)
        clone_home = home / "clone"
        clone_home.mkdir()
        clone = clone_home / "Component Project"
        subprocess.run(["git", "clone", "--quiet", "--no-local", source, clone], env=app_env, check=True)
        clone_env = dict(env, HOME=str(clone_home))
        assert (clone / path.relative_to(source)).read_bytes() == path.read_bytes()
        assert json.loads(subprocess.check_output([cli, "query", clone, "components"], env=clone_env))["result"] == rows
        for folder, instance_env in [(fixture, app_env), (clone_home, clone_env)]:
            run = subprocess.run([app, "--component-library-check", folder], input="initial\ndone\n", text=True,
                                 capture_output=True, env=instance_env, check=True, timeout=45)
            [item] = json.loads(run.stdout)["entries"]
            assert item.pop("filename") == path.name
            assert item == dict(card, width=float(card["width"]), height=float(card["height"]))
        print("selected-component ok: original and local Git clone opened in native Component Library and CLI, identical IDs/hash/tree, single changed source")
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
        original = next(row for row in result("component-source", "check", folder)["components"] if row["name"] == "버튼")
        original_path = Path(original["file"])
        result("component-source", "resize", original_path, original["source_hash"], "42", "11")
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
                read_panel("select " + original["id"])
                untouched = {path: path.read_bytes() for path in folder.glob("*.stellar.yaml") if path != original_path}
                changed = read_panel("resize 248.5 44")
                assert not changed["message"]
                assert all(path.read_bytes() == expected for path, expected in untouched.items())
                paths = subprocess.check_output([*git, "diff", "--name-only", "-z"], env=env)
                assert paths.decode().split("\0")[:-1] == [str(original_path.relative_to(project))]
                resized = next(e for e in compare("reopen")["entries"] if e["id"] == original["id"])
                assert (resized["width"], resized["height"]) == (248.5, 44)
                # The user sees this hash; a real checkout subsequently changes the same definition.
                read_panel("select " + original["id"])
                subprocess.run([*git, "restore", "--worktree", "--", original_path], env=env, check=True)
                subprocess.run([*git, "switch", "-q", "third"], env=env, check=True)
                before = {path: path.read_bytes() for path in folder.glob("*.stellar.yaml")}
                failed = read_panel("resize 600 60")
                assert failed["message"] and all(path.read_bytes() == expected for path, expected in before.items())
                assert len(compare()["entries"]) == 3
                read_panel("select " + original["id"])
                assert read_panel("resize -1 45")["message"]
                assert read_panel("resize nan 45")["message"]
                assert all(path.read_bytes() == expected for path, expected in before.items())
                subprocess.run([*git, "diff", "--quiet", "HEAD", "--"], env=env, check=True)
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
                subprocess.run([*git, "diff", "--quiet", "HEAD", "--"], env=env, check=True)
                process.stdin.write("done\n")
                process.stdin.flush()
                assert process.wait(timeout=15) == 0
            finally:
                if process.poll() is None:
                    process.kill()
        print("component-library ok: root resize and reopen, only selected file changed, stale checkout and invalid size preserved files, trust revocation clears panel")



if __name__ == "__main__":
    main(*(Path(arg).resolve() for arg in sys.argv[1:]))
