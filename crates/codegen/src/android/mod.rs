//! Android adapter (XXII.4). IR만 읽어 Jetpack Compose Kotlin 소스를 만듭니다.
//!
//! [`export_compose`]의 결과는 Kotlin 파일 하나입니다(ADR-0107 결정 6).
//!
//! - `object <prefix>Colors`: Color token과 Color 값 node 속성(token에 묶인 속성은 token 상수를 가리킴).
//! - `class <prefix>State`: 상태 변수를 `mutableStateOf`로 담는 상태 holder. 생성한 composable은 이 holder를 받아
//!   상태를 끌어올립니다(state hoisting).
//! - `@Composable fun <prefix>View(state, onAction)`: root composable. semantic action은 `onAction(<action StableId>)`.
//! - 출력 node마다 `@Composable private fun <부모 scope>.Node<StableId 16진>(state, onAction)`. 부모 container의 scope
//!   (`RowScope`·`ColumnScope`·`BoxScope`·Flow scope)를 receiver로 받아 `weight`·`align` 같은 item modifier를 씁니다.
//!   node마다 함수를 두어 재귀 없이 씁니다.
//!
//! CSS layout longhand는 [`layout::COMPOSE_RULES`]로 낮춥니다. 구조 분석(채움·확정 크기·grid 칸)은 Apple adapter와 같은
//! [`crate::apple::native`]를 씁니다. 같은 IR과 옵션은 항상 같은 bytes를 만듭니다.

use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_target_profile::spec::{AndroidProfile, ProfileSpec};

use crate::apple::{ControlNote, LayoutFinding};
use crate::facet::Facet;
use crate::ir::CodegenIr;
use crate::text::identifier;

/// Kotlin 소스 도우미.
pub mod kotlin;

/// 색공간 → Compose `Color`.
pub mod color;

/// CSS layout longhand → Compose layout 규칙.
pub mod layout;

/// Compose 생성기.
pub(crate) mod compose;

#[cfg(test)]
mod tests;

/// 내보내기 옵션.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposeOptions {
    /// Kotlin package(소문자로 시작하는 `[a-z][a-z0-9_]*` 조각을 `.`로 이은 이름).
    pub package: String,
    /// 생성 타입·함수 이름 접두어(ASCII 영문자로 시작하는 식별자).
    pub type_prefix: String,
}

impl Default for ComposeOptions {
    fn default() -> ComposeOptions {
        ComposeOptions {
            package: "stellar.generated".to_string(),
            type_prefix: "Stellar".to_string(),
        }
    }
}

/// 색 변환 노트.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ColorNote {
    /// 채널이 `[0, 1]` 밖입니다.
    ExtendedRange,
    /// Compose `Color` 생성자가 색공간 범위 밖 채널을 받지 않아 `[0, 1]`로 잘랐습니다.
    Clamped,
}

/// 노트 대상.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AndroidNoteSubject {
    /// IR node.
    Node(StableId),
    /// token.
    Token(StableId),
}

/// 노트 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AndroidNoteKind {
    /// 색 변환.
    Color(ColorNote),
    /// Control·element 출력.
    Control(ControlNote),
    /// layout 규칙 진단.
    Layout(LayoutFinding),
    /// 대상 API가 이 의미를 근사로만 표현합니다.
    Approximated(Facet),
    /// 대상 API에 대응 표현이 없습니다.
    NoEquivalent(Facet),
    /// binding 대상이 IR에 없거나 값 타입을 Kotlin으로 표현할 수 없습니다.
    UnresolvedBinding(Facet),
    /// 컴포넌트 Property에 묶여 있습니다.
    ComponentProperty(Facet),
    /// profile의 Compose UI 버전에서 쓸 수 없는 API입니다.
    Unavailable(Facet),
}

/// 노트.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AndroidNote {
    /// 대상.
    pub subject: AndroidNoteSubject,
    /// 종류.
    pub kind: AndroidNoteKind,
}

/// 내보내기 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposeExport {
    /// Kotlin 소스.
    pub source: String,
    /// 노트(대상 → 종류 순).
    pub notes: Vec<AndroidNote>,
}

/// 내보내기 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComposeError {
    /// IR이 Android profile이 아닙니다.
    NotAndroidProfile,
    /// 타입 접두어가 식별자가 아닙니다.
    InvalidTypePrefix,
    /// package 이름이 규칙에 맞지 않습니다.
    InvalidPackage,
}

impl fmt::Display for ComposeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ComposeError::NotAndroidProfile => f.write_str("Android Target Profile IR이 아닙니다"),
            ComposeError::InvalidTypePrefix => {
                f.write_str("타입 접두어는 ASCII 영문자로 시작하는 식별자여야 합니다")
            }
            ComposeError::InvalidPackage => f.write_str(
                "package는 소문자로 시작하는 [a-z][a-z0-9_]* 조각을 .으로 이은 이름이어야 합니다",
            ),
        }
    }
}

impl std::error::Error for ComposeError {}

fn android_profile(ir: &CodegenIr) -> Option<AndroidProfile> {
    match ir.target.profile.spec {
        ProfileSpec::Android(android) => Some(android),
        _ => None,
    }
}

fn valid_package(package: &str) -> bool {
    !package.is_empty()
        && package.split('.').all(|part| {
            part.starts_with(|c: char| c.is_ascii_lowercase())
                && part
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
                && !kotlin::RESERVED.contains(&part)
        })
}

/// IR을 Compose Kotlin 소스로 내보냅니다(XXII.4.1·XXII.4.2).
pub fn export_compose(
    ir: &CodegenIr,
    options: &ComposeOptions,
) -> Result<ComposeExport, ComposeError> {
    let android = android_profile(ir).ok_or(ComposeError::NotAndroidProfile)?;
    let prefix = &options.type_prefix;
    if !(prefix.starts_with(|c: char| c.is_ascii_alphabetic()) && identifier(prefix, "") == *prefix)
    {
        return Err(ComposeError::InvalidTypePrefix);
    }
    if !valid_package(&options.package) {
        return Err(ComposeError::InvalidPackage);
    }
    let mut notes = Vec::new();
    let analysis = crate::apple::native::analyze(ir);
    let mut generator = compose::Generator::new(ir, &analysis, android, prefix, &mut notes);
    let source = generator.source(&options.package);
    notes.sort();
    notes.dedup();
    Ok(ComposeExport { source, notes })
}
