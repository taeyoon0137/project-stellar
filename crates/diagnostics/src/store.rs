//! 로컬 diagnostic store schema (XVI.8.1, ADR-0109 결정 1~3).
//!
//! 아키텍처 §39는 기본으로 로컬 진단만 저장하고, 동의할 때만 보낼 수 있는 항목을 비식별 성능 수치, 앱 충돌 정보,
//! 렌더러 오류 코드, 기능별 실패 카운트 네 가지로 정합니다. 이 module은 그 네 가지를 record 종류로 정의하고,
//! 운영체제 사용자 데이터 영역(프로젝트 폴더·Git 밖)에 둘 store 파일 형식을 정합니다.
//!
//! - 모든 field는 정수, 닫힌 enum 이름, 점으로 나눈 version 숫자뿐입니다. 런타임 문자열을 담는 field가 없어서
//!   프로젝트 텍스트·이름·경로, Git URL·branch·commit, 터미널·MCP 입출력을 넣을 자리가 없습니다(타입으로 강제).
//! - 한 줄이 record 하나입니다: `<tag> <field>=<value> ...`. field 순서와 이름은 [`RecordSchema`]가 고정합니다.
//!   같은 schema를 store 파일, 진단 번들([`crate::bundle`]), 금지 필드 scanner([`crate::scanner`])의 구조 검사가
//!   함께 씁니다(codec 하나).
//! - store는 고정 용량입니다. event record(crash·renderer error·performance)는 가득 차면 가장 오래된 것부터 지우고,
//!   기능별 실패 카운트는 `(기능, 코드)`마다 하나의 counter로 합칩니다.
//!
//! ```text
//! stellar.diagnostic-store v1
//! meta dropped=0
//! crash at_ms=1763856000000 kind=panic code=0 thread=document uptime_ms=5000
//! renderer_error at_ms=1763856000100 backend=metal stage=pipeline code=7
//! perf at_ms=1763856000200 interaction=drag document=le_1k visible=le_1k cache=warm latency_ns=4000000 gpu_ns=none
//! feature_failure feature=save code=3 count=2 first_ms=1763856000000 last_ms=1763856000300
//! ```

use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::io::{self, Write as _};
use std::path::Path;

use crate::interaction::{
    CacheState, CompletedTrace, DocumentSizeBucket, InteractionKind, VisibleNodeBucket,
};

/// 닫힌 enum과 저장 이름을 함께 정의합니다. 이름은 `[a-z][a-z0-9_]*`입니다.
macro_rules! closed_enum {
    (
        $(#[$meta:meta])*
        $name:ident { $($(#[$vmeta:meta])* $variant:ident => $text:literal),+ $(,)? }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum $name {
            $($(#[$vmeta])* $variant),+
        }

        impl $name {
            /// 모든 값(선언 순서).
            pub const ALL: &'static [$name] = &[$($name::$variant),+];
            /// 저장 이름 전체([`Self::ALL`] 순서).
            pub const NAMES: &'static [&'static str] = &[$($text),+];

            /// 저장 이름.
            pub const fn name(self) -> &'static str {
                match self {
                    $($name::$variant => $text),+
                }
            }

            /// 저장 이름을 해석합니다.
            pub fn from_name(text: &str) -> Option<$name> {
                Self::ALL.iter().copied().find(|value| value.name() == text)
            }
        }
    };
}

closed_enum! {
    /// 앱 충돌 종류.
    CrashKind {
        /// Rust panic.
        Panic => "panic",
        /// abort.
        Abort => "abort",
        /// 운영체제 signal(번호는 `code`).
        Signal => "signal",
        /// 메모리 부족.
        OutOfMemory => "out_of_memory",
        /// 응답 없음 감시에 걸림.
        Hang => "hang",
    }
}

closed_enum! {
    /// 충돌한 스레드 역할(아키텍처 §8 스레드 모델).
    ThreadRole {
        /// main(UI) 스레드.
        Main => "main",
        /// Document 스레드.
        Document => "document",
        /// render 스레드.
        Render => "render",
        /// 입출력 스레드.
        Io => "io",
        /// worker pool.
        Worker => "worker",
        /// 알 수 없음.
        Unknown => "unknown",
    }
}

closed_enum! {
    /// 렌더러 backend. Windows 지원 보류(ADR-0006)로 D3D12는 두지 않습니다.
    RendererBackend {
        /// Metal.
        Metal => "metal",
    }
}

closed_enum! {
    /// 렌더러 오류가 난 단계.
    RendererStage {
        /// device 생성·소실.
        Device => "device",
        /// 표시 surface.
        Surface => "surface",
        /// pipeline 생성.
        Pipeline => "pipeline",
        /// GPU resource 할당.
        Resource => "resource",
        /// resource upload.
        Upload => "upload",
        /// command encode.
        Encode => "encode",
        /// present.
        Present => "present",
    }
}

closed_enum! {
    /// 실패를 세는 기능(초기 출시 범위).
    ///
    /// 내장 터미널·MCP·에이전트는 초기 출시에서 꺼져 있고, 그 기능의 값은 schema에 두지 않습니다(XVI.8.6).
    Feature {
        /// 프로젝트 열기.
        OpenProject => "open_project",
        /// 저장.
        Save => "save",
        /// 자동 저장.
        Autosave => "autosave",
        /// 복구.
        Recovery => "recovery",
        /// generation migration.
        Migration => "migration",
        /// Git 상태 조회.
        GitStatus => "git_status",
        /// Git stage.
        GitStage => "git_stage",
        /// Git commit.
        GitCommit => "git_commit",
        /// Git merge.
        GitMerge => "git_merge",
        /// 프로젝트 clone.
        CloneProject => "clone_project",
        /// asset 가져오기.
        ImportAsset => "import_asset",
        /// 내보내기.
        Export => "export",
        /// 텍스트 layout.
        TextLayout => "text_layout",
        /// 렌더링.
        Render => "render",
        /// 컴포넌트 갱신.
        ComponentUpdate => "component_update",
        /// 토큰 해석.
        TokenResolve => "token_resolve",
    }
}

closed_enum! {
    /// CPU 구조(진단 번들 `env` 줄).
    Arch {
        /// Apple silicon.
        Arm64 => "arm64",
        /// Intel.
        X86_64 => "x86_64",
    }
}

impl Arch {
    /// 이 build의 CPU 구조.
    pub const fn current() -> Arch {
        if cfg!(target_arch = "x86_64") {
            Arch::X86_64
        } else {
            Arch::Arm64
        }
    }
}

/// [`InteractionKind`] 저장 이름(`InteractionKind::ALL` 순서).
pub const INTERACTION_NAMES: [&str; 14] = {
    let mut out = [""; 14];
    let mut i = 0;
    while i < 14 {
        out[i] = InteractionKind::ALL[i].name();
        i += 1;
    }
    out
};

/// [`DocumentSizeBucket`] 전체.
pub const DOCUMENT_BUCKETS: [DocumentSizeBucket; 5] = [
    DocumentSizeBucket::UpTo1k,
    DocumentSizeBucket::UpTo10k,
    DocumentSizeBucket::UpTo50k,
    DocumentSizeBucket::UpTo200k,
    DocumentSizeBucket::Over200k,
];

/// [`DocumentSizeBucket`] 저장 이름.
pub const DOCUMENT_BUCKET_NAMES: [&str; 5] = [
    DOCUMENT_BUCKETS[0].name(),
    DOCUMENT_BUCKETS[1].name(),
    DOCUMENT_BUCKETS[2].name(),
    DOCUMENT_BUCKETS[3].name(),
    DOCUMENT_BUCKETS[4].name(),
];

/// [`VisibleNodeBucket`] 전체.
pub const VISIBLE_BUCKETS: [VisibleNodeBucket; 4] = [
    VisibleNodeBucket::UpTo1k,
    VisibleNodeBucket::UpTo5k,
    VisibleNodeBucket::UpTo20k,
    VisibleNodeBucket::Over20k,
];

/// [`VisibleNodeBucket`] 저장 이름.
pub const VISIBLE_BUCKET_NAMES: [&str; 4] = [
    VISIBLE_BUCKETS[0].name(),
    VISIBLE_BUCKETS[1].name(),
    VISIBLE_BUCKETS[2].name(),
    VISIBLE_BUCKETS[3].name(),
];

/// [`CacheState`] 전체.
pub const CACHE_STATES: [CacheState; 2] = [CacheState::Cold, CacheState::Warm];

/// [`CacheState`] 저장 이름.
pub const CACHE_STATE_NAMES: [&str; 2] = [CACHE_STATES[0].name(), CACHE_STATES[1].name()];

/// 새 variant가 생기면 위 목록을 함께 고치도록 컴파일에서 막습니다.
#[allow(dead_code)]
const fn exhaustive(document: DocumentSizeBucket, visible: VisibleNodeBucket, cache: CacheState) {
    match document {
        DocumentSizeBucket::UpTo1k
        | DocumentSizeBucket::UpTo10k
        | DocumentSizeBucket::UpTo50k
        | DocumentSizeBucket::UpTo200k
        | DocumentSizeBucket::Over200k => {}
    }
    match visible {
        VisibleNodeBucket::UpTo1k
        | VisibleNodeBucket::UpTo5k
        | VisibleNodeBucket::UpTo20k
        | VisibleNodeBucket::Over20k => {}
    }
    match cache {
        CacheState::Cold | CacheState::Warm => {}
    }
}

/// 세 숫자 version(`major.minor.patch`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Version3 {
    /// major.
    pub major: u32,
    /// minor.
    pub minor: u32,
    /// patch.
    pub patch: u32,
}

impl Version3 {
    /// version.
    pub const fn new(major: u32, minor: u32, patch: u32) -> Version3 {
        Version3 {
            major,
            minor,
            patch,
        }
    }

    /// `major[.minor[.patch]]` 숫자만 해석합니다. 빠진 자리는 0입니다. 숫자 밖의 문자(`-beta` 등)는 거부합니다.
    pub fn parse(text: &str) -> Option<Version3> {
        let mut numbers = [0u32; 3];
        for (index, part) in text.split('.').enumerate() {
            *numbers.get_mut(index)? = u32::try_from(parse_decimal(part)?).ok()?;
        }
        Some(Version3::new(numbers[0], numbers[1], numbers[2]))
    }
}

impl fmt::Display for Version3 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// field 값 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldType {
    /// 0..=u32::MAX 정수.
    U32,
    /// 0..=u64::MAX 정수.
    U64,
    /// 정수 또는 `none`.
    OptU64,
    /// 닫힌 이름 목록 중 하나.
    Name(&'static [&'static str]),
    /// `major.minor.patch`.
    Version,
}

/// field 하나.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldSpec {
    /// field 이름.
    pub name: &'static str,
    /// 값 종류.
    pub ty: FieldType,
}

const fn field(name: &'static str, ty: FieldType) -> FieldSpec {
    FieldSpec { name, ty }
}

/// 한 줄 record schema(field 순서 고정).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordSchema {
    /// 줄 첫 token.
    pub tag: &'static str,
    /// field(순서).
    pub fields: &'static [FieldSpec],
}

/// 앱 충돌.
pub const CRASH_SCHEMA: RecordSchema = RecordSchema {
    tag: "crash",
    fields: &[
        field("at_ms", FieldType::U64),
        field("kind", FieldType::Name(CrashKind::NAMES)),
        field("code", FieldType::U32),
        field("thread", FieldType::Name(ThreadRole::NAMES)),
        field("uptime_ms", FieldType::U64),
    ],
};

/// 렌더러 오류 코드.
pub const RENDERER_ERROR_SCHEMA: RecordSchema = RecordSchema {
    tag: "renderer_error",
    fields: &[
        field("at_ms", FieldType::U64),
        field("backend", FieldType::Name(RendererBackend::NAMES)),
        field("stage", FieldType::Name(RendererStage::NAMES)),
        field("code", FieldType::U32),
    ],
};

/// 비식별 성능 수치(상호작용 trace 요약).
pub const PERFORMANCE_SCHEMA: RecordSchema = RecordSchema {
    tag: "perf",
    fields: &[
        field("at_ms", FieldType::U64),
        field("interaction", FieldType::Name(&INTERACTION_NAMES)),
        field("document", FieldType::Name(&DOCUMENT_BUCKET_NAMES)),
        field("visible", FieldType::Name(&VISIBLE_BUCKET_NAMES)),
        field("cache", FieldType::Name(&CACHE_STATE_NAMES)),
        field("latency_ns", FieldType::U64),
        field("gpu_ns", FieldType::OptU64),
    ],
};

/// 기능별 실패 카운트.
pub const FEATURE_FAILURE_SCHEMA: RecordSchema = RecordSchema {
    tag: "feature_failure",
    fields: &[
        field("feature", FieldType::Name(Feature::NAMES)),
        field("code", FieldType::U32),
        field("count", FieldType::U64),
        field("first_ms", FieldType::U64),
        field("last_ms", FieldType::U64),
    ],
};

/// store·번들 metadata(용량 초과로 지운 event 수).
pub const META_SCHEMA: RecordSchema = RecordSchema {
    tag: "meta",
    fields: &[field("dropped", FieldType::U64)],
};

/// 진단 record schema 네 가지(아키텍처 §39 전송 가능 항목).
pub const RECORD_SCHEMAS: [&RecordSchema; 4] = [
    &CRASH_SCHEMA,
    &RENDERER_ERROR_SCHEMA,
    &PERFORMANCE_SCHEMA,
    &FEATURE_FAILURE_SCHEMA,
];

/// store 파일 첫 줄.
pub const STORE_HEADER: &str = "stellar.diagnostic-store v1";

/// store 파일 이름(운영체제 사용자 데이터의 진단 디렉터리 아래).
pub const STORE_FILE: &str = "store.stldiag";

/// event record 기본 용량.
pub const DEFAULT_EVENT_CAPACITY: usize = 2_048;

/// event record 용량 상한.
pub const MAX_EVENT_CAPACITY: usize = 65_536;

/// 기능별 실패 counter 상한(`(기능, 코드)` 쌍 수). 넘는 새 쌍은 `dropped`에 셉니다.
pub const MAX_FEATURE_COUNTERS: usize = 1_024;

/// 읽을 store·번들 bytes 상한.
pub const MAX_DIAGNOSTIC_BYTES: usize = 8 * 1024 * 1024;

/// 기본 보존 기간(30일).
pub const DEFAULT_RETENTION_MS: u64 = 30 * 24 * 60 * 60 * 1000;

/// 앱 충돌 record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppCrash {
    /// 시각(Unix epoch ms).
    pub at_ms: u64,
    /// 종류.
    pub kind: CrashKind,
    /// signal 번호나 내부 충돌 코드(없으면 0).
    pub code: u32,
    /// 충돌 스레드.
    pub thread: ThreadRole,
    /// 앱 시작부터 충돌까지(ms).
    pub uptime_ms: u64,
}

/// 렌더러 오류 record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RendererError {
    /// 시각(Unix epoch ms).
    pub at_ms: u64,
    /// backend.
    pub backend: RendererBackend,
    /// 단계.
    pub stage: RendererStage,
    /// 오류 코드.
    pub code: u32,
}

/// 비식별 성능 수치 record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PerformanceSample {
    /// 시각(Unix epoch ms).
    pub at_ms: u64,
    /// 상호작용 종류.
    pub interaction: InteractionKind,
    /// 문서 크기 bucket.
    pub document: DocumentSizeBucket,
    /// 가시 node bucket.
    pub visible: VisibleNodeBucket,
    /// cache 상태.
    pub cache: CacheState,
    /// Logical Interaction Latency(ns).
    pub latency_ns: u64,
    /// GPU 실행 시간(ns).
    pub gpu_ns: Option<u64>,
}

impl PerformanceSample {
    /// 완료 trace(XVI.1.2)를 요약합니다. trace ID와 stage 시각은 남기지 않습니다.
    pub fn from_trace(trace: &CompletedTrace, at_ms: u64) -> PerformanceSample {
        let dims = trace.dimensions();
        PerformanceSample {
            at_ms,
            interaction: dims.kind,
            document: dims.document,
            visible: dims.visible,
            cache: dims.cache,
            latency_ns: trace.logical_latency_nanos(),
            gpu_ns: trace.gpu_nanos(),
        }
    }
}

/// 기능별 실패 카운트 record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeatureFailureCount {
    /// 기능.
    pub feature: Feature,
    /// 실패 코드.
    pub code: u32,
    /// 횟수(1 이상).
    pub count: u64,
    /// 처음 실패 시각(Unix epoch ms).
    pub first_ms: u64,
    /// 마지막 실패 시각(Unix epoch ms).
    pub last_ms: u64,
}

/// 진단 record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticRecord {
    /// 앱 충돌.
    Crash(AppCrash),
    /// 렌더러 오류.
    RendererError(RendererError),
    /// 성능 수치.
    Performance(PerformanceSample),
    /// 기능별 실패 카운트.
    FeatureFailure(FeatureFailureCount),
}

/// 해석한 field 값.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldValue {
    /// 정수.
    Int(u64),
    /// `none`.
    Absent,
    /// 닫힌 이름.
    Name(&'static str),
    /// version.
    Version(Version3),
}

impl DiagnosticRecord {
    /// record의 schema.
    pub const fn schema(&self) -> &'static RecordSchema {
        match self {
            DiagnosticRecord::Crash(_) => &CRASH_SCHEMA,
            DiagnosticRecord::RendererError(_) => &RENDERER_ERROR_SCHEMA,
            DiagnosticRecord::Performance(_) => &PERFORMANCE_SCHEMA,
            DiagnosticRecord::FeatureFailure(_) => &FEATURE_FAILURE_SCHEMA,
        }
    }

    /// 시각(실패 카운트는 마지막 실패 시각).
    pub const fn at_ms(&self) -> u64 {
        match self {
            DiagnosticRecord::Crash(r) => r.at_ms,
            DiagnosticRecord::RendererError(r) => r.at_ms,
            DiagnosticRecord::Performance(r) => r.at_ms,
            DiagnosticRecord::FeatureFailure(r) => r.last_ms,
        }
    }

    fn values(&self) -> Vec<FieldValue> {
        use FieldValue::{Absent, Int, Name};
        match self {
            DiagnosticRecord::Crash(r) => vec![
                Int(r.at_ms),
                Name(r.kind.name()),
                Int(u64::from(r.code)),
                Name(r.thread.name()),
                Int(r.uptime_ms),
            ],
            DiagnosticRecord::RendererError(r) => vec![
                Int(r.at_ms),
                Name(r.backend.name()),
                Name(r.stage.name()),
                Int(u64::from(r.code)),
            ],
            DiagnosticRecord::Performance(r) => vec![
                Int(r.at_ms),
                Name(r.interaction.name()),
                Name(r.document.name()),
                Name(r.visible.name()),
                Name(r.cache.name()),
                Int(r.latency_ns),
                r.gpu_ns.map_or(Absent, Int),
            ],
            DiagnosticRecord::FeatureFailure(r) => vec![
                Name(r.feature.name()),
                Int(u64::from(r.code)),
                Int(r.count),
                Int(r.first_ms),
                Int(r.last_ms),
            ],
        }
    }

    /// 고정 형식 한 줄(줄바꿈 없음).
    pub fn to_line(&self) -> String {
        encode_line(self.schema(), &self.values())
    }

    /// 한 줄을 해석합니다. `line_no`는 오류 보고용 1부터 센 줄 번호입니다.
    pub fn from_line(line: &str, line_no: usize) -> Result<DiagnosticRecord, LineError> {
        let (schema, values) = decode_line(&RECORD_SCHEMAS, line, line_no)?;
        let err = |kind| LineError {
            line: line_no,
            kind,
        };
        let int = |i: usize| match values[i] {
            FieldValue::Int(n) => n,
            _ => 0,
        };
        let u32_at = |i: usize| u32::try_from(int(i)).unwrap_or(u32::MAX);
        let name = |i: usize| match values[i] {
            FieldValue::Name(n) => n,
            _ => "",
        };
        // decode_line이 값 종류와 이름 목록을 검사했으므로 아래 from_name은 실패하지 않습니다.
        let invalid = || err(LineErrorKind::Value { index: 0 });
        Ok(match schema.tag {
            "crash" => DiagnosticRecord::Crash(AppCrash {
                at_ms: int(0),
                kind: CrashKind::from_name(name(1)).ok_or_else(invalid)?,
                code: u32_at(2),
                thread: ThreadRole::from_name(name(3)).ok_or_else(invalid)?,
                uptime_ms: int(4),
            }),
            "renderer_error" => DiagnosticRecord::RendererError(RendererError {
                at_ms: int(0),
                backend: RendererBackend::from_name(name(1)).ok_or_else(invalid)?,
                stage: RendererStage::from_name(name(2)).ok_or_else(invalid)?,
                code: u32_at(3),
            }),
            "perf" => DiagnosticRecord::Performance(PerformanceSample {
                at_ms: int(0),
                interaction: InteractionKind::ALL
                    .iter()
                    .copied()
                    .find(|k| k.name() == name(1))
                    .ok_or_else(invalid)?,
                document: DOCUMENT_BUCKETS
                    .iter()
                    .copied()
                    .find(|b| b.name() == name(2))
                    .ok_or_else(invalid)?,
                visible: VISIBLE_BUCKETS
                    .iter()
                    .copied()
                    .find(|b| b.name() == name(3))
                    .ok_or_else(invalid)?,
                cache: CACHE_STATES
                    .iter()
                    .copied()
                    .find(|c| c.name() == name(4))
                    .ok_or_else(invalid)?,
                latency_ns: int(5),
                gpu_ns: match values[6] {
                    FieldValue::Int(n) => Some(n),
                    _ => None,
                },
            }),
            _ => {
                let record = FeatureFailureCount {
                    feature: Feature::from_name(name(0)).ok_or_else(invalid)?,
                    code: u32_at(1),
                    count: int(2),
                    first_ms: int(3),
                    last_ms: int(4),
                };
                if record.count == 0 {
                    return Err(err(LineErrorKind::Value { index: 2 }));
                }
                if record.first_ms > record.last_ms {
                    return Err(err(LineErrorKind::Value { index: 3 }));
                }
                DiagnosticRecord::FeatureFailure(record)
            }
        })
    }
}

/// 줄 해석 오류 종류. 줄 내용은 담지 않습니다(진단 파일에 금지 정보가 섞여 있어도 오류로 새지 않게).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineErrorKind {
    /// 첫 줄이 형식 이름이 아닙니다.
    Header,
    /// 모르는 tag.
    UnknownTag,
    /// field 수가 다릅니다.
    FieldCount {
        /// 기대.
        expected: usize,
        /// 실제.
        actual: usize,
    },
    /// field 이름이나 순서가 다릅니다.
    FieldName {
        /// field index.
        index: usize,
    },
    /// 값이 field 종류에 맞지 않습니다.
    Value {
        /// field index.
        index: usize,
    },
    /// 빈 줄, 연속 공백, 줄 끝 공백, 제어 문자 같은 구문 오류.
    Syntax,
    /// 같은 record가 두 번 있습니다(실패 카운트 `(기능, 코드)` 중복, meta 중복 등).
    Duplicate,
    /// 용량을 넘었습니다.
    Capacity,
}

/// 줄 해석 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineError {
    /// 1부터 센 줄 번호.
    pub line: usize,
    /// 종류.
    pub kind: LineErrorKind,
}

impl fmt::Display for LineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let line = self.line;
        match self.kind {
            LineErrorKind::Header => write!(f, "{line}번째 줄: 진단 형식 이름이 아닙니다"),
            LineErrorKind::UnknownTag => write!(f, "{line}번째 줄: 모르는 record 종류입니다"),
            LineErrorKind::FieldCount { expected, actual } => write!(
                f,
                "{line}번째 줄: field가 {expected}개여야 하지만 {actual}개입니다"
            ),
            LineErrorKind::FieldName { index } => {
                write!(
                    f,
                    "{line}번째 줄: {index}번째 field 이름이 schema와 다릅니다"
                )
            }
            LineErrorKind::Value { index } => {
                write!(
                    f,
                    "{line}번째 줄: {index}번째 field 값이 schema에 맞지 않습니다"
                )
            }
            LineErrorKind::Syntax => write!(f, "{line}번째 줄: 구문 오류입니다"),
            LineErrorKind::Duplicate => write!(f, "{line}번째 줄: 같은 record가 이미 있습니다"),
            LineErrorKind::Capacity => write!(f, "{line}번째 줄: store 용량을 넘었습니다"),
        }
    }
}

impl std::error::Error for LineError {}

/// 0 또는 앞자리 0 없는 10진수만 받습니다(정규형 하나).
fn parse_decimal(text: &str) -> Option<u64> {
    if text.is_empty()
        || text.len() > 20
        || !text.bytes().all(|b| b.is_ascii_digit())
        || (text.len() > 1 && text.starts_with('0'))
    {
        return None;
    }
    text.parse().ok()
}

fn format_value(value: FieldValue) -> String {
    match value {
        FieldValue::Int(n) => n.to_string(),
        FieldValue::Absent => "none".to_owned(),
        FieldValue::Name(name) => name.to_owned(),
        FieldValue::Version(v) => v.to_string(),
    }
}

/// schema 순서대로 `tag field=value ...` 한 줄을 만듭니다. `values`는 schema field 수와 같아야 합니다.
pub fn encode_line(schema: &RecordSchema, values: &[FieldValue]) -> String {
    debug_assert_eq!(schema.fields.len(), values.len());
    let mut line = String::from(schema.tag);
    for (spec, value) in schema.fields.iter().zip(values) {
        line.push(' ');
        line.push_str(spec.name);
        line.push('=');
        line.push_str(&format_value(*value));
    }
    line
}

/// 값 하나를 field 종류로 해석합니다.
pub fn parse_value(ty: FieldType, text: &str) -> Option<FieldValue> {
    match ty {
        FieldType::U32 => parse_decimal(text)
            .filter(|n| *n <= u64::from(u32::MAX))
            .map(FieldValue::Int),
        FieldType::U64 => parse_decimal(text).map(FieldValue::Int),
        FieldType::OptU64 if text == "none" => Some(FieldValue::Absent),
        FieldType::OptU64 => parse_decimal(text).map(FieldValue::Int),
        FieldType::Name(names) => names
            .iter()
            .copied()
            .find(|name| *name == text)
            .map(FieldValue::Name),
        FieldType::Version => {
            let version = Version3::parse(text)?;
            // 정규형(`a.b.c`)만 받습니다.
            (version.to_string() == text).then_some(FieldValue::Version(version))
        }
    }
}

/// `schemas` 중 tag가 맞는 schema로 한 줄을 해석합니다.
pub fn decode_line(
    schemas: &[&'static RecordSchema],
    line: &str,
    line_no: usize,
) -> Result<(&'static RecordSchema, Vec<FieldValue>), LineError> {
    let err = |kind| LineError {
        line: line_no,
        kind,
    };
    if line.is_empty()
        || line.starts_with(' ')
        || line.ends_with(' ')
        || line.contains("  ")
        || line.chars().any(|c| c.is_control())
    {
        return Err(err(LineErrorKind::Syntax));
    }
    let mut tokens = line.split(' ');
    let tag = tokens.next().unwrap_or_default();
    let schema = schemas
        .iter()
        .copied()
        .find(|schema| schema.tag == tag)
        .ok_or_else(|| err(LineErrorKind::UnknownTag))?;
    let tokens: Vec<&str> = tokens.collect();
    if tokens.len() != schema.fields.len() {
        return Err(err(LineErrorKind::FieldCount {
            expected: schema.fields.len(),
            actual: tokens.len(),
        }));
    }
    let mut values = Vec::with_capacity(tokens.len());
    for (index, (spec, token)) in schema.fields.iter().zip(tokens).enumerate() {
        let Some((name, text)) = token.split_once('=') else {
            return Err(err(LineErrorKind::FieldName { index }));
        };
        if name != spec.name {
            return Err(err(LineErrorKind::FieldName { index }));
        }
        values.push(parse_value(spec.ty, text).ok_or_else(|| err(LineErrorKind::Value { index }))?);
    }
    Ok((schema, values))
}

/// schema 어휘 전체: tag, field 이름, 닫힌 enum 이름(중복 없이 정렬).
///
/// scanner는 이 어휘와 같은 문맥 값을 공개 어휘로 보고 비교에서 빼고(ADR-0109 결정 4), feature inventory는 이
/// 어휘에 꺼진 기능의 단어가 없는지 봅니다(결정 8).
pub fn schema_vocabulary(schemas: &[&RecordSchema]) -> Vec<&'static str> {
    let mut words = std::collections::BTreeSet::new();
    for schema in schemas {
        words.insert(schema.tag);
        for spec in schema.fields {
            words.insert(spec.name);
            if let FieldType::Name(names) = spec.ty {
                words.extend(names.iter().copied());
            }
        }
    }
    words.into_iter().collect()
}

/// 보존 정책.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetentionPolicy {
    /// 이보다 오래된 record는 지웁니다(ms).
    pub max_age_ms: u64,
}

impl Default for RetentionPolicy {
    fn default() -> RetentionPolicy {
        RetentionPolicy {
            max_age_ms: DEFAULT_RETENTION_MS,
        }
    }
}

/// store 읽기·쓰기 오류. 경로·내용은 담지 않습니다.
#[derive(Debug)]
pub enum StoreError {
    /// 입출력.
    Io(io::ErrorKind),
    /// bytes 상한을 넘었습니다.
    TooLarge,
    /// UTF-8이 아닙니다.
    Encoding,
    /// 형식 오류.
    Line(LineError),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::Io(kind) => write!(f, "진단 파일 입출력 오류: {kind}"),
            StoreError::TooLarge => write!(
                f,
                "진단 파일이 {MAX_DIAGNOSTIC_BYTES} bytes 상한을 넘었습니다"
            ),
            StoreError::Encoding => write!(f, "진단 파일이 UTF-8이 아닙니다"),
            StoreError::Line(e) => write!(f, "진단 파일 형식 오류: {e}"),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<LineError> for StoreError {
    fn from(e: LineError) -> StoreError {
        StoreError::Line(e)
    }
}

/// 고정 용량 로컬 diagnostic store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticStore {
    capacity: usize,
    events: VecDeque<DiagnosticRecord>,
    counters: BTreeMap<(Feature, u32), FeatureFailureCount>,
    dropped: u64,
}

impl Default for DiagnosticStore {
    fn default() -> DiagnosticStore {
        DiagnosticStore::new(DEFAULT_EVENT_CAPACITY)
    }
}

impl DiagnosticStore {
    /// event `capacity`개를 보관하는 store. 0은 1로, 상한보다 크면 상한으로 맞춥니다.
    pub fn new(capacity: usize) -> DiagnosticStore {
        DiagnosticStore {
            capacity: capacity.clamp(1, MAX_EVENT_CAPACITY),
            events: VecDeque::new(),
            counters: BTreeMap::new(),
            dropped: 0,
        }
    }

    /// event 용량.
    pub const fn capacity(&self) -> usize {
        self.capacity
    }

    /// 용량 초과로 지운 event 수와 상한을 넘어 세지 못한 counter 수.
    pub const fn dropped(&self) -> u64 {
        self.dropped
    }

    /// event record 수.
    pub fn event_count(&self) -> usize {
        self.events.len()
    }

    /// 실패 counter 수.
    pub fn counter_count(&self) -> usize {
        self.counters.len()
    }

    /// 전체 record 수.
    pub fn len(&self) -> usize {
        self.events.len() + self.counters.len()
    }

    /// 비어 있는지.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn push_event(&mut self, record: DiagnosticRecord) {
        if self.events.len() == self.capacity {
            self.events.pop_front();
            self.dropped = self.dropped.saturating_add(1);
        }
        self.events.push_back(record);
    }

    /// 앱 충돌을 남깁니다.
    pub fn push_crash(&mut self, crash: AppCrash) {
        self.push_event(DiagnosticRecord::Crash(crash));
    }

    /// 렌더러 오류를 남깁니다.
    pub fn push_renderer_error(&mut self, error: RendererError) {
        self.push_event(DiagnosticRecord::RendererError(error));
    }

    /// 성능 수치를 남깁니다.
    pub fn push_performance(&mut self, sample: PerformanceSample) {
        self.push_event(DiagnosticRecord::Performance(sample));
    }

    /// 기능 실패 한 번을 셉니다. 같은 `(기능, 코드)`는 counter 하나로 합칩니다.
    pub fn record_feature_failure(&mut self, feature: Feature, code: u32, at_ms: u64) {
        if let Some(counter) = self.counters.get_mut(&(feature, code)) {
            counter.count = counter.count.saturating_add(1);
            counter.first_ms = counter.first_ms.min(at_ms);
            counter.last_ms = counter.last_ms.max(at_ms);
            return;
        }
        if self.counters.len() >= MAX_FEATURE_COUNTERS {
            self.dropped = self.dropped.saturating_add(1);
            return;
        }
        self.counters.insert(
            (feature, code),
            FeatureFailureCount {
                feature,
                code,
                count: 1,
                first_ms: at_ms,
                last_ms: at_ms,
            },
        );
    }

    /// event(넣은 순서) 뒤에 counter(`(기능, 코드)` 순서)를 돌려줍니다.
    pub fn records(&self) -> Vec<DiagnosticRecord> {
        self.events
            .iter()
            .copied()
            .chain(
                self.counters
                    .values()
                    .copied()
                    .map(DiagnosticRecord::FeatureFailure),
            )
            .collect()
    }

    /// 보존 기간보다 오래된 record를 지우고 지운 수를 돌려줍니다.
    pub fn prune(&mut self, policy: RetentionPolicy, now_ms: u64) -> usize {
        let keep = |at_ms: u64| at_ms.saturating_add(policy.max_age_ms) >= now_ms;
        let before = self.len();
        self.events.retain(|record| keep(record.at_ms()));
        self.counters.retain(|_, counter| keep(counter.last_ms));
        before - self.len()
    }

    /// 모든 record를 지웁니다.
    pub fn clear(&mut self) {
        self.events.clear();
        self.counters.clear();
        self.dropped = 0;
    }

    /// record 본문 줄(meta, event, counter).
    pub(crate) fn body_lines(&self) -> Vec<String> {
        let mut lines = vec![encode_line(&META_SCHEMA, &[FieldValue::Int(self.dropped)])];
        lines.extend(self.records().iter().map(DiagnosticRecord::to_line));
        lines
    }

    /// store 파일 bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut text = String::from(STORE_HEADER);
        text.push('\n');
        for line in self.body_lines() {
            text.push_str(&line);
            text.push('\n');
        }
        text.into_bytes()
    }

    /// meta·record 줄을 차례로 넣습니다. `first_line_no`는 첫 줄의 줄 번호입니다.
    pub(crate) fn from_body_lines<'a>(
        lines: impl Iterator<Item = &'a str>,
        first_line_no: usize,
        capacity: usize,
    ) -> Result<DiagnosticStore, LineError> {
        let mut store = DiagnosticStore::new(capacity);
        let mut line_no = first_line_no;
        let mut lines = lines.peekable();
        // 첫 줄은 meta입니다.
        let meta = lines.next().ok_or(LineError {
            line: line_no,
            kind: LineErrorKind::Syntax,
        })?;
        let (_, values) = decode_line(&[&META_SCHEMA], meta, line_no)?;
        if let FieldValue::Int(dropped) = values[0] {
            store.dropped = dropped;
        }
        for line in lines {
            line_no += 1;
            let err = |kind| LineError {
                line: line_no,
                kind,
            };
            if line.starts_with("meta ") || line == "meta" {
                return Err(err(LineErrorKind::Duplicate));
            }
            match DiagnosticRecord::from_line(line, line_no)? {
                DiagnosticRecord::FeatureFailure(counter) => {
                    if store.counters.len() >= MAX_FEATURE_COUNTERS {
                        return Err(err(LineErrorKind::Capacity));
                    }
                    let key = (counter.feature, counter.code);
                    if let Some(last) = store.counters.keys().next_back() {
                        // counter는 key 오름차순 하나뿐입니다(정규형).
                        if *last >= key {
                            return Err(err(LineErrorKind::Duplicate));
                        }
                    }
                    store.counters.insert(key, counter);
                }
                event => {
                    if !store.counters.is_empty() {
                        // event는 counter보다 앞에 있습니다(정규형).
                        return Err(err(LineErrorKind::Syntax));
                    }
                    if store.events.len() == store.capacity {
                        return Err(err(LineErrorKind::Capacity));
                    }
                    store.events.push_back(event);
                }
            }
        }
        Ok(store)
    }

    /// store 파일 bytes를 기본 용량으로 해석합니다.
    pub fn from_bytes(bytes: &[u8]) -> Result<DiagnosticStore, StoreError> {
        DiagnosticStore::from_bytes_with_capacity(bytes, DEFAULT_EVENT_CAPACITY)
    }

    /// store 파일 bytes를 해석합니다.
    pub fn from_bytes_with_capacity(
        bytes: &[u8],
        capacity: usize,
    ) -> Result<DiagnosticStore, StoreError> {
        let text = checked_text(bytes)?;
        let mut lines = text.split('\n');
        if lines.next() != Some(STORE_HEADER) {
            return Err(LineError {
                line: 1,
                kind: LineErrorKind::Header,
            }
            .into());
        }
        let body: Vec<&str> = lines.collect();
        let body = strip_final_newline(&body, body.len() + 1)?;
        Ok(DiagnosticStore::from_body_lines(
            body.iter().copied(),
            2,
            capacity,
        )?)
    }

    /// store 파일을 읽습니다. 파일이 없으면 빈 store입니다.
    pub fn read_file(path: &Path) -> Result<DiagnosticStore, StoreError> {
        match read_limited(path) {
            Ok(bytes) => DiagnosticStore::from_bytes(&bytes),
            Err(StoreError::Io(io::ErrorKind::NotFound)) => Ok(DiagnosticStore::default()),
            Err(e) => Err(e),
        }
    }

    /// store 파일을 소유자 전용 권한으로 원자적으로 씁니다.
    pub fn write_file(&self, path: &Path) -> Result<(), StoreError> {
        write_private_file(path, &self.to_bytes()).map_err(|e| StoreError::Io(e.kind()))
    }
}

/// bytes 상한과 UTF-8을 검사합니다.
pub(crate) fn checked_text(bytes: &[u8]) -> Result<&str, StoreError> {
    if bytes.len() > MAX_DIAGNOSTIC_BYTES {
        return Err(StoreError::TooLarge);
    }
    std::str::from_utf8(bytes).map_err(|_| StoreError::Encoding)
}

/// 마지막 줄바꿈 뒤의 빈 조각을 떼어 냅니다. 파일은 줄바꿈으로 끝나야 합니다.
pub(crate) fn strip_final_newline<'a, 'b>(
    body: &'b [&'a str],
    last_line_no: usize,
) -> Result<&'b [&'a str], StoreError> {
    match body.split_last() {
        Some((&"", rest)) => Ok(rest),
        _ => Err(LineError {
            line: last_line_no,
            kind: LineErrorKind::Syntax,
        }
        .into()),
    }
}

/// 상한까지만 읽습니다.
pub(crate) fn read_limited(path: &Path) -> Result<Vec<u8>, StoreError> {
    use std::io::Read as _;
    let file = std::fs::File::open(path).map_err(|e| StoreError::Io(e.kind()))?;
    let mut bytes = Vec::new();
    file.take(MAX_DIAGNOSTIC_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| StoreError::Io(e.kind()))?;
    if bytes.len() > MAX_DIAGNOSTIC_BYTES {
        return Err(StoreError::TooLarge);
    }
    Ok(bytes)
}

/// 소유자 전용(`0600`) 임시 파일에 쓰고 fsync한 뒤 rename합니다. 부모 디렉터리가 없으면 `0700`으로 만듭니다.
///
/// store, 동의 상태, 전송 대기열이 같은 쓰기 경로를 씁니다.
pub(crate) fn write_private_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    create_private_dir(parent)?;
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
    let mut tmp_name = std::ffi::OsString::from(".");
    tmp_name.push(name);
    tmp_name.push(format!(".{}.tmp", std::process::id()));
    let tmp = parent.join(tmp_name);
    let result = (|| {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = options.open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// 소유자 전용 디렉터리를 만듭니다(이미 있으면 그대로 둡니다).
pub(crate) fn create_private_dir(dir: &Path) -> io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        builder.mode(0o700);
    }
    builder.create(dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interaction::{InteractionTrace, InteractionTraceIds, Stage, TraceDimensions};

    /// 결정적 의사 난수(xorshift).
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }

        fn pick<T: Copy>(&mut self, items: &[T]) -> T {
            items[(self.next() % items.len() as u64) as usize]
        }
    }

    fn random_record(rng: &mut Rng) -> DiagnosticRecord {
        match rng.next() % 3 {
            0 => DiagnosticRecord::Crash(AppCrash {
                at_ms: rng.next(),
                kind: rng.pick(CrashKind::ALL),
                code: rng.next() as u32,
                thread: rng.pick(ThreadRole::ALL),
                uptime_ms: rng.next(),
            }),
            1 => DiagnosticRecord::RendererError(RendererError {
                at_ms: rng.next(),
                backend: rng.pick(RendererBackend::ALL),
                stage: rng.pick(RendererStage::ALL),
                code: rng.next() as u32,
            }),
            _ => DiagnosticRecord::Performance(PerformanceSample {
                at_ms: rng.next(),
                interaction: rng.pick(&InteractionKind::ALL),
                document: rng.pick(&DOCUMENT_BUCKETS),
                visible: rng.pick(&VISIBLE_BUCKETS),
                cache: rng.pick(&CACHE_STATES),
                latency_ns: rng.next(),
                gpu_ns: rng.next().is_multiple_of(2).then(|| rng.next()),
            }),
        }
    }

    fn sample_store() -> DiagnosticStore {
        let mut store = DiagnosticStore::new(16);
        store.push_crash(AppCrash {
            at_ms: 1_763_856_000_000,
            kind: CrashKind::Panic,
            code: 0,
            thread: ThreadRole::Document,
            uptime_ms: 5_000,
        });
        store.push_renderer_error(RendererError {
            at_ms: 1_763_856_000_100,
            backend: RendererBackend::Metal,
            stage: RendererStage::Pipeline,
            code: 7,
        });
        store.push_performance(PerformanceSample {
            at_ms: 1_763_856_000_200,
            interaction: InteractionKind::Drag,
            document: DocumentSizeBucket::UpTo1k,
            visible: VisibleNodeBucket::UpTo1k,
            cache: CacheState::Warm,
            latency_ns: 4_000_000,
            gpu_ns: None,
        });
        store.record_feature_failure(Feature::Save, 3, 1_763_856_000_300);
        store.record_feature_failure(Feature::Save, 3, 1_763_856_000_000);
        store
    }

    #[test]
    fn store_bytes_are_golden() {
        let text = String::from_utf8(sample_store().to_bytes()).unwrap();
        assert_eq!(
            text,
            "stellar.diagnostic-store v1\n\
             meta dropped=0\n\
             crash at_ms=1763856000000 kind=panic code=0 thread=document uptime_ms=5000\n\
             renderer_error at_ms=1763856000100 backend=metal stage=pipeline code=7\n\
             perf at_ms=1763856000200 interaction=drag document=le_1k visible=le_1k cache=warm latency_ns=4000000 gpu_ns=none\n\
             feature_failure feature=save code=3 count=2 first_ms=1763856000000 last_ms=1763856000300\n"
        );
    }

    #[test]
    fn schema_round_trips_every_record_kind_and_value() {
        // 검증 기준(XVI.8.1): schema round-trip.
        let store = sample_store();
        let decoded = DiagnosticStore::from_bytes_with_capacity(&store.to_bytes(), 16).unwrap();
        assert_eq!(decoded, store);
        assert_eq!(decoded.to_bytes(), store.to_bytes());

        // 모든 enum 값, 경계 정수, gpu 있음·없음을 넣은 결정적 무작위 store.
        let mut rng = Rng(0x5eed_1234_abcd_ef01);
        let mut store = DiagnosticStore::new(MAX_EVENT_CAPACITY);
        for _ in 0..4_000 {
            match random_record(&mut rng) {
                DiagnosticRecord::Crash(r) => store.push_crash(r),
                DiagnosticRecord::RendererError(r) => store.push_renderer_error(r),
                DiagnosticRecord::Performance(r) => store.push_performance(r),
                DiagnosticRecord::FeatureFailure(_) => unreachable!(),
            }
        }
        for feature in Feature::ALL {
            for code in [0, 1, u32::MAX] {
                store.record_feature_failure(*feature, code, u64::MAX);
            }
        }
        store.push_crash(AppCrash {
            at_ms: u64::MAX,
            kind: CrashKind::Signal,
            code: u32::MAX,
            thread: ThreadRole::Unknown,
            uptime_ms: 0,
        });
        let bytes = store.to_bytes();
        let decoded =
            DiagnosticStore::from_bytes_with_capacity(&bytes, MAX_EVENT_CAPACITY).unwrap();
        assert_eq!(decoded, store);
        for record in store.records() {
            let line = record.to_line();
            assert_eq!(DiagnosticRecord::from_line(&line, 1), Ok(record), "{line}");
        }
        // 모든 enum 이름이 적어도 한 번 나왔습니다.
        let text = String::from_utf8(bytes).unwrap();
        for name in CrashKind::NAMES
            .iter()
            .chain(ThreadRole::NAMES)
            .chain(RendererStage::NAMES)
            .chain(&INTERACTION_NAMES)
        {
            assert!(text.contains(&format!("={name} ")), "{name}");
        }
    }

    #[test]
    fn trace_summary_keeps_only_buckets_and_durations() {
        let mut ids = InteractionTraceIds::new();
        let mut trace = InteractionTrace::begin(
            ids.issue(),
            TraceDimensions {
                kind: InteractionKind::Zoom,
                document: DocumentSizeBucket::of(40_000),
                visible: VisibleNodeBucket::of(3_000),
                cache: CacheState::Cold,
            },
            100,
        );
        for (i, stage) in Stage::ALL[1..].iter().enumerate() {
            trace.mark(*stage, 200 + i as u64 * 10).unwrap();
        }
        let completed = trace.validate().unwrap();
        let sample = PerformanceSample::from_trace(&completed, 9);
        assert_eq!(sample.latency_ns, completed.logical_latency_nanos());
        assert_eq!(
            DiagnosticRecord::Performance(sample).to_line(),
            "perf at_ms=9 interaction=zoom document=le_50k visible=le_5k cache=cold latency_ns=140 gpu_ns=none"
        );
    }

    #[test]
    fn malformed_lines_are_rejected_without_echo() {
        let cases: &[(&str, LineErrorKind)] = &[
            ("", LineErrorKind::Syntax),
            ("crash  at_ms=1", LineErrorKind::Syntax),
            ("crash at_ms=1 ", LineErrorKind::Syntax),
            ("crash\tat_ms=1", LineErrorKind::Syntax),
            ("terminal at_ms=1", LineErrorKind::UnknownTag),
            (
                "crash at_ms=1",
                LineErrorKind::FieldCount {
                    expected: 5,
                    actual: 1,
                },
            ),
            (
                "crash at_ms=1 kind=panic code=0 thread=main path=/Users/x",
                LineErrorKind::FieldName { index: 4 },
            ),
            (
                "crash at_ms=01 kind=panic code=0 thread=main uptime_ms=1",
                LineErrorKind::Value { index: 0 },
            ),
            (
                "crash at_ms=1 kind=Panic code=0 thread=main uptime_ms=1",
                LineErrorKind::Value { index: 1 },
            ),
            (
                "crash at_ms=1 kind=panic code=4294967296 thread=main uptime_ms=1",
                LineErrorKind::Value { index: 2 },
            ),
            (
                "feature_failure feature=save code=1 count=0 first_ms=1 last_ms=1",
                LineErrorKind::Value { index: 2 },
            ),
            (
                "feature_failure feature=save code=1 count=1 first_ms=2 last_ms=1",
                LineErrorKind::Value { index: 3 },
            ),
            (
                "feature_failure feature=terminal code=1 count=1 first_ms=1 last_ms=1",
                LineErrorKind::Value { index: 0 },
            ),
        ];
        for (line, kind) in cases {
            let err = DiagnosticRecord::from_line(line, 3).unwrap_err();
            assert_eq!(
                err,
                LineError {
                    line: 3,
                    kind: *kind
                },
                "{line:?}"
            );
            assert!(!err.to_string().contains("/Users"), "{err}");
        }
    }

    #[test]
    fn store_files_reject_non_canonical_layouts() {
        let good = String::from_utf8(sample_store().to_bytes()).unwrap();
        let bad = [
            good.replace("stellar.diagnostic-store v1", "stellar.diagnostic-store v2"),
            good.trim_end().to_owned(),
            good.replace("meta dropped=0\n", ""),
            good.replace("meta dropped=0\n", "meta dropped=0\nmeta dropped=0\n"),
            format!(
                "{good}{}",
                "crash at_ms=1 kind=panic code=0 thread=main uptime_ms=1\n"
            ),
            format!(
                "{good}{}",
                "feature_failure feature=save code=3 count=1 first_ms=1 last_ms=1\n"
            ),
        ];
        for text in bad {
            assert!(
                DiagnosticStore::from_bytes(text.as_bytes()).is_err(),
                "{text}"
            );
        }
        assert!(matches!(
            DiagnosticStore::from_bytes(&[0xff, 0xfe]),
            Err(StoreError::Encoding)
        ));
        let too_many = {
            let mut store = DiagnosticStore::new(4);
            for i in 0..4 {
                store.record_feature_failure(Feature::Render, i, 1);
            }
            for _ in 0..4 {
                store.push_crash(AppCrash {
                    at_ms: 1,
                    kind: CrashKind::Abort,
                    code: 0,
                    thread: ThreadRole::Main,
                    uptime_ms: 1,
                });
            }
            store.to_bytes()
        };
        assert!(DiagnosticStore::from_bytes_with_capacity(&too_many, 4).is_ok());
        assert!(matches!(
            DiagnosticStore::from_bytes_with_capacity(&too_many, 3),
            Err(StoreError::Line(LineError {
                kind: LineErrorKind::Capacity,
                ..
            }))
        ));
    }

    #[test]
    fn capacity_and_counters_are_bounded() {
        let mut store = DiagnosticStore::new(8);
        for i in 0..20 {
            store.push_renderer_error(RendererError {
                at_ms: i,
                backend: RendererBackend::Metal,
                stage: RendererStage::Encode,
                code: i as u32,
            });
        }
        assert_eq!(store.event_count(), 8);
        assert_eq!(store.dropped(), 12);
        assert_eq!(store.records()[0].at_ms(), 12);
        for code in 0..(MAX_FEATURE_COUNTERS as u32 + 5) {
            store.record_feature_failure(Feature::Export, code, 1);
        }
        assert_eq!(store.counter_count(), MAX_FEATURE_COUNTERS);
        assert_eq!(store.dropped(), 17);
        // 기존 counter는 계속 셉니다.
        store.record_feature_failure(Feature::Export, 0, 9);
        let first = store.records()[8];
        assert_eq!(
            first,
            DiagnosticRecord::FeatureFailure(FeatureFailureCount {
                feature: Feature::Export,
                code: 0,
                count: 2,
                first_ms: 1,
                last_ms: 9
            })
        );
    }

    #[test]
    fn retention_prunes_old_records_only() {
        let mut store = DiagnosticStore::new(8);
        let day = 24 * 60 * 60 * 1000;
        for at in [0, 10 * day, 40 * day] {
            store.push_crash(AppCrash {
                at_ms: at,
                kind: CrashKind::Hang,
                code: 0,
                thread: ThreadRole::Main,
                uptime_ms: 0,
            });
        }
        store.record_feature_failure(Feature::Save, 1, 5 * day);
        store.record_feature_failure(Feature::Save, 2, 39 * day);
        let removed = store.prune(RetentionPolicy::default(), 40 * day);
        assert_eq!(removed, 2);
        let left: Vec<u64> = store
            .records()
            .iter()
            .map(DiagnosticRecord::at_ms)
            .collect();
        assert_eq!(left, vec![10 * day, 40 * day, 39 * day]);
    }

    #[test]
    fn version_accepts_digits_only() {
        assert_eq!(Version3::parse("26"), Some(Version3::new(26, 0, 0)));
        assert_eq!(Version3::parse("26.1"), Some(Version3::new(26, 1, 0)));
        assert_eq!(Version3::parse("0.1.2"), Some(Version3::new(0, 1, 2)));
        for bad in ["", "1.", "1.2.3.4", "1.2-beta", "v1", "1..2", "01.2"] {
            assert_eq!(Version3::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn files_are_written_owner_only_and_round_trip() {
        let dir = std::env::temp_dir().join(format!(
            "stellar-diag-store-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("nested").join(STORE_FILE);
        assert_eq!(
            DiagnosticStore::read_file(&path).unwrap(),
            DiagnosticStore::default()
        );
        let mut store = DiagnosticStore::default();
        store.record_feature_failure(Feature::GitCommit, 4, 10);
        store.write_file(&path).unwrap();
        assert_eq!(DiagnosticStore::read_file(&path).unwrap(), store);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
            let dir_mode = std::fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(dir_mode & 0o777, 0o700);
        }
        // 임시 파일이 남지 않습니다.
        let names: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from(STORE_FILE)]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
