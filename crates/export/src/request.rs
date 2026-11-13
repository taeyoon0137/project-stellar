//! 내보내기 요청과 field schema (XX.1.1).
//!
//! [`ExportRequest`]는 내보내기 한 건의 입력입니다. 모든 구성 값은 **생성 시 검증된 타입**이라,
//! 만들어진 요청은 항상 schema를 만족합니다.
//!
//! | field | 타입 | 규칙 |
//! |---|---|---|
//! | source | [`ExportSource`] | View 하나 또는 그 View 안의 selection(1..=[`MAX_SELECTION`]개, 중복·nil 불가) |
//! | target profile | [`TargetProfileKey`] | `stellar.yaml` `target_profiles[].id`와 같은 문자열 key |
//! | scale | [`ExportScale`] | finite, [`MIN_SCALE`]..=[`MAX_SCALE`] |
//! | background | [`ExportBackground`] | 투명 또는 단색([`Color`], 색공간 포함) |
//! | output path | [`OutputPath`] | 절대 경로, `..` 없음, NUL 없음, 파일 이름 있음, [`MAX_OUTPUT_PATH_BYTES`] 이하 |
//! | format | [`ExportFormat`] | PNG 또는 SVG. output path 확장자와 일치(대소문자 무시) |
//!
//! ## field 표현 (schema)
//!
//! CLI·agent tool처럼 문자열 key/value로 요청을 받는 경계를 위해 [`ExportRequest::from_fields`]와
//! [`ExportRequest::to_fields`]를 제공합니다. field 목록과 필수 여부는 [`REQUEST_SCHEMA`]가
//! 정의하며, 입력은 신뢰하지 않는다고 가정합니다.
//!
//! - 미지 field, 중복 field, 필수 field 누락, `source = view`인데 `selection`이 있는 경우를
//!   거부합니다.
//! - field 값 길이는 field별 상한([`FieldSchema::max_len`])을 넘으면 parse 전에 거부합니다.
//! - 숫자는 `-?digits(.digits)?` 십진 문법만 받습니다(지수·`inf`·`NaN`·`+` 불가).
//! - [`ExportRequest::to_fields`] 출력은 canonical(schema 순서, 기본값도 명시)이며
//!   [`ExportRequest::from_fields`]로 같은 요청이 복원됩니다.
//!
//! TargetProfile의 정식 식별자 `TargetProfileId`(IX.10.1)가 아직 없으므로, 이 crate는 매니페스트
//! profile id 문자열을 검증된 key로 받습니다. crate 의존 정책(II.2.8, ADR-0019)상 export는
//! target-profile crate에 의존하지 않으므로, 정식 타입이 생기면 host가 key로 변환해 넘깁니다.

use std::collections::BTreeSet;
use std::fmt;
use std::path::{Component, Path, PathBuf};

use stellar_core_types::color::{Color, ColorError, ColorSpace};
use stellar_core_types::ids::StableId;
use stellar_core_types::value::FiniteF32;

/// field schema 버전. [`ExportRequest::from_fields`]는 이 값만 받습니다.
pub const EXPORT_REQUEST_SCHEMA_VERSION: u32 = 1;

/// selection 최대 node 수.
pub const MAX_SELECTION: usize = 100_000;

/// 최소 scale.
pub const MIN_SCALE: f32 = 0.01;

/// 최대 scale.
pub const MAX_SCALE: f32 = 16.0;

/// output path 최대 길이(byte).
pub const MAX_OUTPUT_PATH_BYTES: usize = 4096;

/// target profile key 최대 길이(byte).
pub const MAX_PROFILE_KEY_BYTES: usize = 64;

/// 오류 메시지에 담는 미지 field 이름의 최대 길이(byte). 신뢰하지 않는 입력을 그대로 복사하지 않습니다.
const MAX_REPORTED_NAME_BYTES: usize = 64;

/// 출력 형식.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ExportFormat {
    /// PNG raster.
    Png,
    /// SVG vector(텍스트 outline·부분 래스터화 포함).
    Svg,
}

impl ExportFormat {
    /// 정의된 모든 형식.
    pub const ALL: [ExportFormat; 2] = [ExportFormat::Png, ExportFormat::Svg];

    /// 안정 numeric tag(Png=1, Svg=2).
    pub const fn tag(self) -> u8 {
        match self {
            ExportFormat::Png => 1,
            ExportFormat::Svg => 2,
        }
    }

    /// tag에서 복원합니다. 미지 tag는 `None`.
    pub fn from_tag(tag: u8) -> Option<ExportFormat> {
        ExportFormat::ALL.into_iter().find(|f| f.tag() == tag)
    }

    /// field 표현과 파일 확장자(소문자, 점 제외).
    pub const fn name(self) -> &'static str {
        match self {
            ExportFormat::Png => "png",
            ExportFormat::Svg => "svg",
        }
    }

    /// field 표현에서 복원합니다(정확히 소문자 이름).
    pub fn from_name(name: &str) -> Option<ExportFormat> {
        ExportFormat::ALL.into_iter().find(|f| f.name() == name)
    }
}

impl fmt::Display for ExportFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// 내보낼 범위. View 전체 또는 그 View 안의 selection입니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportSource {
    /// View 전체.
    View {
        /// ViewDefinition의 StableId.
        view: StableId,
    },
    /// View 안에서 선택한 node들. 정렬된 집합이라 순서가 결과에 영향을 주지 않습니다.
    Selection {
        /// ViewDefinition의 StableId.
        view: StableId,
        /// 선택한 document node의 StableId(1개 이상).
        nodes: BTreeSet<StableId>,
    },
}

impl ExportSource {
    /// View 전체를 내보냅니다. nil id는 거부합니다.
    pub fn view(view: StableId) -> Result<ExportSource, ExportRequestError> {
        if view == StableId::NIL {
            return Err(ExportRequestError::NilView);
        }
        Ok(ExportSource::View { view })
    }

    /// View 안의 selection을 내보냅니다.
    ///
    /// 비어 있거나, [`MAX_SELECTION`]개를 넘거나, nil·중복 id가 있으면 거부합니다. 상한을 넘는
    /// 순간 순회를 멈춥니다.
    pub fn selection(
        view: StableId,
        nodes: impl IntoIterator<Item = StableId>,
    ) -> Result<ExportSource, ExportRequestError> {
        if view == StableId::NIL {
            return Err(ExportRequestError::NilView);
        }
        let mut set = BTreeSet::new();
        for node in nodes {
            if node == StableId::NIL {
                return Err(ExportRequestError::NilSelectionNode);
            }
            if set.len() == MAX_SELECTION {
                return Err(ExportRequestError::SelectionTooLarge {
                    limit: MAX_SELECTION,
                });
            }
            if !set.insert(node) {
                return Err(ExportRequestError::DuplicateSelection { id: node });
            }
        }
        if set.is_empty() {
            return Err(ExportRequestError::EmptySelection);
        }
        Ok(ExportSource::Selection { view, nodes: set })
    }

    /// 대상 View.
    pub const fn view_id(&self) -> StableId {
        match self {
            ExportSource::View { view } | ExportSource::Selection { view, .. } => *view,
        }
    }

    /// selection이면 선택 node 집합.
    pub const fn selection_nodes(&self) -> Option<&BTreeSet<StableId>> {
        match self {
            ExportSource::View { .. } => None,
            ExportSource::Selection { nodes, .. } => Some(nodes),
        }
    }
}

/// target profile key 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileKeyError {
    /// 비어 있습니다.
    Empty,
    /// [`MAX_PROFILE_KEY_BYTES`]를 넘습니다.
    TooLong,
    /// 첫 문자가 ASCII 영숫자가 아닙니다.
    InvalidStart,
    /// 허용하지 않는 문자(ASCII 영숫자·`-`·`_`·`.` 외)가 `index` byte에 있습니다.
    InvalidChar {
        /// byte 위치.
        index: usize,
    },
}

impl fmt::Display for ProfileKeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProfileKeyError::Empty => f.write_str("target profile key가 비어 있습니다"),
            ProfileKeyError::TooLong => write!(
                f,
                "target profile key는 {MAX_PROFILE_KEY_BYTES} byte 이하여야 합니다"
            ),
            ProfileKeyError::InvalidStart => {
                f.write_str("target profile key는 ASCII 영숫자로 시작해야 합니다")
            }
            ProfileKeyError::InvalidChar { index } => write!(
                f,
                "target profile key {index} byte에 허용하지 않는 문자가 있습니다"
            ),
        }
    }
}

impl std::error::Error for ProfileKeyError {}

/// 검증된 target profile key(`stellar.yaml` `target_profiles[].id`).
///
/// 1..=[`MAX_PROFILE_KEY_BYTES`] byte, ASCII 영숫자로 시작하고 ASCII 영숫자·`-`·`_`·`.`만
/// 포함합니다. 대소문자를 구분합니다.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TargetProfileKey(String);

impl TargetProfileKey {
    /// key를 검증해 만듭니다.
    pub fn new(key: &str) -> Result<TargetProfileKey, ProfileKeyError> {
        let bytes = key.as_bytes();
        let Some(&first) = bytes.first() else {
            return Err(ProfileKeyError::Empty);
        };
        if bytes.len() > MAX_PROFILE_KEY_BYTES {
            return Err(ProfileKeyError::TooLong);
        }
        if !first.is_ascii_alphanumeric() {
            return Err(ProfileKeyError::InvalidStart);
        }
        if let Some(index) = bytes
            .iter()
            .position(|b| !(b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.')))
        {
            return Err(ProfileKeyError::InvalidChar { index });
        }
        Ok(TargetProfileKey(key.to_string()))
    }

    /// key 문자열.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TargetProfileKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// 출력 배율(scene 단위 1 → 출력 단위 `scale`).
///
/// PNG는 pixel 크기, SVG는 `width`·`height` 속성에 적용합니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExportScale(FiniteF32);

impl ExportScale {
    /// 1배.
    pub const ONE: ExportScale = ExportScale(FiniteF32::ONE);

    /// finite·[`MIN_SCALE`]..=[`MAX_SCALE`] 배율을 만듭니다.
    pub fn new(value: f32) -> Result<ExportScale, ExportRequestError> {
        let finite = FiniteF32::new(value).map_err(|_| ExportRequestError::ScaleNonFinite)?;
        if !(MIN_SCALE..=MAX_SCALE).contains(&finite.get()) {
            return Err(ExportRequestError::ScaleOutOfRange { value: finite });
        }
        Ok(ExportScale(finite))
    }

    /// 값.
    pub const fn get(&self) -> f32 {
        self.0.get()
    }
}

impl Default for ExportScale {
    fn default() -> ExportScale {
        ExportScale::ONE
    }
}

/// 배경.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ExportBackground {
    /// 투명(기본).
    #[default]
    Transparent,
    /// 단색(색공간 포함, 비연관 알파).
    Solid(Color),
}

/// output path 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputPathError {
    /// 비어 있습니다.
    Empty,
    /// [`MAX_OUTPUT_PATH_BYTES`]를 넘습니다.
    TooLong,
    /// NUL byte가 있습니다.
    ContainsNul,
    /// 절대 경로가 아닙니다.
    NotAbsolute,
    /// `..` 구성 요소가 있습니다.
    ParentComponent,
    /// 파일 이름이 없습니다(디렉터리 경로).
    MissingFileName,
}

impl fmt::Display for OutputPathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            OutputPathError::Empty => "출력 경로가 비어 있습니다",
            OutputPathError::TooLong => "출력 경로가 너무 깁니다",
            OutputPathError::ContainsNul => "출력 경로에 NUL 문자가 있습니다",
            OutputPathError::NotAbsolute => "출력 경로는 절대 경로여야 합니다",
            OutputPathError::ParentComponent => "출력 경로에 `..`를 쓸 수 없습니다",
            OutputPathError::MissingFileName => "출력 경로에 파일 이름이 없습니다",
        })
    }
}

impl std::error::Error for OutputPathError {}

/// 검증된 출력 파일 경로.
///
/// 작업 디렉터리에 따라 의미가 바뀌지 않도록 절대 경로만 받고, 정규화 모호성을 피하려고 `..`를
/// 거부합니다. 파일 존재 여부·쓰기 권한은 저장 단계가 확인합니다.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OutputPath(PathBuf);

impl OutputPath {
    /// 경로를 검증해 만듭니다.
    pub fn new(path: impl Into<PathBuf>) -> Result<OutputPath, OutputPathError> {
        let path = path.into();
        let bytes = path.as_os_str().as_encoded_bytes();
        let Some(&last) = bytes.last() else {
            return Err(OutputPathError::Empty);
        };
        if bytes.len() > MAX_OUTPUT_PATH_BYTES {
            return Err(OutputPathError::TooLong);
        }
        if bytes.contains(&0) {
            return Err(OutputPathError::ContainsNul);
        }
        if !path.is_absolute() {
            return Err(OutputPathError::NotAbsolute);
        }
        if path.components().any(|c| matches!(c, Component::ParentDir)) {
            return Err(OutputPathError::ParentComponent);
        }
        if std::path::is_separator(char::from(last)) || path.file_name().is_none() {
            return Err(OutputPathError::MissingFileName);
        }
        Ok(OutputPath(path))
    }

    /// 경로.
    pub fn as_path(&self) -> &Path {
        &self.0
    }

    /// 확장자가 `format`과 같으면 참입니다(ASCII 대소문자 무시).
    pub fn has_extension_of(&self, format: ExportFormat) -> bool {
        self.0
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case(format.name()))
    }
}

/// 내보내기 요청 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportRequestError {
    /// View id가 nil입니다.
    NilView,
    /// selection에 nil id가 있습니다.
    NilSelectionNode,
    /// selection이 비어 있습니다.
    EmptySelection,
    /// selection이 상한을 넘습니다.
    SelectionTooLarge {
        /// 상한.
        limit: usize,
    },
    /// selection에 같은 id가 두 번 있습니다.
    DuplicateSelection {
        /// 중복 id.
        id: StableId,
    },
    /// target profile key가 잘못되었습니다.
    ProfileKey(ProfileKeyError),
    /// scale이 유한하지 않습니다.
    ScaleNonFinite,
    /// scale이 허용 범위 밖입니다.
    ScaleOutOfRange {
        /// 입력 값.
        value: FiniteF32,
    },
    /// 배경색이 잘못되었습니다.
    Background(ColorError),
    /// output path가 잘못되었습니다.
    OutputPath(OutputPathError),
    /// output path 확장자가 format과 다릅니다.
    ExtensionMismatch {
        /// 요청 format.
        format: ExportFormat,
    },
    /// schema에 없는 field입니다.
    UnknownField {
        /// field 이름(최대 64 byte로 자름).
        name: String,
    },
    /// 같은 field가 두 번 있습니다.
    DuplicateField {
        /// field.
        field: &'static str,
    },
    /// 필수 field가 없습니다.
    MissingField {
        /// field.
        field: &'static str,
    },
    /// 이 요청에서 허용하지 않는 field입니다(`source = view`의 `selection`).
    FieldNotAllowed {
        /// field.
        field: &'static str,
    },
    /// field 값이 상한보다 깁니다.
    FieldTooLong {
        /// field.
        field: &'static str,
        /// 상한(byte).
        limit: usize,
    },
    /// field 값 문법이 잘못되었습니다.
    Malformed {
        /// field.
        field: &'static str,
    },
    /// 지원하지 않는 schema 버전입니다.
    UnsupportedSchemaVersion,
}

impl fmt::Display for ExportRequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExportRequestError::NilView => f.write_str("View id가 nil입니다"),
            ExportRequestError::NilSelectionNode => f.write_str("selection에 nil id가 있습니다"),
            ExportRequestError::EmptySelection => f.write_str("selection이 비어 있습니다"),
            ExportRequestError::SelectionTooLarge { limit } => {
                write!(f, "selection은 {limit}개 이하여야 합니다")
            }
            ExportRequestError::DuplicateSelection { id } => {
                write!(f, "selection에 {id}가 중복되었습니다")
            }
            ExportRequestError::ProfileKey(error) => write!(f, "{error}"),
            ExportRequestError::ScaleNonFinite => f.write_str("scale이 유한하지 않습니다"),
            ExportRequestError::ScaleOutOfRange { value } => write!(
                f,
                "scale {}은 {MIN_SCALE}..={MAX_SCALE} 범위여야 합니다",
                value.get()
            ),
            ExportRequestError::Background(error) => write!(f, "배경색 오류: {error}"),
            ExportRequestError::OutputPath(error) => write!(f, "{error}"),
            ExportRequestError::ExtensionMismatch { format } => {
                write!(f, "출력 경로 확장자가 .{format}이 아닙니다")
            }
            ExportRequestError::UnknownField { name } => write!(f, "알 수 없는 field: {name}"),
            ExportRequestError::DuplicateField { field } => {
                write!(f, "field {field}가 중복되었습니다")
            }
            ExportRequestError::MissingField { field } => {
                write!(f, "필수 field {field}가 없습니다")
            }
            ExportRequestError::FieldNotAllowed { field } => {
                write!(f, "이 요청에서는 field {field}를 쓸 수 없습니다")
            }
            ExportRequestError::FieldTooLong { field, limit } => {
                write!(f, "field {field} 값은 {limit} byte 이하여야 합니다")
            }
            ExportRequestError::Malformed { field } => {
                write!(f, "field {field} 값의 형식이 잘못되었습니다")
            }
            ExportRequestError::UnsupportedSchemaVersion => write!(
                f,
                "지원하지 않는 schema 버전입니다(지원: {EXPORT_REQUEST_SCHEMA_VERSION})"
            ),
        }
    }
}

impl std::error::Error for ExportRequestError {}

impl From<ProfileKeyError> for ExportRequestError {
    fn from(error: ProfileKeyError) -> ExportRequestError {
        ExportRequestError::ProfileKey(error)
    }
}

impl From<OutputPathError> for ExportRequestError {
    fn from(error: OutputPathError) -> ExportRequestError {
        ExportRequestError::OutputPath(error)
    }
}

/// field 필수 여부.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldRequirement {
    /// 항상 필수.
    Required,
    /// `source = selection`일 때만 필수이고 그 외에는 허용하지 않습니다.
    SelectionOnly,
    /// 선택(없으면 기본값).
    Optional,
}

/// field 하나의 schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldSchema {
    /// field 이름.
    pub name: &'static str,
    /// 필수 여부.
    pub requirement: FieldRequirement,
    /// 값 최대 길이(byte).
    pub max_len: usize,
    /// 값 문법 설명.
    pub syntax: &'static str,
}

/// field 이름 상수.
pub mod field {
    /// schema 버전.
    pub const SCHEMA_VERSION: &str = "schema_version";
    /// 출력 형식.
    pub const FORMAT: &str = "format";
    /// 범위 종류.
    pub const SOURCE: &str = "source";
    /// View id.
    pub const VIEW: &str = "view";
    /// selection id 목록.
    pub const SELECTION: &str = "selection";
    /// target profile key.
    pub const TARGET_PROFILE: &str = "target_profile";
    /// 배율.
    pub const SCALE: &str = "scale";
    /// 배경.
    pub const BACKGROUND: &str = "background";
    /// 출력 경로.
    pub const OUTPUT_PATH: &str = "output_path";
}

/// StableId 문자열 길이.
const STABLE_ID_LEN: usize = 36;

/// 십진수 하나의 최대 길이(f32 `Display`는 subnormal에서도 이 안에 들어옵니다).
const MAX_DECIMAL_LEN: usize = 128;

/// 요청 field schema(canonical 순서).
pub const REQUEST_SCHEMA: [FieldSchema; 9] = [
    FieldSchema {
        name: field::SCHEMA_VERSION,
        requirement: FieldRequirement::Required,
        max_len: 8,
        syntax: "1",
    },
    FieldSchema {
        name: field::FORMAT,
        requirement: FieldRequirement::Required,
        max_len: 8,
        syntax: "png | svg",
    },
    FieldSchema {
        name: field::SOURCE,
        requirement: FieldRequirement::Required,
        max_len: 16,
        syntax: "view | selection",
    },
    FieldSchema {
        name: field::VIEW,
        requirement: FieldRequirement::Required,
        max_len: STABLE_ID_LEN,
        syntax: "StableId(소문자 hyphen UUID, 대소문자 무관 입력)",
    },
    FieldSchema {
        name: field::SELECTION,
        requirement: FieldRequirement::SelectionOnly,
        max_len: MAX_SELECTION * (STABLE_ID_LEN + 1),
        syntax: "쉼표로 구분한 StableId 목록(공백 없음, 1..=100000개, 중복 불가)",
    },
    FieldSchema {
        name: field::TARGET_PROFILE,
        requirement: FieldRequirement::Required,
        max_len: MAX_PROFILE_KEY_BYTES,
        syntax: "ASCII 영숫자로 시작하는 [A-Za-z0-9._-] 1..=64 byte",
    },
    FieldSchema {
        name: field::SCALE,
        requirement: FieldRequirement::Optional,
        max_len: MAX_DECIMAL_LEN,
        syntax: "십진수 0.01..=16 (기본 1)",
    },
    FieldSchema {
        name: field::BACKGROUND,
        requirement: FieldRequirement::Optional,
        max_len: 16 + 4 * MAX_DECIMAL_LEN,
        syntax: "transparent | srgb(r,g,b,a) | display-p3(r,g,b,a) (기본 transparent)",
    },
    FieldSchema {
        name: field::OUTPUT_PATH,
        requirement: FieldRequirement::Required,
        max_len: MAX_OUTPUT_PATH_BYTES,
        syntax: "절대 경로, 확장자는 format과 같음",
    },
];

/// 검증된 내보내기 요청.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportRequest {
    source: ExportSource,
    target_profile: TargetProfileKey,
    scale: ExportScale,
    background: ExportBackground,
    output_path: OutputPath,
    format: ExportFormat,
}

impl ExportRequest {
    /// 검증된 구성 값으로 요청을 만듭니다. output path 확장자가 format과 다르면 거부합니다.
    pub fn new(
        source: ExportSource,
        target_profile: TargetProfileKey,
        scale: ExportScale,
        background: ExportBackground,
        output_path: OutputPath,
        format: ExportFormat,
    ) -> Result<ExportRequest, ExportRequestError> {
        if !output_path.has_extension_of(format) {
            return Err(ExportRequestError::ExtensionMismatch { format });
        }
        Ok(ExportRequest {
            source,
            target_profile,
            scale,
            background,
            output_path,
            format,
        })
    }

    /// 내보낼 범위.
    pub const fn source(&self) -> &ExportSource {
        &self.source
    }

    /// target profile.
    pub const fn target_profile(&self) -> &TargetProfileKey {
        &self.target_profile
    }

    /// 배율.
    pub const fn scale(&self) -> ExportScale {
        self.scale
    }

    /// 배경.
    pub const fn background(&self) -> ExportBackground {
        self.background
    }

    /// 출력 경로.
    pub const fn output_path(&self) -> &OutputPath {
        &self.output_path
    }

    /// 출력 형식.
    pub const fn format(&self) -> ExportFormat {
        self.format
    }

    /// 문자열 key/value field로부터 요청을 만듭니다(모듈 문서의 schema 규칙).
    pub fn from_fields<'a, I>(fields: I) -> Result<ExportRequest, ExportRequestError>
    where
        I: IntoIterator<Item = (&'a str, &'a str)>,
    {
        let mut values: [Option<&'a str>; REQUEST_SCHEMA.len()] = [None; REQUEST_SCHEMA.len()];
        for (name, value) in fields {
            let Some(index) = REQUEST_SCHEMA.iter().position(|s| s.name == name) else {
                return Err(ExportRequestError::UnknownField {
                    name: truncate_name(name),
                });
            };
            let schema = &REQUEST_SCHEMA[index];
            if values[index].is_some() {
                return Err(ExportRequestError::DuplicateField { field: schema.name });
            }
            if value.len() > schema.max_len {
                return Err(ExportRequestError::FieldTooLong {
                    field: schema.name,
                    limit: schema.max_len,
                });
            }
            values[index] = Some(value);
        }
        let get = |name: &'static str| -> Option<&'a str> {
            REQUEST_SCHEMA
                .iter()
                .position(|s| s.name == name)
                .and_then(|index| values[index])
        };
        let require = |name: &'static str| -> Result<&'a str, ExportRequestError> {
            get(name).ok_or(ExportRequestError::MissingField { field: name })
        };

        if require(field::SCHEMA_VERSION)? != "1" {
            return Err(ExportRequestError::UnsupportedSchemaVersion);
        }
        let format = ExportFormat::from_name(require(field::FORMAT)?).ok_or(
            ExportRequestError::Malformed {
                field: field::FORMAT,
            },
        )?;
        let view = parse_stable_id(require(field::VIEW)?, field::VIEW)?;
        let source = match require(field::SOURCE)? {
            "view" => {
                if get(field::SELECTION).is_some() {
                    return Err(ExportRequestError::FieldNotAllowed {
                        field: field::SELECTION,
                    });
                }
                ExportSource::view(view)?
            }
            "selection" => {
                let list = require(field::SELECTION)?;
                ExportSource::selection(view, parse_selection(list)?)?
            }
            _ => {
                return Err(ExportRequestError::Malformed {
                    field: field::SOURCE,
                })
            }
        };
        let target_profile = TargetProfileKey::new(require(field::TARGET_PROFILE)?)?;
        let scale = match get(field::SCALE) {
            None => ExportScale::ONE,
            Some(text) => ExportScale::new(parse_decimal(text, field::SCALE)?)?,
        };
        let background = match get(field::BACKGROUND) {
            None => ExportBackground::Transparent,
            Some(text) => parse_background(text)?,
        };
        let output_path = OutputPath::new(require(field::OUTPUT_PATH)?)?;
        ExportRequest::new(
            source,
            target_profile,
            scale,
            background,
            output_path,
            format,
        )
    }

    /// canonical field 표현(schema 순서, 기본값 포함).
    ///
    /// output path가 UTF-8이 아니면 표현할 수 없어 `None`을 돌려줍니다.
    pub fn to_fields(&self) -> Option<Vec<(&'static str, String)>> {
        let mut out = vec![
            (
                field::SCHEMA_VERSION,
                EXPORT_REQUEST_SCHEMA_VERSION.to_string(),
            ),
            (field::FORMAT, self.format.name().to_string()),
        ];
        match &self.source {
            ExportSource::View { view } => {
                out.push((field::SOURCE, "view".to_string()));
                out.push((field::VIEW, view.to_string()));
            }
            ExportSource::Selection { view, nodes } => {
                out.push((field::SOURCE, "selection".to_string()));
                out.push((field::VIEW, view.to_string()));
                let list: Vec<String> = nodes.iter().map(StableId::to_string).collect();
                out.push((field::SELECTION, list.join(",")));
            }
        }
        out.push((field::TARGET_PROFILE, self.target_profile.0.clone()));
        out.push((field::SCALE, self.scale.get().to_string()));
        out.push((field::BACKGROUND, background_text(self.background)));
        out.push((
            field::OUTPUT_PATH,
            self.output_path.as_path().to_str()?.to_string(),
        ));
        Some(out)
    }
}

/// 미지 field 이름을 오류에 담을 길이로 자릅니다(UTF-8 경계 유지).
fn truncate_name(name: &str) -> String {
    let mut end = name.len().min(MAX_REPORTED_NAME_BYTES);
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    name[..end].to_string()
}

fn parse_stable_id(text: &str, field: &'static str) -> Result<StableId, ExportRequestError> {
    text.parse::<StableId>()
        .map_err(|_| ExportRequestError::Malformed { field })
}

/// 쉼표 목록을 parse합니다. 개수 상한은 길이 상한([`REQUEST_SCHEMA`])이 보장합니다.
fn parse_selection(list: &str) -> Result<Vec<StableId>, ExportRequestError> {
    list.split(',')
        .map(|item| parse_stable_id(item, field::SELECTION))
        .collect()
}

/// `-?digits(.digits)?` 십진수만 받아 f32로 parse합니다.
fn parse_decimal(text: &str, field: &'static str) -> Result<f32, ExportRequestError> {
    let malformed = ExportRequestError::Malformed { field };
    if text.len() > MAX_DECIMAL_LEN {
        return Err(malformed);
    }
    let unsigned = text.strip_prefix('-').unwrap_or(text);
    let (int, frac) = match unsigned.split_once('.') {
        Some((int, frac)) => (int, Some(frac)),
        None => (unsigned, None),
    };
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(int) || frac.is_some_and(|f| !digits(f)) {
        return Err(malformed);
    }
    let value: f32 = text.parse().map_err(|_| malformed.clone())?;
    if value.is_finite() {
        Ok(value)
    } else {
        Err(malformed)
    }
}

fn parse_background(text: &str) -> Result<ExportBackground, ExportRequestError> {
    let malformed = ExportRequestError::Malformed {
        field: field::BACKGROUND,
    };
    if text == "transparent" {
        return Ok(ExportBackground::Transparent);
    }
    let (space, body) = if let Some(body) = text.strip_prefix("srgb(") {
        (ColorSpace::Srgb, body)
    } else if let Some(body) = text.strip_prefix("display-p3(") {
        (ColorSpace::DisplayP3, body)
    } else {
        return Err(malformed);
    };
    let body = body.strip_suffix(')').ok_or(malformed.clone())?;
    let mut channels = [0.0f32; 4];
    let mut parts = body.split(',');
    for channel in &mut channels {
        let part = parts.next().ok_or(malformed.clone())?;
        *channel = parse_decimal(part, field::BACKGROUND)?;
    }
    if parts.next().is_some() {
        return Err(malformed);
    }
    let [r, g, b, a] = channels;
    Color::new(space, r, g, b, a)
        .map(ExportBackground::Solid)
        .map_err(ExportRequestError::Background)
}

fn background_text(background: ExportBackground) -> String {
    match background {
        ExportBackground::Transparent => "transparent".to_string(),
        ExportBackground::Solid(color) => {
            let space = match color.space() {
                ColorSpace::Srgb => "srgb",
                ColorSpace::DisplayP3 => "display-p3",
            };
            format!(
                "{space}({},{},{},{})",
                color.r(),
                color.g(),
                color.b(),
                color.alpha()
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIEW: &str = "0195c530-3acd-7bc1-8414-c213f61cb965";
    const NODE_A: &str = "0195c530-3acd-7bc1-8414-c213f61cb966";
    const NODE_B: &str = "0195c530-3acd-7bc1-8414-c213f61cb967";

    fn id(text: &str) -> StableId {
        text.parse().unwrap()
    }

    fn base_fields() -> Vec<(&'static str, &'static str)> {
        vec![
            ("schema_version", "1"),
            ("format", "svg"),
            ("source", "view"),
            ("view", VIEW),
            ("target_profile", "web-modern"),
            ("output_path", "/tmp/export/out.svg"),
        ]
    }

    fn with(
        mut fields: Vec<(&'static str, &'static str)>,
        name: &'static str,
        value: &'static str,
    ) -> Vec<(&'static str, &'static str)> {
        match fields.iter_mut().find(|(n, _)| *n == name) {
            Some(entry) => entry.1 = value,
            None => fields.push((name, value)),
        }
        fields
    }

    fn without(
        fields: Vec<(&'static str, &'static str)>,
        name: &str,
    ) -> Vec<(&'static str, &'static str)> {
        fields.into_iter().filter(|(n, _)| *n != name).collect()
    }

    #[test]
    fn minimal_fields_use_defaults() {
        let request = ExportRequest::from_fields(base_fields()).unwrap();
        assert_eq!(request.format(), ExportFormat::Svg);
        assert_eq!(request.source(), &ExportSource::View { view: id(VIEW) });
        assert_eq!(request.target_profile().as_str(), "web-modern");
        assert_eq!(request.scale(), ExportScale::ONE);
        assert_eq!(request.background(), ExportBackground::Transparent);
        assert_eq!(
            request.output_path().as_path(),
            Path::new("/tmp/export/out.svg")
        );
        assert!(request.source().selection_nodes().is_none());
        assert_eq!(request.source().view_id(), id(VIEW));
    }

    #[test]
    fn full_fields_round_trip_canonically() {
        let fields = vec![
            ("output_path", "/Users/me/Exports/Card.PNG"),
            ("background", "display-p3(1,0.5,0,0.25)"),
            ("scale", "2.5"),
            ("target_profile", "apple-desktop"),
            (
                "selection",
                "0195C530-3ACD-7BC1-8414-C213F61CB967,0195c530-3acd-7bc1-8414-c213f61cb966",
            ),
            ("view", VIEW),
            ("source", "selection"),
            ("format", "png"),
            ("schema_version", "1"),
        ];
        let request = ExportRequest::from_fields(fields).unwrap();
        let nodes = request.source().selection_nodes().unwrap();
        assert_eq!(
            nodes.iter().copied().collect::<Vec<_>>(),
            vec![id(NODE_A), id(NODE_B)]
        );
        assert_eq!(request.scale().get(), 2.5);
        let canonical = request.to_fields().unwrap();
        let names: Vec<&str> = canonical.iter().map(|(n, _)| *n).collect();
        let schema_names: Vec<&str> = REQUEST_SCHEMA.iter().map(|s| s.name).collect();
        assert_eq!(names, schema_names, "canonical 출력은 schema 순서");
        assert_eq!(
            canonical,
            vec![
                ("schema_version", "1".to_string()),
                ("format", "png".to_string()),
                ("source", "selection".to_string()),
                ("view", VIEW.to_string()),
                ("selection", format!("{NODE_A},{NODE_B}")),
                ("target_profile", "apple-desktop".to_string()),
                ("scale", "2.5".to_string()),
                ("background", "display-p3(1,0.5,0,0.25)".to_string()),
                ("output_path", "/Users/me/Exports/Card.PNG".to_string()),
            ]
        );
        let again =
            ExportRequest::from_fields(canonical.iter().map(|(n, v)| (*n, v.as_str()))).unwrap();
        assert_eq!(again, request);
    }

    #[test]
    fn schema_violations_are_rejected() {
        // 검증 기준(XX.1.1): schema validation. 규칙마다 하나의 fixture.
        let cases: Vec<(Vec<(&'static str, &'static str)>, ExportRequestError)> = vec![
            (
                with(base_fields(), "colour", "red"),
                ExportRequestError::UnknownField {
                    name: "colour".to_string(),
                },
            ),
            (
                {
                    let mut f = base_fields();
                    f.push(("format", "svg"));
                    f
                },
                ExportRequestError::DuplicateField { field: "format" },
            ),
            (
                without(base_fields(), "output_path"),
                ExportRequestError::MissingField {
                    field: "output_path",
                },
            ),
            (
                without(base_fields(), "schema_version"),
                ExportRequestError::MissingField {
                    field: "schema_version",
                },
            ),
            (
                with(base_fields(), "schema_version", "2"),
                ExportRequestError::UnsupportedSchemaVersion,
            ),
            (
                with(base_fields(), "format", "PNG"),
                ExportRequestError::Malformed { field: "format" },
            ),
            (
                with(base_fields(), "format", "pdf"),
                ExportRequestError::Malformed { field: "format" },
            ),
            (
                with(base_fields(), "source", "page"),
                ExportRequestError::Malformed { field: "source" },
            ),
            (
                with(base_fields(), "view", "not-a-uuid"),
                ExportRequestError::Malformed { field: "view" },
            ),
            (
                with(
                    base_fields(),
                    "view",
                    "00000000-0000-0000-0000-000000000000",
                ),
                ExportRequestError::NilView,
            ),
            (
                with(base_fields(), "selection", NODE_A),
                ExportRequestError::FieldNotAllowed { field: "selection" },
            ),
            (
                with(base_fields(), "source", "selection"),
                ExportRequestError::MissingField { field: "selection" },
            ),
            (
                with(with(base_fields(), "source", "selection"), "selection", ""),
                ExportRequestError::Malformed { field: "selection" },
            ),
            (
                with(
                    with(base_fields(), "source", "selection"),
                    "selection",
                    "0195c530-3acd-7bc1-8414-c213f61cb966,",
                ),
                ExportRequestError::Malformed { field: "selection" },
            ),
            (
                with(
                    with(base_fields(), "source", "selection"),
                    "selection",
                    "0195c530-3acd-7bc1-8414-c213f61cb966,0195c530-3acd-7bc1-8414-c213f61cb966",
                ),
                ExportRequestError::DuplicateSelection { id: id(NODE_A) },
            ),
            (
                with(
                    with(base_fields(), "source", "selection"),
                    "selection",
                    "00000000-0000-0000-0000-000000000000",
                ),
                ExportRequestError::NilSelectionNode,
            ),
            (
                with(base_fields(), "target_profile", ""),
                ExportRequestError::ProfileKey(ProfileKeyError::Empty),
            ),
            (
                with(base_fields(), "target_profile", "-web"),
                ExportRequestError::ProfileKey(ProfileKeyError::InvalidStart),
            ),
            (
                with(base_fields(), "target_profile", "web modern"),
                ExportRequestError::ProfileKey(ProfileKeyError::InvalidChar { index: 3 }),
            ),
            (
                with(base_fields(), "scale", "0"),
                ExportRequestError::ScaleOutOfRange {
                    value: FiniteF32::ZERO,
                },
            ),
            (
                with(base_fields(), "scale", "16.5"),
                ExportRequestError::ScaleOutOfRange {
                    value: FiniteF32::new(16.5).unwrap(),
                },
            ),
            (
                with(base_fields(), "scale", "1e1"),
                ExportRequestError::Malformed { field: "scale" },
            ),
            (
                with(base_fields(), "scale", "NaN"),
                ExportRequestError::Malformed { field: "scale" },
            ),
            (
                with(base_fields(), "scale", "inf"),
                ExportRequestError::Malformed { field: "scale" },
            ),
            (
                with(base_fields(), "scale", "+2"),
                ExportRequestError::Malformed { field: "scale" },
            ),
            (
                with(base_fields(), "scale", ".5"),
                ExportRequestError::Malformed { field: "scale" },
            ),
            (
                with(base_fields(), "background", "red"),
                ExportRequestError::Malformed {
                    field: "background",
                },
            ),
            (
                with(base_fields(), "background", "srgb(1,0,0)"),
                ExportRequestError::Malformed {
                    field: "background",
                },
            ),
            (
                with(base_fields(), "background", "srgb(1,0,0,1,1)"),
                ExportRequestError::Malformed {
                    field: "background",
                },
            ),
            (
                with(base_fields(), "background", "srgb(1, 0, 0, 1)"),
                ExportRequestError::Malformed {
                    field: "background",
                },
            ),
            (
                with(base_fields(), "background", "srgb(1,0,0,2)"),
                ExportRequestError::Background(ColorError::AlphaOutOfRange),
            ),
            (
                with(base_fields(), "output_path", "out.svg"),
                ExportRequestError::OutputPath(OutputPathError::NotAbsolute),
            ),
            (
                with(base_fields(), "output_path", ""),
                ExportRequestError::OutputPath(OutputPathError::Empty),
            ),
            (
                with(base_fields(), "output_path", "/tmp/../etc/out.svg"),
                ExportRequestError::OutputPath(OutputPathError::ParentComponent),
            ),
            (
                with(base_fields(), "output_path", "/tmp/out\0.svg"),
                ExportRequestError::OutputPath(OutputPathError::ContainsNul),
            ),
            (
                with(base_fields(), "output_path", "/tmp/out.svg/"),
                ExportRequestError::OutputPath(OutputPathError::MissingFileName),
            ),
            (
                with(base_fields(), "output_path", "/"),
                ExportRequestError::OutputPath(OutputPathError::MissingFileName),
            ),
            (
                with(base_fields(), "output_path", "/tmp/out.png"),
                ExportRequestError::ExtensionMismatch {
                    format: ExportFormat::Svg,
                },
            ),
            (
                with(base_fields(), "output_path", "/tmp/out"),
                ExportRequestError::ExtensionMismatch {
                    format: ExportFormat::Svg,
                },
            ),
        ];
        for (fields, expected) in cases {
            assert_eq!(
                ExportRequest::from_fields(fields.clone()),
                Err(expected.clone()),
                "fields: {fields:?}"
            );
            // 모든 오류는 사용자 메시지를 가집니다.
            assert!(!expected.to_string().is_empty());
        }
    }

    #[test]
    fn untrusted_lengths_are_bounded_before_parsing() {
        let long_profile = "a".repeat(MAX_PROFILE_KEY_BYTES + 1);
        let fields = vec![
            ("schema_version", "1"),
            ("format", "svg"),
            ("source", "view"),
            ("view", VIEW),
            ("target_profile", long_profile.as_str()),
            ("output_path", "/tmp/out.svg"),
        ];
        assert_eq!(
            ExportRequest::from_fields(fields),
            Err(ExportRequestError::FieldTooLong {
                field: "target_profile",
                limit: MAX_PROFILE_KEY_BYTES
            })
        );
        let long_name = "x".repeat(10_000);
        match ExportRequest::from_fields(vec![(long_name.as_str(), "1")]) {
            Err(ExportRequestError::UnknownField { name }) => {
                assert_eq!(name.len(), MAX_REPORTED_NAME_BYTES)
            }
            other => panic!("unexpected {other:?}"),
        }
        // 다중 byte 문자 경계에서 자릅니다.
        let wide = "가".repeat(100);
        match ExportRequest::from_fields(vec![(wide.as_str(), "1")]) {
            Err(ExportRequestError::UnknownField { name }) => {
                assert!(name.len() <= MAX_REPORTED_NAME_BYTES);
                assert!(name.chars().all(|c| c == '가'));
            }
            other => panic!("unexpected {other:?}"),
        }
        let long_path = format!("/{}.svg", "p".repeat(MAX_OUTPUT_PATH_BYTES));
        let fields = vec![
            ("schema_version", "1"),
            ("format", "svg"),
            ("source", "view"),
            ("view", VIEW),
            ("target_profile", "web"),
            ("output_path", long_path.as_str()),
        ];
        assert_eq!(
            ExportRequest::from_fields(fields),
            Err(ExportRequestError::FieldTooLong {
                field: "output_path",
                limit: MAX_OUTPUT_PATH_BYTES
            })
        );
    }

    #[test]
    fn selection_limits_are_enforced() {
        let view = id(VIEW);
        let make = |n: u64| StableId::new_v7(1, 0, n + 1);
        assert_eq!(
            ExportSource::selection(view, Vec::new()),
            Err(ExportRequestError::EmptySelection)
        );
        let max = ExportSource::selection(view, (0..MAX_SELECTION as u64).map(make)).unwrap();
        assert_eq!(max.selection_nodes().unwrap().len(), MAX_SELECTION);
        // 상한을 넘으면 그 즉시 멈춥니다(무한 iterator도 종료).
        assert_eq!(
            ExportSource::selection(view, (0..).map(make)),
            Err(ExportRequestError::SelectionTooLarge {
                limit: MAX_SELECTION
            })
        );
        assert_eq!(
            ExportSource::selection(StableId::NIL, [make(1)]),
            Err(ExportRequestError::NilView)
        );
        assert_eq!(
            ExportSource::view(StableId::NIL),
            Err(ExportRequestError::NilView)
        );
        // 입력 순서와 무관하게 같은 source.
        assert_eq!(
            ExportSource::selection(view, [make(2), make(1)]).unwrap(),
            ExportSource::selection(view, [make(1), make(2)]).unwrap()
        );
    }

    #[test]
    fn typed_constructors_validate_values() {
        assert_eq!(
            ExportScale::new(f32::NAN),
            Err(ExportRequestError::ScaleNonFinite)
        );
        assert_eq!(ExportScale::new(MIN_SCALE).unwrap().get(), MIN_SCALE);
        assert_eq!(ExportScale::new(MAX_SCALE).unwrap().get(), MAX_SCALE);
        assert_eq!(ExportScale::default(), ExportScale::ONE);
        assert_eq!(
            TargetProfileKey::new(&"k".repeat(MAX_PROFILE_KEY_BYTES))
                .unwrap()
                .as_str()
                .len(),
            MAX_PROFILE_KEY_BYTES
        );
        assert_eq!(
            TargetProfileKey::new(&"k".repeat(MAX_PROFILE_KEY_BYTES + 1)),
            Err(ProfileKeyError::TooLong)
        );
        assert_eq!(
            TargetProfileKey::new("web_v2.1-a").unwrap().to_string(),
            "web_v2.1-a"
        );
        assert_eq!(
            TargetProfileKey::new("웹"),
            Err(ProfileKeyError::InvalidStart)
        );
        let path = OutputPath::new("/tmp/a.SvG").unwrap();
        assert!(path.has_extension_of(ExportFormat::Svg));
        assert!(!path.has_extension_of(ExportFormat::Png));
        // 확장자 불일치는 요청 생성에서 거부합니다.
        let source = ExportSource::view(id(VIEW)).unwrap();
        let profile = TargetProfileKey::new("web").unwrap();
        assert_eq!(
            ExportRequest::new(
                source.clone(),
                profile.clone(),
                ExportScale::ONE,
                ExportBackground::Transparent,
                path.clone(),
                ExportFormat::Png
            ),
            Err(ExportRequestError::ExtensionMismatch {
                format: ExportFormat::Png
            })
        );
        assert!(ExportRequest::new(
            source,
            profile,
            ExportScale::ONE,
            ExportBackground::Transparent,
            path,
            ExportFormat::Svg
        )
        .is_ok());
    }

    #[test]
    fn format_tags_are_stable() {
        assert_eq!(ExportFormat::Png.tag(), 1);
        assert_eq!(ExportFormat::Svg.tag(), 2);
        for format in ExportFormat::ALL {
            assert_eq!(ExportFormat::from_tag(format.tag()), Some(format));
            assert_eq!(ExportFormat::from_name(format.name()), Some(format));
            assert_eq!(format.to_string(), format.name());
        }
        assert_eq!(ExportFormat::from_tag(0), None);
        assert_eq!(ExportFormat::from_tag(3), None);
    }

    #[test]
    fn schema_table_is_consistent() {
        let mut names: Vec<&str> = REQUEST_SCHEMA.iter().map(|s| s.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), REQUEST_SCHEMA.len(), "field 이름 중복 없음");
        for schema in REQUEST_SCHEMA {
            assert!(schema.max_len > 0);
            assert!(!schema.syntax.is_empty());
        }
        let required: Vec<&str> = REQUEST_SCHEMA
            .iter()
            .filter(|s| s.requirement == FieldRequirement::Required)
            .map(|s| s.name)
            .collect();
        assert_eq!(
            required,
            vec![
                "schema_version",
                "format",
                "source",
                "view",
                "target_profile",
                "output_path"
            ]
        );
        // 최대 selection의 field 표현이 길이 상한 안에 들어갑니다.
        let selection = REQUEST_SCHEMA
            .iter()
            .find(|s| s.name == field::SELECTION)
            .unwrap();
        assert!(MAX_SELECTION * STABLE_ID_LEN + (MAX_SELECTION - 1) <= selection.max_len);
        // 극단 f32 값도 canonical 표현이 길이 상한 안입니다.
        for value in [f32::MAX, f32::MIN_POSITIVE, 1.0e-45f32, -f32::MAX] {
            assert!(value.to_string().len() <= MAX_DECIMAL_LEN);
            assert_eq!(parse_decimal(&value.to_string(), "x").unwrap(), value);
        }
    }

    #[test]
    fn background_round_trips_all_spaces() {
        for background in [
            ExportBackground::Transparent,
            ExportBackground::Solid(Color::new(ColorSpace::Srgb, 0.1, 0.2, 0.3, 1.0).unwrap()),
            ExportBackground::Solid(
                Color::new(ColorSpace::DisplayP3, 1.25, -0.5, 0.0, 0.0).unwrap(),
            ),
        ] {
            assert_eq!(
                parse_background(&background_text(background)),
                Ok(background)
            );
        }
    }
}
