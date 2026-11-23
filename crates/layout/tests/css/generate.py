#!/usr/bin/env python3
"""CSS 레이아웃 적합성 fixture 기대값 생성기 (IX.11.8, ADR-0076).

fixture 정의(`definitions.py`)를 HTML 한 장으로 만들고, 고정 버전 Chromium 계열 브라우저(headless)로 열어 각
요소의 border box rect를 1/64 px LayoutUnit 정수로 읽은 뒤 `expected.json`에 씁니다. 테스트는 브라우저를 실행하지
않고 이 JSON만 읽습니다(`crates/layout/tests/css_conformance.rs`).

사용법:

    python3 crates/layout/tests/css/generate.py [--chrome <실행 파일>]

브라우저 경로는 `--chrome`, 환경 변수 `STELLAR_CHROME`, macOS 기본 설치 경로 순서로 찾습니다. 표준 라이브러리만
씁니다. 빌드·테스트 의존성이 아니라 기대값을 다시 만들 때만 쓰는 개발 도구입니다(README).
"""

import argparse
import html
import json
import os
import re
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

import definitions  # noqa: E402

DEFAULT_CHROME = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"

# root containing block 너비(px). 엔진 root 가로 가용 공간(engine::ROOT_AVAILABLE_PX)과 같습니다.
ROOT_AVAILABLE_PX = 1000000

BASE_CSS = (
    "*{box-sizing:border-box;margin:0;border:0 solid;padding:0;font-size:0;line-height:0;}"
    "html,body{margin:0;padding:0;}"
    ".cb{position:relative;width:%dpx;height:0;}" % ROOT_AVAILABLE_PX
)

# `--ua-box-sizing`: 요소의 `box-sizing`을 정하지 않아 브라우저 UA 기본값(`content-box`)을 쓰는 page입니다. `css`
# 기본값 세트(ADR-0082, ADR-0086)가 브라우저 기본값과 같은지 보는 기대값(`expected_ua_box_sizing.json`)을 만듭니다.
UA_BOX_SIZING_CSS = (
    "*{margin:0;border:0 solid;padding:0;font-size:0;line-height:0;}"
    "html,body{margin:0;padding:0;}"
    ".cb{position:relative;width:%dpx;height:0;}" % ROOT_AVAILABLE_PX
)

INSET_RE = re.compile(r"^(top|right|bottom|left)$")


def declarations(style):
    """`name: value; ...` 문자열을 (name, value) 목록으로 나눕니다."""
    out = []
    for part in style.split(";"):
        part = part.strip()
        if not part:
            continue
        name, _, value = part.partition(":")
        out.append((name.strip(), value.strip()))
    return out


def element_style(node, is_root):
    """엔진 의미와 같은 HTML style을 만듭니다.

    엔진은 모든 box를 absolute 자식의 containing block으로 씁니다(속성 모델 3장 stacking 규칙). 그래서
    `position: static` 요소는 inset 없이 `position: relative`로 냅니다. root는 containing block 원점에 둡니다.
    """
    decls = declarations(node["style"])
    position = "static"
    for name, value in decls:
        if name == "position":
            position = value
    out = []
    for name, value in decls:
        if name == "position":
            continue
        if INSET_RE.match(name) and (position == "static" or is_root):
            continue
        out.append("%s:%s" % (name, value))
    if is_root:
        out.append("position:absolute;left:0;top:0")
    elif position == "static":
        out.append("position:relative")
    else:
        out.append("position:%s" % position)
    return ";".join(out)


def content_html(content):
    """고정 intrinsic 콘텐츠(대체 요소처럼 크기가 고정된 콘텐츠)를 HTML로 씁니다.

    min-content·max-content 너비가 모두 `width`, 콘텐츠 높이가 `height`입니다. baseline이 있으면 줄 상자 하나를
    만들어 첫 baseline을 콘텐츠 top에서 `baseline`에 둡니다(font-size 0, line-height 0이라 strut 높이 0).
    """
    w = content["width"]
    h = content["height"]
    b = content.get("baseline")
    if b is None:
        return '<div style="width:%spx;height:%spx"></div>' % (w, h)
    return (
        '<span style="display:inline-block;width:%spx;height:%spx;vertical-align:baseline">'
        '<span style="display:inline-block;width:0;height:%spx"></span></span>' % (w, h, b)
    )


def node_html(nodes, node_id, is_root):
    node = nodes[node_id]
    inner = ""
    if node.get("content") is not None:
        inner = content_html(node["content"])
    for child in node["children"]:
        inner += node_html(nodes, child, False)
    return '<div data-node="%s" style="%s">%s</div>' % (
        html.escape(node_id),
        html.escape(element_style(node, is_root)),
        inner,
    )


SCRIPT = """
<script>
const out = {};
for (const fixture of document.querySelectorAll('[data-fixture]')) {
  const rects = {};
  const cb = fixture.getBoundingClientRect();
  for (const el of fixture.querySelectorAll('[data-node]')) {
    const r = el.getBoundingClientRect();
    const parent = el.parentElement.closest('[data-node]');
    const base = parent && fixture.contains(parent) ? parent.getBoundingClientRect() : cb;
    rects[el.dataset.node] = [
      Math.round((r.left - base.left) * 64),
      Math.round((r.top - base.top) * 64),
      Math.round(r.width * 64),
      Math.round(r.height * 64),
    ];
  }
  out[fixture.dataset.fixture] = rects;
}
document.body.innerHTML = '<pre id="out"></pre>';
document.getElementById('out').textContent = JSON.stringify({ua: navigator.userAgent, rects: out});
</script>
"""


def build_page(fixtures, base_css=BASE_CSS):
    parts = ["<!doctype html><html><head><meta charset=utf-8><style>%s</style></head><body>" % base_css]
    for fixture in fixtures:
        nodes = {n["id"]: n for n in fixture["nodes"]}
        parts.append('<div class="cb" data-fixture="%s">' % html.escape(fixture["name"]))
        parts.append(node_html(nodes, fixture["nodes"][0]["id"], True))
        parts.append("</div>")
    parts.append(SCRIPT)
    parts.append("</body></html>")
    return "".join(parts)


def run_chrome(chrome, page):
    """headless 브라우저로 측정합니다. 브라우저 process가 드물게 비정상 종료하면 세 번까지 다시 시도합니다."""
    last = None
    for _ in range(3):
        try:
            return run_chrome_once(chrome, page)
        except subprocess.CalledProcessError as error:
            last = error
    raise SystemExit("브라우저 실행 실패: %s\n%s" % (last, last.stderr if last else ""))


def run_chrome_once(chrome, page):
    with tempfile.TemporaryDirectory() as tmp:
        path = os.path.join(tmp, "fixtures.html")
        with open(path, "w", encoding="utf-8") as f:
            f.write(page)
        profile = os.path.join(tmp, "profile")
        result = subprocess.run(
            [
                chrome,
                "--headless=new",
                "--disable-gpu",
                "--no-first-run",
                "--no-default-browser-check",
                "--user-data-dir=" + profile,
                "--window-size=1280,800",
                "--dump-dom",
                "file://" + path,
            ],
            check=True,
            capture_output=True,
            text=True,
            timeout=300,
        )
    match = re.search(r'<pre id="out">(.*?)</pre>', result.stdout, re.S)
    if not match:
        raise SystemExit("브라우저 출력에서 측정 결과를 찾지 못했습니다")
    return json.loads(html.unescape(match.group(1)))


def browser_version(chrome):
    out = subprocess.run([chrome, "--version"], check=True, capture_output=True, text=True)
    return out.stdout.strip()


def compact(value):
    return json.dumps(value, ensure_ascii=False, separators=(", ", ": "))


def dump(document):
    """리뷰하기 쉬운 고정 형식: fixture 머리 한 줄, node·기대값은 한 줄에 하나."""
    lines = ["{"]
    for key in ["schema", "browser", "user_agent", "layout_unit", "root_available_px"]:
        lines.append(' %s: %s,' % (compact(key), compact(document[key])))
    lines.append(' "fixtures": [')
    fixtures = document["fixtures"]
    for n, fixture in enumerate(fixtures):
        head = {k: fixture[k] for k in ["name", "suite", "bundle", "exception", "note"]}
        lines.append("  {")
        for key, value in head.items():
            lines.append("   %s: %s," % (compact(key), compact(value)))
        lines.append('   "nodes": [')
        nodes = fixture["nodes"]
        for k, node in enumerate(nodes):
            lines.append("    %s%s" % (compact(node), "," if k + 1 < len(nodes) else ""))
        lines.append("   ],")
        lines.append('   "expected": {')
        items = list(fixture["expected"].items())
        for k, (node_id, rect) in enumerate(items):
            lines.append("    %s: %s%s" % (compact(node_id), compact(rect), "," if k + 1 < len(items) else ""))
        lines.append("   }")
        lines.append("  }%s" % ("," if n + 1 < len(fixtures) else ""))
    lines.append(" ]")
    lines.append("}")
    return "\n".join(lines) + "\n"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--chrome", default=os.environ.get("STELLAR_CHROME", DEFAULT_CHROME))
    parser.add_argument("--ua-box-sizing", action="store_true")
    args = parser.parse_args()
    fixtures = definitions.all_fixtures()
    names = [f["name"] for f in fixtures]
    if len(names) != len(set(names)):
        raise SystemExit("fixture 이름이 중복되었습니다")
    base_css = UA_BOX_SIZING_CSS if args.ua_box_sizing else BASE_CSS
    measured = run_chrome(args.chrome, build_page(fixtures, base_css))
    version = browser_version(args.chrome)
    out_fixtures = []
    for fixture in fixtures:
        rects = measured["rects"][fixture["name"]]
        expected = {}
        for node in fixture["nodes"]:
            expected[node["id"]] = rects[node["id"]]
        entry = dict(fixture)
        entry["expected"] = expected
        out_fixtures.append(entry)
    document = {
        "schema": 1,
        "browser": version,
        "user_agent": measured["ua"],
        "layout_unit": 64,
        "root_available_px": ROOT_AVAILABLE_PX,
        "fixtures": out_fixtures,
    }
    target = os.path.join(
        HERE, "expected_ua_box_sizing.json" if args.ua_box_sizing else "expected.json"
    )
    with open(target, "w", encoding="utf-8") as f:
        f.write(dump(document))
    counts = {}
    for fixture in fixtures:
        counts[fixture["suite"]] = counts.get(fixture["suite"], 0) + 1
    print("기대값 %d개 fixture를 %s로 썼습니다(%s): %s" % (len(fixtures), target, version, counts))


if __name__ == "__main__":
    main()
