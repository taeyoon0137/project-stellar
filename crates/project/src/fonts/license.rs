//! 프로젝트 폰트 라이선스 metadata (XIII.6.1).
//!
//! 사용자가 입력한 라이선스 정보를 그대로 기록합니다. Stellar는 이 값을 해석해 저장·임베딩
//! 가능 여부를 판정하지 않습니다(라이선스 URL도 열거나 내려받지 않습니다).
//!
//! 모든 field는 선택적입니다. 값이 있으면 비어 있지 않고 [`MAX_LICENSE_FIELD_BYTES`] 이하이며,
//! 제어 문자가 없습니다(메모 field만 줄바꿈 `\n`·탭 `\t` 허용).

use std::fmt;

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

/// 프로젝트 폰트의 사용자 입력 라이선스 metadata.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct FontLicenseInfo {
    license_name: Option<String>,
    license_url: Option<String>,
    copyright: Option<String>,
    notes: Option<String>,
}

impl FontLicenseInfo {
    /// 모든 field가 비어 있는 값.
    pub fn new() -> FontLicenseInfo {
        FontLicenseInfo::default()
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

    /// 입력된 값이 하나도 없으면 `true`.
    pub fn is_empty(&self) -> bool {
        self.license_name.is_none()
            && self.license_url.is_none()
            && self.copyright.is_none()
            && self.notes.is_none()
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
}
