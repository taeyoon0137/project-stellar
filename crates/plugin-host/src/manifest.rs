//! `plugin.yaml` manifest 형식 (XXI.1.1, ADR-0097 결정 1).
//!
//! `.stellar-plugin` package 최상위의 `plugin.yaml`을 제한 YAML loader(`stellar_project::yaml`, IV.1.2)로 읽고
//! 여기서 필드를 엄격하게 해석합니다. 모르는 key는 거부합니다(fail closed). 형식이 바뀌면 `manifest_version`을
//! 올립니다.
//!
//! ```yaml
//! manifest_version: 1
//! id: com.example.hello
//! version: 1.2.0
//! name: Hello
//! entry: ui/index.html
//! host_api: 1
//! permissions:
//!   - document.read
//! wasm:
//!   - path: wasm/calc.wasm
//! ```
//!
//! JSON Schema(`schemas/plugin-manifest.schema.json`)는 [`manifest_json_schema`]가 이 모듈의 상수로 생성합니다.
//! 사람이 schema를 따로 고치지 않습니다(`tests/schema_drift.rs`).

use std::fmt;

use stellar_project::yaml::scalar::parse_u32;
use stellar_project::yaml::{load_restricted, LoaderError, YamlValue};

use crate::path::{PackagePath, PathError};
use crate::permission::Permission;

/// 이 빌드가 읽는 manifest 형식 버전.
pub const MANIFEST_VERSION: u32 = 1;

/// manifest 파일 이름.
pub const MANIFEST_FILE: &str = "plugin.yaml";

/// manifest 최대 byte 수.
pub const MAX_MANIFEST_BYTES: usize = 64 * 1024;

/// id 최대 byte 수.
pub const MAX_ID_BYTES: usize = 128;

/// 표시 이름 최대 문자 수.
pub const MAX_NAME_CHARS: usize = 128;

/// WASM module 최대 개수.
pub const MAX_WASM_MODULES: usize = 16;

/// 최상위 key(문서화 순서). `wasm`만 선택입니다.
pub const TOP_LEVEL_KEYS: [&str; 8] = [
    "manifest_version",
    "id",
    "version",
    "name",
    "entry",
    "host_api",
    "permissions",
    "wasm",
];

/// 필수 key.
pub const REQUIRED_KEYS: [&str; 7] = [
    "manifest_version",
    "id",
    "version",
    "name",
    "entry",
    "host_api",
    "permissions",
];

/// id 형식(JSON Schema `pattern`과 같음): 소문자 reverse-DNS, 두 label 이상.
pub const ID_PATTERN: &str = "^[a-z][a-z0-9-]*(\\.[a-z][a-z0-9-]*)+$";

/// version 형식(JSON Schema `pattern`과 같음): SemVer core와 선택 pre-release. build metadata는 받지 않습니다.
pub const VERSION_PATTERN: &str =
    "^(0|[1-9][0-9]*)\\.(0|[1-9][0-9]*)\\.(0|[1-9][0-9]*)(-[0-9A-Za-z-]+(\\.[0-9A-Za-z-]+)*)?$";

/// 플러그인 id(소문자 reverse-DNS).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PluginId(String);

impl PluginId {
    /// [`ID_PATTERN`]과 [`MAX_ID_BYTES`]로 검사합니다.
    pub fn parse(text: &str) -> Option<PluginId> {
        if text.is_empty() || text.len() > MAX_ID_BYTES {
            return None;
        }
        let mut labels = 0;
        for label in text.split('.') {
            let mut bytes = label.bytes();
            let first = bytes.next()?;
            if !first.is_ascii_lowercase() {
                return None;
            }
            if !bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') {
                return None;
            }
            labels += 1;
        }
        (labels >= 2).then(|| PluginId(text.to_owned()))
    }

    /// id 문자열.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PluginId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// 플러그인 version(SemVer core + 선택 pre-release).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PluginVersion {
    /// major.
    pub major: u64,
    /// minor.
    pub minor: u64,
    /// patch.
    pub patch: u64,
    /// pre-release 식별자(`-` 뒤, 없으면 빈 문자열).
    pub pre: String,
}

fn canonical_number(text: &str) -> Option<u64> {
    if text.is_empty()
        || !text.bytes().all(|b| b.is_ascii_digit())
        || (text.len() > 1 && text.starts_with('0'))
    {
        return None;
    }
    text.parse().ok()
}

impl PluginVersion {
    /// [`VERSION_PATTERN`]으로 검사합니다.
    pub fn parse(text: &str) -> Option<PluginVersion> {
        let (core, pre) = match text.split_once('-') {
            Some((core, pre)) => (core, pre),
            None => (text, ""),
        };
        if text.contains('+') {
            return None;
        }
        if text.contains('-')
            && pre.split('.').any(|id| {
                id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            })
        {
            return None;
        }
        let mut parts = core.split('.');
        let major = canonical_number(parts.next()?)?;
        let minor = canonical_number(parts.next()?)?;
        let patch = canonical_number(parts.next()?)?;
        if parts.next().is_some() {
            return None;
        }
        Some(PluginVersion {
            major,
            minor,
            patch,
            pre: pre.to_owned(),
        })
    }
}

impl fmt::Display for PluginVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if !self.pre.is_empty() {
            write!(f, "-{}", self.pre)?;
        }
        Ok(())
    }
}

/// WASM module 선언.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WasmModule {
    /// package 상대 경로(`.wasm`).
    pub path: PackagePath,
}

/// 해석한 manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginManifest {
    /// manifest 형식 버전(현재 [`MANIFEST_VERSION`]).
    pub manifest_version: u32,
    /// 플러그인 id.
    pub id: PluginId,
    /// 플러그인 version.
    pub version: PluginVersion,
    /// 표시 이름(식별에 쓰지 않음).
    pub name: String,
    /// WebView entry HTML(package 상대, `.html`).
    pub entry: PackagePath,
    /// 요구 Host API major version.
    pub host_api: u32,
    /// 요청 권한(이름 순 정렬, 중복 없음).
    pub permissions: Vec<Permission>,
    /// 선택 WASM module(선언 순).
    pub wasm: Vec<WasmModule>,
}

/// manifest 해석 오류. 문구에 manifest 원문 값을 넣지 않습니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestError {
    /// [`MAX_MANIFEST_BYTES`] 초과.
    TooLarge,
    /// 제한 YAML로 읽을 수 없음.
    Yaml(LoaderError),
    /// 최상위가 매핑이 아님.
    NotMapping,
    /// 모르는 key.
    UnknownKey {
        /// key 위치(`<top>` 또는 `wasm[i]`).
        at: String,
    },
    /// 필수 key 없음.
    MissingKey {
        /// key 이름.
        key: &'static str,
    },
    /// 값 형식 오류.
    InvalidValue {
        /// 필드 위치.
        field: String,
        /// 고정 원인.
        reason: &'static str,
    },
    /// 이 빌드가 읽지 못하는 manifest 형식 버전.
    UnsupportedManifestVersion {
        /// 선언 버전.
        found: u32,
    },
    /// package 경로 문법 오류.
    Path {
        /// 필드 위치.
        field: String,
        /// 원인.
        error: PathError,
    },
}

impl ManifestError {
    /// 고정 진단 code.
    pub fn code(&self) -> &'static str {
        match self {
            ManifestError::TooLarge => "manifest-too-large",
            ManifestError::Yaml(_) => "manifest-yaml",
            ManifestError::NotMapping => "manifest-not-mapping",
            ManifestError::UnknownKey { .. } => "manifest-unknown-key",
            ManifestError::MissingKey { .. } => "manifest-missing-key",
            ManifestError::InvalidValue { .. } => "manifest-invalid-value",
            ManifestError::UnsupportedManifestVersion { .. } => "unsupported-manifest-version",
            ManifestError::Path { error, .. } => error.code(),
        }
    }
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ManifestError::TooLarge => {
                write!(f, "plugin.yaml이 {MAX_MANIFEST_BYTES} bytes를 넘습니다")
            }
            ManifestError::Yaml(e) => write!(f, "plugin.yaml을 읽을 수 없습니다: {e}"),
            ManifestError::NotMapping => f.write_str("plugin.yaml 최상위는 매핑이어야 합니다"),
            ManifestError::UnknownKey { at } => write!(f, "{at}에 모르는 key가 있습니다"),
            ManifestError::MissingKey { key } => write!(f, "필수 key {key}가 없습니다"),
            ManifestError::InvalidValue { field, reason } => write!(f, "{field}: {reason}"),
            ManifestError::UnsupportedManifestVersion { found } => write!(
                f,
                "manifest_version {found}은 지원하지 않습니다(지원: {MANIFEST_VERSION})"
            ),
            ManifestError::Path { field, error } => write!(f, "{field}: {error}"),
        }
    }
}

impl std::error::Error for ManifestError {}

fn invalid(field: &str, reason: &'static str) -> ManifestError {
    ManifestError::InvalidValue {
        field: field.to_owned(),
        reason,
    }
}

fn scalar<'a>(value: &'a YamlValue, field: &str) -> Result<&'a str, ManifestError> {
    value
        .as_scalar()
        .ok_or_else(|| invalid(field, "스칼라여야 합니다"))
}

fn only_keys(
    pairs: &[(YamlValue, YamlValue)],
    allowed: &[&str],
    at: &str,
) -> Result<(), ManifestError> {
    for (key, _) in pairs {
        let known = key.as_scalar().is_some_and(|k| allowed.contains(&k));
        if !known {
            return Err(ManifestError::UnknownKey { at: at.to_owned() });
        }
    }
    Ok(())
}

fn package_path(value: &YamlValue, field: &str, ext: &str) -> Result<PackagePath, ManifestError> {
    let path = PackagePath::parse(scalar(value, field)?).map_err(|error| ManifestError::Path {
        field: field.to_owned(),
        error,
    })?;
    if path.extension() != Some(ext) {
        return Err(invalid(
            field,
            if ext == "html" {
                "확장자가 .html이어야 합니다"
            } else {
                "확장자가 .wasm이어야 합니다"
            },
        ));
    }
    if path.as_str() == MANIFEST_FILE {
        return Err(invalid(field, "plugin.yaml을 가리킬 수 없습니다"));
    }
    Ok(path)
}

impl PluginManifest {
    /// `plugin.yaml` 텍스트를 해석합니다.
    pub fn parse(text: &str) -> Result<PluginManifest, ManifestError> {
        if text.len() > MAX_MANIFEST_BYTES {
            return Err(ManifestError::TooLarge);
        }
        let root = load_restricted(text).map_err(ManifestError::Yaml)?;
        let pairs = root.as_mapping().ok_or(ManifestError::NotMapping)?;
        only_keys(pairs, &TOP_LEVEL_KEYS, "<top>")?;
        for key in REQUIRED_KEYS {
            if root.get(key).is_none() {
                return Err(ManifestError::MissingKey { key });
            }
        }
        let get = |key: &str| root.get(key).expect("필수 key 확인됨");

        let manifest_version = parse_u32(scalar(get("manifest_version"), "manifest_version")?)
            .ok_or_else(|| invalid("manifest_version", "음이 아닌 정수여야 합니다"))?;
        if manifest_version != MANIFEST_VERSION {
            return Err(ManifestError::UnsupportedManifestVersion {
                found: manifest_version,
            });
        }
        let id = PluginId::parse(scalar(get("id"), "id")?)
            .ok_or_else(|| invalid("id", "소문자 reverse-DNS id여야 합니다"))?;
        let version = PluginVersion::parse(scalar(get("version"), "version")?)
            .ok_or_else(|| invalid("version", "SemVer MAJOR.MINOR.PATCH[-pre]여야 합니다"))?;
        let name = scalar(get("name"), "name")?.to_owned();
        if name.trim().is_empty()
            || name.chars().count() > MAX_NAME_CHARS
            || name.chars().any(char::is_control)
        {
            return Err(invalid(
                "name",
                "비어 있지 않은 128자 이하 한 줄 문자열이어야 합니다",
            ));
        }
        let entry = package_path(get("entry"), "entry", "html")?;
        let host_api = parse_u32(scalar(get("host_api"), "host_api")?)
            .filter(|v| *v >= 1)
            .ok_or_else(|| invalid("host_api", "1 이상의 정수여야 합니다"))?;

        let items = get("permissions")
            .as_sequence()
            .ok_or_else(|| invalid("permissions", "시퀀스여야 합니다"))?;
        let mut permissions = Vec::with_capacity(items.len());
        for (i, item) in items.iter().enumerate() {
            let field = format!("permissions[{i}]");
            let permission = Permission::from_name(scalar(item, &field)?)
                .ok_or_else(|| invalid(&field, "모르는 권한입니다"))?;
            if permissions.contains(&permission) {
                return Err(invalid(&field, "중복 권한입니다"));
            }
            permissions.push(permission);
        }
        permissions.sort_unstable();

        let mut wasm = Vec::new();
        if let Some(value) = root.get("wasm") {
            let items = value
                .as_sequence()
                .ok_or_else(|| invalid("wasm", "시퀀스여야 합니다"))?;
            if items.len() > MAX_WASM_MODULES {
                return Err(invalid("wasm", "module은 16개 이하여야 합니다"));
            }
            for (i, item) in items.iter().enumerate() {
                let at = format!("wasm[{i}]");
                let pairs = item
                    .as_mapping()
                    .ok_or_else(|| invalid(&at, "매핑이어야 합니다"))?;
                only_keys(pairs, &["path"], &at)?;
                let field = format!("{at}.path");
                let path = item
                    .get("path")
                    .ok_or(ManifestError::MissingKey { key: "wasm[].path" })?;
                let path = package_path(path, &field, "wasm")?;
                if wasm.iter().any(|m: &WasmModule| m.path == path) {
                    return Err(invalid(&field, "중복 경로입니다"));
                }
                wasm.push(WasmModule { path });
            }
        }

        Ok(PluginManifest {
            manifest_version,
            id,
            version,
            name,
            entry,
            host_api,
            permissions,
            wasm,
        })
    }

    /// manifest가 선언한 권한인지.
    pub fn declares(&self, permission: Permission) -> bool {
        self.permissions.contains(&permission)
    }
}

fn json_string(text: &str) -> String {
    crate::json::write(&crate::json::JsonValue::string(text))
}

/// `schemas/plugin-manifest.schema.json` 내용(JSON Schema 2020-12). 이 모듈 상수에서만 만듭니다.
pub fn manifest_json_schema() -> String {
    let permissions = Permission::ALL
        .iter()
        .map(|p| json_string(p.name()))
        .collect::<Vec<_>>()
        .join(", ");
    let required = REQUIRED_KEYS
        .iter()
        .map(|k| json_string(k))
        .collect::<Vec<_>>()
        .join(", ");
    let path_pattern =
        json_string("^(?!/)(?!.*(^|/)\\.{1,2}(/|$))(?!.*//)[^\\\\:\\u0000-\\u001f]+$");
    format!(
        r##"{{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://stellar.taeyoon.xyz/schemas/plugin-manifest.schema.json",
  "title": "Stellar plugin.yaml (manifest_version {MANIFEST_VERSION})",
  "description": "생성 파일입니다. crates/plugin-host/src/manifest.rs의 상수에서 stellar-plugin-bindgen이 만듭니다(ADR-0097).",
  "type": "object",
  "additionalProperties": false,
  "required": [{required}],
  "properties": {{
    "manifest_version": {{ "const": {MANIFEST_VERSION} }},
    "id": {{ "type": "string", "maxLength": {MAX_ID_BYTES}, "pattern": {id} }},
    "version": {{ "type": "string", "pattern": {version} }},
    "name": {{ "type": "string", "minLength": 1, "maxLength": {MAX_NAME_CHARS} }},
    "entry": {{ "$ref": "#/$defs/packagePath", "pattern": "\\.html$" }},
    "host_api": {{ "type": "integer", "minimum": 1 }},
    "permissions": {{
      "type": "array",
      "uniqueItems": true,
      "items": {{ "enum": [{permissions}] }}
    }},
    "wasm": {{
      "type": "array",
      "maxItems": {MAX_WASM_MODULES},
      "items": {{
        "type": "object",
        "additionalProperties": false,
        "required": ["path"],
        "properties": {{ "path": {{ "$ref": "#/$defs/packagePath", "pattern": "\\.wasm$" }} }}
      }}
    }}
  }},
  "$defs": {{
    "packagePath": {{
      "type": "string",
      "minLength": 1,
      "maxLength": {max_path},
      "pattern": {path_pattern}
    }}
  }}
}}
"##,
        id = json_string(ID_PATTERN),
        version = json_string(VERSION_PATTERN),
        max_path = crate::path::MAX_PATH_BYTES,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = "\
manifest_version: 1
id: com.example.hello
version: 1.2.0-beta.1
name: Hello
entry: ui/index.html
host_api: 1
permissions:
  - git.read
  - document.read
wasm:
  - path: wasm/calc.wasm
";

    #[test]
    fn parses_valid_manifest() {
        let m = PluginManifest::parse(VALID).unwrap();
        assert_eq!(m.id.as_str(), "com.example.hello");
        assert_eq!(m.version.to_string(), "1.2.0-beta.1");
        assert_eq!(m.entry.as_str(), "ui/index.html");
        assert_eq!(m.host_api, 1);
        assert_eq!(
            m.permissions,
            [Permission::DocumentRead, Permission::GitRead]
        );
        assert_eq!(m.wasm.len(), 1);
        assert!(m.declares(Permission::GitRead));
        assert!(!m.declares(Permission::Network));
    }

    #[test]
    fn wasm_is_optional_and_permissions_may_be_empty() {
        let text = "manifest_version: 1\nid: a.b\nversion: 0.0.1\nname: X\nentry: i.html\nhost_api: 1\npermissions: []\n";
        let m = PluginManifest::parse(text).unwrap();
        assert!(m.wasm.is_empty());
        assert!(m.permissions.is_empty());
    }

    fn code(text: &str) -> &'static str {
        PluginManifest::parse(text).unwrap_err().code()
    }

    #[test]
    fn rejects_bad_fields() {
        assert_eq!(
            code(&VALID.replace("manifest_version: 1", "manifest_version: 2")),
            "unsupported-manifest-version"
        );
        assert_eq!(code(&format!("{VALID}extra: 1\n")), "manifest-unknown-key");
        assert_eq!(
            code(&VALID.replace("host_api: 1\n", "")),
            "manifest-missing-key"
        );
        assert_eq!(
            code(&VALID.replace("com.example.hello", "Hello")),
            "manifest-invalid-value"
        );
        assert_eq!(
            code(&VALID.replace("com.example.hello", "single")),
            "manifest-invalid-value"
        );
        assert_eq!(
            code(&VALID.replace("1.2.0-beta.1", "1.2")),
            "manifest-invalid-value"
        );
        assert_eq!(
            code(&VALID.replace("1.2.0-beta.1", "01.2.0")),
            "manifest-invalid-value"
        );
        assert_eq!(
            code(&VALID.replace("1.2.0-beta.1", "1.2.0+build")),
            "manifest-invalid-value"
        );
        assert_eq!(
            code(&VALID.replace("ui/index.html", "../index.html")),
            "path-traversal"
        );
        assert_eq!(
            code(&VALID.replace("ui/index.html", "/index.html")),
            "path-absolute"
        );
        assert_eq!(
            code(&VALID.replace("ui/index.html", "ui/index.js")),
            "manifest-invalid-value"
        );
        assert_eq!(
            code(&VALID.replace("git.read", "shell")),
            "manifest-invalid-value"
        );
        assert_eq!(
            code(&VALID.replace("git.read", "document.read")),
            "manifest-invalid-value"
        );
        assert_eq!(
            code(&VALID.replace("wasm/calc.wasm", "wasm/../../x.wasm")),
            "path-traversal"
        );
        assert_eq!(
            code(&VALID.replace("host_api: 1", "host_api: 0")),
            "manifest-invalid-value"
        );
        assert_eq!(code("- a\n"), "manifest-not-mapping");
        assert_eq!(code("a: &x 1\nb: *x\n"), "manifest-yaml");
    }

    #[test]
    fn schema_lists_every_permission_and_required_key() {
        let schema = manifest_json_schema();
        let parsed = crate::json::parse(&schema, crate::json::JsonLimits::default()).unwrap();
        let required = parsed.get("required").and_then(|v| v.as_array()).unwrap();
        assert_eq!(required.len(), REQUIRED_KEYS.len());
        for p in Permission::ALL {
            assert!(schema.contains(&format!("\"{}\"", p.name())));
        }
    }
}
