//! 플러그인별 권한 승인·저장 (XXI.4.5, ADR-0097 결정 5).
//!
//! - 유효 권한 = manifest 선언 ∩ (session 승인 ∪ 저장 승인). 선언하지 않은 권한은 승인할 수 없습니다.
//! - session 승인은 메모리에만 있고 앱이 끝나면 사라집니다(기술 아키텍처 §37.3 세션 단위 승인).
//! - 저장 승인은 plugin id별로 승인 당시 package digest와 함께 파일에 둡니다. digest가 바뀐(업데이트된) package에는
//!   저장 승인을 쓰지 않습니다. 같은 id의 새 digest를 저장 승인하면 예전 항목을 바꿉니다.
//! - 파일 형식: `{"format":1,"plugins":[{"id":…,"digest":"sha256:…","permissions":[…]}]}`(id 순, 권한 이름 순).
//!   쓰기는 임시 파일 뒤 rename입니다. 모르는 권한·형식은 열기에서 거부합니다(fail closed).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};

use crate::admission::AdmittedPlugin;
use crate::json::{parse, write, JsonLimits, JsonValue as J};
use crate::manifest::PluginId;
use crate::permission::Permission;

/// 저장 파일 형식 버전.
pub const GRANTS_FORMAT: i64 = 1;

/// 앱 사용자 데이터 안 저장 파일 상대 경로.
pub const GRANTS_FILE: &str = "plugins/grants.json";

/// 저장 파일 최대 byte 수.
pub const MAX_GRANTS_BYTES: usize = 1024 * 1024;

/// 승인 범위.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantScope {
    /// 이번 앱 실행 동안만.
    Session,
    /// 파일에 저장(같은 package digest 동안).
    Persistent,
}

/// 승인 저장 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrantError {
    /// manifest가 선언하지 않은 권한.
    NotDeclared(Permission),
    /// 저장 파일 형식 오류.
    Corrupt,
    /// 파일 읽기·쓰기 실패.
    Io(std::io::ErrorKind),
}

impl GrantError {
    /// 고정 진단 code.
    pub fn code(&self) -> &'static str {
        match self {
            GrantError::NotDeclared(_) => "permission-not-declared",
            GrantError::Corrupt => "grants-corrupt",
            GrantError::Io(_) => "grants-io",
        }
    }
}

impl fmt::Display for GrantError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GrantError::NotDeclared(p) => {
                write!(f, "manifest가 선언하지 않은 권한 {p}은 승인할 수 없습니다")
            }
            GrantError::Corrupt => f.write_str("권한 저장 파일 형식이 틀렸습니다"),
            GrantError::Io(kind) => write!(f, "권한 저장 파일을 읽거나 쓰지 못했습니다({kind:?})"),
        }
    }
}

impl std::error::Error for GrantError {}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Stored {
    digest: String,
    permissions: BTreeSet<Permission>,
}

/// 승인 저장소.
#[derive(Debug)]
pub struct GrantStore {
    path: PathBuf,
    persistent: BTreeMap<PluginId, Stored>,
    session: BTreeMap<PluginId, BTreeSet<Permission>>,
}

/// OS 사용자 데이터의 기본 저장 파일 위치(`<OS 사용자 데이터>/xyz.taeyoon.stellar/plugins/grants.json`).
pub fn default_grants_path() -> Option<PathBuf> {
    stellar_ops::trusted::os_trust_data_dir().map(|dir| dir.join(GRANTS_FILE))
}

fn parse_file(text: &str) -> Option<BTreeMap<PluginId, Stored>> {
    let root = parse(
        text,
        JsonLimits {
            max_bytes: MAX_GRANTS_BYTES,
            ..JsonLimits::default()
        },
    )
    .ok()?;
    let pairs = root.as_object()?;
    if pairs.iter().any(|(k, _)| k != "format" && k != "plugins")
        || root.get("format").and_then(J::as_i64) != Some(GRANTS_FORMAT)
    {
        return None;
    }
    let mut out = BTreeMap::new();
    for item in root.get("plugins")?.as_array()? {
        let keys = item.as_object()?;
        if keys
            .iter()
            .any(|(k, _)| !["id", "digest", "permissions"].contains(&k.as_str()))
        {
            return None;
        }
        let id = PluginId::parse(item.get("id")?.as_str()?)?;
        let digest = item.get("digest")?.as_str()?.to_owned();
        if !digest.starts_with("sha256:") {
            return None;
        }
        let mut permissions = BTreeSet::new();
        for name in item.get("permissions")?.as_array()? {
            permissions.insert(Permission::from_name(name.as_str()?)?);
        }
        if out
            .insert(
                id,
                Stored {
                    digest,
                    permissions,
                },
            )
            .is_some()
        {
            return None;
        }
    }
    Some(out)
}

impl GrantStore {
    /// 저장 파일을 엽니다. 파일이 없으면 빈 저장소입니다(만들지 않음).
    pub fn open(path: &Path) -> Result<GrantStore, GrantError> {
        let persistent = match std::fs::read(path) {
            Ok(bytes) => {
                if bytes.len() > MAX_GRANTS_BYTES {
                    return Err(GrantError::Corrupt);
                }
                let text = String::from_utf8(bytes).map_err(|_| GrantError::Corrupt)?;
                parse_file(&text).ok_or(GrantError::Corrupt)?
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => return Err(GrantError::Io(e.kind())),
        };
        Ok(GrantStore {
            path: path.to_path_buf(),
            persistent,
            session: BTreeMap::new(),
        })
    }

    fn save(&self) -> Result<(), GrantError> {
        let plugins = self
            .persistent
            .iter()
            .map(|(id, stored)| {
                J::object(vec![
                    ("id", J::string(id.as_str())),
                    ("digest", J::string(stored.digest.clone())),
                    (
                        "permissions",
                        J::Array(
                            stored
                                .permissions
                                .iter()
                                .map(|p| J::string(p.name()))
                                .collect(),
                        ),
                    ),
                ])
            })
            .collect();
        let text = write(&J::object(vec![
            ("format", J::int(GRANTS_FORMAT)),
            ("plugins", J::Array(plugins)),
        ])) + "\n";
        let io = |e: std::io::Error| GrantError::Io(e.kind());
        let parent = self
            .path
            .parent()
            .ok_or(GrantError::Io(std::io::ErrorKind::InvalidInput))?;
        std::fs::create_dir_all(parent).map_err(io)?;
        let temp = parent.join(format!(".grants-{}.tmp", std::process::id()));
        std::fs::write(&temp, text).map_err(io)?;
        std::fs::rename(&temp, &self.path).map_err(|e| {
            let _ = std::fs::remove_file(&temp);
            io(e)
        })
    }

    /// 권한을 승인합니다. 저장 범위면 파일에 씁니다.
    pub fn grant(
        &mut self,
        plugin: &AdmittedPlugin,
        permission: Permission,
        scope: GrantScope,
    ) -> Result<(), GrantError> {
        if !plugin.manifest().declares(permission) {
            return Err(GrantError::NotDeclared(permission));
        }
        let id = plugin.manifest().id.clone();
        match scope {
            GrantScope::Session => {
                self.session.entry(id).or_default().insert(permission);
                Ok(())
            }
            GrantScope::Persistent => {
                let digest = plugin.package().digest().to_string();
                let entry = self.persistent.entry(id).or_insert_with(|| Stored {
                    digest: digest.clone(),
                    permissions: BTreeSet::new(),
                });
                if entry.digest != digest {
                    *entry = Stored {
                        digest,
                        permissions: BTreeSet::new(),
                    };
                }
                entry.permissions.insert(permission);
                self.save()
            }
        }
    }

    /// 권한 승인을 session·저장 모두에서 철회합니다.
    pub fn revoke(&mut self, id: &PluginId, permission: Permission) -> Result<(), GrantError> {
        if let Some(set) = self.session.get_mut(id) {
            set.remove(&permission);
        }
        let changed = self
            .persistent
            .get_mut(id)
            .is_some_and(|stored| stored.permissions.remove(&permission));
        if changed {
            if self.persistent[id].permissions.is_empty() {
                self.persistent.remove(id);
            }
            self.save()?;
        }
        Ok(())
    }

    /// 플러그인의 모든 승인을 철회합니다(플러그인 제거).
    pub fn forget(&mut self, id: &PluginId) -> Result<(), GrantError> {
        self.session.remove(id);
        if self.persistent.remove(id).is_some() {
            self.save()?;
        }
        Ok(())
    }

    /// 유효 권한: manifest 선언 ∩ (session 승인 ∪ digest가 같은 저장 승인).
    pub fn effective(&self, plugin: &AdmittedPlugin) -> BTreeSet<Permission> {
        let manifest = plugin.manifest();
        let digest = plugin.package().digest().to_string();
        let mut granted: BTreeSet<Permission> =
            self.session.get(&manifest.id).cloned().unwrap_or_default();
        if let Some(stored) = self.persistent.get(&manifest.id) {
            if stored.digest == digest {
                granted.extend(stored.permissions.iter().copied());
            }
        }
        granted.retain(|p| manifest.declares(*p));
        granted
    }
}
