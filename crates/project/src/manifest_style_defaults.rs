//! 기본값 세트 선택(`stellar.yaml` `style_defaults`)과 사용자 세트 파일 (SD.3, ADR-0082 결정 3·4, ADR-0086 결정 9).
//!
//! - 사용자 세트 파일은 `design/style-defaults/<name>.stellar.yaml`입니다. 형식은 `docs/spec/style-defaults.md`
//!   4장입니다. 의미 모델은 [`UserSetSource`](stellar_document::css::style_defaults::UserSetSource)이고, 값 해석은
//!   문서 crate의 longhand parser(`LayoutLonghand::parse`) 하나를 씁니다.
//! - 활성 세트는 매니페스트 키로 고르고 [`active_set`]이 `extends` 사슬을 풀어 평탄화합니다.
//! - [`set_active`]는 `stellar.yaml`을 읽은 bytes hash가 같을 때만 그 키 한 줄을 바꿔 씁니다. `css`를 고르면 키를
//!   지웁니다(키가 없으면 `css`). 프로젝트 저장 잠금은 호출자 몫입니다.

use std::collections::BTreeMap;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use stellar_document::css::style_defaults::{
    self, FlatSet, NodeClass, StyleDefaultsError, UserSetSource, UserSets,
};
use stellar_document::layout_record::{format_value, LayoutLonghand, LayoutValue};
use stellar_storage::hash::ContentHash;
use stellar_storage::source_file;

use crate::manifest::{Manifest, ManifestError};
use crate::root::MANIFEST_FILE;
use crate::yaml::loader::MAX_INPUT_BYTES;
use crate::yaml::{load_restricted, write_canonical_checked, LoaderError, YamlValue};

/// 사용자 세트 디렉터리(프로젝트 root 기준).
pub const STYLE_DEFAULTS_DIR: &str = "design/style-defaults";
/// 사용자 세트 파일 확장자.
pub const SET_FILE_SUFFIX: &str = ".stellar.yaml";
/// 사용자 세트 파일 수 상한.
pub const MAX_SET_FILES: usize = 256;

/// 기본값 세트 파일·선택 오류.
#[derive(Debug)]
pub enum StyleDefaultsFileError {
    /// 파일을 읽거나 쓰지 못했습니다.
    Io {
        /// 프로젝트 root 기준 경로.
        path: String,
        /// 원인.
        error: io::Error,
    },
    /// YAML 형식이 아닙니다(제한 loader).
    Yaml {
        /// 경로.
        path: String,
        /// 원인.
        error: LoaderError,
    },
    /// 파일 형식 위반(모르는 키·구획, 값 문법 등).
    Format {
        /// 경로.
        path: String,
        /// 진단 code.
        code: &'static str,
        /// 설명.
        message: String,
    },
    /// 매니페스트 오류.
    Manifest(ManifestError),
    /// 세트 평탄화 오류(순환, 없는 세트, 이름 …).
    Set(StyleDefaultsError),
}

impl fmt::Display for StyleDefaultsFileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StyleDefaultsFileError::Io { path, error } => write!(f, "{path}: {error}"),
            StyleDefaultsFileError::Yaml { path, error } => write!(f, "{path}: {error}"),
            StyleDefaultsFileError::Format { path, message, .. } => write!(f, "{path}: {message}"),
            StyleDefaultsFileError::Manifest(e) => write!(f, "{MANIFEST_FILE}: {e}"),
            StyleDefaultsFileError::Set(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for StyleDefaultsFileError {}

impl StyleDefaultsFileError {
    /// 안정 진단 code.
    pub fn code(&self) -> &'static str {
        match self {
            StyleDefaultsFileError::Io { .. } => "style-defaults-io",
            StyleDefaultsFileError::Yaml { .. } => "style-defaults-yaml",
            StyleDefaultsFileError::Format { code, .. } => code,
            StyleDefaultsFileError::Manifest(_) => "manifest-invalid",
            StyleDefaultsFileError::Set(e) => e.code(),
        }
    }
}

fn format_error(
    path: &str,
    code: &'static str,
    message: impl Into<String>,
) -> StyleDefaultsFileError {
    StyleDefaultsFileError::Format {
        path: path.to_string(),
        code,
        message: message.into(),
    }
}

/// 세트 이름의 파일 경로(프로젝트 root 기준).
pub fn set_file_path(name: &str) -> String {
    format!("{STYLE_DEFAULTS_DIR}/{name}{SET_FILE_SUFFIX}")
}

fn parse_entries(
    path: &str,
    pairs: &[(YamlValue, YamlValue)],
) -> Result<Vec<(LayoutLonghand, LayoutValue)>, StyleDefaultsFileError> {
    let mut out = Vec::new();
    for (key, value) in pairs {
        let name = key
            .as_scalar()
            .ok_or_else(|| format_error(path, "style-defaults-format", "키는 scalar여야 합니다"))?;
        let longhand = LayoutLonghand::from_name(name).ok_or_else(|| {
            format_error(
                path,
                "style-defaults-unknown-longhand",
                format!("{name}은 기본값을 둘 수 있는 longhand가 아닙니다"),
            )
        })?;
        let text = value.as_scalar().ok_or_else(|| {
            format_error(
                path,
                "style-defaults-format",
                format!("{name} 값은 한 줄 scalar여야 합니다"),
            )
        })?;
        let parsed = longhand.parse(text).map_err(|e| {
            format_error(path, "style-defaults-invalid-value", format!("{name}: {e}"))
        })?;
        out.push((longhand, parsed));
    }
    Ok(out)
}

/// 사용자 세트 파일 텍스트를 읽습니다. `name`은 파일 이름 stem입니다.
pub fn parse_set_file(name: &str, text: &str) -> Result<UserSetSource, StyleDefaultsFileError> {
    let path = set_file_path(name);
    let value = load_restricted(text).map_err(|error| StyleDefaultsFileError::Yaml {
        path: path.clone(),
        error,
    })?;
    let pairs = value.as_mapping().ok_or_else(|| {
        format_error(
            &path,
            "style-defaults-format",
            "최상위는 mapping이어야 합니다",
        )
    })?;
    let mut extends = None;
    let mut common = Vec::new();
    let mut sections: Vec<(NodeClass, Vec<(LayoutLonghand, LayoutValue)>)> = Vec::new();
    for (key, item) in pairs {
        let key_name = key.as_scalar().ok_or_else(|| {
            format_error(&path, "style-defaults-format", "키는 scalar여야 합니다")
        })?;
        if key_name == "extends" {
            let parent = item.as_scalar().ok_or_else(|| {
                format_error(
                    &path,
                    "style-defaults-format",
                    "extends는 세트 이름 scalar여야 합니다",
                )
            })?;
            extends = Some(parent.to_string());
        } else if let Some(class) = NodeClass::from_name(key_name) {
            let entries = item.as_mapping().ok_or_else(|| {
                format_error(
                    &path,
                    "style-defaults-format",
                    format!("{key_name} 구획은 mapping이어야 합니다"),
                )
            })?;
            sections.push((class, parse_entries(&path, entries)?));
        } else if item.as_mapping().is_some() {
            return Err(format_error(
                &path,
                "style-defaults-unknown-section",
                format!("{key_name}은 구획 이름(frame·text·image·shape)이 아닙니다"),
            ));
        } else {
            common.extend(parse_entries(
                &path,
                std::slice::from_ref(&(key.clone(), item.clone())),
            )?);
        }
    }
    let extends = extends
        .ok_or_else(|| format_error(&path, "style-defaults-format", "extends가 없습니다"))?;
    let source = UserSetSource {
        name: name.to_string(),
        extends,
        common,
        sections,
    };
    style_defaults::check_source(&source).map_err(StyleDefaultsFileError::Set)?;
    Ok(source)
}

/// 사용자 세트를 정규 텍스트로 씁니다: `extends`, 공통 값(longhand 번호 순), 구획(구획 순, 안은 longhand 순).
pub fn write_set_file(source: &UserSetSource) -> Result<String, StyleDefaultsFileError> {
    style_defaults::check_source(source).map_err(StyleDefaultsFileError::Set)?;
    let entries = |values: &[(LayoutLonghand, LayoutValue)]| {
        let mut sorted: Vec<_> = values.iter().collect();
        sorted.sort_by_key(|(l, _)| *l);
        sorted
            .into_iter()
            .map(|(l, v)| {
                (
                    YamlValue::scalar(l.name()),
                    YamlValue::scalar(format_value(v)),
                )
            })
            .collect::<Vec<_>>()
    };
    let mut top = vec![(
        YamlValue::scalar("extends"),
        YamlValue::scalar(source.extends.clone()),
    )];
    top.extend(entries(&source.common));
    let mut sections: Vec<_> = source.sections.iter().collect();
    sections.sort_by_key(|(c, _)| *c);
    for (class, values) in sections {
        if values.is_empty() {
            continue;
        }
        top.push((
            YamlValue::scalar(class.name()),
            YamlValue::Mapping(entries(values)),
        ));
    }
    write_canonical_checked(&YamlValue::Mapping(top)).map_err(|e| {
        format_error(
            &set_file_path(&source.name),
            "style-defaults-format",
            e.to_string(),
        )
    })
}

fn io_error(path: &str) -> impl Fn(io::Error) -> StyleDefaultsFileError + '_ {
    move |error| StyleDefaultsFileError::Io {
        path: path.to_string(),
        error,
    }
}

/// 프로젝트의 사용자 세트 파일을 모두 읽습니다. 디렉터리가 없으면 빈 표입니다. symlink 디렉터리·파일, 이름 형식이
/// 아닌 파일, 세트 파일이 아닌 항목은 거부합니다.
pub fn load_user_sets(root: &Path) -> Result<UserSets, StyleDefaultsFileError> {
    let dir = root.join(STYLE_DEFAULTS_DIR);
    let meta = match std::fs::symlink_metadata(&dir) {
        Ok(meta) => meta,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(e) => return Err(io_error(STYLE_DEFAULTS_DIR)(e)),
    };
    if !meta.is_dir() {
        return Err(format_error(
            STYLE_DEFAULTS_DIR,
            "style-defaults-format",
            "symlink가 아닌 디렉터리여야 합니다",
        ));
    }
    let mut names: Vec<(String, PathBuf)> = Vec::new();
    for entry in std::fs::read_dir(&dir).map_err(io_error(STYLE_DEFAULTS_DIR))? {
        let entry = entry.map_err(io_error(STYLE_DEFAULTS_DIR))?;
        let file_name = entry.file_name();
        let file_name = file_name.to_str().ok_or_else(|| {
            format_error(
                STYLE_DEFAULTS_DIR,
                "style-defaults-format",
                "파일 이름이 UTF-8이 아닙니다",
            )
        })?;
        if file_name.starts_with('.') {
            continue;
        }
        let rel = format!("{STYLE_DEFAULTS_DIR}/{file_name}");
        let name = file_name.strip_suffix(SET_FILE_SUFFIX).ok_or_else(|| {
            format_error(
                &rel,
                "style-defaults-format",
                format!("세트 파일은 <이름>{SET_FILE_SUFFIX}입니다"),
            )
        })?;
        if !style_defaults::is_valid_set_name(name) {
            return Err(StyleDefaultsFileError::Set(
                StyleDefaultsError::InvalidName {
                    name: name.to_string(),
                },
            ));
        }
        let kind = entry.file_type().map_err(io_error(&rel))?;
        if !kind.is_file() {
            return Err(format_error(
                &rel,
                "style-defaults-format",
                "정규 파일이어야 합니다(symlink 거부)",
            ));
        }
        names.push((name.to_string(), entry.path()));
        if names.len() > MAX_SET_FILES {
            return Err(format_error(
                STYLE_DEFAULTS_DIR,
                "style-defaults-format",
                format!("세트 파일은 {MAX_SET_FILES}개까지입니다"),
            ));
        }
    }
    names.sort();
    let mut sets = BTreeMap::new();
    for (name, path) in names {
        let rel = set_file_path(&name);
        let bytes = source_file::read(&path, MAX_INPUT_BYTES as u64).map_err(io_error(&rel))?;
        let text = String::from_utf8(bytes)
            .map_err(|_| format_error(&rel, "style-defaults-format", "UTF-8이 아닙니다"))?;
        sets.insert(name.clone(), parse_set_file(&name, &text)?);
    }
    Ok(sets)
}

/// 매니페스트가 고른 세트 이름(키가 없으면 `css`).
pub fn active_name(manifest: &Manifest) -> &str {
    manifest
        .style_defaults
        .as_deref()
        .unwrap_or(style_defaults::DEFAULT_SET)
}

/// `name` 세트를 평탄화합니다(내장 세트 또는 `sets`의 사용자 세트).
pub fn flatten_named(name: &str, sets: &UserSets) -> Result<FlatSet, StyleDefaultsFileError> {
    style_defaults::flatten(name, |n| sets.get(n)).map_err(StyleDefaultsFileError::Set)
}

/// 프로젝트의 활성 세트를 평탄화합니다.
pub fn active_set(root: &Path, manifest: &Manifest) -> Result<FlatSet, StyleDefaultsFileError> {
    let sets = load_user_sets(root)?;
    flatten_named(active_name(manifest), &sets)
}

/// `stellar.yaml`의 `style_defaults`를 `name`으로 바꿉니다. 세트가 평탄화되지 않으면(없는 세트, 순환) 바꾸지
/// 않습니다. 읽은 뒤 파일이 바뀌었으면 쓰지 않고 오류입니다. 이전 선택 이름을 돌려줍니다.
pub fn set_active(root: &Path, name: &str) -> Result<String, StyleDefaultsFileError> {
    let path = root.join(MANIFEST_FILE);
    let meta = std::fs::symlink_metadata(&path).map_err(io_error(MANIFEST_FILE))?;
    if meta.file_type().is_symlink() || !meta.is_file() {
        return Err(format_error(
            MANIFEST_FILE,
            "manifest-invalid",
            "정규 파일이어야 합니다",
        ));
    }
    let bytes =
        source_file::read(&path, MAX_INPUT_BYTES as u64).map_err(io_error(MANIFEST_FILE))?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| format_error(MANIFEST_FILE, "manifest-invalid", "UTF-8이 아닙니다"))?;
    let value = load_restricted(text).map_err(|error| StyleDefaultsFileError::Yaml {
        path: MANIFEST_FILE.to_string(),
        error,
    })?;
    let mut manifest = Manifest::from_yaml(&value).map_err(StyleDefaultsFileError::Manifest)?;
    let sets = load_user_sets(root)?;
    flatten_named(name, &sets)?;
    let previous = active_name(&manifest).to_string();
    manifest.style_defaults = (name != style_defaults::DEFAULT_SET).then(|| name.to_string());
    let written = manifest
        .write()
        .map_err(|e| format_error(MANIFEST_FILE, "manifest-invalid", e.to_string()))?;
    if written.as_bytes() != bytes.as_slice() {
        source_file::write_checked(
            &path,
            written.as_bytes(),
            Some(ContentHash::compute(&bytes)),
            MAX_INPUT_BYTES as u64,
        )
        .map_err(io_error(MANIFEST_FILE))?;
    }
    Ok(previous)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::ProjectMeta;

    struct TestDir(PathBuf);

    impl TestDir {
        fn new(tag: &str) -> TestDir {
            let path = std::env::temp_dir().join(format!("stellar-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            TestDir(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn manifest(style: Option<&str>) -> Manifest {
        Manifest {
            schema_version: crate::manifest::CURRENT_SCHEMA_VERSION,
            project: ProjectMeta {
                id: "0195c530-3acd-7bc1-8414-c213f61cb960".into(),
                name: "Sample".into(),
            },
            target_profiles: Vec::new(),
            dependencies: Vec::new(),
            style_defaults: style.map(str::to_string),
        }
    }

    #[test]
    fn manifest_key_round_trips_and_is_optional() {
        let plain = manifest(None);
        let text = plain.write().unwrap();
        assert!(!text.contains("style_defaults"));
        let chosen = manifest(Some("react-native"));
        let text = chosen.write().unwrap();
        assert!(text.ends_with("style_defaults: react-native\n"), "{text}");
        let parsed = Manifest::from_yaml(&load_restricted(&text).unwrap()).unwrap();
        assert_eq!(parsed, chosen);
        assert_eq!(active_name(&plain), "css");
        assert_eq!(active_name(&parsed), "react-native");
        let bad = text.replace("react-native", "Bad Name");
        assert!(matches!(
            Manifest::from_yaml(&load_restricted(&bad).unwrap()),
            Err(ManifestError::InvalidStyleDefaults { .. })
        ));
    }

    const COMPACT: &str = "extends: react-native\nflex-shrink: \"1\"\nframe:\n  row-gap: 8px\n  column-gap: 8px\ntext:\n  min-width: auto\n";

    #[test]
    fn set_file_parses_and_writes_canonically() {
        let source = parse_set_file("compact", COMPACT).unwrap();
        assert_eq!(source.extends, "react-native");
        assert_eq!(source.common.len(), 1);
        assert_eq!(source.sections.len(), 2);
        let written = write_set_file(&source).unwrap();
        // longhand 번호 순(row-gap 0x011b, column-gap 0x011c)과 구획 순.
        assert_eq!(
            written,
            "extends: react-native\nflex-shrink: \"1\"\nframe:\n  row-gap: \"8px\"\n  column-gap: \"8px\"\ntext:\n  min-width: auto\n"
        );
        assert_eq!(parse_set_file("compact", &written).unwrap(), source);
    }

    #[test]
    fn set_files_reject_unknown_keys_values_and_tokens() {
        let cases = [
            (
                "extends: css\nfloat: left\n",
                "style-defaults-unknown-longhand",
            ),
            (
                "extends: css\nheader:\n  display: flex\n",
                "style-defaults-unknown-section",
            ),
            (
                "extends: css\ndisplay: inline\n",
                "style-defaults-invalid-value",
            ),
            (
                "extends: css\nrow-gap: var(--gap)\n",
                "style-defaults-invalid-value",
            ),
            (
                "extends: css\nrow-gap: calc(1px + 2px)\n",
                "style-defaults-invalid-value",
            ),
            ("display: flex\n", "style-defaults-format"),
            (
                "extends: css\nframe:\n  display: flex\n  display: grid\n",
                "style-defaults-yaml",
            ),
        ];
        for (text, code) in cases {
            let error = parse_set_file("x", text).unwrap_err();
            assert_eq!(error.code(), code, "{text}: {error}");
        }
        assert_eq!(
            parse_set_file("css", "extends: react-native\n")
                .unwrap_err()
                .code(),
            "style-defaults-reserved-name"
        );
    }

    fn project(dir: &Path, style: Option<&str>) {
        std::fs::write(dir.join(MANIFEST_FILE), manifest(style).write().unwrap()).unwrap();
    }

    #[test]
    fn project_sets_load_flatten_and_reject_cycles() {
        let dir = TestDir::new("style-defaults-load");
        project(dir.path(), Some("dense"));
        let sets_dir = dir.path().join(STYLE_DEFAULTS_DIR);
        std::fs::create_dir_all(&sets_dir).unwrap();
        std::fs::write(sets_dir.join("compact.stellar.yaml"), COMPACT).unwrap();
        std::fs::write(
            sets_dir.join("dense.stellar.yaml"),
            "extends: compact\nrow-gap: 4px\n",
        )
        .unwrap();
        let manifest = manifest(Some("dense"));
        let flat = active_set(dir.path(), &manifest).unwrap();
        assert_eq!(flat.chain(), ["dense", "compact", "react-native", "css"]);
        assert_eq!(
            flat.value(NodeClass::Frame, LayoutLonghand::RowGap),
            &LayoutValue::Length(4.0)
        );
        // 순환.
        std::fs::write(sets_dir.join("compact.stellar.yaml"), "extends: dense\n").unwrap();
        assert_eq!(
            active_set(dir.path(), &manifest).unwrap_err().code(),
            "style-defaults-cycle"
        );
        // 이름 형식이 아닌 파일.
        std::fs::write(sets_dir.join("compact.stellar.yaml"), COMPACT).unwrap();
        std::fs::write(sets_dir.join("Bad.stellar.yaml"), "extends: css\n").unwrap();
        assert_eq!(
            load_user_sets(dir.path()).unwrap_err().code(),
            "style-defaults-invalid-name"
        );
    }

    #[test]
    fn set_active_rewrites_one_line_and_rejects_unknown_sets() {
        let dir = TestDir::new("style-defaults-set");
        project(dir.path(), None);
        let before = std::fs::read_to_string(dir.path().join(MANIFEST_FILE)).unwrap();
        assert_eq!(set_active(dir.path(), "react-native").unwrap(), "css");
        let after = std::fs::read_to_string(dir.path().join(MANIFEST_FILE)).unwrap();
        assert_eq!(
            after,
            format!("{before}style_defaults: react-native\n"),
            "한 줄 diff"
        );
        assert!(set_active(dir.path(), "missing").is_err());
        assert_eq!(
            std::fs::read_to_string(dir.path().join(MANIFEST_FILE)).unwrap(),
            after,
            "거부하면 그대로"
        );
        assert_eq!(set_active(dir.path(), "css").unwrap(), "react-native");
        assert_eq!(
            std::fs::read_to_string(dir.path().join(MANIFEST_FILE)).unwrap(),
            before,
            "css는 키를 지움"
        );
    }
}
