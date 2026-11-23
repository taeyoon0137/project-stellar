//! Apple adapter (XXII.3). IR만 읽어 Swift 소스를 만듭니다.
//!
//! [`export_apple`]은 Apple Target Profile IR에서 접근성 의미와 색공간을 지정한 dialect의 Apple API로 내보냅니다
//! (XXII.3.3). 결과는 Swift 파일 하나입니다.
//!
//! - **색**: Color token 전체와 Color 값을 가진 node 속성을 `<prefix>Colors` 상수로 둡니다. token에 묶인 속성은
//!   token 상수를 가리킵니다([`color`] 표).
//! - **접근성**: SwiftUI는 node마다 `ViewModifier`(`<prefix>Accessibility<StableId 16진>`), AppKit은
//!   `<prefix>Accessibility.apply<StableId 16진>(to:)` 함수입니다([`accessibility`] 표). 컨트롤 자체의 생성(XXII.3.1)과
//!   layout(XXII.3.2)은 이 module 범위 밖이며, 생성한 modifier·함수를 그 view에 붙입니다.
//!
//! [`export_apple_ui`]는 같은 소스에 view tree를 더합니다(XXII.3.1·XXII.3.2). SwiftUI는 `struct <prefix>View: View`,
//! AppKit은 `final class <prefix>View: NSView`이고, Control contract와 CSS layout longhand를 [`layout`] 규칙으로
//! 낮추며 node마다 위 접근성 modifier·함수를 붙입니다.
//!
//! 같은 IR과 옵션은 항상 같은 bytes를 만듭니다.

use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_target_profile::spec::{ApplePlatform, AppleProfile, ProfileSpec};

use crate::facet::Facet;
use crate::ir::CodegenIr;
use crate::text::identifier;

/// Swift 소스 도우미.
pub mod swift;

/// 색공간 → Apple API 표현.
pub mod color;

/// 접근성 의미 → SwiftUI·AppKit API.
pub mod accessibility;

/// Apple·Android 공용 native 구조·layout 분석.
pub mod native;

/// CSS layout longhand → Apple layout 규칙.
pub mod layout;

/// SwiftUI view tree.
mod swiftui;

/// AppKit view tree.
mod appkit;

/// Context → SwiftUI Environment 값 출력 형태 (XXII.3.4 일부).
pub mod context;

/// 저장 View 투영 → CodegenIR 입력(CLI·MCP용, Apple·Android 공용).
pub mod native_input;

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) mod ui_tests;

#[cfg(test)]
pub(crate) mod layout_tests;

pub use color::{ColorApi, ColorNote};
pub use layout::LayoutFinding;
pub use native::Control;

/// 출력 dialect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AppleDialect {
    /// SwiftUI.
    SwiftUi,
    /// AppKit(macOS 전용).
    AppKit,
}

/// 내보내기 옵션.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppleOptions {
    /// 출력 dialect.
    pub dialect: AppleDialect,
    /// 생성 타입 이름 접두어(ASCII 영문자로 시작하는 식별자).
    pub type_prefix: String,
}

impl AppleOptions {
    /// dialect와 기본 접두어 `Stellar`.
    pub fn new(dialect: AppleDialect) -> AppleOptions {
        AppleOptions {
            dialect,
            type_prefix: "Stellar".to_string(),
        }
    }
}

/// 노트가 가리키는 대상.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AppleNoteSubject {
    /// IR node.
    Node(StableId),
    /// token.
    Token(StableId),
}

/// 노트 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AppleNoteKind {
    /// 색 변환 노트.
    Color(ColorNote),
    /// 이 의미는 native control(예: `TextField`)이 제공합니다(XXII.3.1에서 연결).
    NativeControlRequired(Facet),
    /// 대상 API가 이 의미를 근사로만 표현합니다.
    Approximated(Facet),
    /// 대상 API에 대응 표현이 없어 쓰지 못했습니다.
    NoEquivalent(Facet),
    /// binding 대상이 IR에 없거나 값 타입을 Swift로 표현할 수 없어 쓰지 못했습니다.
    UnresolvedBinding(Facet),
    /// 컴포넌트 Property에 묶여 있어 쓰지 못했습니다(입력 주입은 XXII.3.1).
    ComponentProperty(Facet),
    /// profile 최소 OS에서 쓸 수 없는 API라 쓰지 않았습니다.
    Unavailable(Facet),
    /// Control·element 출력 노트(XXII.3.1).
    Control(ControlNote),
    /// layout 규칙 진단(XXII.3.2).
    Layout(LayoutFinding),
}

/// Control·element 출력 노트.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ControlNote {
    /// 이 profile에서 Unsupported인 contract라 contract 없는 container로 냈습니다.
    ContractUnsupported(Control),
    /// native control이 자식 view를 그리지 않아 자식을 내지 않았습니다.
    ChildrenIgnored(Control),
    /// Modal 표시 상태가 IR에 없어 내용을 그 자리에 냈습니다.
    ModalInPlace,
    /// 이미지 원본이 IR에 없어 빈 view로 냈습니다(SF Symbol 속성만 읽음).
    ImageSourceUnavailable,
    /// 도형 기하가 IR에 없어 빈 view로 냈습니다.
    ShapeGeometryUnavailable,
    /// 펼치지 않은 component instance라 빈 view로 냈습니다.
    InstanceUnexpanded,
}

/// 노트.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AppleNote {
    /// 대상.
    pub subject: AppleNoteSubject,
    /// 종류.
    pub kind: AppleNoteKind,
}

/// 내보내기 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppleExport {
    /// Swift 소스.
    pub source: String,
    /// 노트(대상 → 종류 순).
    pub notes: Vec<AppleNote>,
}

/// 내보내기 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppleError {
    /// IR이 Apple profile이 아닙니다.
    NotAppleProfile,
    /// 이 플랫폼에서 쓸 수 없는 dialect입니다(AppKit은 macOS 전용).
    DialectUnavailable {
        /// dialect.
        dialect: AppleDialect,
        /// 플랫폼.
        platform: ApplePlatform,
    },
    /// 타입 접두어가 식별자가 아닙니다.
    InvalidTypePrefix,
}

impl fmt::Display for AppleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AppleError::NotAppleProfile => f.write_str("Apple Target Profile IR이 아닙니다"),
            AppleError::DialectUnavailable { dialect, platform } => write!(
                f,
                "{dialect:?} dialect는 {} 플랫폼에서 쓸 수 없습니다",
                platform.name()
            ),
            AppleError::InvalidTypePrefix => {
                f.write_str("타입 접두어는 ASCII 영문자로 시작하는 식별자여야 합니다")
            }
        }
    }
}

impl std::error::Error for AppleError {}

/// IR의 Apple profile.
pub(crate) fn apple_profile(ir: &CodegenIr) -> Option<AppleProfile> {
    match ir.target.profile.spec {
        ProfileSpec::Apple(apple) => Some(apple),
        _ => None,
    }
}

struct Prepared {
    apple: AppleProfile,
    source: String,
    hooks: Vec<Option<accessibility::AccessibilityHook>>,
    notes: Vec<AppleNote>,
}

fn prepare(ir: &CodegenIr, options: &AppleOptions) -> Result<Prepared, AppleError> {
    let apple = apple_profile(ir).ok_or(AppleError::NotAppleProfile)?;
    if options.dialect == AppleDialect::AppKit && apple.platform != ApplePlatform::MacOs {
        return Err(AppleError::DialectUnavailable {
            dialect: options.dialect,
            platform: apple.platform,
        });
    }
    let prefix = &options.type_prefix;
    let valid_prefix =
        prefix.starts_with(|c: char| c.is_ascii_alphabetic()) && identifier(prefix, "") == *prefix;
    if !valid_prefix {
        return Err(AppleError::InvalidTypePrefix);
    }
    let mut notes = Vec::new();
    let mut source = String::new();
    source.push_str(match options.dialect {
        AppleDialect::SwiftUi => "import SwiftUI\n",
        AppleDialect::AppKit => "import AppKit\n",
    });
    let api = match options.dialect {
        AppleDialect::SwiftUi => ColorApi::SwiftUi,
        AppleDialect::AppKit => ColorApi::AppKit,
    };
    source.push('\n');
    source.push_str(&color::colors_enum(ir, api, prefix, &mut notes));
    let (code, hooks) =
        accessibility::accessibility_source(ir, options.dialect, apple, prefix, &mut notes);
    source.push_str(&code);
    Ok(Prepared {
        apple,
        source,
        hooks,
        notes,
    })
}

fn finish(source: String, mut notes: Vec<AppleNote>) -> AppleExport {
    notes.sort();
    notes.dedup();
    AppleExport { source, notes }
}

/// 접근성 의미와 색공간을 Apple API로 내보냅니다.
pub fn export_apple(ir: &CodegenIr, options: &AppleOptions) -> Result<AppleExport, AppleError> {
    let prepared = prepare(ir, options)?;
    Ok(finish(prepared.source, prepared.notes))
}

/// 색·접근성에 view tree(Control contract·layout·상태)를 더한 Swift 소스를 내보냅니다(XXII.3.1·XXII.3.2).
pub fn export_apple_ui(ir: &CodegenIr, options: &AppleOptions) -> Result<AppleExport, AppleError> {
    let Prepared {
        apple,
        mut source,
        hooks,
        mut notes,
    } = prepare(ir, options)?;
    let analysis = native::analyze(ir);
    let states = accessibility::state_names(ir);
    match options.dialect {
        AppleDialect::SwiftUi => {
            let mut generator = swiftui::Generator {
                ir,
                analysis: &analysis,
                states: &states,
                hooks: &hooks,
                prefix: &options.type_prefix,
                apple,
                notes: &mut notes,
            };
            source.push_str(&generator.source());
        }
        AppleDialect::AppKit => {
            let mut generator = appkit::Generator {
                ir,
                analysis: &analysis,
                states: &states,
                hooks: &hooks,
                prefix: &options.type_prefix,
                notes: &mut notes,
            };
            source.push_str(&generator.source());
        }
    }
    Ok(finish(source, notes))
}
