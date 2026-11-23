//! SVG 내보내기 report: 래스터화한 subtree 목록과 이유 (XX.4.3).
//!
//! writer([`crate::svg::write_svg_with`])는 래스터 경계마다 [`RasterizedRegion`] 하나를 paint 순서로 기록하고, 문서
//! 전체의 보이는·vector·래스터 node 수를 함께 둡니다. 이유([`RasterReason`])는 래스터 기능, 그 값의 원천인 registry
//! CSS longhand(IX.11.20), SVG로 정확히 표현할 수 없는 까닭(mapping table 설명, XX.3.1), blend면 backdrop 의존(XX.4.1)
//! 입니다.
//!
//! [`SvgExportReport::to_text`]는 사람이 읽고 fixture로 비교하는 고정 표기입니다. 같은 snapshot이면 같은 문자열입니다.
//!
//! ```text
//! nodes visible=<n> vector=<n> rasterized=<n>
//! region <content|subtrees parent=<id>|-|canvas> roots=<id>,... pixels=<x0>,<y0>,<x1>,<y1>|none nodes=<n>
//!   reason node=<id> feature=<기능> css=<longhand>,...|- why=<설명>[ isolation=<id>|canvas backdrop=<id>|background|none]
//! ```

use std::fmt::Write as _;

use stellar_core_types::ids::StableId;

use crate::svg::mapping::SvgFeature;
use crate::svg::raster::{
    BackdropItem, PixelRect, RasterBoundary, RasterCause, RasterPlan, RasterRegion,
};

/// 래스터 영역 종류.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RasterizedKind {
    /// node 자기 내용만.
    Content,
    /// 같은 부모 아래 연속 subtree. `parent`가 `None`이면 최상위 node 목록입니다.
    Subtrees {
        /// 부모 node.
        parent: Option<StableId>,
    },
    /// 배경을 포함한 내보내기 전체.
    Canvas,
}

/// blend backdrop 의존(XX.4.1 [`crate::svg::raster::BlendDependency`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BackdropReport {
    /// blend가 합성되는 isolation group. `None`이면 내보내기 canvas.
    pub isolation: Option<StableId>,
    /// 영역을 넓힌 앞선 내용. `None`이면 겹치는 앞선 내용이 없습니다.
    pub widest: Option<BackdropSource>,
}

/// 영역을 넓힌 앞선 내용.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BackdropSource {
    /// 앞서 그린 node.
    Node(StableId),
    /// 내보내기 배경.
    Background,
}

/// 래스터화 이유 하나.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RasterReason {
    /// 기능을 쓰는 node.
    pub node: StableId,
    /// 래스터 기능.
    pub feature: SvgFeature,
    /// 값의 원천인 registry CSS longhand(IX.11.20).
    pub css: &'static [&'static str],
    /// SVG로 정확히 표현할 수 없는 까닭(mapping table 설명).
    pub why: &'static str,
    /// blend면 backdrop 의존.
    pub backdrop: Option<BackdropReport>,
}

impl RasterReason {
    fn of(cause: &RasterCause) -> RasterReason {
        RasterReason {
            node: cause.node.source,
            feature: cause.feature,
            css: cause.feature.css_longhands(),
            why: cause.feature.mapping().svg,
            backdrop: cause.blend.map(|blend| BackdropReport {
                isolation: blend.isolation.map(|node| node.source),
                widest: blend.widest.map(|item| match item {
                    BackdropItem::Node(node) => BackdropSource::Node(node.source),
                    BackdropItem::Background => BackdropSource::Background,
                }),
            }),
        }
    }
}

/// 래스터화한 영역 하나.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RasterizedRegion {
    /// 종류.
    pub kind: RasterizedKind,
    /// 영역 최상위 node(paint 순서). Content는 그 node 하나입니다.
    pub roots: Vec<StableId>,
    /// 렌더한 device pixel 상자. 아무것도 그리지 않는 영역이면 `None`(image를 넣지 않음).
    pub pixels: Option<PixelRect>,
    /// 내용이 래스터화된 보이는 node 수.
    pub rasterized_nodes: u64,
    /// 이유(node paint 순서, 같은 node는 기능 선언 순서).
    pub reasons: Vec<RasterReason>,
}

impl RasterizedRegion {
    pub(crate) fn of(boundary: &RasterBoundary, roots: Vec<StableId>) -> RasterizedRegion {
        RasterizedRegion {
            kind: match boundary.region() {
                RasterRegion::Content { .. } => RasterizedKind::Content,
                RasterRegion::Subtrees { parent, .. } => RasterizedKind::Subtrees {
                    parent: parent.map(|node| node.source),
                },
                RasterRegion::Canvas => RasterizedKind::Canvas,
            },
            roots,
            pixels: boundary.pixels(),
            rasterized_nodes: boundary.rasterized_nodes(),
            reasons: boundary.causes().iter().map(RasterReason::of).collect(),
        }
    }
}

/// SVG 내보내기 report.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SvgExportReport {
    /// 보이는 내보내기 node 수.
    pub visible_nodes: u64,
    /// vector로 남은 보이는 node 수.
    pub vector_nodes: u64,
    /// 내용이 래스터화된 보이는 node 수.
    pub rasterized_nodes: u64,
    /// 래스터화한 영역(paint 순서).
    pub rasterized: Vec<RasterizedRegion>,
}

impl SvgExportReport {
    pub(crate) fn new(plan: &RasterPlan, rasterized: Vec<RasterizedRegion>) -> SvgExportReport {
        SvgExportReport {
            visible_nodes: plan.visible_nodes(),
            vector_nodes: plan.vector_nodes(),
            rasterized_nodes: plan.rasterized_nodes(),
            rasterized,
        }
    }

    /// 래스터화한 영역이 없으면 참입니다.
    pub fn is_fully_vector(&self) -> bool {
        self.rasterized.is_empty()
    }

    /// 고정 표기(모듈 문서).
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        // String에 쓰는 write!는 실패하지 않습니다.
        let _ = writeln!(
            out,
            "nodes visible={} vector={} rasterized={}",
            self.visible_nodes, self.vector_nodes, self.rasterized_nodes
        );
        for region in &self.rasterized {
            let kind = match &region.kind {
                RasterizedKind::Content => "content".to_string(),
                RasterizedKind::Subtrees { parent } => format!(
                    "subtrees parent={}",
                    parent.map_or("-".to_string(), |id| id.to_string())
                ),
                RasterizedKind::Canvas => "canvas".to_string(),
            };
            let roots: Vec<String> = region.roots.iter().map(ToString::to_string).collect();
            let pixels = region.pixels.map_or("none".to_string(), |p| {
                format!("{},{},{},{}", p.x0(), p.y0(), p.x1(), p.y1())
            });
            let _ = writeln!(
                out,
                "region {kind} roots={} pixels={pixels} nodes={}",
                roots.join(","),
                region.rasterized_nodes
            );
            for reason in &region.reasons {
                let css = if reason.css.is_empty() {
                    "-".to_string()
                } else {
                    reason.css.join(",")
                };
                let _ = write!(
                    out,
                    "  reason node={} feature={} css={css} why={}",
                    reason.node, reason.feature, reason.why
                );
                if let Some(backdrop) = reason.backdrop {
                    let isolation = backdrop
                        .isolation
                        .map_or("canvas".to_string(), |id| id.to_string());
                    let widest = match backdrop.widest {
                        Some(BackdropSource::Node(id)) => id.to_string(),
                        Some(BackdropSource::Background) => "background".to_string(),
                        None => "none".to_string(),
                    };
                    let _ = write!(out, " isolation={isolation} backdrop={widest}");
                }
                out.push('\n');
            }
        }
        out
    }
}
