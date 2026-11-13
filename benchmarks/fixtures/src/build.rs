//! 생성기 공통 부품: payload factory와 문서 마무리(로드·검증·직렬화).
//!
//! 모든 좌표·크기는 1/4 px 격자 정수(`Quarter`)에서 만들어 f32로 정확히 표현되고 플랫폼과
//! 무관합니다. 문서는 영속 레코드([`DocumentRecords`])로 조립한 뒤 **실제 경로**인
//! [`DocumentModel::from_records`](전체 validator 포함)로 런타임 모델을 만들고,
//! `stellar_project::document_io`의 serializer로 chunk model을 만듭니다.

use stellar_core_types::color::{Color, ColorSpace};
use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::reference::AssetRef;
use stellar_core_types::text::UserText;
use stellar_core_types::value::{Angle, FiniteF32, Rect, Size2, Transform2D, Vec2};
use stellar_document::frame::{Clipping, CornerRadius, FramePayload};
use stellar_document::image::{CropRect, FitMode, ImagePayload, SamplingMode};
use stellar_document::model::DocumentModel;
use stellar_document::modifier::{
    BlendMode, ModifierEntry, ModifierId, ModifierKind, ModifierPayload, ModifierStack,
};
use stellar_document::path::{IdentifiedPath, PathCommand, PathData, WindingRule};
use stellar_document::records::{DocumentRecords, NodePayload, NodeRecord};
use stellar_document::shape::{
    EllipseParams, LineCap, LineParams, PolygonParams, RectangleParams, ShapeGeometry,
};
use stellar_document::text::{Paragraph, StyleRun, TextPayload, TextRange};
use stellar_project::document_io::serialize::{serialize_document, SerializedDocument};

use crate::error::FixtureError;
use crate::ids::IdAllocator;
use crate::rng::SplitMix64;

/// 1/4 px 단위 정수 길이.
pub(crate) type Quarter = u32;

/// 1/4 px 정수를 f32로(2^24 미만이라 정확).
pub(crate) fn px(q: Quarter) -> f32 {
    q as f32 / 4.0
}

/// `0..=max` 범위의 1/4 px 값.
pub(crate) fn quarter_below(rng: &mut SplitMix64, max: Quarter) -> Quarter {
    rng.below(u64::from(max) + 1) as Quarter
}

pub(crate) fn size(w: Quarter, h: Quarter) -> Size2 {
    Size2::new(px(w), px(h)).unwrap_or(Size2::ZERO)
}

pub(crate) fn vec2(x: Quarter, y: Quarter) -> Vec2 {
    Vec2::new(px(x), px(y)).unwrap_or(Vec2::ZERO)
}

/// 평행 이동만 있는 변환.
pub(crate) fn translation(x: Quarter, y: Quarter) -> Transform2D {
    Transform2D::new(1.0, 0.0, 0.0, 1.0, px(x), px(y)).unwrap_or_else(|_| Transform2D::identity())
}

/// 결정론적 sRGB 색(채널은 1/16 격자).
pub(crate) fn color(rng: &mut SplitMix64) -> Color {
    let channel = |rng: &mut SplitMix64| rng.below(17) as f32 / 16.0;
    let r = channel(rng);
    let g = channel(rng);
    let b = channel(rng);
    let a = rng.range(8, 16) as f32 / 16.0;
    // 채널·알파가 [0, 1] 격자 값이라 Color::new는 실패하지 않습니다(상수 fallback은 방어용).
    Color::new(ColorSpace::Srgb, r, g, b, a)
        .unwrap_or_else(|_| Color::new(ColorSpace::Srgb, 0.0, 0.0, 0.0, 1.0).expect("상수 색"))
}

/// 균일 corner radius.
pub(crate) fn uniform_radius(r: Quarter) -> CornerRadius {
    CornerRadius::new(px(r), px(r), px(r), px(r)).unwrap_or(CornerRadius::ZERO)
}

/// 크기·위치가 주어진 Frame payload.
pub(crate) fn frame(
    rng: &mut SplitMix64,
    w: Quarter,
    h: Quarter,
    x: Quarter,
    y: Quarter,
) -> FramePayload {
    FramePayload {
        size: size(w, h),
        transform: translation(x, y),
        clipping: if rng.one_in(4) {
            Clipping::Disabled
        } else {
            Clipping::FrameBounds
        },
        background: rng.one_in(2).then(|| color(rng)),
        border: None,
        corner_radius: uniform_radius(quarter_below(rng, 64)),
        layout_mode: None,
    }
}

/// 이름 pool(표시 이름은 식별에 쓰지 않으므로 반복돼도 됩니다).
pub(crate) const NAME_POOL: [&str; 12] = [
    "Frame",
    "Group",
    "Button",
    "Label",
    "Icon",
    "Card",
    "Header",
    "Row",
    "목록",
    "아이콘",
    "제목",
    "본문",
];

/// 단어 pool(텍스트 내용). 라틴·한글·이모지·결합 문자를 섞습니다.
pub(crate) const WORD_POOL: [&str; 16] = [
    "Stellar",
    "design",
    "component",
    "레이아웃",
    "디자인",
    "토큰",
    "버튼",
    "결정론",
    "👩‍💻",
    "🎨",
    "e\u{301}cole",
    "naïve",
    "12,480",
    "—",
    "Ünïcode",
    "가나다",
];

/// 단어 `words`개로 된 텍스트 payload. `paragraph_every`개 단어마다 줄을 바꾸고, 앞쪽 일부
/// 범위에 style run을 둡니다.
pub(crate) fn text(rng: &mut SplitMix64, words: usize, paragraph_every: usize) -> TextPayload {
    let mut content = String::new();
    for i in 0..words {
        if i > 0 {
            if paragraph_every > 0 && i % paragraph_every == 0 {
                content.push('\n');
            } else {
                content.push(' ');
            }
        }
        content.push_str(rng.pick(&WORD_POOL));
    }
    let paragraphs = (0..=content.matches('\n').count())
        .map(|_| Paragraph {
            style: rng.below(4) as u32,
        })
        .collect();
    let len = stellar_document::text::scalar_len(&content);
    let mut runs = Vec::new();
    let run_count = rng.below(3) as u32;
    let mut cursor = 0u32;
    for _ in 0..run_count {
        if cursor >= len {
            break;
        }
        let end = (cursor + 1 + rng.below(u64::from(len - cursor)) as u32).min(len);
        if let Some(range) = TextRange::new(cursor, end) {
            runs.push(StyleRun {
                range,
                style: rng.below(8) as u32,
            });
        }
        cursor = end;
    }
    TextPayload::new(UserText::new(&content), paragraphs, runs)
        .unwrap_or_else(|_| TextPayload::empty(0))
}

/// 크기가 주어진 도형 geometry.
pub(crate) fn shape(rng: &mut SplitMix64, w: Quarter, h: Quarter) -> ShapeGeometry {
    match rng.below(4) {
        0 => ShapeGeometry::Rectangle(RectangleParams {
            size: size(w, h),
            corner_radius: uniform_radius(quarter_below(rng, 32)),
        }),
        1 => ShapeGeometry::Ellipse(EllipseParams {
            bounds: Rect::new(Vec2::ZERO, size(w, h)),
        }),
        2 => ShapeGeometry::Line(LineParams {
            start: Vec2::ZERO,
            end: vec2(w, h),
            cap: rng.pick(&[LineCap::Butt, LineCap::Round, LineCap::Square]),
        }),
        _ => {
            let bounds = Rect::new(Vec2::ZERO, size(w, h));
            let sides = rng.range(3, 8) as u32;
            let rotation =
                Angle::new((rng.below(8) * 45) as f32).unwrap_or(Angle::from_raw(FiniteF32::ZERO));
            match PolygonParams::new(bounds, sides, rotation, px(quarter_below(rng, 16))) {
                Ok(params) => ShapeGeometry::Polygon(params),
                Err(_) => ShapeGeometry::Ellipse(EllipseParams { bounds }),
            }
        }
    }
}

/// `w × h` 상자 안의 path. point·handle id는 `ids`에서 발급합니다.
pub(crate) fn path(
    rng: &mut SplitMix64,
    ids: &mut IdAllocator,
    w: Quarter,
    h: Quarter,
) -> IdentifiedPath {
    let mut commands = Vec::new();
    for _ in 0..rng.range(1, 2) {
        commands.push(PathCommand::MoveTo(vec2(
            quarter_below(rng, w),
            quarter_below(rng, h),
        )));
        for _ in 0..rng.range(2, 5) {
            if rng.one_in(2) {
                commands.push(PathCommand::LineTo(vec2(
                    quarter_below(rng, w),
                    quarter_below(rng, h),
                )));
            } else {
                commands.push(PathCommand::CubicTo {
                    c1: vec2(quarter_below(rng, w), quarter_below(rng, h)),
                    c2: vec2(quarter_below(rng, w), quarter_below(rng, h)),
                    to: vec2(quarter_below(rng, w), quarter_below(rng, h)),
                });
            }
        }
        if rng.one_in(2) {
            commands.push(PathCommand::Close);
        }
    }
    let winding = if rng.one_in(2) {
        WindingRule::NonZero
    } else {
        WindingRule::EvenOdd
    };
    let data = PathData::new(winding, commands).unwrap_or_default();
    IdentifiedPath::assign(data, &mut || ids.next_id())
}

/// asset을 참조하는 이미지 payload.
pub(crate) fn image(rng: &mut SplitMix64, asset: AssetRef, w: Quarter, h: Quarter) -> ImagePayload {
    let mut image = ImagePayload::new(asset, size(w, h));
    image.fit = rng.pick(&[
        FitMode::Fill,
        FitMode::Contain,
        FitMode::Cover,
        FitMode::None,
    ]);
    image.sampling = rng.pick(&[SamplingMode::Linear, SamplingMode::Nearest]);
    if rng.one_in(4) {
        image.crop = CropRect::from_components(0.25, 0.25, 0.5, 0.5).unwrap_or(CropRect::FULL);
    }
    image
}

/// 참조 없는 modifier payload(`Mask`·`Clip`·`Boolean`은 호출자가 따로 만듭니다).
pub(crate) fn plain_modifier(rng: &mut SplitMix64, kind: ModifierKind) -> ModifierPayload {
    let len = |rng: &mut SplitMix64, max: Quarter| {
        FiniteF32::new(px(quarter_below(rng, max))).unwrap_or(FiniteF32::ZERO)
    };
    match kind {
        ModifierKind::Transform => {
            ModifierPayload::Transform(translation(quarter_below(rng, 64), quarter_below(rng, 64)))
        }
        ModifierKind::StrokeExpand => ModifierPayload::StrokeExpand {
            width: len(rng, 32),
        },
        ModifierKind::Corner => ModifierPayload::Corner {
            radius: len(rng, 64),
        },
        ModifierKind::Fill => ModifierPayload::Fill { color: color(rng) },
        ModifierKind::Stroke => ModifierPayload::Stroke {
            color: color(rng),
            width: len(rng, 32),
        },
        ModifierKind::Shadow => ModifierPayload::Shadow {
            color: color(rng),
            offset: vec2(quarter_below(rng, 64), quarter_below(rng, 64)),
            blur: len(rng, 256),
        },
        ModifierKind::Blur => ModifierPayload::Blur {
            radius: len(rng, 256),
        },
        ModifierKind::Blend => ModifierPayload::Blend {
            mode: rng.pick(&[
                BlendMode::Normal,
                BlendMode::Multiply,
                BlendMode::Screen,
                BlendMode::Overlay,
                BlendMode::Darken,
                BlendMode::Lighten,
            ]),
        },
        ModifierKind::Opacity => ModifierPayload::Opacity {
            value: FiniteF32::new(rng.range(4, 16) as f32 / 16.0).unwrap_or(FiniteF32::ONE),
        },
        // 참조형은 기본값(참조 없음)으로 둡니다. 호출자가 대상 node를 정해 덮어씁니다.
        other => ModifierPayload::default_for(other),
    }
}

/// payload 목록으로 modifier stack을 만듭니다(순서 key는 균등 분포).
pub(crate) fn modifier_stack(
    ids: &mut IdAllocator,
    owner: StableId,
    payloads: Vec<ModifierPayload>,
) -> ModifierStack {
    let mut stack = ModifierStack::new(owner);
    let keys = OrderKey::evenly_spaced(payloads.len());
    for (payload, order) in payloads.into_iter().zip(keys) {
        let entry = ModifierEntry {
            id: ModifierId::new(ids.next_id()),
            order,
            kind: payload.kind(),
            enabled: true,
            payload,
        };
        // 새 id·균등 key라 중복이 없어 실패하지 않습니다. 실패하면 그 항목만 빠지고
        // 문서 validator가 아니라 fixture 통계 테스트가 드러냅니다.
        let _ = stack.insert(entry);
    }
    stack
}

/// 레코드 하나.
pub(crate) fn record(
    id: StableId,
    parent: Option<StableId>,
    order_key: OrderKey,
    visible: bool,
    display_name: &str,
    payload: NodePayload,
) -> NodeRecord {
    NodeRecord {
        id,
        parent,
        order_key,
        visible,
        locked: false,
        display_name: display_name.to_string(),
        payload,
    }
}

/// 실제 경로로 마무리한 문서: 런타임 모델과 chunk model.
#[derive(Debug)]
pub struct BuiltDocument {
    /// 전체 validator를 통과해 로드된 런타임 모델.
    pub model: DocumentModel,
    /// `stellar_project::document_io` serializer 결과.
    pub serialized: SerializedDocument,
}

/// 레코드를 런타임 모델로 로드(전체 validator)하고 chunk model로 직렬화합니다.
///
/// 미해결 참조가 하나라도 있으면 거부합니다 — 벤치마크 fixture는 모든 참조가 해결된 문서여야
/// 측정이 미해결 진단 경로에 치우치지 않습니다.
pub(crate) fn finish_document(records: DocumentRecords) -> Result<BuiltDocument, FixtureError> {
    let (model, diagnostics) = DocumentModel::from_records(records).map_err(FixtureError::Load)?;
    if !diagnostics.unresolved.is_empty() {
        return Err(FixtureError::Unresolved {
            count: diagnostics.unresolved.len(),
        });
    }
    let serialized = serialize_document(&model).map_err(FixtureError::Serialize)?;
    Ok(BuiltDocument { model, serialized })
}
