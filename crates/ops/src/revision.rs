//! 디스크 generation 기준 프로젝트 revision (XXIV.4.2).
//!
//! headless 도구에는 앱 창의 인메모리 [`DocumentRevision`](crate::core_types::revision::DocumentRevision)
//! 이 없습니다. 파일이 원본이므로(ADR-0005 §1) 프로젝트의 revision은 **current pointer가 가리키는
//! generation**입니다: generation 번호와 그 manifest의 content hash(ADR-0004). 같은 번호라도 다른
//! writer가 만든 generation은 hash가 다르므로 둘을 함께 비교합니다.
//!
//! 프로젝트 revision에는 `stellar.yaml`의 활성 기본값 세트 선택도 들어갑니다(ADR-0128 결정 2,
//! [`ProjectRevision::with_style_defaults`]). 세트만 바꾸는 쓰기는 generation을 쓰지 않으므로, 선택이 revision에 없으면
//! 다른 writer의 세트 변경을 base revision 비교가 보지 못합니다. 기본 세트(`css`)를 고른 프로젝트의 표기는 선택이
//! 없을 때와 같습니다.
//!
//! 문자열 표기(`token`)는 조회 결과에 넣고 쓰기 명령의 base revision 입력으로 다시 받습니다.
//!
//! | 상태 | 표기 |
//! |---|---|
//! | 저장된 generation 없음(current pointer 없음), 세트 `css` | `g0` |
//! | 저장됨 | `g<generation>:<content hash>` 예: `g3:sha256:<64 hex>` |
//! | 저장된 generation 없음, 세트가 `css`가 아님 | `g0:<content hash>` |

use std::fmt;
use std::str::FromStr;

use stellar_document::css::style_defaults::DEFAULT_SET;
use stellar_storage::hash::ContentHash;

/// 세트 선택을 revision hash에 더할 때 쓰는 영역 구분자(ADR-0128 결정 2).
const STYLE_DEFAULTS_DOMAIN: &[u8] = b"stellar.project-revision.style-defaults.v1\n";

/// 프로젝트 revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProjectRevision {
    /// 저장된 generation이 없습니다(새 scaffold처럼 current pointer가 없는 프로젝트).
    Unsaved,
    /// current pointer가 가리키는 generation(또는 원본에서 읽은 상태, 세트 선택을 더한 상태).
    Saved {
        /// generation 번호(1부터). 저장된 generation 없이 세트 선택만 더한 revision은 0입니다.
        generation: u64,
        /// revision hash. cache에서 읽은 기본 세트 프로젝트는 manifest content hash, 원본에서 읽으면 원본 revision
        /// hash(`snapshot` 모듈 문서 5번), 세트가 `css`가 아니면 그 hash에 선택을 더한 hash입니다.
        hash: ContentHash,
    },
}

impl ProjectRevision {
    /// generation 번호. 저장된 generation이 없으면 0.
    pub fn generation(&self) -> u64 {
        match self {
            ProjectRevision::Unsaved => 0,
            ProjectRevision::Saved { generation, .. } => *generation,
        }
    }

    /// revision hash([`ProjectRevision::Saved`]의 `hash`). [`ProjectRevision::Unsaved`]이면 `None`.
    pub fn hash(&self) -> Option<ContentHash> {
        match self {
            ProjectRevision::Unsaved => None,
            ProjectRevision::Saved { hash, .. } => Some(*hash),
        }
    }

    /// 원본·cache revision(`self`)에 `stellar.yaml` 활성 기본값 세트 선택(`selection`, 키가 없으면 `css`)을 더한
    /// 프로젝트 revision(ADR-0128 결정 2).
    ///
    /// - 선택이 기본 세트(`css`)이면 `self` 그대로입니다. 그래서 세트를 고르지 않은 프로젝트의 표기는 바뀌지 않고,
    ///   세트를 바꿨다가 `css`로 되돌리면 처음 revision과 같습니다(내용이 같은 상태).
    /// - 그 밖에는 generation 번호를 그대로 두고 hash를 `H(영역 구분자 ‖ 이전 hash 표기 ‖ 선택 이름)`으로 바꿉니다.
    ///   저장된 generation이 없으면 generation 0에 그 hash입니다(`g0:<hash>`).
    ///
    /// 결과에서 원래 revision을 되찾을 수 없습니다. 호출자는 원본·cache revision을 따로 들고 이 함수를 부릅니다.
    pub fn with_style_defaults(self, selection: &str) -> ProjectRevision {
        if selection == DEFAULT_SET {
            return self;
        }
        let mut bytes = STYLE_DEFAULTS_DOMAIN.to_vec();
        match self.hash() {
            Some(hash) => bytes.extend_from_slice(hash.to_string().as_bytes()),
            None => bytes.push(b'-'),
        }
        bytes.push(b'\n');
        bytes.extend_from_slice(selection.as_bytes());
        ProjectRevision::Saved {
            generation: self.generation(),
            hash: ContentHash::compute(&bytes),
        }
    }

    /// 문자열 표기(모듈 문서 표).
    pub fn token(&self) -> String {
        self.to_string()
    }
}

impl fmt::Display for ProjectRevision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProjectRevision::Unsaved => f.write_str("g0"),
            ProjectRevision::Saved { generation, hash } => write!(f, "g{generation}:{hash}"),
        }
    }
}

/// revision 표기 해석 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseRevisionError {
    /// `g`로 시작하지 않습니다.
    MissingPrefix,
    /// generation 번호가 10진 정수가 아니거나 범위를 넘습니다(선행 0 금지).
    InvalidGeneration,
    /// 저장된 revision에 content hash가 없거나 형식이 틀렸습니다.
    InvalidHash,
    /// 1 이상 generation에 hash가 없습니다.
    Inconsistent,
}

impl fmt::Display for ParseRevisionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseRevisionError::MissingPrefix => f.write_str("revision은 'g'로 시작해야 합니다"),
            ParseRevisionError::InvalidGeneration => {
                f.write_str("revision의 generation 번호가 올바른 10진 정수가 아닙니다")
            }
            ParseRevisionError::InvalidHash => {
                f.write_str("revision의 content hash 형식이 올바르지 않습니다")
            }
            ParseRevisionError::Inconsistent => {
                f.write_str("revision은 'g0' 또는 'g<generation>:<content hash>' 형태여야 합니다(1 이상 generation에는 hash가 있어야 함)")
            }
        }
    }
}

impl std::error::Error for ParseRevisionError {}

impl FromStr for ProjectRevision {
    type Err = ParseRevisionError;

    fn from_str(text: &str) -> Result<ProjectRevision, ParseRevisionError> {
        let rest = text
            .strip_prefix('g')
            .ok_or(ParseRevisionError::MissingPrefix)?;
        let (number, hash) = match rest.split_once(':') {
            Some((number, hash)) => (number, Some(hash)),
            None => (rest, None),
        };
        let canonical_number = number == "0"
            || (!number.is_empty()
                && !number.starts_with('0')
                && number.bytes().all(|b| b.is_ascii_digit()));
        if !canonical_number {
            return Err(ParseRevisionError::InvalidGeneration);
        }
        let generation: u64 = number
            .parse()
            .map_err(|_| ParseRevisionError::InvalidGeneration)?;
        match (generation, hash) {
            (0, None) => Ok(ProjectRevision::Unsaved),
            (_, None) => Err(ParseRevisionError::Inconsistent),
            // `g0:<hash>`는 저장된 generation 없이 세트 선택을 더한 revision입니다(ADR-0128 결정 2).
            (generation, Some(hash)) => {
                let hash = hash
                    .parse::<ContentHash>()
                    .map_err(|_| ParseRevisionError::InvalidHash)?;
                Ok(ProjectRevision::Saved { generation, hash })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_round_trips() {
        let saved = ProjectRevision::Saved {
            generation: 12,
            hash: ContentHash::compute(b"m"),
        };
        for revision in [ProjectRevision::Unsaved, saved] {
            let token = revision.token();
            assert_eq!(token.parse::<ProjectRevision>().unwrap(), revision);
        }
        assert_eq!(ProjectRevision::Unsaved.token(), "g0");
        assert_eq!(saved.token(), format!("g12:{}", ContentHash::compute(b"m")));
        assert_eq!(saved.generation(), 12);
        assert_eq!(ProjectRevision::Unsaved.hash(), None);
    }

    #[test]
    fn malformed_tokens_are_rejected() {
        let hash = ContentHash::compute(b"m").to_string();
        let cases = [
            ("", ParseRevisionError::MissingPrefix),
            ("3", ParseRevisionError::MissingPrefix),
            ("g", ParseRevisionError::InvalidGeneration),
            ("g03", ParseRevisionError::InvalidGeneration),
            ("g+3", ParseRevisionError::InvalidGeneration),
            (
                "g99999999999999999999",
                ParseRevisionError::InvalidGeneration,
            ),
            ("g3", ParseRevisionError::Inconsistent),
            ("g3:", ParseRevisionError::InvalidHash),
            ("g3:sha256:zz", ParseRevisionError::InvalidHash),
        ];
        for (text, expected) in cases {
            assert_eq!(text.parse::<ProjectRevision>(), Err(expected), "{text:?}");
        }
        // 저장 전 프로젝트에 세트 선택을 더한 revision은 `g0:<hash>`이고 다시 읽힙니다.
        let selected = ProjectRevision::Unsaved.with_style_defaults("react-native");
        assert_eq!(selected.generation(), 0);
        assert!(selected.token().starts_with("g0:sha256:"));
        assert_eq!(selected.token().parse::<ProjectRevision>(), Ok(selected));
        assert!(format!("g0:{hash}").parse::<ProjectRevision>().is_ok());
    }

    #[test]
    fn style_defaults_selection_changes_the_revision_except_for_the_default_set() {
        let saved = ProjectRevision::Saved {
            generation: 4,
            hash: ContentHash::compute(b"m"),
        };
        // 기본 세트는 표기를 바꾸지 않습니다(키가 없을 때와 같음).
        assert_eq!(saved.with_style_defaults("css"), saved);
        assert_eq!(
            ProjectRevision::Unsaved.with_style_defaults("css"),
            ProjectRevision::Unsaved
        );
        // 다른 세트는 generation 번호를 두고 hash만 바꿉니다. 세트마다, 바탕 revision마다 다릅니다.
        let rn = saved.with_style_defaults("react-native");
        let compact = saved.with_style_defaults("compact");
        assert_eq!(rn.generation(), 4);
        assert_ne!(rn, saved);
        assert_ne!(rn, compact);
        let other = ProjectRevision::Saved {
            generation: 4,
            hash: ContentHash::compute(b"n"),
        };
        assert_ne!(other.with_style_defaults("react-native"), rn);
        // 같은 입력은 같은 결과(결정론적)입니다.
        assert_eq!(saved.with_style_defaults("react-native"), rn);
        assert_ne!(
            ProjectRevision::Unsaved.with_style_defaults("react-native"),
            rn
        );
    }
}
