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
import time
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
        doc = json.loads(subprocess.check_output([cli, "query", clone, "project"], env=clone_env))["result"]["documents"][0]["id"]
        def instance_nodes(project):
            tree = json.loads(subprocess.check_output([cli, "query", project, "tree", doc], env=clone_env))["result"]
            return [row["id"] for row in tree["nodes"] if row["kind"] == "component-instance"]

        with subprocess.Popen([app, "--component-library-check", clone_home], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, env=clone_env) as proc:
            def command(line):
                proc.stdin.write(line + "\n")
                proc.stdin.flush()
                if not select.select([proc.stdout], [], [], 30)[0]:
                    raise TimeoutError("native instance action: " + line)
                return json.loads(proc.stdout.readline())
            command("initial")
            command("fit")
            command("select " + card["id"])
            first = command("place 30 20")
            assert first["message"] == "Instance placed." and first["selection"] == 1, first
            again = command("place 230.5 -10")
            assert again["message"] == "Instance placed." and again["undo"] == first["undo"] + 1, again
            assert command("undo")["undo"] == first["undo"]
            assert command("redo")["undo"] == again["undo"]
            assert command("save")["generation"] > first["generation"]
            ids = set(instance_nodes(clone))
            assert len(ids) == 2

            def wait_canvas(label, predicate):
                deadline = time.monotonic() + 10
                while True:
                    sample = command("canvas")
                    if predicate(sample["samples"]):
                        return sample
                    if time.monotonic() >= deadline:
                        raise AssertionError(f"{label}: {sample}")
                    time.sleep(.05)

            visible = wait_canvas("two independently pickable instance roots", lambda s:
                                  {s[0]["id"], s[1]["id"]} == ids)
            initial = visible["samples"]
            first_id, second_id, view_root = initial[0]["id"], initial[1]["id"], initial[6]["id"]
            assert view_root and view_root not in ids
            assert [p["id"] for p in initial] == [first_id, second_id, first_id, second_id, first_id, second_id, view_root]
            assert [(p["x"], p["y"]) for p in initial[:2]] == [(30, 20), (230.5, -10)]
            assert all(min(p["rgb"]) >= 250 for p in initial[:4]), initial
            revision = visible["revision"]

            def pixels_changed(s):
                # Both roots are blue, the unchanged children stay white, the trimmed regions reveal the View.
                return ([p["id"] for p in s] == [first_id, second_id, first_id, second_id, view_root, view_root, view_root]
                        and all(p["rgb"][2] > 240 and max(p["rgb"][:2]) < 12 for p in s[:2])
                        and all(min(p["rgb"]) >= 250 for p in s[2:4])
                        and all(p["rgb"] == initial[6]["rgb"] for p in s[4:]))

            # Keep the document generation and original SHA while replacing only this definition, as a checkout would.
            old_path = clone / path.relative_to(source)
            original_source = old_path.read_bytes()
            # Isolate a real one-file source diff from the earlier native Save Now.
            clone_git = ["git", "-C", clone]
            subprocess.run([*clone_git, "add", "-A"], env=clone_env, check=True)
            subprocess.run([*clone_git, "commit", "-qm", "persist placed instances"], env=clone_env, check=True)
            def edit_fill(action, value):
                command("fill-open")
                changed = command("fill-" + action + " " + value)
                assert changed["selected"] == card["id"] and not changed["message"], changed
                return changed["fill"]

            assert edit_fill("cancel", "2 0.5 0.125 0.75 0.5") == [1, 1, 1, 1, 1]
            assert old_path.read_bytes() == original_source
            assert edit_fill("apply", "1 0 0 1 0.5") == [1, 0, 0, 1, 0.5]
            assert old_path.read_bytes() == original_source.replace(b'rgba: "1 1 1 1"', b'rgba: "0 0 1 0.5"', 1)
            changed_paths = subprocess.check_output([*clone_git, "status", "--porcelain", "--untracked-files=all"], env=clone_env, text=True)
            assert changed_paths.splitlines() == [" M " + path.relative_to(source).as_posix()], changed_paths
            def both_translucent_blue(samples):
                r, g, b = samples[0]["rgb"]
                return (samples[1]["rgb"] == [r, g, b] and b > r + 40 and abs(r - g) < 4
                        and r > 10 and [p["id"] for p in samples] == [p["id"] for p in initial]
                        and samples[2:4] == initial[2:4])
            assert wait_canvas("native translucent fill at both positions", both_translucent_blue)["revision"] == revision

            annotated = b"# preserve unchanged colors verbatim\n" + old_path.read_bytes()
            old_path.write_bytes(annotated)
            command("refresh")
            assert edit_fill("apply", "1 0 0 1 0.5") == [1, 0, 0, 1, 0.5]
            assert old_path.read_bytes() == annotated
            extended = annotated.replace(b'rgba: "0 0 1 0.5"', b'rgba: "-0.125 1.25 1 0.5"')
            old_path.write_bytes(extended)
            command("refresh")
            assert edit_fill("apply", "unchanged") == [1, -0.125, 1.25, 1, 0.5]
            assert old_path.read_bytes() == extended
            assert edit_fill("apply", "none") is None
            assert b'background: none' in old_path.read_bytes()
            assert edit_fill("apply", "2 0.5 0.125 0.75 0") == [2, 0.5, 0.125, 0.75, 0]
            p3_source = old_path.read_bytes()
            assert b'display-p3' in p3_source and b'rgba: "0.5 0.125 0.75 0"' in p3_source
            command("reopen")
            command("fit")
            selected = command("select " + card["id"])
            assert selected["fill"] == [2, 0.5, 0.125, 0.75, 0]
            command("fill-open")
            subprocess.run([*clone_git, "restore", "--", old_path], env=clone_env, check=True)
            command("refresh")  # A focus-triggered refresh must retain the sheet's original hash.
            rejected = command("fill-apply 1 1 0 0 1")
            assert rejected["message"] and rejected["selected"] == card["id"], rejected
            assert old_path.read_bytes() == original_source
            assert command("refresh")["fill"] == [1, 1, 1, 1, 1]
            before_stale = command("refresh")
            revision = command("canvas")["revision"]
            # Replacing only the definition again simulates a checkout between reading and placing.
            subprocess.run([cli, "component-source", "resize", old_path, card["source_hash"], "201", "77"], env=clone_env, check=True, stdout=subprocess.DEVNULL)
            named_sections = old_path.read_text().split('root_node:', 1)
            assert len(named_sections) == 2
            head, root_tail = named_sections
            assert 'rgba: "1 1 1 1"' in root_tail
            old_path.write_text(head + 'root_node:' + root_tail.replace('rgba: "1 1 1 1"', 'rgba: "0 0 1 1"', 1))
            blue_source = old_path.read_bytes()
            stale = command("place 10 10")
            assert stale["message"] != "Instance placed." and stale["undo"] == before_stale["undo"], stale
            assert command("refresh")["entries"][0]["width"] == 201
            assert wait_canvas("external edit updates both instances", pixels_changed)["revision"] == revision
            subprocess.run(["git", "-C", clone, "restore", "--source=HEAD", "--", old_path], env=clone_env, check=True)
            assert old_path.read_bytes() == original_source
            command("refresh")
            assert wait_canvas("git restore repaints both original instances", lambda s: s == initial)["revision"] == revision

            def no_instances(s):
                return all(p["id"] == view_root and p["rgb"] == initial[6]["rgb"] for p in s)
            for broken in [None, b"invalid yaml without a component"]:
                if broken is None:
                    old_path.unlink()
                else:
                    old_path.write_bytes(broken)
                assert not command("refresh")["entries"]
                assert wait_canvas("missing or invalid definition clears stale paint", no_instances)["revision"] == revision
                old_path.write_bytes(blue_source)
                command("refresh")
                assert wait_canvas("definition restored at the same revision", pixels_changed)["revision"] == revision
            restored = command("reopen")
            assert restored["entries"][0]["width"] == 201
            command("fit")
            wait_canvas("reopened instances resolve to updated source", pixels_changed)
            assert not command("revoke")["entries"]
            wait_canvas("trust revocation clears geometry and pick targets", no_instances)
            proc.stdin.write("done\n"); proc.stdin.flush()
            assert proc.wait(timeout=15) == 0
        ids = instance_nodes(clone)
        assert len(set(ids + [card["id"], card["root"]])) == 4
        for node in ids:
            detail = json.loads(subprocess.check_output([cli, "query", clone, "node", doc, node], env=clone_env))["result"]["node"]
            assert detail["modifier_count"] == 1 and card["id"] in json.dumps(detail["properties"])
        print("component root fill ok: native Apply/Cancel, translucent pixels at both instances, single-file Git diff, no fill, transparent Display P3 reopen, byte-preserving no-op and stale open sheet")
        print("instance rendering ok: two independent picks and offsets, nested hits, size/fill propagate at same document revision, Git restore, missing/invalid repair, reopen and trust revocation")
        print("instance placement ok: native Place twice at different positions, Undo/Redo, Save Now, reopen, 2 independent kind-7 nodes sharing original definition, stale source rejected")
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
                read_panel("fill-open")
                changed = read_panel("fill-apply 1 0.25 0.5 0.75 1")
                assert changed["fill"] == [1, 0.25, 0.5, 0.75, 1] and not changed["message"]
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
