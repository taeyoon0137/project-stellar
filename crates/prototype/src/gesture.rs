//! 포인터 제스처 판정과 release velocity (XXX.1.4, ADR-0025 결정 2.10, ADR-0094 결정 7).
//!
//! 한 이벤트 원천 node의 포인터 입력 로그([`PointerSample`])를 EventKind 이벤트로 바꿉니다. 판정 임계값은 논리 tick과
//! px 단위로 이 registry에 고정합니다([`DRAG_THRESHOLD_PX`], [`LONG_PRESS_TICKS`], [`VELOCITY_WINDOW_TICKS`]).
//!
//! - `enter`·`leave` → `pointerEnter`·`pointerLeave`.
//! - `down` → `pointerDown`. 누른 위치에서 한 축이라도 [`DRAG_THRESHOLD_PX`] 이상 움직이면 drag가 시작되고, 그 뒤 움직일
//!   때마다 누적 변위 `dragX`, `dragY`(x 다음 y)를 냅니다.
//! - 누른 채 drag 없이 [`LONG_PRESS_TICKS`]가 지나면 `longPress`를 한 번 냅니다.
//! - `up` → drag 중이면 `dragEndX`, `dragEndY`(release velocity, px/tick) 다음 `pointerUp`. drag·long press가 없었으면
//!   `pointerUp` 다음 `activate`.
//! - `scroll` → `scrollX`, `scrollY`(이 sample의 변위, 0인 축도 냅니다).
//!
//! release velocity는 마지막 [`VELOCITY_WINDOW_TICKS`]개 tick 각각의 변위(그 tick 끝 위치 − 앞 tick 끝 위치)를 f32 덧셈으로
//! 차례대로 더한 뒤 개수로 나눈 평균입니다. drag가 그보다 짧으면 있는 tick 수로 나눕니다. IEEE-754 기본 연산만 쓰므로 같은
//! 입력 로그는 byte 단위로 같은 velocity를 냅니다.

use std::collections::BTreeMap;

use stellar_core_types::typed_value::TypedValue;
use stellar_core_types::value::Length;
use stellar_document::component::event::EventKind;

use crate::clock::Tick;

/// drag 시작 임계 변위(px, 축별).
pub const DRAG_THRESHOLD_PX: f32 = 4.0;
/// long press 판정 시간(tick, 120Hz에서 500ms).
pub const LONG_PRESS_TICKS: Tick = 60;
/// release velocity 평균 창(tick).
pub const VELOCITY_WINDOW_TICKS: usize = 4;

/// 포인터 입력 종류.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PointerInput {
    /// 영역 진입.
    Enter,
    /// 영역 이탈.
    Leave,
    /// 누름(위치).
    Down {
        /// x(px).
        x: f32,
        /// y(px).
        y: f32,
    },
    /// 이동(위치).
    Move {
        /// x(px).
        x: f32,
        /// y(px).
        y: f32,
    },
    /// 뗌(위치).
    Up {
        /// x(px).
        x: f32,
        /// y(px).
        y: f32,
    },
    /// scroll(변위).
    Scroll {
        /// x 변위(px).
        dx: f32,
        /// y 변위(px).
        dy: f32,
    },
}

/// tick이 붙은 포인터 입력.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointerSample {
    /// 논리 tick.
    pub tick: Tick,
    /// 입력.
    pub input: PointerInput,
}

/// 판정된 이벤트.
#[derive(Debug, Clone, PartialEq)]
pub struct GestureEvent {
    /// 논리 tick.
    pub tick: Tick,
    /// 이벤트 종류.
    pub kind: EventKind,
    /// payload(종류가 정한 타입).
    pub payload: Option<TypedValue>,
}

fn length(v: f32) -> TypedValue {
    // 입력 좌표가 유한하면 변위도 유한합니다. 비유한 입력은 0으로 둡니다.
    TypedValue::Length(Length::new(v).unwrap_or_else(|_| Length::new(0.0).expect("0")))
}

#[derive(Debug, Clone)]
struct Press {
    origin: (f32, f32),
    down_tick: Tick,
    dragging: bool,
    long_pressed: bool,
    /// tick 끝 위치(입력이 있었던 tick만). 위치는 그 tick 이하의 가장 늦은 기록입니다.
    ends: BTreeMap<Tick, (f32, f32)>,
}

impl Press {
    fn position_at(&self, tick: Tick) -> (f32, f32) {
        self.ends
            .range(..=tick)
            .next_back()
            .map(|(_, p)| *p)
            .unwrap_or(self.origin)
    }

    fn record(&mut self, tick: Tick, position: (f32, f32)) {
        self.ends.insert(tick, position);
        // 창 계산에 필요 없는 오래된 기록을 지웁니다(창 시작 직전 위치 하나는 남깁니다).
        let horizon = tick.saturating_sub(VELOCITY_WINDOW_TICKS as Tick);
        if let Some(keep) = self.ends.range(..=horizon).next_back().map(|(t, _)| *t) {
            self.ends = self.ends.split_off(&keep);
        }
    }

    /// `tick`에서 끝난 drag의 release velocity(px/tick).
    fn velocity(&self, tick: Tick) -> (f32, f32) {
        let first = tick
            .saturating_sub(VELOCITY_WINDOW_TICKS as Tick - 1)
            .max(self.down_tick + 1);
        let mut sx = 0.0f32;
        let mut sy = 0.0f32;
        let mut count = 0u32;
        let mut k = first;
        while k <= tick {
            let (x1, y1) = self.position_at(k);
            let (x0, y0) = self.position_at(k - 1);
            sx += x1 - x0;
            sy += y1 - y0;
            count += 1;
            k += 1;
        }
        if count == 0 {
            return (0.0, 0.0);
        }
        (sx / count as f32, sy / count as f32)
    }
}

/// 한 이벤트 원천의 증분 제스처 판정기.
#[derive(Debug, Clone, Default)]
pub struct GestureRecognizer {
    press: Option<Press>,
    last_tick: Option<Tick>,
}

/// 입력 tick이 앞 입력보다 작습니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutOfOrder {
    /// 앞 tick.
    pub previous: Tick,
    /// 받은 tick.
    pub got: Tick,
}

fn event(tick: Tick, kind: EventKind, payload: Option<TypedValue>) -> GestureEvent {
    GestureEvent {
        tick,
        kind,
        payload,
    }
}

impl GestureRecognizer {
    /// 새 판정기.
    pub fn new() -> GestureRecognizer {
        GestureRecognizer::default()
    }

    /// `tick`까지 지난 시간으로 판정할 일(long press)을 확정합니다.
    fn settle(&mut self, tick: Tick, out: &mut Vec<GestureEvent>) {
        if let Some(press) = self.press.as_mut() {
            let due = press.down_tick.saturating_add(LONG_PRESS_TICKS);
            if !press.dragging && !press.long_pressed && tick >= due {
                press.long_pressed = true;
                out.push(event(due, EventKind::LongPress, None));
            }
        }
    }

    /// 입력 하나를 넣습니다. tick은 앞 입력보다 작으면 안 됩니다.
    pub fn push(&mut self, sample: PointerSample) -> Result<Vec<GestureEvent>, OutOfOrder> {
        if let Some(previous) = self.last_tick {
            if sample.tick < previous {
                return Err(OutOfOrder {
                    previous,
                    got: sample.tick,
                });
            }
        }
        self.last_tick = Some(sample.tick);
        let t = sample.tick;
        let mut out = Vec::new();
        self.settle(t, &mut out);
        match sample.input {
            PointerInput::Enter => out.push(event(t, EventKind::PointerEnter, None)),
            PointerInput::Leave => out.push(event(t, EventKind::PointerLeave, None)),
            PointerInput::Down { x, y } => {
                let mut press = Press {
                    origin: (x, y),
                    down_tick: t,
                    dragging: false,
                    long_pressed: false,
                    ends: BTreeMap::new(),
                };
                press.record(t, (x, y));
                self.press = Some(press);
                out.push(event(t, EventKind::PointerDown, None));
            }
            PointerInput::Move { x, y } => {
                if let Some(press) = self.press.as_mut() {
                    press.record(t, (x, y));
                    let (dx, dy) = (x - press.origin.0, y - press.origin.1);
                    if !press.dragging
                        && !press.long_pressed
                        && (dx.abs() >= DRAG_THRESHOLD_PX || dy.abs() >= DRAG_THRESHOLD_PX)
                    {
                        press.dragging = true;
                    }
                    if press.dragging {
                        out.push(event(t, EventKind::DragX, Some(length(dx))));
                        out.push(event(t, EventKind::DragY, Some(length(dy))));
                    }
                }
            }
            PointerInput::Up { x, y } => {
                if let Some(mut press) = self.press.take() {
                    press.record(t, (x, y));
                    if press.dragging {
                        let (vx, vy) = press.velocity(t);
                        out.push(event(t, EventKind::DragEndX, Some(length(vx))));
                        out.push(event(t, EventKind::DragEndY, Some(length(vy))));
                        out.push(event(t, EventKind::PointerUp, None));
                    } else {
                        out.push(event(t, EventKind::PointerUp, None));
                        if !press.long_pressed {
                            out.push(event(t, EventKind::Activate, None));
                        }
                    }
                }
            }
            PointerInput::Scroll { dx, dy } => {
                out.push(event(t, EventKind::ScrollX, Some(length(dx))));
                out.push(event(t, EventKind::ScrollY, Some(length(dy))));
            }
        }
        Ok(out)
    }

    /// tick `tick`까지 입력이 끝났다고 알립니다(long press 판정).
    pub fn finish_tick(&mut self, tick: Tick) -> Vec<GestureEvent> {
        let mut out = Vec::new();
        self.settle(tick, &mut out);
        out
    }
}

/// 입력 로그 전체를 `last_tick`까지 판정합니다(순수 함수).
pub fn recognize(log: &[PointerSample], last_tick: Tick) -> Result<Vec<GestureEvent>, OutOfOrder> {
    let mut recognizer = GestureRecognizer::new();
    let mut out = Vec::new();
    for sample in log {
        out.extend(recognizer.push(*sample)?);
    }
    out.extend(recognizer.finish_tick(last_tick));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(tick: Tick, input: PointerInput) -> PointerSample {
        PointerSample { tick, input }
    }

    fn kinds(events: &[GestureEvent]) -> Vec<EventKind> {
        events.iter().map(|e| e.kind).collect()
    }

    fn len(e: &GestureEvent) -> f32 {
        match &e.payload {
            Some(TypedValue::Length(l)) => l.get(),
            other => panic!("길이 아님: {other:?}"),
        }
    }

    #[test]
    fn tap_activates() {
        let log = [
            s(0, PointerInput::Enter),
            s(1, PointerInput::Down { x: 10.0, y: 10.0 }),
            s(3, PointerInput::Move { x: 11.0, y: 10.0 }),
            s(5, PointerInput::Up { x: 11.0, y: 10.0 }),
            s(6, PointerInput::Leave),
        ];
        let events = recognize(&log, 10).expect("판정");
        assert_eq!(
            kinds(&events),
            vec![
                EventKind::PointerEnter,
                EventKind::PointerDown,
                EventKind::PointerUp,
                EventKind::Activate,
                EventKind::PointerLeave
            ]
        );
    }

    #[test]
    fn drag_emits_axis_events_x_then_y_and_release_velocity() {
        let log = [
            s(0, PointerInput::Down { x: 0.0, y: 0.0 }),
            s(1, PointerInput::Move { x: 5.0, y: 1.0 }),
            s(2, PointerInput::Move { x: 15.0, y: 2.0 }),
            s(3, PointerInput::Move { x: 30.0, y: 4.0 }),
            s(4, PointerInput::Move { x: 50.0, y: 4.0 }),
            s(5, PointerInput::Up { x: 70.0, y: 6.0 }),
        ];
        let events = recognize(&log, 5).expect("판정");
        assert_eq!(events[0].kind, EventKind::PointerDown);
        assert_eq!(
            kinds(&events[1..3]),
            vec![EventKind::DragX, EventKind::DragY]
        );
        assert_eq!(len(&events[1]), 5.0);
        assert_eq!(len(&events[2]), 1.0);
        let end: Vec<&GestureEvent> = events
            .iter()
            .filter(|e| matches!(e.kind, EventKind::DragEndX | EventKind::DragEndY))
            .collect();
        assert_eq!(end.len(), 2);
        assert_eq!(end[0].kind, EventKind::DragEndX);
        // 마지막 4 tick 변위: x 10, 15, 20, 20 → 16.25, y 1, 2, 0, 2 → 1.25.
        assert_eq!(len(end[0]), 16.25);
        assert_eq!(len(end[1]), 1.25);
        assert_eq!(events.last().map(|e| e.kind), Some(EventKind::PointerUp));
        assert!(!kinds(&events).contains(&EventKind::Activate));
    }

    #[test]
    fn same_log_same_velocity_bytes() {
        // 검증 기준(XXX.1.4): 같은 입력 로그의 velocity 값 byte-identical.
        let log: Vec<PointerSample> = std::iter::once(s(0, PointerInput::Down { x: 0.1, y: 0.2 }))
            .chain((1..40).map(|t| {
                s(
                    t,
                    PointerInput::Move {
                        x: 0.1 + t as f32 * 1.37,
                        y: 0.2 - t as f32 * 0.73,
                    },
                )
            }))
            .chain(std::iter::once(s(
                41,
                PointerInput::Up { x: 60.3, y: -30.1 },
            )))
            .collect();
        let a = recognize(&log, 41).expect("판정");
        let b = recognize(&log, 41).expect("판정");
        let bits = |events: &[GestureEvent]| -> Vec<u32> {
            events
                .iter()
                .filter(|e| matches!(e.kind, EventKind::DragEndX | EventKind::DragEndY))
                .map(|e| len(e).to_bits())
                .collect()
        };
        assert_eq!(bits(&a), bits(&b));
        assert_eq!(bits(&a).len(), 2);
    }

    #[test]
    fn long_press_fires_once_without_activate() {
        let log = [
            s(0, PointerInput::Down { x: 0.0, y: 0.0 }),
            s(100, PointerInput::Up { x: 0.0, y: 0.0 }),
        ];
        let events = recognize(&log, 100).expect("판정");
        assert_eq!(
            kinds(&events),
            vec![
                EventKind::PointerDown,
                EventKind::LongPress,
                EventKind::PointerUp
            ]
        );
        assert_eq!(events[1].tick, LONG_PRESS_TICKS);
    }

    #[test]
    fn scroll_and_order_check() {
        let mut r = GestureRecognizer::new();
        let e = r
            .push(s(5, PointerInput::Scroll { dx: 0.0, dy: -3.0 }))
            .expect("판정");
        assert_eq!(kinds(&e), vec![EventKind::ScrollX, EventKind::ScrollY]);
        assert_eq!(len(&e[1]), -3.0);
        assert_eq!(
            r.push(s(4, PointerInput::Enter)),
            Err(OutOfOrder {
                previous: 5,
                got: 4
            })
        );
    }
}
