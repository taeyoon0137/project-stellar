//! 디스크 generation 기준 프로젝트 revision (XXIV.4.2).
//!
//! headless 도구에는 앱 창의 인메모리 [`DocumentRevision`](crate::core_types::revision::DocumentRevision)
//! 이 없습니다. 파일이 원본이므로(ADR-0005 §1) 프로젝트의 revision은 **current pointer가 가리키는
//! generation**입니다: generation 번호와 그 manifest의 content hash(ADR-0004). 같은 번호라도 다른
//! writer가 만든 generation은 hash가 다르므로 둘을 함께 비교합니다.
//!
//! 문자열 표기(`token`)는 조회 결과에 넣고 쓰기 명령의 base revision 입력으로 다시 받습니다.
//!
//! | 상태 | 표기 |
//! |---|---|
//! | 저장된 generation 없음(current pointer 없음) | `g0` |
//! | 저장됨 | `g<generation>:<content hash>` 예: `g3:sha256:<64 hex>` |

use std::fmt;
use std::str::FromStr;

use stellar_storage::hash::ContentHash;

/// 프로젝트 revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProjectRevision {
    /// 저장된 generation이 없습니다(새 scaffold처럼 current pointer가 없는 프로젝트).
    Unsaved,
    /// current pointer가 가리키는 generation.
    Saved {
        /// generation 번호(1부터).
        generation: u64,
        /// manifest content hash.
        manifest_hash: ContentHash,
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

    /// manifest content hash. 저장된 generation이 없으면 `None`.
    pub fn manifest_hash(&self) -> Option<ContentHash> {
        match self {
            ProjectRevision::Unsaved => None,
            ProjectRevision::Saved { manifest_hash, .. } => Some(*manifest_hash),
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
            ProjectRevision::Saved {
                generation,
                manifest_hash,
            } => write!(f, "g{generation}:{manifest_hash}"),
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
    /// `g0`에 hash가 붙었거나, 1 이상 generation에 hash가 없습니다.
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
                f.write_str("revision은 'g0' 또는 'g<generation>:<content hash>' 형태여야 합니다")
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
            (0, Some(_)) | (_, None) => Err(ParseRevisionError::Inconsistent),
            (generation, Some(hash)) => {
                let manifest_hash = hash
                    .parse::<ContentHash>()
                    .map_err(|_| ParseRevisionError::InvalidHash)?;
                Ok(ProjectRevision::Saved {
                    generation,
                    manifest_hash,
                })
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
            manifest_hash: ContentHash::compute(b"m"),
        };
        for revision in [ProjectRevision::Unsaved, saved] {
            let token = revision.token();
            assert_eq!(token.parse::<ProjectRevision>().unwrap(), revision);
        }
        assert_eq!(ProjectRevision::Unsaved.token(), "g0");
        assert_eq!(saved.token(), format!("g12:{}", ContentHash::compute(b"m")));
        assert_eq!(saved.generation(), 12);
        assert_eq!(ProjectRevision::Unsaved.manifest_hash(), None);
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
        assert_eq!(
            format!("g0:{hash}").parse::<ProjectRevision>(),
            Err(ParseRevisionError::Inconsistent)
        );
    }
}
