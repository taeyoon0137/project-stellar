//! 스냅·스마트 가이드·눈금자·그리드 표시 토글과 스냅 일시 해제 수식키 (XII.10.17, ADR-0117 결정 1).
//!
//! 토글은 창 session 상태입니다. 문서·Git·history에 넣지 않으므로 토글을 바꿔도 save bytes와 history가 그대로입니다.
//! 새 창의 초기값은 앱 설정이 정하고 셸이 [`CanvasViewOptions::with`]로 넘깁니다. 메뉴 명령의 공유 이름은
//! [`ViewToggle::name`]이 원본입니다(ADR-0117 결정 1 표).
//!
//! 끄는 동안 Control(⌃)을 누르면 그 update는 스냅하지 않습니다([`CanvasViewOptions::snapping`]이 `None`). 가이드·
//! 레이아웃 그리드·객체·픽셀 스냅이 모두 꺼지고 스마트 가이드도 그리지 않습니다([`CanvasViewOptions::smart_guides_now`]).

use crate::canvas::snap::SnapSettings;
use crate::canvas::tool::Modifiers;

/// 토글 하나.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ViewToggle {
    /// 부모·형제 객체 스냅.
    SnapToObjects,
    /// 정수 px 스냅.
    SnapToPixelGrid,
    /// 스마트 가이드(정렬선·거리 라벨) 표시.
    ShowSmartGuides,
    /// 눈금자 표시(⌘R).
    ShowRulers,
    /// Frame 레이아웃 그리드 표시.
    ShowLayoutGrids,
    /// 픽셀 그리드 표시(800% 이상에서만 보임).
    ShowPixelGrid,
}

impl ViewToggle {
    /// 전체 토글(표 순서).
    pub const ALL: [ViewToggle; 6] = [
        ViewToggle::SnapToObjects,
        ViewToggle::SnapToPixelGrid,
        ViewToggle::ShowSmartGuides,
        ViewToggle::ShowRulers,
        ViewToggle::ShowLayoutGrids,
        ViewToggle::ShowPixelGrid,
    ];

    /// 셸 메뉴 명령과 공유하는 이름.
    pub const fn name(self) -> &'static str {
        match self {
            ViewToggle::SnapToObjects => "view.snap_to_objects",
            ViewToggle::SnapToPixelGrid => "view.snap_to_pixel_grid",
            ViewToggle::ShowSmartGuides => "view.show_smart_guides",
            ViewToggle::ShowRulers => "view.show_rulers",
            ViewToggle::ShowLayoutGrids => "view.show_layout_grids",
            ViewToggle::ShowPixelGrid => "view.show_pixel_grid",
        }
    }

    /// 공유 이름으로 찾습니다.
    pub fn from_name(name: &str) -> Option<ViewToggle> {
        ViewToggle::ALL.into_iter().find(|t| t.name() == name)
    }
}

/// 창별 캔버스 표시·스냅 토글.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CanvasViewOptions {
    snap_objects: bool,
    snap_pixels: bool,
    smart_guides: bool,
    rulers: bool,
    layout_grids: bool,
    pixel_grid: bool,
}

impl Default for CanvasViewOptions {
    fn default() -> CanvasViewOptions {
        CanvasViewOptions::DEFAULT
    }
}

impl CanvasViewOptions {
    /// 코어 기본값: 모두 켬, 눈금자만 끔.
    pub const DEFAULT: CanvasViewOptions = CanvasViewOptions {
        snap_objects: true,
        snap_pixels: true,
        smart_guides: true,
        rulers: false,
        layout_grids: true,
        pixel_grid: true,
    };

    /// `toggle` 값을 `on`으로 둔 사본(앱 설정 초기값을 넘길 때).
    pub const fn with(mut self, toggle: ViewToggle, on: bool) -> CanvasViewOptions {
        *self.slot(toggle) = on;
        self
    }

    const fn slot(&mut self, toggle: ViewToggle) -> &mut bool {
        match toggle {
            ViewToggle::SnapToObjects => &mut self.snap_objects,
            ViewToggle::SnapToPixelGrid => &mut self.snap_pixels,
            ViewToggle::ShowSmartGuides => &mut self.smart_guides,
            ViewToggle::ShowRulers => &mut self.rulers,
            ViewToggle::ShowLayoutGrids => &mut self.layout_grids,
            ViewToggle::ShowPixelGrid => &mut self.pixel_grid,
        }
    }

    /// 토글이 켜졌는지.
    pub const fn get(&self, toggle: ViewToggle) -> bool {
        match toggle {
            ViewToggle::SnapToObjects => self.snap_objects,
            ViewToggle::SnapToPixelGrid => self.snap_pixels,
            ViewToggle::ShowSmartGuides => self.smart_guides,
            ViewToggle::ShowRulers => self.rulers,
            ViewToggle::ShowLayoutGrids => self.layout_grids,
            ViewToggle::ShowPixelGrid => self.pixel_grid,
        }
    }

    /// 메뉴 명령 하나: 토글을 뒤집고 새 값을 돌려줍니다.
    pub fn toggle(&mut self, toggle: ViewToggle) -> bool {
        let slot = self.slot(toggle);
        *slot = !*slot;
        *slot
    }

    /// 공유 이름의 메뉴 명령. 알 수 없는 이름이면 `None`입니다.
    pub fn perform(&mut self, name: &str) -> Option<bool> {
        ViewToggle::from_name(name).map(|toggle| self.toggle(toggle))
    }

    /// 스냅 일시 해제 수식키(Control)가 눌렸는지.
    pub const fn suspended(modifiers: Modifiers) -> bool {
        modifiers.control
    }

    /// 지금 update에 쓸 스냅 설정과 부모 local px 허용 오차. 일시 해제 중이거나 허용 오차가 없으면 `None`이라
    /// 스냅 결과가 없습니다. 가이드·레이아웃 그리드 선은 두 토글과 무관하게 후보가 있으면 씁니다(ADR-0105 결정 3).
    pub fn snapping(
        &self,
        modifiers: Modifiers,
        tolerance: Option<f32>,
    ) -> Option<(SnapSettings, f32)> {
        if CanvasViewOptions::suspended(modifiers) {
            return None;
        }
        Some((
            SnapSettings {
                objects: self.snap_objects,
                pixels: self.snap_pixels,
            },
            tolerance?,
        ))
    }

    /// 지금 스마트 가이드를 그릴지(토글이 켜졌고 일시 해제 중이 아님).
    pub const fn smart_guides_now(&self, modifiers: Modifiers) -> bool {
        self.smart_guides && !CanvasViewOptions::suspended(modifiers)
    }

    /// 픽셀 그리드를 보일지: 토글이 켜졌고 배율이 ADR-0104 기준 이상.
    pub fn pixel_grid_now(&self, zoom: f64) -> bool {
        self.pixel_grid && crate::canvas::tool_units::pixel_grid_visible(zoom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::select_fixtures::save_bytes;
    use crate::canvas::snap::{snap, CandidateKind, SnapCandidate, SnapCandidates, SnapLines, SnapMode};
    use crate::canvas::transform::drag::MoveDrag;
    use crate::canvas::transform::fixtures::{canvas_document, open, scene, select};

    const ADR: &str = include_str!(
        "../../../../docs/adr/ADR-0117-canvas-guides-grids-flow-drag-navigation-clipboard.md"
    );

    #[test]
    fn names_and_defaults_match_the_adr_table() {
        let defaults = CanvasViewOptions::default();
        for toggle in ViewToggle::ALL {
            let line = ADR
                .lines()
                .find(|l| l.contains(&format!("`{}`", toggle.name())))
                .unwrap_or_else(|| panic!("ADR-0117 표에 {} 없음", toggle.name()));
            let expected = if defaults.get(toggle) { "| 켬 |" } else { "| 끔 |" };
            assert!(line.ends_with(expected), "{line}");
            assert_eq!(ViewToggle::from_name(toggle.name()), Some(toggle));
        }
        assert_eq!(ViewToggle::from_name("view.unknown"), None);
    }

    #[test]
    fn toggles_flip_per_window_and_do_not_touch_the_document() {
        // 검증 기준(XII.10.17): 토글 후 save bytes 불변. history도 그대로입니다.
        let (_dir, w) = open("view-options", &canvas_document());
        let before = save_bytes(w.document().unwrap());
        let history = w.history().history().len();
        let mut options = CanvasViewOptions::default();
        let other = CanvasViewOptions::default();
        for toggle in ViewToggle::ALL {
            let was = options.get(toggle);
            assert_eq!(options.toggle(toggle), !was);
            assert_eq!(options.perform(toggle.name()), Some(was));
            assert_eq!(options.toggle(toggle), !was);
        }
        assert_ne!(options, other, "다른 창의 토글은 그대로");
        assert_eq!(save_bytes(w.document().unwrap()), before);
        assert_eq!(w.history().history().len(), history);
    }

    #[test]
    fn control_suspends_every_snap_kind_and_smart_guides() {
        // 검증 기준(XII.10.17): 일시 해제 중 snap 결과 없음.
        let options = CanvasViewOptions::default();
        let control = Modifiers {
            control: true,
            ..Modifiers::NONE
        };
        assert!(options.snapping(Modifiers::NONE, Some(5.0)).is_some());
        assert_eq!(options.snapping(control, Some(5.0)), None);
        assert!(options.smart_guides_now(Modifiers::NONE));
        assert!(!options.smart_guides_now(control));
        let off = options.with(ViewToggle::ShowSmartGuides, false);
        assert!(!off.smart_guides_now(Modifiers::NONE));

        // 가이드·그리드·형제·픽셀 후보가 모두 있는 상자: 해제하지 않으면 맞고, 해제하면 그대로입니다.
        let candidates = SnapCandidates::with_lines(
            vec![SnapCandidate {
                rect: [100.0, 0.0, 50.0, 50.0],
                kind: CandidateKind::Sibling,
            }],
            SnapLines {
                guides: [vec![40.0], vec![]],
                grid: [vec![], vec![70.0]],
            },
            [0.0, 0.0, 10.0, 10.0],
        );
        let rect = [41.5, 68.3, 10.0, 10.0];
        let (settings, tolerance) = options.snapping(Modifiers::NONE, Some(5.0)).unwrap();
        let snapped = snap(rect, SnapMode::Move, tolerance, settings, &candidates).unwrap();
        assert_eq!(snapped.rect[0], 40.0);
        assert!(options.snapping(control, Some(5.0)).is_none());
    }

    #[test]
    fn suspended_drag_moves_by_the_raw_delta() {
        // 실제 이동 제스처: 일시 해제 update는 스냅 없이 pointer 이동 그대로입니다.
        let (_dir, mut w) = open("view-options-drag", &canvas_document());
        select(&mut w, &[13]);
        let scene = scene(&w);
        let drag = MoveDrag::begin(&w, &scene, None, [0.0, 0.0], false, None).unwrap();
        let options = CanvasViewOptions::default();
        let control = Modifiers {
            control: true,
            ..Modifiers::NONE
        };
        let raw = drag
            .update([3.3, 4.4], false, options.snapping(control, Some(5.0)))
            .unwrap();
        assert_eq!(raw.delta, [3.3, 4.4]);
        assert!(raw.snap.is_none());
    }
}
