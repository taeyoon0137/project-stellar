//! Android adapter (XXII.4). IR만 읽어 Jetpack Compose Kotlin 소스를 만듭니다.
//!
//! [`export_compose`]는 Android Target Profile IR의 layout·control semantics·접근성을 Compose API로 매핑한
//! `@Composable` 함수 하나를 씁니다(XXII.4.2, 규칙은 [`compose`]). import는 [`api`] 표의 symbol만 쓰고, 사용 여부로
//! 계산해 정렬합니다. 정적 검사는 [`check::check_kotlin`]입니다.
//!
//! 같은 IR과 옵션은 항상 같은 bytes를 만듭니다.

use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_target_profile::spec::ProfileSpec;

use crate::facet::Facet;
use crate::ir::CodegenIr;

/// Kotlin 소스 도우미.
pub mod kotlin;

/// 생성 코드가 쓰는 Compose API 표.
pub mod api;

/// layout·control semantics·접근성 → Compose 매핑.
pub mod compose;

/// 생성 Kotlin 정적 검사.
pub mod check;

#[cfg(test)]
mod tests;

/// layout 노트 facet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LayoutFacet {
    /// Stack main 정렬과 gap 조합.
    Gap,
    /// Stack 역방향.
    Reverse,
    /// wrap 줄 정렬.
    Wrap,
    /// 백분율 sizing.
    Sizing,
    /// flex grow·basis.
    Flex,
    /// Free anchor.
    Position,
    /// local transform.
    Transform,
    /// Grid track 크기.
    GridTracks,
    /// Grid item 위치.
    GridPlacement,
    /// Anchor·Constraint 배치.
    Constraint,
    /// leaf 콘텐츠(이미지 리소스, 알 수 없는 텍스트, text field 장식).
    Content,
}

/// 노트 facet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ComposeFacet {
    /// 접근성·control 의미.
    Semantics(Facet),
    /// layout.
    Layout(LayoutFacet),
}

/// 노트 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ComposeNoteKind {
    /// Compose API가 근사로만 표현합니다.
    Approximated(ComposeFacet),
    /// 대응 표현이 없어 쓰지 못했습니다.
    NoEquivalent(ComposeFacet),
    /// binding 대상이 IR에 없거나 Kotlin 타입으로 표현할 수 없어 쓰지 못했습니다.
    UnresolvedBinding(Facet),
    /// 컴포넌트 Property에 묶여 있어 쓰지 못했습니다(입력 주입은 XXII.4.1).
    ComponentProperty(Facet),
    /// profile Compose UI 버전에서 쓸 수 없는 API라 쓰지 않았습니다.
    Unavailable(ComposeFacet),
}

/// 노트.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ComposeNote {
    /// IR node.
    pub node: StableId,
    /// 종류.
    pub kind: ComposeNoteKind,
}

/// 내보내기 옵션.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposeOptions {
    /// Kotlin package(점으로 구분한 식별자).
    pub package: String,
    /// `@Composable` 함수 이름(ASCII 영문자로 시작하는 식별자).
    pub function_name: String,
}

/// 내보내기 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposeExport {
    /// Kotlin 소스.
    pub source: String,
    /// 노트(node 문서 순서 → 종류 순).
    pub notes: Vec<ComposeNote>,
    /// 검사 기준 Compose UI 버전(major, minor).
    pub ui_version: (u16, u16),
}

/// 내보내기 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComposeError {
    /// IR이 Android profile이 아닙니다.
    NotAndroidProfile,
    /// package 이름이 식별자 목록이 아닙니다.
    InvalidPackage,
    /// 함수 이름이 식별자가 아닙니다.
    InvalidFunctionName,
}

impl fmt::Display for ComposeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ComposeError::NotAndroidProfile => f.write_str("Android Target Profile IR이 아닙니다"),
            ComposeError::InvalidPackage => {
                f.write_str("package는 점으로 구분한 ASCII 식별자여야 합니다")
            }
            ComposeError::InvalidFunctionName => {
                f.write_str("함수 이름은 ASCII 영문자로 시작하는 식별자여야 합니다")
            }
        }
    }
}

impl std::error::Error for ComposeError {}

fn is_identifier(text: &str) -> bool {
    text.starts_with(|c: char| c.is_ascii_alphabetic())
        && text.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !kotlin::RESERVED.contains(&text)
}

/// layout·control semantics·접근성을 Compose API로 매핑한 Kotlin 소스를 만듭니다.
pub fn export_compose(
    ir: &CodegenIr,
    options: &ComposeOptions,
) -> Result<ComposeExport, ComposeError> {
    let ProfileSpec::Android(android) = ir.target.profile.spec else {
        return Err(ComposeError::NotAndroidProfile);
    };
    if options.package.is_empty() || !options.package.split('.').all(is_identifier) {
        return Err(ComposeError::InvalidPackage);
    }
    if !is_identifier(&options.function_name) {
        return Err(ComposeError::InvalidFunctionName);
    }
    let ui = android.ui_system_version;
    let ui_version = (ui.major(), ui.minor());
    let body = compose::map_compose(ir, ui_version);
    let mut code = String::new();
    if body.uses_flow {
        code.push_str("@OptIn(ExperimentalLayoutApi::class)\n");
    }
    code.push_str("@Composable\n");
    if body.uses_action {
        code.push_str(&format!(
            "fun {}(onAction: (String) -> Unit = {{}}) {{\n",
            options.function_name
        ));
    } else {
        code.push_str(&format!("fun {}() {{\n", options.function_name));
    }
    code.push_str(&body.body);
    code.push_str("}\n");
    let used = check::used_symbols(&code);
    let mut imports: Vec<&str> = api::COMPOSE_API
        .iter()
        .filter(|entry| used.contains(entry.name))
        .map(|entry| entry.import)
        .collect();
    imports.sort_unstable();
    let mut source = format!("package {}\n\n", options.package);
    for import in imports {
        source.push_str(&format!("import {import}\n"));
    }
    source.push('\n');
    source.push_str(&code);
    Ok(ComposeExport {
        source,
        notes: body.notes,
        ui_version,
    })
}
