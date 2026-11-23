#!/usr/bin/env python3
"""`css` 기본값 세트 conformance 측정기 (IX.11.31, ADR-0115 결정 2).

저자 스타일이 없는 HTML 한 장을 고정 버전 Chromium 계열 브라우저(headless)로 열어, 노드 종류 구획마다 대응 요소의
CSS 계산값(`computedStyleMap`, CSS Typed OM)을 읽고 JSON 한 줄로 출력합니다. 측정 결과는
`style_defaults_conformance.txt`의 `css` 줄 `observed` 칸에 옮겨 적습니다. 테스트는 브라우저를 실행하지 않고 그
파일만 읽습니다(`style_defaults.rs` `conformance` test).

- 구획 대응: `frame` = `div`, `text` = `span`, `image` = `img`, `shape` = `svg` 안 `rect`(Path는 `path`).
- `computedStyleMap`은 사용값(used value)이 아니라 계산값을 돌려줍니다. 그래서 `width: auto`처럼 배치 결과와
  무관한 값을 읽습니다.

사용법:

    python3 crates/document/src/css/style_defaults_conformance.py [--chrome <실행 파일>]

브라우저 경로는 `--chrome`, 환경 변수 `STELLAR_CHROME`, macOS 기본 설치 경로 순서로 찾습니다. 표준 라이브러리만
씁니다. 빌드·테스트 의존성이 아니라 기대값을 다시 확인할 때만 쓰는 개발 도구입니다.
"""

import argparse
import json
import os
import re
import subprocess
import sys
import tempfile

DEFAULT_CHROME = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"

LAYOUT = [
    "position", "box-sizing", "top", "right", "bottom", "left", "width", "height", "min-width",
    "min-height", "max-width", "max-height", "aspect-ratio", "flex-grow", "flex-shrink",
    "flex-basis", "align-self", "justify-self", "grid-column-start", "grid-column-end",
    "grid-row-start", "grid-row-end", "display", "padding-top", "padding-right",
    "padding-bottom", "padding-left", "flex-direction", "flex-wrap", "justify-content",
    "align-content", "align-items", "justify-items", "row-gap", "column-gap",
    "grid-template-columns", "grid-template-rows", "grid-auto-columns", "grid-auto-rows",
    "grid-auto-flow",
]

PAYLOAD = {
    "frame": [
        "transform", "background-color", "border-top-left-radius", "border-top-right-radius",
        "border-bottom-right-radius", "border-bottom-left-radius", "border-top-width",
        "border-top-style", "border-top-color", "border-right-width", "border-right-style",
        "border-right-color", "border-bottom-width", "border-bottom-style",
        "border-bottom-color", "border-left-width", "border-left-style", "border-left-color",
    ],
    "shape": [
        "border-top-left-radius", "border-top-right-radius", "border-bottom-right-radius",
        "border-bottom-left-radius", "stroke-linecap",
    ],
    "path": ["fill-rule"],
    "text": [
        "font-family", "font-size", "font-weight", "font-style", "font-stretch", "line-height",
        "letter-spacing",
    ],
    "image": ["object-fit", "image-rendering"],
}

ELEMENTS = {"frame": "d", "text": "s", "image": "i", "shape": "r", "path": "p"}

PAGE = """<!doctype html><html><head><meta charset="utf-8"></head><body>
<div id="d"></div><span id="s">x</span><img id="i" alt="">
<svg id="g" width="10" height="10"><rect id="r"/><path id="p" d="M0 0"/></svg>
<pre id="out"></pre>
<script>
const layout = %s;
const payload = %s;
const ids = %s;
const out = {};
for (const [klass, id] of Object.entries(ids)) {
  const map = document.getElementById(id).computedStyleMap();
  const names = klass === "path" ? [] : layout.slice();
  for (const name of payload[klass] || []) names.push(name);
  out[klass] = {};
  for (const name of names) out[klass][name] = String(map.get(name));
}
document.getElementById("out").textContent = JSON.stringify(out);
</script></body></html>
"""


def find_chrome(arg):
    for candidate in (arg, os.environ.get("STELLAR_CHROME"), DEFAULT_CHROME):
        if candidate and os.path.exists(candidate):
            return candidate
    sys.exit("브라우저를 찾지 못했습니다. --chrome 또는 STELLAR_CHROME을 주세요")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--chrome")
    args = parser.parse_args()
    chrome = find_chrome(args.chrome)
    with tempfile.TemporaryDirectory() as tmp:
        page = os.path.join(tmp, "page.html")
        with open(page, "w", encoding="utf-8") as f:
            f.write(PAGE % (json.dumps(LAYOUT), json.dumps(PAYLOAD), json.dumps(ELEMENTS)))
        dom = subprocess.run(
            [
                chrome,
                "--headless=new",
                "--disable-gpu",
                "--no-first-run",
                "--user-data-dir=" + os.path.join(tmp, "profile"),
                "--dump-dom",
                "file://" + page,
            ],
            check=True,
            capture_output=True,
            text=True,
        ).stdout
    match = re.search(r'<pre id="out">(.*?)</pre>', dom, re.S)
    if not match:
        sys.exit("측정 결과가 없습니다")
    version = subprocess.run([chrome, "--version"], check=True, capture_output=True, text=True)
    result = json.loads(match.group(1).replace("&quot;", '"').replace("&amp;", "&"))
    print(json.dumps({"browser": version.stdout.strip(), "computed": result}, ensure_ascii=False))


if __name__ == "__main__":
    main()
