//! chunk kind registry와 unknown kind version 정책 (V.2.7).
//!
//! 새 format version이 새 kind를 추가해도 구버전 reader가 안전하게 동작해야 합니다:
//!
//! - **unknown optional kind → skip**: 몰라도 문서를 여는 데 지장이 없는 kind.
//! - **unknown required kind → 오류**: 모르면 문서를 잘못 해석하게 되는 kind.
//!
//! requiredness는 registry가 아니라 **kind code 자체의 최상위 bit**에 인코딩합니다
//! ([`ChunkKindCode::REQUIRED_BIT`]). reader가 모르는 kind라도 code만 보고
//! skip/오류를 판정할 수 있습니다(PNG ancillary bit와 같은 원리).

use std::collections::BTreeMap;
use std::fmt;

use super::envelope::ChunkKindCode;

impl ChunkKindCode {
    /// 최상위 bit: 1이면 required, 0이면 optional.
    pub const REQUIRED_BIT: u32 = 0x8000_0000;

    /// required kind code를 만듭니다.
    pub const fn required(value: u32) -> ChunkKindCode {
        ChunkKindCode(value | ChunkKindCode::REQUIRED_BIT)
    }

    /// optional kind code를 만듭니다.
    pub const fn optional(value: u32) -> ChunkKindCode {
        ChunkKindCode(value & !ChunkKindCode::REQUIRED_BIT)
    }

    /// 이 kind를 모르는 reader가 오류를 내야 하면 true.
    pub const fn is_required(self) -> bool {
        self.0 & ChunkKindCode::REQUIRED_BIT != 0
    }
}

/// registry 등록 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KindRegistryError {
    /// 같은 code가 이미 등록되어 있습니다.
    DuplicateCode {
        /// 중복 code.
        code: ChunkKindCode,
    },
    /// 같은 이름이 이미 등록되어 있습니다.
    DuplicateName {
        /// 중복 이름.
        name: String,
    },
}

impl fmt::Display for KindRegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KindRegistryError::DuplicateCode { code } => {
                write!(f, "kind code가 이미 등록되어 있습니다: {:#010x}", code.0)
            }
            KindRegistryError::DuplicateName { name } => {
                write!(f, "kind 이름이 이미 등록되어 있습니다: {name}")
            }
        }
    }
}

impl std::error::Error for KindRegistryError {}

/// kind code 해석 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KindResolution<'a> {
    /// 등록된 kind.
    Known {
        /// 등록된 이름.
        name: &'a str,
    },
    /// 모르는 optional kind — 호출자는 이 chunk를 건너뜁니다.
    SkipUnknownOptional,
}

/// unknown required kind 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownRequiredKind {
    /// 알 수 없는 required code.
    pub code: ChunkKindCode,
}

impl fmt::Display for UnknownRequiredKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "알 수 없는 required chunk kind입니다(신버전 문서일 수 있음): {:#010x}",
            self.code.0
        )
    }
}

impl std::error::Error for UnknownRequiredKind {}

/// chunk kind registry.
///
/// 초기 kind 목록 등록은 V.6.1에서 수행합니다.
#[derive(Debug, Default)]
pub struct ChunkKindRegistry {
    known: BTreeMap<ChunkKindCode, String>,
}

impl ChunkKindRegistry {
    /// 빈 registry를 만듭니다.
    pub fn new() -> ChunkKindRegistry {
        ChunkKindRegistry::default()
    }

    /// kind를 등록합니다. code·이름 중복은 거부합니다.
    pub fn register(
        &mut self,
        code: ChunkKindCode,
        name: impl Into<String>,
    ) -> Result<(), KindRegistryError> {
        let name = name.into();
        if self.known.contains_key(&code) {
            return Err(KindRegistryError::DuplicateCode { code });
        }
        if self.known.values().any(|n| *n == name) {
            return Err(KindRegistryError::DuplicateName { name });
        }
        self.known.insert(code, name);
        Ok(())
    }

    /// 등록된 kind 수.
    pub fn len(&self) -> usize {
        self.known.len()
    }

    /// 등록이 하나도 없는지 여부.
    pub fn is_empty(&self) -> bool {
        self.known.is_empty()
    }

    /// code를 해석합니다(version 정책):
    ///
    /// - 등록된 code → [`KindResolution::Known`].
    /// - 모르는 optional code → [`KindResolution::SkipUnknownOptional`].
    /// - 모르는 required code → [`UnknownRequiredKind`] 오류.
    pub fn resolve(&self, code: ChunkKindCode) -> Result<KindResolution<'_>, UnknownRequiredKind> {
        if let Some(name) = self.known.get(&code) {
            return Ok(KindResolution::Known { name });
        }
        if code.is_required() {
            return Err(UnknownRequiredKind { code });
        }
        Ok(KindResolution::SkipUnknownOptional)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> ChunkKindRegistry {
        let mut reg = ChunkKindRegistry::new();
        reg.register(ChunkKindCode::required(1), "test-required")
            .unwrap();
        reg.register(ChunkKindCode::optional(2), "test-optional")
            .unwrap();
        reg
    }

    #[test]
    fn required_bit_round_trip() {
        assert!(ChunkKindCode::required(7).is_required());
        assert!(!ChunkKindCode::optional(7).is_required());
        assert_eq!(ChunkKindCode::required(7).0, 0x8000_0007);
        assert_eq!(ChunkKindCode::optional(7).0, 0x0000_0007);
    }

    #[test]
    fn known_kinds_resolve_to_names() {
        let reg = registry();
        assert_eq!(
            reg.resolve(ChunkKindCode::required(1)).unwrap(),
            KindResolution::Known {
                name: "test-required"
            }
        );
        assert_eq!(
            reg.resolve(ChunkKindCode::optional(2)).unwrap(),
            KindResolution::Known {
                name: "test-optional"
            }
        );
    }

    #[test]
    fn unknown_optional_kind_is_skipped() {
        // 검증 기준(V.2.7): optional unknown fixture → skip.
        let reg = registry();
        assert_eq!(
            reg.resolve(ChunkKindCode::optional(999)).unwrap(),
            KindResolution::SkipUnknownOptional
        );
    }

    #[test]
    fn unknown_required_kind_is_error() {
        // 검증 기준(V.2.7): required unknown fixture → 오류.
        let reg = registry();
        let err = reg.resolve(ChunkKindCode::required(999)).unwrap_err();
        assert_eq!(
            err,
            UnknownRequiredKind {
                code: ChunkKindCode::required(999)
            }
        );
        assert!(err.to_string().contains("required"));
    }

    #[test]
    fn duplicate_code_and_name_are_rejected() {
        let mut reg = registry();
        assert_eq!(
            reg.register(ChunkKindCode::required(1), "another"),
            Err(KindRegistryError::DuplicateCode {
                code: ChunkKindCode::required(1)
            })
        );
        assert_eq!(
            reg.register(ChunkKindCode::optional(3), "test-optional"),
            Err(KindRegistryError::DuplicateName {
                name: "test-optional".to_string()
            })
        );
        // 실패한 등록은 registry를 바꾸지 않습니다.
        assert_eq!(reg.len(), 2);
    }

    #[test]
    fn same_value_required_and_optional_are_distinct_codes() {
        // bit 31이 code의 일부이므로 required(5)와 optional(5)는 다른 kind입니다.
        let mut reg = ChunkKindRegistry::new();
        reg.register(ChunkKindCode::required(5), "req-5").unwrap();
        reg.register(ChunkKindCode::optional(5), "opt-5").unwrap();
        assert_eq!(reg.len(), 2);
    }
}
