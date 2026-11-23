//! 스마트 가이드와 측정 표시 자료, transient scene layer (XII.10.16, ADR-0105 결정 3).
//!
//! [`smart_guides`]는 스냅 결과([`SnapResult`])와 후보로 정렬선·가장자리 거리·같은 간격·같은 크기 표시를 만들고,
//! [`measure`]는 선택 후 Option-hover 거리 측정을 만듭니다. 값은 모두 대상 부모 local CSS px입니다.
//! [`build_guide_layer`]는 이 자료를 화면 좌표 scene snapshot(문서 scene과 별도 layer)으로 바꿉니다. 이 layer는 문서·
//! project snapshot·저장 경로 어디에도 들어가지 않습니다(selection overlay와 같은 규칙, XII.4.4).
//!
//! # 결정
//!
//! - **C2 측정 라벨 glyph 출처: 셸의 시스템 UI 글꼴.** 코어는 라벨 문자열(CSS 길이 표기, `24px`)과 자리·배경 상자만
//!   냅니다. 문서 글꼴 asset이나 코어 text layout을 쓰지 않습니다. 라벨은 문서 내용이 아니라 도구 UI이고, 대비 높이기·
//!   글자 크기 같은 시스템 접근성 설정을 셸이 그대로 따라야 하기 때문입니다.
//! - **C5 Text 측정 기준: layout 상자.** Text도 다른 node처럼 scene 행의 layout 상자(줄 상자 묶음) 가장자리로 잽니다.
//!   baseline 사이 거리는 내지 않습니다. 코드 export의 CSS에는 baseline 사이 간격 속성이 없고, 상자 거리가 곧 CSS
//!   `margin`·`gap`·inset 값이라 측정값을 그대로 옮길 수 있어야 하기 때문입니다.
//! - **대비 높이기·투명도 줄이기**: 선 두께·색·라벨 배경 불투명도는 셸이 [`GuideStyle`]로 넘깁니다(ShellTheme XII.9.3가
//!   값을 정함). 코어는 기본 모양을 두지 않습니다.

use std::sync::Arc;

use stellar_core_types::color::Color;
use stellar_core_types::ids::StableId;
use stellar_core_types::revision::{DocumentRevision, SceneRevision};
use stellar_core_types::value::{Rect, Size2, Transform2D, Vec2};
use stellar_document::css::value::format_length;
use stellar_scene::builder::{
    FrameClipInput, FrameInput, ModifierPaint, NodeInput, PathInput, SceneBuilder, StrokeInput,
};
use stellar_scene::geometry::{FillRule, PathCommand, PathGeometry};
use stellar_scene::node::{CornerRadii, SceneNode};
use stellar_scene::snapshot::SceneSnapshot;
use stellar_scene::stack::Paint;

use super::{CandidateKind, SnapCandidates, SnapResult};

/// 정렬선 하나. `axis` 0은 세로선(x = `position`), 1은 가로선(y = `position`)입니다.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GuideLine {
    /// 축.
    pub axis: usize,
    /// 선 좌표.
    pub position: f32,
    /// 반대 축 시작.
    pub from: f32,
    /// 반대 축 끝.
    pub to: f32,
    /// 선을 만든 후보 종류.
    pub kind: CandidateKind,
}

/// 거리 라벨 하나. `axis` 축 위 `from`–`to` 구간이고, 반대 축 좌표 `at`에 그립니다.
#[derive(Clone, Debug, PartialEq)]
pub struct DistanceLabel {
    /// 잰 축.
    pub axis: usize,
    /// 구간 시작.
    pub from: f32,
    /// 구간 끝.
    pub to: f32,
    /// 반대 축 좌표.
    pub at: f32,
    /// 거리(px).
    pub value: f32,
    /// 라벨 문자열(CSS 길이 표기).
    pub text: String,
}

/// 같은 간격·같은 크기 표시.
#[derive(Clone, Debug, PartialEq)]
pub enum Marker {
    /// 같은 간격: 같은 값 `gap`인 구간들.
    EqualSpacing {
        /// 축.
        axis: usize,
        /// 간격.
        gap: f32,
        /// 구간(축 위 시작, 끝, 반대 축 좌표).
        segments: Vec<[f32; 3]>,
    },
    /// 같은 크기: 움직인 상자와 같은 크기의 형제 상자들.
    SameSize {
        /// 축(0 너비, 1 높이).
        axis: usize,
        /// 크기.
        size: f32,
        /// 같은 크기 상자(움직인 상자 포함).
        boxes: Vec<[f32; 4]>,
    },
}

/// 한 update의 스마트 가이드 자료.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GuideOverlay {
    /// 정렬선.
    pub lines: Vec<GuideLine>,
    /// 거리 라벨.
    pub labels: Vec<DistanceLabel>,
    /// 같은 간격·같은 크기.
    pub markers: Vec<Marker>,
}

fn label(axis: usize, from: f32, to: f32, at: f32) -> DistanceLabel {
    let value = to - from;
    DistanceLabel {
        axis,
        from,
        to,
        at,
        value,
        text: format_length(value),
    }
}

fn anchors(rect: [f32; 4], axis: usize) -> [f32; 3] {
    [
        rect[axis],
        rect[axis] + rect[axis + 2] / 2.0,
        rect[axis] + rect[axis + 2],
    ]
}

/// 스냅 결과의 정렬선·거리·같은 간격·같은 크기 자료.
pub fn smart_guides(result: &SnapResult, candidates: &SnapCandidates) -> GuideOverlay {
    let rect = result.rect;
    let mut overlay = GuideOverlay::default();
    for axis in 0..2 {
        let other = 1 - axis;
        if let (Some(position), Some(kind)) = (result.guides[axis], result.guide_kind[axis]) {
            // 선은 움직인 상자와 그 좌표를 가장자리·중심으로 가진 후보 상자를 모두 덮습니다.
            let mut from = rect[other];
            let mut to = rect[other] + rect[other + 2];
            if matches!(kind, CandidateKind::Parent | CandidateKind::Sibling) {
                for c in &candidates.boxes {
                    if anchors(c.rect, axis)
                        .iter()
                        .any(|v| v.to_bits() == position.to_bits())
                    {
                        from = from.min(c.rect[other]);
                        to = to.max(c.rect[other] + c.rect[other + 2]);
                    }
                }
            }
            overlay.lines.push(GuideLine {
                axis,
                position,
                from,
                to,
                kind,
            });
        }
        // 가장 가까운 형제까지의 거리(겹치는 축).
        if let Some(gap) = result.distance[axis] {
            let start = rect[axis];
            let end = rect[axis] + rect[axis + 2];
            let nearest = candidates
                .band(rect, axis, gap + 1.0)
                .into_iter()
                .map(|i| candidates.boxes[i].rect)
                .find(|c| {
                    (c[axis] - end - gap).abs() <= f32::EPSILON * gap.max(1.0) * 4.0
                        || (start - (c[axis] + c[axis + 2]) - gap).abs()
                            <= f32::EPSILON * gap.max(1.0) * 4.0
                });
            if let Some(c) = nearest {
                let lo = rect[other].max(c[other]);
                let hi = (rect[other] + rect[other + 2]).min(c[other] + c[other + 2]);
                let at = (lo + hi) / 2.0;
                overlay.labels.push(if c[axis] >= end {
                    label(axis, end, c[axis], at)
                } else {
                    label(axis, c[axis] + c[axis + 2], start, at)
                });
            }
        }
        if let Some(gap) = result.spacing[axis] {
            let mut segments = Vec::new();
            let mut ordered: Vec<[f32; 4]> = candidates
                .siblings()
                .map(|(_, c)| c.rect)
                .chain(std::iter::once(rect))
                .collect();
            ordered.sort_by(|a, b| a[axis].total_cmp(&b[axis]));
            for pair in ordered.windows(2) {
                let (a, b) = (pair[0], pair[1]);
                if a[other] >= b[other] + b[other + 2] || b[other] >= a[other] + a[other + 2] {
                    continue;
                }
                let space = b[axis] - a[axis] - a[axis + 2];
                if (space - gap).abs() <= 1e-3 {
                    let lo = a[other].max(b[other]);
                    let hi = (a[other] + a[other + 2]).min(b[other] + b[other + 2]);
                    segments.push([a[axis] + a[axis + 2], b[axis], (lo + hi) / 2.0]);
                }
            }
            overlay.markers.push(Marker::EqualSpacing {
                axis,
                gap,
                segments,
            });
        }
        if let Some(size) = result.size_match[axis] {
            let mut boxes: Vec<[f32; 4]> = candidates
                .siblings()
                .map(|(_, c)| c.rect)
                .filter(|c| c[axis + 2].to_bits() == size.to_bits())
                .collect();
            boxes.push(rect);
            overlay.markers.push(Marker::SameSize { axis, size, boxes });
        }
    }
    overlay
}

/// 선택 후 Option-hover 거리 측정: 고른 상자 `selected`와 hover한 상자 `hovered`(같은 부모 local)의 가장자리 거리.
///
/// 떨어져 있으면 축마다 마주 보는 가장자리 사이 거리, 한쪽이 다른 쪽을 품으면 네 변 각각의 안쪽 거리입니다.
pub fn measure(selected: [f32; 4], hovered: [f32; 4]) -> Vec<DistanceLabel> {
    let mut out = Vec::new();
    let contains = |outer: [f32; 4], inner: [f32; 4]| {
        (0..2).all(|a| outer[a] <= inner[a] && inner[a] + inner[a + 2] <= outer[a] + outer[a + 2])
    };
    let inside = if contains(hovered, selected) {
        Some((hovered, selected))
    } else if contains(selected, hovered) {
        Some((selected, hovered))
    } else {
        None
    };
    for axis in 0..2 {
        let other = 1 - axis;
        let mid = |r: [f32; 4]| r[other] + r[other + 2] / 2.0;
        if let Some((outer, inner)) = inside {
            let at = mid(inner);
            if inner[axis] > outer[axis] {
                out.push(label(axis, outer[axis], inner[axis], at));
            }
            let (inner_end, outer_end) =
                (inner[axis] + inner[axis + 2], outer[axis] + outer[axis + 2]);
            if outer_end > inner_end {
                out.push(label(axis, inner_end, outer_end, at));
            }
            continue;
        }
        let (a, b) = if selected[axis] <= hovered[axis] {
            (selected, hovered)
        } else {
            (hovered, selected)
        };
        let end = a[axis] + a[axis + 2];
        if b[axis] > end {
            let lo = a[other].max(b[other]);
            let hi = (a[other] + a[other + 2]).min(b[other] + b[other + 2]);
            let at = if lo <= hi {
                (lo + hi) / 2.0
            } else {
                mid(selected)
            };
            out.push(label(axis, end, b[axis], at));
        }
    }
    out
}

/// 가이드 layer 모양(셸이 ShellTheme로 정함, 대비 높이기·투명도 줄이기 반영).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GuideStyle {
    /// 정렬선·거리선 색.
    pub line: Color,
    /// 선 두께(view px).
    pub line_width: f32,
    /// 라벨 배경 색.
    pub label_fill: Color,
    /// 라벨 배경 상자 크기(view px). 문자열은 셸이 이 상자 가운데 그립니다(C2).
    pub label_size: [f32; 2],
}

/// layer 생성 오류.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuideLayerError;

/// 셸이 라벨 문자열을 그릴 자리(view 좌표 상자 가운데).
#[derive(Clone, Debug, PartialEq)]
pub struct LabelPlacement {
    /// view 좌표 x, y, width, height.
    pub rect: [f32; 4],
    /// 문자열.
    pub text: String,
}

/// 가이드 transient layer와 라벨 자리.
#[derive(Clone, Debug)]
pub struct GuideLayer {
    /// 화면 좌표 scene snapshot(선·라벨 배경). 문서 scene과 별도이고 저장되지 않습니다.
    pub layer: SceneSnapshot,
    /// 라벨 문자열 자리.
    pub labels: Vec<LabelPlacement>,
}

/// `axis` 좌표가 `along`, 반대 축 좌표가 `at`인 점을 view로 옮깁니다.
fn point(transform: &Transform2D, axis: usize, along: f32, at: f32) -> Option<Vec2> {
    let (x, y) = if axis == 0 { (along, at) } else { (at, along) };
    transform.apply(Vec2::new(x, y).ok()?).ok()
}

/// 자료를 화면 좌표 layer로 바꿉니다. `to_view`는 부모 local → view(화면) 변환입니다.
pub fn build_guide_layer(
    overlay: &GuideOverlay,
    to_view: Transform2D,
    style: &GuideStyle,
    revision: (DocumentRevision, SceneRevision),
) -> Result<GuideLayer, GuideLayerError> {
    let mut builder = SceneBuilder::new();
    let mut root = builder.root_scope();
    let mut segments: Vec<[Vec2; 2]> = Vec::new();
    for line in &overlay.lines {
        // 정렬선: axis 0이면 x = position인 세로선이므로 y(반대 축)를 따라 그립니다.
        let a = point(&to_view, 1 - line.axis, line.from, line.position).ok_or(GuideLayerError)?;
        let b = point(&to_view, 1 - line.axis, line.to, line.position).ok_or(GuideLayerError)?;
        segments.push([a, b]);
    }
    let mut labels = Vec::new();
    let mut label_at =
        |from: f32, to: f32, at: f32, axis: usize, text: &str, segments: &mut Vec<[Vec2; 2]>| {
            let a = point(&to_view, axis, from, at).ok_or(GuideLayerError)?;
            let b = point(&to_view, axis, to, at).ok_or(GuideLayerError)?;
            segments.push([a, b]);
            let [w, h] = style.label_size;
            let cx = (a.x() + b.x()) / 2.0;
            let cy = (a.y() + b.y()) / 2.0;
            labels.push(LabelPlacement {
                rect: [cx - w / 2.0, cy - h / 2.0, w, h],
                text: text.to_string(),
            });
            Ok::<(), GuideLayerError>(())
        };
    for l in &overlay.labels {
        label_at(l.from, l.to, l.at, l.axis, &l.text, &mut segments)?;
    }
    for marker in &overlay.markers {
        if let Marker::EqualSpacing {
            axis,
            gap,
            segments: parts,
        } = marker
        {
            for [from, to, at] in parts {
                label_at(*from, *to, *at, *axis, &format_length(*gap), &mut segments)?;
            }
        }
    }
    let id = StableId::NIL;
    for [a, b] in &segments {
        let geometry = PathGeometry::new(vec![PathCommand::MoveTo(*a), PathCommand::LineTo(*b)])
            .map_err(|_| GuideLayerError)?;
        let min = Vec2::new(a.x().min(b.x()), a.y().min(b.y())).map_err(|_| GuideLayerError)?;
        let size = Size2::new((a.x() - b.x()).abs(), (a.y() - b.y()).abs())
            .map_err(|_| GuideLayerError)?;
        let mut node = NodeInput::new(id, Transform2D::identity());
        node.paint = ModifierPaint {
            strokes: vec![StrokeInput {
                paint: Paint::Solid(style.line),
                width: style.line_width,
            }],
            ..ModifierPaint::default()
        };
        builder
            .build_path(
                &mut root,
                &PathInput {
                    node,
                    layout_rect: Rect::new(min, size),
                    geometry,
                    fill_rule: FillRule::NonZero,
                },
            )
            .map_err(|_| GuideLayerError)?;
    }
    for placement in &labels {
        let [x, y, w, h] = placement.rect;
        let transform = Transform2D::new(1.0, 0.0, 0.0, 1.0, x, y).map_err(|_| GuideLayerError)?;
        let frame = builder
            .begin_frame(
                &root,
                &FrameInput {
                    node: NodeInput::new(id, transform),
                    size: Size2::new(w, h).map_err(|_| GuideLayerError)?,
                    corner_radii: CornerRadii::ZERO,
                    background: Some(style.label_fill),
                    border: None,
                    clip: FrameClipInput::None,
                },
            )
            .map_err(|_| GuideLayerError)?;
        builder
            .finish_frame(&mut root, frame)
            .map_err(|_| GuideLayerError)?;
    }
    let roots: Vec<Arc<SceneNode>> = builder.finish(root);
    let layer = SceneSnapshot::new(revision.0, revision.1, roots).map_err(|_| GuideLayerError)?;
    Ok(GuideLayer { layer, labels })
}

#[cfg(test)]
mod tests {
    use super::super::{snap, SnapCandidate, SnapMode, SnapSettings};
    use super::*;
    use stellar_core_types::color::ColorSpace;

    fn set(siblings: &[[f32; 4]]) -> SnapCandidates {
        let mut boxes = vec![SnapCandidate {
            rect: [0.0, 0.0, 400.0, 300.0],
            kind: CandidateKind::Parent,
        }];
        boxes.extend(siblings.iter().map(|r| SnapCandidate {
            rect: *r,
            kind: CandidateKind::Sibling,
        }));
        SnapCandidates::new(boxes, [0.0; 4])
    }

    const OBJECTS: SnapSettings = SnapSettings {
        objects: true,
        pixels: false,
    };

    #[test]
    fn alignment_line_spans_the_moved_box_and_matching_siblings_with_distance_label() {
        let candidates = set(&[[100.0, 10.0, 50.0, 20.0]]);
        let result = snap(
            [102.0, 100.0, 30.0, 30.0],
            SnapMode::Move,
            6.0,
            OBJECTS,
            &candidates,
        )
        .unwrap();
        let overlay = smart_guides(&result, &candidates);
        assert_eq!(
            overlay.lines,
            vec![GuideLine {
                axis: 0,
                position: 100.0,
                from: 10.0,
                to: 130.0,
                kind: CandidateKind::Sibling
            }]
        );
        // 위 형제까지 세로 거리 70px.
        let label = &overlay.labels[0];
        assert_eq!(
            (label.axis, label.from, label.to, label.value),
            (1, 30.0, 100.0, 70.0)
        );
        assert_eq!(label.text, "70px");
    }

    #[test]
    fn equal_spacing_and_same_size_markers() {
        let candidates = set(&[[10.0, 100.0, 40.0, 40.0], [80.0, 100.0, 40.0, 40.0]]);
        let result = snap(
            [148.0, 100.0, 40.0, 40.0],
            SnapMode::Move,
            3.0,
            OBJECTS,
            &candidates,
        )
        .unwrap();
        let overlay = smart_guides(&result, &candidates);
        let Some(Marker::EqualSpacing { gap, segments, .. }) = overlay.markers.first() else {
            panic!("{overlay:?}")
        };
        assert_eq!(*gap, 30.0);
        assert_eq!(segments.len(), 2);
        let resize = SnapMode::Resize {
            handle: crate::app::gesture_math::ResizeHandle::Right,
            modifiers: Default::default(),
        };
        let result = snap(
            [200.0, 200.0, 39.0, 10.0],
            resize,
            3.0,
            OBJECTS,
            &candidates,
        )
        .unwrap();
        let overlay = smart_guides(&result, &candidates);
        assert!(overlay
            .markers
            .iter()
            .any(|m| matches!(m, Marker::SameSize { axis: 0, size, boxes } if *size == 40.0 && boxes.len() == 3)));
    }

    #[test]
    fn option_hover_measures_gaps_or_inner_insets() {
        let apart = measure([0.0, 0.0, 10.0, 10.0], [30.0, 0.0, 10.0, 10.0]);
        assert_eq!(apart.len(), 1);
        assert_eq!((apart[0].axis, apart[0].value), (0, 20.0));
        let inside = measure([10.0, 20.0, 30.0, 10.0], [0.0, 0.0, 100.0, 50.0]);
        let values: Vec<f32> = inside.iter().map(|l| l.value).collect();
        assert_eq!(values, vec![10.0, 60.0, 20.0, 20.0]);
    }

    fn style(width: f32, alpha: f32) -> GuideStyle {
        GuideStyle {
            line: Color::new(ColorSpace::Srgb, 1.0, 0.2, 0.2, alpha).unwrap(),
            line_width: width,
            label_fill: Color::new(ColorSpace::Srgb, 1.0, 0.2, 0.2, alpha).unwrap(),
            label_size: [40.0, 16.0],
        }
    }

    #[test]
    fn guide_layer_is_a_separate_view_space_snapshot_and_follows_the_style() {
        let candidates = set(&[[100.0, 10.0, 50.0, 20.0]]);
        let result = snap(
            [102.0, 100.0, 30.0, 30.0],
            SnapMode::Move,
            6.0,
            OBJECTS,
            &candidates,
        )
        .unwrap();
        let overlay = smart_guides(&result, &candidates);
        let zoom = Transform2D::new(2.0, 0.0, 0.0, 2.0, 5.0, 5.0).unwrap();
        let revision = (DocumentRevision(3), SceneRevision(4));
        let normal = build_guide_layer(&overlay, zoom, &style(1.0, 0.8), revision).unwrap();
        // 정렬선 1, 거리선 1, 라벨 배경 1.
        assert_eq!(normal.layer.node_count(), 3);
        assert_eq!(normal.labels.len(), 1);
        assert_eq!(normal.labels[0].text, "70px");
        // 라벨은 거리선 가운데(view): x = 100 + 15 → 235, y = (30 + 100)/2 → 135.
        assert_eq!(normal.labels[0].rect, [215.0, 127.0, 40.0, 16.0]);
        let contrast = build_guide_layer(&overlay, zoom, &style(2.0, 1.0), revision).unwrap();
        assert_ne!(
            format!("{:?}", normal.layer),
            format!("{:?}", contrast.layer),
            "대비 높이기 모양이 layer에 반영"
        );
    }
}
