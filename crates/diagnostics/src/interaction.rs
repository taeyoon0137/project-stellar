//! 상호작용 지연 trace T0~T5 (XVI.1.1~XVI.1.4, ADR-0084 결정 8).
//!
//! 아키텍처 §6.1의 측정 구간을 입력 하나마다 기록합니다.
//!
//! | stage | 의미 |
//! |---|---|
//! | T0 | 운영체제 입력 이벤트를 Stellar가 수신 |
//! | T1 | 이벤트 병합·좌표 변환·Hit Test 완료 |
//! | T2 | Command 생성 및 문서 트랜잭션 반영 완료 |
//! | T3 | 토큰·Binding·레이아웃 증분 갱신 완료 |
//! | T4 | Scene Snapshot·Render Graph 갱신 완료 |
//! | T5 | GPU 명령 제출 및 Present 예약 완료 |
//!
//! - **ID**(XVI.1.1): pointer·key event마다 [`InteractionTraceIds::issue`]가 새 [`InteractionTraceId`]를 발급하고 T0을
//!   기록한 [`InteractionTrace`]를 엽니다. 한 event에 trace 하나입니다.
//! - **stage**(XVI.1.2): T1~T5를 같은 trace에 [`InteractionTrace::mark`]로 기록합니다. 한 stage는 한 번만, 시각은
//!   감소하지 않아야 합니다. [`InteractionTrace::validate`]는 빠진 stage를 오류로 표시합니다. 이 event에서 할 일이
//!   없는 stage(예: pan의 command)도 그 stage를 지나간 시각을 기록합니다.
//! - **duration**(XVI.1.3): stage 구간 5개와 `Logical Interaction Latency = T5 − T0`입니다. GPU timestamp는 다른
//!   clock domain([`GpuTiming`])이라 CPU T0~T5와 더하지 않고 따로 GPU 실행 시간만 냅니다.
//! - **dimension**(XVI.1.4): interaction kind, document size bucket, visible node bucket, cache state만 둡니다.
//!   모두 닫힌 enum이라 object 이름·text·path를 담을 수 없습니다(타입으로 강제).

use std::fmt;
use std::fmt::Write as _;

/// 상호작용 trace 식별자(프로세스 안 단조 증가, 1부터).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InteractionTraceId(u64);

impl InteractionTraceId {
    /// 값.
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// trace ID 발급기.
#[derive(Debug, Clone, Default)]
pub struct InteractionTraceIds {
    issued: u64,
}

impl InteractionTraceIds {
    /// 아직 발급하지 않은 발급기.
    pub const fn new() -> InteractionTraceIds {
        InteractionTraceIds { issued: 0 }
    }

    /// 새 ID. `u64`를 다 쓰면 마지막 값을 반복하지 않고 포화합니다(실사용 범위 밖).
    pub fn issue(&mut self) -> InteractionTraceId {
        self.issued = self.issued.saturating_add(1);
        InteractionTraceId(self.issued)
    }

    /// 발급한 수.
    pub const fn issued(&self) -> u64 {
        self.issued
    }
}

/// T0~T5 stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Stage {
    /// 입력 수신.
    T0,
    /// 병합·좌표 변환·Hit Test 완료.
    T1,
    /// Command·문서 트랜잭션 완료.
    T2,
    /// 토큰·Binding·레이아웃 완료.
    T3,
    /// Scene Snapshot·Render Graph 완료.
    T4,
    /// GPU 제출·Present 예약 완료.
    T5,
}

impl Stage {
    /// 모든 stage(순서).
    pub const ALL: [Stage; 6] = [
        Stage::T0,
        Stage::T1,
        Stage::T2,
        Stage::T3,
        Stage::T4,
        Stage::T5,
    ];

    /// 고정 이름.
    pub const fn name(self) -> &'static str {
        match self {
            Stage::T0 => "t0",
            Stage::T1 => "t1",
            Stage::T2 => "t2",
            Stage::T3 => "t3",
            Stage::T4 => "t4",
            Stage::T5 => "t5",
        }
    }

    const fn index(self) -> usize {
        self as usize
    }
}

/// stage 구간(앞 stage → 뒤 stage).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StageSpan {
    /// T0 → T1: 입력 전달·병합, Hit Test·인터랙션 해석.
    Input,
    /// T1 → T2: Command 생성·문서 변경.
    Command,
    /// T2 → T3: 의존성·토큰·Binding 재평가, 증분 레이아웃.
    Layout,
    /// T3 → T4: Scene Snapshot 갱신, Render Graph·명령 인코딩.
    Scene,
    /// T4 → T5: GPU 제출·Present 예약.
    Present,
}

impl StageSpan {
    /// 모든 구간(순서).
    pub const ALL: [StageSpan; 5] = [
        StageSpan::Input,
        StageSpan::Command,
        StageSpan::Layout,
        StageSpan::Scene,
        StageSpan::Present,
    ];

    /// 고정 이름.
    pub const fn name(self) -> &'static str {
        match self {
            StageSpan::Input => "input",
            StageSpan::Command => "command",
            StageSpan::Layout => "layout",
            StageSpan::Scene => "scene",
            StageSpan::Present => "present",
        }
    }

    /// (시작 stage, 끝 stage).
    pub const fn stages(self) -> (Stage, Stage) {
        match self {
            StageSpan::Input => (Stage::T0, Stage::T1),
            StageSpan::Command => (Stage::T1, Stage::T2),
            StageSpan::Layout => (Stage::T2, Stage::T3),
            StageSpan::Scene => (Stage::T3, Stage::T4),
            StageSpan::Present => (Stage::T4, Stage::T5),
        }
    }
}

/// 상호작용 종류(아키텍처 §6.4·§38.1 시나리오).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum InteractionKind {
    /// pan.
    Pan,
    /// zoom.
    Zoom,
    /// 끌기.
    Drag,
    /// 크기 조절.
    Resize,
    /// 회전.
    Rotate,
    /// hover.
    Hover,
    /// 선택.
    Select,
    /// 텍스트 편집.
    TextEdit,
    /// 단일 속성 변경.
    PropertyEdit,
    /// 토큰 전파.
    TokenEdit,
    /// undo·redo.
    UndoRedo,
}

impl InteractionKind {
    /// 고정 이름.
    pub const fn name(self) -> &'static str {
        match self {
            InteractionKind::Pan => "pan",
            InteractionKind::Zoom => "zoom",
            InteractionKind::Drag => "drag",
            InteractionKind::Resize => "resize",
            InteractionKind::Rotate => "rotate",
            InteractionKind::Hover => "hover",
            InteractionKind::Select => "select",
            InteractionKind::TextEdit => "text_edit",
            InteractionKind::PropertyEdit => "property_edit",
            InteractionKind::TokenEdit => "token_edit",
            InteractionKind::UndoRedo => "undo_redo",
        }
    }
}

/// 활성 문서 node 수 bucket(아키텍처 §5.3 기준 상한 200,000).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DocumentSizeBucket {
    /// 1,000 이하.
    UpTo1k,
    /// 10,000 이하.
    UpTo10k,
    /// 50,000 이하.
    UpTo50k,
    /// 200,000 이하.
    UpTo200k,
    /// 200,000 초과(보장 범위 밖).
    Over200k,
}

impl DocumentSizeBucket {
    /// node 수의 bucket.
    pub const fn of(nodes: u64) -> DocumentSizeBucket {
        match nodes {
            0..=1_000 => DocumentSizeBucket::UpTo1k,
            1_001..=10_000 => DocumentSizeBucket::UpTo10k,
            10_001..=50_000 => DocumentSizeBucket::UpTo50k,
            50_001..=200_000 => DocumentSizeBucket::UpTo200k,
            _ => DocumentSizeBucket::Over200k,
        }
    }

    /// 고정 이름.
    pub const fn name(self) -> &'static str {
        match self {
            DocumentSizeBucket::UpTo1k => "le_1k",
            DocumentSizeBucket::UpTo10k => "le_10k",
            DocumentSizeBucket::UpTo50k => "le_50k",
            DocumentSizeBucket::UpTo200k => "le_200k",
            DocumentSizeBucket::Over200k => "gt_200k",
        }
    }
}

/// 한 프레임 가시 node 수 bucket(아키텍처 §5.3 기준 상한 20,000).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum VisibleNodeBucket {
    /// 1,000 이하.
    UpTo1k,
    /// 5,000 이하.
    UpTo5k,
    /// 20,000 이하.
    UpTo20k,
    /// 20,000 초과(보장 범위 밖).
    Over20k,
}

impl VisibleNodeBucket {
    /// 가시 node 수의 bucket.
    pub const fn of(nodes: u64) -> VisibleNodeBucket {
        match nodes {
            0..=1_000 => VisibleNodeBucket::UpTo1k,
            1_001..=5_000 => VisibleNodeBucket::UpTo5k,
            5_001..=20_000 => VisibleNodeBucket::UpTo20k,
            _ => VisibleNodeBucket::Over20k,
        }
    }

    /// 고정 이름.
    pub const fn name(self) -> &'static str {
        match self {
            VisibleNodeBucket::UpTo1k => "le_1k",
            VisibleNodeBucket::UpTo5k => "le_5k",
            VisibleNodeBucket::UpTo20k => "le_20k",
            VisibleNodeBucket::Over20k => "gt_20k",
        }
    }
}

/// cache 상태(아키텍처 §38.1 cold·warm cache).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CacheState {
    /// cold.
    Cold,
    /// warm.
    Warm,
}

impl CacheState {
    /// 고정 이름.
    pub const fn name(self) -> &'static str {
        match self {
            CacheState::Cold => "cold",
            CacheState::Warm => "warm",
        }
    }
}

/// trace dimension. 허용한 네 값만 담을 수 있습니다(XVI.1.4).
///
/// 문자열을 넣을 field가 없습니다.
///
/// ```compile_fail
/// use stellar_diagnostics::interaction::{TraceDimensions, InteractionKind, DocumentSizeBucket,
///     VisibleNodeBucket, CacheState};
/// let _ = TraceDimensions {
///     kind: InteractionKind::Drag,
///     document: DocumentSizeBucket::UpTo1k,
///     visible: VisibleNodeBucket::UpTo1k,
///     cache: CacheState::Warm,
///     layer_name: String::from("Button"),
/// };
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TraceDimensions {
    /// 상호작용 종류.
    pub kind: InteractionKind,
    /// 문서 크기 bucket.
    pub document: DocumentSizeBucket,
    /// 가시 node bucket.
    pub visible: VisibleNodeBucket,
    /// cache 상태.
    pub cache: CacheState,
}

/// GPU 실행 시간(XVI.1.3). CPU T0~T5와 다른 clock domain이라 더하지 않습니다.
///
/// macOS는 Metal command buffer `GPUStartTime`·`GPUEndTime`만 씁니다(ADR-0084 결정 12). D3D12 timestamp query는
/// Windows 지원 보류(ADR-0006)로 정의하지 않습니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GpuTiming {
    /// GPU 시작(ns, GPU clock domain).
    pub start_nanos: u64,
    /// GPU 끝(ns, GPU clock domain).
    pub end_nanos: u64,
}

impl GpuTiming {
    /// GPU 실행 시간.
    pub const fn duration_nanos(&self) -> u64 {
        self.end_nanos.saturating_sub(self.start_nanos)
    }
}

/// stage 기록 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceError {
    /// stage를 이미 기록했습니다.
    Duplicate {
        /// stage.
        stage: Stage,
    },
    /// 앞 stage보다 이른 시각입니다.
    OutOfOrder {
        /// stage.
        stage: Stage,
        /// 앞 stage 시각.
        previous: u64,
        /// 이 stage 시각.
        at: u64,
    },
    /// 앞 stage가 기록되지 않았는데 뒤 stage를 기록하려 했습니다.
    SkippedStage {
        /// 빠진 앞 stage.
        missing: Stage,
        /// 기록하려 한 stage.
        stage: Stage,
    },
    /// 완료 검사에서 stage가 빠졌습니다(XVI.1.2 validator).
    MissingStage {
        /// 빠진 stage.
        stage: Stage,
    },
    /// GPU 시각이 거꾸로입니다.
    GpuOutOfOrder,
}

impl fmt::Display for TraceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TraceError::Duplicate { stage } => {
                write!(f, "stage {}를 이미 기록했습니다", stage.name())
            }
            TraceError::OutOfOrder {
                stage,
                previous,
                at,
            } => write!(
                f,
                "stage {} 시각 {at}이 앞 stage 시각 {previous}보다 이릅니다",
                stage.name()
            ),
            TraceError::SkippedStage { missing, stage } => write!(
                f,
                "stage {} 앞의 {}가 기록되지 않았습니다",
                stage.name(),
                missing.name()
            ),
            TraceError::MissingStage { stage } => {
                write!(f, "trace에 stage {}가 없습니다", stage.name())
            }
            TraceError::GpuOutOfOrder => f.write_str("GPU 끝 시각이 시작보다 이릅니다"),
        }
    }
}

impl std::error::Error for TraceError {}

/// 진행 중인 trace 하나(event 하나).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InteractionTrace {
    id: InteractionTraceId,
    dimensions: TraceDimensions,
    stamps: [Option<u64>; 6],
    gpu: Option<GpuTiming>,
    queue_depth: Option<u32>,
}

impl InteractionTrace {
    /// T0을 기록한 trace를 엽니다(XVI.1.1).
    pub fn begin(
        id: InteractionTraceId,
        dimensions: TraceDimensions,
        t0_nanos: u64,
    ) -> InteractionTrace {
        let mut stamps = [None; 6];
        stamps[0] = Some(t0_nanos);
        InteractionTrace {
            id,
            dimensions,
            stamps,
            gpu: None,
            queue_depth: None,
        }
    }

    /// ID.
    pub const fn id(&self) -> InteractionTraceId {
        self.id
    }

    /// dimension.
    pub const fn dimensions(&self) -> TraceDimensions {
        self.dimensions
    }

    /// stage 시각.
    pub const fn stamp(&self, stage: Stage) -> Option<u64> {
        self.stamps[stage.index()]
    }

    /// stage 시각을 기록합니다(XVI.1.2). 앞 stage가 모두 있어야 하고 시각은 감소하지 않아야 합니다.
    pub fn mark(&mut self, stage: Stage, at_nanos: u64) -> Result<(), TraceError> {
        let index = stage.index();
        if self.stamps[index].is_some() {
            return Err(TraceError::Duplicate { stage });
        }
        if let Some(previous_stage) = index.checked_sub(1).map(|i| Stage::ALL[i]) {
            let Some(previous) = self.stamps[previous_stage.index()] else {
                return Err(TraceError::SkippedStage {
                    missing: previous_stage,
                    stage,
                });
            };
            if at_nanos < previous {
                return Err(TraceError::OutOfOrder {
                    stage,
                    previous,
                    at: at_nanos,
                });
            }
        }
        self.stamps[index] = Some(at_nanos);
        Ok(())
    }

    /// GPU 실행 시각(다른 clock domain)을 기록합니다.
    pub fn set_gpu(&mut self, gpu: GpuTiming) -> Result<(), TraceError> {
        if gpu.end_nanos < gpu.start_nanos {
            return Err(TraceError::GpuOutOfOrder);
        }
        self.gpu = Some(gpu);
        Ok(())
    }

    /// present 대기열 깊이(frame 수)를 기록합니다.
    pub fn set_queue_depth(&mut self, depth: u32) {
        self.queue_depth = Some(depth);
    }

    /// 모든 stage가 있는지 검사하고 완료 trace를 만듭니다(XVI.1.2 validator).
    pub fn validate(&self) -> Result<CompletedTrace, TraceError> {
        let mut stamps = [0u64; 6];
        for stage in Stage::ALL {
            stamps[stage.index()] =
                self.stamps[stage.index()].ok_or(TraceError::MissingStage { stage })?;
        }
        Ok(CompletedTrace {
            id: self.id,
            dimensions: self.dimensions,
            stamps,
            gpu: self.gpu,
            queue_depth: self.queue_depth,
        })
    }
}

/// 모든 stage가 있는 trace(검증 통과).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompletedTrace {
    id: InteractionTraceId,
    dimensions: TraceDimensions,
    stamps: [u64; 6],
    gpu: Option<GpuTiming>,
    queue_depth: Option<u32>,
}

impl CompletedTrace {
    /// ID.
    pub const fn id(&self) -> InteractionTraceId {
        self.id
    }

    /// dimension.
    pub const fn dimensions(&self) -> TraceDimensions {
        self.dimensions
    }

    /// stage 시각.
    pub const fn stamp(&self, stage: Stage) -> u64 {
        self.stamps[stage.index()]
    }

    /// 구간 시간(ns, XVI.1.3).
    pub const fn span_nanos(&self, span: StageSpan) -> u64 {
        let (from, to) = span.stages();
        self.stamps[to.index()] - self.stamps[from.index()]
    }

    /// Logical Interaction Latency = T5 − T0(ns).
    pub const fn logical_latency_nanos(&self) -> u64 {
        self.stamps[5] - self.stamps[0]
    }

    /// GPU 실행 시간(ns). CPU 구간과 더하지 않습니다.
    pub fn gpu_nanos(&self) -> Option<u64> {
        self.gpu.map(|gpu| gpu.duration_nanos())
    }

    /// present 대기열 깊이.
    pub const fn queue_depth(&self) -> Option<u32> {
        self.queue_depth
    }

    /// 허용한 key만 쓰는 고정 형식 한 줄(로컬 진단 파일용, [`TRACE_LINE_KEYS`]).
    pub fn to_line(&self) -> String {
        let mut line = format!(
            "interaction id={} kind={} document={} visible={} cache={}",
            self.id.0,
            self.dimensions.kind.name(),
            self.dimensions.document.name(),
            self.dimensions.visible.name(),
            self.dimensions.cache.name()
        );
        for stage in Stage::ALL {
            let _ = write!(line, " {}_ns={}", stage.name(), self.stamp(stage));
        }
        let _ = write!(line, " latency_ns={}", self.logical_latency_nanos());
        if let Some(gpu) = self.gpu_nanos() {
            let _ = write!(line, " gpu_ns={gpu}");
        }
        if let Some(depth) = self.queue_depth {
            let _ = write!(line, " queue_depth={depth}");
        }
        line
    }
}

/// [`CompletedTrace::to_line`]이 쓸 수 있는 key 전체(XVI.1.4 privacy schema).
pub const TRACE_LINE_KEYS: [&str; 15] = [
    "id",
    "kind",
    "document",
    "visible",
    "cache",
    "t0_ns",
    "t1_ns",
    "t2_ns",
    "t3_ns",
    "t4_ns",
    "t5_ns",
    "latency_ns",
    "gpu_ns",
    "queue_depth",
    "interaction",
];

#[cfg(test)]
mod tests {
    use super::*;
    use stellar_core_types::clock::{ManualClock, MonotonicClock};

    fn dims() -> TraceDimensions {
        TraceDimensions {
            kind: InteractionKind::Drag,
            document: DocumentSizeBucket::of(200_000),
            visible: VisibleNodeBucket::of(20_000),
            cache: CacheState::Warm,
        }
    }

    #[test]
    fn every_event_gets_its_own_trace_id_with_t0() {
        // 검증 기준(XVI.1.1): 한 event당 trace ID 하나.
        let mut ids = InteractionTraceIds::new();
        let mut clock = ManualClock::new(100);
        let traces: Vec<InteractionTrace> = (0..5)
            .map(|_| {
                clock.advance(10);
                InteractionTrace::begin(ids.issue(), dims(), clock.now_nanos())
            })
            .collect();
        let unique: std::collections::BTreeSet<u64> = traces.iter().map(|t| t.id().get()).collect();
        assert_eq!(unique.len(), 5);
        assert_eq!(ids.issued(), 5);
        assert_eq!(traces[0].stamp(Stage::T0), Some(110));
        assert_eq!(traces[4].stamp(Stage::T0), Some(150));
    }

    #[test]
    fn validator_reports_missing_stages() {
        // 검증 기준(XVI.1.2): 누락 stage를 오류로 표시하는 validator.
        let mut trace = InteractionTrace::begin(InteractionTraceIds::new().issue(), dims(), 0);
        trace.mark(Stage::T1, 10).unwrap();
        trace.mark(Stage::T2, 20).unwrap();
        assert_eq!(
            trace.validate(),
            Err(TraceError::MissingStage { stage: Stage::T3 })
        );
        assert_eq!(
            trace.mark(Stage::T4, 30),
            Err(TraceError::SkippedStage {
                missing: Stage::T3,
                stage: Stage::T4
            })
        );
        assert_eq!(
            trace.mark(Stage::T3, 15),
            Err(TraceError::OutOfOrder {
                stage: Stage::T3,
                previous: 20,
                at: 15
            })
        );
        trace.mark(Stage::T3, 20).unwrap();
        assert_eq!(
            trace.mark(Stage::T3, 21),
            Err(TraceError::Duplicate { stage: Stage::T3 })
        );
        trace.mark(Stage::T4, 40).unwrap();
        trace.mark(Stage::T5, 45).unwrap();
        assert!(trace.validate().is_ok());
    }

    #[test]
    fn manual_clock_reference_durations() {
        // 검증 기준(XVI.1.3): ManualClock reference fixture.
        let mut clock = ManualClock::new(1_000_000);
        let mut trace = InteractionTrace::begin(
            InteractionTraceIds::new().issue(),
            dims(),
            clock.now_nanos(),
        );
        for (stage, delta) in [
            (Stage::T1, 400_000),
            (Stage::T2, 500_000),
            (Stage::T3, 1_600_000),
            (Stage::T4, 1_750_000),
            (Stage::T5, 200_000),
        ] {
            clock.advance(delta);
            trace.mark(stage, clock.now_nanos()).unwrap();
        }
        // GPU timestamp는 다른 domain(값이 CPU와 무관)입니다.
        trace
            .set_gpu(GpuTiming {
                start_nanos: 9_000_000_000,
                end_nanos: 9_003_500_000,
            })
            .unwrap();
        trace.set_queue_depth(1);
        let done = trace.validate().unwrap();
        assert_eq!(done.span_nanos(StageSpan::Input), 400_000);
        assert_eq!(done.span_nanos(StageSpan::Command), 500_000);
        assert_eq!(done.span_nanos(StageSpan::Layout), 1_600_000);
        assert_eq!(done.span_nanos(StageSpan::Scene), 1_750_000);
        assert_eq!(done.span_nanos(StageSpan::Present), 200_000);
        assert_eq!(done.logical_latency_nanos(), 4_450_000);
        // GPU 실행 시간은 따로 내고 latency에 더하지 않습니다.
        assert_eq!(done.gpu_nanos(), Some(3_500_000));
        let total: u64 = StageSpan::ALL.iter().map(|s| done.span_nanos(*s)).sum();
        assert_eq!(total, done.logical_latency_nanos());
        assert_eq!(
            trace.set_gpu(GpuTiming {
                start_nanos: 5,
                end_nanos: 4
            }),
            Err(TraceError::GpuOutOfOrder)
        );
    }

    #[test]
    fn trace_line_uses_only_allowed_keys() {
        // 검증 기준(XVI.1.4): privacy schema. dimension은 닫힌 enum 이름만 냅니다.
        let mut trace = InteractionTrace::begin(InteractionTraceIds::new().issue(), dims(), 0);
        for (stage, at) in [
            (Stage::T1, 1),
            (Stage::T2, 2),
            (Stage::T3, 3),
            (Stage::T4, 4),
            (Stage::T5, 5),
        ] {
            trace.mark(stage, at).unwrap();
        }
        trace.set_queue_depth(1);
        let line = trace.validate().unwrap().to_line();
        let mut parts = line.split(' ');
        assert_eq!(parts.next(), Some("interaction"));
        for part in parts {
            let (key, value) = part.split_once('=').expect("key=value");
            assert!(TRACE_LINE_KEYS.contains(&key), "허용하지 않은 key {key}");
            // 값은 숫자이거나 고정 이름(소문자·숫자·밑줄)입니다.
            assert!(
                value
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
                "{key}={value}"
            );
        }
        assert_eq!(
            line,
            "interaction id=1 kind=drag document=le_200k visible=le_20k cache=warm t0_ns=0 t1_ns=1 \
             t2_ns=2 t3_ns=3 t4_ns=4 t5_ns=5 latency_ns=5 queue_depth=1"
        );
    }

    #[test]
    fn buckets_follow_the_guaranteed_scale() {
        assert_eq!(DocumentSizeBucket::of(1_000), DocumentSizeBucket::UpTo1k);
        assert_eq!(DocumentSizeBucket::of(1_001), DocumentSizeBucket::UpTo10k);
        assert_eq!(
            DocumentSizeBucket::of(200_001),
            DocumentSizeBucket::Over200k
        );
        assert_eq!(VisibleNodeBucket::of(20_000), VisibleNodeBucket::UpTo20k);
        assert_eq!(VisibleNodeBucket::of(20_001), VisibleNodeBucket::Over20k);
    }
}
