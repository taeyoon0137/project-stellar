//! 제한 YAML parser fuzz harness (XVI.6.1).
//!
//! 외부 fuzz 도구(cargo-fuzz/libFuzzer)는 새 의존성과 nightly toolchain이 필요하므로, 저장소 안의
//! **결정론적** harness를 `#[test]`로 실행합니다.
//!
//! - **입력**: `tests/fuzz_corpus/restricted_yaml/`의 seed corpus, 공개 모델(`Manifest`·`Lockfile`)의
//!   canonical 출력, 무작위 값 트리의 canonical(block)·flow 출력, 고정 stress 입력(깊은 중첩, 많은
//!   키, 긴 스칼라, 크기 상한 경계 등).
//! - **변형**: 고정 seed의 SplitMix64로 byte 뒤집기·삽입·구간 삭제·YAML token 삽입·구간 복제·
//!   splice·구간 반복·절단·들여쓰기 변경·CRLF 변환을 적용합니다. 같은 seed는 항상 같은 입력열을
//!   만듭니다.
//! - **검사(입력마다)**: (1) panic 없음(`catch_unwind`), (2) hang 없음(입력당 시간 상한 + 전체
//!   watchdog), (3) 과도한 할당 없음(thread-local 할당 계측: `load_restricted`의 peak heap이
//!   [`alloc_limit`] 이하 — 입력 길이에 선형인 값 트리 몫 + 선읽기 상한으로 묶인 scanner token
//!   버퍼 몫 + 고정 여유).
//! - **대상**: `load_restricted` → 성공 시 `Manifest::from_yaml`·`Lockfile::from_yaml`·스칼라 엄격
//!   해석·`write_canonical` 재기록 후 재로드 일치(round-trip).
//!
//! 발견한 입력은 `CARGO_TARGET_TMPDIR/restricted-yaml-fuzz/`에 저장하고 테스트를 실패시킵니다.
//! 반복 수 기본값은 고정이며 `STELLAR_YAML_FUZZ_ITERATIONS`로 늘릴 수 있습니다.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::collections::BTreeMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use stellar_dependencies::content_hash::ContentHashText;
use stellar_dependencies::declaration::{
    LogicalName, RefName, RepositoryUrl, SourceKind, VersionSelector,
};
use stellar_dependencies::graph::DependencyPath;
use stellar_project::lock::{Lockfile, ResolvedDependency};
use stellar_project::manifest::{Dependency, Manifest, ProjectMeta, TargetProfile};
use stellar_project::yaml::loader::{
    MAX_EVENT_LOOKAHEAD_CHARS, MAX_INPUT_BYTES, MAX_NESTING_DEPTH,
};
use stellar_project::yaml::scalar::{parse_bool, parse_u32};
use stellar_project::yaml::{load_restricted, write_canonical, LoaderErrorKind, YamlValue};

// --- 할당 계측 ---

/// System 할당자에 위임하면서 현재 thread의 할당량(현재·peak)을 기록하는 할당자.
struct CountingAlloc;

thread_local! {
    static TRACKING: Cell<bool> = const { Cell::new(false) };
    static CURRENT: Cell<isize> = const { Cell::new(0) };
    static PEAK: Cell<isize> = const { Cell::new(0) };
}

/// 계측 중이면 현재 thread의 할당량을 갱신합니다(할당 없음).
fn record(delta: isize) {
    let _ = TRACKING.try_with(|tracking| {
        if tracking.get() {
            let _ = CURRENT.try_with(|current| {
                let now = current.get() + delta;
                current.set(now);
                let _ = PEAK.try_with(|peak| {
                    if now > peak.get() {
                        peak.set(now);
                    }
                });
            });
        }
    });
}

/// 크기를 isize로 변환합니다(Layout 크기는 isize::MAX 이하가 보장됨).
fn signed(size: usize) -> isize {
    isize::try_from(size).unwrap_or(isize::MAX)
}

// SAFETY: 모든 연산을 System 할당자에 그대로 위임하고, 부가 작업(크기 기록)은 할당하지 않습니다.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: 호출자가 GlobalAlloc::alloc 계약을 지키며, 그대로 System에 전달합니다.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            record(signed(layout.size()));
        }
        ptr
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: 호출자가 GlobalAlloc::alloc_zeroed 계약을 지키며, 그대로 System에 전달합니다.
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() {
            record(signed(layout.size()));
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: ptr은 같은 layout으로 이 할당자(=System)에서 할당된 포인터입니다.
        unsafe { System.dealloc(ptr, layout) };
        record(-signed(layout.size()));
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: 호출자가 GlobalAlloc::realloc 계약을 지키며, 그대로 System에 전달합니다.
        let new_ptr = unsafe { System.realloc(ptr, layout, new_size) };
        if !new_ptr.is_null() {
            // 이동 순간 두 블록이 공존할 수 있으므로 새 크기를 먼저 더합니다(보수적 peak).
            record(signed(new_size));
            record(-signed(layout.size()));
        }
        new_ptr
    }
}

#[global_allocator]
static ALLOCATOR: CountingAlloc = CountingAlloc;

/// `f` 실행 동안 현재 thread의 peak heap 증가량(bytes)을 잽니다.
fn measure<T>(f: impl FnOnce() -> T) -> (T, usize) {
    CURRENT.with(|c| c.set(0));
    PEAK.with(|p| p.set(0));
    TRACKING.with(|t| t.set(true));
    let out = f();
    TRACKING.with(|t| t.set(false));
    let peak = usize::try_from(PEAK.with(Cell::get)).unwrap_or(0);
    (out, peak)
}

// --- 결정론적 난수 ---

/// 고정 seed.
const SEED: u64 = 0x5354_454c_4c41_5259;
/// 기본 변형 반복 수.
const DEFAULT_ITERATIONS: usize = 25_000;
/// 변형 입력 길이 상한(실행 시간 고정).
const MAX_MUTATED_LEN: usize = 64 * 1024;
/// 입력당 시간 상한(debug 빌드 기준, 넉넉한 값).
const PER_INPUT_LIMIT: Duration = Duration::from_secs(3);
/// 진행 소식이 이 시간 동안 없으면 hang으로 판정합니다.
const WATCHDOG: Duration = Duration::from_secs(60);
/// 값 트리 할당 계수(입력 1 byte당 허용 bytes). 측정 최대 약 21(block 시퀀스)의 3배 여유.
const TREE_ALLOC_FACTOR: usize = 64;
/// scanner token 버퍼 계수: token 1개 128 bytes × 버퍼 증설 시 이전·새 블록 공존(최대 3배),
/// 입력 1자당 token 최대 1개. 버퍼 크기는 선읽기 상한으로 묶입니다.
const SCANNER_ALLOC_FACTOR: usize = 384;
/// 할당 상한 고정 여유(bytes).
const ALLOC_SLACK: usize = 256 * 1024;

/// 입력 길이별 `load_restricted` peak heap 상한.
fn alloc_limit(len: usize) -> usize {
    TREE_ALLOC_FACTOR * len
        + SCANNER_ALLOC_FACTOR * len.min(MAX_EVENT_LOOKAHEAD_CHARS)
        + ALLOC_SLACK
}

/// SplitMix64 난수기.
struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// `0..n` 범위(n > 0).
    fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }

    /// `num/den` 확률로 참.
    fn chance(&mut self, num: u64, den: u64) -> bool {
        self.next_u64() % den < num
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
}

/// 변형에 삽입하는 YAML 문법 조각(금지 문법·경계 문자 포함).
const TOKENS: &[&str] = &[
    "&a ",
    "*a",
    "&anchor ",
    "*anchor",
    "!!str ",
    "!!map\n",
    "!tag ",
    "!<tag:yaml.org,2002:str> ",
    "!",
    "<<: ",
    "<<",
    "---",
    "--- ",
    "...",
    "%YAML 1.2\n",
    "%TAG ! tag:example.com,2000:\n",
    "? ",
    ": ",
    ":",
    "- ",
    "-",
    "[",
    "]",
    "{",
    "}",
    ", ",
    "\"",
    "'",
    "''",
    "\\",
    "\\u00",
    "\\x4",
    "\\U0010FFFF",
    "|",
    "|-",
    ">+",
    "|2",
    "#",
    " # c",
    "\n",
    "\r\n",
    "\r",
    "\t",
    "  ",
    "    ",
    "\u{FEFF}",
    "\u{85}",
    "\u{2028}",
    "\u{0}",
    "\u{7f}",
    "\u{1b}",
    "~",
    "null",
    "yes",
    "No",
    "on",
    "010",
    "0x1F",
    "+5",
    "1_000",
    ".inf",
    "2020-01-01",
    "true",
    "TRUE",
    "a:",
    "a: b\n",
    "key: value\n",
    "- item\n",
    "schema_version: 1\n",
    "é",
    "한글",
    "😀",
];

/// 무작위 스칼라 문자열.
fn gen_scalar(rng: &mut Rng) -> String {
    const POOL: &[char] = &[
        'a', 'z', 'A', '0', '9', ' ', '\t', '\n', '\r', ':', '-', '#', '&', '*', '!', '?', '|',
        '>', '\'', '"', '\\', '%', '@', '`', '{', '}', '[', ']', ',', 'é', '한', '😀', '\u{0}',
        '\u{1}', '\u{1b}', '\u{7f}', '\u{85}', '\u{a0}', '\u{2028}', '\u{feff}', '\u{fffd}',
    ];
    match rng.below(6) {
        0 => rng.pick(TOKENS).to_string(),
        1 => (0..rng.below(12))
            .map(|_| char::from(b' ' + (rng.below(95) as u8)))
            .collect(),
        2 => (0..rng.below(10)).map(|_| *rng.pick(POOL)).collect(),
        3 => {
            let digits: String = (0..1 + rng.below(6))
                .map(|_| char::from(b'0' + rng.below(10) as u8))
                .collect();
            match rng.below(4) {
                0 => format!("0{digits}"),
                1 => format!("-{digits}"),
                2 => format!("{digits}.{digits}"),
                _ => digits,
            }
        }
        4 => "line one\nline two  \n\tthree".to_string(),
        _ => {
            let mut s = String::from(char::from(b'a' + rng.below(26) as u8));
            for _ in 0..rng.below(10) {
                s.push(*rng.pick(&['a', 'b', 'x', '0', '_', '-']));
            }
            s
        }
    }
}

/// 무작위 값 트리(깊이·폭 제한).
fn gen_value(rng: &mut Rng, depth: usize) -> YamlValue {
    if depth >= 5 || rng.chance(2, 5) {
        return YamlValue::Scalar(gen_scalar(rng));
    }
    let width = rng.below(6);
    if rng.chance(1, 2) {
        let mut pairs = Vec::new();
        for i in 0..width {
            // 가끔 중복 키(거부 경로), 대부분 고유 키.
            let key = if rng.chance(1, 10) {
                gen_scalar(rng)
            } else {
                format!("k{i}_{}", gen_scalar(rng).len())
            };
            pairs.push((YamlValue::Scalar(key), gen_value(rng, depth + 1)));
        }
        YamlValue::Mapping(pairs)
    } else {
        YamlValue::Sequence((0..width).map(|_| gen_value(rng, depth + 1)).collect())
    }
}

/// 값 트리를 flow 스타일 텍스트로 기록합니다(스칼라는 이스케이프한 더블쿼트).
fn to_flow(value: &YamlValue, out: &mut String) {
    match value {
        YamlValue::Scalar(s) => {
            out.push('"');
            for c in s.chars() {
                match c {
                    '"' => out.push_str("\\\""),
                    '\\' => out.push_str("\\\\"),
                    c if (c as u32) < 0x20 || (0x7f..=0x9f).contains(&(c as u32)) => {
                        out.push_str(&format!("\\u{:04x}", c as u32));
                    }
                    c => out.push(c),
                }
            }
            out.push('"');
        }
        YamlValue::Sequence(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                to_flow(item, out);
            }
            out.push(']');
        }
        YamlValue::Mapping(pairs) => {
            out.push('{');
            for (i, (k, v)) in pairs.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                to_flow(k, out);
                out.push_str(": ");
                to_flow(v, out);
            }
            out.push('}');
        }
    }
}

/// 새 무작위 문서(block 또는 flow).
fn gen_document(rng: &mut Rng) -> Vec<u8> {
    let value = gen_value(rng, 0);
    if rng.chance(1, 2) {
        write_canonical(&value).into_bytes()
    } else {
        let mut out = String::new();
        to_flow(&value, &mut out);
        out.push('\n');
        out.into_bytes()
    }
}

/// 입력 하나에 1~8회 변형을 적용합니다.
fn mutate(rng: &mut Rng, mut data: Vec<u8>, seeds: &[Vec<u8>]) -> Vec<u8> {
    for _ in 0..1 + rng.below(8) {
        let len = data.len();
        match rng.below(10) {
            // byte 뒤집기.
            0 if len > 0 => {
                let at = rng.below(len);
                data[at] ^= 1 << rng.below(8);
            }
            // 임의 byte 삽입.
            1 => {
                let at = rng.below(len + 1);
                data.insert(at, rng.below(256) as u8);
            }
            // 구간 삭제.
            2 if len > 0 => {
                let start = rng.below(len);
                let end = (start + 1 + rng.below(16)).min(len);
                data.drain(start..end);
            }
            // YAML token 삽입.
            3 | 4 => {
                let at = rng.below(len + 1);
                let token = rng.pick(TOKENS).as_bytes();
                data.splice(at..at, token.iter().copied());
            }
            // 구간 복제.
            5 if len > 0 => {
                let start = rng.below(len);
                let end = (start + 1 + rng.below(64)).min(len);
                let chunk = data[start..end].to_vec();
                let at = rng.below(data.len() + 1);
                data.splice(at..at, chunk);
            }
            // 다른 seed와 splice.
            6 => {
                let other = rng.pick(seeds);
                let cut = rng.below(len + 1);
                let from = rng.below(other.len() + 1);
                data.truncate(cut);
                data.extend_from_slice(&other[from..]);
            }
            // 구간 반복(중첩·개수 증폭).
            7 if len > 0 => {
                let start = rng.below(len);
                let end = (start + 1 + rng.below(8)).min(len);
                let chunk = data[start..end].to_vec();
                let times = 1 + rng.below(200);
                let at = end;
                let repeated: Vec<u8> = chunk
                    .iter()
                    .copied()
                    .cycle()
                    .take(chunk.len() * times)
                    .collect();
                data.splice(at..at, repeated);
            }
            // 절단.
            8 => {
                let cut = rng.below(len + 1);
                data.truncate(cut);
            }
            // 들여쓰기 변경 또는 CRLF 변환.
            _ => {
                if rng.chance(1, 4) {
                    let mut out = Vec::with_capacity(data.len() + data.len() / 8);
                    for &b in &data {
                        if b == b'\n' {
                            out.push(b'\r');
                        }
                        out.push(b);
                    }
                    data = out;
                } else if let Some(pos) = (0..len)
                    .map(|_| rng.below(len.max(1)))
                    .find(|&p| data.get(p) == Some(&b'\n'))
                {
                    let spaces = rng.below(5);
                    data.splice(pos + 1..pos + 1, std::iter::repeat_n(b' ', spaces));
                }
            }
        }
        if data.len() > MAX_MUTATED_LEN {
            data.truncate(MAX_MUTATED_LEN);
        }
    }
    data
}

// --- seed corpus ---

/// `tests/fuzz_corpus/restricted_yaml/`의 seed 파일(이름 순).
fn corpus_files() -> Vec<(String, Vec<u8>)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fuzz_corpus/restricted_yaml");
    let mut files: Vec<(String, Vec<u8>)> = std::fs::read_dir(&dir)
        .expect("seed corpus 디렉터리")
        .map(|entry| {
            let path = entry.expect("corpus 항목").path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            (name, std::fs::read(&path).expect("corpus 파일 읽기"))
        })
        .collect();
    files.sort();
    files
}

/// 공개 모델의 canonical 출력 seed.
fn model_seeds() -> Vec<Vec<u8>> {
    let manifest = Manifest {
        schema_version: 1,
        project: ProjectMeta {
            id: "0195c530-3acd-7bc1-8414-c213f61cb965".to_string(),
            name: "Fuzz Seed".to_string(),
        },
        target_profiles: vec![TargetProfile {
            id: "apple-desktop".to_string(),
            platform: "apple".to_string(),
            minimum_os: Some("26.0".to_string()),
            baseline: Some("modern".to_string()),
        }],
        dependencies: vec![Dependency {
            name: LogicalName::new("design-system").expect("name"),
            library_id: "0195c530-3acd-7bc1-8414-c213f61cb966"
                .parse()
                .expect("library id"),
            source: SourceKind::Git,
            repository: RepositoryUrl::new("https://example.invalid/design-system.git")
                .expect("url"),
            version: VersionSelector::Tag(RefName::new("v2.4.0").expect("tag")),
            local_override: None,
            components: Default::default(),
        }],
        style_defaults: None,
    };
    let lock = Lockfile {
        schema_version: 1,
        resolved: vec![ResolvedDependency {
            library_id: "0195c530-3acd-7bc1-8414-c213f61cb966"
                .parse()
                .expect("library id"),
            source: SourceKind::Git,
            repository: RepositoryUrl::new("https://example.invalid/design-system.git")
                .expect("url"),
            requested: VersionSelector::Tag(RefName::new("v2.4.0").expect("tag")),
            exact_commit: "0123456789abcdef0123456789abcdef01234567"
                .parse()
                .expect("commit"),
            content_hash: ContentHashText::from_sha256_digest([0x00; 32]),
            schema_version: 1,
            dependency_path: DependencyPath::from_names(vec![
                LogicalName::new("icons").expect("name"),
                LogicalName::new("design-system").expect("name"),
            ]),
            dependencies: Vec::new(),
        }],
    };
    vec![
        write_canonical(&manifest.to_yaml()).into_bytes(),
        lock.write().into_bytes(),
    ]
}

/// 고정 stress 입력(이름, bytes).
fn stress_inputs() -> Vec<(&'static str, Vec<u8>)> {
    let fit = |mut s: String| {
        s.truncate(MAX_INPUT_BYTES);
        s.into_bytes()
    };
    let mut deep_mapping = String::new();
    for i in 0..1_000 {
        deep_mapping.push_str(&" ".repeat(i));
        deep_mapping.push_str("k:\n");
    }
    let many_keys: String = (0..90_000).map(|i| format!("k{i}: v\n")).collect();
    let flow_keys = format!(
        "{{{}}}",
        (0..60_000)
            .map(|i| format!("k{i}: v, "))
            .collect::<String>()
    );
    vec![
        (
            "deep-block-sequence",
            fit(format!("{}x\n", "- ".repeat(200_000))),
        ),
        ("deep-block-mapping", fit(deep_mapping)),
        (
            "deep-flow-over-u8",
            format!("{}{}", "[".repeat(300), "]".repeat(300)).into_bytes(),
        ),
        (
            "deep-flow-within-u8",
            format!("{}{}", "[".repeat(200), "]".repeat(200)).into_bytes(),
        ),
        (
            "deep-flow-mapping",
            format!("{}b{}", "{a: ".repeat(120), "}".repeat(120)).into_bytes(),
        ),
        (
            "nesting-at-limit",
            format!(
                "{}{}",
                "[".repeat(MAX_NESTING_DEPTH),
                "]".repeat(MAX_NESTING_DEPTH)
            )
            .into_bytes(),
        ),
        ("many-block-keys", fit(many_keys)),
        ("many-flow-keys", fit(flow_keys)),
        (
            "many-sequence-items",
            fit(format!("[{}a]", "a,".repeat(400_000))),
        ),
        (
            "long-plain-scalar",
            fit(format!("a: {}", "x".repeat(MAX_INPUT_BYTES))),
        ),
        (
            "long-folded-plain-scalar",
            fit(format!("a: {}", "word\n  ".repeat(120_000))),
        ),
        (
            "many-escapes",
            fit(format!("a: \"{}\"", "\\U0001F600".repeat(90_000))),
        ),
        ("many-documents", fit("a: 1\n---\n".repeat(100_000))),
        (
            "alias-amplification",
            b"a: &a [x, x]\nb: &b [*a, *a]\nc: &c [*b, *b]\nd: [*c, *c]\n".to_vec(),
        ),
        (
            "long-simple-key",
            format!("{}: b\n", "a".repeat(5_000)).into_bytes(),
        ),
        (
            "whitespace-run",
            fit(format!("{}a: b\n", " ".repeat(MAX_INPUT_BYTES))),
        ),
        ("comment-run", fit("#".repeat(MAX_INPUT_BYTES))),
        ("nul-run", "\u{0}".repeat(100_000).into_bytes()),
        ("tab-run", fit(format!("a:{}b\n", "\t".repeat(500_000)))),
        ("crlf-lines", fit("- a\r\n".repeat(150_000))),
        ("exact-size-limit", "#".repeat(MAX_INPUT_BYTES).into_bytes()),
        (
            "over-size-limit",
            "#".repeat(MAX_INPUT_BYTES + 1).into_bytes(),
        ),
    ]
}

// --- 실행과 판정 ---

/// 발견 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum FindingKind {
    Panic,
    Hang,
    ExcessiveAllocation,
    RoundTrip,
}

/// 발견 한 건.
#[derive(Debug)]
struct Finding {
    kind: FindingKind,
    origin: String,
    detail: String,
    input: Vec<u8>,
}

/// 캠페인 결과.
#[derive(Debug, Default)]
struct Report {
    executed: usize,
    accepted: usize,
    rejected: BTreeMap<&'static str, usize>,
    /// 1 KiB 이상 입력의 최대 peak/입력 비율.
    max_alloc_ratio: f64,
    max_alloc_origin: String,
    /// 선읽기 상한보다 긴 입력의 최대 peak/입력 비율(값 트리 몫이 지배).
    max_large_alloc_ratio: f64,
    max_large_alloc_origin: String,
    /// 최대 peak heap(bytes).
    max_peak: usize,
    max_peak_origin: String,
    max_elapsed: Duration,
    max_elapsed_origin: String,
    findings: Vec<Finding>,
}

/// panic payload를 문자열로.
fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "<non-string panic>".to_string()
    }
}

/// 값 트리의 모든 스칼라에 엄격 해석기를 적용합니다(panic 검사용).
fn visit_scalars(value: &YamlValue) {
    match value {
        YamlValue::Scalar(s) => {
            let _ = parse_bool(s);
            let _ = parse_u32(s);
        }
        YamlValue::Sequence(items) => items.iter().for_each(visit_scalars),
        YamlValue::Mapping(pairs) => {
            for (k, v) in pairs {
                visit_scalars(k);
                visit_scalars(v);
            }
        }
    }
}

/// 성공적으로 로드된 값의 후속 경로를 실행하고 round-trip 불일치를 보고합니다.
fn exercise_accepted(value: &YamlValue) -> Result<(), String> {
    let _ = Manifest::from_yaml(value);
    let _ = Lockfile::from_yaml(value);
    visit_scalars(value);
    let written = write_canonical(value);
    match load_restricted(&written) {
        Ok(reloaded) if &reloaded == value => Ok(()),
        Ok(_) => Err("재기록 후 재로드 값이 다릅니다".to_string()),
        Err(e) => Err(format!("재기록 텍스트를 다시 읽지 못함: {e}")),
    }
}

/// 입력 하나를 실행하고 결과를 report에 반영합니다.
fn check(report: &mut Report, origin: String, bytes: Vec<u8>) {
    let input = String::from_utf8_lossy(&bytes).into_owned();
    let started = Instant::now();
    let (outcome, peak) = measure(|| catch_unwind(AssertUnwindSafe(|| load_restricted(&input))));
    let follow_up = match &outcome {
        Ok(Ok(value)) => catch_unwind(AssertUnwindSafe(|| exercise_accepted(value))),
        _ => Ok(Ok(())),
    };
    let elapsed = started.elapsed();
    report.executed += 1;

    let mut found = |kind: FindingKind, detail: String| {
        report.findings.push(Finding {
            kind,
            origin: origin.clone(),
            detail,
            input: input.clone().into_bytes(),
        });
    };
    match &outcome {
        Err(payload) => found(FindingKind::Panic, panic_message(payload.as_ref())),
        Ok(Ok(_)) => report.accepted += 1,
        Ok(Err(error)) => *report.rejected.entry(error.kind.code()).or_insert(0) += 1,
    }
    match follow_up {
        Err(payload) => found(FindingKind::Panic, panic_message(payload.as_ref())),
        Ok(Err(detail)) => found(FindingKind::RoundTrip, detail),
        Ok(Ok(())) => {}
    }
    if elapsed > PER_INPUT_LIMIT {
        found(
            FindingKind::Hang,
            format!("{elapsed:?} > {PER_INPUT_LIMIT:?}"),
        );
    }
    let limit = alloc_limit(input.len());
    if peak > limit {
        found(
            FindingKind::ExcessiveAllocation,
            format!(
                "peak {peak} bytes > limit {limit} (input {} bytes)",
                input.len()
            ),
        );
    }

    let ratio = peak as f64 / (input.len().max(1)) as f64;
    if input.len() >= 1024 && ratio > report.max_alloc_ratio {
        report.max_alloc_ratio = ratio;
        report.max_alloc_origin = origin.clone();
    }
    if input.len() > MAX_EVENT_LOOKAHEAD_CHARS && ratio > report.max_large_alloc_ratio {
        report.max_large_alloc_ratio = ratio;
        report.max_large_alloc_origin = origin.clone();
    }
    if peak > report.max_peak {
        report.max_peak = peak;
        report.max_peak_origin = origin.clone();
    }
    if elapsed > report.max_elapsed {
        report.max_elapsed = elapsed;
        report.max_elapsed_origin = origin;
    }
}

/// 진행 메시지.
enum Progress {
    Tick,
    Done(Box<Report>),
}

/// 캠페인을 실행합니다(stress → seed → 변형).
fn run_campaign(
    iterations: usize,
    current: &Mutex<(String, Vec<u8>)>,
    tx: &mpsc::Sender<Progress>,
) -> Report {
    let mut report = Report::default();
    let run = |report: &mut Report, origin: String, bytes: Vec<u8>| {
        if let Ok(mut slot) = current.lock() {
            *slot = (origin.clone(), bytes.clone());
        }
        check(report, origin, bytes);
        let _ = tx.send(Progress::Tick);
    };

    for (name, bytes) in stress_inputs() {
        run(&mut report, format!("stress:{name}"), bytes);
    }
    let mut seeds: Vec<Vec<u8>> = Vec::new();
    for (name, bytes) in corpus_files() {
        run(&mut report, format!("corpus:{name}"), bytes.clone());
        seeds.push(bytes);
    }
    for (i, bytes) in model_seeds().into_iter().enumerate() {
        run(&mut report, format!("model:{i}"), bytes.clone());
        seeds.push(bytes);
    }

    let mut rng = Rng(SEED);
    for i in 0..iterations {
        let base = if rng.chance(1, 4) {
            gen_document(&mut rng)
        } else {
            rng.pick(&seeds).clone()
        };
        let mutated = mutate(&mut rng, base, &seeds);
        run(&mut report, format!("mutation:{i}"), mutated);
    }
    report
}

/// 발견 입력을 `CARGO_TARGET_TMPDIR`에 저장하고 경로를 반환합니다.
fn save_artifact(index: usize, kind: FindingKind, input: &[u8]) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("restricted-yaml-fuzz");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(format!("{kind:?}-{index}.yaml"));
    let _ = std::fs::write(&path, input);
    path
}

#[test]
fn restricted_yaml_fuzz_campaign_has_no_findings() {
    let iterations = std::env::var("STELLAR_YAML_FUZZ_ITERATIONS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_ITERATIONS);
    let current = Arc::new(Mutex::new((String::new(), Vec::new())));
    let (tx, rx) = mpsc::channel();
    let worker_current = Arc::clone(&current);
    // 기본 크기(2 MiB) stack의 별도 thread에서 실행: 앱 worker thread와 같은 조건.
    std::thread::spawn(move || {
        let report = run_campaign(iterations, &worker_current, &tx);
        let _ = tx.send(Progress::Done(Box::new(report)));
    });

    let report = loop {
        match rx.recv_timeout(WATCHDOG) {
            Ok(Progress::Tick) => {}
            Ok(Progress::Done(report)) => break report,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let (origin, input) = current.lock().map(|g| g.clone()).unwrap_or_default();
                let path = save_artifact(0, FindingKind::Hang, &input);
                panic!("hang: {origin} 입력이 {WATCHDOG:?} 동안 끝나지 않음 (저장: {path:?})");
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                let (origin, _) = current.lock().map(|g| g.clone()).unwrap_or_default();
                panic!("fuzz worker가 비정상 종료됨(마지막 입력: {origin})");
            }
        }
    };

    println!(
        "restricted YAML fuzz: executed={} accepted={} rejected={:?}",
        report.executed, report.accepted, report.rejected
    );
    println!(
        "  max alloc ratio(>=1KiB)={:.1} ({}), max alloc ratio(>lookahead)={:.1} ({})",
        report.max_alloc_ratio,
        report.max_alloc_origin,
        report.max_large_alloc_ratio,
        report.max_large_alloc_origin
    );
    println!(
        "  max peak={} bytes ({}), max elapsed={:?} ({})",
        report.max_peak, report.max_peak_origin, report.max_elapsed, report.max_elapsed_origin
    );

    let mut lines = Vec::new();
    for (i, finding) in report.findings.iter().enumerate() {
        let path = save_artifact(i, finding.kind, &finding.input);
        lines.push(format!(
            "{:?} {} — {} (저장: {path:?})",
            finding.kind, finding.origin, finding.detail
        ));
    }
    assert!(
        report.findings.is_empty(),
        "fuzz 발견 {}건:\n{}",
        report.findings.len(),
        lines.join("\n")
    );
    // 모든 실행이 결정론적 seed 위에서 이루어졌는지(입력 수 고정) 확인.
    assert!(report.executed > iterations);
}

#[test]
fn campaign_inputs_are_deterministic() {
    let seeds: Vec<Vec<u8>> = corpus_files().into_iter().map(|(_, b)| b).collect();
    let sample = |seed: u64| {
        let mut rng = Rng(seed);
        (0..200)
            .map(|_| {
                let base = if rng.chance(1, 4) {
                    gen_document(&mut rng)
                } else {
                    rng.pick(&seeds).clone()
                };
                mutate(&mut rng, base, &seeds)
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(sample(SEED), sample(SEED));
    assert_ne!(sample(SEED), sample(SEED ^ 1));
}

#[test]
fn allocation_meter_counts_heap_bytes() {
    let (_, peak) = measure(|| {
        let v: Vec<u8> = Vec::with_capacity(1 << 20);
        std::hint::black_box(&v);
    });
    assert!(peak >= 1 << 20, "peak {peak}");
    let (_, idle) = measure(|| 1 + 1);
    assert_eq!(idle, 0);
}

#[test]
fn size_and_depth_limits_reject_without_large_allocation() {
    // 크기 상한 초과는 파싱 전에 거부(입력에 비례하는 할당 없음).
    let big = "#".repeat(MAX_INPUT_BYTES + 1);
    let (result, peak) = measure(|| load_restricted(&big));
    assert_eq!(result.unwrap_err().kind, LoaderErrorKind::InputTooLarge);
    assert!(peak < 4096, "peak {peak}");

    // 깊은 block 중첩은 stack overflow 없이 깊이 오류.
    let deep = format!("{}x\n", "- ".repeat(200_000));
    let (result, peak) = measure(|| load_restricted(&deep));
    assert_eq!(result.unwrap_err().kind, LoaderErrorKind::NestingTooDeep);
    assert!(peak < ALLOC_SLACK, "peak {peak}");

    // 상한 깊이까지는 허용, 하나 더 깊으면 거부.
    let at_limit = format!(
        "{}{}",
        "[".repeat(MAX_NESTING_DEPTH),
        "]".repeat(MAX_NESTING_DEPTH)
    );
    assert!(load_restricted(&at_limit).is_ok());
    let over = format!(
        "{}{}",
        "[".repeat(MAX_NESTING_DEPTH + 1),
        "]".repeat(MAX_NESTING_DEPTH + 1)
    );
    assert_eq!(
        load_restricted(&over).unwrap_err().kind,
        LoaderErrorKind::NestingTooDeep
    );
}
