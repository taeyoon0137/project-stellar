//! 내보내기 사전 진단과 사용자 확인 (XX.1.3).
//!
//! 내보내기는 고정한 입력([`ExportCapture`], XX.1.2)을 쓰기 전에 [`ExportPreflight::run`]으로 진단하고, 확인이
//! 필요한 진단이 있으면 사용자 확인([`ExportConfirmation`]) 없이는 진행 허가([`ConfirmedExport`])를 내주지 않습니다.
//! writer(SVG)는 [`ConfirmedExport`]만 받으므로 진단과 확인을 건너뛸 수 없습니다.
//!
//! ## 진단 범위
//!
//! 내보내기에 **그려지는** node, 즉 capture 범위(View 전체 또는 selection subtree) 안의 보이는 scene node만 봅니다.
//! 숨긴 node, selection 밖 node, scene node를 만들지 않는 document node(Fragment 등)의 진단은 결과에 영향이 없어
//! 제외합니다. resource는 capture가 고정한 revision(font·asset)으로만 확인합니다.
//!
//! | 분류 | 진단 | 처리 |
//! |---|---|---|
//! | target capability(IX.10.8) | Unsupported·NotRegistered(Error 심각도) | 확인 필요 |
//! | | pin한 registry release를 몰라 검사하지 못한 profile | 확인 필요 |
//! | | Approximated·Deprecated | 참고([`ExportPreflight::notes`]), 확인 불필요 |
//! | missing resource | 고정 asset revision에 없는 이미지 | 확인 필요. 확인하면 그 이미지 pixel을 생략 |
//! | | 고정 font revision에 없는 text layout | 확인 필요. 확인하면 그 Text glyph를 생략 |
//! | | 고정 font revision에 없는 폰트 | 확인 필요. 확인하면 그 폰트 glyph를 생략 |
//! | | 명시 fallback chain으로 대체한 폰트(XIII.6.7) | 확인 필요(줄바꿈 변경 판정 포함) |
//! | 출력 형식 | SVG로 아직 기록할 수 없는 기능(mapping table, XX.3.1). 래스터 기능은 host가 offscreen 렌더
//! ([`PreflightInput::rasterizer`], XX.4.2)를 주지 않을 때만 | 확인해도 진행 불가([`FormatBlocker`]) |
//! | | SVG outline으로 쓸 수 없는 glyph(outline 없는 color glyph, 손상 glyph) | 확인해도 진행 불가([`GlyphBlocker`]) |
//!
//! 진단 순서는 위 표 순서이고, 같은 분류 안에서는 capability는 입력 순서(IX.10.8 결정론적 순서), resource는 key 순서,
//! 폰트 대체는 Text id 순서입니다. 그래서 같은 입력이면 같은 진단 목록이 나옵니다.
//!
//! ## 확인
//!
//! - [`ExportPreflight::confirm`]은 사용자가 진단 목록을 보고 진행을 승인했을 때 shell이 부릅니다. 확인 값은 그 진단
//!   목록과 capture 정체(scene semantic hash, resource pin, 요청)를 기록합니다.
//! - [`ExportPreflight::proceed`]는 확인이 필요한데 확인이 없으면 [`PreflightRejection::ConfirmationRequired`], 다른
//!   preflight(다른 진단·다른 capture)의 확인이면 [`PreflightRejection::ConfirmationMismatch`], 형식 차단이 있으면 확인과
//!   무관하게 [`PreflightRejection::Blocked`]입니다.
//! - 확인한 missing resource는 [`ConfirmedExport`]가 기억하고, writer는 그 resource만 생략합니다. 진단 때 있던 resource가
//!   쓰는 중에 없으면(host 계약 위반) 생략하지 않고 오류입니다.
//!
//! export crate는 crate 의존 정책(ADR-0019)상 target-profile·project crate에 의존하지 않으므로 IX.10.8 capability
//! 진단과 XIII.6.7 폰트 대체 결과는 같은 의미의 값([`TargetCapabilityFinding`], [`FontFallbackUse`])으로 host가 옮겨
//! 넘깁니다.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_core_types::property::PropertyPath;
use stellar_core_types::reference::AssetRef;
use stellar_scene::node::{SceneNode, SceneResourceRef, TextLayoutKey, Visibility};
use stellar_scene::snapshot::SceneSemanticHash;

use crate::capture::{ExportAssetSource, ExportCapture, ResourcePins};
use crate::request::{ExportFormat, ExportRequest, TargetProfileKey};
use crate::svg::mapping::{node_features, SvgFeature, SvgSupport};
use crate::svg::raster::{plan_raster, raster_scope};
use crate::svg::rasterize::ExportRasterizer;
use crate::text::{
    ExportFontFace, ExportTextSource, GlyphOutline, GlyphOutlineError, GlyphOutlineKey,
};

/// capability 진단 code(IX.10.8 `CapabilityDiagnosticCode`와 같은 번호).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum CapabilityCode {
    /// Native.
    Native = 1,
    /// Approximated.
    Approximated = 2,
    /// Unsupported.
    Unsupported = 3,
    /// Deprecated.
    Deprecated = 4,
    /// registry에 없는 subject.
    NotRegistered = 5,
}

impl CapabilityCode {
    /// 확인이 필요한 code(IX.10.8 Error 심각도)인지.
    pub const fn requires_confirmation(self) -> bool {
        matches!(
            self,
            CapabilityCode::Unsupported | CapabilityCode::NotRegistered
        )
    }

    /// 참고로만 알리는 code(Info·Warning 심각도)인지.
    pub const fn is_note(self) -> bool {
        matches!(
            self,
            CapabilityCode::Approximated | CapabilityCode::Deprecated
        )
    }
}

/// capability 대상 분류(IX.10.8 `CapabilitySubject::class_code`와 같은 번호).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum CapabilityClass {
    /// element 종류.
    Element = 1,
    /// 속성.
    Property = 2,
    /// semantic contract.
    Contract = 3,
}

/// capability 대상(분류 + 분류 안 번호, IX.10.8 `CapabilitySubject::raw`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CapabilitySubjectRef {
    /// 분류.
    pub class: CapabilityClass,
    /// 분류 안 번호.
    pub raw: u32,
}

/// capability 진단 하나(IX.10.8 `CapabilityFinding`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TargetCapabilityFinding {
    /// node.
    pub node: StableId,
    /// 대상.
    pub subject: CapabilitySubjectRef,
    /// 속성이면 그 path.
    pub path: Option<PropertyPath>,
    /// 진단 code.
    pub code: CapabilityCode,
}

/// 요청 target profile 하나의 capability 검사 결과(IX.10.8 `ProfileReport`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetCapabilityReport {
    /// 검사함.
    Validated {
        /// profile.
        profile: TargetProfileKey,
        /// 진단(IX.10.8 결정론적 순서).
        findings: Vec<TargetCapabilityFinding>,
    },
    /// pin한 registry release를 앱이 몰라 검사하지 않음.
    RegistryUnavailable {
        /// profile.
        profile: TargetProfileKey,
    },
}

impl TargetCapabilityReport {
    /// 검사한 profile.
    pub const fn profile(&self) -> &TargetProfileKey {
        match self {
            TargetCapabilityReport::Validated { profile, .. }
            | TargetCapabilityReport::RegistryUnavailable { profile } => profile,
        }
    }
}

/// 폰트 대체 결과(XIII.6.7 `TextFaceResolution`의 대체 경우).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FontSubstitution {
    /// primary가 없어 chain의 다음 등록 face를 씀.
    Fallback {
        /// chain 안 위치(1 이상).
        chain_index: u32,
    },
    /// chain 안에 등록 face가 없음(missing glyph 경로).
    NoFace,
}

/// 줄바꿈 변경 판정(XIII.6.7 `LineBreakChange`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LineBreakImpact {
    /// 비교 입력이 없어 변경 가능성만 알림.
    MayChange,
    /// 의도 face와 대체 결과의 줄 경계가 다름.
    Changed {
        /// 의도 face 줄 수.
        before_lines: u64,
        /// 대체 결과 줄 수.
        after_lines: u64,
    },
}

/// Text 하나의 폰트 대체(XIII.6.7 해석 결과와 줄바꿈 진단).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FontFallbackUse {
    /// Text node.
    pub text: StableId,
    /// 요청한 primary face.
    pub requested: ExportFontFace,
    /// 대체 결과.
    pub substitution: FontSubstitution,
    /// 줄바꿈 판정. `None`이면 줄 경계가 같음을 확인했습니다.
    pub line_break: Option<LineBreakImpact>,
}

/// preflight 입력(host 제공).
pub struct PreflightInput<'a> {
    /// 요청 target profile의 capability 검사 결과.
    pub capabilities: &'a TargetCapabilityReport,
    /// 폰트 대체가 일어난 Text.
    pub font_fallbacks: &'a [FontFallbackUse],
    /// asset 내용(고정 asset revision으로 조회).
    pub assets: &'a dyn ExportAssetSource,
    /// text glyph(고정 font revision으로 조회).
    pub text: &'a dyn ExportTextSource,
    /// 래스터 경계 offscreen 렌더(XX.4.2). 렌더를 제공하지 않으면 래스터 기능이 형식 차단입니다.
    pub rasterizer: &'a dyn ExportRasterizer,
}

/// 확인이 필요한 진단.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PreflightIssue {
    /// 요청 target profile을 검사하지 못했습니다(registry release 미지).
    ProfileNotValidated {
        /// profile.
        profile: TargetProfileKey,
    },
    /// target profile이 지원하지 않는 capability.
    UnsupportedCapability(TargetCapabilityFinding),
    /// 고정 asset revision에 없는 이미지.
    MissingImage {
        /// asset.
        asset: AssetRef,
        /// 쓰는 node(정렬).
        nodes: Vec<StableId>,
    },
    /// 고정 font revision에 없는 text layout.
    MissingTextLayout {
        /// layout key.
        layout: TextLayoutKey,
        /// 쓰는 Text node(정렬).
        nodes: Vec<StableId>,
    },
    /// 고정 font revision에 없는 폰트.
    MissingFont {
        /// 폰트 face.
        font: ExportFontFace,
        /// 쓰는 Text node(정렬).
        nodes: Vec<StableId>,
    },
    /// 명시 fallback chain으로 대체한 폰트.
    FontFallback(FontFallbackUse),
}

impl fmt::Display for PreflightIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PreflightIssue::ProfileNotValidated { profile } => write!(
                f,
                "target profile {profile}의 capability를 검사하지 못했습니다"
            ),
            PreflightIssue::UnsupportedCapability(finding) => write!(
                f,
                "node {}: target profile이 지원하지 않는 기능({:?} {})",
                finding.node, finding.subject.class, finding.subject.raw
            ),
            PreflightIssue::MissingImage { asset, nodes } => write!(
                f,
                "이미지 {}가 없습니다(node {}개)",
                asset.id(),
                nodes.len()
            ),
            PreflightIssue::MissingTextLayout { nodes, .. } => {
                write!(f, "text layout이 없습니다(Text {}개)", nodes.len())
            }
            PreflightIssue::MissingFont { font, nodes } => {
                write!(f, "폰트 {font}가 없습니다(Text {}개)", nodes.len())
            }
            PreflightIssue::FontFallback(fallback) => {
                write!(f, "Text {}의 폰트를 대체했습니다", fallback.text)?;
                match fallback.line_break {
                    Some(LineBreakImpact::Changed {
                        before_lines,
                        after_lines,
                    }) => write!(f, "(줄 수 {before_lines} → {after_lines})"),
                    Some(LineBreakImpact::MayChange) => f.write_str("(줄바꿈이 바뀔 수 있음)"),
                    None => Ok(()),
                }
            }
        }
    }
}

/// 확인해도 진행할 수 없는 진단: 요청 형식으로 아직 기록할 수 없는 기능.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FormatBlocker {
    /// node.
    pub node: StableId,
    /// 첫 미지원 기능.
    pub feature: SvgFeature,
    /// 지원 상태.
    pub support: SvgSupport,
}

/// SVG outline으로 쓸 수 없는 glyph 이유.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GlyphBlockReason {
    /// outline이 없고 bitmap·color 표현만 있습니다(emoji bitmap 등).
    NoOutline,
    /// 폰트 데이터가 손상되어 outline을 읽지 못했습니다.
    Malformed,
}

/// 확인해도 진행할 수 없는 진단: SVG outline으로 쓸 수 없는 glyph(XX.3.2). 폰트 없음([`PreflightIssue::MissingFont`])과
/// 달리 생략하면 글자가 조용히 빠지므로 확인으로 넘기지 않습니다.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GlyphBlocker {
    /// glyph를 쓰는 text layout.
    pub layout: TextLayoutKey,
    /// 그 layout을 쓰는 Text node(정렬).
    pub nodes: Vec<StableId>,
    /// 폰트 face.
    pub font: ExportFontFace,
    /// 폰트 안 glyph 번호.
    pub glyph: u32,
    /// 이유.
    pub reason: GlyphBlockReason,
}

/// host 입력 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreflightError {
    /// capability 검사 결과가 요청과 다른 target profile의 것입니다.
    ProfileMismatch {
        /// 요청 profile.
        requested: TargetProfileKey,
        /// 검사 결과 profile.
        report: TargetProfileKey,
    },
}

impl fmt::Display for PreflightError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PreflightError::ProfileMismatch { requested, report } => write!(
                f,
                "capability 검사 결과의 target profile {report}가 요청 {requested}와 다릅니다"
            ),
        }
    }
}

impl std::error::Error for PreflightError {}

/// 진행 거부.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreflightRejection {
    /// 요청 형식으로 기록할 수 없는 기능이 있습니다(확인과 무관).
    Blocked(Vec<FormatBlocker>),
    /// SVG outline으로 쓸 수 없는 glyph가 있습니다(확인과 무관).
    GlyphsBlocked(Vec<GlyphBlocker>),
    /// 확인이 필요한데 확인이 없습니다.
    ConfirmationRequired(Vec<PreflightIssue>),
    /// 다른 preflight의 확인입니다.
    ConfirmationMismatch,
}

impl fmt::Display for PreflightRejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PreflightRejection::Blocked(blockers) => write!(
                f,
                "요청 형식으로 기록할 수 없는 기능이 있어 내보낼 수 없습니다(node {}개)",
                blockers.len()
            ),
            PreflightRejection::GlyphsBlocked(glyphs) => write!(
                f,
                "SVG outline으로 쓸 수 없는 glyph가 있어 내보낼 수 없습니다(glyph {}개)",
                glyphs.len()
            ),
            PreflightRejection::ConfirmationRequired(issues) => {
                write!(f, "내보내기 전에 진단 {}건을 확인해야 합니다", issues.len())
            }
            PreflightRejection::ConfirmationMismatch => {
                f.write_str("확인한 진단이 지금 내보내기의 진단과 다릅니다")
            }
        }
    }
}

impl std::error::Error for PreflightRejection {}

/// 사용자가 확인한 진단 목록과 capture 정체.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportConfirmation {
    scene: SceneSemanticHash,
    pins: ResourcePins,
    request: ExportRequest,
    issues: Vec<PreflightIssue>,
}

impl ExportConfirmation {
    /// 확인한 진단.
    pub fn issues(&self) -> &[PreflightIssue] {
        &self.issues
    }
}

/// 사전 진단 결과.
#[derive(Debug, Clone)]
pub struct ExportPreflight {
    capture: ExportCapture,
    issues: Vec<PreflightIssue>,
    notes: Vec<TargetCapabilityFinding>,
    blockers: Vec<FormatBlocker>,
    glyph_blockers: Vec<GlyphBlocker>,
}

/// capture 범위의 보이는 scene node와 resource 사용처.
#[derive(Default)]
struct Scope {
    nodes: BTreeSet<StableId>,
    images: BTreeMap<AssetRef, BTreeSet<StableId>>,
    layouts: BTreeMap<TextLayoutKey, BTreeSet<StableId>>,
    /// SVG에서 glyph outline으로 쓸(래스터 경계 밖 node가 쓰는) layout.
    outlined_layouts: BTreeSet<TextLayoutKey>,
    blockers: Vec<FormatBlocker>,
}

fn scan(capture: &ExportCapture, rasterizer: &dyn ExportRasterizer) -> Scope {
    let check_svg = capture.request().format() == ExportFormat::Svg;
    // SVG에서 래스터 경계 안 node 내용은 rasterizer가 그리므로 glyph outline이 필요 없습니다(XX.4.2).
    let plan = (check_svg && rasterizer.is_available()).then(|| plan_raster(capture));
    let mut scope = Scope::default();
    for root in capture.roots() {
        let mut stack: Vec<&SceneNode> = vec![root.node()];
        while let Some(node) = stack.pop() {
            if node.hit().visibility == Visibility::Hidden {
                continue;
            }
            let source = node.source();
            scope.nodes.insert(source);
            match node.content().resource() {
                Some(SceneResourceRef::Image(asset)) => {
                    scope.images.entry(asset).or_default().insert(source);
                }
                Some(SceneResourceRef::TextLayout(layout)) => {
                    scope.layouts.entry(layout).or_default().insert(source);
                    if check_svg && !plan.as_ref().is_some_and(|plan| plan.rasterizes(node.z())) {
                        scope.outlined_layouts.insert(layout);
                    }
                }
                None => {}
            }
            if check_svg {
                // rasterizer가 있으면 래스터 기능은 planner가 고른 경계가 image로 담습니다(XX.4.1, XX.4.2).
                let blocked = node_features(node).into_iter().find(|feature| {
                    let rasterized = plan.is_some() && raster_scope(*feature).is_some();
                    !(feature.support().is_vector() || rasterized)
                });
                if let Some(feature) = blocked {
                    scope.blockers.push(FormatBlocker {
                        node: source,
                        feature,
                        support: feature.support(),
                    });
                }
            }
            // 자식을 역순으로 쌓아 paint 순서(pre-order)로 꺼냅니다.
            let start = stack.len();
            stack.extend(node.children().map(|child| &**child));
            stack[start..].reverse();
        }
    }
    scope
}

impl ExportPreflight {
    /// capture를 진단합니다. capability 검사 결과는 요청 target profile의 것이어야 합니다.
    pub fn run(
        capture: ExportCapture,
        input: &PreflightInput<'_>,
    ) -> Result<ExportPreflight, PreflightError> {
        let requested = capture.request().target_profile();
        if input.capabilities.profile() != requested {
            return Err(PreflightError::ProfileMismatch {
                requested: requested.clone(),
                report: input.capabilities.profile().clone(),
            });
        }
        let scope = scan(&capture, input.rasterizer);
        let mut issues = Vec::new();
        let mut notes = Vec::new();
        match input.capabilities {
            TargetCapabilityReport::RegistryUnavailable { profile } => {
                issues.push(PreflightIssue::ProfileNotValidated {
                    profile: profile.clone(),
                });
            }
            TargetCapabilityReport::Validated { findings, .. } => {
                for finding in findings.iter().filter(|f| scope.nodes.contains(&f.node)) {
                    if finding.code.requires_confirmation() {
                        issues.push(PreflightIssue::UnsupportedCapability(finding.clone()));
                    } else if finding.code.is_note() {
                        notes.push(finding.clone());
                    }
                }
            }
        }
        for (asset, nodes) in &scope.images {
            if capture.encoded_image(input.assets, *asset).is_none() {
                issues.push(PreflightIssue::MissingImage {
                    asset: *asset,
                    nodes: nodes.iter().copied().collect(),
                });
            }
        }
        let mut fonts: BTreeMap<ExportFontFace, (bool, BTreeSet<StableId>)> = BTreeMap::new();
        // SVG는 모든 glyph를 outline으로 쓰므로(XX.3.2) 그릴 glyph key를 모두 조회합니다. PNG는 renderer가 bitmap glyph도
        // 그리므로 폰트 유무만 봅니다(face마다 glyph 하나).
        let svg = capture.request().format() == ExportFormat::Svg;
        let mut checked: HashSet<GlyphOutlineKey> = HashSet::new();
        let mut glyph_blockers = Vec::new();
        for (layout, nodes) in &scope.layouts {
            let Some(glyphs) = capture.glyph_layout(input.text, *layout) else {
                issues.push(PreflightIssue::MissingTextLayout {
                    layout: *layout,
                    nodes: nodes.iter().copied().collect(),
                });
                continue;
            };
            for run in &glyphs.runs {
                let Some(first) = run.glyphs.first() else {
                    continue;
                };
                let entry = match fonts.entry(run.font) {
                    std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
                    std::collections::btree_map::Entry::Vacant(entry) => {
                        let missing = matches!(
                            capture.glyph_outline(input.text, &run.key(first.glyph)),
                            Err(GlyphOutlineError::FontUnavailable)
                        );
                        entry.insert((missing, BTreeSet::new()))
                    }
                };
                entry.1.extend(nodes.iter().copied());
                if !(svg && scope.outlined_layouts.contains(layout)) || entry.0 {
                    // 폰트가 없으면 glyph 진단 대신 MissingFont 하나로 알립니다.
                    continue;
                }
                for placed in &run.glyphs {
                    let key = run.key(placed.glyph);
                    if !checked.insert(key.clone()) {
                        continue;
                    }
                    let reason = match capture.glyph_outline(input.text, &key) {
                        Ok(GlyphOutline::NoOutline) => GlyphBlockReason::NoOutline,
                        Err(GlyphOutlineError::Malformed) => GlyphBlockReason::Malformed,
                        Ok(GlyphOutline::Empty | GlyphOutline::Path(_))
                        | Err(GlyphOutlineError::FontUnavailable) => continue,
                    };
                    glyph_blockers.push(GlyphBlocker {
                        layout: *layout,
                        nodes: nodes.iter().copied().collect(),
                        font: run.font,
                        glyph: placed.glyph,
                        reason,
                    });
                }
            }
        }
        for (font, (missing, nodes)) in fonts {
            if missing {
                issues.push(PreflightIssue::MissingFont {
                    font,
                    nodes: nodes.into_iter().collect(),
                });
            }
        }
        let mut fallbacks: Vec<FontFallbackUse> = input
            .font_fallbacks
            .iter()
            .filter(|f| scope.nodes.contains(&f.text))
            .copied()
            .collect();
        fallbacks.sort();
        fallbacks.dedup();
        issues.extend(fallbacks.into_iter().map(PreflightIssue::FontFallback));
        Ok(ExportPreflight {
            capture,
            issues,
            notes,
            blockers: scope.blockers,
            glyph_blockers,
        })
    }

    /// 진단한 capture.
    pub const fn capture(&self) -> &ExportCapture {
        &self.capture
    }

    /// 확인이 필요한 진단(모듈 문서 순서).
    pub fn issues(&self) -> &[PreflightIssue] {
        &self.issues
    }

    /// 참고 진단(Approximated·Deprecated capability).
    pub fn notes(&self) -> &[TargetCapabilityFinding] {
        &self.notes
    }

    /// 확인해도 진행할 수 없는 진단(paint 순서).
    pub fn blockers(&self) -> &[FormatBlocker] {
        &self.blockers
    }

    /// 확인해도 진행할 수 없는 glyph 진단(layout key 순서, 그 안 run·glyph 순서, 같은 key는 처음 한 번).
    pub fn glyph_blockers(&self) -> &[GlyphBlocker] {
        &self.glyph_blockers
    }

    /// 진행에 사용자 확인이 필요한지.
    pub fn requires_confirmation(&self) -> bool {
        !self.issues.is_empty()
    }

    /// 사용자가 [`ExportPreflight::issues`]를 보고 진행을 승인했음을 기록합니다. shell이 확인 UI에서 승인을 받은 뒤에만
    /// 부릅니다.
    pub fn confirm(&self) -> ExportConfirmation {
        ExportConfirmation {
            scene: self.capture.content_hash(),
            pins: self.capture.pins(),
            request: self.capture.request().clone(),
            issues: self.issues.clone(),
        }
    }

    fn matches(&self, confirmation: &ExportConfirmation) -> bool {
        confirmation.scene == self.capture.content_hash()
            && confirmation.pins == self.capture.pins()
            && &confirmation.request == self.capture.request()
            && confirmation.issues == self.issues
    }

    /// 진행 허가를 받습니다(모듈 문서 "확인").
    pub fn proceed(
        &self,
        confirmation: Option<&ExportConfirmation>,
    ) -> Result<ConfirmedExport, PreflightRejection> {
        if !self.blockers.is_empty() {
            return Err(PreflightRejection::Blocked(self.blockers.clone()));
        }
        if !self.glyph_blockers.is_empty() {
            return Err(PreflightRejection::GlyphsBlocked(
                self.glyph_blockers.clone(),
            ));
        }
        match confirmation {
            Some(confirmation) if !self.matches(confirmation) => {
                return Err(PreflightRejection::ConfirmationMismatch);
            }
            None if self.requires_confirmation() => {
                return Err(PreflightRejection::ConfirmationRequired(
                    self.issues.clone(),
                ));
            }
            _ => {}
        }
        Ok(ConfirmedExport::new(
            self.capture.clone(),
            self.issues.clone(),
        ))
    }
}

/// 진행 허가: 진단을 통과했거나 사용자가 진단을 확인한 capture.
#[derive(Debug, Clone)]
pub struct ConfirmedExport {
    capture: ExportCapture,
    accepted: Vec<PreflightIssue>,
    omitted_images: BTreeSet<AssetRef>,
    omitted_layouts: BTreeSet<TextLayoutKey>,
    omitted_fonts: BTreeSet<ExportFontFace>,
}

impl ConfirmedExport {
    fn new(capture: ExportCapture, accepted: Vec<PreflightIssue>) -> ConfirmedExport {
        let mut export = ConfirmedExport {
            capture,
            accepted: Vec::new(),
            omitted_images: BTreeSet::new(),
            omitted_layouts: BTreeSet::new(),
            omitted_fonts: BTreeSet::new(),
        };
        for issue in &accepted {
            match issue {
                PreflightIssue::MissingImage { asset, .. } => {
                    export.omitted_images.insert(*asset);
                }
                PreflightIssue::MissingTextLayout { layout, .. } => {
                    export.omitted_layouts.insert(*layout);
                }
                PreflightIssue::MissingFont { font, .. } => {
                    export.omitted_fonts.insert(*font);
                }
                PreflightIssue::ProfileNotValidated { .. }
                | PreflightIssue::UnsupportedCapability(_)
                | PreflightIssue::FontFallback(_) => {}
            }
        }
        export.accepted = accepted;
        export
    }

    /// 진단 없이 진행 허가를 만듭니다(crate 안 단위 테스트 전용: writer 동작만 검사할 때).
    #[cfg(test)]
    pub(crate) fn assume_confirmed(capture: ExportCapture) -> ConfirmedExport {
        ConfirmedExport::new(capture, Vec::new())
    }

    /// 내보낼 capture.
    pub const fn capture(&self) -> &ExportCapture {
        &self.capture
    }

    /// 사용자가 확인한 진단(없으면 빈 목록).
    pub fn accepted(&self) -> &[PreflightIssue] {
        &self.accepted
    }

    /// 확인한 missing image라 생략하는지.
    pub fn omits_image(&self, asset: AssetRef) -> bool {
        self.omitted_images.contains(&asset)
    }

    /// 확인한 missing text layout이라 생략하는지.
    pub fn omits_text_layout(&self, layout: TextLayoutKey) -> bool {
        self.omitted_layouts.contains(&layout)
    }

    /// 확인한 missing font라 그 glyph를 생략하는지.
    pub fn omits_font(&self, font: ExportFontFace) -> bool {
        self.omitted_fonts.contains(&font)
    }
}

#[cfg(test)]
mod tests;
