//! 플랫폼 공통 폴더 이름 검증기 (IV.2.1).
//!
//! macOS와 Windows에서 모두 안전한 폴더 이름만 허용합니다. Windows가 가장 제약이 크므로 그
//! 규칙을 공통 하한으로 적용합니다: 예약 문자(`< > : " / \ | ? *`)·제어 문자, 예약 장치 이름
//! (CON/PRN/AUX/NUL/COM1-9/LPT1-9), trailing space/dot, `.`/`..`, 길이 상한.
//!
//! 대소문자 충돌은 [`collision_key`]로 case-insensitive 키를 만들어 판정합니다. 이름은 상위
//! 계층에서 이미 Unicode NFC로 정규화된 UTF-8이라고 가정합니다(정규화 경계는 III.3.4). NFC로
//! 정규화된 일반 Unicode 이름은 그대로 허용합니다.

/// 폴더 이름 길이 상한(문자 수). Windows 컴포넌트 한계(255 UTF-16)에 맞춘 보수적 값.
pub const MAX_FOLDER_NAME_LEN: usize = 255;

/// 폴더 이름 검증 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FolderNameError {
    /// 빈 이름.
    Empty,
    /// `.` 또는 `..`.
    DotName,
    /// 길이 상한 초과.
    TooLong {
        /// 실제 문자 수.
        len: usize,
        /// 허용 상한.
        max: usize,
    },
    /// 예약 문자 포함.
    ReservedChar {
        /// 문제된 문자.
        ch: char,
    },
    /// 제어 문자 포함.
    ControlChar,
    /// trailing space.
    TrailingSpace,
    /// trailing dot.
    TrailingDot,
    /// 예약 장치 이름(CON 등).
    ReservedName {
        /// 정규화한 예약 이름.
        name: String,
    },
}

impl std::fmt::Display for FolderNameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FolderNameError::Empty => f.write_str("폴더 이름은 비어 있을 수 없습니다"),
            FolderNameError::DotName => f.write_str("'.'·'..'는 폴더 이름으로 쓸 수 없습니다"),
            FolderNameError::TooLong { len, max } => {
                write!(f, "폴더 이름이 너무 깁니다: {len} > {max}")
            }
            FolderNameError::ReservedChar { ch } => {
                write!(f, "예약 문자를 포함합니다: {ch:?}")
            }
            FolderNameError::ControlChar => f.write_str("제어 문자를 포함합니다"),
            FolderNameError::TrailingSpace => f.write_str("이름이 공백으로 끝날 수 없습니다"),
            FolderNameError::TrailingDot => f.write_str("이름이 마침표로 끝날 수 없습니다"),
            FolderNameError::ReservedName { name } => {
                write!(f, "예약 장치 이름입니다: {name}")
            }
        }
    }
}

impl std::error::Error for FolderNameError {}

/// Windows 예약 문자 집합.
const RESERVED_CHARS: [char; 9] = ['<', '>', ':', '"', '/', '\\', '|', '?', '*'];

/// 폴더 이름 하나가 플랫폼 공통 규칙을 만족하는지 검증합니다.
pub fn validate_folder_name(name: &str) -> Result<(), FolderNameError> {
    if name.is_empty() {
        return Err(FolderNameError::Empty);
    }
    if name == "." || name == ".." {
        return Err(FolderNameError::DotName);
    }
    let len = name.chars().count();
    if len > MAX_FOLDER_NAME_LEN {
        return Err(FolderNameError::TooLong {
            len,
            max: MAX_FOLDER_NAME_LEN,
        });
    }
    for ch in name.chars() {
        if RESERVED_CHARS.contains(&ch) {
            return Err(FolderNameError::ReservedChar { ch });
        }
        if ch.is_control() {
            return Err(FolderNameError::ControlChar);
        }
    }
    if name.ends_with(' ') {
        return Err(FolderNameError::TrailingSpace);
    }
    if name.ends_with('.') {
        return Err(FolderNameError::TrailingDot);
    }
    if let Some(reserved) = reserved_device_name(name) {
        return Err(FolderNameError::ReservedName { name: reserved });
    }
    Ok(())
}

/// 예약 장치 이름이면 정규화된 이름(대문자)을, 아니면 `None`을 반환합니다.
///
/// Windows는 확장자가 붙어도 예약합니다(`CON.txt`도 예약). 첫 `.` 앞 basename으로 판정합니다.
fn reserved_device_name(name: &str) -> Option<String> {
    let base = name.split('.').next().unwrap_or(name);
    if base.is_empty() {
        return None; // dotfile(`.gitignore`)은 basename이 비어 있어 예약 아님.
    }
    let upper = base.to_ascii_uppercase();
    let reserved = matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL") || is_com_or_lpt(&upper);
    if reserved {
        Some(upper)
    } else {
        None
    }
}

/// `COM1`~`COM9` 또는 `LPT1`~`LPT9`인지 판정합니다(대문자 가정).
fn is_com_or_lpt(upper: &str) -> bool {
    let rest = upper
        .strip_prefix("COM")
        .or_else(|| upper.strip_prefix("LPT"));
    match rest {
        Some(digits) => digits.len() == 1 && matches!(digits.as_bytes()[0], b'1'..=b'9'),
        None => false,
    }
}

/// 대소문자 무시 충돌 비교용 키. 이름이 NFC로 정규화돼 있다고 가정하고 case-fold만 적용합니다.
pub fn collision_key(name: &str) -> String {
    name.to_lowercase()
}

/// 이름 목록에 대소문자 무시 충돌이 있는지 검사합니다.
pub fn has_case_collision(names: &[&str]) -> bool {
    let mut seen = std::collections::HashSet::new();
    for name in names {
        if !seen.insert(collision_key(name)) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid_names() {
        for name in [
            "my-folder",
            "design_2024",
            ".gitignore",
            "Project Stellar",
            "a.b.c",
            "v2.4.0", // trailing이 dot이 아니므로 OK
        ] {
            assert_eq!(validate_folder_name(name), Ok(()), "이름: {name:?}");
        }
    }

    #[test]
    fn accepts_nfc_unicode_names() {
        // NFC로 정규화된 일반 Unicode 이름은 허용.
        for name in [
            "café", // U+00E9 (NFC 합성형)
            "日本語",
            "Ünnaturen",
            "Москва",
            "그래픽-자산",
        ] {
            assert_eq!(validate_folder_name(name), Ok(()), "이름: {name:?}");
        }
    }

    #[test]
    fn rejects_empty_and_dot_names() {
        assert_eq!(validate_folder_name(""), Err(FolderNameError::Empty));
        assert_eq!(validate_folder_name("."), Err(FolderNameError::DotName));
        assert_eq!(validate_folder_name(".."), Err(FolderNameError::DotName));
    }

    #[test]
    fn rejects_reserved_chars() {
        for ch in RESERVED_CHARS {
            let name = format!("a{ch}b");
            assert_eq!(
                validate_folder_name(&name),
                Err(FolderNameError::ReservedChar { ch }),
                "문자: {ch:?}"
            );
        }
    }

    #[test]
    fn rejects_control_chars() {
        assert_eq!(
            validate_folder_name("a\tb"),
            Err(FolderNameError::ControlChar)
        );
        assert_eq!(
            validate_folder_name("a\u{0001}b"),
            Err(FolderNameError::ControlChar)
        );
    }

    #[test]
    fn rejects_trailing_space_and_dot() {
        assert_eq!(
            validate_folder_name("name "),
            Err(FolderNameError::TrailingSpace)
        );
        assert_eq!(
            validate_folder_name("name."),
            Err(FolderNameError::TrailingDot)
        );
    }

    #[test]
    fn rejects_reserved_device_names() {
        for name in [
            "CON", "con", "Con", "PRN", "AUX", "NUL", "nul", "COM1", "com9", "LPT1", "lpt9",
            "CON.txt", "nul.dat",
        ] {
            assert!(
                matches!(
                    validate_folder_name(name),
                    Err(FolderNameError::ReservedName { .. })
                ),
                "이름: {name:?}"
            );
        }
        // COM0/LPT0/COM10은 예약이 아님.
        assert_eq!(validate_folder_name("COM0"), Ok(()));
        assert_eq!(validate_folder_name("LPT0"), Ok(()));
        assert_eq!(validate_folder_name("COM10"), Ok(()));
        // CON을 포함하지만 예약 basename이 아닌 이름.
        assert_eq!(validate_folder_name("CONfig"), Ok(()));
    }

    #[test]
    fn rejects_too_long() {
        let long = "a".repeat(MAX_FOLDER_NAME_LEN + 1);
        assert_eq!(
            validate_folder_name(&long),
            Err(FolderNameError::TooLong {
                len: MAX_FOLDER_NAME_LEN + 1,
                max: MAX_FOLDER_NAME_LEN,
            })
        );
        // 경계: 정확히 상한은 허용.
        assert_eq!(
            validate_folder_name(&"a".repeat(MAX_FOLDER_NAME_LEN)),
            Ok(())
        );
    }

    #[test]
    fn case_collision_detection() {
        assert!(has_case_collision(&["Docs", "docs"]));
        assert!(has_case_collision(&["a", "b", "A"]));
        assert!(!has_case_collision(&["a", "b", "c"]));
        // NFC Unicode + 대소문자 충돌.
        assert!(has_case_collision(&["Café", "café"]));
        assert_eq!(collision_key("Café"), collision_key("café"));
    }
}
