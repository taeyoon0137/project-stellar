"""CSS 레이아웃 적합성 fixture 정의 (IX.11.8).

fixture는 node tree와 CSS 선언입니다. 같은 선언이 HTML style과 엔진 입력(`css_conformance.rs`의 parser)에
그대로 들어갑니다. 콘텐츠 leaf는 고정 intrinsic 크기 대체 요소처럼 다룹니다(`content`: 너비·높이·baseline px).

bundle:
- `basic`: 남는 공간과 분배 몫이 1/64 px 격자에서 나누어떨어지는 입력. 1 LayoutUnit 안에서 일치해야 합니다.
- `distribution`: 나누어떨어지지 않는 입력. 같은 줄 item 수 이하 LayoutUnit 차이를 허용합니다(속성 모델 1.3절).

exception: 속성 모델 14장 확장(Stellar가 CSS와 다르게 정한 항목) 이름. 지금은 없습니다.
"""


def n(node_id, style="", children=(), content=None):
    return {"id": node_id, "style": style, "content": content, "kids": list(children)}


def c(node_id, width, height, baseline=None, style=""):
    """고정 intrinsic 콘텐츠 leaf."""
    return n(node_id, style, (), {"width": width, "height": height, "baseline": baseline})


def flatten(root):
    out = []

    def visit(node, parent):
        out.append(
            {
                "id": node["id"],
                "parent": parent,
                "style": node["style"],
                "content": node["content"],
                "children": [kid["id"] for kid in node["kids"]],
            }
        )
        for kid in node["kids"]:
            visit(kid, node["id"])

    visit(root, None)
    return out


FIXTURES = []


def fx(name, suite, root, bundle="basic", note=""):
    FIXTURES.append(
        {
            "name": name,
            "suite": suite,
            "bundle": bundle,
            "exception": None,
            "note": note,
            "nodes": flatten(root),
        }
    )


def boxes(prefix, sizes, extra=""):
    return [n("%s%d" % (prefix, i + 1), "width:%spx;height:%spx;%s" % (w, h, extra)) for i, (w, h) in enumerate(sizes)]


# --- Flexbox ---

ROW3 = [(40, 20), (60, 30), (20, 10)]

for justify in [
    "normal",
    "flex-start",
    "flex-end",
    "center",
    "space-between",
    "space-around",
    "space-evenly",
    "start",
    "end",
    "stretch",
]:
    fx(
        "flex/justify-%s" % justify,
        "flex",
        n("root", "display:flex;width:240px;height:60px;justify-content:%s" % justify, boxes("i", ROW3)),
    )

for justify in ["center", "space-between", "flex-end", "space-around"]:
    fx(
        "flex/column-justify-%s" % justify,
        "flex",
        n(
            "root",
            "display:flex;flex-direction:column;width:100px;height:200px;justify-content:%s" % justify,
            boxes("i", ROW3),
        ),
    )

for direction in ["row-reverse", "column-reverse"]:
    for justify in ["flex-start", "flex-end", "start", "end", "space-between"]:
        fx(
            "flex/%s-%s" % (direction, justify),
            "flex",
            n(
                "root",
                "display:flex;flex-direction:%s;width:200px;height:160px;justify-content:%s"
                % (direction, justify),
                boxes("i", ROW3),
            ),
        )

for align in ["normal", "stretch", "flex-start", "flex-end", "center", "start", "end"]:
    fx(
        "flex/align-items-%s" % align,
        "flex",
        n(
            "root",
            "display:flex;width:200px;height:80px;align-items:%s" % align,
            [
                n("auto", "width:40px"),
                n("fixed", "width:40px;height:20px"),
                c("text", 30, 12),
                n("fit", "width:40px;height:fit-content", [n("fitkid", "width:10px;height:6px")]),
            ],
        ),
    )

fx(
    "flex/default-align-fit-content-child-not-stretched",
    "flex",
    n(
        "root",
        "display:flex;width:200px;height:100px",
        [
            n("fit", "height:fit-content;width:fit-content", [n("k", "width:30px;height:16px")]),
            n("auto", "", [n("k2", "width:30px;height:16px")]),
        ],
    ),
    note="기본 align-items(normal) 안 fit-content 자식은 늘어나지 않고 시작 쪽에 놓임",
)

fx(
    "flex/align-self-overrides",
    "flex",
    n(
        "root",
        "display:flex;width:200px;height:80px;align-items:center",
        [
            n("a", "width:20px;height:20px;align-self:flex-start"),
            n("b", "width:20px;align-self:stretch"),
            n("c", "width:20px;height:20px;align-self:flex-end"),
            n("d", "width:20px;height:20px"),
            n("e", "width:20px;align-self:auto"),
        ],
    ),
)

fx(
    "flex/align-self-stretch-respects-max",
    "flex",
    n(
        "root",
        "display:flex;width:200px;height:80px",
        [n("a", "width:20px;max-height:50px"), n("b", "width:20px;min-height:100px")],
    ),
)

fx(
    "flex/column-align-items-default-stretches-width",
    "flex",
    n(
        "root",
        "display:flex;flex-direction:column;width:120px;height:100px",
        [n("a", "height:20px"), n("b", "height:20px;width:50px"), n("c", "height:20px;width:fit-content", [n("ck", "width:24px;height:4px")])],
    ),
)

fx(
    "flex/grow-equal",
    "flex",
    n("root", "display:flex;width:240px;height:40px", [n("a", "flex-grow:1"), n("b", "flex-grow:1"), n("c", "flex-grow:2")]),
)
fx(
    "flex/grow-with-basis",
    "flex",
    n(
        "root",
        "display:flex;width:300px;height:40px",
        [n("a", "flex-basis:40px;flex-grow:1"), n("b", "flex-basis:80px;flex-grow:3"), n("c", "width:20px")],
    ),
)
fx(
    "flex/grow-freeze-max",
    "flex",
    n(
        "root",
        "display:flex;width:300px;height:40px",
        [n("a", "flex-grow:1;max-width:50px"), n("b", "flex-grow:1"), n("c", "flex-grow:1;max-width:30px")],
    ),
)
fx(
    "flex/grow-fraction-sum-below-one",
    "flex",
    n(
        "root",
        "display:flex;width:300px;height:40px",
        [n("a", "width:20px;flex-grow:0.25"), n("b", "width:20px;flex-grow:0.25")],
    ),
)
fx(
    "flex/shrink-default-one",
    "flex",
    n("root", "display:flex;width:200px;height:40px", boxes("i", [(100, 20), (100, 20), (200, 20)])),
    note="flex-shrink 생략값 1",
)
fx(
    "flex/shrink-weighted-by-basis",
    "flex",
    n(
        "root",
        "display:flex;width:240px;height:40px",
        [n("a", "width:160px;flex-shrink:1"), n("b", "width:160px;flex-shrink:3")],
    ),
)
fx(
    "flex/shrink-zero",
    "flex",
    n("root", "display:flex;width:200px;height:40px", [n("a", "width:150px;flex-shrink:0"), n("b", "width:150px")]),
)
fx(
    "flex/shrink-min-width-freeze",
    "flex",
    n(
        "root",
        "display:flex;width:200px;height:40px",
        [n("a", "width:160px;min-width:140px"), n("b", "width:160px")],
    ),
)
fx(
    "flex/shrink-fraction-sum-below-one",
    "flex",
    n(
        "root",
        "display:flex;width:200px;height:40px",
        [n("a", "width:160px;flex-shrink:0.25"), n("b", "width:160px;flex-shrink:0.25")],
    ),
)
for basis in ["0px", "48px", "25%", "auto", "content"]:
    fx(
        "flex/basis-%s" % basis.replace("%", "pct"),
        "flex",
        n(
            "root",
            "display:flex;width:320px;height:40px",
            [
                n("a", "flex-basis:%s;width:64px;flex-grow:1" % basis, [n("ak", "width:16px;height:8px")]),
                n("b", "width:32px"),
            ],
        ),
    )

for align_content in ["normal", "stretch", "flex-start", "flex-end", "center", "space-between", "space-around", "space-evenly"]:
    fx(
        "flex/wrap-align-content-%s" % align_content,
        "flex",
        n(
            "root",
            "display:flex;flex-wrap:wrap;width:200px;height:200px;align-content:%s" % align_content,
            boxes("i", [(80, 20), (80, 30), (80, 20), (60, 10), (120, 40)]),
        ),
    )
fx(
    "flex/wrap-reverse",
    "flex",
    n(
        "root",
        "display:flex;flex-wrap:wrap-reverse;width:200px;height:160px;align-items:flex-start",
        boxes("i", [(80, 20), (80, 30), (80, 20), (60, 10)]),
    ),
)
fx(
    "flex/wrap-gaps",
    "flex",
    n(
        "root",
        "display:flex;flex-wrap:wrap;width:200px;column-gap:10px;row-gap:6px",
        boxes("i", [(60, 20), (60, 30), (60, 20), (60, 10), (60, 10)]),
    ),
)
fx(
    "flex/wrap-stretch-lines-auto-height",
    "flex",
    n(
        "root",
        "display:flex;flex-wrap:wrap;width:120px",
        [n("a", "width:60px;height:20px"), n("b", "width:60px"), n("c", "width:80px;height:30px"), n("d", "width:20px")],
    ),
)
fx(
    "flex/column-wrap",
    "flex",
    n(
        "root",
        "display:flex;flex-direction:column;flex-wrap:wrap;width:200px;height:100px;column-gap:4px",
        boxes("i", [(30, 40), (50, 40), (40, 40), (20, 20)]),
    ),
)
fx(
    "flex/row-gap-column-direction",
    "flex",
    n("root", "display:flex;flex-direction:column;width:100px;row-gap:8px;column-gap:50px", boxes("i", ROW3)),
)
fx(
    "flex/column-gap-row-direction",
    "flex",
    n("root", "display:flex;width:300px;column-gap:12px;row-gap:50px", boxes("i", ROW3)),
)
fx(
    "flex/root-auto-width-max-content",
    "flex",
    n("root", "display:flex;column-gap:4px", [c("a", 30, 10), c("b", 50, 20), n("c", "width:12px;height:12px")]),
)
fx(
    "flex/fit-content-container-in-block",
    "flex",
    n(
        "root",
        "width:400px",
        [n("row", "display:flex;width:fit-content;padding:4px 6px", [c("a", 30, 10), c("b", 50, 20)])],
    ),
)
fx(
    "flex/column-auto-height",
    "flex",
    n("root", "width:200px", [n("col", "display:flex;flex-direction:column;row-gap:5px;padding:3px", boxes("i", ROW3))]),
)
fx(
    "flex/item-percent-width",
    "flex",
    n("root", "display:flex;width:400px;height:50px", [n("a", "width:25%;height:50%"), n("b", "width:50%;flex-shrink:0")]),
)
for free in ["space-between", "space-around", "space-evenly", "center"]:
    fx(
        "flex/negative-free-space-%s" % free,
        "flex",
        n(
            "root",
            "display:flex;width:100px;height:40px;justify-content:%s" % free,
            boxes("i", [(80, 20), (80, 20)], "flex-shrink:0"),
        ),
        note="남는 공간이 음수일 때 분배 정렬 fallback(속성 모델 4.2절 기록 대상)",
    )
fx(
    "flex/max-height-column-grow",
    "flex",
    n(
        "root",
        "display:flex;flex-direction:column;width:60px;height:200px",
        [n("a", "flex-grow:1;max-height:40px"), n("b", "flex-grow:1"), n("c", "min-height:30px;height:10px")],
    ),
)
fx(
    "flex/aspect-ratio-item-definite-width",
    "flex",
    n("root", "display:flex;width:300px;align-items:flex-start", [n("a", "width:80px;aspect-ratio:2/1"), n("b", "height:30px;aspect-ratio:1/1")]),
)
fx(
    "flex/padding-container-and-items",
    "flex",
    n(
        "root",
        "display:flex;width:240px;height:80px;padding:10px 20px 6px 4px;column-gap:8px",
        [n("a", "flex-grow:1;padding:4px"), n("b", "width:40px;padding:0 10px")],
    ),
)
fx(
    "flex/nested-row-in-column",
    "flex",
    n(
        "root",
        "display:flex;flex-direction:column;width:200px;height:200px;row-gap:10px",
        [
            n("row", "display:flex;column-gap:6px;justify-content:space-between", boxes("r", [(30, 20), (40, 10)])),
            n("grow", "flex-grow:1;display:flex", [n("g1", "flex-grow:1"), n("g2", "width:50px")]),
        ],
    ),
)
fx(
    "flex/nested-auto-width-items",
    "flex",
    n(
        "root",
        "display:flex;width:300px;height:60px;align-items:flex-start",
        [n("box", "display:flex;padding:2px;column-gap:3px", [c("x", 20, 10), c("y", 30, 12)]), c("z", 40, 14)],
    ),
)
fx(
    "flex/display-none-child",
    "flex",
    n("root", "display:flex;width:200px;height:40px;column-gap:10px", [n("a", "width:20px"), n("hidden", "width:50px;display:none"), n("b", "width:20px")]),
)
fx(
    "flex/zero-size-item-keeps-gap",
    "flex",
    n("root", "display:flex;width:200px;height:40px;column-gap:10px", [n("a", "width:20px"), n("zero", "width:0px"), n("b", "width:20px")]),
)
fx(
    "flex/min-width-explicit-zero",
    "flex",
    n("root", "display:flex;width:100px;height:20px", [c("a", 80, 10, None, "min-width:0px"), c("b", 80, 10, None, "min-width:0px")]),
)
fx(
    "flex/grow-distribution-thirds",
    "flex",
    n("root", "display:flex;width:100px;height:20px", [n("a", "flex-grow:1"), n("b", "flex-grow:1"), n("c", "flex-grow:1")]),
    bundle="distribution",
)
fx(
    "flex/center-odd-free-space",
    "flex",
    n("root", "display:flex;width:101px;height:21px;justify-content:center;align-items:center", [n("a", "width:20px;height:10px")]),
    bundle="distribution",
)
fx(
    "flex/space-between-uneven",
    "flex",
    n("root", "display:flex;width:100px;height:20px;justify-content:space-between", boxes("i", [(10, 10), (10, 10), (10, 10), (10, 10)])),
    bundle="distribution",
)

# --- positioning ---

PARENT = "width:200px;height:120px;padding:10px 20px 30px 40px"

for name, child in [
    ("left-top", "position:absolute;left:5px;top:7px;width:30px;height:20px"),
    ("right-bottom", "position:absolute;right:5px;bottom:7px;width:30px;height:20px"),
    ("left-right-stretch", "position:absolute;left:5px;right:15px;top:0px;height:20px"),
    ("top-bottom-stretch", "position:absolute;top:5px;bottom:15px;left:0px;width:20px"),
    ("all-insets-zero", "position:absolute;left:0px;right:0px;top:0px;bottom:0px"),
    ("percent-insets", "position:absolute;left:10%;top:25%;width:20px;height:20px"),
    ("percent-size-padding-box", "position:absolute;left:0px;top:0px;width:50%;height:50%"),
    ("over-constrained-ignores-right", "position:absolute;left:10px;right:10px;width:50px;top:0px;bottom:0px;height:30px"),
    ("no-insets-static-position", "position:absolute;width:30px;height:20px"),
    ("negative-insets", "position:absolute;left:-10px;top:-5px;width:30px;height:20px"),
    ("right-only-percent", "position:absolute;right:25%;top:0px;width:30px;height:20px"),
    ("min-max-on-stretch", "position:absolute;left:0px;right:0px;top:0px;bottom:0px;max-width:100px;min-height:150px"),
]:
    fx("position/abs-%s" % name, "positioning", n("root", PARENT, [n("a", child)]))

fx(
    "position/abs-auto-width-shrink-to-fit",
    "positioning",
    n("root", PARENT, [n("a", "position:absolute;left:10px;top:0px", [c("k", 50, 10)])]),
)
fx(
    "position/abs-auto-width-left-right",
    "positioning",
    n("root", PARENT, [n("a", "position:absolute;left:10px;right:20px;top:0px", [c("k", 50, 10)])]),
)
fx(
    "position/abs-right-auto-width-shrink",
    "positioning",
    n("root", PARENT, [n("a", "position:absolute;right:10px;top:4px;padding:2px", [c("k", 40, 10), c("k2", 60, 10)])]),
)
fx(
    "position/abs-static-after-flow-siblings",
    "positioning",
    n(
        "root",
        PARENT,
        [n("flow1", "height:20px"), n("a", "position:absolute;width:30px;height:10px"), n("flow2", "height:15px"), n("b", "position:absolute;left:3px;width:30px;height:10px")],
    ),
)
fx(
    "position/abs-does-not-size-parent",
    "positioning",
    n(
        "root",
        "width:300px;height:200px",
        [n("parent", "position:absolute;left:0px;top:0px;padding:5px", [n("flow", "width:40px;height:20px"), n("a", "position:absolute;left:100px;top:100px;width:80px;height:80px")])],
    ),
)
fx(
    "position/abs-aspect-ratio",
    "positioning",
    n("root", PARENT, [n("a", "position:absolute;left:0px;top:0px;width:60px;aspect-ratio:3/1"), n("b", "position:absolute;right:0px;bottom:0px;height:40px;aspect-ratio:1/2")]),
)
fx(
    "position/abs-nested",
    "positioning",
    n(
        "root",
        PARENT,
        [n("a", "position:absolute;left:10px;top:10px;width:100px;height:60px;padding:4px", [n("b", "position:absolute;right:0px;bottom:0px;width:20px;height:10px")])],
    ),
)
for justify, align in [("center", "center"), ("flex-end", "flex-end"), ("space-between", "stretch"), ("flex-start", "end")]:
    fx(
        "position/abs-in-flex-static-%s-%s" % (justify, align),
        "positioning",
        n(
            "root",
            "display:flex;width:200px;height:100px;padding:10px;justify-content:%s;align-items:%s" % (justify, align),
            [n("flow", "width:40px;height:20px"), n("a", "position:absolute;width:30px;height:20px")],
        ),
    )
fx(
    "position/abs-in-flex-reverse-static",
    "positioning",
    n(
        "root",
        "display:flex;flex-direction:row-reverse;width:200px;height:100px;padding:10px",
        [n("a", "position:absolute;width:30px;height:20px")],
    ),
)
fx(
    "position/abs-in-flex-with-insets",
    "positioning",
    n(
        "root",
        "display:flex;width:200px;height:100px;padding:10px;justify-content:center",
        [n("a", "position:absolute;left:0px;width:30px;height:20px")],
    ),
)
for name, child in [
    ("left-top", "position:relative;left:5px;top:7px;width:30px;height:20px"),
    ("right-bottom", "position:relative;right:5px;bottom:7px;width:30px;height:20px"),
    ("left-wins-over-right", "position:relative;left:5px;right:50px;width:30px;height:20px"),
    ("percent", "position:relative;left:10%;top:10%;width:30px;height:20px"),
]:
    fx(
        "position/relative-%s" % name,
        "positioning",
        n("root", "width:200px;height:100px;padding:4px", [n("a", child), n("after", "width:20px;height:20px")]),
    )
fx(
    "position/relative-in-flex",
    "positioning",
    n("root", "display:flex;width:200px;height:50px;column-gap:5px", [n("a", "width:30px;height:20px;position:relative;left:10px;top:5px"), n("b", "width:30px;height:20px")]),
)

# --- block ---

fx(
    "block/stacking-and-stretch",
    "block",
    n("root", "width:200px;padding:5px", [n("a", "height:20px"), n("b", "width:50px;height:10px"), c("t", 30, 12), n("c", "height:5px;padding:0 10px")]),
)
fx(
    "block/auto-width-fit-content",
    "block",
    n("root", "width:300px", [n("fit", "width:fit-content;padding:3px", [c("x", 40, 10), c("y", 70, 10)]), n("min", "width:min-content", [c("m1", 40, 10), c("m2", 70, 10)]), n("max", "width:max-content", [c("z", 30, 10)])]),
)
fx(
    "block/min-max-width",
    "block",
    n("root", "width:300px", [n("a", "max-width:100px;height:10px"), n("b", "min-width:400px;height:10px"), n("c", "width:50%;min-width:200px;height:10px")]),
)
fx(
    "block/percent-height-definite",
    "block",
    n("root", "width:100px;height:200px", [n("a", "height:25%"), n("b", "height:50%;width:50%")]),
)
fx(
    "block/percent-height-indefinite-is-auto",
    "block",
    n("root", "width:100px", [n("wrap", "", [n("a", "height:50%", [n("k", "height:30px")])])]),
)
fx(
    "block/aspect-ratio-auto-height",
    "block",
    n("root", "width:200px", [n("a", "aspect-ratio:4/1"), n("b", "width:50px;aspect-ratio:1/2"), n("c", "height:20px;aspect-ratio:3/1")]),
)
fx(
    "block/aspect-ratio-content-overflow-min-auto",
    "block",
    n("root", "width:200px", [n("a", "width:100px;aspect-ratio:1/1", [n("tall", "height:300px")]), n("b", "width:100px;aspect-ratio:1/1;min-height:0px", [n("tall2", "height:300px")])]),
    note="한 축만 definite인 aspect-ratio Frame 안 넘치는 콘텐츠(14장 E10): min-height:auto는 콘텐츠, 0px 명시는 비율",
)
fx(
    "block/display-none",
    "block",
    n("root", "width:100px", [n("a", "height:10px"), n("hidden", "display:none;height:50px", [n("hk", "height:5px")]), n("b", "height:10px")]),
)


def all_fixtures():
    return FIXTURES
