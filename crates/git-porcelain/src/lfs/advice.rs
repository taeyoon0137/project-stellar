//! import 후 Git LFS 권고 (XIII.7.2).
//!
//! 에셋 크기·저장 확장자 기준 LFS 권고 정책을 코드에 하드코딩하지 않고 config
//! ([`SHIPPED_CONFIG`], `crates/git-porcelain/config/lfs-advice.yaml`)에서 읽습니다.
//!
//! - **크기**: `size >= size_threshold_bytes`이면 권고합니다(경계값 포함, 1 byte 작으면 권고 없음).
//! - **확장자**: 저장 경로 마지막 component의 확장자(ASCII 대소문자 무시)가 목록에 있으면 크기와
//!   무관하게 권고합니다. 점으로 시작하는 이름(`.bin`)은 확장자가 없는 것으로 봅니다.
//! - **경고만**: 결과는 [`LfsAdvice`] 경고 목록입니다. `.gitattributes`·index·작업 트리를 바꾸지
//!   않습니다. LFS 추적 적용은 사용자 승인 API(XIII.7.3)만 합니다.
//! - [`advise_after_import`]는 이미 LFS로 추적되는 경로(`filter=lfs`)를 시스템 Git
//!   `check-attr`로 확인해 경고에서 뺍니다(`.gitattributes` 문법을 직접 해석하지 않음).

use std::collections::BTreeSet;
use std::path::Path;

use super::attr::{check_attribute, AttrError};
use crate::decode::GitPath;
use crate::trust::GatedGitRunner;

/// 출하 정책 config 원문(`crates/git-porcelain/config/lfs-advice.yaml`).
pub const SHIPPED_CONFIG: &str = include_str!("../../config/lfs-advice.yaml");

/// 정책 config 파일 최대 크기(byte). 이보다 크면 읽지 않습니다.
pub const MAX_CONFIG_BYTES: u64 = 64 * 1024;

/// 확장자 최대 길이(byte).
pub const MAX_EXTENSION_LEN: usize = 16;

const KEY_SIZE: &str = "size_threshold_bytes";
const KEY_EXTENSION: &str = "extension";

/// 정책 config 해석 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LfsPolicyConfigError {
    /// 필수 key가 없습니다.
    MissingKey {
        /// 없는 key.
        key: &'static str,
    },
    /// 한 번만 쓸 수 있는 key가 반복됨.
    DuplicateKey {
        /// key.
        key: &'static str,
        /// 1부터 시작하는 줄 번호.
        line: usize,
    },
    /// 알 수 없는 key.
    UnknownKey {
        /// key.
        key: String,
        /// 줄 번호.
        line: usize,
    },
    /// `key: value` 형식이 아님.
    MalformedLine {
        /// 줄 번호.
        line: usize,
    },
    /// 값 형식 오류(0 이하 크기, 잘못된 확장자 등).
    InvalidValue {
        /// key.
        key: &'static str,
        /// 줄 번호.
        line: usize,
    },
    /// 같은 확장자가 반복됨.
    DuplicateExtension {
        /// 확장자.
        extension: String,
        /// 줄 번호.
        line: usize,
    },
    /// config 파일이 [`MAX_CONFIG_BYTES`]보다 큼.
    TooLarge {
        /// 파일 크기.
        size: u64,
    },
    /// config 파일 읽기 실패.
    Io {
        /// io 오류 종류.
        kind: std::io::ErrorKind,
    },
}

impl std::fmt::Display for LfsPolicyConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LfsPolicyConfigError::MissingKey { key } => write!(f, "LFS 정책 key 없음: {key}"),
            LfsPolicyConfigError::DuplicateKey { key, line } => {
                write!(f, "LFS 정책 key 중복: {key} ({line}번째 줄)")
            }
            LfsPolicyConfigError::UnknownKey { key, line } => {
                write!(f, "알 수 없는 LFS 정책 key: {key:?} ({line}번째 줄)")
            }
            LfsPolicyConfigError::MalformedLine { line } => {
                write!(f, "LFS 정책 형식 오류({line}번째 줄)")
            }
            LfsPolicyConfigError::InvalidValue { key, line } => {
                write!(f, "LFS 정책 값 오류: {key} ({line}번째 줄)")
            }
            LfsPolicyConfigError::DuplicateExtension { extension, line } => {
                write!(f, "LFS 정책 확장자 중복: {extension} ({line}번째 줄)")
            }
            LfsPolicyConfigError::TooLarge { size } => {
                write!(f, "LFS 정책 config가 너무 큼: {size} bytes")
            }
            LfsPolicyConfigError::Io { kind } => write!(f, "LFS 정책 config 읽기 실패: {kind:?}"),
        }
    }
}

impl std::error::Error for LfsPolicyConfigError {}

/// LFS 권고 정책.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LfsAdvicePolicy {
    size_threshold_bytes: u64,
    extensions: BTreeSet<String>,
}

/// 권고 이유.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LfsAdviceReason {
    /// 크기가 임계값 이상.
    SizeAtLeast {
        /// 정책 임계값(byte).
        threshold: u64,
    },
    /// 정책 확장자와 일치(소문자).
    Extension {
        /// 일치한 확장자.
        extension: String,
    },
}

/// import된 에셋(저장 경로와 크기).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedAsset {
    /// worktree root 기준 저장 경로(예: `assets/<hash>.bin`).
    pub path: GitPath,
    /// content 크기(byte).
    pub size: u64,
}

/// LFS 권고 경고(변경 없음, 표시 전용).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LfsAdvice {
    /// 에셋 저장 경로.
    pub path: GitPath,
    /// content 크기(byte).
    pub size: u64,
    /// 권고 이유(크기 → 확장자 순).
    pub reasons: Vec<LfsAdviceReason>,
}

impl LfsAdvice {
    /// 고정 경고 코드.
    pub const CODE: &'static str = "lfs-recommended";

    /// 고정 경고 코드.
    pub fn code(&self) -> &'static str {
        Self::CODE
    }
}

/// import 후 권고 결과.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LfsAdviceReport {
    /// LFS 권고 경고(입력 순서).
    pub advice: Vec<LfsAdvice>,
    /// 정책에 걸렸지만 이미 `filter=lfs`로 추적되는 경로(경고 없음).
    pub already_tracked: Vec<GitPath>,
}

/// 확장자 형식: 소문자 ASCII 영숫자 1..=[`MAX_EXTENSION_LEN`].
fn valid_extension(ext: &str) -> bool {
    !ext.is_empty()
        && ext.len() <= MAX_EXTENSION_LEN
        && ext
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

/// 10진수 양의 정수(부호·공백·선행 0 없음).
fn parse_positive(value: &str) -> Option<u64> {
    if value.is_empty() || value.starts_with('0') || !value.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}

/// 경로 마지막 component의 확장자(소문자). 점으로 시작하는 이름·확장자 없는 이름은 `None`.
fn extension_of(path: &GitPath) -> Option<String> {
    let bytes = path.as_bytes();
    let name = bytes.rsplit(|b| *b == b'/').next()?;
    let dot = name.iter().rposition(|b| *b == b'.')?;
    if dot == 0 || dot + 1 == name.len() {
        return None;
    }
    let ext = std::str::from_utf8(&name[dot + 1..]).ok()?;
    Some(ext.to_ascii_lowercase())
}

impl LfsAdvicePolicy {
    /// config 원문을 해석합니다.
    pub fn from_config_str(text: &str) -> Result<LfsAdvicePolicy, LfsPolicyConfigError> {
        let mut size: Option<u64> = None;
        let mut extensions = BTreeSet::new();
        for (index, raw) in text.lines().enumerate() {
            let line = index + 1;
            let trimmed = raw.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let (key, value) = trimmed
                .split_once(':')
                .ok_or(LfsPolicyConfigError::MalformedLine { line })?;
            let (key, value) = (key.trim(), value.trim());
            match key {
                KEY_SIZE => {
                    if size.is_some() {
                        return Err(LfsPolicyConfigError::DuplicateKey {
                            key: KEY_SIZE,
                            line,
                        });
                    }
                    size = Some(parse_positive(value).ok_or(
                        LfsPolicyConfigError::InvalidValue {
                            key: KEY_SIZE,
                            line,
                        },
                    )?);
                }
                KEY_EXTENSION => {
                    if !valid_extension(value) {
                        return Err(LfsPolicyConfigError::InvalidValue {
                            key: KEY_EXTENSION,
                            line,
                        });
                    }
                    if !extensions.insert(value.to_string()) {
                        return Err(LfsPolicyConfigError::DuplicateExtension {
                            extension: value.to_string(),
                            line,
                        });
                    }
                }
                other => {
                    return Err(LfsPolicyConfigError::UnknownKey {
                        key: other.to_string(),
                        line,
                    })
                }
            }
        }
        Ok(LfsAdvicePolicy {
            size_threshold_bytes: size.ok_or(LfsPolicyConfigError::MissingKey { key: KEY_SIZE })?,
            extensions,
        })
    }

    /// config 파일을 읽어 해석합니다([`MAX_CONFIG_BYTES`] 초과 거부).
    pub fn from_config_file(path: &Path) -> Result<LfsAdvicePolicy, LfsPolicyConfigError> {
        use std::io::Read;
        let io = |e: std::io::Error| LfsPolicyConfigError::Io { kind: e.kind() };
        let file = std::fs::File::open(path).map_err(io)?;
        let size = file.metadata().map_err(io)?.len();
        if size > MAX_CONFIG_BYTES {
            return Err(LfsPolicyConfigError::TooLarge { size });
        }
        let mut text = String::new();
        // 읽는 도중 커져도 상한 + 1 byte까지만 읽습니다.
        file.take(MAX_CONFIG_BYTES + 1)
            .read_to_string(&mut text)
            .map_err(io)?;
        if text.len() as u64 > MAX_CONFIG_BYTES {
            return Err(LfsPolicyConfigError::TooLarge {
                size: text.len() as u64,
            });
        }
        LfsAdvicePolicy::from_config_str(&text)
    }

    /// 출하 config([`SHIPPED_CONFIG`])를 해석합니다.
    pub fn shipped() -> Result<LfsAdvicePolicy, LfsPolicyConfigError> {
        LfsAdvicePolicy::from_config_str(SHIPPED_CONFIG)
    }

    /// 크기 임계값(byte).
    pub fn size_threshold_bytes(&self) -> u64 {
        self.size_threshold_bytes
    }

    /// 권고 확장자(소문자, 정렬).
    pub fn extensions(&self) -> impl Iterator<Item = &str> {
        self.extensions.iter().map(String::as_str)
    }

    /// 에셋 하나를 판정합니다(순수 함수, Git 실행 없음).
    pub fn evaluate(&self, asset: &ImportedAsset) -> Option<LfsAdvice> {
        let mut reasons = Vec::new();
        if asset.size >= self.size_threshold_bytes {
            reasons.push(LfsAdviceReason::SizeAtLeast {
                threshold: self.size_threshold_bytes,
            });
        }
        if let Some(ext) = extension_of(&asset.path) {
            if self.extensions.contains(&ext) {
                reasons.push(LfsAdviceReason::Extension { extension: ext });
            }
        }
        if reasons.is_empty() {
            return None;
        }
        Some(LfsAdvice {
            path: asset.path.clone(),
            size: asset.size,
            reasons,
        })
    }
}

/// import된 에셋에 대한 LFS 권고 경고를 만듭니다(경고만, 변경 없음).
///
/// 정책에 걸린 경로만 `git check-attr filter`로 확인해 이미 `filter=lfs`인 경로는
/// [`LfsAdviceReport::already_tracked`]로 분리합니다. 정책에 걸린 경로가 없으면 Git을 실행하지
/// 않습니다. `worktree_root`는 저장소 worktree root(`--show-toplevel`)이고 경로는 그 기준입니다.
pub fn advise_after_import(
    git: &GatedGitRunner,
    worktree_root: &Path,
    policy: &LfsAdvicePolicy,
    assets: &[ImportedAsset],
) -> Result<LfsAdviceReport, AttrError> {
    let candidates: Vec<LfsAdvice> = assets.iter().filter_map(|a| policy.evaluate(a)).collect();
    if candidates.is_empty() {
        return Ok(LfsAdviceReport::default());
    }
    let paths: Vec<GitPath> = candidates.iter().map(|a| a.path.clone()).collect();
    let filters = check_attribute(git, worktree_root, "filter", &paths)?;
    let mut report = LfsAdviceReport::default();
    for (advice, (_, filter)) in candidates.into_iter().zip(filters) {
        if filter.is_value(b"lfs") {
            report.already_tracked.push(advice.path);
        } else {
            report.advice.push(advice);
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(path: &str, size: u64) -> ImportedAsset {
        ImportedAsset {
            path: GitPath::from_bytes(path.as_bytes()),
            size,
        }
    }

    fn policy(threshold: u64, exts: &[&str]) -> LfsAdvicePolicy {
        let mut text = format!("size_threshold_bytes: {threshold}\n");
        for e in exts {
            text.push_str(&format!("extension: {e}\n"));
        }
        LfsAdvicePolicy::from_config_str(&text).unwrap()
    }

    #[test]
    fn shipped_config_parses() {
        let shipped = LfsAdvicePolicy::shipped().unwrap();
        assert_eq!(shipped.size_threshold_bytes(), 10 * 1024 * 1024);
        assert_eq!(shipped.extensions().collect::<Vec<_>>(), vec!["bin"]);
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("config/lfs-advice.yaml");
        assert_eq!(LfsAdvicePolicy::from_config_file(&path).unwrap(), shipped);
    }

    #[test]
    fn threshold_boundary_fixture() {
        let p = policy(1000, &[]);
        for size in [0, 1, 998, 999] {
            assert_eq!(p.evaluate(&asset("assets/a.png", size)), None, "{size}");
        }
        for size in [1000, 1001, 2000, u64::MAX] {
            assert_eq!(
                p.evaluate(&asset("assets/a.png", size)).unwrap().reasons,
                vec![LfsAdviceReason::SizeAtLeast { threshold: 1000 }],
                "{size}"
            );
        }
        // 출하 임계값 경계.
        let shipped = LfsAdvicePolicy::shipped().unwrap();
        let t = shipped.size_threshold_bytes();
        assert_eq!(shipped.evaluate(&asset("assets/a.png", t - 1)), None);
        assert!(shipped.evaluate(&asset("assets/a.png", t)).is_some());
        assert!(shipped.evaluate(&asset("assets/a.png", t + 1)).is_some());
        // 임계값 1(최솟값): 빈 파일만 권고 없음.
        let one = policy(1, &[]);
        assert_eq!(one.evaluate(&asset("a.png", 0)), None);
        assert!(one.evaluate(&asset("a.png", 1)).is_some());
    }

    #[test]
    fn extension_matching_rules() {
        let p = policy(u64::MAX, &["bin", "psd"]);
        let ext = |e: &str| {
            vec![LfsAdviceReason::Extension {
                extension: e.into(),
            }]
        };
        assert_eq!(
            p.evaluate(&asset("assets/x.bin", 0)).unwrap().reasons,
            ext("bin")
        );
        assert_eq!(
            p.evaluate(&asset("assets/x.BIN", 0)).unwrap().reasons,
            ext("bin")
        );
        assert_eq!(
            p.evaluate(&asset("a/b/c.v2.psd", 3)).unwrap().reasons,
            ext("psd")
        );
        for no in [
            "assets/.bin",
            "assets/x.bin.png",
            "dir.bin/file",
            "assets/xbin",
            "assets/x.",
            "assets/x.bi",
        ] {
            assert_eq!(p.evaluate(&asset(no, 0)), None, "{no}");
        }
        // non-UTF8 이름도 panic 없이 확장자로 판정.
        let raw = ImportedAsset {
            path: GitPath::from_bytes(b"assets/\xff\xfe.bin"),
            size: 0,
        };
        assert!(p.evaluate(&raw).is_some());
        // 크기와 확장자가 모두 맞으면 크기 → 확장자 순.
        let both = policy(10, &["bin"]);
        let advice = both.evaluate(&asset("assets/a.bin", 10)).unwrap();
        assert_eq!(
            advice.reasons,
            vec![
                LfsAdviceReason::SizeAtLeast { threshold: 10 },
                LfsAdviceReason::Extension {
                    extension: "bin".into()
                }
            ]
        );
        assert_eq!(advice.code(), "lfs-recommended");
    }

    #[test]
    fn config_errors_are_explicit() {
        use LfsPolicyConfigError::*;
        let cases: Vec<(&str, LfsPolicyConfigError)> = vec![
            ("", MissingKey { key: KEY_SIZE }),
            ("extension: bin\n", MissingKey { key: KEY_SIZE }),
            (
                "size_threshold_bytes: 1\nsize_threshold_bytes: 2\n",
                DuplicateKey {
                    key: KEY_SIZE,
                    line: 2,
                },
            ),
            (
                "size_threshold_bytes: 1\nthreshold: 2\n",
                UnknownKey {
                    key: "threshold".into(),
                    line: 2,
                },
            ),
            ("size_threshold_bytes 1\n", MalformedLine { line: 1 }),
            (
                "size_threshold_bytes: 0\n",
                InvalidValue {
                    key: KEY_SIZE,
                    line: 1,
                },
            ),
            (
                "size_threshold_bytes: -1\n",
                InvalidValue {
                    key: KEY_SIZE,
                    line: 1,
                },
            ),
            (
                "size_threshold_bytes: +5\n",
                InvalidValue {
                    key: KEY_SIZE,
                    line: 1,
                },
            ),
            (
                "size_threshold_bytes: 010\n",
                InvalidValue {
                    key: KEY_SIZE,
                    line: 1,
                },
            ),
            (
                "size_threshold_bytes: 18446744073709551616\n",
                InvalidValue {
                    key: KEY_SIZE,
                    line: 1,
                },
            ),
            (
                "size_threshold_bytes: 1\nextension: .bin\n",
                InvalidValue {
                    key: KEY_EXTENSION,
                    line: 2,
                },
            ),
            (
                "size_threshold_bytes: 1\nextension: BIN\n",
                InvalidValue {
                    key: KEY_EXTENSION,
                    line: 2,
                },
            ),
            (
                "size_threshold_bytes: 1\nextension:\n",
                InvalidValue {
                    key: KEY_EXTENSION,
                    line: 2,
                },
            ),
            (
                "size_threshold_bytes: 1\nextension: abcdefghijklmnopq\n",
                InvalidValue {
                    key: KEY_EXTENSION,
                    line: 2,
                },
            ),
            (
                "size_threshold_bytes: 1\nextension: bin\nextension: bin\n",
                DuplicateExtension {
                    extension: "bin".into(),
                    line: 3,
                },
            ),
        ];
        for (text, expected) in cases {
            assert_eq!(
                LfsAdvicePolicy::from_config_str(text),
                Err(expected),
                "{text:?}"
            );
        }
        assert!(matches!(
            LfsAdvicePolicy::from_config_file(Path::new("/nonexistent/lfs-advice.yaml")),
            Err(Io { .. })
        ));
    }

    #[test]
    fn oversized_config_file_is_rejected() {
        let dir = crate::test_support::TempDir::new("lfs-advice-big");
        let path = dir.path().join("big.yaml");
        let mut text = String::from("size_threshold_bytes: 1\n");
        while (text.len() as u64) <= MAX_CONFIG_BYTES {
            text.push_str("# padding padding padding padding padding padding\n");
        }
        std::fs::write(&path, &text).unwrap();
        assert_eq!(
            LfsAdvicePolicy::from_config_file(&path),
            Err(LfsPolicyConfigError::TooLarge {
                size: text.len() as u64
            })
        );
    }

    #[test]
    fn no_candidates_spawns_no_git() {
        use crate::runner::GitRunner;
        use crate::trust::GitTrust;
        let runner = GitRunner::new(
            std::path::PathBuf::from("/nonexistent/git"),
            Vec::new(),
            std::time::Duration::from_secs(5),
        );
        let gate = GatedGitRunner::new(runner.clone(), GitTrust::Trusted);
        let report = advise_after_import(
            &gate,
            &std::env::temp_dir(),
            &policy(1000, &["bin"]),
            &[asset("assets/a.png", 10), asset("assets/b.svg", 999)],
        )
        .unwrap();
        assert_eq!(report, LfsAdviceReport::default());
        assert_eq!(runner.spawn_count(), 0);
    }

    #[cfg(unix)]
    mod real_git {
        use super::*;
        use crate::test_support::{
            commit_file, git_ok, hermetic_runner, init_repo_with_identity, system_git, TempDir,
        };
        use crate::trust::GitTrust;

        #[test]
        fn advice_is_warning_only_and_skips_lfs_tracked_paths() {
            let Some(git) = system_git() else {
                return; // git 미설치 환경 건너뜀.
            };
            let dir = TempDir::new("lfs-advice");
            let runner = hermetic_runner(git);
            let repo = dir.path();
            init_repo_with_identity(&runner, repo);
            let attributes = b"*.psd filter=lfs diff=lfs merge=lfs -text\n";
            commit_file(&runner, repo, ".gitattributes", attributes);
            std::fs::create_dir_all(repo.join("assets")).unwrap();
            std::fs::write(repo.join("assets/big.png"), vec![0u8; 2000]).unwrap();
            std::fs::write(repo.join("assets/tracked.psd"), vec![0u8; 2000]).unwrap();
            std::fs::write(repo.join("assets/blob.bin"), b"x").unwrap();
            std::fs::write(repo.join("assets/small.png"), b"x").unwrap();
            let status_before = git_ok(&runner, repo, ["status", "--porcelain=v2", "-z"]).stdout;

            let gate = GatedGitRunner::new(runner.clone(), GitTrust::Trusted);
            let report = advise_after_import(
                &gate,
                repo,
                &policy(1000, &["bin"]),
                &[
                    asset("assets/big.png", 2000),
                    asset("assets/tracked.psd", 2000),
                    asset("assets/blob.bin", 1),
                    asset("assets/small.png", 1),
                ],
            )
            .unwrap();
            let paths: Vec<&[u8]> = report.advice.iter().map(|a| a.path.as_bytes()).collect();
            assert_eq!(paths, vec![&b"assets/big.png"[..], b"assets/blob.bin"]);
            assert_eq!(
                report.already_tracked,
                vec![GitPath::from_bytes(b"assets/tracked.psd")]
            );

            // 경고만: .gitattributes·Git 상태 변화 없음.
            assert_eq!(
                std::fs::read(repo.join(".gitattributes")).unwrap(),
                attributes
            );
            let status_after = git_ok(&runner, repo, ["status", "--porcelain=v2", "-z"]).stdout;
            assert_eq!(status_before, status_after);
        }

        #[test]
        fn advice_without_gitattributes_does_not_create_it() {
            let Some(git) = system_git() else {
                return;
            };
            let dir = TempDir::new("lfs-advice-none");
            let runner = hermetic_runner(git);
            let repo = dir.path();
            init_repo_with_identity(&runner, repo);
            commit_file(&runner, repo, "stellar.yaml", b"v: 1\n");
            let gate = GatedGitRunner::new(runner, GitTrust::Trusted);
            let report = advise_after_import(
                &gate,
                repo,
                &LfsAdvicePolicy::shipped().unwrap(),
                &[asset("assets/a.bin", 1), asset("assets/b.png", 20 << 20)],
            )
            .unwrap();
            assert_eq!(report.advice.len(), 2);
            assert!(report.already_tracked.is_empty());
            assert!(!repo.join(".gitattributes").exists());
        }
    }
}
