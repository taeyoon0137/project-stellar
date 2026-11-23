//! Frame 레이아웃 그리드 스냅 선·표시와 픽셀 그리드 overlay (XII.10.20, ADR-0117 결정 4).
//!
//! - 트랙 계산은 문서 값 [`LayoutGrid::tracks`] 하나입니다. 이 module은 그 트랙을 스냅 선과 화면 상자로 옮깁니다.
//! - 스냅은 표시 여부(그리드 `visible`, 레이아웃 그리드 토글)와 관계없이 트랙 가장자리를 후보로 냅니다. Control
//!   일시 해제는 스냅 엔진 호출 쪽(`CanvasViewOptions::snapping`)이 따릅니다.
//! - 레이아웃 그리드는 문서 layout 입력이 아니므로 자식 배치·scene 결과를 바꾸지 않습니다.
//! - 픽셀 그리드는 배율 800% 이상이고 토글이 켜졌을 때만 가시 영역의 정수 world 선을 화면 좌표로 냅니다.

use stellar_commands::view_tabs::CanvasViewport;
use stellar_document::guides::LayoutGrid;

use crate::canvas::view_options::{CanvasViewOptions, ViewToggle};

/// 그리드 Frame의 자식(부모 local 좌표)에 쓸 스냅 선. `rect`는 그 Frame local 상자(x, y, width, height)입니다.
/// 축마다 오름차순이고 같은 값은 한 번입니다.
pub fn grid_snap_lines(grids: &[LayoutGrid], rect: [f32; 4]) -> [Vec<f32>; 2] {
    let mut lines: [Vec<f32>; 2] = [Vec::new(), Vec::new()];
    for grid in grids {
        let tracks = grid.tracks(rect[2], rect[3]);
        for (axis, list) in tracks.iter().enumerate() {
            for (start, end) in list {
                lines[axis].push(rect[axis] + start);
                lines[axis].push(rect[axis] + end);
            }
        }
    }
    for list in &mut lines {
        list.retain(|v| v.is_finite());
        list.sort_by(f32::total_cmp);
        list.dedup_by(|a, b| a.to_bits() == b.to_bits());
    }
    lines
}

/// 화면에 그릴 트랙 상자 하나(world 좌표, 축 정렬 Frame 기준).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridBand {
    /// 그 Frame의 몇 번째 그리드인지(그리는 순서).
    pub grid: usize,
    /// world x, y, width, height.
    pub rect: [f32; 4],
}

/// 표시할 트랙 상자. 레이아웃 그리드 토글이 꺼졌거나 그리드가 `visible`이 아니면 내지 않습니다. `origin`은 Frame
/// local 원점의 world 좌표(평행 이동만 있는 Frame)입니다.
pub fn grid_bands(
    grids: &[LayoutGrid],
    rect: [f32; 4],
    origin: [f32; 2],
    options: &CanvasViewOptions,
) -> Vec<GridBand> {
    if !options.get(ViewToggle::ShowLayoutGrids) {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (index, grid) in grids.iter().enumerate().filter(|(_, g)| g.visible) {
        let [xs, ys] = grid.tracks(rect[2], rect[3]);
        let x0 = origin[0] + rect[0];
        let y0 = origin[1] + rect[1];
        let full_x = [(0.0, rect[2])];
        let full_y = [(0.0, rect[3])];
        let xs: &[(f32, f32)] = if xs.is_empty() { &full_x } else { &xs };
        let ys: &[(f32, f32)] = if ys.is_empty() { &full_y } else { &ys };
        for &(xa, xb) in xs {
            for &(ya, yb) in ys {
                out.push(GridBand {
                    grid: index,
                    rect: [x0 + xa, y0 + ya, xb - xa, yb - ya],
                });
            }
        }
    }
    out
}

/// 픽셀 그리드 선(화면 pt). `x`는 세로선의 화면 x, `y`는 가로선의 화면 y입니다.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PixelGrid {
    /// 세로선.
    pub x: Vec<f64>,
    /// 가로선.
    pub y: Vec<f64>,
}

/// 가시 영역의 픽셀 그리드. 토글이 꺼졌거나 배율이 기준 미만이면 `None`입니다. 선 수는 화면 크기/배율에 비례합니다
/// (800%에서 화면 8pt마다 하나).
pub fn pixel_grid_lines(
    viewport: CanvasViewport,
    view_size: [f64; 2],
    options: &CanvasViewOptions,
) -> Option<PixelGrid> {
    let zoom = viewport.zoom();
    if !options.pixel_grid_now(zoom) {
        return None;
    }
    let center = [viewport.center_x(), viewport.center_y()];
    let mut out = [Vec::new(), Vec::new()];
    for axis in 0..2 {
        let size = view_size[axis];
        if !(size.is_finite() && size > 0.0) {
            return None;
        }
        let low = center[axis] - size / 2.0 / zoom;
        let high = center[axis] + size / 2.0 / zoom;
        let first = low.ceil() as i64;
        let last = high.floor() as i64;
        out[axis] = (first..=last)
            .map(|k| (k as f64 - center[axis]) * zoom + size / 2.0)
            .collect();
    }
    let [x, y] = out;
    Some(PixelGrid { x, y })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::select_fixtures::save_bytes;
    use crate::canvas::snap::{snap, SnapCandidates, SnapLines, SnapMode, SnapSettings};
    use crate::canvas::transform::fixtures::{canvas_document, open, scene};
    use stellar_document::guides::{GridAlign, GridKind, TrackSize};

    #[test]
    fn twelve_column_grid_snaps_children_to_column_edges() {
        // 검증 기준(XII.10.20): 12열 그리드 snap 결과 fixture. 1440px Frame, gap 24px, margin 120px → 열 78px.
        let grid = LayoutGrid::columns(12, 24.0, 120.0).unwrap();
        let lines = grid_snap_lines(&[grid], [0.0, 0.0, 1440.0, 900.0]);
        assert_eq!(lines[0].len(), 24);
        assert_eq!(&lines[0][..4], &[120.0, 198.0, 222.0, 300.0]);
        assert!(lines[1].is_empty());
        let candidates = SnapCandidates::with_lines(
            Vec::new(),
            SnapLines {
                guides: [Vec::new(), Vec::new()],
                grid: lines,
            },
            [0.0, 0.0, 1.0, 1.0],
        );
        let settings = SnapSettings {
            objects: true,
            pixels: true,
        };
        // 왼쪽 가장자리 224 → 222(셋째 열 시작). 크기 78px 상자는 열 하나에 정확히 들어갑니다.
        let moved = snap(
            [224.0, 40.0, 78.0, 50.0],
            SnapMode::Move,
            5.0,
            settings,
            &candidates,
        )
        .unwrap();
        assert_eq!(moved.rect[0], 222.0);
        assert_eq!(
            moved.guide_kind[0],
            Some(crate::canvas::snap::CandidateKind::GridLine)
        );
        // 숨긴 그리드도 스냅 후보는 같습니다(스냅은 표시와 무관).
        let hidden = LayoutGrid {
            visible: false,
            ..grid
        };
        assert_eq!(
            grid_snap_lines(&[hidden], [0.0, 0.0, 1440.0, 900.0]),
            grid_snap_lines(&[grid], [0.0, 0.0, 1440.0, 900.0])
        );
        assert!(grid_bands(
            &[hidden],
            [0.0, 0.0, 1440.0, 900.0],
            [0.0, 0.0],
            &CanvasViewOptions::DEFAULT
        )
        .is_empty());
    }

    #[test]
    fn bands_follow_the_toggle_and_frame_origin() {
        let rows = LayoutGrid {
            kind: GridKind::Rows,
            count: 2,
            size: TrackSize::Px(100.0),
            gap: 10.0,
            margin: 0.0,
            align: GridAlign::Start,
            visible: true,
        };
        let bands = grid_bands(
            &[rows],
            [0.0, 0.0, 300.0, 400.0],
            [10.0, 20.0],
            &CanvasViewOptions::DEFAULT,
        );
        assert_eq!(
            bands,
            vec![
                GridBand {
                    grid: 0,
                    rect: [10.0, 20.0, 300.0, 100.0]
                },
                GridBand {
                    grid: 0,
                    rect: [10.0, 130.0, 300.0, 100.0]
                }
            ]
        );
        let off = CanvasViewOptions::DEFAULT.with(ViewToggle::ShowLayoutGrids, false);
        assert!(grid_bands(&[rows], [0.0, 0.0, 300.0, 400.0], [0.0, 0.0], &off).is_empty());
    }

    #[test]
    fn pixel_grid_appears_only_from_the_zoom_threshold() {
        // 검증 기준(XII.10.20): 줌 경계 표시 fixture.
        let options = CanvasViewOptions::DEFAULT;
        let size = [80.0, 40.0];
        let below = CanvasViewport::new(0.0, 0.0, 7.99).unwrap();
        assert_eq!(pixel_grid_lines(below, size, &options), None);
        let at = CanvasViewport::new(0.5, 0.0, 8.0).unwrap();
        let grid = pixel_grid_lines(at, size, &options).unwrap();
        // 화면 80pt / 8 = world 10px 폭: 정수 선 -4..=5 → 10개, 8pt 간격.
        assert_eq!(grid.x.len(), 10);
        assert!(grid.x.windows(2).all(|w| (w[1] - w[0] - 8.0).abs() < 1e-9));
        assert_eq!(grid.x[0], 4.0);
        assert_eq!(grid.y.len(), 5);
        let off = options.with(ViewToggle::ShowPixelGrid, false);
        assert_eq!(pixel_grid_lines(at, size, &off), None);
    }

    #[test]
    fn layout_grid_does_not_change_the_scene_or_document() {
        // 검증 기준(XII.10.20): 그리드 유무에 따른 layout 결과 동일. 그리드는 layout 입력이 아니라 계산을 거쳐도
        // 게시한 scene과 저장 bytes가 같습니다.
        let (_dir, w) = open("layout-grid-scene", &canvas_document());
        let before = scene(&w);
        let bytes = save_bytes(w.document().unwrap());
        let grid = LayoutGrid::columns(12, 20.0, 0.0).unwrap();
        let _ = grid_snap_lines(&[grid], [0.0, 0.0, 400.0, 300.0]);
        let _ = grid_bands(
            &[grid],
            [0.0, 0.0, 400.0, 300.0],
            [0.0, 0.0],
            &CanvasViewOptions::DEFAULT,
        );
        assert_eq!(*scene(&w), *before);
        assert_eq!(save_bytes(w.document().unwrap()), bytes);
    }
}
