"""토큰 CSS 브라우저 계산값 기대값 생성(XXII.2.3, 개발 도구).

    python3 crates/ops/src/export_web_tokens/fixture/generate.py [--chrome <실행 파일>]

같은 폴더의 `tokens.css`(ops test가 `STELLAR_UPDATE_SNAPSHOT_FIXTURES=1`로 갱신)를 읽고, 등록 토큰을 `var()`로 쓰는
자손 요소의 계산값을 media 조합 8개(prefers-color-scheme × prefers-contrast × prefers-reduced-motion)마다 기록해
`expected.txt`에 씁니다. 줄 형식: `<조합 bit> <속성> <계산값>`(dark=1, contrast more=2, reduced motion=4).
"""

import argparse
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

import cdp  # noqa: E402

PROBES = [
    ("width", "var(--radius-lg)"),
    ("color", "var(--accent)"),
    ("outline-color", "var(--accent-alias)"),
    ("opacity", "var(--motion-scale)"),
    ("padding-top", "var(--Space-2)"),
]


def page(css):
    style = "; ".join(f"{p}: {v}" for p, v in PROBES)
    return (
        "<!doctype html><html><head><meta charset=\"utf-8\"><style>\n"
        + css
        + "\n#probe { outline-style: solid; }\n</style></head><body>"
        + "<div id=\"outer\"><div id=\"probe\" style=\""
        + style
        + "\"></div></div></body></html>"
    )


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--chrome")
    args = parser.parse_args()
    chrome = cdp.find_chrome(args.chrome)
    with open(os.path.join(HERE, "tokens.css"), encoding="utf-8") as f:
        css = f.read()
    browser = cdp.Page(chrome)
    lines = [f"# {cdp.browser_version(chrome)}, headless=new, Emulation.setEmulatedMedia"]
    try:
        browser.load_html(page(css))
        for combo in range(8):
            browser.emulate_media(
                [
                    ("prefers-color-scheme", "dark" if combo & 1 else "light"),
                    ("prefers-contrast", "more" if combo & 2 else "no-preference"),
                    ("prefers-reduced-motion", "reduce" if combo & 4 else "no-preference"),
                ]
            )
            for prop, _ in PROBES:
                value = browser.evaluate(
                    f"getComputedStyle(document.getElementById('probe')).getPropertyValue('{prop}')"
                )
                lines.append(f"{combo} {prop} {value.strip()}")
            hover = browser.evaluate(
                "getComputedStyle(document.documentElement).getPropertyValue('--hover-fill')"
            )
            lines.append(f"{combo} --hover-fill {hover.strip()}".rstrip())
    finally:
        browser.close()
    with open(os.path.join(HERE, "expected.txt"), "w", encoding="utf-8") as f:
        f.write("\n".join(lines) + "\n")


if __name__ == "__main__":
    main()
