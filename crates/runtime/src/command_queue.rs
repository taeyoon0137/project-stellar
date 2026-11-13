//! UI→DocumentThread bounded command queue와 입력 우선순위 (XII.1.2).
//!
//! document thread 수신함을 우선순위별 bounded lane으로 나눕니다. [`CommandQueue`]는 항목 타입에
//! 대해 generic한 순수 자료구조이고, document thread는 이를 job 수신함으로 씁니다
//! ([`crate::document_thread`]).
//!
//! # 우선순위
//!
//! | 우선순위 | 대상 | 처리 순서 |
//! |---|---|---|
//! | [`CommandPriority::Input`] | 사용자 입력 이벤트(pointer, wheel, key, text composition, focus, resize) | 1 |
//! | [`CommandPriority::Command`] | 일반 편집 command(메뉴, 단축키, inspector 결과 등) | 2 |
//! | [`CommandPriority::Background`] | 백그라운드 요청(autosave trigger, 캐시·진단 갱신 등) | 3 |
//!
//! 꺼낼 때는 항상 가장 높은 우선순위 lane의 가장 오래된 항목을 먼저 꺼냅니다. 같은 lane 안은
//! FIFO이고, lane 사이의 접수 순서는 보존하지 않습니다. 순서가 중요한 두 job은 같은 우선순위로
//! 보냅니다.
//!
//! # overflow 정책
//!
//! lane마다 독립 용량([`QueueCapacity`])을 가지므로 낮은 우선순위 폭주가 입력 용량을 잠식하지
//! 않습니다. 접수([`CommandQueue::try_push`])는 아래 순서로 판정합니다.
//!
//! 1. 닫힌 queue는 [`RejectReason::Closed`]로 거부합니다.
//! 2. **병합**: 항목에 [`CoalesceKey`]가 있고 같은 lane의 **마지막** 항목이 같은 key면, 그 항목을
//!    새 항목으로 대체합니다([`Pushed::Coalesced`]). 용량을 쓰지 않습니다. 연속된 같은 key만
//!    병합하므로 사이에 낀 다른 이벤트와의 순서가 바뀌지 않습니다.
//! 3. 용량이 남으면 lane 끝에 넣습니다([`Pushed::Enqueued`]).
//! 4. 용량이 찼을 때:
//!    - `Input`·`Command`: [`RejectReason::QueueFull`]로 바로 거부합니다. 이미 접수된 항목을
//!      버리지 않고, 자리가 날 때까지 기다리지 않습니다.
//!    - `Background`: lane에서 가장 오래된 항목을 밀어내고 새 항목을 넣습니다
//!      ([`Pushed::EnqueuedEvicting`]).
//!
//! 거부·대체·밀려난 항목은 호출자에게 돌려주므로, 호출자는 lock 밖에서 정리합니다.
//!
//! # 대기
//!
//! 접수·닫기·`try_pop`은 O(1) 임계 구역만 잡고 기다리지 않습니다. 기다리는 연산은 소비자
//! (document thread)의 [`CommandQueue::pop_blocking`] 하나뿐입니다.

use std::collections::VecDeque;
use std::fmt;
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};

/// 명령 우선순위. 선언 순서가 처리 순서입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CommandPriority {
    /// 사용자 입력 이벤트. 가장 먼저 처리합니다.
    Input,
    /// 일반 편집 command.
    Command,
    /// 백그라운드 요청. 용량이 차면 가장 오래된 요청을 밀어냅니다.
    Background,
}

impl CommandPriority {
    /// 처리 순서대로 나열한 전체 우선순위.
    pub const ALL: [CommandPriority; 3] = [
        CommandPriority::Input,
        CommandPriority::Command,
        CommandPriority::Background,
    ];

    /// 고정 이름(fixture·진단용).
    pub const fn name(self) -> &'static str {
        match self {
            CommandPriority::Input => "input",
            CommandPriority::Command => "command",
            CommandPriority::Background => "background",
        }
    }

    /// 이름에서 복원합니다.
    pub fn from_name(name: &str) -> Option<CommandPriority> {
        CommandPriority::ALL
            .into_iter()
            .find(|priority| priority.name() == name)
    }

    const fn lane(self) -> usize {
        match self {
            CommandPriority::Input => 0,
            CommandPriority::Command => 1,
            CommandPriority::Background => 2,
        }
    }
}

/// 연속 항목 병합 key. 같은 lane의 마지막 항목과 key가 같으면 새 항목이 대체합니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CoalesceKey {
    /// pointer move. `device`는 셸 경계의 pointer device tag입니다.
    PointerMove {
        /// pointer device tag.
        device: u8,
    },
    /// 캔버스 크기·backing scale 변화.
    Resize,
    /// 호출자가 정의한 key(예: autosave trigger, 같은 대상의 캐시 갱신 요청).
    Custom(u64),
}

/// 입력 이벤트의 우선순위 분류.
///
/// 셸 경계(`stellar-ffi`)가 공통 `InputEvent`를 이 분류로 바꿔 [`SubmitOptions`]를 얻습니다.
/// 모든 입력은 [`CommandPriority::Input`]이며, 병합 여부만 다릅니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InputEventClass {
    /// pointer move. 절대 위치이므로 같은 장치의 연속 move는 최신 값만 남깁니다.
    PointerMove {
        /// pointer device tag.
        device: u8,
    },
    /// 크기·backing scale 변화. 절대 상태이므로 연속 resize는 최신 값만 남깁니다.
    Resize,
    /// wheel·scroll. delta가 상대량이라 대체하면 scroll 양을 잃으므로 병합하지 않습니다.
    Wheel,
    /// pointer down/up/cancel, key, text composition, focus. 병합하거나 버리지 않습니다.
    Discrete,
}

impl InputEventClass {
    /// 이 분류의 접수 옵션.
    pub const fn submit_options(self) -> SubmitOptions {
        match self {
            InputEventClass::PointerMove { device } => {
                SubmitOptions::input().with_coalesce(CoalesceKey::PointerMove { device })
            }
            InputEventClass::Resize => SubmitOptions::input().with_coalesce(CoalesceKey::Resize),
            InputEventClass::Wheel | InputEventClass::Discrete => SubmitOptions::input(),
        }
    }
}

/// 접수 옵션: 우선순위와 선택적 병합 key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SubmitOptions {
    /// 우선순위.
    pub priority: CommandPriority,
    /// 병합 key.
    pub coalesce: Option<CoalesceKey>,
}

impl SubmitOptions {
    /// 병합하지 않는 입력.
    pub const fn input() -> SubmitOptions {
        SubmitOptions {
            priority: CommandPriority::Input,
            coalesce: None,
        }
    }

    /// 일반 command.
    pub const fn command() -> SubmitOptions {
        SubmitOptions {
            priority: CommandPriority::Command,
            coalesce: None,
        }
    }

    /// 백그라운드 요청.
    pub const fn background() -> SubmitOptions {
        SubmitOptions {
            priority: CommandPriority::Background,
            coalesce: None,
        }
    }

    /// 병합 key를 지정합니다.
    pub const fn with_coalesce(self, key: CoalesceKey) -> SubmitOptions {
        SubmitOptions {
            priority: self.priority,
            coalesce: Some(key),
        }
    }
}

/// queue 설정 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueConfigError {
    /// lane 용량은 1 이상이어야 합니다.
    ZeroCapacity {
        /// 해당 lane.
        priority: CommandPriority,
    },
}

impl fmt::Display for QueueConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            QueueConfigError::ZeroCapacity { priority } => {
                write!(f, "{} lane 용량은 1 이상이어야 합니다", priority.name())
            }
        }
    }
}

impl std::error::Error for QueueConfigError {}

/// lane별 용량.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QueueCapacity {
    lanes: [usize; 3],
}

impl QueueCapacity {
    /// 기본 입력 lane 용량(초기값, XII.3.5 latency 측정으로 조정).
    pub const DEFAULT_INPUT: usize = 1024;
    /// 기본 command lane 용량(초기값).
    pub const DEFAULT_COMMAND: usize = 1024;
    /// 기본 background lane 용량(초기값).
    pub const DEFAULT_BACKGROUND: usize = 256;

    /// lane별 용량을 지정합니다. 0은 거부합니다.
    pub fn new(
        input: usize,
        command: usize,
        background: usize,
    ) -> Result<QueueCapacity, QueueConfigError> {
        let lanes = [input, command, background];
        for priority in CommandPriority::ALL {
            if lanes[priority.lane()] == 0 {
                return Err(QueueConfigError::ZeroCapacity { priority });
            }
        }
        Ok(QueueCapacity { lanes })
    }

    /// lane 용량.
    pub const fn get(&self, priority: CommandPriority) -> usize {
        self.lanes[priority.lane()]
    }
}

impl Default for QueueCapacity {
    fn default() -> QueueCapacity {
        QueueCapacity {
            lanes: [
                QueueCapacity::DEFAULT_INPUT,
                QueueCapacity::DEFAULT_COMMAND,
                QueueCapacity::DEFAULT_BACKGROUND,
            ],
        }
    }
}

/// 접수 성공 결과. 대체·밀려난 항목은 호출자가 정리합니다.
#[derive(Debug, PartialEq, Eq)]
pub enum Pushed<T> {
    /// lane 끝에 넣었습니다.
    Enqueued,
    /// 같은 key의 마지막 항목을 대체했습니다.
    Coalesced {
        /// 대체된 항목.
        superseded: T,
    },
    /// background lane의 가장 오래된 항목을 밀어내고 넣었습니다.
    EnqueuedEvicting {
        /// 밀려난 항목.
        evicted: T,
    },
}

/// 접수 거부 이유.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectReason {
    /// lane 용량이 찼습니다.
    QueueFull {
        /// 해당 lane.
        priority: CommandPriority,
    },
    /// queue가 닫혔습니다.
    Closed,
}

impl fmt::Display for RejectReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RejectReason::QueueFull { priority } => {
                write!(f, "{} lane 용량이 찼습니다", priority.name())
            }
            RejectReason::Closed => write!(f, "queue가 닫혔습니다"),
        }
    }
}

/// 접수 거부. 항목을 돌려줍니다.
#[derive(Debug, PartialEq, Eq)]
pub struct Rejected<T> {
    /// 거부 이유.
    pub reason: RejectReason,
    /// 접수되지 않은 항목.
    pub item: T,
}

struct Entry<T> {
    coalesce: Option<CoalesceKey>,
    item: T,
}

struct QueueState<T> {
    lanes: [VecDeque<Entry<T>>; 3],
    closed: bool,
}

impl<T> QueueState<T> {
    fn pop(&mut self) -> Option<T> {
        self.lanes
            .iter_mut()
            .find_map(VecDeque::pop_front)
            .map(|entry| entry.item)
    }
}

/// 우선순위별 bounded lane을 가진 다중 생산자·단일 소비자 queue.
pub struct CommandQueue<T> {
    capacity: QueueCapacity,
    state: Mutex<QueueState<T>>,
    ready: Condvar,
}

impl<T> fmt::Debug for CommandQueue<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.lock();
        let lens: Vec<usize> = state.lanes.iter().map(VecDeque::len).collect();
        f.debug_struct("CommandQueue")
            .field("capacity", &self.capacity)
            .field("lane_lens", &lens)
            .field("closed", &state.closed)
            .finish()
    }
}

impl<T> CommandQueue<T> {
    /// 빈 queue를 만듭니다.
    pub fn new(capacity: QueueCapacity) -> CommandQueue<T> {
        CommandQueue {
            capacity,
            state: Mutex::new(QueueState {
                lanes: [VecDeque::new(), VecDeque::new(), VecDeque::new()],
                closed: false,
            }),
            ready: Condvar::new(),
        }
    }

    /// lane별 용량.
    pub fn capacity(&self) -> QueueCapacity {
        self.capacity
    }

    fn lock(&self) -> MutexGuard<'_, QueueState<T>> {
        // 임계 구역에서 panic하는 코드가 없으므로 poison 상태도 일관됩니다.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// 항목을 접수합니다. 기다리지 않고 overflow 정책(모듈 문서)대로 바로 판정합니다.
    pub fn try_push(&self, options: SubmitOptions, item: T) -> Result<Pushed<T>, Rejected<T>> {
        let mut state = self.lock();
        if state.closed {
            return Err(Rejected {
                reason: RejectReason::Closed,
                item,
            });
        }
        let priority = options.priority;
        let capacity = self.capacity.get(priority);
        let lane = &mut state.lanes[priority.lane()];
        if let (Some(key), Some(last)) = (options.coalesce, lane.back_mut()) {
            if last.coalesce == Some(key) {
                let superseded = std::mem::replace(&mut last.item, item);
                return Ok(Pushed::Coalesced { superseded });
            }
        }
        let entry = Entry {
            coalesce: options.coalesce,
            item,
        };
        let pushed = if lane.len() < capacity {
            lane.push_back(entry);
            Pushed::Enqueued
        } else {
            match priority {
                CommandPriority::Input | CommandPriority::Command => {
                    return Err(Rejected {
                        reason: RejectReason::QueueFull { priority },
                        item: entry.item,
                    });
                }
                CommandPriority::Background => match lane.pop_front() {
                    Some(oldest) => {
                        lane.push_back(entry);
                        Pushed::EnqueuedEvicting {
                            evicted: oldest.item,
                        }
                    }
                    // 용량은 1 이상이므로 가득 찬 lane은 비어 있지 않습니다.
                    None => {
                        lane.push_back(entry);
                        Pushed::Enqueued
                    }
                },
            }
        };
        drop(state);
        self.ready.notify_one();
        Ok(pushed)
    }

    /// 가장 높은 우선순위의 가장 오래된 항목을 기다리지 않고 꺼냅니다.
    pub fn try_pop(&self) -> Option<T> {
        self.lock().pop()
    }

    /// 다음 항목을 꺼냅니다. 비어 있으면 기다리고, 닫힌 뒤 비면 `None`입니다.
    ///
    /// 소비자(document thread) 전용입니다. 닫힌 뒤에도 남은 항목은 모두 꺼낼 수 있습니다.
    pub fn pop_blocking(&self) -> Option<T> {
        let mut state = self.lock();
        loop {
            if let Some(item) = state.pop() {
                return Some(item);
            }
            if state.closed {
                return None;
            }
            state = self
                .ready
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// 새 접수를 닫습니다. 이미 접수된 항목은 그대로 둡니다.
    pub fn close(&self) {
        let mut state = self.lock();
        state.closed = true;
        drop(state);
        self.ready.notify_all();
    }

    /// 닫고 남은 항목을 처리 순서대로 꺼냅니다.
    pub fn close_and_drain(&self) -> Vec<T> {
        let mut state = self.lock();
        state.closed = true;
        let mut drained = Vec::new();
        while let Some(item) = state.pop() {
            drained.push(item);
        }
        drop(state);
        self.ready.notify_all();
        drained
    }

    /// 닫혔는지.
    pub fn is_closed(&self) -> bool {
        self.lock().closed
    }

    /// 전체 대기 항목 수.
    pub fn len(&self) -> usize {
        self.lock().lanes.iter().map(VecDeque::len).sum()
    }

    /// 비어 있는지.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// lane의 대기 항목 수.
    pub fn lane_len(&self, priority: CommandPriority) -> usize {
        self.lock().lanes[priority.lane()].len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queue(input: usize, command: usize, background: usize) -> CommandQueue<&'static str> {
        CommandQueue::new(QueueCapacity::new(input, command, background).unwrap())
    }

    fn drain(queue: &CommandQueue<&'static str>) -> Vec<&'static str> {
        std::iter::from_fn(|| queue.try_pop()).collect()
    }

    #[test]
    fn zero_capacity_is_rejected() {
        assert_eq!(
            QueueCapacity::new(0, 1, 1),
            Err(QueueConfigError::ZeroCapacity {
                priority: CommandPriority::Input
            })
        );
        assert_eq!(
            QueueCapacity::new(1, 1, 0),
            Err(QueueConfigError::ZeroCapacity {
                priority: CommandPriority::Background
            })
        );
        let capacity = QueueCapacity::new(3, 2, 1).unwrap();
        assert_eq!(capacity.get(CommandPriority::Input), 3);
        assert_eq!(capacity.get(CommandPriority::Command), 2);
        assert_eq!(capacity.get(CommandPriority::Background), 1);
    }

    #[test]
    fn input_classes_map_to_input_priority() {
        for class in [
            InputEventClass::PointerMove { device: 1 },
            InputEventClass::Resize,
            InputEventClass::Wheel,
            InputEventClass::Discrete,
        ] {
            assert_eq!(class.submit_options().priority, CommandPriority::Input);
        }
        assert_eq!(
            InputEventClass::PointerMove { device: 2 }
                .submit_options()
                .coalesce,
            Some(CoalesceKey::PointerMove { device: 2 })
        );
        assert_eq!(
            InputEventClass::Resize.submit_options().coalesce,
            Some(CoalesceKey::Resize)
        );
        assert_eq!(InputEventClass::Wheel.submit_options().coalesce, None);
        assert_eq!(InputEventClass::Discrete.submit_options().coalesce, None);
    }

    #[test]
    fn priority_names_round_trip() {
        for priority in CommandPriority::ALL {
            assert_eq!(CommandPriority::from_name(priority.name()), Some(priority));
        }
        assert_eq!(CommandPriority::from_name("urgent"), None);
    }

    #[test]
    fn rejected_item_is_returned_to_the_caller() {
        let q = queue(1, 1, 1);
        assert_eq!(
            q.try_push(SubmitOptions::input(), "a"),
            Ok(Pushed::Enqueued)
        );
        assert_eq!(
            q.try_push(SubmitOptions::input(), "b"),
            Err(Rejected {
                reason: RejectReason::QueueFull {
                    priority: CommandPriority::Input
                },
                item: "b"
            })
        );
        q.close();
        assert_eq!(
            q.try_push(SubmitOptions::command(), "c"),
            Err(Rejected {
                reason: RejectReason::Closed,
                item: "c"
            })
        );
        assert_eq!(drain(&q), vec!["a"]);
        assert!(q.pop_blocking().is_none(), "닫힌 빈 queue는 기다리지 않음");
    }

    #[test]
    fn close_and_drain_returns_pending_in_priority_order() {
        let q = queue(2, 2, 2);
        for (options, item) in [
            (SubmitOptions::background(), "b1"),
            (SubmitOptions::command(), "c1"),
            (SubmitOptions::input(), "i1"),
        ] {
            assert_eq!(q.try_push(options, item), Ok(Pushed::Enqueued));
        }
        assert_eq!(q.len(), 3);
        assert_eq!(q.lane_len(CommandPriority::Command), 1);
        assert_eq!(q.close_and_drain(), vec!["i1", "c1", "b1"]);
        assert!(q.is_closed());
        assert!(q.is_empty());
    }

    #[test]
    fn pop_blocking_wakes_on_push_from_another_thread() {
        let q = std::sync::Arc::new(queue(4, 4, 4));
        let consumer = {
            let q = std::sync::Arc::clone(&q);
            std::thread::spawn(move || {
                let mut got = Vec::new();
                while let Some(item) = q.pop_blocking() {
                    got.push(item);
                }
                got
            })
        };
        assert_eq!(
            q.try_push(SubmitOptions::input(), "x"),
            Ok(Pushed::Enqueued)
        );
        q.close();
        assert_eq!(consumer.join().unwrap(), vec!["x"]);
    }

    #[test]
    fn reject_reasons_display_in_korean() {
        assert!(RejectReason::Closed.to_string().contains("닫혔"));
        assert!(RejectReason::QueueFull {
            priority: CommandPriority::Input
        }
        .to_string()
        .contains("input"));
        assert!(QueueConfigError::ZeroCapacity {
            priority: CommandPriority::Command
        }
        .to_string()
        .contains("command"));
    }
}
