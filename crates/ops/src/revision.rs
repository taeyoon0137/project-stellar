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
//! 없을 때와 같습니다. 사용자 세트(`design/style-defaults/*`)를 고르면 그 세트를 평탄화한 값도 들어갑니다
//! ([`StyleDefaultsKey`], ADR-0135 결정 2). 세트 파일 편집은 생략값 해석을 바꾸지만 generation도 원본 digest도
//! 바꾸지 않기 때문입니다.
//!
//! revision의 hash([`ProjectRevision::hash`])는 manifest content hash가 아닐 수 있습니다. cache에서 읽은 기본 세트
//! 프로젝트만 manifest hash와 같고, 원본에서 읽으면 원본 revision hash, 세트를 고르면 그 위에 세트를 더한 hash입니다
//! (ADR-0135 결정 1). manifest hash가 필요한 비교(HEAD generation 재사용 판정)는 세트를 더하지 않은
//! `ProjectSnapshot::storage_revision`이나 `ProjectSnapshot::current_manifest`를 씁니다.
//!
//! 문자열 표기(`token`)는 조회 결과에 넣고 쓰기 명령의 base revision 입력으로 다시 받습니다.
//!
//! | 상태 | 표기 |
//! |---|---|
//! | 저장된 generation 없음(current pointer 없음), 세트 `css` | `g0` |
//! | 저장됨 | `g<generation>:<content hash>` 예: `g3:sha256:<64 hex>` |
//! | 저장된 generation 없음, 세트가 `css`가 아님 | `g0:<content hash>` |

use std::fmt;
use std::path::Path;
use std::str::FromStr;

use stellar_document::css::style_defaults::{is_builtin, NodeClass, UserSets, DEFAULT_SET};
use stellar_document::layout_record::{format_value, LayoutLonghand};
use stellar_project::manifest_style_defaults::{
    flatten_named, load_user_sets, StyleDefaultsFileError,
};
use stellar_storage::hash::ContentHash;

/// 세트 선택을 revision hash에 더할 때 쓰는 영역 구분자(ADR-0128 결정 2).
const STYLE_DEFAULTS_DOMAIN: &[u8] = b"stellar.project-revision.style-defaults.v1\n";

/// 사용자 세트의 평탄화 결과를 content hash로 줄일 때 쓰는 영역 구분자(ADR-0135 결정 2).
const USER_SET_DOMAIN: &[u8] = b"stellar.project-revision.user-set.v1\n";

/// revision에 더하는 활성 기본값 세트 상태(ADR-0128 결정 2, ADR-0135 결정 2).
///
/// - 내장 세트(`css`, `react-native`)는 이름뿐입니다. 값은 엔진 표가 정하고 프로젝트 파일이 바꾸지 못합니다.
/// - 사용자 세트는 이름과, 그 세트를 `extends` 사슬까지 평탄화한 값의 content hash입니다. 값의 뜻이 같으면(주석·
///   줄 순서·공백) hash가 같고, 생략값 해석을 바꾸는 편집이면 다릅니다. 사슬에 들지 않는 다른 세트 파일 편집은
///   hash를 바꾸지 않습니다.
/// - 세트 파일을 읽거나 평탄화하지 못하면 그 오류 code와 문장의 hash입니다. 오류 상태도 revision이 정해지고,
///   고치면 바뀝니다.
///
/// 앱 창([`StyleDefaultsKey::load`])과 headless snapshot(`ProjectSnapshot::style_defaults_key`)이 같은
/// [`StyleDefaultsKey::resolve`]를 거치므로 같은 디스크 상태에서 같은 값입니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyleDefaultsKey {
    name: String,
    /// 사용자 세트면 평탄화 결과(또는 오류)의 hash. 내장 세트면 `None`.
    content: Option<ContentHash>,
}

impl StyleDefaultsKey {
    /// `name` 세트를 프로젝트 사용자 세트 `sets`(읽기에 실패했으면 그 오류)로 푼 상태입니다. 내장 세트는 `sets`를
    /// 보지 않습니다. 평탄화는 적용 경로와 같은 `flatten_named`입니다.
    pub fn resolve(
        name: &str,
        sets: Result<&UserSets, &StyleDefaultsFileError>,
    ) -> StyleDefaultsKey {
        if is_builtin(name) {
            return StyleDefaultsKey {
                name: name.to_owned(),
                content: None,
            };
        }
        let mut bytes = USER_SET_DOMAIN.to_vec();
        let flat = sets
            .map_err(|error| (error.code(), error.to_string()))
            .and_then(|sets| {
                flatten_named(name, sets).map_err(|error| (error.code(), error.to_string()))
            });
        match flat {
            Ok(flat) => {
                bytes.extend_from_slice(b"chain");
                for link in flat.chain() {
                    bytes.push(b' ');
                    bytes.extend_from_slice(link.as_bytes());
                }
                bytes.push(b'\n');
                for class in NodeClass::ALL {
                    for longhand in LayoutLonghand::ALL {
                        let line = format!(
                            "{} {}: {}\n",
                            class.name(),
                            longhand.name(),
                            format_value(flat.value(class, longhand))
                        );
                        bytes.extend_from_slice(line.as_bytes());
                    }
                }
            }
            Err((code, message)) => {
                bytes.extend_from_slice(format!("error {code}\n{message}\n").as_bytes());
            }
        }
        StyleDefaultsKey {
            name: name.to_owned(),
            content: Some(ContentHash::compute(&bytes)),
        }
    }

    /// 프로젝트 `root`의 사용자 세트 파일을 지금 읽어 `name` 세트를 풉니다([`StyleDefaultsKey::resolve`]). 내장
    /// 세트면 파일을 읽지 않습니다. 세트 파일 내용을 따로 들고 있지 않은 앱 창이 씁니다.
    pub fn load(root: &Path, name: &str) -> StyleDefaultsKey {
        if is_builtin(name) {
            return StyleDefaultsKey::resolve(name, Ok(&UserSets::new()));
        }
        let sets = load_user_sets(root);
        StyleDefaultsKey::resolve(name, sets.as_ref())
    }

    /// 세트 이름.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// 사용자 세트의 평탄화 결과 hash(내장 세트면 `None`).
    pub fn content(&self) -> Option<ContentHash> {
        self.content
    }
}

/// 프로젝트 revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProjectRevision {
    /// 저장된 generation이 없습니다(새 scaffold처럼 current pointer가 없는 프로젝트).
    Unsaved,
    /// current pointer가 가리키는 generation(또는 원본에서 읽은 상태, 세트 선택을 더한 상태).
    Saved {
        /// generation 번호(1부터). 저장된 generation 없이 세트 선택만 더한 revision은 0입니다.
        generation: u64,
        /// revision hash(모듈 문서). cache에서 읽은 기본 세트 프로젝트만 manifest content hash와 같습니다. 원본에서
        /// 읽으면 원본 revision hash(`snapshot` 모듈 문서 5번), 세트가 `css`가 아니면 그 hash에 세트를 더한
        /// hash입니다.
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

    /// revision hash([`ProjectRevision::Saved`]의 `hash`, 모듈 문서). [`ProjectRevision::Unsaved`]이면 `None`.
    pub fn hash(&self) -> Option<ContentHash> {
        match self {
            ProjectRevision::Unsaved => None,
            ProjectRevision::Saved { hash, .. } => Some(*hash),
        }
    }

    /// 원본·cache revision(`self`)에 `stellar.yaml` 활성 기본값 세트 상태(`set`)를 더한 프로젝트
    /// revision(ADR-0128 결정 2, ADR-0135 결정 2).
    ///
    /// - 세트가 기본 세트(`css`)이면 `self` 그대로입니다. 그래서 세트를 고르지 않은 프로젝트의 표기는 바뀌지 않고,
    ///   세트를 바꿨다가 `css`로 되돌리면 처음 revision과 같습니다(내용이 같은 상태).
    /// - 그 밖에는 generation 번호를 그대로 두고 hash를 `H(영역 구분자 ‖ 이전 hash 표기 ‖ 줄바꿈 ‖ 세트 이름)`으로
    ///   바꿉니다. 사용자 세트면 그 뒤에 줄바꿈과 평탄화 결과 hash 표기를 더합니다(세트 이름에는 줄바꿈이 없음).
    ///   내장 세트의 결과는 ADR-0128 표기와 같습니다. 저장된 generation이 없으면 generation 0에 그 hash입니다
    ///   (`g0:<hash>`).
    ///
    /// 결과에서 원래 revision을 되찾을 수 없습니다. 호출자는 원본·cache revision을 따로 들고 이 함수를 부릅니다.
    pub fn with_style_defaults(self, set: &StyleDefaultsKey) -> ProjectRevision {
        if set.name == DEFAULT_SET {
            return self;
        }
        let mut bytes = STYLE_DEFAULTS_DOMAIN.to_vec();
        match self.hash() {
            Some(hash) => bytes.extend_from_slice(hash.to_string().as_bytes()),
            None => bytes.push(b'-'),
        }
        bytes.push(b'\n');
        bytes.extend_from_slice(set.name.as_bytes());
        if let Some(content) = set.content {
            bytes.push(b'\n');
            bytes.extend_from_slice(content.to_string().as_bytes());
        }
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
        let selected = ProjectRevision::Unsaved.with_style_defaults(&builtin("react-native"));
        assert_eq!(selected.generation(), 0);
        assert!(selected.token().starts_with("g0:sha256:"));
        assert_eq!(selected.token().parse::<ProjectRevision>(), Ok(selected));
        assert!(format!("g0:{hash}").parse::<ProjectRevision>().is_ok());
    }

    /// 사용자 세트 없이 푼 세트 상태.
    fn builtin(name: &str) -> StyleDefaultsKey {
        StyleDefaultsKey::resolve(name, Ok(&UserSets::new()))
    }

    /// 사용자 세트 파일 텍스트(이름, 내용)로 만든 세트 모음.
    fn user_sets(files: &[(&str, &str)]) -> UserSets {
        files
            .iter()
            .map(|(name, text)| {
                (
                    (*name).to_owned(),
                    stellar_project::manifest_style_defaults::parse_set_file(name, text).unwrap(),
                )
            })
            .collect()
    }

    #[test]
    fn style_defaults_selection_changes_the_revision_except_for_the_default_set() {
        let saved = ProjectRevision::Saved {
            generation: 4,
            hash: ContentHash::compute(b"m"),
        };
        // 기본 세트는 표기를 바꾸지 않습니다(키가 없을 때와 같음).
        assert_eq!(saved.with_style_defaults(&builtin("css")), saved);
        assert_eq!(
            ProjectRevision::Unsaved.with_style_defaults(&builtin("css")),
            ProjectRevision::Unsaved
        );
        // 다른 세트는 generation 번호를 두고 hash만 바꿉니다. 세트마다, 바탕 revision마다 다릅니다.
        let rn = saved.with_style_defaults(&builtin("react-native"));
        let compact = saved.with_style_defaults(&builtin("compact"));
        assert_eq!(rn.generation(), 4);
        assert_ne!(rn, saved);
        assert_ne!(rn, compact);
        let other = ProjectRevision::Saved {
            generation: 4,
            hash: ContentHash::compute(b"n"),
        };
        assert_ne!(other.with_style_defaults(&builtin("react-native")), rn);
        // 같은 입력은 같은 결과(결정론적)입니다.
        assert_eq!(saved.with_style_defaults(&builtin("react-native")), rn);
        assert_ne!(
            ProjectRevision::Unsaved.with_style_defaults(&builtin("react-native")),
            rn
        );
        // 내장 세트의 hash는 ADR-0128 표기 그대로입니다(영역 구분자 ‖ 이전 hash ‖ 줄바꿈 ‖ 이름).
        let mut bytes = STYLE_DEFAULTS_DOMAIN.to_vec();
        bytes.extend_from_slice(ContentHash::compute(b"m").to_string().as_bytes());
        bytes.extend_from_slice(b"\nreact-native");
        assert_eq!(rn.hash(), Some(ContentHash::compute(&bytes)));
    }

    #[test]
    fn user_set_content_changes_the_revision_only_when_its_flattened_values_change() {
        // ADR-0135 결정 2: 사용자 세트는 평탄화한 값까지 revision에 들어갑니다.
        let saved = ProjectRevision::Saved {
            generation: 2,
            hash: ContentHash::compute(b"m"),
        };
        let revision = |files: &[(&str, &str)]| {
            let sets = user_sets(files);
            saved.with_style_defaults(&StyleDefaultsKey::resolve("brand", Ok(&sets)))
        };
        let base = revision(&[(
            "brand",
            "extends: react-native\nflex-shrink: \"1\"\nrow-gap: 4px\n",
        )]);
        assert_ne!(base, saved);
        assert_eq!(base.generation(), 2);
        // 내장 세트 이름만 더한 값과 다릅니다(내용이 들어감).
        assert_ne!(base, saved.with_style_defaults(&builtin("react-native")));
        // 값을 바꾸면 revision이 바뀝니다.
        let changed = revision(&[(
            "brand",
            "extends: react-native\nflex-shrink: \"0\"\nrow-gap: 4px\n",
        )]);
        assert_ne!(changed, base);
        // 부모를 바꾸면(평탄화 결과가 달라짐) revision이 바뀝니다.
        let reparented = revision(&[("brand", "extends: css\nflex-shrink: \"1\"\nrow-gap: 4px\n")]);
        assert_ne!(reparented, base);
        // 뜻이 같은 편집(줄 순서)과 사슬 밖 세트 파일은 revision을 바꾸지 않습니다.
        assert_eq!(
            revision(&[
                (
                    "brand",
                    "extends: react-native\nrow-gap: 4px\nflex-shrink: \"1\"\n",
                ),
                ("other", "extends: css\nrow-gap: 4px\n"),
            ]),
            base
        );
        // 사슬 안의 다른 사용자 세트를 바꾸면 바뀝니다.
        let chained = |gap: &str| {
            revision(&[
                ("brand", "extends: base\nflex-shrink: \"1\"\n"),
                ("base", &format!("extends: react-native\nrow-gap: {gap}\n")),
            ])
        };
        assert_ne!(chained("4px"), chained("8px"));
        assert_eq!(chained("4px"), chained("4px"));
    }

    #[test]
    fn unresolvable_user_sets_still_give_a_deterministic_revision() {
        let saved = ProjectRevision::Saved {
            generation: 2,
            hash: ContentHash::compute(b"m"),
        };
        let unknown = StyleDefaultsKey::resolve("brand", Ok(&UserSets::new()));
        assert!(unknown.content().is_some());
        assert_eq!(unknown.name(), "brand");
        let cycle_sets = user_sets(&[("brand", "extends: loop\n"), ("loop", "extends: brand\n")]);
        let cycle = StyleDefaultsKey::resolve("brand", Ok(&cycle_sets));
        assert_ne!(unknown, cycle);
        assert_eq!(
            saved.with_style_defaults(&unknown),
            saved.with_style_defaults(&StyleDefaultsKey::resolve("brand", Ok(&UserSets::new())))
        );
        // 내장 세트는 사용자 세트를 읽지 못해도 이름뿐입니다.
        let error = StyleDefaultsFileError::Set(
            stellar_document::css::style_defaults::StyleDefaultsError::UnknownSet {
                name: "x".into(),
            },
        );
        assert_eq!(
            StyleDefaultsKey::resolve("react-native", Err(&error)),
            builtin("react-native")
        );
        assert_ne!(
            StyleDefaultsKey::resolve("brand", Err(&error)),
            StyleDefaultsKey::resolve("brand", Ok(&UserSets::new()))
        );
    }
}
