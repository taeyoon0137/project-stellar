//! 에셋 import 정책 (XIII.5.7).
//!
//! 허용 media type(과 저장 확장자), 최대 단일 파일 크기, unknown type 처리를 코드에
//! 하드코딩하지 않고 `config/asset-import.yaml`에서 읽습니다.
//!
//! - **크기**: `size <= max_file_size_bytes`만 허용합니다(경계값 포함, 1 byte 초과 거부).
//!   import는 원본 메타데이터 크기로 먼저 거부하고, 읽는 도중 파일이 커져도 상한을 넘는
//!   순간 멈춥니다([`ImportError::TooLarge`]) — 아무 것도 쓰지 않습니다.
//! - **허용 목록**: 목록의 media type은 그 type 그대로, 표의 확장자로 저장합니다(원본 파일
//!   확장자와 무관 — 같은 content는 media type이 같으면 같은 경로).
//! - **unknown type**: `store_as_binary`면 binary asset(`application/octet-stream`, 확장자
//!   `bin`)으로 저장하고([`ImportDecision::fallback_to_binary`]), `reject`면 거부합니다.
//! - **강제**: crate 밖에서 import 요청([`ImportRequest`])은 [`AssetImportPolicy::request`]로만
//!   만들 수 있고, `Update from Source`도 정책 인자를 요구합니다. 정책 없는 요청·확장자/크기
//!   덮어쓰기는 crate 내부 전용입니다(opt-in이 아님).

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use crate::media_type::MediaType;
use crate::path::SafeExtension;
use crate::store::ImportRequest;

/// binary asset fallback media type.
pub const BINARY_MEDIA_TYPE: &str = "application/octet-stream";

/// 허용 목록 밖 media type 처리.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnknownMediaTypePolicy {
    /// binary asset(`application/octet-stream`, `bin`)으로 저장합니다.
    StoreAsBinary,
    /// import를 거부합니다.
    Reject,
}

/// 정책 config 파싱 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyConfigError {
    /// 필수 key가 없습니다.
    MissingKey {
        /// 없는 key.
        key: &'static str,
    },
    /// 단일 값 key가 두 번 나옵니다.
    DuplicateKey {
        /// 중복 key.
        key: &'static str,
    },
    /// 값이 잘못되었습니다.
    InvalidValue {
        /// 해당 key.
        key: &'static str,
        /// 원본 값.
        value: String,
    },
    /// 알 수 없는 key(또는 `key: value` 형식이 아닌 줄)입니다.
    UnknownKey {
        /// 해당 key 또는 줄.
        key: String,
    },
    /// 같은 media type이 두 번 나옵니다(정규화 후 비교).
    DuplicateMediaType {
        /// 중복 media type.
        media_type: String,
    },
    /// 파일을 읽을 수 없습니다.
    Io {
        /// 원인 메시지.
        message: String,
    },
}

impl fmt::Display for PolicyConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PolicyConfigError::MissingKey { key } => {
                write!(f, "asset-import config에 {key}가 없습니다")
            }
            PolicyConfigError::DuplicateKey { key } => {
                write!(f, "asset-import config에 {key}가 두 번 있습니다")
            }
            PolicyConfigError::InvalidValue { key, value } => {
                write!(
                    f,
                    "asset-import config {key} 값이 잘못되었습니다: {value:?}"
                )
            }
            PolicyConfigError::UnknownKey { key } => {
                write!(f, "asset-import config에 알 수 없는 key: {key}")
            }
            PolicyConfigError::DuplicateMediaType { media_type } => {
                write!(
                    f,
                    "asset-import config에 media type이 중복됩니다: {media_type}"
                )
            }
            PolicyConfigError::Io { message } => write!(f, "config 읽기 실패: {message}"),
        }
    }
}

impl std::error::Error for PolicyConfigError {}

/// 정책 위반.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyViolation {
    /// 허용 목록 밖 media type이고 unknown 정책이 `reject`입니다.
    MediaTypeNotAllowed {
        /// 요청 media type.
        media_type: MediaType,
    },
    /// 최대 단일 파일 크기를 넘습니다.
    FileTooLarge {
        /// 크기(byte).
        size: u64,
        /// 허용 최대 크기(byte).
        max: u64,
    },
}

impl fmt::Display for PolicyViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PolicyViolation::MediaTypeNotAllowed { media_type } => {
                write!(f, "import가 허용되지 않은 media type입니다: {media_type}")
            }
            PolicyViolation::FileTooLarge { size, max } => {
                write!(
                    f,
                    "최대 단일 파일 크기를 넘습니다: {size} bytes > {max} bytes"
                )
            }
        }
    }
}

impl std::error::Error for PolicyViolation {}

/// media type 판정 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportDecision {
    /// record에 기록할 media type.
    pub media_type: MediaType,
    /// 저장 확장자.
    pub extension: SafeExtension,
    /// unknown type을 binary asset으로 바꿔 저장하면 `true`.
    pub fallback_to_binary: bool,
}

/// 에셋 import 정책.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetImportPolicy {
    max_file_size_bytes: u64,
    unknown_media_type: UnknownMediaTypePolicy,
    allowed: BTreeMap<MediaType, SafeExtension>,
    /// binary fallback media type(파싱 시 한 번 검증해 보관 — 프로덕션 경로 panic 없음).
    binary: MediaType,
}

impl AssetImportPolicy {
    /// `key: value` 한 줄 형식의 config 텍스트에서 정책을 읽습니다.
    pub fn from_config_str(text: &str) -> Result<AssetImportPolicy, PolicyConfigError> {
        let mut max: Option<u64> = None;
        let mut unknown: Option<UnknownMediaTypePolicy> = None;
        let mut allowed: BTreeMap<MediaType, SafeExtension> = BTreeMap::new();
        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once(':') else {
                return Err(PolicyConfigError::UnknownKey {
                    key: line.to_string(),
                });
            };
            let value = value.trim();
            match key.trim() {
                "max_file_size_bytes" => {
                    let key = "max_file_size_bytes";
                    if max.is_some() {
                        return Err(PolicyConfigError::DuplicateKey { key });
                    }
                    let parsed = value.parse::<u64>().ok().filter(|v| *v >= 1).ok_or(
                        PolicyConfigError::InvalidValue {
                            key,
                            value: value.to_string(),
                        },
                    )?;
                    max = Some(parsed);
                }
                "unknown_media_type" => {
                    let key = "unknown_media_type";
                    if unknown.is_some() {
                        return Err(PolicyConfigError::DuplicateKey { key });
                    }
                    unknown = Some(match value {
                        "store_as_binary" => UnknownMediaTypePolicy::StoreAsBinary,
                        "reject" => UnknownMediaTypePolicy::Reject,
                        other => {
                            return Err(PolicyConfigError::InvalidValue {
                                key,
                                value: other.to_string(),
                            })
                        }
                    });
                }
                "media_type" => {
                    let invalid = || PolicyConfigError::InvalidValue {
                        key: "media_type",
                        value: value.to_string(),
                    };
                    let mut parts = value.split_whitespace();
                    let (Some(media), Some(ext), None) = (parts.next(), parts.next(), parts.next())
                    else {
                        return Err(invalid());
                    };
                    let media = MediaType::new(media).map_err(|_| invalid())?;
                    let ext = SafeExtension::new(ext).map_err(|_| invalid())?;
                    if allowed.contains_key(&media) {
                        return Err(PolicyConfigError::DuplicateMediaType {
                            media_type: media.to_string(),
                        });
                    }
                    allowed.insert(media, ext);
                }
                other => {
                    return Err(PolicyConfigError::UnknownKey {
                        key: other.to_string(),
                    })
                }
            }
        }
        if allowed.is_empty() {
            return Err(PolicyConfigError::MissingKey { key: "media_type" });
        }
        let binary =
            MediaType::new(BINARY_MEDIA_TYPE).map_err(|_| PolicyConfigError::InvalidValue {
                key: "unknown_media_type",
                value: BINARY_MEDIA_TYPE.to_string(),
            })?;
        Ok(AssetImportPolicy {
            binary,
            max_file_size_bytes: max.ok_or(PolicyConfigError::MissingKey {
                key: "max_file_size_bytes",
            })?,
            unknown_media_type: unknown.ok_or(PolicyConfigError::MissingKey {
                key: "unknown_media_type",
            })?,
            allowed,
        })
    }

    /// config 파일에서 정책을 읽습니다.
    pub fn from_config_file(path: &Path) -> Result<AssetImportPolicy, PolicyConfigError> {
        let text = std::fs::read_to_string(path).map_err(|e| PolicyConfigError::Io {
            message: format!("{}: {e}", path.display()),
        })?;
        AssetImportPolicy::from_config_str(&text)
    }

    /// 최대 단일 파일 크기(byte, 이 값까지 허용).
    pub fn max_file_size_bytes(&self) -> u64 {
        self.max_file_size_bytes
    }

    /// unknown media type 처리.
    pub fn unknown_media_type(&self) -> UnknownMediaTypePolicy {
        self.unknown_media_type
    }

    /// 허용 목록 media type의 저장 확장자.
    pub fn extension_for(&self, media_type: &MediaType) -> Option<&SafeExtension> {
        self.allowed.get(media_type)
    }

    /// 허용 media type 목록(정렬 순).
    pub fn allowed_media_types(&self) -> impl Iterator<Item = &MediaType> {
        self.allowed.keys()
    }

    /// 크기 판정(`size <= max`만 허용).
    pub fn check_size(&self, size: u64) -> Result<(), PolicyViolation> {
        if size > self.max_file_size_bytes {
            return Err(PolicyViolation::FileTooLarge {
                size,
                max: self.max_file_size_bytes,
            });
        }
        Ok(())
    }

    /// media type 판정.
    pub fn decide(&self, declared: &MediaType) -> Result<ImportDecision, PolicyViolation> {
        if let Some(extension) = self.allowed.get(declared) {
            return Ok(ImportDecision {
                media_type: declared.clone(),
                extension: extension.clone(),
                fallback_to_binary: false,
            });
        }
        match self.unknown_media_type {
            UnknownMediaTypePolicy::StoreAsBinary => Ok(ImportDecision {
                media_type: self.binary.clone(),
                extension: SafeExtension::binary(),
                fallback_to_binary: true,
            }),
            UnknownMediaTypePolicy::Reject => Err(PolicyViolation::MediaTypeNotAllowed {
                media_type: declared.clone(),
            }),
        }
    }

    /// 정책을 적용한 import 요청(판정된 media type·확장자, 최대 크기)을 만듭니다.
    ///
    /// 크기는 파일을 열 때 검사되므로 여기서는 media type만 거부될 수 있습니다.
    pub fn request(
        &self,
        source: impl Into<PathBuf>,
        declared: &MediaType,
    ) -> Result<ImportRequest, PolicyViolation> {
        let decision = self.decide(declared)?;
        Ok(ImportRequest::new(source, decision.media_type)
            .with_extension(decision.extension)
            .with_max_size(self.max_file_size_bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_support::{files_under, shipped_policy, test_store, TempDir};
    use crate::store::{AssetStore, ImportError};
    use crate::update_source::UpdateFromSourceOutcome;
    use stellar_storage::hash::ContentHash;

    fn shipped() -> AssetImportPolicy {
        shipped_policy()
    }

    fn media(text: &str) -> MediaType {
        MediaType::new(text).unwrap()
    }

    fn small_policy(unknown: &str) -> AssetImportPolicy {
        AssetImportPolicy::from_config_str(&format!(
            "max_file_size_bytes: 1000\nunknown_media_type: {unknown}\n\
             media_type: image/png png\nmedia_type: image/jpeg jpg\n"
        ))
        .unwrap()
    }

    /// 크기 경계 fixture: (max 기준 offset, 허용 여부).
    const SIZE_BOUNDARIES: [(i64, bool); 5] = [
        (-1000, true),
        (-1, true),
        (0, true),
        (1, false),
        (1000, false),
    ];

    #[test]
    fn shipped_config_defines_size_and_type_boundaries() {
        let policy = shipped();
        assert_eq!(policy.max_file_size_bytes(), 512 * 1024 * 1024);
        assert_eq!(
            policy.unknown_media_type(),
            UnknownMediaTypePolicy::StoreAsBinary,
            "unknown type은 binary asset으로 저장 가능"
        );
        let max = policy.max_file_size_bytes() as i64;
        for (offset, allowed) in SIZE_BOUNDARIES {
            let size = (max + offset) as u64;
            assert_eq!(policy.check_size(size).is_ok(), allowed, "size {size}");
        }
        assert!(policy.check_size(0).is_ok());
        assert_eq!(
            policy.check_size(u64::MAX).unwrap_err(),
            PolicyViolation::FileTooLarge {
                size: u64::MAX,
                max: max as u64
            }
        );

        for (declared, ext) in [
            ("image/png", "png"),
            ("image/jpeg", "jpg"),
            ("image/svg+xml", "svg"),
            ("font/woff2", "woff2"),
            ("font/collection", "ttc"),
            ("application/octet-stream", "bin"),
        ] {
            let decision = policy.decide(&media(declared)).unwrap();
            assert_eq!(decision.media_type, media(declared));
            assert_eq!(decision.extension.as_str(), ext);
            assert!(!decision.fallback_to_binary, "{declared}");
        }
        let unknown = policy.decide(&media("application/x-sketch")).unwrap();
        assert_eq!(unknown.media_type, media(BINARY_MEDIA_TYPE));
        assert_eq!(unknown.extension, SafeExtension::binary());
        assert!(unknown.fallback_to_binary);
        assert_eq!(policy.allowed_media_types().count(), 12);
    }

    #[test]
    fn reject_policy_refuses_unknown_types() {
        let policy = small_policy("reject");
        assert_eq!(policy.unknown_media_type(), UnknownMediaTypePolicy::Reject);
        let mp4 = media("video/mp4");
        assert_eq!(
            policy.decide(&mp4).unwrap_err(),
            PolicyViolation::MediaTypeNotAllowed {
                media_type: mp4.clone()
            }
        );
        assert_eq!(
            policy.request("/x/a.mp4", &mp4).unwrap_err(),
            PolicyViolation::MediaTypeNotAllowed { media_type: mp4 }
        );
        assert!(policy.decide(&media("image/png")).is_ok());
        assert_eq!(
            policy.extension_for(&media("image/jpeg")).unwrap().as_str(),
            "jpg"
        );
        assert_eq!(policy.extension_for(&media("image/gif")), None);
    }

    #[test]
    fn config_errors_are_explicit() {
        let base =
            "max_file_size_bytes: 10\nunknown_media_type: reject\nmedia_type: image/png png\n";
        assert!(AssetImportPolicy::from_config_str(base).is_ok());
        let invalid_media = |value: &str| PolicyConfigError::InvalidValue {
            key: "media_type",
            value: value.to_string(),
        };
        let cases: Vec<(String, PolicyConfigError)> = vec![
            (
                "unknown_media_type: reject\nmedia_type: image/png png\n".to_string(),
                PolicyConfigError::MissingKey {
                    key: "max_file_size_bytes",
                },
            ),
            (
                "max_file_size_bytes: 10\nmedia_type: image/png png\n".to_string(),
                PolicyConfigError::MissingKey {
                    key: "unknown_media_type",
                },
            ),
            (
                "max_file_size_bytes: 10\nunknown_media_type: reject\n".to_string(),
                PolicyConfigError::MissingKey { key: "media_type" },
            ),
            (
                format!("{base}max_file_size_bytes: 20\n"),
                PolicyConfigError::DuplicateKey {
                    key: "max_file_size_bytes",
                },
            ),
            (
                format!("{base}unknown_media_type: reject\n"),
                PolicyConfigError::DuplicateKey {
                    key: "unknown_media_type",
                },
            ),
            (
                base.replace("10", "0"),
                PolicyConfigError::InvalidValue {
                    key: "max_file_size_bytes",
                    value: "0".to_string(),
                },
            ),
            (
                base.replace("10", "-1"),
                PolicyConfigError::InvalidValue {
                    key: "max_file_size_bytes",
                    value: "-1".to_string(),
                },
            ),
            (
                base.replace("reject", "allow"),
                PolicyConfigError::InvalidValue {
                    key: "unknown_media_type",
                    value: "allow".to_string(),
                },
            ),
            (
                format!("{base}media_type: Image/PNG png\n"),
                PolicyConfigError::DuplicateMediaType {
                    media_type: "image/png".to_string(),
                },
            ),
            (
                format!("{base}media_type: image/gif\n"),
                invalid_media("image/gif"),
            ),
            (
                format!("{base}media_type: image/gif g.if\n"),
                invalid_media("image/gif g.if"),
            ),
            (
                format!("{base}media_type: image gif\n"),
                invalid_media("image gif"),
            ),
            (
                format!("{base}media_type: image/gif gif extra\n"),
                invalid_media("image/gif gif extra"),
            ),
            (
                format!("{base}max_total: 5\n"),
                PolicyConfigError::UnknownKey {
                    key: "max_total".to_string(),
                },
            ),
            (
                format!("{base}no separator\n"),
                PolicyConfigError::UnknownKey {
                    key: "no separator".to_string(),
                },
            ),
        ];
        for (text, expected) in cases {
            assert_eq!(
                AssetImportPolicy::from_config_str(&text).unwrap_err(),
                expected,
                "{text}"
            );
        }
        assert!(matches!(
            AssetImportPolicy::from_config_file(Path::new("/nonexistent/asset-import.yaml"))
                .unwrap_err(),
            PolicyConfigError::Io { .. }
        ));
    }

    struct Fixture {
        _dir: TempDir,
        project: PathBuf,
        outside: PathBuf,
        store: AssetStore,
    }

    fn fixture() -> Fixture {
        let dir = TempDir::new("asset-policy");
        let project = dir.path.join("project");
        let outside = dir.path.join("outside");
        std::fs::create_dir_all(project.join("images")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let store = test_store(&project);
        Fixture {
            _dir: dir,
            project,
            outside,
            store,
        }
    }

    #[test]
    fn import_size_boundary_fixture() {
        let fx = fixture();
        let policy = small_policy("store_as_binary");
        let max = policy.max_file_size_bytes() as i64;
        for (offset, allowed) in SIZE_BOUNDARIES {
            let size = (max + offset) as usize;
            let source = fx.outside.join(format!("f{size}.png"));
            std::fs::write(&source, vec![(size % 251) as u8; size]).unwrap();
            let before = files_under(&fx.project.join("assets"));
            let request = policy.request(&source, &media("image/png")).unwrap();
            let result = fx.store.import_file(&request);
            if allowed {
                assert_eq!(result.unwrap().record.size, size as u64);
            } else {
                assert_eq!(
                    result.unwrap_err(),
                    ImportError::TooLarge {
                        size: size as u64,
                        max: max as u64
                    },
                    "size {size}"
                );
                assert_eq!(
                    files_under(&fx.project.join("assets")),
                    before,
                    "초과 파일은 아무 것도 쓰지 않음"
                );
            }
        }
        // 공개 경로의 요청은 항상 정책 상한을 담습니다(크기 제한 없는 요청은 crate 내부 전용).
        let large = fx.outside.join("large.png");
        std::fs::write(&large, vec![1u8; 2000]).unwrap();
        let request = policy.request(&large, &media("image/png")).unwrap();
        assert_eq!(request.max_size(), Some(1000));
        assert!(matches!(
            fx.store.import_file(&request).unwrap_err(),
            ImportError::TooLarge { .. }
        ));
    }

    /// 공개 import 경로의 정책 강제 fixture: 원본 확장자(html·exe)는 저장 확장자가 되지 않고,
    /// unknown type은 정책대로 binary 저장 또는 거부, 크기 상한은 항상 적용됩니다.
    #[test]
    fn public_import_path_always_applies_policy() {
        let fx = fixture();
        let shipped = shipped();
        for (name, declared) in [
            ("page.html", "text/html"),
            ("tool.exe", "application/x-msdownload"),
        ] {
            let source = fx.outside.join(name);
            std::fs::write(&source, name.as_bytes()).unwrap();
            let request = shipped.request(&source, &media(declared)).unwrap();
            assert_eq!(request.max_size(), Some(shipped.max_file_size_bytes()));
            let imported = fx.store.import_file(&request).unwrap();
            assert_eq!(
                imported.record.media_type,
                media(BINARY_MEDIA_TYPE),
                "{name}"
            );
            assert!(
                imported.object_path.file_name().ends_with(".bin"),
                "{name}: {}",
                imported.object_path
            );
        }
        // 허용 type이라도 원본 확장자가 아니라 표의 확장자로 저장합니다.
        let disguised = fx.outside.join("logo.html");
        std::fs::write(&disguised, b"png bytes").unwrap();
        let imported = fx
            .store
            .import_file(&shipped.request(&disguised, &media("image/png")).unwrap())
            .unwrap();
        assert!(imported.object_path.file_name().ends_with(".png"));

        // reject 정책은 unknown type을 요청 단계에서 거부합니다(아무 것도 쓰지 않음).
        let strict = small_policy("reject");
        let before = files_under(&fx.project.join("assets"));
        assert_eq!(
            strict
                .request(fx.outside.join("page.html"), &media("text/html"))
                .unwrap_err(),
            PolicyViolation::MediaTypeNotAllowed {
                media_type: media("text/html")
            }
        );
        assert_eq!(files_under(&fx.project.join("assets")), before);
    }

    #[test]
    fn policy_controls_media_type_and_extension() {
        let fx = fixture();
        let policy = small_policy("store_as_binary");
        let photo = fx.outside.join("photo.JPEG");
        std::fs::write(&photo, b"jpeg bytes").unwrap();
        let imported = fx
            .store
            .import_file(&policy.request(&photo, &media("image/jpeg")).unwrap())
            .unwrap();
        assert_eq!(imported.record.media_type, media("image/jpeg"));
        assert!(
            imported.object_path.file_name().ends_with(".jpg"),
            "표의 확장자"
        );

        // unknown type → binary asset.
        let sketch = fx.outside.join("design.sketch");
        std::fs::write(&sketch, b"sketch bytes").unwrap();
        let request = policy
            .request(&sketch, &media("application/x-sketch"))
            .unwrap();
        let imported = fx.store.import_file(&request).unwrap();
        assert_eq!(imported.record.media_type, media(BINARY_MEDIA_TYPE));
        assert!(imported.object_path.file_name().ends_with(".bin"));
        assert_eq!(
            imported.record.content_hash,
            ContentHash::compute(b"sketch bytes")
        );
    }

    #[test]
    fn update_from_source_respects_policy() {
        let fx = fixture();
        let policy = small_policy("store_as_binary");
        let source = fx.project.join("images").join("logo.png");
        std::fs::write(&source, vec![1u8; 10]).unwrap();
        let current = fx
            .store
            .import_file(&policy.request(&source, &media("image/png")).unwrap())
            .unwrap()
            .record;
        let id = crate::id::AssetId::from_bytes([3u8; 16]);

        // 원본이 상한을 넘게 커지면 거부하고 기존 asset을 유지합니다.
        std::fs::write(&source, vec![2u8; 1001]).unwrap();
        let before = files_under(&fx.project.join("assets"));
        assert_eq!(
            fx.store
                .update_from_source(id, &current, None, &policy)
                .unwrap_err(),
            ImportError::TooLarge {
                size: 1001,
                max: 1000
            }
        );
        assert_eq!(files_under(&fx.project.join("assets")), before);

        // 상한 이내 변경은 정상 갱신됩니다.
        std::fs::write(&source, vec![2u8; 1000]).unwrap();
        assert!(matches!(
            fx.store
                .update_from_source(id, &current, None, &policy)
                .unwrap(),
            UpdateFromSourceOutcome::Updated { .. }
        ));

        // 정책이 바뀌어 더 이상 허용되지 않는 type이면 거부합니다.
        let strict = AssetImportPolicy::from_config_str(
            "max_file_size_bytes: 1000\nunknown_media_type: reject\nmedia_type: image/jpeg jpg\n",
        )
        .unwrap();
        assert_eq!(
            fx.store
                .update_from_source(id, &current, None, &strict)
                .unwrap_err(),
            ImportError::Policy(PolicyViolation::MediaTypeNotAllowed {
                media_type: media("image/png")
            })
        );
    }

    #[test]
    fn messages_are_descriptive() {
        assert!(PolicyViolation::FileTooLarge { size: 2, max: 1 }
            .to_string()
            .contains("2 bytes > 1 bytes"));
        assert!(PolicyConfigError::MissingKey { key: "media_type" }
            .to_string()
            .contains("media_type"));
    }
}
