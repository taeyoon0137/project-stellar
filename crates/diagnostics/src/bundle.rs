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

use std::path::Path;

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
