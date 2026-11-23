"""Control 상태 selector 브라우저 계산값 기대값 생성(XXII.2.6, 개발 도구).

    python3 crates/ops/src/export_web_motion/fixture/generate.py [--chrome <실행 파일>]

같은 폴더의 `control.css`와 생성 element 조각 `control.html`(둘 다 ops test가 `STELLAR_UPDATE_SNAPSHOT_FIXTURES=1`로
갱신)을 읽고 node 2·3·4 element의 배경 계산값을 상태(default·hover·focus-visible·active·disabled)마다 기록해
`expected.txt`에 씁니다. pseudo-class는 DevTools `CSS.forcePseudoState`, disabled는 `disabled` 속성으로 만듭니다.
줄 형식: `<node 번호> <상태> <계산값>`. 마지막 줄은 `:root`의 토큰 이름 값(비어 있어야 함)입니다.
"""

import argparse
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "..", "export_web_tokens", "fixture"))

import cdp  # noqa: E402


def element_id(n):
    return "s-" + "0" * 12 + "7000" + "8000" + "0" * 10 + f"{n:02x}"


NODES = [2, 3, 4]
STATES = ["default", "hover", "focus-visible", "active", "disabled"]


def page(css, html):
    return (
        '<!doctype html><html><head><meta charset="utf-8"><style>\n'
        + css
        + "\n</style></head><body>"
        + html
        + "</body></html>"
    )


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--chrome")
    args = parser.parse_args()
    chrome = cdp.find_chrome(args.chrome)
    with open(os.path.join(HERE, "control.css"), encoding="utf-8") as f:
        css = f.read()
    with open(os.path.join(HERE, "control.html"), encoding="utf-8") as f:
        html = f.read()
    lines = [f"# {cdp.browser_version(chrome)}, headless=new, CSS.forcePseudoState"]
    browser = cdp.Page(chrome)
    try:
        browser.load_html(page(css, html))
        for n in NODES:
            selector = "#" + element_id(n)
            for state in STATES:
                if state in ("hover", "focus-visible", "active"):
                    browser.force_pseudo(selector, [state])
                elif state == "disabled":
                    browser.evaluate(f"document.querySelector('{selector}').setAttribute('disabled', '')")
                value = browser.evaluate(
                    f"getComputedStyle(document.querySelector('{selector}')).backgroundColor"
                )
                lines.append(f"{n} {state} {value}")
                browser.force_pseudo(selector, [])
                browser.evaluate(f"document.querySelector('{selector}').removeAttribute('disabled')")
        root = browser.evaluate(
            "getComputedStyle(document.documentElement).getPropertyValue('--control-fill')"
        )
        lines.append(f"1 --control-fill {root.strip()}".rstrip())
    finally:
        browser.close()
    with open(os.path.join(HERE, "expected.txt"), "w", encoding="utf-8") as f:
        f.write("\n".join(lines) + "\n")


if __name__ == "__main__":
    main()
