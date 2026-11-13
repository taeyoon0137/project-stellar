//! 프로젝트 폰트 라이선스 metadata (XIII.6.1)와 라이선스·임베딩 상태 (XIII.6.6).
//!
//! 사용자가 입력한 라이선스 정보를 그대로 기록합니다. Stellar는 이 값을 해석해 저장·임베딩
//! 가능 여부를 판정하지 않습니다(라이선스 URL도 열거나 내려받지 않습니다).
//!
//! 모든 field는 선택적입니다. 값이 있으면 비어 있지 않고 [`MAX_LICENSE_FIELD_BYTES`] 이하이며,
//! 제어 문자가 없습니다(메모 field만 줄바꿈 `\n`·탭 `\t` 허용).
//!
//! # 라이선스·임베딩 상태 (XIII.6.6)
//!
//! [`FontLicenseState`]는 `Unknown`, `Allowed`, `Restricted`, `UserConfirmed` 네 값입니다. 새 폰트는
//! 항상 `Unknown`으로 시작하고, **Stellar는 법적 판정을 자동으로 확정하지 않습니다.**
//!
//! - 상태 변경은 [`FontLicenseState::transition`]으로만 하며 변경 주체([`LicenseChangeOrigin`])를
//!   함께 받습니다.
//! - 사용자 변경(`User`)은 어떤 상태로도 바꿀 수 있습니다.
//! - 자동 변경(`Automatic` — import, 폰트 파일 metadata 분석, 동기화 등 사람의 명시 결정이 아닌
//!   경로)은 `Unknown`으로 되돌리는 것만 허용합니다. `Allowed`·`Restricted`·`UserConfirmed`로의 자동
//!   변경은 [`LicenseTransitionError::AutomaticDetermination`]으로 거부합니다.
//! - 폰트 파일이 선언한 OS/2 `fsType` 임베딩 권한(`crate::fonts::sfnt::EmbeddingPermissions`)은
//!   참고 정보일 뿐 상태를 바꾸지 않습니다.
//!
//! 상태 의미(사용자 기록): `Allowed` = 라이선스상 저장·임베딩 허용, `Restricted` = 제한,
//! `UserConfirmed` = 사용자가 별도 근거(계약 등)로 사용 권리를 직접 확인. 숫자 tag는 저장 포맷
//! 계약입니다.

use std::fmt;

/// 폰트 라이선스·임베딩 상태 (XIII.6.6).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FontLicenseState {
    /// 판정 없음(기본값).
    #[default]
    Unknown,
    /// 사용자가 저장·임베딩 허용으로 기록했습니다.
    Allowed,
    /// 사용자가 저장·임베딩 제한으로 기록했습니다.
    Restricted,
    /// 사용자가 사용 권리를 직접 확인했습니다.
    UserConfirmed,
}

impl FontLicenseState {
    /// 모든 상태(tag 오름차순).
    pub const ALL: [FontLicenseState; 4] = [
        FontLicenseState::Unknown,
        FontLicenseState::Allowed,
        FontLicenseState::Restricted,
        FontLicenseState::UserConfirmed,
    ];

    /// 안정 numeric tag(저장 포맷 계약).
    pub const fn tag(self) -> u8 {
        match self {
            FontLicenseState::Unknown => 1,
            FontLicenseState::Allowed => 2,
            FontLicenseState::Restricted => 3,
            FontLicenseState::UserConfirmed => 4,
        }
    }

    /// numeric tag에서 상태를 복원합니다. 알 수 없는 tag는 `None`.
    pub const fn from_tag(tag: u8) -> Option<FontLicenseState> {
        match tag {
            1 => Some(FontLicenseState::Unknown),
            2 => Some(FontLicenseState::Allowed),
            3 => Some(FontLicenseState::Restricted),
            4 => Some(FontLicenseState::UserConfirmed),
            _ => None,
        }
    }

    /// 진단·UI용 안정 이름.
    pub const fn name(self) -> &'static str {
        match self {
            FontLicenseState::Unknown => "unknown",
            FontLicenseState::Allowed => "allowed",
            FontLicenseState::Restricted => "restricted",
            FontLicenseState::UserConfirmed => "user-confirmed",
        }
    }

    /// 상태를 바꿉니다(모듈 문서의 규칙). 성공하면 새 상태를 돌려줍니다.
    pub fn transition(
        self,
        to: FontLicenseState,
        origin: LicenseChangeOrigin,
    ) -> Result<FontLicenseState, LicenseTransitionError> {
        match origin {
            LicenseChangeOrigin::User => Ok(to),
            LicenseChangeOrigin::Automatic if to == FontLicenseState::Unknown => Ok(to),
            LicenseChangeOrigin::Automatic => {
                Err(LicenseTransitionError::AutomaticDetermination { from: self, to })
            }
        }
    }
}

impl fmt::Display for FontLicenseState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// 라이선스 상태 변경 주체.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LicenseChangeOrigin {
    /// 사용자의 명시 결정(UI의 직접 선택).
    User,
    /// 사람의 명시 결정이 아닌 경로(import, 파일 metadata 분석, 동기화 등).
    Automatic,
}

/// 라이선스 상태 변경 거부.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LicenseTransitionError {
    /// 자동 경로가 판정 상태(`Unknown` 이외)를 확정하려 했습니다.
    AutomaticDetermination {
        /// 현재 상태.
        from: FontLicenseState,
        /// 요청 상태.
        to: FontLicenseState,
    },
}

impl fmt::Display for LicenseTransitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LicenseTransitionError::AutomaticDetermination { from, to } => write!(
                f,
                "폰트 라이선스 상태는 자동으로 확정할 수 없습니다: {from} → {to}(사용자 결정 필요)"
            ),
        }
    }
}

impl std::error::Error for LicenseTransitionError {}

/// 라이선스 field 하나의 최대 UTF-8 길이(byte).
pub const MAX_LICENSE_FIELD_BYTES: usize = 4096;

/// 라이선스 입력 field 종류(오류 보고용).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LicenseField {
    /// 라이선스 이름(예: `SIL Open Font License 1.1`).
    Name,
    /// 라이선스 문서 위치(URL 또는 문서 이름). Stellar는 열지 않습니다.
    Url,
    /// 저작권 표시.
    Copyright,
    /// 자유 메모.
    Notes,
}

impl LicenseField {
    /// 진단용 안정 이름.
    pub const fn name(self) -> &'static str {
        match self {
            LicenseField::Name => "license-name",
            LicenseField::Url => "license-url",
            LicenseField::Copyright => "copyright",
            LicenseField::Notes => "notes",
        }
    }
}

impl fmt::Display for LicenseField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// 라이선스 field 값 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LicenseFieldError {
    /// 빈 문자열입니다. 값이 없으면 `None`을 씁니다.
    Empty {
        /// field.
        field: LicenseField,
    },
    /// [`MAX_LICENSE_FIELD_BYTES`]를 넘습니다.
    TooLong {
        /// field.
        field: LicenseField,
        /// 실제 길이(byte).
        len: usize,
    },
    /// 허용하지 않는 제어 문자가 있습니다.
    ControlCharacter {
        /// field.
        field: LicenseField,
    },
}

impl fmt::Display for LicenseFieldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LicenseFieldError::Empty { field } => write!(f, "라이선스 {field} 값이 비었습니다"),
            LicenseFieldError::TooLong { field, len } => write!(
                f,
                "라이선스 {field} 값이 너무 깁니다: {len} bytes > {MAX_LICENSE_FIELD_BYTES} bytes"
            ),
            LicenseFieldError::ControlCharacter { field } => {
                write!(f, "라이선스 {field} 값에 제어 문자가 있습니다")
            }
        }
    }
}

impl std::error::Error for LicenseFieldError {}

/// field 값 하나를 검사합니다.
pub(crate) fn validate_license_field(
    field: LicenseField,
    value: &str,
) -> Result<(), LicenseFieldError> {
    if value.is_empty() {
        return Err(LicenseFieldError::Empty { field });
    }
    if value.len() > MAX_LICENSE_FIELD_BYTES {
        return Err(LicenseFieldError::TooLong {
            field,
            len: value.len(),
        });
    }
    let allows_layout = field == LicenseField::Notes;
    if value
        .chars()
        .any(|c| c.is_control() && !(allows_layout && matches!(c, '\n' | '\t')))
    {
        return Err(LicenseFieldError::ControlCharacter { field });
    }
    Ok(())
}

/// 프로젝트 폰트의 사용자 입력 라이선스 metadata와 라이선스·임베딩 상태.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct FontLicenseInfo {
    license_name: Option<String>,
    license_url: Option<String>,
    copyright: Option<String>,
    notes: Option<String>,
    state: FontLicenseState,
}

impl FontLicenseInfo {
    /// 모든 field가 비어 있고 상태가 `Unknown`인 값.
    pub fn new() -> FontLicenseInfo {
        FontLicenseInfo::default()
    }

    /// 라이선스·임베딩 상태 (XIII.6.6).
    pub fn state(&self) -> FontLicenseState {
        self.state
    }

    /// 상태를 바꿉니다. 거부되면 상태는 그대로입니다([`FontLicenseState::transition`]).
    pub fn change_state(
        &mut self,
        to: FontLicenseState,
        origin: LicenseChangeOrigin,
    ) -> Result<(), LicenseTransitionError> {
        self.state = self.state.transition(to, origin)?;
        Ok(())
    }

    /// field 값을 설정합니다(`None`이면 지웁니다).
    pub fn set(
        &mut self,
        field: LicenseField,
        value: Option<String>,
    ) -> Result<(), LicenseFieldError> {
        if let Some(value) = &value {
            validate_license_field(field, value)?;
        }
        *self.slot(field) = value;
        Ok(())
    }

    /// [`FontLicenseInfo::set`]의 builder 형태.
    pub fn with(
        mut self,
        field: LicenseField,
        value: impl Into<String>,
    ) -> Result<FontLicenseInfo, LicenseFieldError> {
        self.set(field, Some(value.into()))?;
        Ok(self)
    }

    /// field 값.
    pub fn get(&self, field: LicenseField) -> Option<&str> {
        match field {
            LicenseField::Name => self.license_name.as_deref(),
            LicenseField::Url => self.license_url.as_deref(),
            LicenseField::Copyright => self.copyright.as_deref(),
            LicenseField::Notes => self.notes.as_deref(),
        }
    }

    /// 입력된 값이 하나도 없고 상태가 `Unknown`이면 `true`.
    pub fn is_empty(&self) -> bool {
        self.license_name.is_none()
            && self.license_url.is_none()
            && self.copyright.is_none()
            && self.notes.is_none()
            && self.state == FontLicenseState::Unknown
    }

    fn slot(&mut self, field: LicenseField) -> &mut Option<String> {
        match field {
            LicenseField::Name => &mut self.license_name,
            LicenseField::Url => &mut self.license_url,
            LicenseField::Copyright => &mut self.copyright,
            LicenseField::Notes => &mut self.notes,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_are_recorded_as_entered() {
        let info = FontLicenseInfo::new()
            .with(LicenseField::Name, "SIL Open Font License 1.1")
            .unwrap()
            .with(LicenseField::Url, "https://openfontlicense.org")
            .unwrap()
            .with(LicenseField::Copyright, "Copyright 2026 Fixture Foundry")
            .unwrap()
            .with(
                LicenseField::Notes,
                "구매 영수증은 사내 위키에 보관\n\t담당: 디자인팀",
            )
            .unwrap();
        assert_eq!(
            info.get(LicenseField::Name),
            Some("SIL Open Font License 1.1")
        );
        assert_eq!(
            info.get(LicenseField::Url),
            Some("https://openfontlicense.org")
        );
        assert!(info.get(LicenseField::Notes).unwrap().contains('\n'));
        assert!(!info.is_empty());

        let mut cleared = info.clone();
        for field in [
            LicenseField::Name,
            LicenseField::Url,
            LicenseField::Copyright,
            LicenseField::Notes,
        ] {
            cleared.set(field, None).unwrap();
        }
        assert!(cleared.is_empty());
        assert_eq!(cleared, FontLicenseInfo::new());
    }

    #[test]
    fn invalid_values_are_rejected_without_change() {
        let mut info = FontLicenseInfo::new()
            .with(LicenseField::Name, "OFL")
            .unwrap();
        assert_eq!(
            info.set(LicenseField::Name, Some(String::new())),
            Err(LicenseFieldError::Empty {
                field: LicenseField::Name
            })
        );
        assert_eq!(
            info.set(LicenseField::Name, Some("a\nb".into())),
            Err(LicenseFieldError::ControlCharacter {
                field: LicenseField::Name
            })
        );
        assert_eq!(
            info.set(LicenseField::Notes, Some("a\u{7}b".into())),
            Err(LicenseFieldError::ControlCharacter {
                field: LicenseField::Notes
            })
        );
        let long = "x".repeat(MAX_LICENSE_FIELD_BYTES + 1);
        assert_eq!(
            info.set(LicenseField::Copyright, Some(long)),
            Err(LicenseFieldError::TooLong {
                field: LicenseField::Copyright,
                len: MAX_LICENSE_FIELD_BYTES + 1
            })
        );
        // 실패한 set은 기존 값을 바꾸지 않습니다.
        assert_eq!(info.get(LicenseField::Name), Some("OFL"));
        assert!(info
            .set(
                LicenseField::Copyright,
                Some("x".repeat(MAX_LICENSE_FIELD_BYTES))
            )
            .is_ok());
    }

    #[test]
    fn state_tags_are_stable_golden() {
        let tags: Vec<(u8, &str)> = FontLicenseState::ALL
            .iter()
            .map(|s| (s.tag(), s.name()))
            .collect();
        assert_eq!(
            tags,
            vec![
                (1, "unknown"),
                (2, "allowed"),
                (3, "restricted"),
                (4, "user-confirmed")
            ]
        );
        for state in FontLicenseState::ALL {
            assert_eq!(FontLicenseState::from_tag(state.tag()), Some(state));
        }
        assert_eq!(FontLicenseState::from_tag(0), None);
        assert_eq!(FontLicenseState::from_tag(5), None);
        assert_eq!(FontLicenseState::default(), FontLicenseState::Unknown);
        assert_eq!(FontLicenseInfo::new().state(), FontLicenseState::Unknown);
    }

    /// 상태 transition fixture: (현재, 요청, 주체) 32개 조합 전체.
    #[test]
    fn state_transition_fixture() {
        use FontLicenseState::{Allowed, Restricted, Unknown, UserConfirmed};
        use LicenseChangeOrigin::{Automatic, User};
        let mut checked = 0;
        for from in FontLicenseState::ALL {
            for to in FontLicenseState::ALL {
                // 사용자는 어떤 상태로도 바꿀 수 있습니다.
                assert_eq!(from.transition(to, User), Ok(to), "{from} → {to} (user)");
                // 자동 경로는 Unknown으로 되돌리는 것만 허용합니다.
                let automatic = from.transition(to, Automatic);
                if to == Unknown {
                    assert_eq!(automatic, Ok(Unknown), "{from} → {to} (automatic)");
                } else {
                    assert_eq!(
                        automatic,
                        Err(LicenseTransitionError::AutomaticDetermination { from, to }),
                        "{from} → {to} (automatic)"
                    );
                }
                checked += 2;
            }
        }
        assert_eq!(checked, 32);

        // 대표 경로: 사용자 기록 → 자동 판정 시도 거부(상태 유지) → 자동 초기화 → 사용자 확인.
        let mut info = FontLicenseInfo::new();
        info.change_state(Restricted, User).unwrap();
        assert_eq!(
            info.change_state(Allowed, Automatic),
            Err(LicenseTransitionError::AutomaticDetermination {
                from: Restricted,
                to: Allowed
            })
        );
        assert_eq!(info.state(), Restricted);
        assert!(!info.is_empty());
        info.change_state(Unknown, Automatic).unwrap();
        assert!(info.is_empty());
        info.change_state(UserConfirmed, User).unwrap();
        assert_eq!(info.state(), UserConfirmed);
        assert!(LicenseTransitionError::AutomaticDetermination {
            from: Unknown,
            to: Allowed
        }
        .to_string()
        .contains("사용자 결정"));
    }
}
