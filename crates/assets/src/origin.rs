//! 외부 원본 출처 지문 — 기기별 비밀 key의 HMAC-SHA-256 (XIII.5.1 후속 수정).
//!
//! 프로젝트 밖 원본은 절대 경로를 record에 남기지 않고 파일 이름과 출처 지문만 기록합니다
//! ([`crate::record::SourceLocation::External`]). record는 Git에 commit되므로 지문이 **추측
//! 확인 도구**가 되면 안 됩니다. key 없는 SHA-256(경로)는 누구나 `/Users/<이름>/Downloads/
//! <파일 이름>` 같은 후보를 계산해 맞춰 볼 수 있어 사용자 이름·디렉터리가 사전 공격으로
//! 드러납니다. 그래서 지문은 기기별 비밀 key의 HMAC입니다.
//!
//! - **key**: 32 bytes OS 난수([`OriginKey`]). 프로젝트 **밖** OS 사용자 데이터 영역의 파일
//!   하나에 둡니다([`OriginKeyFile`], Unix `0600`, 처음 필요할 때 생성). 프로젝트 안 경로는
//!   거부합니다(commit되면 key가 공개됨).
//! - **지문**: `HMAC-SHA-256(key, domain ‖ 해석된 절대 경로의 OS encoded bytes)`와 key 식별자
//!   ([`OriginKeyId`], `HMAC(key, id domain)` 앞 8 bytes)를 함께 기록합니다
//!   ([`OriginFingerprint`], `hmac-sha256:<key-id hex16>:<mac hex64>`).
//! - **같은 기기**: 같은 key로 후보 경로의 지문을 계산해 비교하므로 같은 출처 확인이 그대로
//!   동작합니다.
//! - **다른 기기**: key 식별자가 달라 **확인할 수 없습니다**([`OriginCheck::ForeignKey`]).
//!   `Update from Source`는 이때 사용자에게 원본 경로를 다시 선택받아야 합니다
//!   ([`crate::update_source::SourceUnavailable::OriginUnverifiable`]).
//!
//! # 보장 범위(정확히)
//!
//! - key가 없는 사람(저장소만 가진 사람)은 지문으로 경로 추측을 확인할 수 없습니다(HMAC
//!   key 256 bit). 지문은 경로를 "복원 불가능하게 숨기는" 것이 아니라 key 없이 **검증할 수
//!   없게** 하는 것입니다.
//! - key 파일을 읽을 수 있는 사람(그 기기의 같은 사용자 계정 등)은 추측을 확인할 수 있습니다.
//! - key 식별자는 경로 정보를 담지 않지만, 같은 기기에서 만든 record끼리 연결(linkability)할
//!   수 있게 합니다.
//! - key 파일을 지우거나 바꾸면 그 기기에서도 기존 지문을 확인할 수 없습니다(다른 기기와 같음).

use std::fmt;
use std::io::{ErrorKind, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::atomic_write::{create_new_temp, TEMP_NAME_ATTEMPTS};
use crate::hash_stream::hmac_sha256;

/// key 길이(byte).
pub const ORIGIN_KEY_LEN: usize = 32;

/// key 식별자 길이(byte).
pub const ORIGIN_KEY_ID_LEN: usize = 8;

/// 출처 지문 문자열 prefix.
pub const ORIGIN_FINGERPRINT_PREFIX: &str = "hmac-sha256:";

/// 출처 지문 HMAC 입력의 domain 구분 prefix(경로 bytes 앞).
const SOURCE_ORIGIN_DOMAIN: &[u8] = b"stellar.asset.source-origin.v2\0";

/// key 식별자 HMAC 입력.
const KEY_ID_DOMAIN: &[u8] = b"stellar.asset.origin-key-id.v1";

/// key 파일 첫 줄(형식 버전).
const KEY_FILE_HEADER: &str = "stellar.asset.origin-key.v1";

/// key 파일 하위 디렉터리 이름(`<data_dir>/assets`).
pub const ORIGIN_KEY_DIR: &str = "assets";

/// key 파일 이름.
pub const ORIGIN_KEY_FILE: &str = "origin-key";

/// 출처 key 관련 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OriginKeyError {
    /// store에 key가 설정되지 않았습니다(외부 원본 지문을 만들거나 확인할 수 없음).
    Unconfigured,
    /// key 파일 경로가 절대 경로가 아니거나 `.`·`..`를 포함합니다.
    InvalidPath {
        /// 문제된 경로.
        path: PathBuf,
    },
    /// key 파일이 project root 안에 있습니다(commit되면 key가 공개됨).
    InsideProject {
        /// 문제된 경로.
        path: PathBuf,
    },
    /// key 파일 형식이 잘못되었습니다(덮어쓰지 않음).
    Corrupt {
        /// 문제된 경로.
        path: PathBuf,
    },
    /// key 파일 권한이 소유자 전용(`0600`)보다 넓습니다(Unix).
    InsecurePermissions {
        /// 문제된 경로.
        path: PathBuf,
        /// 파일 권한 bits(`0o777` mask).
        mode: u32,
    },
    /// OS 난수를 얻을 수 없습니다.
    RandomUnavailable {
        /// I/O 오류 종류.
        kind: ErrorKind,
    },
    /// 파일 I/O 실패.
    Io {
        /// 실패한 경로.
        path: PathBuf,
        /// I/O 오류 종류.
        kind: ErrorKind,
    },
}

impl fmt::Display for OriginKeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OriginKeyError::Unconfigured => f.write_str("외부 원본 출처 key가 설정되지 않았습니다"),
            OriginKeyError::InvalidPath { path } => write!(
                f,
                "출처 key 파일 경로는 정규화된 절대 경로여야 합니다: {path:?}"
            ),
            OriginKeyError::InsideProject { path } => {
                write!(f, "출처 key 파일은 프로젝트 밖에 있어야 합니다: {path:?}")
            }
            OriginKeyError::Corrupt { path } => {
                write!(f, "출처 key 파일 형식이 잘못되었습니다: {path:?}")
            }
            OriginKeyError::InsecurePermissions { path, mode } => write!(
                f,
                "출처 key 파일 권한이 너무 넓습니다({mode:o}, 0600 필요): {path:?}"
            ),
            OriginKeyError::RandomUnavailable { kind } => {
                write!(f, "OS 난수를 얻을 수 없습니다: {kind}")
            }
            OriginKeyError::Io { path, kind } => {
                write!(f, "출처 key 파일 입출력 실패 {path:?}: {kind}")
            }
        }
    }
}

impl std::error::Error for OriginKeyError {}

/// key 식별자(공개 값). `HMAC(key, id domain)` 앞 8 bytes이며 key를 복원할 수 없습니다.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct OriginKeyId([u8; ORIGIN_KEY_ID_LEN]);

impl OriginKeyId {
    /// 8 bytes 표현.
    pub const fn to_bytes(&self) -> [u8; ORIGIN_KEY_ID_LEN] {
        self.0
    }
}

impl fmt::Display for OriginKeyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_hex(f, &self.0)
    }
}

impl fmt::Debug for OriginKeyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "OriginKeyId({self})")
    }
}

/// 기기별 출처 key(비밀 값). `Debug`는 key 식별자만 보여 줍니다.
#[derive(Clone, PartialEq, Eq)]
pub struct OriginKey([u8; ORIGIN_KEY_LEN]);

impl OriginKey {
    /// 32 bytes로부터 만듭니다(테스트·key 파일 복원용).
    pub const fn from_bytes(bytes: [u8; ORIGIN_KEY_LEN]) -> OriginKey {
        OriginKey(bytes)
    }

    /// OS 난수로 새 key를 만듭니다(Unix `/dev/urandom`).
    pub fn generate() -> Result<OriginKey, OriginKeyError> {
        let mut bytes = [0u8; ORIGIN_KEY_LEN];
        os_random(&mut bytes).map_err(|e| OriginKeyError::RandomUnavailable { kind: e.kind() })?;
        Ok(OriginKey(bytes))
    }

    /// key 식별자.
    pub fn key_id(&self) -> OriginKeyId {
        let mac = hmac_sha256(&self.0, KEY_ID_DOMAIN);
        let mut id = [0u8; ORIGIN_KEY_ID_LEN];
        id.copy_from_slice(&mac[..ORIGIN_KEY_ID_LEN]);
        OriginKeyId(id)
    }

    /// key 파일 본문(`<header>\n<hex64>\n`).
    fn to_file_text(&self) -> String {
        let mut text = String::with_capacity(KEY_FILE_HEADER.len() + 2 + 2 * ORIGIN_KEY_LEN);
        text.push_str(KEY_FILE_HEADER);
        text.push('\n');
        for b in self.0 {
            text.push_str(&format!("{b:02x}"));
        }
        text.push('\n');
        text
    }

    /// key 파일 본문을 엄격히 읽습니다(소문자 hex 64자리만).
    fn from_file_text(text: &str) -> Option<OriginKey> {
        let body = text.strip_prefix(KEY_FILE_HEADER)?.strip_prefix('\n')?;
        let hex = body.strip_suffix('\n')?;
        decode_hex::<ORIGIN_KEY_LEN>(hex).map(OriginKey)
    }
}

impl fmt::Debug for OriginKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "OriginKey {{ id: {}, .. }}", self.key_id())
    }
}

/// 출처 key 공급원. 외부 원본을 기록·확인할 때만 호출됩니다(내부 원본은 key 불필요).
pub trait OriginKeyProvider {
    /// key를 돌려줍니다. 파일 공급원은 처음 호출 때 key 파일을 만듭니다.
    fn origin_key(&self) -> Result<OriginKey, OriginKeyError>;
}

impl OriginKeyProvider for OriginKey {
    fn origin_key(&self) -> Result<OriginKey, OriginKeyError> {
        Ok(self.clone())
    }
}

/// 외부 원본 출처 지문(record에 commit되는 값).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OriginFingerprint {
    key_id: OriginKeyId,
    mac: [u8; 32],
}

impl OriginFingerprint {
    /// 해석된(canonical) 절대 원본 경로의 지문을 key로 계산합니다.
    pub fn compute(key: &OriginKey, resolved_source: &Path) -> OriginFingerprint {
        let mut input = SOURCE_ORIGIN_DOMAIN.to_vec();
        input.extend_from_slice(resolved_source.as_os_str().as_encoded_bytes());
        OriginFingerprint {
            key_id: key.key_id(),
            mac: hmac_sha256(&key.0, &input),
        }
    }

    /// 지문을 만든 key의 식별자.
    pub fn key_id(&self) -> OriginKeyId {
        self.key_id
    }

    /// 경로가 이 지문의 출처와 같은지 판정합니다.
    pub fn check(&self, key: &OriginKey, resolved_source: &Path) -> OriginCheck {
        if self.key_id != key.key_id() {
            return OriginCheck::ForeignKey;
        }
        if OriginFingerprint::compute(key, resolved_source) == *self {
            OriginCheck::Matches
        } else {
            OriginCheck::DifferentOrigin
        }
    }
}

impl fmt::Display for OriginFingerprint {
    /// `hmac-sha256:<key-id hex16>:<mac hex64>`(소문자).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(ORIGIN_FINGERPRINT_PREFIX)?;
        write_hex(f, &self.key_id.0)?;
        f.write_str(":")?;
        write_hex(f, &self.mac)
    }
}

/// 출처 지문 문자열 파싱 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseOriginFingerprintError {
    /// `hmac-sha256:` prefix가 없습니다(이전 key 없는 `sha256:` 형식 포함).
    UnknownScheme,
    /// key 식별자·mac 구분 `:`가 없거나 길이가 다릅니다.
    InvalidLength,
    /// 소문자 hex가 아닌 문자가 있습니다.
    InvalidHex,
}

impl fmt::Display for ParseOriginFingerprintError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseOriginFingerprintError::UnknownScheme => {
                f.write_str("출처 지문은 hmac-sha256: 형식이어야 합니다")
            }
            ParseOriginFingerprintError::InvalidLength => {
                f.write_str("출처 지문 길이가 잘못되었습니다")
            }
            ParseOriginFingerprintError::InvalidHex => {
                f.write_str("출처 지문은 소문자 hex여야 합니다")
            }
        }
    }
}

impl std::error::Error for ParseOriginFingerprintError {}

impl FromStr for OriginFingerprint {
    type Err = ParseOriginFingerprintError;

    /// canonical 형식(소문자)만 받습니다.
    fn from_str(s: &str) -> Result<OriginFingerprint, ParseOriginFingerprintError> {
        let rest = s
            .strip_prefix(ORIGIN_FINGERPRINT_PREFIX)
            .ok_or(ParseOriginFingerprintError::UnknownScheme)?;
        let (id, mac) = rest
            .split_once(':')
            .ok_or(ParseOriginFingerprintError::InvalidLength)?;
        if id.len() != 2 * ORIGIN_KEY_ID_LEN || mac.len() != 64 {
            return Err(ParseOriginFingerprintError::InvalidLength);
        }
        let id =
            decode_hex::<ORIGIN_KEY_ID_LEN>(id).ok_or(ParseOriginFingerprintError::InvalidHex)?;
        let mac = decode_hex::<32>(mac).ok_or(ParseOriginFingerprintError::InvalidHex)?;
        Ok(OriginFingerprint {
            key_id: OriginKeyId(id),
            mac,
        })
    }
}

/// 출처 확인 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OriginCheck {
    /// 같은 key, 같은 출처입니다.
    Matches,
    /// 같은 key지만 다른 경로입니다.
    DifferentOrigin,
    /// 다른 key(다른 기기 또는 바뀐 key)로 기록되어 확인할 수 없습니다.
    ForeignKey,
    /// 외부 원본 record가 아닙니다.
    NotExternal,
}

/// OS 사용자 데이터 영역의 출처 key 파일.
///
/// 위치는 `<data_dir>/assets/origin-key`이며 `data_dir`은 셸이 정한 Stellar 앱 데이터
/// 디렉터리입니다(신뢰 저장소 XVI.7.2와 같은 base). 형식은 `stellar.asset.origin-key.v1` 한 줄과
/// 소문자 hex 64자리 한 줄입니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OriginKeyFile {
    path: PathBuf,
}

impl OriginKeyFile {
    /// key 파일 경로를 직접 지정합니다(정규화된 절대 경로여야 함, I/O 없음).
    pub fn new(path: impl Into<PathBuf>) -> Result<OriginKeyFile, OriginKeyError> {
        let path = path.into();
        let normalized = path.is_absolute()
            && path
                .components()
                .all(|c| !matches!(c, Component::CurDir | Component::ParentDir));
        if !normalized || path.file_name().is_none() {
            return Err(OriginKeyError::InvalidPath { path });
        }
        Ok(OriginKeyFile { path })
    }

    /// Stellar 앱 데이터 디렉터리 아래 기본 위치(`<data_dir>/assets/origin-key`).
    pub fn in_data_dir(data_dir: &Path) -> Result<OriginKeyFile, OriginKeyError> {
        OriginKeyFile::new(data_dir.join(ORIGIN_KEY_DIR).join(ORIGIN_KEY_FILE))
    }

    /// key 파일 경로.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 경로가 project root(canonical) 밖인지 확인합니다. 아직 없는 부분은 가장 가까운 기존
    /// 상위 디렉터리를 canonicalize해 판정합니다(symlink를 거쳐 안으로 들어오는 경우 포함).
    pub fn ensure_outside(&self, project_root: &Path) -> Result<(), OriginKeyError> {
        let io = |path: &Path, e: std::io::Error| OriginKeyError::Io {
            path: path.to_path_buf(),
            kind: e.kind(),
        };
        let root = std::fs::canonicalize(project_root).map_err(|e| io(project_root, e))?;
        let mut existing = self.path.as_path();
        let mut rest: Vec<&std::ffi::OsStr> = Vec::new();
        let resolved = loop {
            match std::fs::canonicalize(existing) {
                Ok(path) => break path,
                Err(e) if e.kind() == ErrorKind::NotFound => {
                    let (Some(name), Some(parent)) = (existing.file_name(), existing.parent())
                    else {
                        return Err(io(existing, e));
                    };
                    rest.push(name);
                    existing = parent;
                }
                Err(e) => return Err(io(existing, e)),
            }
        };
        let resolved = rest
            .iter()
            .rev()
            .fold(resolved, |path, name| path.join(name));
        if resolved.starts_with(&root) {
            return Err(OriginKeyError::InsideProject {
                path: self.path.clone(),
            });
        }
        Ok(())
    }

    /// 기존 key를 읽습니다. 파일이 없으면 `None`(만들지 않음).
    pub fn load(&self) -> Result<Option<OriginKey>, OriginKeyError> {
        let io = |e: std::io::Error| OriginKeyError::Io {
            path: self.path.clone(),
            kind: e.kind(),
        };
        let metadata = match std::fs::symlink_metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(io(e)),
        };
        if !metadata.is_file() {
            return Err(OriginKeyError::Corrupt {
                path: self.path.clone(),
            });
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = metadata.permissions().mode() & 0o777;
            if mode & 0o077 != 0 {
                return Err(OriginKeyError::InsecurePermissions {
                    path: self.path.clone(),
                    mode,
                });
            }
        }
        let mut text = String::new();
        std::fs::File::open(&self.path)
            .and_then(|mut file| file.read_to_string(&mut text))
            .map_err(|e| match e.kind() {
                ErrorKind::InvalidData => OriginKeyError::Corrupt {
                    path: self.path.clone(),
                },
                _ => io(e),
            })?;
        OriginKey::from_file_text(&text)
            .map(Some)
            .ok_or(OriginKeyError::Corrupt {
                path: self.path.clone(),
            })
    }

    /// key를 읽고, 없으면 새로 만들어 기록합니다(처음 사용 시 생성).
    ///
    /// **이 호출이 새로 만든** temp 파일(`0600`, 예측할 수 없는 이름)에 쓰고 sync한 뒤 hard
    /// link로 최종 이름을 만듭니다. temp 이름이 이미 있으면 다른 이름을 쓰며 기존 항목은 열거나
    /// 지우지 않습니다. 최종 이름이 이미 있으면(다른 process가 먼저 만듦) 그 key를 다시 읽어
    /// 씁니다 — 기존 key를 절대 덮어쓰지 않습니다. 손상된 key 파일도 덮어쓰지 않고 오류로
    /// 돌려줍니다.
    pub fn load_or_create(&self) -> Result<OriginKey, OriginKeyError> {
        let Some(dir) = self.path.parent() else {
            return Err(OriginKeyError::InvalidPath {
                path: self.path.clone(),
            });
        };
        for _ in 0..PUBLISH_ATTEMPTS {
            if let Some(key) = self.load()? {
                return Ok(key);
            }
            create_private_dir_all(dir).map_err(|e| key_io(dir, e))?;
            let key = OriginKey::generate()?;
            let names = temp_key_names(dir)?;
            match self.publish(dir, &key, names)? {
                Published::Created => return Ok(key),
                // 다른 process가 먼저 최종 이름을 만들었습니다. 다시 읽습니다(손상이면 오류).
                Published::AlreadyExists => continue,
            }
        }
        // 최종 이름이 만들어졌다 사라지기를 반복합니다(동시 삭제 경합).
        Err(OriginKeyError::Io {
            path: self.path.clone(),
            kind: ErrorKind::AlreadyExists,
        })
    }

    /// `key`를 `names` 중 처음 비어 있는 이름의 새 temp에 쓰고 최종 이름으로 hard link합니다.
    ///
    /// temp 생성의 `AlreadyExists`(이름 점유)와 hard link의 `AlreadyExists`(최종 key 존재)를
    /// 구분합니다. 앞의 것은 다음 이름으로 넘어가고, 뒤의 것만 [`Published::AlreadyExists`]입니다.
    /// 정리(`remove_file`)는 이 호출이 만든 temp에만 합니다.
    fn publish(
        &self,
        dir: &Path,
        key: &OriginKey,
        names: impl IntoIterator<Item = PathBuf>,
    ) -> Result<Published, OriginKeyError> {
        let (temp, mut file) =
            create_new_temp(&private_new_file_options(), names).map_err(|e| key_io(dir, e))?;
        let written = file
            .write_all(key.to_file_text().as_bytes())
            .and_then(|()| file.sync_all());
        drop(file);
        if let Err(e) = written {
            let _ = std::fs::remove_file(&temp);
            return Err(key_io(&temp, e));
        }
        let linked = std::fs::hard_link(&temp, &self.path);
        let _ = std::fs::remove_file(&temp);
        match linked {
            Ok(()) => {
                sync_dir(dir).map_err(|e| key_io(dir, e))?;
                Ok(Published::Created)
            }
            Err(e) if e.kind() == ErrorKind::AlreadyExists => Ok(Published::AlreadyExists),
            Err(e) => Err(key_io(&self.path, e)),
        }
    }
}

/// 최종 이름 경합 뒤 다시 읽고 만드는 최대 횟수.
const PUBLISH_ATTEMPTS: usize = 4;

/// [`OriginKeyFile::publish`] 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Published {
    /// 이 호출의 key로 최종 이름을 만들었습니다.
    Created,
    /// 최종 이름이 이미 있습니다(다른 process가 먼저 만듦 — 이 호출의 key는 버림).
    AlreadyExists,
}

fn key_io(path: &Path, e: std::io::Error) -> OriginKeyError {
    OriginKeyError::Io {
        path: path.to_path_buf(),
        kind: e.kind(),
    }
}

/// 예측할 수 없는 temp 이름 후보(`.origin-key.tmp-<pid>-<n>-<random hex16>`).
///
/// 난수 suffix가 있어 같은 디렉터리에 이름을 미리 심어 두는 방식으로 temp 생성을 막거나
/// 다른 항목을 지우게 할 수 없습니다.
fn temp_key_names(dir: &Path) -> Result<Vec<PathBuf>, OriginKeyError> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut nonce = [0u8; 8];
    os_random(&mut nonce).map_err(|e| OriginKeyError::RandomUnavailable { kind: e.kind() })?;
    let nonce: String = nonce.iter().map(|b| format!("{b:02x}")).collect();
    Ok((0..TEMP_NAME_ATTEMPTS)
        .map(|_| {
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            dir.join(format!(
                ".{ORIGIN_KEY_FILE}.tmp-{}-{n}-{nonce}",
                std::process::id()
            ))
        })
        .collect())
}

impl OriginKeyProvider for OriginKeyFile {
    fn origin_key(&self) -> Result<OriginKey, OriginKeyError> {
        self.load_or_create()
    }
}

/// OS 난수.
#[cfg(unix)]
fn os_random(buf: &mut [u8]) -> std::io::Result<()> {
    std::fs::File::open("/dev/urandom")?.read_exact(buf)
}

/// OS 난수(비Unix는 아직 지원하지 않음 — Windows 지원 보류).
#[cfg(not(unix))]
fn os_random(_buf: &mut [u8]) -> std::io::Result<()> {
    Err(std::io::Error::from(ErrorKind::Unsupported))
}

/// 소유자 전용(`0700`) 디렉터리를 만듭니다(이미 있으면 그대로).
fn create_private_dir_all(dir: &Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)
}

/// 소유자 전용(`0600`) 새 파일 열기 옵션(`create_new` — 기존 항목·symlink를 열지 않음).
fn private_new_file_options() -> std::fs::OpenOptions {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
}

/// 소유자 전용(`0600`) 새 파일에 기록하고 sync합니다(테스트 fixture용).
#[cfg(test)]
fn write_private_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = private_new_file_options().open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

#[cfg(unix)]
fn sync_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::File::open(dir)?.sync_all()
}

#[cfg(not(unix))]
fn sync_dir(_dir: &Path) -> std::io::Result<()> {
    Ok(())
}

fn write_hex(f: &mut fmt::Formatter<'_>, bytes: &[u8]) -> fmt::Result {
    for b in bytes {
        write!(f, "{b:02x}")?;
    }
    Ok(())
}

/// 소문자 hex `2N`자리를 `N` bytes로 읽습니다.
fn decode_hex<const N: usize>(text: &str) -> Option<[u8; N]> {
    let bytes = text.as_bytes();
    if bytes.len() != 2 * N {
        return None;
    }
    let nibble = |c: u8| match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    };
    let mut out = [0u8; N];
    for (slot, pair) in out.iter_mut().zip(bytes.chunks_exact(2)) {
        *slot = (nibble(pair[0])? << 4) | nibble(pair[1])?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_support::TempDir;
    use stellar_storage::hash::ContentHash;

    fn key(n: u8) -> OriginKey {
        OriginKey::from_bytes([n; ORIGIN_KEY_LEN])
    }

    #[test]
    fn fingerprint_is_keyed_domain_separated_and_path_sensitive() {
        let a = Path::new("/Users/someone/Downloads/logo.png");
        let b = Path::new("/Users/someone/Desktop/logo.png");
        let fp = OriginFingerprint::compute(&key(1), a);
        assert_eq!(fp, OriginFingerprint::compute(&key(1), a), "결정론");
        assert_ne!(fp, OriginFingerprint::compute(&key(1), b), "경로 구분");
        assert_ne!(
            fp,
            OriginFingerprint::compute(&key(2), a),
            "key가 다르면 다름"
        );
        assert_eq!(fp.key_id(), key(1).key_id());
        assert_ne!(key(1).key_id(), key(2).key_id());

        assert_eq!(fp.check(&key(1), a), OriginCheck::Matches);
        assert_eq!(fp.check(&key(1), b), OriginCheck::DifferentOrigin);
        assert_eq!(fp.check(&key(2), a), OriginCheck::ForeignKey);
    }

    /// 저장소만 가진 공격자의 경로 추측 확인 fixture.
    #[test]
    fn committed_fingerprint_does_not_confirm_guessed_paths_without_key() {
        let secret = key(7);
        let real = Path::new("/Users/alice/Downloads/logo.png");
        let fp = OriginFingerprint::compute(&secret, real);
        let text = fp.to_string();
        for guess in [
            "/Users/alice/Downloads/logo.png",
            "/Users/alice/Desktop/logo.png",
            "/Users/bob/Downloads/logo.png",
        ] {
            // key 없는 hash(이전 형식)와 기록 값이 일치하지 않습니다.
            let mut v1 = b"stellar.asset.source-origin.v1\0".to_vec();
            v1.extend_from_slice(guess.as_bytes());
            let unkeyed = ContentHash::compute(&v1).digest_hex();
            assert!(!text.contains(&unkeyed), "{guess}");
            let mut v2 = SOURCE_ORIGIN_DOMAIN.to_vec();
            v2.extend_from_slice(guess.as_bytes());
            assert!(!text.contains(&ContentHash::compute(&v2).digest_hex()));
            // 다른(추측한) key로는 실제 경로도 확인되지 않습니다.
            for attacker in [key(0), key(8), OriginKey::from_bytes([0xff; 32])] {
                assert_ne!(
                    fp.check(&attacker, Path::new(guess)),
                    OriginCheck::Matches,
                    "{guess}"
                );
            }
        }
        // key를 가진 같은 기기만 확인합니다.
        assert_eq!(fp.check(&secret, real), OriginCheck::Matches);
    }

    #[test]
    fn fingerprint_string_round_trips_strictly() {
        let fp = OriginFingerprint::compute(&key(3), Path::new("/x/a.png"));
        let text = fp.to_string();
        assert!(text.starts_with("hmac-sha256:"));
        assert_eq!(text.len(), "hmac-sha256:".len() + 16 + 1 + 64);
        assert_eq!(text.parse::<OriginFingerprint>().unwrap(), fp);

        let (head, mac) = text.rsplit_once(':').unwrap();
        for (input, expected) in [
            (
                // 이전 key 없는 형식은 받지 않습니다.
                format!("sha256:{}", ContentHash::compute(b"x").digest_hex()),
                ParseOriginFingerprintError::UnknownScheme,
            ),
            (
                text.to_uppercase(),
                ParseOriginFingerprintError::UnknownScheme,
            ),
            (
                format!("{head}{mac}"),
                ParseOriginFingerprintError::InvalidLength,
            ),
            (
                format!("{head}:{}", &mac[..63]),
                ParseOriginFingerprintError::InvalidLength,
            ),
            (
                format!("{head}:{}", mac.to_uppercase()),
                ParseOriginFingerprintError::InvalidHex,
            ),
            (
                format!("{head}:{}g", &mac[..63]),
                ParseOriginFingerprintError::InvalidHex,
            ),
        ] {
            assert_eq!(
                input.parse::<OriginFingerprint>().unwrap_err(),
                expected,
                "{input}"
            );
        }
    }

    #[test]
    fn key_debug_does_not_print_secret() {
        let text = format!("{:?}", key(0xab));
        assert!(!text.contains("abab"), "{text}");
        assert!(text.contains(&key(0xab).key_id().to_string()));
    }

    #[test]
    fn generated_keys_differ() {
        let a = OriginKey::generate().unwrap();
        let b = OriginKey::generate().unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn key_file_is_created_once_private_and_reused() {
        let dir = TempDir::new("origin-key");
        let file = OriginKeyFile::in_data_dir(&dir.path.join("data")).unwrap();
        assert_eq!(file.load().unwrap(), None, "처음에는 없음(만들지 않음)");
        assert!(!file.path().exists());

        let created = file.load_or_create().unwrap();
        assert!(file.path().exists());
        assert_eq!(file.load_or_create().unwrap(), created, "같은 key 재사용");
        assert_eq!(file.origin_key().unwrap(), created);
        let text = std::fs::read_to_string(file.path()).unwrap();
        assert_eq!(text, created.to_file_text());
        assert!(text.starts_with("stellar.asset.origin-key.v1\n"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(file.path()).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        // temp가 남지 않습니다.
        let entries: Vec<_> = std::fs::read_dir(file.path().parent().unwrap())
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(entries, vec![std::ffi::OsString::from(ORIGIN_KEY_FILE)]);
    }

    #[test]
    fn existing_key_file_is_never_overwritten() {
        let dir = TempDir::new("origin-key-existing");
        let file = OriginKeyFile::new(dir.path.join("k")).unwrap();
        write_private_file(file.path(), key(5).to_file_text().as_bytes()).unwrap();
        assert_eq!(file.load_or_create().unwrap(), key(5));

        // 손상 파일은 덮어쓰지 않고 오류입니다.
        let corrupt = OriginKeyFile::new(dir.path.join("corrupt")).unwrap();
        for body in [
            "".to_string(),
            "stellar.asset.origin-key.v1\nabc\n".to_string(),
            key(5).to_file_text().to_uppercase(),
            key(5).to_file_text().trim_end().to_string(),
        ] {
            let _ = std::fs::remove_file(corrupt.path());
            write_private_file(corrupt.path(), body.as_bytes()).unwrap();
            assert_eq!(
                corrupt.load_or_create().unwrap_err(),
                OriginKeyError::Corrupt {
                    path: corrupt.path().to_path_buf()
                },
                "{body:?}"
            );
            assert_eq!(std::fs::read_to_string(corrupt.path()).unwrap(), body);
        }
    }

    fn entry_names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    /// 보안 리뷰 S2 재현: 예측 가능한 temp 이름에 미리 심어 둔 symlink fixture.
    #[cfg(unix)]
    #[test]
    fn squatted_temp_names_neither_block_creation_nor_get_removed() {
        let dir = TempDir::new("origin-key-squat");
        let file = OriginKeyFile::in_data_dir(&dir.path.join("data")).unwrap();
        let key_dir = file.path().parent().unwrap().to_path_buf();
        create_private_dir_all(&key_dir).unwrap();
        let victim = dir.path.join("victim");
        std::fs::write(&victim, b"victim").unwrap();
        // 이전 형식(`.origin-key.tmp-<pid>-<n>`)의 예측 가능한 이름을 점유합니다.
        let squatters: Vec<PathBuf> = (0..32)
            .map(|n| key_dir.join(format!(".{ORIGIN_KEY_FILE}.tmp-{}-{n}", std::process::id())))
            .collect();
        for squatter in &squatters {
            std::os::unix::fs::symlink(&victim, squatter).unwrap();
        }

        let key = file.load_or_create().unwrap();
        assert_eq!(file.load().unwrap(), Some(key));
        assert_eq!(
            std::fs::read(&victim).unwrap(),
            b"victim",
            "symlink로 쓰지 않음"
        );
        for squatter in &squatters {
            assert!(
                squatter
                    .symlink_metadata()
                    .unwrap()
                    .file_type()
                    .is_symlink(),
                "{squatter:?} 보존"
            );
        }
        // 남은 항목은 key 파일과 점유 항목뿐입니다(이 호출의 temp는 정리).
        assert_eq!(entry_names(&key_dir).len(), squatters.len() + 1);
    }

    /// temp 생성 충돌(이름 점유)과 hard link 충돌(최종 key 존재)을 구분합니다.
    #[test]
    fn temp_collision_is_not_mistaken_for_published_key() {
        let dir = TempDir::new("origin-key-publish");
        let file = OriginKeyFile::new(dir.path.join("k")).unwrap();
        let taken = dir.path.join(".k.tmp-taken");
        std::fs::write(&taken, b"other").unwrap();
        let fresh = dir.path.join(".k.tmp-fresh");

        // 점유된 이름뿐이면 AlreadyExists I/O 오류입니다(Corrupt 아님). 점유 항목은 보존합니다.
        assert_eq!(
            file.publish(&dir.path, &key(1), [taken.clone()])
                .unwrap_err(),
            OriginKeyError::Io {
                path: dir.path.clone(),
                kind: ErrorKind::AlreadyExists
            }
        );
        assert_eq!(std::fs::read(&taken).unwrap(), b"other");
        assert!(!file.path().exists());

        // 점유된 이름은 건너뛰고 새 이름으로 만듭니다.
        assert_eq!(
            file.publish(&dir.path, &key(1), [taken.clone(), fresh.clone()])
                .unwrap(),
            Published::Created
        );
        assert_eq!(file.load().unwrap(), Some(key(1)));
        assert!(!fresh.exists(), "이 호출의 temp는 정리");
        assert_eq!(std::fs::read(&taken).unwrap(), b"other");

        // 최종 이름이 이미 있으면(다른 process가 먼저 만듦) 덮어쓰지 않고 그대로 둡니다.
        assert_eq!(
            file.publish(&dir.path, &key(2), [fresh.clone()]).unwrap(),
            Published::AlreadyExists
        );
        assert_eq!(file.load().unwrap(), Some(key(1)));
        assert!(!fresh.exists());
        assert_eq!(
            entry_names(&dir.path),
            vec![".k.tmp-taken".to_string(), "k".to_string()]
        );
    }

    #[test]
    fn temp_key_names_are_unpredictable_and_distinct() {
        let dir = Path::new("/d");
        let a = temp_key_names(dir).unwrap();
        let b = temp_key_names(dir).unwrap();
        assert_eq!(a.len(), TEMP_NAME_ATTEMPTS);
        let prefix = format!(".{ORIGIN_KEY_FILE}.tmp-{}-", std::process::id());
        for name in a.iter().chain(&b) {
            let name = name.file_name().unwrap().to_str().unwrap();
            let rest = name.strip_prefix(&prefix).unwrap();
            let (_, nonce) = rest.split_once('-').unwrap();
            assert_eq!(nonce.len(), 16, "{name}");
        }
        assert_ne!(a[0], b[0]);
        assert_ne!(a[0], a[1]);
    }

    #[cfg(unix)]
    #[test]
    fn insecure_key_file_permissions_are_rejected() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new("origin-key-perm");
        let file = OriginKeyFile::new(dir.path.join("k")).unwrap();
        write_private_file(file.path(), key(5).to_file_text().as_bytes()).unwrap();
        std::fs::set_permissions(file.path(), std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(
            file.load().unwrap_err(),
            OriginKeyError::InsecurePermissions {
                path: file.path().to_path_buf(),
                mode: 0o644
            }
        );
    }

    #[test]
    fn key_file_path_must_be_absolute_and_outside_project() {
        // 중간 `.`는 `Path::components`가 정규화하므로 `..`와 상대 경로만 거부 대상입니다.
        for bad in ["relative/k", "./k", "/a/../k"] {
            assert_eq!(
                OriginKeyFile::new(bad).unwrap_err(),
                OriginKeyError::InvalidPath { path: bad.into() }
            );
        }

        let dir = TempDir::new("origin-key-outside");
        let project = dir.path.join("project");
        let data = dir.path.join("data");
        std::fs::create_dir_all(&project).unwrap();
        let inside = OriginKeyFile::new(project.join(".stellar").join("origin-key")).unwrap();
        assert_eq!(
            inside.ensure_outside(&project).unwrap_err(),
            OriginKeyError::InsideProject {
                path: inside.path().to_path_buf()
            }
        );
        let outside = OriginKeyFile::in_data_dir(&data).unwrap();
        outside.ensure_outside(&project).unwrap();
        // 접두 문자열만 같은 형제 디렉터리는 밖입니다.
        let sibling = OriginKeyFile::new(dir.path.join("project-old").join("k")).unwrap();
        sibling.ensure_outside(&project).unwrap();
        // symlink를 거쳐 프로젝트 안으로 들어오는 경로도 안으로 판정합니다.
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&project, dir.path.join("alias")).unwrap();
            let via_link = OriginKeyFile::new(dir.path.join("alias").join("k")).unwrap();
            assert!(matches!(
                via_link.ensure_outside(&project).unwrap_err(),
                OriginKeyError::InsideProject { .. }
            ));
        }
    }

    #[test]
    fn errors_are_descriptive() {
        assert!(OriginKeyError::Unconfigured.to_string().contains("key"));
        assert!(ParseOriginFingerprintError::UnknownScheme
            .to_string()
            .contains("hmac-sha256"));
    }
}
