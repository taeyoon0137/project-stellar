//! 줌·팬 입력을 camera(viewport)에 연결 (XII.10.37, ADR-0117 결정 8.2).
//!
//! [`Navigator`]는 창 하나의 탐색 상태(Hand 드래그 위치, 관성 차단, smart magnify 이전 viewport)를 들고 입력 하나를
//! 새 [`CanvasViewport`]로 바꿉니다. 문서 geometry·history는 바꾸지 않습니다. 배율 상하한·줌 단계는 ADR-0104 값
//! (`tool_units`, `view_tabs`)입니다.
//!
//! | 입력 | 결과 |
//! |---|---|
//! | 핀치 | 커서 기준 `× (1 + magnification)` |
//! | 정밀 스크롤·관성 | 팬(새 제스처가 시작되면 남은 관성 event를 버림) |
//! | ⌘+스크롤 | 커서 기준 `× 2^(dy/100)` |
//! | 마우스 휠(정밀 아님) Shift | 세로 값을 가로 팬으로 |
//! | Hand·Space 드래그 | pointer 화면 이동만큼 팬 |
//! | Zoom 도구 클릭·Option-클릭 | 커서 기준 다음·이전 줌 단계 |
//! | Zoom 도구 영역 | 그 world 영역을 화면에 맞춤(3pt 미만은 클릭) |
//! | smart magnify | 선택 맞춤(없으면 100%)과 이전 viewport를 번갈아 |

use stellar_commands::view_tabs::CanvasViewport;

use crate::canvas::tool_units::{zoom_in_step, zoom_out_step, DRAG_THRESHOLD_PT};
use crate::input_gesture::{GesturePhase, MomentumPhase, NavigationInput};
use crate::window_session::ProjectWindowSession;

/// 맞춤 여백: 화면 짧은 변의 비율.
pub const FIT_PADDING: f64 = 0.05;
/// ⌘+스크롤 배율: `2^(dy / ZOOM_SCROLL_PT_PER_DOUBLING)`.
pub const ZOOM_SCROLL_PT_PER_DOUBLING: f64 = 100.0;

/// 창별 탐색 상태.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Navigator {
    hand: Option<[f64; 2]>,
    /// 관성 진행 중 새 제스처가 시작됨: 다음 관성 시작 전까지 관성 event를 버립니다.
    momentum_blocked: bool,
    momentum_live: bool,
    smart_previous: Option<CanvasViewport>,
}

fn anchor(location: [f64; 2], size: [f64; 2]) -> (f64, f64) {
    (location[0] - size[0] / 2.0, location[1] - size[1] / 2.0)
}

/// world 상자 `rect`(x, y, w, h)를 화면 `size`에 여백을 두고 맞춘 viewport.
pub fn fit(rect: [f64; 4], size: [f64; 2]) -> Option<CanvasViewport> {
    if !rect.iter().chain(size.iter()).all(|v| v.is_finite()) || size[0] <= 0.0 || size[1] <= 0.0 {
        return None;
    }
    let pad = FIT_PADDING * size[0].min(size[1]);
    let avail = [
        (size[0] - 2.0 * pad).max(1.0),
        (size[1] - 2.0 * pad).max(1.0),
    ];
    let zoom = match (rect[2] > 0.0, rect[3] > 0.0) {
        (true, true) => (avail[0] / rect[2]).min(avail[1] / rect[3]),
        (true, false) => avail[0] / rect[2],
        (false, true) => avail[1] / rect[3],
        (false, false) => 1.0,
    };
    CanvasViewport::new(rect[0] + rect[2] / 2.0, rect[1] + rect[3] / 2.0, zoom).ok()
}

/// 화면 점의 world 좌표.
pub fn screen_to_world(viewport: CanvasViewport, size: [f64; 2], p: [f64; 2]) -> [f64; 2] {
    [
        viewport.center_x() + (p[0] - size[0] / 2.0) / viewport.zoom(),
        viewport.center_y() + (p[1] - size[1] / 2.0) / viewport.zoom(),
    ]
}

impl Navigator {
    /// 새 상태.
    pub fn new() -> Navigator {
        Navigator::default()
    }

    /// 입력 하나를 적용한 viewport. 바뀌지 않으면 `None`입니다. `selection`은 smart magnify가 맞출 선택 world
    /// 경계입니다.
    pub fn apply(
        &mut self,
        viewport: CanvasViewport,
        size: [f64; 2],
        input: NavigationInput,
        selection: Option<[f64; 4]>,
    ) -> Option<CanvasViewport> {
        input.validate().ok()?;
        if !matches!(input, NavigationInput::SmartMagnify { .. }) {
            self.smart_previous = None;
        }
        let next = match input {
            NavigationInput::Magnify {
                magnification,
                location,
                ..
            } => {
                let (ax, ay) = anchor(location, size);
                viewport.zoomed(1.0 + magnification, ax, ay).ok()
            }
            NavigationInput::Scroll {
                delta,
                precise,
                phase,
                momentum,
                modifiers,
                location,
            } => {
                match momentum {
                    MomentumPhase::Began => {
                        self.momentum_blocked = false;
                        self.momentum_live = true;
                    }
                    MomentumPhase::Ended => {
                        self.momentum_live = false;
                        self.momentum_blocked = false;
                        return None;
                    }
                    MomentumPhase::Changed if self.momentum_blocked => return None,
                    _ => {}
                }
                if phase == Some(GesturePhase::Began) && self.momentum_live {
                    self.momentum_blocked = true;
                    self.momentum_live = false;
                }
                if modifiers.command {
                    let (ax, ay) = anchor(location, size);
                    let factor = 2f64.powf(delta[1] / ZOOM_SCROLL_PT_PER_DOUBLING);
                    viewport.zoomed(factor, ax, ay).ok()
                } else if modifiers.shift && !precise {
                    viewport.panned(delta[1], 0.0).ok()
                } else {
                    viewport.panned(delta[0], delta[1]).ok()
                }
            }
            NavigationInput::HandDrag { phase, location } => match phase {
                GesturePhase::Began => {
                    self.hand = Some(location);
                    None
                }
                GesturePhase::Changed => {
                    let last = self.hand.replace(location)?;
                    viewport
                        .panned(location[0] - last[0], location[1] - last[1])
                        .ok()
                }
                GesturePhase::Ended | GesturePhase::Cancelled => {
                    let last = self.hand.take()?;
                    viewport
                        .panned(location[0] - last[0], location[1] - last[1])
                        .ok()
                }
            },
            NavigationInput::ZoomClick { location, zoom_out } => {
                let zoom = viewport.zoom();
                let target = if zoom_out {
                    zoom_out_step(zoom)
                } else {
                    zoom_in_step(zoom)
                };
                let (ax, ay) = anchor(location, size);
                viewport.zoomed(target / zoom, ax, ay).ok()
            }
            NavigationInput::ZoomArea { from, to } => {
                let dragged = (to[0] - from[0]).abs().max((to[1] - from[1]).abs());
                if dragged < f64::from(DRAG_THRESHOLD_PT) {
                    return self.apply(
                        viewport,
                        size,
                        NavigationInput::ZoomClick {
                            location: to,
                            zoom_out: false,
                        },
                        selection,
                    );
                }
                let a = screen_to_world(viewport, size, from);
                let b = screen_to_world(viewport, size, to);
                fit(
                    [
                        a[0].min(b[0]),
                        a[1].min(b[1]),
                        (a[0] - b[0]).abs(),
                        (a[1] - b[1]).abs(),
                    ],
                    size,
                )
            }
            NavigationInput::SmartMagnify { location } => {
                if let Some(previous) = self.smart_previous.take() {
                    Some(previous)
                } else {
                    let target = match selection {
                        Some(rect) => fit(rect, size),
                        None => {
                            let (ax, ay) = anchor(location, size);
                            viewport.zoomed(1.0 / viewport.zoom(), ax, ay).ok()
                        }
                    };
                    if target.is_some() {
                        self.smart_previous = Some(viewport);
                    }
                    target
                }
            }
        };
        next.filter(|n| *n != viewport)
    }

    /// 창의 active 탭 viewport에 입력을 적용합니다. 문서·history는 바뀌지 않습니다. 바뀌면 새 viewport입니다.
    pub fn apply_to(
        &mut self,
        window: &mut ProjectWindowSession,
        size: [f64; 2],
        input: NavigationInput,
        selection: Option<[f64; 4]>,
    ) -> Option<CanvasViewport> {
        let current = window.tabs().active_viewport()?;
        let next = self.apply(current, size, input, selection)?;
        window.set_viewport(next).ok()?;
        Some(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::select_fixtures::save_bytes;
    use crate::canvas::tool::Modifiers;
    use crate::canvas::transform::fixtures::{canvas_document, history_len, open};

    const SIZE: [f64; 2] = [800.0, 600.0];

    fn vp(x: f64, y: f64, z: f64) -> CanvasViewport {
        CanvasViewport::new(x, y, z).unwrap()
    }

    fn scroll(delta: [f64; 2], precise: bool, modifiers: Modifiers) -> NavigationInput {
        NavigationInput::Scroll {
            delta,
            precise,
            phase: precise.then_some(GesturePhase::Changed),
            momentum: MomentumPhase::None,
            modifiers,
            location: [600.0, 300.0],
        }
    }

    fn world_at(v: CanvasViewport, p: [f64; 2]) -> [f64; 2] {
        screen_to_world(v, SIZE, p)
    }

    fn close(a: [f64; 2], b: [f64; 2]) -> bool {
        (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9
    }

    #[test]
    fn pinch_and_command_scroll_zoom_about_the_cursor() {
        // 검증 기준(XII.10.37): 입력별 camera 결과 fixture.
        let mut nav = Navigator::new();
        let start = vp(100.0, 50.0, 1.0);
        let cursor = [600.0, 150.0];
        let fixed = world_at(start, cursor);
        let pinch = NavigationInput::Magnify {
            phase: GesturePhase::Changed,
            magnification: 0.5,
            location: cursor,
        };
        let zoomed = nav.apply(start, SIZE, pinch, None).unwrap();
        assert!((zoomed.zoom() - 1.5).abs() < 1e-12);
        assert!(close(world_at(zoomed, cursor), fixed), "커서 아래 점 고정");
        // ⌘+스크롤 dy 100pt = 2배.
        let command = Modifiers {
            command: true,
            ..Modifiers::NONE
        };
        let doubled = nav
            .apply(start, SIZE, scroll([0.0, 100.0], true, command), None)
            .unwrap();
        assert!((doubled.zoom() - 2.0).abs() < 1e-12);
        assert!(close(
            world_at(doubled, [600.0, 300.0]),
            world_at(start, [600.0, 300.0])
        ));
        // 상한: 25600%에서 더 키우면 그대로(바뀌지 않음).
        let max = vp(0.0, 0.0, 256.0);
        assert_eq!(nav.apply(max, SIZE, pinch, None), None);
    }

    #[test]
    fn scroll_pans_and_shift_wheel_pans_horizontally() {
        let mut nav = Navigator::new();
        let start = vp(0.0, 0.0, 2.0);
        let panned = nav
            .apply(
                start,
                SIZE,
                scroll([20.0, -40.0], true, Modifiers::NONE),
                None,
            )
            .unwrap();
        assert_eq!((panned.center_x(), panned.center_y()), (-10.0, 20.0));
        let shift = Modifiers {
            shift: true,
            ..Modifiers::NONE
        };
        let sideways = nav
            .apply(start, SIZE, scroll([0.0, 30.0], false, shift), None)
            .unwrap();
        assert_eq!((sideways.center_x(), sideways.center_y()), (-15.0, 0.0));
        // 트랙패드 Shift는 그대로 두 축 팬입니다.
        let precise = nav
            .apply(start, SIZE, scroll([0.0, 30.0], true, shift), None)
            .unwrap();
        assert_eq!(precise.center_y(), -15.0);
    }

    #[test]
    fn momentum_keeps_panning_until_a_new_gesture_starts() {
        let mut nav = Navigator::new();
        let v = vp(0.0, 0.0, 1.0);
        let event = |phase, momentum| NavigationInput::Scroll {
            delta: [10.0, 0.0],
            precise: true,
            phase,
            momentum,
            modifiers: Modifiers::NONE,
            location: [0.0, 0.0],
        };
        assert!(nav
            .apply(v, SIZE, event(None, MomentumPhase::Began), None)
            .is_some());
        assert!(nav
            .apply(v, SIZE, event(None, MomentumPhase::Changed), None)
            .is_some());
        // 새 제스처: 그 event는 팬하고 남은 관성 event는 버립니다.
        assert!(nav
            .apply(
                v,
                SIZE,
                event(Some(GesturePhase::Began), MomentumPhase::None),
                None
            )
            .is_some());
        assert_eq!(
            nav.apply(v, SIZE, event(None, MomentumPhase::Changed), None),
            None
        );
        // 다음 관성 시작부터 다시 팬합니다.
        assert!(nav
            .apply(v, SIZE, event(None, MomentumPhase::Began), None)
            .is_some());
    }

    #[test]
    fn hand_drag_zoom_tool_and_smart_magnify() {
        let mut nav = Navigator::new();
        let start = vp(0.0, 0.0, 1.0);
        let hand = |phase, location| NavigationInput::HandDrag { phase, location };
        assert_eq!(
            nav.apply(start, SIZE, hand(GesturePhase::Began, [100.0, 100.0]), None),
            None
        );
        let moved = nav
            .apply(
                start,
                SIZE,
                hand(GesturePhase::Changed, [130.0, 90.0]),
                None,
            )
            .unwrap();
        assert_eq!((moved.center_x(), moved.center_y()), (-30.0, 10.0));
        assert_eq!(
            nav.apply(moved, SIZE, hand(GesturePhase::Ended, [130.0, 90.0]), None),
            None
        );

        // Zoom 도구: 클릭 = 다음 단계, Option-클릭 = 이전 단계, 커서 고정.
        let cursor = [200.0, 500.0];
        let zin = nav
            .apply(
                start,
                SIZE,
                NavigationInput::ZoomClick {
                    location: cursor,
                    zoom_out: false,
                },
                None,
            )
            .unwrap();
        assert_eq!(zin.zoom(), 2.0);
        assert!(close(world_at(zin, cursor), world_at(start, cursor)));
        let zout = nav
            .apply(
                start,
                SIZE,
                NavigationInput::ZoomClick {
                    location: cursor,
                    zoom_out: true,
                },
                None,
            )
            .unwrap();
        assert_eq!(zout.zoom(), 0.5);
        // 영역 드래그: 화면 (400,300)–(600,400) = world (0,0)–(200,100)을 맞춤.
        let area = nav
            .apply(
                start,
                SIZE,
                NavigationInput::ZoomArea {
                    from: [400.0, 300.0],
                    to: [600.0, 400.0],
                },
                None,
            )
            .unwrap();
        assert_eq!((area.center_x(), area.center_y()), (100.0, 50.0));
        assert!((area.zoom() - (800.0 - 60.0) / 200.0).abs() < 1e-12);
        // 3pt 미만 영역은 클릭.
        let tiny = nav
            .apply(
                start,
                SIZE,
                NavigationInput::ZoomArea {
                    from: [10.0, 10.0],
                    to: [11.0, 11.0],
                },
                None,
            )
            .unwrap();
        assert_eq!(tiny.zoom(), 2.0);

        // smart magnify: 선택 맞춤 ↔ 이전 viewport.
        let selection = Some([100.0, 100.0, 50.0, 25.0]);
        let smart = NavigationInput::SmartMagnify {
            location: [0.0, 0.0],
        };
        let fitted = nav.apply(start, SIZE, smart, selection).unwrap();
        assert_eq!((fitted.center_x(), fitted.center_y()), (125.0, 112.5));
        assert_eq!(nav.apply(fitted, SIZE, smart, selection), Some(start));
        // 선택이 없으면 100%와 이전 viewport.
        let far = vp(0.0, 0.0, 0.25);
        let actual = nav.apply(far, SIZE, smart, None).unwrap();
        assert_eq!(actual.zoom(), 1.0);
        assert_eq!(nav.apply(actual, SIZE, smart, None), Some(far));
    }

    #[test]
    fn navigation_never_touches_the_document_or_history() {
        // 검증 기준(XII.10.37): pan·zoom 후 save bytes와 history 불변.
        let (_dir, mut w) = open("navigate-camera", &canvas_document());
        let bytes = save_bytes(w.document().unwrap());
        let history = history_len(&w);
        let revision = w.revision();
        let mut nav = Navigator::new();
        let before = w.tabs().active_viewport().unwrap();
        let inputs = [
            NavigationInput::Magnify {
                phase: GesturePhase::Changed,
                magnification: 0.3,
                location: [10.0, 10.0],
            },
            scroll([5.0, 7.0], true, Modifiers::NONE),
            NavigationInput::ZoomClick {
                location: [0.0, 0.0],
                zoom_out: true,
            },
            NavigationInput::SmartMagnify {
                location: [0.0, 0.0],
            },
        ];
        for input in inputs {
            nav.apply_to(&mut w, SIZE, input, Some([0.0, 0.0, 400.0, 300.0]));
        }
        assert_ne!(w.tabs().active_viewport().unwrap(), before);
        assert_eq!(save_bytes(w.document().unwrap()), bytes);
        assert_eq!((history_len(&w), w.revision()), (history, revision));
    }
}
