//! 저장 View → SwiftUI·AppKit·Compose 소스 (`stellar export swiftui|appkit|compose`, ADR-0107 결정 8).
//!
//! 구조·배치·layout 값은 Web 투영(`web_frame::export_web_frame_node`)이 정한 것을 그대로 씁니다. 그래서 받는 node
//! 범위·실패 진단·배치 규칙(ADR-0089 결정 4)이 `export-web`과 같습니다. 투영 결과의 요소마다 활성 기본값 세트로 푼
//! 40개 longhand 선언(`ResolvedLayout::declarations`)을 IR 입력으로 넘기고, 배치가 정한 값만 바꿉니다.
//!
//! - `free`: 저장 transform 이동을 `position: absolute` + `left`·`top`으로 냅니다(ADR-0089 결정 4).
//! - `inset`: `position: absolute`와 record inset. 이동은 layout 뒤 시각 이동입니다.
//! - `flow`: 풀린 값 그대로. 이동은 시각 이동입니다.
//! - 숨김: `display: none`.
//!
//! Target Profile은 대상마다 내장 기본 profile(iOS·macOS 26.0, Android API 35·Compose UI 1.7.0)입니다. 저장 profile을
//! 아직 IR registry pin과 잇지 못하기 때문입니다(ADR-0049). 파일을 쓰지 않습니다.

use stellar_codegen::android::{export_compose, ComposeOptions};
use stellar_codegen::apple::native_input::{build_native_ir, NativeNodeInput};
use stellar_codegen::apple::{export_apple_ui, AppleDialect, AppleOptions};
use stellar_document::css::value::format_length;
use stellar_document::records::NodePayload;
use stellar_target_profile::capability::ElementKind;
use stellar_target_profile::profile::TargetProfileId;
use stellar_target_profile::registry::ProfileRegistryVersion;
use stellar_target_profile::spec::{
    AndroidApiLevel, AndroidProfile, ApplePlatform, AppleProfile, DottedVersion, ProfileSpec,
    TargetProfile,
};

use crate::core_types::ids::StableId;
use crate::export_css::Placement;
use crate::revision::ProjectRevision;
use crate::snapshot::ProjectSnapshot;
use crate::web_frame::{export_web_frame_node, HtmlDiagnostic, HtmlError};

/// 출력 대상.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeTarget {
    /// SwiftUI(iOS).
    SwiftUiIos,
    /// SwiftUI(macOS).
    SwiftUiMacOs,
    /// AppKit(macOS).
    AppKit,
    /// Jetpack Compose.
    Compose,
}

impl NativeTarget {
    /// 출력 이름.
    pub fn name(self) -> &'static str {
        match self {
            NativeTarget::SwiftUiIos | NativeTarget::SwiftUiMacOs => "swiftui",
            NativeTarget::AppKit => "appkit",
            NativeTarget::Compose => "compose",
        }
    }

    /// 내장 기본 profile.
    pub fn profile(self) -> TargetProfile {
        let pin = ProfileRegistryVersion::new(1).expect("registry 1");
        let apple = |id: &str, platform| {
            TargetProfile::new(
                TargetProfileId::new(id).expect("id"),
                pin,
                ProfileSpec::Apple(AppleProfile {
                    platform,
                    minimum_os: DottedVersion::new(26, 0, 0),
                }),
            )
        };
        match self {
            NativeTarget::SwiftUiMacOs | NativeTarget::AppKit => {
                apple("macos", ApplePlatform::MacOs)
            }
            NativeTarget::SwiftUiIos => apple("ios", ApplePlatform::Ios),
            NativeTarget::Compose => TargetProfile::new(
                TargetProfileId::new("android").expect("id"),
                pin,
                ProfileSpec::Android(AndroidProfile {
                    api_level: AndroidApiLevel::new(35).expect("api"),
                    ui_system_version: DottedVersion::new(1, 7, 0),
                }),
            ),
        }
    }
}

/// 결과. 오류 진단이 하나라도 있으면 `source`가 `None`입니다.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeExport {
    /// 열린 저장 generation.
    pub revision: ProjectRevision,
    /// 요청 문서.
    pub document: StableId,
    /// 요청 View.
    pub view: StableId,
    /// 투영 root.
    pub root_frame: StableId,
    /// 대상.
    pub target: NativeTarget,
    /// 내장 profile id.
    pub profile: String,
    /// 생성 소스.
    pub source: Option<String>,
    /// Web 투영 진단과 adapter 노트(노트는 경고, code `native-note`).
    pub diagnostics: Vec<HtmlDiagnostic>,
}

fn replace(declarations: &mut [(&'static str, String)], name: &str, value: String) {
    if let Some(entry) = declarations.iter_mut().find(|(n, _)| *n == name) {
        entry.1 = value;
    }
}

/// 저장 View 또는 그 안 Frame 하나를 대상 소스로 내보냅니다.
pub fn export_native(
    snapshot: &ProjectSnapshot,
    document: StableId,
    view: StableId,
    frame: Option<StableId>,
    target: NativeTarget,
) -> Result<NativeExport, HtmlError> {
    let web = export_web_frame_node(snapshot, document, view, frame)?;
    let profile = target.profile();
    let mut result = NativeExport {
        revision: web.revision,
        document,
        view,
        root_frame: web.root_frame,
        target,
        profile: profile.id.as_str().to_string(),
        source: None,
        diagnostics: web
            .diagnostics
            .iter()
            .filter(|d| d.code != "fixed-free-frame-only")
            .cloned()
            .collect(),
    };
    if web.diagnostics.iter().any(|d| d.error) {
        return Ok(result);
    }
    let model = snapshot
        .document_model(document)
        .map_err(HtmlError::Document)?;
    let styles: std::collections::BTreeMap<StableId, &crate::export_css::ElementStyle> =
        web.styles.iter().map(|s| (s.node, s)).collect();
    let mut note = |error: bool, code: &'static str, node: StableId, message: String| {
        result.diagnostics.push(HtmlDiagnostic {
            error,
            node,
            code,
            message: format!("node {node}: {message}"),
        });
    };
    note(
        false,
        "native-scope",
        web.root_frame,
        "저장 구조·layout·기본 접근성만 냅니다. paint(배경·테두리·반경·clip), control contract·상태·token·Variant·Context·Motion은 문서에서 IR로 아직 잇지 못해 내지 않습니다".to_string(),
    );
    // 문서 preorder(형제는 OrderKey 순).
    let mut inputs = Vec::new();
    let mut stack = vec![(web.root_frame, None)];
    while let Some((id, parent)) = stack.pop() {
        let Some(payload) = model.payload(id) else {
            note(
                true,
                "unsupported-component-instance",
                id,
                "펼친 컴포넌트 node는 native 출력에서 아직 받지 않습니다".to_string(),
            );
            return Ok(result);
        };
        let element = match payload {
            NodePayload::Frame(_) => ElementKind::FRAME,
            NodePayload::Fragment(_) => ElementKind::FRAGMENT,
            NodePayload::Shape(_) => ElementKind::SHAPE,
            _ => {
                note(
                    true,
                    "unsupported-node",
                    id,
                    "이 node 종류는 native 출력에서 아직 받지 않습니다".to_string(),
                );
                return Ok(result);
            }
        };
        let mut declarations = Vec::new();
        let mut translation = (0.0, 0.0);
        let mut hidden = false;
        if element != ElementKind::FRAGMENT {
            let Some(style) = styles.get(&id) else {
                note(
                    true,
                    "unsupported-component-instance",
                    id,
                    "Web 투영 계획이 없는 node입니다".to_string(),
                );
                return Ok(result);
            };
            declarations = style.resolved.declarations();
            hidden = !style.visible;
            match style.placement {
                Placement::Free => {
                    replace(&mut declarations, "position", "absolute".to_string());
                    replace(
                        &mut declarations,
                        "left",
                        format_length(style.translation.0),
                    );
                    replace(&mut declarations, "top", format_length(style.translation.1));
                }
                Placement::Inset => {
                    replace(&mut declarations, "position", "absolute".to_string());
                    translation = style.translation;
                }
                Placement::Flow => translation = style.translation,
                Placement::Root => {}
            }
            if !style.linear_identity {
                note(
                    false,
                    "native-linear-transform",
                    id,
                    "저장 transform의 선형 부분(회전·확대)은 native 출력에 내지 않습니다"
                        .to_string(),
                );
            }
            if hidden {
                replace(&mut declarations, "display", "none".to_string());
            }
        }
        inputs.push(NativeNodeInput {
            id,
            parent,
            element,
            declarations,
            translation,
            text: None,
            hidden,
        });
        let children = model.children_in_order(id);
        stack.extend(children.into_iter().rev().map(|child| (child, Some(id))));
    }
    let ir = match build_native_ir(&inputs, &profile) {
        Ok(ir) => ir,
        Err(error) => {
            note(true, "native-projection", web.root_frame, error.to_string());
            return Ok(result);
        }
    };
    let (source, notes): (String, Vec<(StableId, String)>) = match target {
        NativeTarget::SwiftUiIos | NativeTarget::SwiftUiMacOs | NativeTarget::AppKit => {
            let dialect = if target == NativeTarget::AppKit {
                AppleDialect::AppKit
            } else {
                AppleDialect::SwiftUi
            };
            match export_apple_ui(&ir, &AppleOptions::new(dialect)) {
                Ok(export) => (
                    export.source,
                    export
                        .notes
                        .iter()
                        .map(|n| {
                            let message = match &n.kind {
                                stellar_codegen::apple::AppleNoteKind::Layout(finding) => {
                                    layout_message(finding)
                                }
                                other => format!("{other:?}"),
                            };
                            (subject_id(&n.subject), message)
                        })
                        .collect(),
                ),
                Err(error) => {
                    note(true, "native-export", web.root_frame, error.to_string());
                    return Ok(result);
                }
            }
        }
        NativeTarget::Compose => match export_compose(&ir, &ComposeOptions::default()) {
            Ok(export) => (
                export.source,
                export
                    .notes
                    .iter()
                    .map(|n| {
                        let message = match &n.kind {
                            stellar_codegen::android::AndroidNoteKind::Layout(finding) => {
                                layout_message(finding)
                            }
                            other => format!("{other:?}"),
                        };
                        (android_subject_id(&n.subject), message)
                    })
                    .collect(),
            ),
            Err(error) => {
                note(true, "native-export", web.root_frame, error.to_string());
                return Ok(result);
            }
        },
    };
    for (node, message) in notes {
        note(false, "native-note", node, message);
    }
    result.source = Some(source);
    Ok(result)
}

/// layout 규칙 진단은 CSS longhand·상태·code·이유를 그대로 읽을 수 있게 냅니다(Rust Debug 표기가 아님).
fn layout_message(finding: &stellar_codegen::apple::LayoutFinding) -> String {
    format!(
        "{} {:?}({}): {}",
        finding.longhand, finding.status, finding.code, finding.reason
    )
}

fn subject_id(subject: &stellar_codegen::apple::AppleNoteSubject) -> StableId {
    match subject {
        stellar_codegen::apple::AppleNoteSubject::Node(id)
        | stellar_codegen::apple::AppleNoteSubject::Token(id) => *id,
    }
}

fn android_subject_id(subject: &stellar_codegen::android::AndroidNoteSubject) -> StableId {
    match subject {
        stellar_codegen::android::AndroidNoteSubject::Node(id)
        | stellar_codegen::android::AndroidNoteSubject::Token(id) => *id,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export_css::fixture::{self, id};
    use crate::snapshot::open_snapshot;
    use crate::test_support::{save_documents, scaffold_project, TempDir};

    #[test]
    fn saved_view_exports_to_every_native_target() {
        let dir = TempDir::new("export-native");
        scaffold_project(&dir.path);
        save_documents(&dir.path, &[&fixture::document()]);
        let snapshot = open_snapshot(&dir.path).expect("snapshot");
        for (target, marker) in [
            (NativeTarget::SwiftUiIos, "struct StellarView: View"),
            (NativeTarget::SwiftUiMacOs, "struct StellarView: View"),
            (NativeTarget::AppKit, "final class StellarView: NSView"),
            (NativeTarget::Compose, "fun StellarView("),
        ] {
            let first = export_native(&snapshot, id(1), id(3), None, target).expect("export");
            let errors: Vec<_> = first.diagnostics.iter().filter(|d| d.error).collect();
            assert!(errors.is_empty(), "{errors:?}");
            let source = first.source.clone().expect("source");
            // layout 규칙 진단은 longhand·상태·code·이유로 읽히고 Rust Debug 구조체 표기가 아닙니다.
            let notes: Vec<_> = first
                .diagnostics
                .iter()
                .filter(|d| d.code == "native-note")
                .collect();
            assert!(
                notes
                    .iter()
                    .any(|d| d.message.contains(if target == NativeTarget::AppKit {
                        "transform Unsupported(transform): "
                    } else {
                        "flex-shrink Approximated(platform-compression): "
                    })),
                "{notes:?}"
            );
            assert!(notes.iter().all(|d| !d.message.contains("LayoutFinding")));
            assert!(source.contains(marker), "{source}");
            assert_eq!(
                first,
                export_native(&snapshot, id(1), id(3), None, target).expect("export"),
                "결정론"
            );
        }
    }
}
