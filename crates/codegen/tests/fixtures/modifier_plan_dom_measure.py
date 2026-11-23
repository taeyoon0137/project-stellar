"""IX.11.18 DOM 배치 측정기 (ADR-0121 결정 8).

`modifier_plan_dom.html`(codegen test가 쓰는 계획 출력기 페이지)을 로컬 Google Chrome headless(DPR 1)로 열어
요소 상자·radius·offsetParent·stacking 값과 hit test 결과를 `modifier_plan_dom.expected.txt`에 씁니다. test는
브라우저를 실행하지 않고 이 결과 파일만 읽습니다. 페이지가 바뀌면(hash 불일치) 이 스크립트를 다시 실행합니다.

사용: python3 -I modifier_plan_dom_measure.py [Chrome 실행 파일 경로]
"""

import html
import pathlib
import re
import subprocess
import sys
import tempfile

HERE = pathlib.Path(__file__).resolve().parent
PAGE = HERE / "modifier_plan_dom.html"
OUT = HERE / "modifier_plan_dom.expected.txt"
CHROME = sys.argv[1] if len(sys.argv) > 1 else "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
HITS = [(24, 24), (2, 40), (170, 30)]

SCRIPT = """
<script>
const lines = [];
for (const el of document.querySelectorAll('[data-probe]')) {
  const r = el.getBoundingClientRect();
  const s = getComputedStyle(el);
  const parent = el.offsetParent && el.offsetParent.closest('[data-probe]');
  const radius = [s.borderTopLeftRadius, s.borderTopRightRadius, s.borderBottomRightRadius,
    s.borderBottomLeftRadius].join('/');
  lines.push(el.dataset.probe + ' rect=' + [r.x, r.y, r.width, r.height].join(',') +
    ' radius=' + radius.replace(/ /g, '') + ' position=' + s.position + ' zIndex=' + s.zIndex +
    ' offsetParent=' + (parent ? parent.dataset.probe : '-'));
}
for (const [x, y] of HITS) {
  const hit = document.elementFromPoint(x, y);
  const probe = hit && hit.closest('[data-probe]');
  lines.push('hit ' + x + ' ' + y + ' ' + (probe ? probe.dataset.probe : '-'));
}
const pre = document.createElement('pre');
pre.id = 'out';
pre.textContent = lines.join('\\n');
document.body.appendChild(pre);
</script>
"""


def fnv64(data: bytes) -> int:
    h = 0xCBF29CE484222325
    for b in data:
        h = ((h ^ b) * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return h


def main() -> None:
    page = PAGE.read_bytes()
    doc = (
        "<!doctype html><html><head><meta charset=utf-8>"
        "<style>html,body{margin:0;padding:0}</style></head><body>"
        + page.decode("utf-8")
        + SCRIPT.replace("HITS", repr([list(h) for h in HITS]))
        + "</body></html>"
    )
    with tempfile.TemporaryDirectory() as tmp:
        target = pathlib.Path(tmp) / "page.html"
        target.write_text(doc, encoding="utf-8")
        dom = subprocess.run(
            [
                CHROME,
                "--headless=new",
                "--disable-gpu",
                "--force-device-scale-factor=1",
                "--window-size=800,600",
                "--dump-dom",
                target.as_uri(),
            ],
            check=True,
            capture_output=True,
            text=True,
        ).stdout
    match = re.search(r'<pre id="out">(.*?)</pre>', dom, re.S)
    if not match:
        raise SystemExit("측정 결과를 찾지 못했습니다")
    version = subprocess.run([CHROME, "--version"], capture_output=True, text=True).stdout.strip()
    body = html.unescape(match.group(1)).strip()
    OUT.write_text(
        f"page-fnv64 {fnv64(page):016x}\n# {version}, headless, DPR 1\n{body}\n", encoding="utf-8"
    )
    print(OUT.read_text(encoding="utf-8"))


if __name__ == "__main__":
    main()
