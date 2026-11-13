//! 실행 중 Stellar 앱 endpoint registry (XIX.1.2).
//!
//! 브리지는 network port 없이, 사용자 전용 디렉터리의 Unix domain socket으로 실행 중인 앱을
//! 찾습니다.
//!
//! ## 위치와 형식(앱 측 계약)
//!
//! - registry 디렉터리: 사용자 임시 디렉터리([`super::os::user_temp_dir`]) 아래
//!   `stellar-ipc-<uid>`. 환경 변수에 의존하지 않습니다.
//! - 디렉터리는 **현재 사용자 소유, symlink 아님, 권한 정확히 0700**이어야 합니다. 조건을 하나라도
//!   어기면 어떤 항목도 신뢰하지 않고 [`RegistryError::Insecure`]를 반환합니다.
//! - 실행 중인 앱 instance마다 Unix domain socket `<instance-id>.sock` 하나를 둡니다. instance id는
//!   소문자 16진수 16자입니다. 앱은 종료 시 자기 socket 파일을 지웁니다.
//! - 이름 규칙을 벗어난 항목, socket이 아닌 항목(일반 파일·symlink), 다른 사용자 소유 항목은
//!   무시합니다. listener가 없는 socket 파일(비정상 종료 잔재)은 stale로 보고하고 연결 후보에서
//!   제외합니다. 브리지는 registry를 수정하지 않습니다.
//! - socket 경로는 macOS `sun_path` 한도(NUL 제외 103 byte)를 넘을 수 없습니다.

use std::fmt;
use std::io;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use super::os;

/// registry 디렉터리 이름 접두사(뒤에 uid).
pub const REGISTRY_DIR_PREFIX: &str = "stellar-ipc-";
/// endpoint socket 파일 확장자.
pub const ENDPOINT_EXTENSION: &str = "sock";
/// instance id 길이(소문자 16진수 문자 수).
pub const INSTANCE_ID_LEN: usize = 16;
/// socket 경로 최대 byte 수(macOS `sun_path` 104 byte - NUL).
pub const MAX_SOCKET_PATH_BYTES: usize = 103;

/// 앱 instance id(소문자 16진수 16자).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InstanceId(String);

impl InstanceId {
    /// 규칙에 맞으면 instance id로 읽습니다.
    pub fn parse(text: &str) -> Option<InstanceId> {
        let valid = text.len() == INSTANCE_ID_LEN
            && text
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        valid.then(|| InstanceId(text.to_string()))
    }

    /// 8 byte 값을 16진수 id로 만듭니다.
    pub fn from_bytes(bytes: [u8; 8]) -> InstanceId {
        InstanceId(bytes.iter().map(|b| format!("{b:02x}")).collect())
    }

    /// 문자열 표기.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// socket 파일 이름(`<id>.sock`).
    pub fn file_name(&self) -> String {
        format!("{}.{ENDPOINT_EXTENSION}", self.0)
    }
}

impl fmt::Display for InstanceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// 연결 가능한 앱 endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    /// 앱 instance id.
    pub instance: InstanceId,
    /// Unix domain socket 경로.
    pub socket_path: PathBuf,
}

/// registry 디렉터리를 신뢰할 수 없는 이유.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsecureReason {
    /// 디렉터리 자체가 symlink입니다.
    Symlink,
    /// 디렉터리가 아닙니다.
    NotDirectory,
    /// 다른 사용자 소유입니다.
    WrongOwner {
        /// 실제 소유자 uid.
        owner: u32,
    },
    /// 권한이 0700이 아닙니다.
    WrongMode {
        /// 실제 권한 bit(하위 12 bit).
        mode: u32,
    },
}

/// registry 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryError {
    /// registry 디렉터리를 신뢰할 수 없습니다.
    Insecure {
        /// 이유.
        reason: InsecureReason,
    },
    /// socket 경로가 OS 한도를 넘습니다.
    PathTooLong {
        /// 경로 byte 수.
        bytes: usize,
    },
    /// 입출력 오류.
    Io {
        /// io 오류 종류.
        kind: io::ErrorKind,
    },
}

impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RegistryError::Insecure { reason } => match reason {
                InsecureReason::Symlink => write!(f, "IPC registry 디렉터리가 symlink입니다"),
                InsecureReason::NotDirectory => {
                    write!(f, "IPC registry 경로가 디렉터리가 아닙니다")
                }
                InsecureReason::WrongOwner { owner } => {
                    write!(f, "IPC registry 디렉터리 소유자가 다릅니다(uid {owner})")
                }
                InsecureReason::WrongMode { mode } => {
                    write!(
                        f,
                        "IPC registry 디렉터리 권한이 0700이 아닙니다({mode:04o})"
                    )
                }
            },
            RegistryError::PathTooLong { bytes } => write!(
                f,
                "IPC socket 경로가 너무 깁니다({bytes} > {MAX_SOCKET_PATH_BYTES} bytes)"
            ),
            RegistryError::Io { kind } => write!(f, "IPC registry 입출력 오류: {kind:?}"),
        }
    }
}

impl std::error::Error for RegistryError {}

/// 탐색 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Discovery {
    /// 실행 중인 앱이 없습니다.
    NoApp,
    /// 앱이 하나 있습니다.
    One(Endpoint),
    /// 앱이 여러 개 있습니다(instance id 순).
    Many(Vec<Endpoint>),
}

/// endpoint 선택 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectError {
    /// 실행 중인 앱이 없습니다.
    NoApp,
    /// 앱이 여러 개라 자동으로 고를 수 없습니다.
    Ambiguous {
        /// 후보 instance id(정렬).
        candidates: Vec<InstanceId>,
    },
    /// 지정한 instance가 없습니다.
    NotFound {
        /// 찾은 id.
        wanted: InstanceId,
    },
}

impl fmt::Display for SelectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SelectError::NoApp => write!(f, "실행 중인 Stellar 앱이 없습니다"),
            SelectError::Ambiguous { candidates } => write!(
                f,
                "실행 중인 Stellar 앱이 {}개입니다. instance를 지정하세요",
                candidates.len()
            ),
            SelectError::NotFound { wanted } => {
                write!(f, "Stellar 앱 instance {wanted}를 찾을 수 없습니다")
            }
        }
    }
}

impl std::error::Error for SelectError {}

impl Discovery {
    /// 발견한 endpoint(instance id 순).
    pub fn endpoints(&self) -> &[Endpoint] {
        match self {
            Discovery::NoApp => &[],
            Discovery::One(endpoint) => std::slice::from_ref(endpoint),
            Discovery::Many(endpoints) => endpoints,
        }
    }

    /// 연결할 endpoint를 고릅니다. 여러 개이면 `wanted`가 필요합니다.
    pub fn select(&self, wanted: Option<&InstanceId>) -> Result<&Endpoint, SelectError> {
        if let Some(wanted) = wanted {
            return self
                .endpoints()
                .iter()
                .find(|e| &e.instance == wanted)
                .ok_or_else(|| SelectError::NotFound {
                    wanted: wanted.clone(),
                });
        }
        match self {
            Discovery::NoApp => Err(SelectError::NoApp),
            Discovery::One(endpoint) => Ok(endpoint),
            Discovery::Many(endpoints) => Err(SelectError::Ambiguous {
                candidates: endpoints.iter().map(|e| e.instance.clone()).collect(),
            }),
        }
    }
}

/// 탐색 보고.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveryReport {
    /// 연결 가능한 앱.
    pub discovery: Discovery,
    /// listener가 없는 socket 파일(instance id 순).
    pub stale: Vec<InstanceId>,
    /// 규칙에 맞지 않아 무시한 항목 수.
    pub ignored: usize,
}

/// 사용자별 endpoint registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registry {
    dir: PathBuf,
    uid: u32,
}

impl Registry {
    /// 주어진 디렉터리와 기대 소유자 uid의 registry.
    pub fn at(dir: impl Into<PathBuf>, uid: u32) -> Registry {
        Registry {
            dir: dir.into(),
            uid,
        }
    }

    /// 현재 사용자의 기본 registry(`<사용자 임시 디렉터리>/stellar-ipc-<uid>`).
    pub fn for_current_user() -> Result<Registry, RegistryError> {
        let uid = os::current_uid();
        let base = os::user_temp_dir().map_err(|e| RegistryError::Io { kind: e.kind() })?;
        Ok(Registry::at(
            base.join(format!("{REGISTRY_DIR_PREFIX}{uid}")),
            uid,
        ))
    }

    /// registry 디렉터리.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// 기대 소유자 uid.
    pub fn uid(&self) -> u32 {
        self.uid
    }

    /// instance의 socket 경로. OS 한도를 넘으면 오류입니다.
    pub fn socket_path(&self, instance: &InstanceId) -> Result<PathBuf, RegistryError> {
        let path = self.dir.join(instance.file_name());
        let bytes = path.as_os_str().len();
        if bytes > MAX_SOCKET_PATH_BYTES {
            return Err(RegistryError::PathTooLong { bytes });
        }
        Ok(path)
    }

    /// 디렉터리를 검사합니다. 없으면 `Ok(false)`, 안전하면 `Ok(true)`입니다.
    pub fn verify(&self) -> Result<bool, RegistryError> {
        let meta = match std::fs::symlink_metadata(&self.dir) {
            Ok(meta) => meta,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(RegistryError::Io { kind: e.kind() }),
        };
        let reason = if meta.file_type().is_symlink() {
            Some(InsecureReason::Symlink)
        } else if !meta.is_dir() {
            Some(InsecureReason::NotDirectory)
        } else if meta.uid() != self.uid {
            Some(InsecureReason::WrongOwner { owner: meta.uid() })
        } else if meta.mode() & 0o7777 != 0o700 {
            Some(InsecureReason::WrongMode {
                mode: meta.mode() & 0o7777,
            })
        } else {
            None
        };
        match reason {
            Some(reason) => Err(RegistryError::Insecure { reason }),
            None => Ok(true),
        }
    }

    /// 실행 중인 앱 endpoint를 찾습니다. 안전하지 않은 디렉터리의 항목은 신뢰하지 않습니다.
    ///
    /// 후보 socket마다 연결을 한 번 시도해(데이터는 보내지 않음) listener가 있는지 확인합니다.
    pub fn discover(&self) -> Result<DiscoveryReport, RegistryError> {
        if !self.verify()? {
            return Ok(DiscoveryReport {
                discovery: Discovery::NoApp,
                stale: Vec::new(),
                ignored: 0,
            });
        }
        let probe = InstanceId("0".repeat(INSTANCE_ID_LEN));
        self.socket_path(&probe)?;

        let entries =
            std::fs::read_dir(&self.dir).map_err(|e| RegistryError::Io { kind: e.kind() })?;
        let mut live = Vec::new();
        let mut stale = Vec::new();
        let mut ignored = 0usize;
        for entry in entries {
            let entry = entry.map_err(|e| RegistryError::Io { kind: e.kind() })?;
            let Some(instance) = entry
                .file_name()
                .to_str()
                .and_then(|name| name.strip_suffix(&format!(".{ENDPOINT_EXTENSION}")))
                .and_then(InstanceId::parse)
            else {
                ignored += 1;
                continue;
            };
            let path = entry.path();
            // symlink를 따르지 않고 항목 자체를 검사합니다.
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                ignored += 1;
                continue;
            };
            if !meta.file_type().is_socket() || meta.uid() != self.uid {
                ignored += 1;
                continue;
            }
            match UnixStream::connect(&path) {
                Ok(_probe) => live.push(Endpoint {
                    instance,
                    socket_path: path,
                }),
                Err(e) if e.kind() == io::ErrorKind::ConnectionRefused => stale.push(instance),
                Err(_) => ignored += 1,
            }
        }
        live.sort_by(|a, b| a.instance.cmp(&b.instance));
        stale.sort();
        let discovery = match live.len() {
            0 => Discovery::NoApp,
            1 => Discovery::One(live.remove(0)),
            _ => Discovery::Many(live),
        };
        Ok(DiscoveryReport {
            discovery,
            stale,
            ignored,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::test_support::{register, secure_registry, AppDouble, TempDir};
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixListener;

    fn id(n: u8) -> InstanceId {
        InstanceId::from_bytes([n; 8])
    }

    #[test]
    fn instance_ids_are_strict_lowercase_hex() {
        assert!(InstanceId::parse("0123456789abcdef").is_some());
        for bad in [
            "0123456789ABCDEF",
            "0123456789abcde",
            "0123456789abcdef0",
            "g123456789abcdef",
            "",
        ] {
            assert!(InstanceId::parse(bad).is_none(), "{bad}");
        }
        assert_eq!(id(0xab).as_str(), "abababababababab");
        assert_eq!(id(1).file_name(), "0101010101010101.sock");
    }

    #[test]
    fn fixture_no_app_when_registry_missing_or_empty() {
        let tmp = TempDir::new();
        let missing = Registry::at(tmp.path.join("absent"), os::current_uid());
        let report = missing.discover().unwrap();
        assert_eq!(report.discovery, Discovery::NoApp);
        assert_eq!(report.discovery.select(None), Err(SelectError::NoApp));

        let empty = secure_registry(&tmp, "empty");
        let report = empty.discover().unwrap();
        assert_eq!(report.discovery, Discovery::NoApp);
        assert_eq!((report.stale.len(), report.ignored), (0, 0));
    }

    #[test]
    fn fixture_no_app_when_only_stale_or_foreign_entries() {
        let tmp = TempDir::new();
        let registry = secure_registry(&tmp, "r");
        // listener를 닫은 socket 파일(비정상 종료 잔재).
        let stale = register(&registry, &id(1));
        drop(stale.listener);
        // 이름은 맞지만 일반 파일, 규칙 밖 이름의 socket, socket을 가리키는 symlink, 기타 파일.
        std::fs::write(registry.dir().join(id(2).file_name()), b"x").unwrap();
        let _odd = UnixListener::bind(registry.dir().join("not-an-id.sock")).unwrap();
        let live_elsewhere = TempDir::new();
        let target = live_elsewhere.path.join("t.sock");
        let _target = UnixListener::bind(&target).unwrap();
        std::os::unix::fs::symlink(&target, registry.dir().join(id(3).file_name())).unwrap();
        std::fs::write(registry.dir().join("README"), b"x").unwrap();

        let report = registry.discover().unwrap();
        assert_eq!(report.discovery, Discovery::NoApp);
        assert_eq!(report.stale, vec![id(1)]);
        assert_eq!(report.ignored, 4);
    }

    #[test]
    fn fixture_one_app_is_selected_automatically() {
        let tmp = TempDir::new();
        let registry = secure_registry(&tmp, "r");
        let app: AppDouble = register(&registry, &id(7));
        let report = registry.discover().unwrap();
        let Discovery::One(endpoint) = &report.discovery else {
            panic!("{report:?}")
        };
        assert_eq!(endpoint.instance, id(7));
        assert_eq!(endpoint.socket_path, app.endpoint.socket_path);
        assert_eq!(report.discovery.select(None), Ok(endpoint));
        assert_eq!(
            report.discovery.select(Some(&id(8))),
            Err(SelectError::NotFound { wanted: id(8) })
        );
    }

    #[test]
    fn fixture_many_apps_require_explicit_instance() {
        let tmp = TempDir::new();
        let registry = secure_registry(&tmp, "r");
        let _apps: Vec<AppDouble> = [9, 3, 5]
            .iter()
            .map(|n| register(&registry, &id(*n)))
            .collect();
        let stale = register(&registry, &id(4));
        drop(stale.listener);

        let report = registry.discover().unwrap();
        let ids: Vec<InstanceId> = report
            .discovery
            .endpoints()
            .iter()
            .map(|e| e.instance.clone())
            .collect();
        assert_eq!(ids, vec![id(3), id(5), id(9)], "instance id 순");
        assert!(matches!(report.discovery, Discovery::Many(_)));
        assert_eq!(report.stale, vec![id(4)]);
        assert_eq!(
            report.discovery.select(None),
            Err(SelectError::Ambiguous {
                candidates: vec![id(3), id(5), id(9)]
            })
        );
        assert_eq!(
            report
                .discovery
                .select(Some(&id(5)))
                .map(|e| e.instance.clone()),
            Ok(id(5))
        );
    }

    #[test]
    fn insecure_registry_directories_are_not_trusted() {
        let tmp = TempDir::new();
        let uid = os::current_uid();

        // 권한 0755: 다른 사용자가 목록을 볼 수 있음.
        let open = secure_registry(&tmp, "open");
        let _app = register(&open, &id(1));
        std::fs::set_permissions(open.dir(), std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            open.discover(),
            Err(RegistryError::Insecure {
                reason: InsecureReason::WrongMode { mode: 0o755 }
            })
        );

        // 다른 사용자 소유(기대 uid 불일치로 재현).
        let owned = secure_registry(&tmp, "owned");
        let other = Registry::at(owned.dir(), uid.wrapping_add(1));
        assert_eq!(
            other.discover(),
            Err(RegistryError::Insecure {
                reason: InsecureReason::WrongOwner { owner: uid }
            })
        );

        // symlink로 바꿔치기한 디렉터리.
        let real = secure_registry(&tmp, "real");
        let _real_app = register(&real, &id(2));
        let link = tmp.path.join("link");
        std::os::unix::fs::symlink(real.dir(), &link).unwrap();
        assert_eq!(
            Registry::at(&link, uid).discover(),
            Err(RegistryError::Insecure {
                reason: InsecureReason::Symlink
            })
        );

        // 디렉터리가 아닌 파일.
        let file = tmp.path.join("file");
        std::fs::write(&file, b"x").unwrap();
        assert_eq!(
            Registry::at(&file, uid).discover(),
            Err(RegistryError::Insecure {
                reason: InsecureReason::NotDirectory
            })
        );
    }

    #[test]
    fn socket_paths_beyond_os_limit_are_rejected() {
        let tmp = TempDir::new();
        let long = tmp.path.join("d".repeat(100));
        let registry = Registry::at(&long, os::current_uid());
        assert!(matches!(
            registry.socket_path(&id(1)),
            Err(RegistryError::PathTooLong { .. })
        ));
        std::fs::create_dir(&long).unwrap();
        std::fs::set_permissions(&long, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(matches!(
            registry.discover(),
            Err(RegistryError::PathTooLong { .. })
        ));
    }

    #[test]
    fn default_registry_is_per_user_and_env_independent() {
        let registry = Registry::for_current_user().unwrap();
        let uid = os::current_uid();
        assert_eq!(registry.uid(), uid);
        assert_eq!(
            registry.dir().parent(),
            Some(os::user_temp_dir().unwrap().as_path())
        );
        assert_eq!(
            registry.dir().file_name().and_then(|n| n.to_str()),
            Some(format!("{REGISTRY_DIR_PREFIX}{uid}").as_str())
        );
        assert!(
            registry.socket_path(&id(1)).is_ok(),
            "기본 위치는 경로 한도 안"
        );
    }
}
