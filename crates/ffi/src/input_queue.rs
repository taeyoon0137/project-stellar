//! 셸→코어 입력 대기열과 연속 이벤트 coalescing (XII.3.4).
//!
//! 셸이 제출한 [`InputEvent`]를 FIFO로 보관합니다. 소비자가 아직 꺼내지 않은 **마지막** 이벤트와
//! 새 이벤트가 같은 연속 동작이면 하나로 합칩니다. 합치는 대상은 대기열 끝 하나뿐이므로
//! pointer down/up·cancel, key, text composition, focus, resize 이벤트의 상대 순서는 바뀌지 않습니다
//! (이들은 병합하지 않고, 앞뒤 move/wheel의 병합을 끊는 경계가 됩니다).
//!
//! # 병합 규칙
//!
//! - **pointer move**: phase가 둘 다 `Move`이고 button·device·modifier가 같으면 최신 이벤트(위치·시각)만
//!   남깁니다. 중간 위치는 버립니다.
//! - **wheel**: phase가 같고 그 phase가 `Changed`·`Momentum`·`Discrete`이며 단위·modifier가 같으면 delta를
//!   더하고 위치·시각은 최신 값을 씁니다(scroll 거리 보존). gesture 경계인 `Began`·`Ended`는 병합하지
//!   않습니다. 합이 `f32` 범위를 넘으면 병합하지 않고 따로 넣습니다.
//! - 그 밖의 조합은 병합하지 않습니다.
//!
//! # 수신 기록 (XII.3.5)
//!
//! 항목마다 T0 후보([`InputReceipt`]: 운영체제 이벤트 시각, 코어 수신 시각)를 함께 보관합니다. 병합하면
//! 이벤트 값은 최신으로 바뀌지만 receipt는 묶음에서 **가장 이른** 값을 유지하고 병합 수를 셉니다
//! ([`InputReceipt::absorb`]). 아직 반영되지 않은 가장 오래된 입력 기준으로 지연을 재기 위해서입니다.

use std::collections::VecDeque;

use stellar_core_types::value::Vec2;
use stellar_runtime::input_trace::InputReceipt;

use crate::input::{InputEvent, InputEventKind, PointerPhase, WheelPhase};

/// [`InputQueue::push`] 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushOutcome {
    /// 새 항목으로 넣음.
    Queued,
    /// 대기열 끝 항목에 병합함(항목 수 변화 없음).
    Coalesced,
    /// 용량이 가득 차 버림.
    Full,
}

/// 대기열 항목: 이벤트와 그 수신 기록.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Queued {
    event: InputEvent,
    receipt: InputReceipt,
}

/// 용량 고정 FIFO 입력 대기열.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputQueue {
    events: VecDeque<Queued>,
    capacity: usize,
    coalesced: u64,
}

impl InputQueue {
    /// `capacity`개까지 보관하는 빈 대기열을 만듭니다.
    pub fn new(capacity: usize) -> InputQueue {
        InputQueue {
            events: VecDeque::new(),
            capacity,
            coalesced: 0,
        }
    }

    /// 이벤트를 넣습니다. 수신 시각을 따로 재지 않은 호출자(재생·테스트)용이며, 이벤트 시각을 수신
    /// 시각으로 씁니다. 셸 경계는 [`InputQueue::push_received`]를 씁니다.
    pub fn push(&mut self, event: InputEvent) -> PushOutcome {
        let time = event.timestamp().get();
        self.push_received(event, InputReceipt::new(time, time))
    }

    /// 수신 기록과 함께 이벤트를 넣습니다. 끝 항목과 병합할 수 있으면 병합하고(receipt는 이른 값 유지),
    /// 아니면 용량 안에서 새 항목으로 넣습니다.
    pub fn push_received(&mut self, event: InputEvent, receipt: InputReceipt) -> PushOutcome {
        if let Some(last) = self.events.back_mut() {
            if let Some(merged) = coalesce(&last.event, &event) {
                last.event = merged;
                last.receipt = last.receipt.absorb(receipt);
                self.coalesced = self.coalesced.saturating_add(1);
                return PushOutcome::Coalesced;
            }
        }
        if self.events.len() >= self.capacity {
            return PushOutcome::Full;
        }
        self.events.push_back(Queued { event, receipt });
        PushOutcome::Queued
    }

    /// 가장 오래된 이벤트.
    pub fn front(&self) -> Option<&InputEvent> {
        self.events.front().map(|queued| &queued.event)
    }

    /// 가장 오래된 이벤트의 수신 기록.
    pub fn front_receipt(&self) -> Option<InputReceipt> {
        self.events.front().map(|queued| queued.receipt)
    }

    /// 가장 오래된 이벤트를 꺼냅니다.
    pub fn pop(&mut self) -> Option<InputEvent> {
        self.pop_received().map(|(event, _)| event)
    }

    /// 가장 오래된 이벤트를 수신 기록과 함께 꺼냅니다.
    pub fn pop_received(&mut self) -> Option<(InputEvent, InputReceipt)> {
        self.events
            .pop_front()
            .map(|queued| (queued.event, queued.receipt))
    }

    /// 대기 중 이벤트 수.
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// 대기 중 이벤트가 없는지.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// 지금까지 병합된 이벤트 수(진단용).
    pub fn coalesced_count(&self) -> u64 {
        self.coalesced
    }
}

/// `last` 뒤에 온 `next`를 `last`에 병합한 결과. 병합할 수 없으면 `None`.
fn coalesce(last: &InputEvent, next: &InputEvent) -> Option<InputEvent> {
    match (last.kind(), next.kind()) {
        (InputEventKind::Pointer(a), InputEventKind::Pointer(b)) => {
            let mergeable = a.phase == PointerPhase::Move
                && b.phase == PointerPhase::Move
                && a.button == b.button
                && a.device == b.device
                && a.modifiers == b.modifiers;
            mergeable.then(|| next.clone())
        }
        (InputEventKind::Wheel(a), InputEventKind::Wheel(b)) => {
            let continuous = matches!(
                a.phase,
                WheelPhase::Changed | WheelPhase::Momentum | WheelPhase::Discrete
            );
            if !(continuous && a.phase == b.phase && a.unit == b.unit && a.modifiers == b.modifiers)
            {
                return None;
            }
            // f32 합이 범위를 넘으면(비유한) 병합하지 않습니다.
            let delta = Vec2::new(a.delta.x() + b.delta.x(), a.delta.y() + b.delta.y()).ok()?;
            let mut merged = *b;
            merged.delta = delta;
            InputEvent::new(next.timestamp(), InputEventKind::Wheel(merged)).ok()
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::{
        CompositionEvent, CompositionPhase, FocusEvent, KeyCode, KeyEvent, KeyState, Modifiers,
        MonotonicNanos, PointerButton, PointerDevice, PointerEvent, TextRange, WheelDeltaUnit,
        WheelEvent,
    };

    fn v(x: f32, y: f32) -> Vec2 {
        Vec2::new(x, y).expect("finite")
    }

    fn pointer(t: u64, phase: PointerPhase, button: PointerButton, at: Vec2) -> InputEvent {
        pointer_with(t, phase, button, at, Modifiers::NONE)
    }

    fn pointer_with(
        t: u64,
        phase: PointerPhase,
        button: PointerButton,
        at: Vec2,
        modifiers: Modifiers,
    ) -> InputEvent {
        let click_count = match phase {
            PointerPhase::Down | PointerPhase::Up => 1,
            PointerPhase::Move | PointerPhase::Cancel => 0,
        };
        InputEvent::new(
            MonotonicNanos::new(t),
            InputEventKind::Pointer(PointerEvent {
                phase,
                button,
                device: PointerDevice::Mouse,
                position: at,
                modifiers,
                click_count,
            }),
        )
        .expect("pointer")
    }

    fn key(t: u64, state: KeyState) -> InputEvent {
        InputEvent::new(
            MonotonicNanos::new(t),
            InputEventKind::Key(KeyEvent {
                state,
                key: KeyCode::new(0x04).expect("hid"),
                repeat: false,
                modifiers: Modifiers::NONE,
            }),
        )
        .expect("key")
    }

    fn wheel(t: u64, phase: WheelPhase, unit: WheelDeltaUnit, delta: Vec2) -> InputEvent {
        InputEvent::new(
            MonotonicNanos::new(t),
            InputEventKind::Wheel(WheelEvent {
                phase,
                unit,
                position: v(t as f32, 0.0),
                delta,
                modifiers: Modifiers::NONE,
            }),
        )
        .expect("wheel")
    }

    fn drain(queue: &mut InputQueue) -> Vec<InputEvent> {
        std::iter::from_fn(|| queue.pop()).collect()
    }

    fn pointer_of(event: &InputEvent) -> PointerEvent {
        match event.kind() {
            InputEventKind::Pointer(pointer) => *pointer,
            other => panic!("pointer가 아님: {other:?}"),
        }
    }

    fn wheel_of(event: &InputEvent) -> WheelEvent {
        match event.kind() {
            InputEventKind::Wheel(wheel) => *wheel,
            other => panic!("wheel이 아님: {other:?}"),
        }
    }

    #[test]
    fn ten_thousand_moves_collapse_to_latest_and_keep_down_up_order() {
        // down → 10,000 drag move → up → key down → 10,000 hover move → key up.
        fn next(t: &mut u64) -> u64 {
            *t += 1;
            *t
        }
        let mut queue = InputQueue::new(16);
        let mut t = 0u64;
        assert_eq!(
            queue.push(pointer(
                next(&mut t),
                PointerPhase::Down,
                PointerButton::Primary,
                v(0.0, 0.0)
            )),
            PushOutcome::Queued
        );
        for i in 0..10_000 {
            let outcome = queue.push(pointer(
                next(&mut t),
                PointerPhase::Move,
                PointerButton::Primary,
                v(i as f32, (i * 2) as f32),
            ));
            let expected = if i == 0 {
                PushOutcome::Queued
            } else {
                PushOutcome::Coalesced
            };
            assert_eq!(outcome, expected);
        }
        let drag_last_t = t;
        queue.push(pointer(
            next(&mut t),
            PointerPhase::Up,
            PointerButton::Primary,
            v(9_999.0, 19_998.0),
        ));
        queue.push(key(next(&mut t), KeyState::Down));
        for i in 0..10_000 {
            queue.push(pointer(
                next(&mut t),
                PointerPhase::Move,
                PointerButton::None,
                v(-(i as f32), 1.0),
            ));
        }
        let hover_last_t = t;
        queue.push(key(next(&mut t), KeyState::Up));
        assert_eq!(queue.len(), 6);
        assert_eq!(queue.coalesced_count(), 2 * 9_999);

        let events = drain(&mut queue);
        let phases: Vec<&str> = events
            .iter()
            .map(|event| match event.kind() {
                InputEventKind::Pointer(p) => match p.phase {
                    PointerPhase::Down => "down",
                    PointerPhase::Up => "up",
                    PointerPhase::Move => "move",
                    PointerPhase::Cancel => "cancel",
                },
                InputEventKind::Key(k) if k.state == KeyState::Down => "key-down",
                InputEventKind::Key(_) => "key-up",
                _ => "other",
            })
            .collect();
        assert_eq!(phases, ["down", "move", "up", "key-down", "move", "key-up"]);
        // 남은 move는 각 연속 구간의 최신 1개(위치·시각)입니다.
        assert_eq!(pointer_of(&events[1]).position, v(9_999.0, 19_998.0));
        assert_eq!(events[1].timestamp().get(), drag_last_t);
        assert_eq!(pointer_of(&events[4]).position, v(-9_999.0, 1.0));
        assert_eq!(events[4].timestamp().get(), hover_last_t);
        // down/up·key 이벤트는 그대로입니다.
        assert_eq!(pointer_of(&events[0]).position, v(0.0, 0.0));
        assert_eq!(events[0].timestamp().get(), 1);
        // 시각 순서도 보존됩니다.
        assert!(events
            .windows(2)
            .all(|pair| pair[0].timestamp() < pair[1].timestamp()));
    }

    #[test]
    fn barriers_and_attribute_changes_stop_move_coalescing() {
        let mut queue = InputQueue::new(32);
        let at = v(1.0, 1.0);
        // key 사이에 낀 move는 병합되지 않습니다(순서 보존).
        queue.push(pointer(1, PointerPhase::Move, PointerButton::None, at));
        queue.push(key(2, KeyState::Down));
        queue.push(pointer(3, PointerPhase::Move, PointerButton::None, at));
        assert_eq!(queue.len(), 3);
        // hover move 뒤 drag move(button 다름), modifier 변화, cancel은 병합하지 않습니다.
        queue.push(pointer(4, PointerPhase::Move, PointerButton::Primary, at));
        queue.push(pointer_with(
            5,
            PointerPhase::Move,
            PointerButton::Primary,
            at,
            Modifiers::SHIFT,
        ));
        queue.push(pointer(6, PointerPhase::Cancel, PointerButton::Primary, at));
        queue.push(pointer(7, PointerPhase::Cancel, PointerButton::Primary, at));
        assert_eq!(queue.len(), 7);
        // down/down, focus, composition은 병합하지 않습니다.
        queue.push(pointer(8, PointerPhase::Down, PointerButton::Primary, at));
        queue.push(pointer(9, PointerPhase::Down, PointerButton::Primary, at));
        for t in [10, 11] {
            queue.push(
                InputEvent::new(
                    MonotonicNanos::new(t),
                    InputEventKind::Focus(FocusEvent::Gained),
                )
                .expect("focus"),
            );
        }
        for t in [12, 13] {
            queue.push(
                InputEvent::new(
                    MonotonicNanos::new(t),
                    InputEventKind::TextComposition(CompositionEvent {
                        phase: CompositionPhase::Update,
                        text: "한".to_string(),
                        selection: TextRange::EMPTY,
                    }),
                )
                .expect("composition"),
            );
        }
        assert_eq!(queue.len(), 13);
        assert_eq!(queue.coalesced_count(), 0);
        let order: Vec<u64> = drain(&mut queue)
            .iter()
            .map(|event| event.timestamp().get())
            .collect();
        assert_eq!(order, (1..=13).collect::<Vec<u64>>());
    }

    #[test]
    fn wheel_deltas_accumulate_within_phase_and_keep_gesture_boundaries() {
        let mut queue = InputQueue::new(16);
        let unit = WheelDeltaUnit::Point;
        queue.push(wheel(1, WheelPhase::Began, unit, v(0.0, 1.0)));
        for t in 2..=101 {
            queue.push(wheel(t, WheelPhase::Changed, unit, v(0.5, 2.0)));
        }
        queue.push(wheel(102, WheelPhase::Ended, unit, v(0.0, 0.0)));
        for t in 103..=112 {
            queue.push(wheel(t, WheelPhase::Momentum, unit, v(0.0, 0.25)));
        }
        // line 단위 discrete는 point와 섞지 않습니다.
        queue.push(wheel(
            113,
            WheelPhase::Discrete,
            WheelDeltaUnit::Line,
            v(0.0, 1.0),
        ));
        queue.push(wheel(
            114,
            WheelPhase::Discrete,
            WheelDeltaUnit::Line,
            v(0.0, 1.0),
        ));
        queue.push(wheel(115, WheelPhase::Discrete, unit, v(0.0, 1.0)));
        let events = drain(&mut queue);
        let summary: Vec<(WheelPhase, WheelDeltaUnit, Vec2, u64)> = events
            .iter()
            .map(|event| {
                let w = wheel_of(event);
                (w.phase, w.unit, w.delta, event.timestamp().get())
            })
            .collect();
        assert_eq!(
            summary,
            vec![
                (WheelPhase::Began, unit, v(0.0, 1.0), 1),
                // 100개 Changed: delta 합(0.5×100, 2×100), 최신 시각.
                (WheelPhase::Changed, unit, v(50.0, 200.0), 101),
                (WheelPhase::Ended, unit, v(0.0, 0.0), 102),
                (WheelPhase::Momentum, unit, v(0.0, 2.5), 112),
                (WheelPhase::Discrete, WheelDeltaUnit::Line, v(0.0, 2.0), 114),
                (WheelPhase::Discrete, unit, v(0.0, 1.0), 115),
            ]
        );
        // 위치는 최신 이벤트 값입니다.
        assert_eq!(wheel_of(&events[1]).position, v(101.0, 0.0));
        // 두 Began은 병합하지 않습니다.
        let mut began = InputQueue::new(4);
        began.push(wheel(1, WheelPhase::Began, unit, v(0.0, 1.0)));
        began.push(wheel(2, WheelPhase::Began, unit, v(0.0, 1.0)));
        assert_eq!(began.len(), 2);
    }

    #[test]
    fn wheel_sum_overflow_is_not_merged() {
        let mut queue = InputQueue::new(4);
        let unit = WheelDeltaUnit::Point;
        queue.push(wheel(1, WheelPhase::Changed, unit, v(0.0, f32::MAX)));
        assert_eq!(
            queue.push(wheel(2, WheelPhase::Changed, unit, v(0.0, f32::MAX))),
            PushOutcome::Queued
        );
        assert_eq!(queue.len(), 2);
    }

    #[test]
    fn capacity_counts_entries_not_merged_events() {
        let mut queue = InputQueue::new(2);
        assert_eq!(queue.push(key(1, KeyState::Down)), PushOutcome::Queued);
        assert_eq!(
            queue.push(pointer(
                2,
                PointerPhase::Move,
                PointerButton::None,
                v(0.0, 0.0)
            )),
            PushOutcome::Queued
        );
        // 가득 찬 상태에서도 끝 항목과 병합되는 move는 받습니다.
        for t in 3..1_000 {
            assert_eq!(
                queue.push(pointer(
                    t,
                    PointerPhase::Move,
                    PointerButton::None,
                    v(1.0, 1.0)
                )),
                PushOutcome::Coalesced
            );
        }
        // 병합할 수 없는 이벤트는 버립니다.
        assert_eq!(queue.push(key(1_000, KeyState::Up)), PushOutcome::Full);
        assert_eq!(queue.len(), 2);
        assert!(!queue.is_empty());
        assert_eq!(queue.front().map(|event| event.timestamp().get()), Some(1));
        // 꺼낸 뒤의 move는 이미 꺼낸 이벤트와 병합하지 않습니다.
        queue.pop();
        queue.pop();
        assert!(queue.is_empty());
        assert_eq!(
            queue.push(pointer(
                1_001,
                PointerPhase::Move,
                PointerButton::None,
                v(2.0, 2.0)
            )),
            PushOutcome::Queued
        );
    }

    #[test]
    fn merged_entries_keep_the_earliest_receipt() {
        // XII.3.5: 병합 항목의 T0 후보는 묶음에서 가장 이른 값이고, 병합 수를 셉니다.
        let mut queue = InputQueue::new(8);
        let receipt = |t: u64| InputReceipt::new(t, t + 1_000);
        queue.push_received(
            pointer(1, PointerPhase::Down, PointerButton::Primary, v(0.0, 0.0)),
            receipt(1),
        );
        for t in 2..=101 {
            queue.push_received(
                pointer(
                    t,
                    PointerPhase::Move,
                    PointerButton::Primary,
                    v(t as f32, 0.0),
                ),
                receipt(t),
            );
        }
        queue.push_received(
            pointer(102, PointerPhase::Up, PointerButton::Primary, v(101.0, 0.0)),
            receipt(102),
        );
        assert_eq!(queue.len(), 3);
        assert_eq!(queue.front_receipt(), Some(receipt(1)));

        let (down, down_receipt) = queue.pop_received().expect("down");
        assert_eq!(pointer_of(&down).phase, PointerPhase::Down);
        assert_eq!(down_receipt, receipt(1));
        let (moved, move_receipt) = queue.pop_received().expect("move");
        // 이벤트는 최신 값, receipt는 첫 move 값과 병합 수 99.
        assert_eq!(pointer_of(&moved).position, v(101.0, 0.0));
        assert_eq!(moved.timestamp().get(), 101);
        assert_eq!(
            (
                move_receipt.event_time(),
                move_receipt.received(),
                move_receipt.merged()
            ),
            (2, 1_002, 99)
        );
        let (_, up_receipt) = queue.pop_received().expect("up");
        assert_eq!(up_receipt, receipt(102));
        assert!(queue.pop_received().is_none());
        assert_eq!(queue.front_receipt(), None);

        // 수신 시각 없이 넣으면 이벤트 시각을 수신 시각으로 씁니다.
        queue.push(key(7, KeyState::Down));
        assert_eq!(queue.front_receipt(), Some(InputReceipt::new(7, 7)));
    }
}
