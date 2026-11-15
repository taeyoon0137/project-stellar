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


def check_window_return(app, cli, home, original, env, card, filename, instance):
    project = home / "Component Project"
    home.mkdir()
    subprocess.run(["git", "clone", "-q", "--no-local", original, project], env=env, check=True)
    env = dict(env, HOME=str(home))
    definition = project / filename
    git = ["git", "-C", project]
    def document_hash(name):
        output = home / name
        subprocess.run([cli, "text-source", "export", project, output], env=env, check=True, stdout=subprocess.DEVNULL)
        return hashlib.sha256(output.read_bytes()).digest()

    saved = document_hash("before.stellar.yaml")
    # LaunchServices grants this isolated app the foreground so AppKit can make its project window key.
    input_pipe, output_pipe = home / "input.fifo", home / "output.fifo"
    os.mkfifo(input_pipe)
    os.mkfifo(output_pipe)
    stderr_file = home / "check.stderr"
    command = ["/usr/bin/open", "-n", "-W", "-a", app.parents[2], "--stdin", input_pipe, "--stdout", output_pipe,
               "--stderr", stderr_file, "--env", "HOME=" + str(home), "--args",
               "--component-library-check", home, "--project-window-return"]
    with (
        os.fdopen(os.open(input_pipe, os.O_RDWR), "w") as requests,
        os.fdopen(os.open(output_pipe, os.O_RDWR), "r") as replies,
        subprocess.Popen(command, env=env) as proc,
    ):
        def read(action):
            requests.write(action + "\n")
            requests.flush()
            if not select.select([replies], [], [], 30)[0]:
                detail = stderr_file.read_text() if stderr_file.is_file() else proc.poll()
                raise TimeoutError(f"Project window action {action}: {detail}")
            return json.loads(replies.readline())

        try:
            # Leave a real local layer rename and selection pending while another process edits the definition.
            read("focus-only")
            initial = read(instance)
            before = initial.pop("samples")
            assert initial["dirty"] == 1 and initial["selection"] == instance and initial["undo"] > 0, initial
            ids = [p["id"] for p in before]
            assert ids[4] == instance and ids[5] != instance and ids[6] not in ids[:6], ids
            read("leave")
            subprocess.run([cli, "component-source", "resize", definition, card["source_hash"], "201", "77"],
                           env=env, check=True, stdout=subprocess.DEVNULL)
            returned = read("return")
            after = returned.pop("samples")
            assert returned == initial, (initial, returned)
            assert [p["id"] for p in after] == ids[:4] + [ids[6]] * 3, ("CLI resize on window return", after)
            assert all(p["rgb"] == before[6]["rgb"] for p in after[4:])
            assert subprocess.check_output([*git, "status", "--porcelain", "--untracked-files=all"],
                                           text=True, env=env).splitlines() == [" M " + filename.as_posix()]
            assert document_hash("after.stellar.yaml") == saved
            read("leave")
            subprocess.run([*git, "restore", "--", filename], env=env, check=True)
            assert read("return") == dict(initial, samples=before)
            read("leave")
            assert read("return") == dict(initial, samples=before)
            # Keep the project window key. Sampling only paints and drains its normal run loop.
            assert read("key") == dict(initial, samples=before)
            def wait_samples(expected):
                start = time.monotonic()
                while time.monotonic() - start < 4:
                    sample = read("sample")
                    observed = sample.pop("samples")
                    assert sample == initial, sample
                    if expected(observed):
                        return round((time.monotonic() - start) * 1000)
                raise AssertionError("external component change stayed stale in key window")
            # The existing focus-return route alone leaves both instances stale while the project stays key.
            subprocess.run([cli, "component-source", "resize", definition, card["source_hash"], "201", "77"],
                           env=env, check=True, stdout=subprocess.DEVNULL)
            for _ in range(6):
                assert read("sample") == dict(initial, samples=before)
            read("watch")
            wait_samples(lambda s: s == after)
            subprocess.run([*git, "restore", "--", filename], env=env, check=True)
            wait_samples(lambda s: s == before)
            subprocess.run([cli, "component-source", "resize", definition, card["source_hash"], "201", "77"],
                           env=env, check=True, stdout=subprocess.DEVNULL)
            resize_ms = wait_samples(lambda s: s == after)
            subprocess.run([*git, "restore", "--", filename], env=env, check=True)
            restore_ms = wait_samples(lambda s: s == before)
            applied = json.loads(subprocess.check_output(
                [cli, "component-source", "set-fill", definition, card["source_hash"], "srgb", "0", "1", "0", "1"],
                env=env))["result"]
            assert applied["id"] == card["id"] and applied["background"] == {"space": "srgb", "rgba": "0 1 0 1"}
            def painted_green(samples):
                return (all(p["id"] == previous["id"] for p, previous in zip(samples, before)) and
                        all(p["rgb"][1] >= 250 and max(p["rgb"][0], p["rgb"][2]) < 5 for p in samples[:2]) and
                        samples[2:4] == before[2:4] and samples[6] == before[6])
            fill_ms = wait_samples(painted_green)
            observed = read("sample")
            colors = [p["rgb"] for p in observed.pop("samples")[:2]]
            assert observed == initial, observed
            assert document_hash("after-set-fill.stellar.yaml") == saved
            assert subprocess.check_output([*git, "status", "--porcelain", "--untracked-files=all"],
                                           text=True, env=env).splitlines() == [" M " + filename.as_posix()]
            print(f"CLI set-fill to key canvas ok: {fill_ms}ms, two sRGB pixels {colors}, dirty selection/history, document hash and one Git path verified")
            subprocess.run([*git, "restore", "--", filename], env=env, check=True)
            wait_samples(lambda s: s == before)
            temporary = definition.with_suffix(".away")
            definition.rename(temporary)
            wait_samples(lambda s: all(p["id"] == ids[6] for p in s))
            for _ in range(12):
                temporary.rename(definition)
                definition.rename(temporary)
            temporary.rename(definition)
            wait_samples(lambda s: s == before)
            read("reopen")
            initial = read("key")
            initial.pop("samples")
            assert initial["dirty"] == 0 and initial["selection"] != instance
            subprocess.run([cli, "component-source", "resize", definition, card["source_hash"], "201", "77"],
                           env=env, check=True, stdout=subprocess.DEVNULL)
            wait_samples(lambda s: [p["id"] for p in s] == [p["id"] for p in after])
            subprocess.run([*git, "restore", "--", filename], env=env, check=True)
            wait_samples(lambda s: [p["id"] for p in s] == ids)
            print(f"key window watcher ok: CLI resize {resize_ms}ms, Git restore {restore_ms}ms, rename away, burst restore, close and reopen")
            read("revoke")
            read("leave")
            assert all(p["id"] == ids[6] for p in read("return")["samples"])
            subprocess.run([*git, "diff", "--quiet", "HEAD", "--"], env=env, check=True)
            requests.write("done\n")
            requests.flush()
            assert proc.wait(timeout=15) == 0
        finally:
            # Open is a LaunchServices client. Closing just that process would leave the fixture blocked in readLine().
            if proc.poll() is None:
                requests.write("done\n")
                requests.flush()
                try:
                    proc.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    proc.kill()
    print("project window return ok: CLI resize, Git restore, two instances, unsaved rename, selection/history and single-file diff")


def native_entry(row):
    values = dict(row, width=float(row["width"]), height=float(row["height"]))
    color = values["background"]
    if color is not None:
        values["background"] = [{"srgb": 1, "display-p3": 2}[color["space"]], *map(float, color["rgba"].split())]
    return values


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
            assert item == native_entry(card), (item, card)
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
            check_window_return(app, cli, home / "window-return", clone, env, card, path.relative_to(source), first_id)
            def edit_fill(action, value):
                command("fill-open")
                changed = command("fill-" + action + " " + value)
                assert changed["selected"] == card["id"] and not changed["message"], changed
                [entry] = json.loads(subprocess.check_output([cli, "query", clone, "components"], env=clone_env))["result"]["entries"]
                assert entry["id"] == card["id"] and native_entry(entry)["background"] == changed["fill"], (entry, changed)
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
            print("native Edit Fill to CLI query ok: sRGB blue at 0.5 alpha, extended RGB, none, transparent Display P3 with preserved channels")
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
            command("layer " + second_id)
            for broken in [None, b"invalid yaml without a component"]:
                if broken is None:
                    old_path.unlink()
                else:
                    old_path.write_bytes(broken)
                assert not command("refresh")["entries"]
                assert wait_canvas("missing or invalid definition clears stale paint", no_instances)["revision"] == revision
                lookup = command("show-library")
                assert not lookup["selected"] and not lookup["details"] and lookup["message"], lookup
                if broken is None:
                    assert card["id"] in lookup["message"], lookup
                    assert card["id"] in command("refresh")["message"]
                old_path.write_bytes(blue_source)
                assert command("refresh")["selected"] == card["id"]
                assert wait_canvas("definition restored at the same revision", pixels_changed)["revision"] == revision
            restored = command("reopen")
            assert restored["entries"][0]["width"] == 201
            command("fit")
            wait_canvas("reopened instances resolve to updated source", pixels_changed)

            # A grandchild of a scaled definition Frame has a different parent space
            # from the document instance. Dragging its hit must use the latter.
            scaled = blue_source.replace(b'1 0 0 1 12 6', b'2 0 0 2 12 6', 1)
            assert scaled != blue_source
            old_path.write_bytes(scaled)
            definition_diff = [*clone_git, "diff", "HEAD", "--", old_path]
            before_move_diff = subprocess.check_output(definition_diff, env=clone_env)
            command("refresh")
            original_revision = command("canvas")["revision"]
            before = command("pointer 55 38 55 38")
            assert before["hit"] == first_id and before["scaleX"] == 2, before
            assert before["revision"] == original_revision
            repeated = command("pointer 55 38 55 38")
            assert repeated == before, (before, repeated)
            # A same-name file is not the definition referenced by this canvas instance.
            other_id = "0195c530-3acd-7bc1-8414-c213f61cb971"
            subprocess.run([cli, "component-source", "create", old_path.parent, card["name"], other_id,
                            "0195c530-3acd-7bc1-8414-c213f61cb972"], env=clone_env, check=True, stdout=subprocess.DEVNULL)
            other = next(row for row in command("refresh")["entries"] if row["id"] == other_id)
            assert command("select " + other_id)["selected"] == other_id
            def project_state():
                return (subprocess.check_output([*clone_git, "status", "--porcelain"], env=clone_env),
                        subprocess.check_output([*clone_git, "diff", "HEAD", "--binary", "--", "design"], env=clone_env),
                        (old_path.parent / other["filename"]).read_bytes())
            before_lookup = project_state()
            lookup = command("show-library")
            assert lookup["selected"] == card["id"] and lookup["undo"] == before["undo"], lookup
            assert lookup["details"].splitlines() == ["design/components/" + old_path.name, card["id"]], lookup
            assert project_state() == before_lookup
            assert command("canvas")["revision"] == original_revision
            (old_path.parent / other["filename"]).unlink()
            moved = command("pointer 55 38 95 58")
            assert moved["hit"] == first_id and moved["undo"] == before["undo"] + 1
            # The normal root sample now lies inside the scaled child. Probe the root
            # with the render/hit row outside that child's bounds for both placements.
            assert command("hit 240 108")["hit"] == first_id
            assert command("undo")["undo"] == before["undo"]
            assert command("hit 240 108")["hit"] != first_id
            assert command("redo")["undo"] == moved["undo"]
            assert command("hit 240 108")["hit"] == first_id
            def view_source(name):
                target = home / name
                subprocess.run([cli, "text-source", "export", clone, target], check=True, env=clone_env, stdout=subprocess.DEVNULL)
                return target.read_bytes()
            original_view = view_source("before-move.stellar.yaml")
            command("save")
            moved_view = view_source("after-move.stellar.yaml")
            assert original_view.count(b'position: "30 20"') == 1
            assert moved_view == original_view.replace(b'position: "30 20"', b'position: "70 40"')
            command("reopen")
            command("fit")
            assert command("hit 240 108")["hit"] == first_id
            assert old_path.read_bytes() == scaled
            assert subprocess.check_output(definition_diff, env=clone_env) == before_move_diff
            trusted_state = command("canvas")
            assert not command("revoke")["entries"]
            wait_canvas("trust revocation clears geometry and pick targets", no_instances)
            blocked = command("pointer 240 108 265 122")
            assert blocked["hit"] != first_id and blocked["revision"] == trusted_state["revision"]
            assert view_source("after-revoke.stellar.yaml") == moved_view
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
        print("instance source ok: Canvas click and Layers selection, actual Library menu, same-name UUID resolution, exact source path, missing/invalid source, document history and Git diff preserved")
        print("instance move ok: scaled grandchild drag from (30,20) to (70,40), click-only, single Undo/Redo, Save Now and reopen, sibling and definition Git diff preserved, untrusted drag blocked")
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
                    assert row == native_entry(expected_row), (row, expected_row)
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
