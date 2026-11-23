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



# --- Grid ---


def cells(prefix, count, style=""):
    return [n("%s%d" % (prefix, i + 1), style) for i in range(count)]


fx(
    "grid/fixed-tracks-auto-placement",
    "grid",
    n("root", "display:grid;width:300px;height:200px;grid-template-columns:50px 80px;grid-template-rows:30px 40px", cells("c", 4)),
)
fx(
    "grid/fr-columns-with-gaps",
    "grid",
    n("root", "display:grid;width:300px;height:100px;grid-template-columns:1fr 2fr;column-gap:12px;row-gap:4px", cells("c", 4)),
)
fx(
    "grid/bare-fr-grows-to-content-min",
    "grid",
    n("root", "display:grid;width:200px;grid-template-columns:1fr 1fr", [c("wide", 150, 10), c("narrow", 20, 10)]),
    note="bare fr = minmax(auto, fr): 콘텐츠 최소 크기 아래로 줄지 않음",
)
fx(
    "grid/minmax-zero-fr-ignores-content-min",
    "grid",
    n("root", "display:grid;width:200px;grid-template-columns:minmax(0px, 1fr) minmax(0px, 1fr)", [c("wide", 150, 10), c("narrow", 20, 10)]),
)
fx(
    "grid/fr-sum-below-one-single",
    "grid",
    n("root", "display:grid;width:200px;height:40px;grid-template-columns:0.5fr", cells("c", 1)),
)
fx(
    "grid/fr-sum-below-one-pair",
    "grid",
    n("root", "display:grid;width:200px;height:40px;grid-template-columns:0.25fr 0.25fr", cells("c", 2)),
)
fx(
    "grid/fr-sum-below-one-indefinite",
    "grid",
    n("root", "width:400px", [n("g", "display:grid;width:fit-content;grid-template-columns:0.5fr 0.25fr", [c("a", 40, 10), c("b", 30, 10)])]),
)
fx(
    "grid/fr-indefinite-factor-above-one",
    "grid",
    n("root", "width:400px", [n("g", "display:grid;width:fit-content;grid-template-columns:2fr 1fr", [c("a", 40, 10), c("b", 30, 10)])]),
)
fx(
    "grid/intrinsic-track-keywords",
    "grid",
    n(
        "root",
        "display:grid;width:400px;grid-template-columns:min-content max-content auto",
        [n("a", "", [c("a1", 30, 10), c("a2", 50, 10)]), c("b", 60, 12), c("c", 20, 8)],
    ),
)
fx(
    "grid/minmax-mixed",
    "grid",
    n("root", "display:grid;width:300px;grid-template-columns:minmax(30px, 1fr) minmax(min-content, 100px) 40px", [c("a", 10, 10), c("b", 70, 10), c("c", 5, 5)]),
)
fx(
    "grid/fit-content-track",
    "grid",
    n("root", "display:grid;width:400px;grid-template-columns:fit-content(60px) fit-content(60px) auto", [c("a", 100, 10), c("b", 40, 10), c("c", 10, 10)]),
)
for justify in ["normal", "start", "end", "center", "space-between", "space-around", "space-evenly", "stretch"]:
    fx(
        "grid/justify-content-%s" % justify,
        "grid",
        n("root", "display:grid;width:300px;height:60px;grid-template-columns:40px auto 60px;justify-content:%s" % justify, [n("a", "height:10px"), c("b", 20, 10), n("c", "height:10px")]),
    )
for align in ["normal", "end", "center", "space-between"]:
    fx(
        "grid/align-content-%s" % align,
        "grid",
        n("root", "display:grid;width:100px;height:200px;grid-template-rows:30px auto;align-content:%s" % align, [n("a", ""), c("b", 20, 15)]),
    )
for items in ["normal", "start", "end", "center", "stretch"]:
    fx(
        "grid/items-%s" % items,
        "grid",
        n(
            "root",
            "display:grid;width:200px;height:120px;grid-template-columns:100px 100px;grid-template-rows:60px 60px;justify-items:%s;align-items:%s" % (items, items),
            [n("auto", ""), n("fixed", "width:30px;height:20px"), c("text", 40, 12), n("fit", "width:fit-content;height:fit-content", [n("fk", "width:25px;height:7px")])],
        ),
    )
fx(
    "grid/self-overrides",
    "grid",
    n(
        "root",
        "display:grid;width:200px;height:100px;grid-template-columns:100px 100px;grid-template-rows:100px;justify-items:center;align-items:end",
        [n("a", "width:20px;height:20px;justify-self:start;align-self:start"), n("b", "justify-self:stretch;align-self:stretch")],
    ),
)
fx(
    "grid/aspect-ratio-item-normal",
    "grid",
    n("root", "display:grid;width:200px;height:100px;grid-template-columns:100px 100px", [n("a", "aspect-ratio:2/1"), n("b", "width:40px;aspect-ratio:1/1")]),
)
fx(
    "grid/explicit-lines-and-spans",
    "grid",
    n(
        "root",
        "display:grid;width:300px;grid-template-columns:repeat(4, 75px);grid-template-rows:20px 20px 20px",
        [
            n("a", "grid-column:2 / 4;grid-row:1 / span 2"),
            n("b", "grid-column:1 / -1;grid-row:3"),
            n("c", "grid-column-start:span 2;grid-row-start:2"),
            n("d", ""),
        ],
    ),
)
fx(
    "grid/implicit-columns-after",
    "grid",
    n("root", "display:grid;width:300px;height:40px;grid-template-columns:50px 50px;grid-auto-columns:20px", [n("a", "grid-column:4"), n("b", "")]),
)
fx(
    "grid/implicit-columns-before",
    "grid",
    n("root", "display:grid;width:300px;height:40px;grid-template-columns:50px 50px;grid-auto-columns:30px 10px", [n("a", "grid-column:-5 / -4"), n("b", "grid-column:1")]),
)
fx(
    "grid/auto-rows-default-content",
    "grid",
    n("root", "display:grid;width:200px;grid-template-columns:100px 100px", [c("a", 20, 15), c("b", 20, 25), c("c", 20, 5)]),
)
fx(
    "grid/auto-rows-stretch-definite-height",
    "grid",
    n("root", "display:grid;width:200px;height:100px;grid-template-columns:100px 100px", [c("a", 20, 15), c("b", 20, 25), c("c", 20, 5)]),
)
fx(
    "grid/auto-rows-explicit",
    "grid",
    n("root", "display:grid;width:200px;grid-template-columns:100px 100px;grid-auto-rows:30px max-content", [c("a", 20, 15), c("b", 20, 25), c("c", 20, 5)]),
)
fx(
    "grid/auto-flow-column",
    "grid",
    n("root", "display:grid;width:300px;height:60px;grid-template-rows:20px 20px;grid-auto-flow:column;grid-auto-columns:40px", cells("c", 5)),
)
fx(
    "grid/sparse-packing-holes",
    "grid",
    n(
        "root",
        "display:grid;width:300px;grid-template-columns:repeat(3, 100px);grid-auto-rows:20px",
        [n("a", ""), n("wide", "grid-column-end:span 3"), n("b", "grid-column-end:span 2"), n("c", ""), n("d", "")],
    ),
)
fx(
    "grid/locked-to-row-items",
    "grid",
    n(
        "root",
        "display:grid;width:300px;grid-template-columns:repeat(3, 100px);grid-auto-rows:20px",
        [n("a", ""), n("r2", "grid-row:2"), n("r2b", "grid-row:2"), n("b", "")],
    ),
)
fx(
    "grid/spanning-item-intrinsic-tracks",
    "grid",
    n(
        "root",
        "display:grid;width:400px;grid-template-columns:auto auto 50px",
        [n("span", "grid-column:1 / 3", [c("wide", 120, 10)]), c("a", 20, 10), c("b", 30, 10)],
    ),
)
fx(
    "grid/spanning-item-fixed-and-auto",
    "grid",
    n(
        "root",
        "display:grid;width:400px;grid-template-columns:30px auto;column-gap:10px",
        [n("span", "grid-column:1 / 3", [c("wide", 150, 10)])],
    ),
)
fx(
    "grid/container-padding",
    "grid",
    n("root", "display:grid;width:200px;height:100px;padding:10px 20px 5px 15px;grid-template-columns:1fr 1fr;gap:6px", cells("c", 2)),
)
fx(
    "grid/container-auto-height-and-gaps",
    "grid",
    n("root", "width:200px", [n("g", "display:grid;grid-template-columns:1fr 1fr;row-gap:8px;column-gap:4px", [c("a", 10, 20), c("b", 10, 30), c("c", 10, 5)])]),
)
fx(
    "grid/fit-content-container-fr-columns",
    "grid",
    n("root", "width:500px", [n("g", "display:grid;width:fit-content;grid-template-columns:1fr 1fr;column-gap:10px", [c("a", 40, 10), c("b", 70, 10)])]),
)
fx(
    "grid/percent-track",
    "grid",
    n("root", "display:grid;width:400px;height:40px;grid-template-columns:25% 1fr", cells("c", 2)),
)
fx(
    "grid/percent-item-size-of-area",
    "grid",
    n("root", "display:grid;width:200px;height:100px;grid-template-columns:100px 100px", [n("a", "width:50%;height:25%"), n("b", "width:100%")]),
)
fx(
    "grid/overlay-single-cell-center",
    "grid",
    n(
        "root",
        "display:grid;width:200px;height:100px;justify-items:center;align-items:center",
        [n("a", "grid-area:1 / 1;width:50px;height:20px"), n("b", "grid-area:1 / 1;width:30px;height:60px")],
    ),
    note="Overlay = 한 칸 grid(모든 자식 grid-area: 1 / 1)",
)
fx(
    "grid/overlay-default-stretch",
    "grid",
    n("root", "display:grid;width:200px;height:100px", [n("a", "grid-area:1 / 1"), n("b", "grid-area:1 / 1;width:30px;height:20px"), n("c", "grid-area:1 / 1;width:fit-content", [n("ck", "width:12px;height:12px")])]),
)
fx(
    "grid/overlay-content-sized",
    "grid",
    n("root", "width:400px", [n("o", "display:grid;width:fit-content;padding:4px", [n("a", "grid-area:1 / 1;width:50px;height:20px"), c("b", 70, 10, None, "grid-area:1 / 1")])]),
)
fx(
    "grid/display-none-child",
    "grid",
    n("root", "display:grid;width:200px;grid-template-columns:100px 100px;grid-auto-rows:20px", [n("a", ""), n("hidden", "display:none"), n("b", "")]),
)
fx(
    "grid/min-height-stretches-auto-rows",
    "grid",
    n("root", "width:200px", [n("g", "display:grid;min-height:100px;grid-template-columns:1fr", [c("a", 10, 20), c("b", 10, 10)])]),
)
fx(
    "grid/nested-in-flex",
    "grid",
    n("root", "display:flex;width:300px;height:80px", [n("g", "display:grid;flex-grow:1;grid-template-columns:1fr 2fr", [n("a", ""), n("b", "")]), n("side", "width:60px")]),
)
fx(
    "grid/min-width-zero-in-bare-fr",
    "grid",
    n("root", "display:grid;width:200px;grid-template-columns:1fr 1fr", [c("wide", 150, 10, None, "min-width:0px"), c("narrow", 20, 10)]),
)
fx(
    "grid/max-content-rows",
    "grid",
    n("root", "display:grid;width:200px;grid-template-columns:100px;grid-template-rows:max-content min-content", [c("a", 50, 15), c("b", 50, 25)]),
)
fx(
    "grid/fr-thirds-distribution",
    "grid",
    n("root", "display:grid;width:100px;height:20px;grid-template-columns:1fr 1fr 1fr", cells("c", 3)),
    bundle="distribution",
)
fx(
    "grid/center-odd-free",
    "grid",
    n("root", "display:grid;width:101px;height:21px;grid-template-columns:20px;grid-template-rows:10px;justify-content:center;align-content:center", cells("c", 1)),
    bundle="distribution",
)
fx(
    "position/abs-in-grid-static-start",
    "positioning",
    n("root", "display:grid;width:200px;height:100px;padding:10px;grid-template-columns:1fr", [n("flow", "height:20px"), n("a", "position:absolute;width:30px;height:20px")]),
)
fx(
    "position/abs-in-grid-static-self-center",
    "positioning",
    n("root", "display:grid;width:200px;height:100px;padding:10px;justify-items:center", [n("a", "position:absolute;width:30px;height:20px;align-self:end")]),
)
fx(
    "position/abs-in-grid-insets",
    "positioning",
    n("root", "display:grid;width:200px;height:100px;padding:10px;grid-template-columns:1fr 1fr", [n("a", "position:absolute;right:5px;bottom:5px;width:30px;height:20px"), n("b", "")]),
)

# --- flex item 자동 최소 크기(IX.11.23) ---

fx(
    "flex/auto-min-content-prevents-shrink",
    "flex",
    n("root", "display:flex;width:200px;height:20px", [c("a", 150, 10), c("b", 150, 10)]),
    note="min-width: auto = min-content(150): 줄어들지 않고 넘침",
)
fx(
    "flex/auto-min-specified-size-smaller",
    "flex",
    n("root", "display:flex;width:100px;height:20px", [c("a", 120, 10, None, "width:60px;flex-basis:200px"), n("b", "width:40px")]),
    note="지정 크기 suggestion(60) < 콘텐츠(120): 최소 60",
)
fx(
    "flex/auto-min-clamped-by-max",
    "flex",
    n("root", "display:flex;width:100px;height:20px", [c("a", 150, 10, None, "max-width:90px"), c("b", 150, 10, None, "min-width:0px")]),
)
fx(
    "flex/auto-min-nested-container",
    "flex",
    n(
        "root",
        "display:flex;width:120px;height:30px",
        [n("box", "display:flex;column-gap:4px;padding:0 3px", [c("x", 40, 10), c("y", 50, 10)]), n("other", "width:100px")],
    ),
    note="container item의 자동 최소 크기 = 자식 min-content 기여 합",
)
fx(
    "flex/auto-min-column-content-height",
    "flex",
    n("root", "display:flex;flex-direction:column;width:60px;height:50px", [c("a", 20, 40), c("b", 20, 40)]),
)
fx(
    "flex/auto-min-wrapping-block-content",
    "flex",
    n(
        "root",
        "display:flex;width:100px;height:20px",
        [n("a", "width:200px", [c("a1", 70, 10), c("a2", 30, 10)]), n("b", "width:200px")],
    ),
    note="block item의 min-content = 가장 넓은 자식(70)",
)
fx(
    "flex/auto-min-aspect-ratio-main-axis",
    "flex",
    n("root", "display:flex;width:100px;height:40px;align-items:flex-start", [n("a", "height:40px;aspect-ratio:2/1;flex-basis:150px"), n("b", "width:80px")]),
    note="비율 축이 main 축이면 자동 최소 크기를 따름(14장 E10 제외)",
)
fx(
    "flex/auto-min-grow-unaffected",
    "flex",
    n("root", "display:flex;width:300px;height:20px", [c("a", 50, 10, None, "flex-grow:1;flex-basis:0px"), c("b", 100, 10, None, "flex-grow:1;flex-basis:0px")]),
    note="flex-basis 0 + grow에서 콘텐츠 최소(100) 위반 item은 freeze",
)
def all_fixtures():
    return FIXTURES
