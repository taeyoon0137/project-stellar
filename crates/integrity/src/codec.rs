//! 진단 목록 저장 schema v1 (XXV.2.2).
//!
//! 앱 cache·process 경계(CLI·MCP 응답 생성 전 단계)에서 진단을 그대로 옮기기 위한 canonical binary 형식입니다.
//! Git 원본에는 쓰지 않습니다(진단은 원본에서 다시 계산할 수 있는 파생 데이터, ADR-0095 결정 3).
//!
//! ```text
//! file    = magic "SDIX" ++ version(u16 BE = 1) ++ count(u32 BE) ++ diagnostic*
//! diag    = key ++ severity(u8) ++ message(str) ++ action_count(u8) ++ action(u8)*
//! key     = opt_id(document) ++ kind(u8) ++ opt_id(source) ++ opt_path(property) ++ str(code) ++ str(subject) ++ str(origin)
//! opt_id  = 0 | 1 ++ StableId(16)
//! opt_path= 0 | 1 ++ PropertyPath canonical bytes(core-types `to_canonical_bytes`)
//! str     = len(u32 BE) ++ UTF-8
//! ```
//!
//! 숫자 값은 [`DiagnosticKind::code`]·[`Severity::code`]·[`RemediationActionId::code`]입니다. decode는 모르는 값,
//! 잘린 입력, 상한 초과, 뒤에 남은 byte를 오류로 거부합니다(fail closed).

use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_core_types::property::{PropertyKey, PropertyPath};

use crate::diagnostic::{Diagnostic, DiagnosticKey, DiagnosticKind, Severity};
use crate::remediation::RemediationActionId;

/// 파일 머리 magic.
pub const MAGIC: [u8; 4] = *b"SDIX";
/// schema version.
pub const VERSION: u16 = 1;
/// 문자열 하나의 상한(byte).
pub const MAX_STRING: usize = 64 * 1024;
/// property path key 수 상한.
pub const MAX_PATH_KEYS: usize = 1024;
/// 진단 하나의 remediation action 수 상한.
pub const MAX_ACTIONS: usize = 32;

/// decode 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodecError {
    /// magic이 다름.
    BadMagic,
    /// 지원하지 않는 version.
    UnsupportedVersion(u16),
    /// 입력이 잘림.
    Truncated,
    /// 모르는 종류 값.
    UnknownKind(u8),
    /// 모르는 심각도 값.
    UnknownSeverity(u8),
    /// 모르는 remediation action 값.
    UnknownAction(u8),
    /// 선택 값 표시가 0·1이 아님.
    BadFlag(u8),
    /// UTF-8이 아님.
    InvalidUtf8,
    /// 상한 초과.
    TooLarge(&'static str),
    /// 끝 뒤에 byte가 남음.
    TrailingBytes,
}

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CodecError::BadMagic => f.write_str("진단 schema magic이 다릅니다"),
            CodecError::UnsupportedVersion(v) => write!(f, "지원하지 않는 진단 schema version {v}"),
            CodecError::Truncated => f.write_str("진단 입력이 잘렸습니다"),
            CodecError::UnknownKind(v) => write!(f, "모르는 진단 종류 {v}"),
            CodecError::UnknownSeverity(v) => write!(f, "모르는 심각도 {v}"),
            CodecError::UnknownAction(v) => write!(f, "모르는 remediation action {v}"),
            CodecError::BadFlag(v) => write!(f, "선택 값 표시 {v}가 0·1이 아닙니다"),
            CodecError::InvalidUtf8 => f.write_str("문자열이 UTF-8이 아닙니다"),
            CodecError::TooLarge(what) => write!(f, "{what} 상한을 넘었습니다"),
            CodecError::TrailingBytes => f.write_str("진단 입력 끝 뒤에 byte가 남았습니다"),
        }
    }
}

impl std::error::Error for CodecError {}

use crate::diagnostic::{write_key, write_str};

fn write_diagnostic(out: &mut Vec<u8>, diagnostic: &Diagnostic) {
    write_key(out, &diagnostic.key);
    out.push(diagnostic.severity.code());
    write_str(out, &diagnostic.message);
    out.push(diagnostic.remediation.len() as u8);
    for action in &diagnostic.remediation {
        out.push(action.code());
    }
}

/// 진단 목록을 schema v1 bytes로 씁니다. 입력 순서를 그대로 둡니다.
pub fn encode(diagnostics: &[Diagnostic]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&VERSION.to_be_bytes());
    out.extend_from_slice(&(diagnostics.len() as u32).to_be_bytes());
    for diagnostic in diagnostics {
        write_diagnostic(&mut out, diagnostic);
    }
    out
}

struct Reader<'a> {
    bytes: &'a [u8],
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], CodecError> {
        if self.bytes.len() < n {
            return Err(CodecError::Truncated);
        }
        let (head, rest) = self.bytes.split_at(n);
        self.bytes = rest;
        Ok(head)
    }

    fn u8(&mut self) -> Result<u8, CodecError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, CodecError> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> Result<u32, CodecError> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn id(&mut self) -> Result<StableId, CodecError> {
        let mut bytes = [0u8; 16];
        bytes.copy_from_slice(self.take(16)?);
        Ok(StableId::from_bytes(bytes))
    }

    fn flag(&mut self) -> Result<bool, CodecError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            other => Err(CodecError::BadFlag(other)),
        }
    }

    fn opt_id(&mut self) -> Result<Option<StableId>, CodecError> {
        Ok(if self.flag()? { Some(self.id()?) } else { None })
    }

    fn string(&mut self) -> Result<String, CodecError> {
        let len = self.u32()? as usize;
        if len > MAX_STRING {
            return Err(CodecError::TooLarge("문자열"));
        }
        String::from_utf8(self.take(len)?.to_vec()).map_err(|_| CodecError::InvalidUtf8)
    }

    fn path(&mut self) -> Result<PropertyPath, CodecError> {
        let root = self.id()?;
        let count = self.u32()? as usize;
        if count > MAX_PATH_KEYS {
            return Err(CodecError::TooLarge("property path key"));
        }
        let mut keys = Vec::with_capacity(count);
        for _ in 0..count {
            keys.push(PropertyKey(self.u32()?));
        }
        let path = PropertyPath::with_keys(root, keys);
        Ok(match self.u8()? {
            0 => path,
            1 => path.with_index(self.u32()?),
            2 => {
                let key = self.string()?;
                path.with_key_segment(&key)
            }
            other => return Err(CodecError::BadFlag(other)),
        })
    }

    fn key(&mut self) -> Result<DiagnosticKey, CodecError> {
        let document = self.opt_id()?;
        let kind = self.u8()?;
        let kind = DiagnosticKind::from_code(kind).ok_or(CodecError::UnknownKind(kind))?;
        let source = self.opt_id()?;
        let property = if self.flag()? {
            Some(self.path()?)
        } else {
            None
        };
        Ok(DiagnosticKey {
            document,
            kind,
            source,
            property,
            code: self.string()?,
            subject: self.string()?,
            origin: self.string()?,
        })
    }

    fn diagnostic(&mut self) -> Result<Diagnostic, CodecError> {
        let key = self.key()?;
        let severity = self.u8()?;
        let severity =
            Severity::from_code(severity).ok_or(CodecError::UnknownSeverity(severity))?;
        let message = self.string()?;
        let count = usize::from(self.u8()?);
        if count > MAX_ACTIONS {
            return Err(CodecError::TooLarge("remediation action"));
        }
        let mut remediation = Vec::with_capacity(count);
        for _ in 0..count {
            let code = self.u8()?;
            remediation
                .push(RemediationActionId::from_code(code).ok_or(CodecError::UnknownAction(code))?);
        }
        Ok(Diagnostic {
            key,
            severity,
            message,
            remediation,
        })
    }
}

/// key 하나의 canonical bytes([`DiagnosticKey::to_bytes`])를 읽습니다(cursor token).
pub(crate) fn read_key(bytes: &[u8]) -> Result<DiagnosticKey, CodecError> {
    let mut reader = Reader { bytes };
    let key = reader.key()?;
    if !reader.bytes.is_empty() {
        return Err(CodecError::TrailingBytes);
    }
    Ok(key)
}

/// schema v1 bytes를 진단 목록으로 읽습니다.
pub fn decode(bytes: &[u8]) -> Result<Vec<Diagnostic>, CodecError> {
    let mut reader = Reader { bytes };
    if reader.take(4)? != MAGIC {
        return Err(CodecError::BadMagic);
    }
    let version = reader.u16()?;
    if version != VERSION {
        return Err(CodecError::UnsupportedVersion(version));
    }
    let count = reader.u32()? as usize;
    // 진단 하나는 최소 13 byte라 count가 남은 입력보다 클 수 없습니다(거대 할당 방지).
    if count > reader.bytes.len() / 13 + 1 {
        return Err(CodecError::Truncated);
    }
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        out.push(reader.diagnostic()?);
    }
    if !reader.bytes.is_empty() {
        return Err(CodecError::TrailingBytes);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remediation::RemediationActionId as A;

    fn sid(n: u64) -> StableId {
        StableId::new_v7(n, (n % 4096) as u16, n.wrapping_mul(31))
    }

    /// 결정론적 생성기(SplitMix64).
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    fn random_diagnostic(rng: &mut Rng) -> Diagnostic {
        let kind = DiagnosticKind::ALL[rng.below(10) as usize];
        let property = match rng.below(4) {
            0 => None,
            1 => Some(PropertyPath::with_keys(
                sid(rng.next()),
                vec![PropertyKey(rng.below(0x2000) as u32)],
            )),
            2 => Some(
                PropertyPath::new(sid(rng.next()))
                    .push(PropertyKey(7))
                    .with_index(rng.below(9) as u32),
            ),
            _ => Some(PropertyPath::new(sid(rng.next())).with_key_segment("가-key")),
        };
        let actions: Vec<A> = A::ALL.into_iter().filter(|_| rng.below(3) == 0).collect();
        Diagnostic {
            key: DiagnosticKey {
                document: (rng.below(3) != 0).then(|| sid(rng.next())),
                kind,
                source: (rng.below(4) != 0).then(|| sid(rng.next())),
                property,
                code: format!("code-{}", rng.below(50)),
                subject: if rng.below(2) == 0 {
                    String::new()
                } else {
                    "글꼴 ab12".into()
                },
                origin: "test".into(),
            },
            severity: Severity::ALL[rng.below(3) as usize],
            message: format!("설명 {}", rng.next()),
            remediation: actions,
        }
    }

    #[test]
    fn schema_round_trip_is_lossless_and_canonical() {
        // 검증 기준(XXV.2.2): schema round-trip.
        let mut rng = Rng(7);
        for size in [0usize, 1, 2, 17, 300] {
            let list: Vec<Diagnostic> = (0..size).map(|_| random_diagnostic(&mut rng)).collect();
            let bytes = encode(&list);
            let decoded = decode(&bytes).expect("decode");
            assert_eq!(decoded, list);
            assert_eq!(encode(&decoded), bytes, "다시 쓰면 같은 bytes");
        }
    }

    #[test]
    fn every_field_survives_round_trip() {
        let diagnostic = Diagnostic {
            key: DiagnosticKey {
                document: Some(sid(1)),
                kind: DiagnosticKind::OrphanedBinding,
                source: Some(sid(2)),
                property: Some(
                    PropertyPath::with_keys(sid(2), vec![PropertyKey(0x0106)]).with_index(3),
                ),
                code: "orphaned-binding/source-missing".into(),
                subject: "binding".into(),
                origin: "bindings".into(),
            },
            severity: Severity::Error,
            message: "참조 대상이 없습니다".into(),
            remediation: vec![A::RevealProperty],
        };
        let decoded = decode(&encode(std::slice::from_ref(&diagnostic))).unwrap();
        let d = &decoded[0];
        assert_eq!(d.severity, Severity::Error);
        assert_eq!(d.source(), Some(sid(2)));
        assert_eq!(d.property(), diagnostic.property());
        assert_eq!(d.remediation, vec![A::RevealProperty]);
        assert_eq!(d.id(), diagnostic.id());
    }

    #[test]
    fn malformed_input_is_rejected() {
        let mut rng = Rng(3);
        let list: Vec<Diagnostic> = (0..4).map(|_| random_diagnostic(&mut rng)).collect();
        let bytes = encode(&list);
        assert_eq!(decode(b"NOPE\0\x01\0\0\0\0"), Err(CodecError::BadMagic));
        let mut version = bytes.clone();
        version[5] = 2;
        assert_eq!(decode(&version), Err(CodecError::UnsupportedVersion(2)));
        for cut in 0..bytes.len() {
            assert!(decode(&bytes[..cut]).is_err(), "잘린 입력 {cut}");
        }
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert_eq!(decode(&trailing), Err(CodecError::TrailingBytes));
        // 첫 진단의 document flag 뒤 종류 byte를 모르는 값으로.
        let one = encode(&[Diagnostic {
            key: DiagnosticKey {
                document: None,
                kind: DiagnosticKind::MissingFont,
                source: None,
                property: None,
                code: String::new(),
                subject: String::new(),
                origin: String::new(),
            },
            severity: Severity::Info,
            message: String::new(),
            remediation: vec![],
        }]);
        let mut unknown = one.clone();
        unknown[11] = 99;
        assert_eq!(decode(&unknown), Err(CodecError::UnknownKind(99)));
        let mut huge = one.clone();
        huge[6..10].copy_from_slice(&u32::MAX.to_be_bytes());
        assert_eq!(decode(&huge), Err(CodecError::Truncated));
    }
}
