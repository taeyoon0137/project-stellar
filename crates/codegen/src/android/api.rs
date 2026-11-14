//! 생성 코드가 쓰는 Jetpack Compose API 표(XXII.4.2).
//!
//! 생성기는 이 표에 있는 symbol만 쓰고, 정적 검사([`crate::android::check`])는 같은 표로 import·사용 형태·최소 Compose UI
//! 버전을 확인합니다. 버전은 `stellar.yaml` Android profile의 `ui_system_version`(Compose UI)과 비교합니다.

/// symbol 사용 형태.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ApiKind {
    /// top-level 함수·composable·타입(`Box(`, `Modifier`, `Role.Button`).
    TopLevel,
    /// 확장 함수·속성(`.width(`, `16.dp`).
    Extension,
    /// semantics 블록의 속성 대입 문장(`contentDescription = …`).
    SemanticsProperty,
    /// semantics 블록의 함수 호출 문장(`disabled()`).
    SemanticsFunction,
    /// 위임 연산자(`by remember { … }`).
    Delegate,
}

/// API 항목.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComposeApi {
    /// 단순 이름.
    pub name: &'static str,
    /// import 경로.
    pub import: &'static str,
    /// 최소 Compose UI 버전(major, minor).
    pub since: (u16, u16),
    /// 사용 형태.
    pub kind: ApiKind,
}

const fn api(
    name: &'static str,
    import: &'static str,
    since: (u16, u16),
    kind: ApiKind,
) -> ComposeApi {
    ComposeApi {
        name,
        import,
        since,
        kind,
    }
}

use ApiKind::{Delegate, Extension, SemanticsFunction, SemanticsProperty, TopLevel};

/// 표(이름 오름차순이 아니라 분류 순, 이름은 겹치지 않음).
pub const COMPOSE_API: &[ComposeApi] = &[
    api(
        "Composable",
        "androidx.compose.runtime.Composable",
        (1, 0),
        TopLevel,
    ),
    api(
        "remember",
        "androidx.compose.runtime.remember",
        (1, 0),
        TopLevel,
    ),
    api(
        "mutableStateOf",
        "androidx.compose.runtime.mutableStateOf",
        (1, 0),
        TopLevel,
    ),
    api(
        "getValue",
        "androidx.compose.runtime.getValue",
        (1, 0),
        Delegate,
    ),
    api(
        "setValue",
        "androidx.compose.runtime.setValue",
        (1, 0),
        Delegate,
    ),
    api("Modifier", "androidx.compose.ui.Modifier", (1, 0), TopLevel),
    api(
        "Alignment",
        "androidx.compose.ui.Alignment",
        (1, 0),
        TopLevel,
    ),
    api("dp", "androidx.compose.ui.unit.dp", (1, 0), Extension),
    api(
        "Arrangement",
        "androidx.compose.foundation.layout.Arrangement",
        (1, 0),
        TopLevel,
    ),
    api(
        "Box",
        "androidx.compose.foundation.layout.Box",
        (1, 0),
        TopLevel,
    ),
    api(
        "Row",
        "androidx.compose.foundation.layout.Row",
        (1, 0),
        TopLevel,
    ),
    api(
        "Column",
        "androidx.compose.foundation.layout.Column",
        (1, 0),
        TopLevel,
    ),
    api(
        "FlowRow",
        "androidx.compose.foundation.layout.FlowRow",
        (1, 4),
        TopLevel,
    ),
    api(
        "FlowColumn",
        "androidx.compose.foundation.layout.FlowColumn",
        (1, 4),
        TopLevel,
    ),
    api(
        "ExperimentalLayoutApi",
        "androidx.compose.foundation.layout.ExperimentalLayoutApi",
        (1, 4),
        TopLevel,
    ),
    api(
        "width",
        "androidx.compose.foundation.layout.width",
        (1, 0),
        Extension,
    ),
    api(
        "height",
        "androidx.compose.foundation.layout.height",
        (1, 0),
        Extension,
    ),
    api(
        "size",
        "androidx.compose.foundation.layout.size",
        (1, 0),
        Extension,
    ),
    api(
        "fillMaxWidth",
        "androidx.compose.foundation.layout.fillMaxWidth",
        (1, 0),
        Extension,
    ),
    api(
        "fillMaxHeight",
        "androidx.compose.foundation.layout.fillMaxHeight",
        (1, 0),
        Extension,
    ),
    api(
        "widthIn",
        "androidx.compose.foundation.layout.widthIn",
        (1, 0),
        Extension,
    ),
    api(
        "heightIn",
        "androidx.compose.foundation.layout.heightIn",
        (1, 0),
        Extension,
    ),
    api(
        "aspectRatio",
        "androidx.compose.foundation.layout.aspectRatio",
        (1, 0),
        Extension,
    ),
    api(
        "padding",
        "androidx.compose.foundation.layout.padding",
        (1, 0),
        Extension,
    ),
    api(
        "absolutePadding",
        "androidx.compose.foundation.layout.absolutePadding",
        (1, 0),
        Extension,
    ),
    api(
        "offset",
        "androidx.compose.foundation.layout.offset",
        (1, 0),
        Extension,
    ),
    api(
        "LazyVerticalGrid",
        "androidx.compose.foundation.lazy.grid.LazyVerticalGrid",
        (1, 2),
        TopLevel,
    ),
    api(
        "GridCells",
        "androidx.compose.foundation.lazy.grid.GridCells",
        (1, 2),
        TopLevel,
    ),
    api(
        "GridItemSpan",
        "androidx.compose.foundation.lazy.grid.GridItemSpan",
        (1, 2),
        TopLevel,
    ),
    api(
        "BasicText",
        "androidx.compose.foundation.text.BasicText",
        (1, 0),
        TopLevel,
    ),
    api(
        "BasicTextField",
        "androidx.compose.foundation.text.BasicTextField",
        (1, 0),
        TopLevel,
    ),
    api(
        "Dialog",
        "androidx.compose.ui.window.Dialog",
        (1, 0),
        TopLevel,
    ),
    api(
        "clickable",
        "androidx.compose.foundation.clickable",
        (1, 0),
        Extension,
    ),
    api(
        "focusable",
        "androidx.compose.foundation.focusable",
        (1, 0),
        Extension,
    ),
    api(
        "selectable",
        "androidx.compose.foundation.selection.selectable",
        (1, 0),
        Extension,
    ),
    api(
        "selectableGroup",
        "androidx.compose.foundation.selection.selectableGroup",
        (1, 0),
        Extension,
    ),
    api(
        "semantics",
        "androidx.compose.ui.semantics.semantics",
        (1, 0),
        Extension,
    ),
    api(
        "clearAndSetSemantics",
        "androidx.compose.ui.semantics.clearAndSetSemantics",
        (1, 0),
        Extension,
    ),
    api(
        "Role",
        "androidx.compose.ui.semantics.Role",
        (1, 0),
        TopLevel,
    ),
    api(
        "CollectionInfo",
        "androidx.compose.ui.semantics.CollectionInfo",
        (1, 0),
        TopLevel,
    ),
    api(
        "CollectionItemInfo",
        "androidx.compose.ui.semantics.CollectionItemInfo",
        (1, 0),
        TopLevel,
    ),
    api(
        "ToggleableState",
        "androidx.compose.ui.state.ToggleableState",
        (1, 0),
        TopLevel,
    ),
    api(
        "contentDescription",
        "androidx.compose.ui.semantics.contentDescription",
        (1, 0),
        SemanticsProperty,
    ),
    api(
        "stateDescription",
        "androidx.compose.ui.semantics.stateDescription",
        (1, 0),
        SemanticsProperty,
    ),
    api(
        "selected",
        "androidx.compose.ui.semantics.selected",
        (1, 0),
        SemanticsProperty,
    ),
    api(
        "focused",
        "androidx.compose.ui.semantics.focused",
        (1, 0),
        SemanticsProperty,
    ),
    api(
        "paneTitle",
        "androidx.compose.ui.semantics.paneTitle",
        (1, 0),
        SemanticsProperty,
    ),
    api(
        "role",
        "androidx.compose.ui.semantics.role",
        (1, 0),
        SemanticsProperty,
    ),
    api(
        "toggleableState",
        "androidx.compose.ui.semantics.toggleableState",
        (1, 0),
        SemanticsProperty,
    ),
    api(
        "collectionInfo",
        "androidx.compose.ui.semantics.collectionInfo",
        (1, 0),
        SemanticsProperty,
    ),
    api(
        "collectionItemInfo",
        "androidx.compose.ui.semantics.collectionItemInfo",
        (1, 0),
        SemanticsProperty,
    ),
    api(
        "traversalIndex",
        "androidx.compose.ui.semantics.traversalIndex",
        (1, 5),
        SemanticsProperty,
    ),
    api(
        "isTraversalGroup",
        "androidx.compose.ui.semantics.isTraversalGroup",
        (1, 5),
        SemanticsProperty,
    ),
    api(
        "disabled",
        "androidx.compose.ui.semantics.disabled",
        (1, 0),
        SemanticsFunction,
    ),
];

/// 이름의 API 항목.
pub fn lookup(name: &str) -> Option<&'static ComposeApi> {
    COMPOSE_API.iter().find(|entry| entry.name == name)
}

/// import 경로의 API 항목.
pub fn by_import(import: &str) -> Option<&'static ComposeApi> {
    COMPOSE_API.iter().find(|entry| entry.import == import)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_names_and_imports_are_unique_and_consistent() {
        for (index, entry) in COMPOSE_API.iter().enumerate() {
            assert!(
                entry.import.ends_with(&format!(".{}", entry.name)),
                "{}",
                entry.name
            );
            assert!(
                entry.import.starts_with("androidx.compose."),
                "{}",
                entry.import
            );
            assert!(
                COMPOSE_API[index + 1..]
                    .iter()
                    .all(|other| other.name != entry.name),
                "중복 {}",
                entry.name
            );
        }
        assert_eq!(lookup("traversalIndex").map(|e| e.since), Some((1, 5)));
        assert_eq!(
            by_import("androidx.compose.ui.unit.dp").map(|e| e.kind),
            Some(ApiKind::Extension)
        );
    }
}
