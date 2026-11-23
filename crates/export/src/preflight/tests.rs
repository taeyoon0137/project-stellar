//! 내보내기 사전 진단·확인 테스트 (XX.1.3).

use std::collections::HashMap;
use std::sync::Arc;

use stellar_core_types::property::{PropertyKey, PropertyPath};
use stellar_core_types::revision::DocumentRevision;
use stellar_core_types::shadow::{BlurRadius, Shadow};
use stellar_core_types::value::{Size2, Vec2};
use stellar_scene::builder::{
    FrameClipInput, FrameInput, ImageInput, NodeInput, SceneBuilder, ShapeInput, TextInput,
};
use stellar_scene::geometry::{PathCommand, PathGeometry};
use stellar_scene::handle::SnapshotHandle;
use stellar_scene::node::{CornerRadii, ImageCrop, ImageFit, ImageSampling};
use stellar_scene::revision::SceneRevisionMap;
use stellar_scene::stack::{Paint, StrokeCap};
use stellar_scene::SceneSnapshot;

use super::*;
use crate::capture::{
    AssetRevision, EncodedImage, FontRevision, ImageMedia, ResourcePins, SceneSource,
};
use crate::request::ExportSource;
use crate::svg::{write_svg, SvgExportError, SvgWriteOptions};
use crate::test_fixtures as fx;
use crate::text::{
    GlyphOutline, GlyphOutlineKey, GlyphRun, GlyphVariations, PlacedGlyph, RunSize, TextGlyphLayout,
};

const PINS: ResourcePins = ResourcePins {
    fonts: FontRevision(4),
    assets: AssetRevision(3),
};

const FONT_OK: ExportFontFace = ExportFontFace::new([0x0A; 32], 0);
const FONT_MISSING: ExportFontFace = ExportFontFace::new([0x0B; 32], 0);

const KEY_OK: TextLayoutKey = TextLayoutKey::from_bytes([1; 16]);
const KEY_GONE: TextLayoutKey = TextLayoutKey::from_bytes([2; 16]);
const KEY_HIDDEN: TextLayoutKey = TextLayoutKey::from_bytes([3; 16]);
/// 첫 glyph는 정상이고 뒤 glyph가 outline 없는 color glyph인 layout.
const KEY_COLOR: TextLayoutKey = TextLayoutKey::from_bytes([4; 16]);
/// 첫 glyph는 정상이고 뒤 glyph가 손상 glyph인 layout.
const KEY_BROKEN: TextLayoutKey = TextLayoutKey::from_bytes([5; 16]);

/// 정상 glyph(크기 비례 사각형).
const GLYPH_OK: u32 = 1;
/// outline 없는 color glyph.
const GLYPH_COLOR: u32 = 2;
/// 기본 손상 glyph.
const GLYPH_BROKEN: u32 = 3;

/// 고정 revision별 asset.
#[derive(Default)]
struct Assets(HashMap<(AssetRef, AssetRevision), EncodedImage>);

impl ExportAssetSource for Assets {
    fn encoded_image(
        &self,
        asset: AssetRef,
        revision: AssetRevision,
    ) -> Option<crate::capture::EncodedImage> {
        self.0.get(&(asset, revision)).cloned()
    }
}

/// asset 1은 고정 revision 3에, asset 77은 revision 2에만 있습니다.
fn assets() -> Assets {
    let image =
        |bytes: &[u8]| EncodedImage::new(ImageMedia::Png, Arc::from(bytes.to_vec()), 4, 4).unwrap();
    let mut map = HashMap::new();
    map.insert((fx::asset(1), AssetRevision(3)), image(b"present"));
    map.insert((fx::asset(77), AssetRevision(2)), image(b"older"));
    Assets(map)
}

/// font revision별 text layout. KEY_GONE은 revision 5에만 있고, FONT_MISSING은 outline 조회가 실패합니다.
/// GLYPH_COLOR는 outline이 없고 `malformed` glyph는 손상으로 조회됩니다.
struct Text {
    layouts: HashMap<(TextLayoutKey, FontRevision), Arc<TextGlyphLayout>>,
    /// 손상(`Malformed`)으로 조회되는 glyph.
    malformed: u32,
}

fn square(size: f32) -> PathGeometry {
    let p = |x: f32, y: f32| Vec2::new(x, y).unwrap();
    PathGeometry::new(vec![
        PathCommand::MoveTo(p(0.0, -size)),
        PathCommand::LineTo(p(size, -size)),
        PathCommand::LineTo(p(size, 0.0)),
        PathCommand::LineTo(p(0.0, 0.0)),
        PathCommand::Close,
    ])
    .unwrap()
}

impl ExportTextSource for Text {
    fn glyph_layout(
        &self,
        layout: TextLayoutKey,
        fonts: FontRevision,
    ) -> Option<Arc<TextGlyphLayout>> {
        self.layouts.get(&(layout, fonts)).cloned()
    }

    fn glyph_outline(
        &self,
        key: &GlyphOutlineKey,
        _fonts: FontRevision,
    ) -> Result<GlyphOutline, GlyphOutlineError> {
        if key.font == FONT_MISSING {
            return Err(GlyphOutlineError::FontUnavailable);
        }
        if key.glyph == self.malformed {
            return Err(GlyphOutlineError::Malformed);
        }
        if key.glyph == GLYPH_COLOR {
            return Ok(GlyphOutline::NoOutline);
        }
        Ok(GlyphOutline::Path(square(key.size.px() / 2.0)))
    }
}

/// 10 px 검은 run. glyph는 (번호, pen x 1/64 px), baseline 10 px.
fn run_of(font: ExportFontFace, glyphs: &[(u32, i32)]) -> GlyphRun {
    GlyphRun {
        font,
        size: RunSize::from_px(10.0).unwrap(),
        variations: GlyphVariations::none(),
        fill: Some(fx::srgb(0.0, 0.0, 0.0, 1.0)),
        glyphs: glyphs
            .iter()
            .map(|&(glyph, x)| PlacedGlyph { glyph, x, y: 640 })
            .collect(),
    }
}

fn glyph_run(font: ExportFontFace, x: i32) -> GlyphRun {
    run_of(font, &[(GLYPH_OK, x)])
}

fn text() -> Text {
    let two_fonts = Arc::new(TextGlyphLayout {
        runs: vec![glyph_run(FONT_OK, 0), glyph_run(FONT_MISSING, 640)],
    });
    let mut layouts = HashMap::new();
    layouts.insert((KEY_OK, FontRevision(4)), two_fonts);
    layouts.insert(
        (KEY_GONE, FontRevision(5)),
        Arc::new(TextGlyphLayout {
            runs: vec![glyph_run(FONT_OK, 0)],
        }),
    );
    layouts.insert(
        (KEY_COLOR, FontRevision(4)),
        Arc::new(TextGlyphLayout {
            runs: vec![run_of(FONT_OK, &[(GLYPH_OK, 0), (GLYPH_COLOR, 640)])],
        }),
    );
    layouts.insert(
        (KEY_BROKEN, FontRevision(4)),
        Arc::new(TextGlyphLayout {
            runs: vec![run_of(
                FONT_OK,
                &[(GLYPH_OK, 0), (GLYPH_BROKEN, 640), (GLYPH_BROKEN, 1280)],
            )],
        }),
    );
    Text {
        layouts,
        malformed: GLYPH_BROKEN,
    }
}

/// 진단 fixture scene.
///
/// Frame 1 아래 Shape 2, Image 3(asset 77, 고정 revision에 없음), Image 4(asset 1), Text 5(KEY_OK), Text 6(KEY_GONE,
/// 고정 revision에 없음), 숨긴 Text 7(KEY_HIDDEN), 숨긴 Image 8(asset 78).
fn scene(extra: impl FnOnce(&mut NodeInput)) -> SceneSnapshot {
    let mut builder = SceneBuilder::new();
    let mut root = builder.root_scope();
    let frame_input = FrameInput {
        node: NodeInput::new(fx::node_id(1), fx::translate(0.0, 0.0)),
        size: Size2::new(200.0, 100.0).unwrap(),
        corner_radii: CornerRadii::ZERO,
        background: Some(fx::srgb(1.0, 1.0, 1.0, 1.0)),
        border: None,
        clip: FrameClipInput::None,
    };
    let mut frame = builder.begin_frame(&root, &frame_input).unwrap();
    let mut shape = NodeInput::new(fx::node_id(2), fx::translate(10.0, 10.0));
    shape
        .paint
        .fills
        .push(Paint::Solid(fx::srgb(1.0, 0.0, 0.0, 1.0)));
    extra(&mut shape);
    builder
        .build_shape(
            frame.scope_mut(),
            &ShapeInput {
                node: shape,
                layout_rect: fx::rect(0.0, 0.0, 40.0, 30.0),
                geometry: fx::triangle(),
                cap: StrokeCap::Butt,
            },
        )
        .unwrap();
    for (id, asset, x, visible) in [
        (3, 77, 60.0, true),
        (4, 1, 100.0, true),
        (8, 78, 140.0, false),
    ] {
        let mut node = NodeInput::new(fx::node_id(id), fx::translate(x, 10.0));
        node.visible = visible;
        builder
            .build_image(
                frame.scope_mut(),
                &ImageInput {
                    node,
                    asset: fx::asset(asset),
                    size: Size2::new(30.0, 20.0).unwrap(),
                    fit: ImageFit::Fill,
                    crop: ImageCrop::FULL,
                    sampling: ImageSampling::Linear,
                    color_space: None,
                },
            )
            .unwrap();
    }
    for (id, layout, x, visible) in [
        (5, KEY_OK, 10.0, true),
        (6, KEY_GONE, 100.0, true),
        (7, KEY_HIDDEN, 150.0, false),
    ] {
        let mut node = NodeInput::new(fx::node_id(id), fx::translate(x, 50.0));
        node.visible = visible;
        builder
            .build_text(
                frame.scope_mut(),
                &TextInput {
                    node,
                    layout,
                    size: Size2::new(40.0, 20.0).unwrap(),
                },
            )
            .unwrap();
    }
    builder.finish_frame(&mut root, frame).unwrap();
    SceneRevisionMap::new()
        .generate(DocumentRevision(1), builder.finish(root))
        .unwrap()
}

/// 최상위 Text만 있는 scene. `(id, layout, shadow 여부)` 순서가 paint 순서입니다.
fn text_scene(texts: &[(u64, TextLayoutKey, bool)]) -> SceneSnapshot {
    let mut builder = SceneBuilder::new();
    let mut root = builder.root_scope();
    for (index, &(id, layout, shadow)) in texts.iter().enumerate() {
        let mut node = NodeInput::new(
            fx::node_id(id),
            fx::translate(10.0 + 50.0 * index as f32, 10.0),
        );
        if shadow {
            node.paint.shadows.push(Shadow::new(
                fx::p(1.0, 1.0),
                BlurRadius::new(2.0).unwrap(),
                fx::srgb(0.0, 0.0, 0.0, 0.5),
            ));
        }
        builder
            .build_text(
                &mut root,
                &TextInput {
                    node,
                    layout,
                    size: Size2::new(40.0, 20.0).unwrap(),
                },
            )
            .unwrap();
    }
    SceneRevisionMap::new()
        .generate(DocumentRevision(1), builder.finish(root))
        .unwrap()
}

fn capture_of(snapshot: SceneSnapshot, request: ExportRequest) -> ExportCapture {
    ExportCapture::new(
        request,
        SceneSource {
            view: fx::view_id(),
            target_profile: fx::profile(),
            snapshot: SnapshotHandle::new(snapshot),
        },
        PINS,
    )
    .unwrap()
}

fn svg_capture(snapshot: SceneSnapshot) -> ExportCapture {
    capture_of(snapshot, fx::view_request(ExportFormat::Svg))
}

fn finding(
    node: u64,
    class: CapabilityClass,
    raw: u32,
    code: CapabilityCode,
) -> TargetCapabilityFinding {
    TargetCapabilityFinding {
        node: fx::node_id(node),
        subject: CapabilitySubjectRef { class, raw },
        path: (class == CapabilityClass::Property)
            .then(|| PropertyPath::with_keys(fx::node_id(node), vec![PropertyKey(raw)])),
        code,
    }
}

/// IX.10.8 순서의 capability 진단(범위 밖·숨김·Native 포함).
fn findings() -> Vec<TargetCapabilityFinding> {
    vec![
        finding(1, CapabilityClass::Element, 10, CapabilityCode::Native),
        finding(1, CapabilityClass::Element, 11, CapabilityCode::Unsupported),
        finding(
            2,
            CapabilityClass::Property,
            20,
            CapabilityCode::NotRegistered,
        ),
        finding(
            3,
            CapabilityClass::Property,
            21,
            CapabilityCode::Approximated,
        ),
        finding(4, CapabilityClass::Contract, 3, CapabilityCode::Deprecated),
        finding(7, CapabilityClass::Element, 12, CapabilityCode::Unsupported),
        finding(
            99,
            CapabilityClass::Element,
            13,
            CapabilityCode::Unsupported,
        ),
    ]
}

fn validated(findings: Vec<TargetCapabilityFinding>) -> TargetCapabilityReport {
    TargetCapabilityReport::Validated {
        profile: fx::profile(),
        findings,
    }
}

fn fallback(
    text: u64,
    substitution: FontSubstitution,
    line_break: Option<LineBreakImpact>,
) -> FontFallbackUse {
    FontFallbackUse {
        text: fx::node_id(text),
        requested: ExportFontFace::new([0xF0; 32], 0),
        substitution,
        line_break,
    }
}

fn fallbacks() -> Vec<FontFallbackUse> {
    vec![
        fallback(
            6,
            FontSubstitution::NoFace,
            Some(LineBreakImpact::Changed {
                before_lines: 2,
                after_lines: 3,
            }),
        ),
        fallback(
            5,
            FontSubstitution::Fallback { chain_index: 1 },
            Some(LineBreakImpact::MayChange),
        ),
        fallback(7, FontSubstitution::Fallback { chain_index: 2 }, None),
        fallback(
            5,
            FontSubstitution::Fallback { chain_index: 1 },
            Some(LineBreakImpact::MayChange),
        ),
    ]
}

fn run_preflight(
    capture: ExportCapture,
    capabilities: &TargetCapabilityReport,
    fallbacks: &[FontFallbackUse],
) -> ExportPreflight {
    ExportPreflight::run(
        capture,
        &PreflightInput {
            capabilities,
            font_fallbacks: fallbacks,
            assets: &assets(),
            text: &text(),
        },
    )
    .unwrap()
}

fn fixture_preflight() -> ExportPreflight {
    run_preflight(
        svg_capture(scene(|_| {})),
        &validated(findings()),
        &fallbacks(),
    )
}

#[test]
fn diagnostic_fixture_reports_expected_issues() {
    // 검증 기준(XX.1.3): diagnostic fixture.
    let preflight = fixture_preflight();
    let all = findings();
    let fb = fallbacks();
    assert_eq!(
        preflight.issues(),
        &[
            PreflightIssue::UnsupportedCapability(all[1].clone()),
            PreflightIssue::UnsupportedCapability(all[2].clone()),
            PreflightIssue::MissingImage {
                asset: fx::asset(77),
                nodes: vec![fx::node_id(3)]
            },
            PreflightIssue::MissingTextLayout {
                layout: KEY_GONE,
                nodes: vec![fx::node_id(6)]
            },
            PreflightIssue::MissingFont {
                font: FONT_MISSING,
                nodes: vec![fx::node_id(5)]
            },
            PreflightIssue::FontFallback(fb[1]),
            PreflightIssue::FontFallback(fb[0]),
        ]
    );
    // Approximated·Deprecated는 참고, Native·숨김·범위 밖은 제외합니다.
    assert_eq!(preflight.notes(), &[all[3].clone(), all[4].clone()]);
    assert!(preflight.blockers().is_empty());
    assert!(preflight.requires_confirmation());
    for issue in preflight.issues() {
        assert!(!issue.to_string().is_empty());
    }
    assert!(preflight.issues()[6].to_string().contains("2 → 3"));
    // 입력 순서가 달라도 같은 진단입니다(폰트 대체는 Text id 순서).
    let mut reversed = fallbacks();
    reversed.reverse();
    let again = run_preflight(
        svg_capture(scene(|_| {})),
        &validated(findings()),
        &reversed,
    );
    assert_eq!(again.issues(), preflight.issues());
}

#[test]
fn export_does_not_proceed_without_confirmation() {
    // 검증 기준(XX.1.3): user confirmation 없이는 진행하지 않음.
    let preflight = fixture_preflight();
    let rejected = preflight.proceed(None).unwrap_err();
    assert_eq!(
        rejected,
        PreflightRejection::ConfirmationRequired(preflight.issues().to_vec())
    );
    assert!(rejected.to_string().contains("7건"));
    let confirmation = preflight.confirm();
    assert_eq!(confirmation.issues(), preflight.issues());
    let confirmed = preflight.proceed(Some(&confirmation)).unwrap();
    assert_eq!(confirmed.accepted(), preflight.issues());
    assert!(confirmed.omits_image(fx::asset(77)));
    assert!(!confirmed.omits_image(fx::asset(1)));
    assert!(confirmed.omits_text_layout(KEY_GONE));
    assert!(confirmed.omits_font(FONT_MISSING));
    assert!(!confirmed.omits_font(FONT_OK));
}

#[test]
fn confirmation_of_another_preflight_is_rejected() {
    let preflight = fixture_preflight();
    // 진단 목록이 다른 preflight(폰트 대체 없음)의 확인.
    let fewer = run_preflight(svg_capture(scene(|_| {})), &validated(findings()), &[]);
    assert_eq!(
        preflight.proceed(Some(&fewer.confirm())).unwrap_err(),
        PreflightRejection::ConfirmationMismatch
    );
    // 진단은 같지만 scene 내용이 다른 capture(Shape 색 편집)의 확인.
    let edited = run_preflight(
        svg_capture(scene(|shape| {
            shape.paint.fills[0] = Paint::Solid(fx::srgb(0.0, 1.0, 0.0, 1.0));
        })),
        &validated(findings()),
        &fallbacks(),
    );
    assert_eq!(edited.issues(), preflight.issues());
    assert_eq!(
        preflight.proceed(Some(&edited.confirm())).unwrap_err(),
        PreflightRejection::ConfirmationMismatch
    );
    assert!(!PreflightRejection::ConfirmationMismatch
        .to_string()
        .is_empty());
    // 같은 내용으로 다시 진단한 preflight의 확인은 같은 승인입니다.
    let same = fixture_preflight();
    assert!(preflight.proceed(Some(&same.confirm())).is_ok());
}

#[test]
fn clean_or_note_only_exports_proceed_directly() {
    // 문제 없는 selection(Image 4)은 확인 없이 진행합니다. 참고 진단(Deprecated)만 있어도 같습니다.
    let selection = ExportSource::selection(fx::view_id(), [fx::node_id(4)]).unwrap();
    let capture = capture_of(scene(|_| {}), fx::request(selection, ExportFormat::Svg));
    let preflight = run_preflight(capture, &validated(findings()), &fallbacks());
    assert!(preflight.issues().is_empty(), "{:?}", preflight.issues());
    assert_eq!(preflight.notes(), &[findings()[4].clone()]);
    assert!(!preflight.requires_confirmation());
    let confirmed = preflight.proceed(None).unwrap();
    assert!(confirmed.accepted().is_empty());
}

#[test]
fn unvalidated_profile_requires_confirmation_and_profile_must_match() {
    let capture = svg_capture(scene(|_| {}));
    let unavailable = TargetCapabilityReport::RegistryUnavailable {
        profile: fx::profile(),
    };
    let preflight = run_preflight(capture.clone(), &unavailable, &[]);
    assert_eq!(
        preflight.issues()[0],
        PreflightIssue::ProfileNotValidated {
            profile: fx::profile()
        }
    );
    assert!(matches!(
        preflight.proceed(None),
        Err(PreflightRejection::ConfirmationRequired(_))
    ));
    let other = TargetCapabilityReport::Validated {
        profile: TargetProfileKey::new("ios-26").unwrap(),
        findings: Vec::new(),
    };
    let error = ExportPreflight::run(
        capture,
        &PreflightInput {
            capabilities: &other,
            font_fallbacks: &[],
            assets: &assets(),
            text: &text(),
        },
    )
    .unwrap_err();
    assert_eq!(
        error,
        PreflightError::ProfileMismatch {
            requested: fx::profile(),
            report: TargetProfileKey::new("ios-26").unwrap()
        }
    );
    assert!(error.to_string().contains("ios-26"));
}

#[test]
fn unsupported_output_features_block_even_with_confirmation() {
    let with_shadow = |shape: &mut NodeInput| {
        shape.paint.shadows.push(Shadow::new(
            fx::p(1.0, 1.0),
            BlurRadius::new(2.0).unwrap(),
            fx::srgb(0.0, 0.0, 0.0, 0.5),
        ));
    };
    let preflight = run_preflight(svg_capture(scene(with_shadow)), &validated(findings()), &[]);
    let blocker = FormatBlocker {
        node: fx::node_id(2),
        feature: SvgFeature::DropShadow,
        support: SvgFeature::DropShadow.support(),
    };
    assert_eq!(preflight.blockers(), &[blocker]);
    let rejected = preflight.proceed(Some(&preflight.confirm())).unwrap_err();
    assert_eq!(rejected, PreflightRejection::Blocked(vec![blocker]));
    assert!(!rejected.to_string().is_empty());
    // PNG 요청은 SVG 기록 한계와 무관합니다.
    let png = run_preflight(
        capture_of(scene(with_shadow), fx::view_request(ExportFormat::Png)),
        &validated(findings()),
        &[],
    );
    assert!(png.blockers().is_empty());
}

#[test]
fn confirmed_missing_resources_are_omitted_and_others_are_kept() {
    let preflight = fixture_preflight();
    let confirmed = preflight.proceed(Some(&preflight.confirm())).unwrap();
    let svg = write_svg(&confirmed, &assets(), &text(), &SvgWriteOptions::default())
        .unwrap()
        .into_string();
    // 확인한 missing image(node 3)는 pixel 없이, 있는 이미지(node 4)는 그대로 씁니다.
    assert_eq!(svg.matches("<image ").count(), 1);
    assert!(svg.contains(&format!("<g id=\"n-{}\">", fx::node_id(3))));
    // Text 5는 있는 폰트 glyph 하나만, Text 6(missing layout)은 glyph 없이 씁니다.
    let text_paths = svg
        .lines()
        .filter(|line| line.contains("fill=\"rgb(0%,0%,0%)\""))
        .collect::<Vec<_>>();
    assert_eq!(text_paths.len(), 1, "{text_paths:?}");
    assert!(text_paths[0].contains("M0 5 L5 5 L5 10 L0 10 Z"));
    assert!(!text_paths[0].contains("M10 5"), "missing font glyph 생략");
    assert!(svg.contains(&format!("<g id=\"n-{}\">", fx::node_id(6))));
    assert!(!svg.contains("<text"));
}

#[test]
fn resources_lost_after_the_preflight_are_errors() {
    // 진단 때 있던 resource가 쓰는 중에 없으면 생략하지 않고 오류입니다.
    let selection =
        ExportSource::selection(fx::view_id(), [fx::node_id(4), fx::node_id(5)]).unwrap();
    let capture = capture_of(scene(|_| {}), fx::request(selection, ExportFormat::Svg));
    let preflight = run_preflight(capture, &validated(Vec::new()), &[]);
    // Text 5의 FONT_MISSING만 확인 대상입니다.
    assert_eq!(preflight.issues().len(), 1);
    let confirmed = preflight.proceed(Some(&preflight.confirm())).unwrap();
    assert_eq!(
        write_svg(
            &confirmed,
            &Assets::default(),
            &text(),
            &SvgWriteOptions::default()
        ),
        Err(SvgExportError::MissingImage {
            node: fx::node_id(4),
            asset: fx::asset(1)
        })
    );
    let empty = Text {
        layouts: HashMap::new(),
        malformed: GLYPH_BROKEN,
    };
    assert_eq!(
        write_svg(&confirmed, &assets(), &empty, &SvgWriteOptions::default()),
        Err(SvgExportError::MissingTextLayout {
            node: fx::node_id(5),
            layout: KEY_OK
        })
    );
}

#[test]
fn exports_that_pass_the_preflight_do_not_stop_on_glyphs() {
    // 진단을 통과하고 확인한 SVG 내보내기는 writer가 glyph 조회 때문에 멈추지 않습니다. 첫 glyph가 정상이어도 뒤
    // glyph(outline 없는 color glyph, 손상 glyph)까지 진단해야 합니다.
    for layout in [KEY_COLOR, KEY_BROKEN] {
        let preflight = run_preflight(
            svg_capture(text_scene(&[(21, layout, false)])),
            &validated(Vec::new()),
            &[],
        );
        match preflight.proceed(Some(&preflight.confirm())) {
            Ok(confirmed) => {
                if let Err(error) =
                    write_svg(&confirmed, &assets(), &text(), &SvgWriteOptions::default())
                {
                    panic!("{layout:?}: 진단을 통과했는데 writer가 멈춤: {error}");
                }
            }
            Err(PreflightRejection::GlyphsBlocked(_)) => {}
            Err(other) => panic!("{layout:?}: {other}"),
        }
    }
}

#[test]
fn glyphs_without_outlines_block_svg_but_not_png() {
    // 첫 glyph가 정상이어도 뒤 glyph를 모두 진단하고, 같은 key는 한 번만 알립니다. 확인해도 진행하지 않습니다.
    let cases = [
        (KEY_COLOR, vec![(GLYPH_COLOR, GlyphBlockReason::NoOutline)]),
        (
            KEY_BROKEN,
            vec![(GLYPH_BROKEN, GlyphBlockReason::Malformed)],
        ),
    ];
    for (layout, expected) in cases {
        let preflight = run_preflight(
            svg_capture(text_scene(&[(21, layout, false)])),
            &validated(Vec::new()),
            &[],
        );
        let blockers: Vec<_> = expected
            .iter()
            .map(|&(glyph, reason)| GlyphBlocker {
                layout,
                nodes: vec![fx::node_id(21)],
                font: FONT_OK,
                glyph,
                reason,
            })
            .collect();
        assert_eq!(preflight.glyph_blockers(), blockers.as_slice());
        assert!(!preflight.requires_confirmation());
        let rejected = preflight.proceed(Some(&preflight.confirm())).unwrap_err();
        assert_eq!(rejected, PreflightRejection::GlyphsBlocked(blockers));
        assert!(!rejected.to_string().is_empty());
        // PNG는 renderer가 bitmap glyph를 그리므로 glyph 진단이 없습니다.
        let png = run_preflight(
            capture_of(
                text_scene(&[(21, layout, false)]),
                fx::view_request(ExportFormat::Png),
            ),
            &validated(Vec::new()),
            &[],
        );
        assert!(png.glyph_blockers().is_empty());
        assert!(png.proceed(None).is_ok());
    }
}
