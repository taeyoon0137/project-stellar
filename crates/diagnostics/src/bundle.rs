//! 진단 번들 형식 (XVI.8.2·XVI.8.4, ADR-0109 결정 6).
//!
//! 번들은 로컬 store([`crate::store`])의 record를 앱·운영체제 version, CPU 구조와 함께 담습니다. 수동 내보내기와
//! 동의 뒤 전송 payload가 같은 형식·같은 bytes입니다.
//!
//! ```text
//! stellar.diagnostic-bundle v1
//! env app_version=0.1.0 os_version=26.1.0 arch=arm64
//! meta dropped=0
//! crash at_ms=1763856000000 kind=panic code=0 thread=document uptime_ms=5000
//! end records=1
//! ```
//!
//! 줄 codec과 record schema는 store와 같은 것을 씁니다. `end` 줄의 `records`는 `meta`를 뺀 record 줄 수입니다.
//!
//! # 전송 전 preview와 수동 내보내기 (XVI.8.4)
//!
//! [`preview_payload`]가 store로 번들 bytes를 만들고 금지 필드 scanner로 검사합니다. 탐지가 있으면 거부합니다.
//! 통과한 bytes는 [`PayloadPreview`]에 담기고, 이 타입은 이 module만 만들 수 있습니다. 사용자에게 보여 주는 bytes,
//! 수동 내보내기 파일([`export_bundle`]), 전송 대기열([`crate::consent_gate`])이 보내는 bytes가 모두 이 값 하나입니다.

use std::fmt;
use std::io::{self, Write as _};
use std::path::Path;

use crate::scanner::{scan_bundle, ScanContext, ScanReport};

use crate::store::{
    checked_text, decode_line, encode_line, strip_final_newline, Arch, DiagnosticStore, FieldSpec,
    FieldType, FieldValue, LineError, LineErrorKind, RecordSchema, StoreError, Version3,
    CRASH_SCHEMA, FEATURE_FAILURE_SCHEMA, MAX_EVENT_CAPACITY, META_SCHEMA, PERFORMANCE_SCHEMA,
    RENDERER_ERROR_SCHEMA,
};

/// 번들 첫 줄.
pub const BUNDLE_HEADER: &str = "stellar.diagnostic-bundle v1";

/// 번들 환경 줄.
pub const ENV_SCHEMA: RecordSchema = RecordSchema {
    tag: "env",
    fields: &[
        FieldSpec {
            name: "app_version",
            ty: FieldType::Version,
        },
        FieldSpec {
            name: "os_version",
            ty: FieldType::Version,
        },
        FieldSpec {
            name: "arch",
            ty: FieldType::Name(Arch::NAMES),
        },
    ],
};

/// 번들 끝 줄.
pub const END_SCHEMA: RecordSchema = RecordSchema {
    tag: "end",
    fields: &[FieldSpec {
        name: "records",
        ty: FieldType::U64,
    }],
};

/// 번들에 나올 수 있는 모든 줄 schema(env, meta, record 네 종류, end).
pub const BUNDLE_SCHEMAS: [&RecordSchema; 7] = [
    &ENV_SCHEMA,
    &META_SCHEMA,
    &CRASH_SCHEMA,
    &RENDERER_ERROR_SCHEMA,
    &PERFORMANCE_SCHEMA,
    &FEATURE_FAILURE_SCHEMA,
    &END_SCHEMA,
];

/// macOS 운영체제 version 파일.
pub const MACOS_SYSTEM_VERSION_PLIST: &str = "/System/Library/CoreServices/SystemVersion.plist";

/// 번들 환경. 숫자 version과 CPU 구조뿐입니다(기기 이름·사용자 이름 없음).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BundleEnvironment {
    /// Stellar version.
    pub app_version: Version3,
    /// 운영체제 version(읽지 못하면 `0.0.0`).
    pub os_version: Version3,
    /// CPU 구조.
    pub arch: Arch,
}

impl BundleEnvironment {
    /// 이 기기의 환경. 운영체제 version은 macOS `SystemVersion.plist`의 `ProductVersion`을 숫자로만 읽습니다.
    pub fn current(app_version: Version3) -> BundleEnvironment {
        let os_version = std::fs::read_to_string(MACOS_SYSTEM_VERSION_PLIST)
            .ok()
            .and_then(|text| product_version(&text))
            .unwrap_or_default();
        BundleEnvironment {
            app_version,
            os_version,
            arch: Arch::current(),
        }
    }

    /// `env` 줄.
    pub fn to_line(&self) -> String {
        encode_line(
            &ENV_SCHEMA,
            &[
                FieldValue::Version(self.app_version),
                FieldValue::Version(self.os_version),
                FieldValue::Name(self.arch.name()),
            ],
        )
    }

    /// `env` 줄을 해석합니다.
    pub fn from_line(line: &str, line_no: usize) -> Result<BundleEnvironment, LineError> {
        let (_, values) = decode_line(&[&ENV_SCHEMA], line, line_no)?;
        let version = |i: usize| match values[i] {
            FieldValue::Version(v) => v,
            _ => Version3::default(),
        };
        let arch = match values[2] {
            FieldValue::Name(name) => Arch::from_name(name),
            _ => None,
        }
        .ok_or(LineError {
            line: line_no,
            kind: LineErrorKind::Value { index: 2 },
        })?;
        Ok(BundleEnvironment {
            app_version: version(0),
            os_version: version(1),
            arch,
        })
    }
}

/// plist 본문에서 `ProductVersion` 값을 숫자 version으로 읽습니다. 숫자가 아니면 `None`.
pub fn product_version(plist: &str) -> Option<Version3> {
    let after_key = plist.split_once("<key>ProductVersion</key>")?.1;
    let value = after_key.trim_start().strip_prefix("<string>")?;
    let value = value.split_once("</string>")?.0;
    Version3::parse(value.trim())
}

/// store를 번들 bytes로 만듭니다.
pub fn encode_bundle(environment: &BundleEnvironment, store: &DiagnosticStore) -> Vec<u8> {
    let body = store.body_lines();
    let mut text = String::from(BUNDLE_HEADER);
    text.push('\n');
    text.push_str(&environment.to_line());
    text.push('\n');
    for line in &body {
        text.push_str(line);
        text.push('\n');
    }
    // body 첫 줄은 meta입니다.
    let records = (body.len() - 1) as u64;
    text.push_str(&encode_line(&END_SCHEMA, &[FieldValue::Int(records)]));
    text.push('\n');
    text.into_bytes()
}

/// 번들 bytes를 해석합니다.
pub fn decode_bundle(bytes: &[u8]) -> Result<(BundleEnvironment, DiagnosticStore), StoreError> {
    let text = checked_text(bytes)?;
    let pieces: Vec<&str> = text.split('\n').collect();
    let lines = strip_final_newline(&pieces, pieces.len())?;
    let header_error = |line| LineError {
        line,
        kind: LineErrorKind::Header,
    };
    if lines.first() != Some(&BUNDLE_HEADER) {
        return Err(header_error(1).into());
    }
    let env_line = lines.get(1).ok_or(header_error(2))?;
    let environment = BundleEnvironment::from_line(env_line, 2)?;
    if lines.len() < 4 {
        return Err(LineError {
            line: lines.len() + 1,
            kind: LineErrorKind::Syntax,
        }
        .into());
    }
    let end_no = lines.len();
    let (_, end) = decode_line(&[&END_SCHEMA], lines[end_no - 1], end_no)?;
    let body = &lines[2..end_no - 1];
    let store = DiagnosticStore::from_body_lines(body.iter().copied(), 3, MAX_EVENT_CAPACITY)?;
    if end[0] != FieldValue::Int((body.len() - 1) as u64) {
        return Err(LineError {
            line: end_no,
            kind: LineErrorKind::Value { index: 0 },
        }
        .into());
    }
    Ok((environment, store))
}

/// 검사를 통과한 전송·내보내기 payload. [`preview_payload`]만 만들 수 있습니다.
#[derive(Clone, PartialEq, Eq)]
pub struct PayloadPreview {
    bytes: Vec<u8>,
    records: usize,
}

impl fmt::Debug for PayloadPreview {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PayloadPreview")
            .field("bytes", &self.bytes.len())
            .field("records", &self.records)
            .finish()
    }
}

impl PayloadPreview {
    /// 사용자에게 보여 주고 그대로 보내거나 내보낼 bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// UTF-8 text. scanner를 통과한 번들은 ASCII입니다.
    pub fn text(&self) -> &str {
        std::str::from_utf8(&self.bytes).unwrap_or_default()
    }

    /// bytes 길이.
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// 비어 있는지(번들은 머리 줄이 있어 비지 않습니다).
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// `meta`를 뺀 record 수.
    pub fn records(&self) -> usize {
        self.records
    }

    /// 저장해 둔 payload(전송 대기열 파일)를 다시 검사해 되살립니다.
    pub(crate) fn revalidate(bytes: Vec<u8>) -> Result<PayloadPreview, PreviewRejected> {
        let report = scan_bundle(&bytes, &ScanContext::new());
        if !report.is_clean() {
            return Err(PreviewRejected { report });
        }
        let records = decode_bundle(&bytes)
            .map(|(_, store)| store.len())
            .map_err(|_| PreviewRejected {
                report: ScanReport::default(),
            })?;
        Ok(PayloadPreview { bytes, records })
    }
}

/// 금지 정보가 있어 preview를 거부했습니다. 탐지에는 찾은 문자열이 없습니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewRejected {
    /// scanner 결과.
    pub report: ScanReport,
}

impl fmt::Display for PreviewRejected {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "진단 번들에 수집 금지 정보가 있어 거부했습니다({}건)",
            self.report.findings.len()
        )
    }
}

impl std::error::Error for PreviewRejected {}

/// 전송·내보내기 전 payload를 만듭니다. `context`는 앱이 아는 금지 값입니다(열린 프로젝트 이름·경로 등).
///
/// store record에는 문자열 field가 없어 정상 store는 언제나 통과합니다. 검사는 형식이 바뀌거나 코드가 잘못돼
/// 금지 정보가 섞이는 경우를 막는 두 번째 방어선입니다.
pub fn preview_payload(
    environment: &BundleEnvironment,
    store: &DiagnosticStore,
    context: &ScanContext,
) -> Result<PayloadPreview, PreviewRejected> {
    let bytes = encode_bundle(environment, store);
    let report = scan_bundle(&bytes, context);
    if !report.is_clean() {
        return Err(PreviewRejected { report });
    }
    Ok(PayloadPreview {
        bytes,
        records: store.len(),
    })
}

/// 수동 내보내기: preview bytes를 새 파일에 소유자 전용 권한으로 씁니다. 이미 있는 파일은 덮어쓰지 않습니다.
///
/// 사용자가 직접 하는 로컬 쓰기라 동의 상태와 무관합니다.
pub fn export_bundle(preview: &PayloadPreview, path: &Path) -> io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    let result = file
        .write_all(preview.bytes())
        .and_then(|()| file.sync_all());
    if result.is_err() {
        drop(file);
        let _ = std::fs::remove_file(path);
    }
    result
}

/// 번들 파일을 읽습니다.
pub fn read_bundle(path: &Path) -> Result<(BundleEnvironment, DiagnosticStore), StoreError> {
    decode_bundle(&crate::store::read_limited(path)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{AppCrash, CrashKind, Feature, ThreadRole};

    fn environment() -> BundleEnvironment {
        BundleEnvironment {
            app_version: Version3::new(0, 1, 0),
            os_version: Version3::new(26, 1, 0),
            arch: Arch::Arm64,
        }
    }

    fn store() -> DiagnosticStore {
        let mut store = DiagnosticStore::new(8);
        store.push_crash(AppCrash {
            at_ms: 1_763_856_000_000,
            kind: CrashKind::Panic,
            code: 0,
            thread: ThreadRole::Document,
            uptime_ms: 5_000,
        });
        store.record_feature_failure(Feature::Export, 2, 9);
        store
    }

    #[test]
    fn bundle_bytes_are_golden_and_round_trip() {
        let bytes = encode_bundle(&environment(), &store());
        assert_eq!(
            String::from_utf8(bytes.clone()).unwrap(),
            "stellar.diagnostic-bundle v1\n\
             env app_version=0.1.0 os_version=26.1.0 arch=arm64\n\
             meta dropped=0\n\
             crash at_ms=1763856000000 kind=panic code=0 thread=document uptime_ms=5000\n\
             feature_failure feature=export code=2 count=1 first_ms=9 last_ms=9\n\
             end records=2\n"
        );
        let (env, decoded) = decode_bundle(&bytes).unwrap();
        assert_eq!(env, environment());
        assert_eq!(decoded.records(), store().records());
        let empty = encode_bundle(&environment(), &DiagnosticStore::default());
        let (_, decoded) = decode_bundle(&empty).unwrap();
        assert!(decoded.is_empty());
    }

    #[test]
    fn bundle_layout_errors_are_rejected() {
        let good = String::from_utf8(encode_bundle(&environment(), &store())).unwrap();
        for bad in [
            good.replace("end records=2", "end records=3"),
            good.replace("end records=2\n", ""),
            good.replace("env app_version=0.1.0", "env app_version=0.1"),
            good.replace("arch=arm64", "arch=riscv"),
            good.replace(
                "stellar.diagnostic-bundle v1",
                "stellar.diagnostic-store v1",
            ),
            good.trim_end().to_owned(),
            "stellar.diagnostic-bundle v1\n".to_owned(),
        ] {
            assert!(decode_bundle(bad.as_bytes()).is_err(), "{bad}");
        }
    }

    #[test]
    fn preview_is_scanned_and_export_writes_the_same_bytes() {
        let preview = preview_payload(&environment(), &store(), &ScanContext::new()).unwrap();
        assert_eq!(
            preview.bytes(),
            encode_bundle(&environment(), &store()).as_slice()
        );
        assert_eq!(preview.records(), 2);
        assert_eq!(preview.text().as_bytes(), preview.bytes());
        assert!(!format!("{preview:?}").contains("crash"));
        let dir = std::env::temp_dir().join(format!("stellar-bundle-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("bundle.stldiag");
        export_bundle(&preview, &path).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), preview.bytes());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        // 이미 있는 파일은 덮어쓰지 않습니다.
        assert_eq!(
            export_bundle(&preview, &path).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(std::fs::read(&path).unwrap(), preview.bytes());
        assert_eq!(read_bundle(&path).unwrap().1.records(), store().records());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn stored_payloads_are_rescanned() {
        // 문맥 값이 번들에 나오지 않으면 통과합니다.
        let ctx = ScanContext::new().with(crate::scanner::ForbiddenCategory::ProjectName, "acme");
        let preview = preview_payload(&environment(), &store(), &ctx).unwrap();
        assert_eq!(
            PayloadPreview::revalidate(preview.bytes().to_vec()).unwrap(),
            preview
        );
        // 저장된 bytes가 바뀌어 금지 정보가 들어오면 되살리지 않습니다.
        let tampered = String::from_utf8(preview.bytes().to_vec())
            .unwrap()
            .replace("thread=document", "thread=/home/alice");
        let rejected = PayloadPreview::revalidate(tampered.into_bytes()).unwrap_err();
        assert!(!rejected.report.is_clean());
        assert!(!rejected.to_string().contains("alice"));
    }

    #[test]
    fn product_version_reads_numbers_only() {
        let plist = "<dict>\n\t<key>ProductName</key>\n\t<string>macOS</string>\n\t<key>ProductVersion</key>\n\t<string>26.1</string>\n</dict>";
        assert_eq!(product_version(plist), Some(Version3::new(26, 1, 0)));
        assert_eq!(product_version(&plist.replace("26.1", "26.1 (beta)")), None);
        assert_eq!(product_version("<dict></dict>"), None);
        // 이 기기에서 읽은 값도 숫자 version입니다(읽지 못하면 0.0.0).
        let current = BundleEnvironment::current(Version3::new(1, 2, 3));
        assert_eq!(current.app_version, Version3::new(1, 2, 3));
        assert_eq!(current.arch, Arch::current());
    }
}
