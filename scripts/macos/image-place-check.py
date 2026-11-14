#!/usr/bin/env python3
"""Verify a native PNG placement and reopen the committed project from a local Git clone."""
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile


def main(app, cli):
    with tempfile.TemporaryDirectory(prefix="stellar-image-test-") as tmp:
        base = Path(tmp)
        home = base / "home"
        home.mkdir()
        env = dict(os.environ, HOME=str(home), GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull,
                   GIT_TERMINAL_PROMPT="0", GIT_ALLOW_PROTOCOL="file")

        def run(*args):
            return subprocess.run(args, env=env, check=True, text=True, capture_output=True, timeout=90).stdout

        result = run(app, "--image-place-check", home)
        match = re.search(r"node=([0-9a-f-]{36})", result)
        assert match, result
        node = match.group(1)
        work = home / "Image Placement"
        assert not (home / "Swatch.png").exists()
        content = list((work / "assets").glob("*/*.png"))
        assert len(content) == 1
        digest = hashlib.sha256(content[0].read_bytes()).hexdigest()
        assert content[0].stem == "sha256-" + digest and content[0].parent.name == digest[:2]

        def query(folder):
            def read(*args):
                return json.loads(run(cli, "query", folder, *args))["result"]
            document = read("project")["documents"][0]["id"]
            detail = read("node", document, node)["node"]
            props = {prop["path"]: prop["value"] for prop in detail["properties"]}
            assert detail["kind"] == "image" and detail["modifier_count"] == 1 and detail["name"] == "Swatch"
            assert props["0xe601"] == [24, 16] and re.fullmatch(r"[0-9a-f-]{36}", props["0x0601"])
            return detail

        expected = query(work)
        run("git", "-C", work, "add", "--", "assets", "design", "stellar.yaml")
        run("git", "-C", work, "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
            "-c", "core.hooksPath=/dev/null", "commit", "-qm", "placed image")
        clone = base / "clone"
        run("git", "clone", "--no-local", work, clone)
        assert query(clone) == expected
        assert (clone / content[0].relative_to(work)).read_bytes() == content[0].read_bytes()
        assert not run("git", "-C", clone, "status", "--porcelain")
        print(result.strip())
        print("png Git/CLI ok: fresh local clone retained reference, size, transform modifier, parent and exact original PNG")


if __name__ == "__main__":
    main(*(Path(arg).resolve() for arg in sys.argv[1:]))
