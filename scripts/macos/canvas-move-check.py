#!/usr/bin/env python3
"""저장 fixture의 Frame 좌표를 CLI로 설정한 뒤 실제 앱 드래그·저장·재열기를 검증합니다."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile


def main(app, cli, fixture):
    with tempfile.TemporaryDirectory(prefix="stellar-canvas-move-") as directory:
        work = Path(directory) / "project"
        home = Path(directory) / "home"
        home.mkdir()
        shutil.copytree(fixture, work)
        env = dict(os.environ, HOME=str(home), GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull)
        subprocess.run(["git", "init", "-q", work], check=True, env=env)
        snapshot = json.loads(subprocess.check_output([cli, "query", work, "project"], env=env))["result"]
        root = "00000000-000a-7000-8000-00000000000a"
        child = "00000000-000b-7000-8000-00000000000b"
        layout = [(root, 200, 120, 2, 2, -200, -120), (child, 15, 15, 1, 1, 80, 50)]
        # Copying this frame must retry the C ABI using its reported byte count.
        commands = [dict(kind="rename", node=child, name="Clipboard " + "x" * 5000)]
        for node, w, h, sx, sy, x, y in layout:
            commands.append(dict(kind="set-frame-size", node=node, width=w, height=h))
            commands.append(dict(kind="set-frame-transform", node=node, a=sx, b=0, c=0, d=sy, tx=x, ty=y))
        request = dict(format="stellar-apply", format_version=1, base_revision=snapshot["revision"]["token"],
                       document=snapshot["documents"][0]["id"], commands=commands)
        subprocess.run([cli, "apply", work, "-"], input=json.dumps(request), text=True, check=True, env=env,
                       stdout=subprocess.DEVNULL)
        subprocess.run([app, "--canvas-move-check", work, home], check=True, env=env, timeout=90)


if __name__ == "__main__":
    main(*(Path(arg).resolve() for arg in sys.argv[1:]))
