//! snapshot → 영속 문서 레코드 역투영 (XIV.5.4).
//!
//! 선택 staging candidate(XIV.5.4)를 문서 validator(VI.10.1)로 검증하고 chunk model로 직렬화(XIV.5.5)하려면
//! semantic snapshot을 다시 [`DocumentRecords`]로 만들어야 합니다. 이 모듈은 [`SnapshotBuilder`]
//! (crate::build)의 문서 투영을 정확히 거꾸로 돌립니다.
//!
//! | 객체 | 레코드 |
//! |---|---|
//! | Document·Page·View | [`DocumentDefinition`](페이지·View 순서는 `DOCUMENT_PAGES`·`PAGE_VIEWS` 목록) |
//! | scene `Node` | [`NodeRecord`](필드별 속성 → typed payload, Path·Text는 canonical payload 해석)와 명시 layout longhand 속성 → [`LayoutRecord`] |
//! | `Modifier` | [`ModifierStack`](parent = 소유 node, OrderKey 순) |
//!
//! - **결정론·왕복**: `document_records(build(R)) == R`(레코드 canonical 정렬 기준)이 성립합니다. 결과는
//!   [`DocumentRecords::sort_canonical`] 순서입니다.
//! - **typed 검증**: 값은 문서 crate 생성자(크기·radius·crop·polygon 등)를 다시 거치므로 범위를 벗어난 값은
//!   [`ProjectionError`]로 거부하고 고쳐 넣지 않습니다.
//! - ComponentInstance는 정의 참조와 희소 override를 복원합니다.

use std::collections::BTreeMap;
use std::fmt;

use stellar_core_types::color::{Color, ColorSpace};
use stellar_core_types::ids::StableId;
use stellar_core_types::property::{PathSegment, PropertyKey};
use stellar_core_types::reference::{AssetRef, ComponentRef, NodeRef};
use stellar_core_types::typed_value::TypedValue;
use stellar_core_types::value::{Angle, FiniteF32, Rect, Size2, Transform2D, Vec2};
use stellar_document::component::definition::{
    ComponentDefinition, ComponentPropertyId, ComponentSlotId,
};
use stellar_document::component::instance::{ComponentInstancePayload, OverrideKey, SlotOverride};
use stellar_document::definitions::{DocumentDefinition, PageDefinition, ViewDefinition};
use stellar_document::fragment::FragmentPayload;
use stellar_document::frame::{Border, Clipping, CornerRadius, FramePayload};
use stellar_document::image::{CropRect, FitMode, ImagePayload, SamplingMode};
use stellar_document::layout_record::{format_value, LayoutLonghand, LayoutRecord};
use stellar_document::modifier::{
    BlendMode, BooleanOp, ModifierEntry, ModifierId, ModifierKind, ModifierPayload, ModifierStack,
};
use stellar_document::node_kind::NodeKind;
use stellar_document::payload_metadata::{image_keys, shape_keys};
use stellar_document::record_codec::decode_payload;
use stellar_document::records::{DocumentRecords, NodePayload, NodeRecord};
use stellar_document::shape::{
    EllipseParams, LineCap, LineParams, PolygonParams, RectangleParams, ShapeGeometry, ShapeKind,
};

use crate::keys;
use crate::snapshot::{ObjectKind, Origin, SemanticObject, SemanticSnapshot};
use crate::value::SemanticValue;

/// 역투영 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectionError {
    /// 문제 객체.
    pub object: StableId,
    /// 원인.
    pub reason: ProjectionFailure,
}

/// 역투영 실패 원인.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectionFailure {
    /// 문서 객체가 없음.
    MissingDocument,
    /// 목록이 가리키는 Page·View 객체가 없거나 종류가 다름.
    MissingDefinition {
        /// 없는 id.
        id: StableId,
    },
    /// 필수 속성이 없거나 형식이 다름.
    BadProperty {
        /// 속성 key.
        key: PropertyKey,
    },
    /// 값이 문서 규칙(범위·종류)을 어김.
    InvalidValue {
        /// 속성 key.
        key: PropertyKey,
        /// 원인.
        detail: String,
    },
    /// node·modifier에 형제 순서 키가 없음.
    MissingOrderKey,
    /// modifier의 소유 node가 없음.
    MissingOwner,
    /// 정의 조립 실패(중복 Page·View 등).
    Definition(String),
    /// 컴포넌트 객체가 없거나 종류가 다름([`component_records`]).
    MissingComponent,
    /// 비어 있지 않은 컴포넌트 interface(Property·Slot·local token)는 역투영하지 않음([`component_records`]).
    UnsupportedInterface,
}

impl fmt::Display for ProjectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "객체 {} 역투영 실패: {:?}", self.object, self.reason)
    }
}

impl std::error::Error for ProjectionError {}

/// 문서 하나의 역투영 결과.
#[derive(Debug, Clone, PartialEq)]
pub struct DocumentProjection {
    /// 문서 레코드(canonical 정렬).
    pub records: DocumentRecords,
    /// 레코드에 포함한 scene ComponentInstance node(StableId 오름차순).
    pub instances: Vec<StableId>,
}

struct Reader<'a> {
    object: &'a SemanticObject,
}

impl<'a> Reader<'a> {
    fn fail(&self, reason: ProjectionFailure) -> ProjectionError {
        ProjectionError {
            object: self.object.id(),
            reason,
        }
    }

    fn bad(&self, key: PropertyKey) -> ProjectionError {
        self.fail(ProjectionFailure::BadProperty { key })
    }

    fn invalid(&self, key: PropertyKey, detail: impl fmt::Display) -> ProjectionError {
        self.fail(ProjectionFailure::InvalidValue {
            key,
            detail: detail.to_string(),
        })
    }

    fn get(&self, key: PropertyKey) -> Option<&'a SemanticValue> {
        self.object.property(&self.object.path(&[key]))
    }

    fn text(&self, key: PropertyKey) -> Result<String, ProjectionError> {
        match self.get(key) {
            Some(SemanticValue::Text(text)) => Ok(text.clone()),
            _ => Err(self.bad(key)),
        }
    }

    fn bool(&self, key: PropertyKey) -> Result<bool, ProjectionError> {
        match self.get(key) {
            Some(SemanticValue::Bool(value)) => Ok(*value),
            _ => Err(self.bad(key)),
        }
    }

    fn uint(&self, key: PropertyKey) -> Result<u64, ProjectionError> {
        match self.get(key) {
            Some(SemanticValue::Uint(value)) => Ok(*value),
            _ => Err(self.bad(key)),
        }
    }

    fn id(&self, key: PropertyKey) -> Result<StableId, ProjectionError> {
        match self.get(key) {
            Some(SemanticValue::Id(id)) => Ok(*id),
            _ => Err(self.bad(key)),
        }
    }

    fn ids(&self, key: PropertyKey) -> Result<Vec<StableId>, ProjectionError> {
        match self.get(key) {
            Some(SemanticValue::Ids(ids)) => Ok(ids.clone()),
            _ => Err(self.bad(key)),
        }
    }

    fn bytes(&self, key: PropertyKey) -> Result<&'a [u8], ProjectionError> {
        match self.get(key) {
            Some(SemanticValue::Bytes(bytes)) => Ok(bytes),
            _ => Err(self.bad(key)),
        }
    }

    fn color(
        &self,
        value: Option<&SemanticValue>,
        key: PropertyKey,
    ) -> Result<Color, ProjectionError> {
        match value {
            Some(SemanticValue::Typed(TypedValue::Color(color))) => Ok(*color),
            _ => Err(self.bad(key)),
        }
    }

    fn floats_of<const N: usize>(
        &self,
        value: Option<&SemanticValue>,
        key: PropertyKey,
    ) -> Result<[f32; N], ProjectionError> {
        match value {
            Some(SemanticValue::Floats(values)) if values.len() == N => {
                let mut out = [0.0; N];
                for (slot, value) in out.iter_mut().zip(values) {
                    *slot = value.get();
                }
                Ok(out)
            }
            _ => Err(self.bad(key)),
        }
    }

    fn floats<const N: usize>(&self, key: PropertyKey) -> Result<[f32; N], ProjectionError> {
        self.floats_of(self.get(key), key)
    }

    fn finite(&self, key: PropertyKey) -> Result<FiniteF32, ProjectionError> {
        let [value] = self.floats::<1>(key)?;
        FiniteF32::new(value).map_err(|e| self.invalid(key, format!("{e:?}")))
    }

    fn size(&self, key: PropertyKey) -> Result<Size2, ProjectionError> {
        let [w, h] = self.floats(key)?;
        Size2::new(w, h).map_err(|e| self.invalid(key, format!("{e:?}")))
    }

    /// longhand `width`·`height` 두 경로에서 크기를 읽습니다.
    fn size_of(&self, width: PropertyKey, height: PropertyKey) -> Result<Size2, ProjectionError> {
        let [w] = self.floats(width)?;
        let [h] = self.floats(height)?;
        Size2::new(w, h)
            .map_err(|e| self.invalid(if w < 0.0 { width } else { height }, format!("{e:?}")))
    }

    /// longhand x·y 두 경로에서 점을 읽습니다.
    fn vec2_of(&self, x: PropertyKey, y: PropertyKey) -> Result<Vec2, ProjectionError> {
        let [vx] = self.floats(x)?;
        let [vy] = self.floats(y)?;
        Vec2::new(vx, vy).map_err(|e| self.invalid(x, format!("{e:?}")))
    }

    /// Ellipse·Polygon bounds: 크기는 longhand `width`·`height`, 원점은 snapshot 전용 `origin_key`.
    fn bounds(&self, origin_key: PropertyKey) -> Result<Rect, ProjectionError> {
        let origin = self.vec2(origin_key)?;
        let size = self.size_of(shape_keys::WIDTH, shape_keys::HEIGHT)?;
        Ok(Rect::new(origin, size))
    }

    /// TL, TR, BR, BL longhand 네 경로에서 모서리 반경을 읽습니다.
    fn corners_of(&self, longhands: [PropertyKey; 4]) -> Result<CornerRadius, ProjectionError> {
        let mut radii = [0.0; 4];
        for (slot, key) in radii.iter_mut().zip(longhands) {
            let [value] = self.floats(key)?;
            if value < 0.0 {
                return Err(self.invalid(key, "음수 모서리 반경"));
            }
            *slot = value;
        }
        let [tl, tr, br, bl] = radii;
        CornerRadius::new(tl, tr, br, bl).map_err(|e| self.invalid(longhands[0], format!("{e:?}")))
    }

    fn frame_size(&self) -> Result<Size2, ProjectionError> {
        use keys::frame;
        if self.get(frame::SIZE).is_some() {
            if self.get(frame::WIDTH).is_some() || self.get(frame::HEIGHT).is_some() {
                return Err(self.bad(frame::SIZE));
            }
            return self.size(frame::SIZE);
        }
        let [width] = self.floats(frame::WIDTH)?;
        let [height] = self.floats(frame::HEIGHT)?;
        Size2::new(width, height).map_err(|e| {
            self.invalid(
                if width < 0.0 {
                    frame::WIDTH
                } else {
                    frame::HEIGHT
                },
                format!("{e:?}"),
            )
        })
    }

    fn vec2(&self, key: PropertyKey) -> Result<Vec2, ProjectionError> {
        let [x, y] = self.floats(key)?;
        Vec2::new(x, y).map_err(|e| self.invalid(key, format!("{e:?}")))
    }

    fn transform(&self, key: PropertyKey) -> Result<Transform2D, ProjectionError> {
        let [a, b, c, d, tx, ty] = self.floats(key)?;
        Transform2D::new(a, b, c, d, tx, ty).map_err(|e| self.invalid(key, format!("{e:?}")))
    }

    fn corner_radius(&self, key: PropertyKey) -> Result<CornerRadius, ProjectionError> {
        let [tl, tr, br, bl] = self.floats(key)?;
        CornerRadius::new(tl, tr, br, bl).map_err(|e| self.invalid(key, format!("{e:?}")))
    }

    fn frame_corners(&self) -> Result<CornerRadius, ProjectionError> {
        use keys::frame;
        if self.get(frame::CORNER_RADIUS).is_some() {
            if frame::BORDER_RADIUS_LONGHANDS
                .iter()
                .any(|&key| self.get(key).is_some())
            {
                return Err(self.bad(frame::CORNER_RADIUS));
            }
            return self.corner_radius(frame::CORNER_RADIUS);
        }
        self.corners_of(frame::BORDER_RADIUS_LONGHANDS)
    }

    /// Shadow offset: longhand x·y 두 경로. 옛 묶음 경로(0x1092)는 longhand가 하나도 없을 때만 읽습니다(Frame 크기·
    /// 모서리와 같은 규칙, 섞이면 거부).
    fn shadow_offset(&self) -> Result<Vec2, ProjectionError> {
        use keys::modifier as m;
        if self.get(m::SHADOW_OFFSET).is_some() {
            if m::SHADOW_OFFSET_LONGHANDS
                .iter()
                .any(|&key| self.get(key).is_some())
            {
                return Err(self.bad(m::SHADOW_OFFSET));
            }
            return self.vec2(m::SHADOW_OFFSET);
        }
        self.vec2_of(m::SHADOW_OFFSET_X, m::SHADOW_OFFSET_Y)
    }

    fn small(&self, key: PropertyKey) -> Result<u8, ProjectionError> {
        u8::try_from(self.uint(key)?).map_err(|_| self.invalid(key, "tag 범위 초과"))
    }
}

/// scene·정의 node payload를 속성에서 복원합니다.
fn node_payload(object: &SemanticObject, kind: NodeKind) -> Result<NodePayload, ProjectionError> {
    let r = Reader { object };
    match kind {
        NodeKind::Frame => frame_payload(&r).map(NodePayload::Frame),
        NodeKind::Fragment => Ok(NodePayload::Fragment(FragmentPayload)),
        NodeKind::Shape => shape_payload(&r).map(NodePayload::Shape),
        NodeKind::Path | NodeKind::Text => {
            let key = if kind == NodeKind::Path {
                keys::PATH_DATA
            } else {
                keys::TEXT_PAYLOAD
            };
            decode_payload(kind, r.bytes(key)?).map_err(|e| r.invalid(key, e))
        }
        NodeKind::Image => image_payload(&r).map(NodePayload::Image),
        NodeKind::ComponentInstance => instance_payload(&r).map(NodePayload::ComponentInstance),
    }
}

fn instance_payload(r: &Reader<'_>) -> Result<ComponentInstancePayload, ProjectionError> {
    use keys::instance::{COMPONENT_REF, PROPERTY_OVERRIDES, SLOT_OVERRIDES};
    let mut instance = ComponentInstancePayload::new(ComponentRef::new(r.id(COMPONENT_REF)?));
    for (path, value) in r.object.properties() {
        let Some(&first) = path.keys().first() else {
            continue;
        };
        if first != PROPERTY_OVERRIDES && first != SLOT_OVERRIDES {
            continue;
        }
        let Some(PathSegment::Key(segment)) = path.tail() else {
            return Err(r.bad(first));
        };
        if path.keys() != [first] {
            return Err(r.bad(first));
        }
        match (first, value) {
            (PROPERTY_OVERRIDES, SemanticValue::Typed(value)) => {
                let mut ids = segment
                    .split('/')
                    .map(|s| s.parse::<StableId>().map_err(|_| r.bad(first)))
                    .collect::<Result<Vec<_>, _>>()?;
                let property = ComponentPropertyId::new(ids.pop().ok_or_else(|| r.bad(first))?);
                let key = OverrideKey::nested(ids, property);
                if crate::build::override_segment(&key) != *segment {
                    return Err(r.bad(first));
                }
                instance.set_property_override(key, value.clone());
            }
            (SLOT_OVERRIDES, SemanticValue::Ids(content)) => {
                let id: StableId = segment.parse().map_err(|_| r.bad(first))?;
                if id.to_string() != *segment {
                    return Err(r.bad(first));
                }
                instance.set_slot_override(
                    ComponentSlotId::new(id),
                    SlotOverride::new(content.clone()).map_err(|e| r.invalid(first, e))?,
                );
            }
            _ => return Err(r.bad(first)),
        }
    }
    Ok(instance)
}

fn frame_payload(r: &Reader<'_>) -> Result<FramePayload, ProjectionError> {
    use keys::frame;
    let clipping = match r.get(frame::CLIPPING) {
        Some(SemanticValue::Uint(0)) => Clipping::Disabled,
        Some(SemanticValue::Uint(1)) => Clipping::FrameBounds,
        Some(SemanticValue::Id(target)) => Clipping::Path(NodeRef::new(*target)),
        _ => return Err(r.bad(frame::CLIPPING)),
    };
    let background = match r.get(frame::BACKGROUND) {
        None => None,
        value => Some(r.color(value, frame::BACKGROUND)?),
    };
    let border_path = r.object.path(&[frame::BORDER]);
    let border = match (
        r.object.property(&border_path.clone().with_index(0)),
        r.object.property(&border_path.with_index(1)),
    ) {
        (None, None) => None,
        (color, thickness) => {
            let color = r.color(color, frame::BORDER)?;
            let [thickness] = r.floats_of::<1>(thickness, frame::BORDER)?;
            Some(
                Border::new(color, thickness)
                    .map_err(|e| r.invalid(frame::BORDER, format!("{e:?}")))?,
            )
        }
    };
    let layout_mode = match r.get(frame::LAYOUT_MODE) {
        None => None,
        Some(SemanticValue::Id(id)) => Some(*id),
        Some(_) => return Err(r.bad(frame::LAYOUT_MODE)),
    };
    Ok(FramePayload {
        size: r.frame_size()?,
        transform: r.transform(frame::TRANSFORM)?,
        clipping,
        background,
        border,
        corner_radius: r.frame_corners()?,
        layout_mode,
    })
}

fn shape_payload(r: &Reader<'_>) -> Result<ShapeGeometry, ProjectionError> {
    let kind = ShapeKind::from_tag(r.small(keys::SHAPE_KIND)?)
        .map_err(|e| r.invalid(keys::SHAPE_KIND, format!("{e:?}")))?;
    Ok(match kind {
        ShapeKind::Rectangle => ShapeGeometry::Rectangle(RectangleParams {
            size: r.size_of(shape_keys::WIDTH, shape_keys::HEIGHT)?,
            corner_radius: r.corners_of(shape_keys::BORDER_RADIUS_LONGHANDS)?,
        }),
        ShapeKind::Ellipse => ShapeGeometry::Ellipse(EllipseParams {
            bounds: r.bounds(keys::ELLIPSE_BOUNDS)?,
        }),
        ShapeKind::Line => ShapeGeometry::Line(LineParams {
            start: r.vec2_of(shape_keys::X1, shape_keys::Y1)?,
            end: r.vec2_of(shape_keys::X2, shape_keys::Y2)?,
            cap: match r.uint(keys::LINE_CAP)? {
                0 => LineCap::Butt,
                1 => LineCap::Round,
                2 => LineCap::Square,
                other => return Err(r.invalid(keys::LINE_CAP, other)),
            },
        }),
        ShapeKind::Polygon => {
            let sides = u32::try_from(r.uint(keys::POLYGON_SIDES)?)
                .map_err(|_| r.invalid(keys::POLYGON_SIDES, "변 수 범위 초과"))?;
            let [rotation] = r.floats::<1>(keys::POLYGON_ROTATION)?;
            let rotation = Angle::new(rotation)
                .map_err(|e| r.invalid(keys::POLYGON_ROTATION, format!("{e:?}")))?;
            let [radius] = r.floats::<1>(keys::POLYGON_CORNER_RADIUS)?;
            ShapeGeometry::Polygon(
                PolygonParams::new(r.bounds(keys::POLYGON_BOUNDS)?, sides, rotation, radius)
                    .map_err(|e| r.invalid(keys::POLYGON_SIDES, format!("{e:?}")))?,
            )
        }
    })
}

fn image_payload(r: &Reader<'_>) -> Result<ImagePayload, ProjectionError> {
    let [x, y, w, h] = r.floats(keys::IMAGE_CROP)?;
    let crop = CropRect::from_components(x, y, w, h)
        .map_err(|e| r.invalid(keys::IMAGE_CROP, format!("{e:?}")))?;
    let color_space_override = match r.get(keys::IMAGE_COLOR_SPACE) {
        None => None,
        Some(_) => {
            let tag = r.small(keys::IMAGE_COLOR_SPACE)?;
            Some(ColorSpace::from_tag(tag).ok_or_else(|| r.invalid(keys::IMAGE_COLOR_SPACE, tag))?)
        }
    };
    Ok(ImagePayload {
        asset: AssetRef::new(r.id(keys::IMAGE_ASSET)?),
        size: r.size_of(image_keys::WIDTH, image_keys::HEIGHT)?,
        fit: match r.uint(keys::IMAGE_FIT)? {
            0 => FitMode::Fill,
            1 => FitMode::Contain,
            2 => FitMode::Cover,
            3 => FitMode::None,
            other => return Err(r.invalid(keys::IMAGE_FIT, other)),
        },
        crop,
        sampling: match r.uint(keys::IMAGE_SAMPLING)? {
            0 => SamplingMode::Linear,
            1 => SamplingMode::Nearest,
            other => return Err(r.invalid(keys::IMAGE_SAMPLING, other)),
        },
        color_space_override,
    })
}

fn modifier_payload(
    r: &Reader<'_>,
    kind: ModifierKind,
) -> Result<ModifierPayload, ProjectionError> {
    use keys::modifier as m;
    let color = |key| r.color(r.get(key), key);
    Ok(match kind {
        ModifierKind::Transform => ModifierPayload::Transform(r.transform(m::TRANSFORM_MATRIX)?),
        ModifierKind::Boolean => ModifierPayload::Boolean {
            op: match r.uint(m::BOOLEAN_OP)? {
                0 => BooleanOp::Union,
                1 => BooleanOp::Subtract,
                2 => BooleanOp::Intersect,
                3 => BooleanOp::Exclude,
                other => return Err(r.invalid(m::BOOLEAN_OP, other)),
            },
            operands: r
                .ids(m::BOOLEAN_OPERANDS)?
                .into_iter()
                .map(NodeRef::new)
                .collect(),
        },
        ModifierKind::StrokeExpand => ModifierPayload::StrokeExpand {
            width: r.finite(m::STROKE_EXPAND_WIDTH)?,
        },
        ModifierKind::Corner => ModifierPayload::Corner {
            radius: r.finite(m::CORNER_RADIUS)?,
        },
        ModifierKind::Mask => ModifierPayload::Mask {
            source: NodeRef::new(r.id(m::MASK_SOURCE)?),
        },
        ModifierKind::Clip => ModifierPayload::Clip {
            source: NodeRef::new(r.id(m::CLIP_SOURCE)?),
        },
        ModifierKind::Fill => ModifierPayload::Fill {
            color: color(m::FILL_COLOR)?,
        },
        ModifierKind::Stroke => ModifierPayload::Stroke {
            color: color(m::STROKE_COLOR)?,
            width: r.finite(m::STROKE_WIDTH)?,
        },
        ModifierKind::Shadow => ModifierPayload::Shadow {
            color: color(m::SHADOW_COLOR)?,
            offset: r.shadow_offset()?,
            blur: r.finite(m::SHADOW_BLUR)?,
        },
        ModifierKind::Blur => ModifierPayload::Blur {
            radius: r.finite(m::BLUR_RADIUS)?,
        },
        ModifierKind::Blend => ModifierPayload::Blend {
            mode: match r.uint(m::BLEND_MODE)? {
                0 => BlendMode::Normal,
                1 => BlendMode::Multiply,
                2 => BlendMode::Screen,
                3 => BlendMode::Overlay,
                4 => BlendMode::Darken,
                5 => BlendMode::Lighten,
                other => return Err(r.invalid(m::BLEND_MODE, other)),
            },
        },
        ModifierKind::Opacity => ModifierPayload::Opacity {
            value: r.finite(m::OPACITY_VALUE)?,
        },
    })
}

/// node 객체의 명시 layout longhand 속성에서 record를 복원합니다. key가 layout longhand record key 하나인 속성만
/// 봅니다. 값은 정규 CSS 값 텍스트여야 하며(`format_value(parse(text)) == text`) 다른 표기는 고쳐 넣지 않고
/// 거부합니다. node 종류에 붙지 않는 longhand는 문서 validator가 거부합니다.
fn layout_record(r: &Reader<'_>) -> Result<Option<LayoutRecord>, ProjectionError> {
    let mut record = LayoutRecord::new(r.object.id());
    for (path, value) in r.object.properties() {
        let [key] = path.keys() else {
            continue;
        };
        let Some(longhand) = LayoutLonghand::ALL
            .into_iter()
            .find(|l| l.record_key() == *key)
        else {
            continue;
        };
        if path.tail().is_some() {
            return Err(r.bad(*key));
        }
        let SemanticValue::Text(text) = value else {
            return Err(r.bad(*key));
        };
        let parsed = longhand.parse(text).map_err(|e| r.invalid(*key, e))?;
        if format_value(&parsed) != *text {
            return Err(r.invalid(*key, format!("{longhand}: 정규 값 표기가 아닙니다")));
        }
        record
            .set(longhand, parsed)
            .map_err(|e| r.invalid(*key, e))?;
    }
    Ok((!record.is_empty()).then_some(record))
}

/// 문서 정의(Document·Page·View)를 복원합니다.
fn definition(
    snapshot: &SemanticSnapshot,
    document: StableId,
) -> Result<DocumentDefinition, ProjectionError> {
    let fail = |object, reason| ProjectionError { object, reason };
    let doc = snapshot
        .get(document)
        .filter(|o| o.kind() == ObjectKind::Document)
        .ok_or(fail(document, ProjectionFailure::MissingDocument))?;
    let r = Reader { object: doc };
    let mut definition = DocumentDefinition::new(document, r.text(keys::DISPLAY_NAME)?);
    definition.text_styles = crate::text_styles::restore(doc)?;
    for page_id in r.ids(keys::DOCUMENT_PAGES)? {
        let page = snapshot
            .get(page_id)
            .filter(|o| o.kind() == ObjectKind::Page)
            .ok_or(fail(
                document,
                ProjectionFailure::MissingDefinition { id: page_id },
            ))?;
        let pr = Reader { object: page };
        let mut page_definition = PageDefinition::new(page_id, pr.text(keys::DISPLAY_NAME)?);
        for view_id in pr.ids(keys::PAGE_VIEWS)? {
            let view = snapshot
                .get(view_id)
                .filter(|o| o.kind() == ObjectKind::View)
                .ok_or(fail(
                    page_id,
                    ProjectionFailure::MissingDefinition { id: view_id },
                ))?;
            let vr = Reader { object: view };
            page_definition
                .add_view(ViewDefinition::new(
                    view_id,
                    vr.text(keys::DISPLAY_NAME)?,
                    vr.id(keys::VIEW_ROOT_FRAME)?,
                ))
                .map_err(|e| fail(page_id, ProjectionFailure::Definition(format!("{e:?}"))))?;
        }
        definition
            .add_page(page_definition)
            .map_err(|e| fail(document, ProjectionFailure::Definition(format!("{e:?}"))))?;
    }
    Ok(definition)
}

/// snapshot에서 문서 하나의 영속 레코드를 복원합니다.
pub fn document_records(
    snapshot: &SemanticSnapshot,
    document: StableId,
) -> Result<DocumentProjection, ProjectionError> {
    let mut records = DocumentRecords::new(definition(snapshot, document)?);
    let mut instances = Vec::new();
    let mut stacks: BTreeMap<StableId, ModifierStack> = BTreeMap::new();
    let origin = Origin::Document(document);
    for object in snapshot.objects().filter(|o| o.origin() == origin) {
        let r = Reader { object };
        match object.kind() {
            ObjectKind::Node(kind) => {
                if kind == NodeKind::ComponentInstance {
                    instances.push(object.id());
                }
                let order_key = object
                    .order_key()
                    .cloned()
                    .ok_or(r.fail(ProjectionFailure::MissingOrderKey))?;
                records.nodes.push(NodeRecord {
                    id: object.id(),
                    parent: object.parent(),
                    order_key,
                    visible: r.bool(keys::VISIBLE)?,
                    locked: r.bool(keys::LOCKED)?,
                    display_name: r.text(keys::DISPLAY_NAME)?,
                    payload: node_payload(object, kind)?,
                });
                if let Some(layout) = layout_record(&r)? {
                    records.layout_records.push(layout);
                }
            }
            ObjectKind::Modifier(kind) => {
                let owner = object
                    .parent()
                    .ok_or(r.fail(ProjectionFailure::MissingOwner))?;
                let order = object
                    .order_key()
                    .cloned()
                    .ok_or(r.fail(ProjectionFailure::MissingOrderKey))?;
                let entry = ModifierEntry {
                    id: ModifierId::new(object.id()),
                    order,
                    kind,
                    enabled: r.bool(keys::modifier::ENABLED)?,
                    payload: modifier_payload(&r, kind)?,
                };
                stacks
                    .entry(owner)
                    .or_insert_with(|| ModifierStack::new(owner))
                    .insert(entry)
                    .map_err(|e| {
                        r.fail(ProjectionFailure::InvalidValue {
                            key: keys::modifier::ENABLED,
                            detail: format!("{e:?}"),
                        })
                    })?;
            }
            _ => {}
        }
    }
    records.modifier_stacks = stacks.into_values().collect();
    crate::guides::restore(snapshot, document, &mut records)?;
    records.sort_canonical();
    Ok(DocumentProjection { records, instances })
}

/// 컴포넌트 정의 하나의 역투영 결과([`component_records`]).
#[derive(Debug, Clone, PartialEq)]
pub struct ComponentProjection {
    /// 정의(id·표시 이름·root). interface는 비어 있습니다.
    pub definition: ComponentDefinition,
    /// 정의 tree node 레코드(StableId 오름차순). root도 포함합니다.
    pub nodes: Vec<NodeRecord>,
}

/// snapshot에서 컴포넌트 정의 하나와 그 정의 tree node 레코드를 복원합니다(XXIV.2.1).
///
/// [`SnapshotBuilder`](crate::build::SnapshotBuilder)의 컴포넌트 투영(`add_component_library`·
/// `add_project_component_library`)을 거꾸로 돌립니다. node payload는 문서 node와 같은 필드별 속성에서
/// 복원합니다. 지금은 interface(Property·Slot·local token)가 빈 정의만 다룹니다 — 텍스트 컴포넌트
/// 원본(ADR-0061)이 빈 interface만 담기 때문이며, 비어 있지 않으면 [`ProjectionFailure::UnsupportedInterface`]
/// 로 거부하고 고쳐 넣지 않습니다.
pub fn component_records(
    snapshot: &SemanticSnapshot,
    component: StableId,
) -> Result<ComponentProjection, ProjectionError> {
    let fail = |object, reason| ProjectionError { object, reason };
    let object = snapshot
        .get(component)
        .filter(|o| o.kind() == ObjectKind::Component)
        .ok_or(fail(component, ProjectionFailure::MissingComponent))?;
    let r = Reader { object };
    for key in [
        keys::COMPONENT_PROPERTIES,
        keys::COMPONENT_SLOTS,
        keys::COMPONENT_LOCAL_TOKENS,
    ] {
        if !r.ids(key)?.is_empty() {
            return Err(fail(component, ProjectionFailure::UnsupportedInterface));
        }
    }
    let definition = ComponentDefinition::new(
        component,
        r.text(keys::DISPLAY_NAME)?,
        NodeRef::new(r.id(keys::COMPONENT_ROOT)?),
    );
    let origin = object.origin();
    let mut nodes = Vec::new();
    for node in snapshot
        .objects()
        .filter(|o| o.id() != component && o.origin() == origin)
    {
        let r = Reader { object: node };
        let ObjectKind::Node(kind) = node.kind() else {
            return Err(r.fail(ProjectionFailure::UnsupportedInterface));
        };
        let order_key = node
            .order_key()
            .cloned()
            .ok_or(r.fail(ProjectionFailure::MissingOrderKey))?;
        nodes.push(NodeRecord {
            id: node.id(),
            parent: node.parent(),
            order_key,
            visible: r.bool(keys::VISIBLE)?,
            locked: r.bool(keys::LOCKED)?,
            display_name: r.text(keys::DISPLAY_NAME)?,
            payload: node_payload(node, kind)?,
        });
    }
    Ok(ComponentProjection { definition, nodes })
}

/// snapshot의 모든 문서 객체 id(오름차순).
pub fn documents(snapshot: &SemanticSnapshot) -> Vec<StableId> {
    snapshot
        .objects()
        .filter(|o| o.kind() == ObjectKind::Document)
        .map(SemanticObject::id)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::tests::{build, head_records, sid};
    use stellar_document::test_support::random_document::random_valid_document;

    fn canonical(mut records: DocumentRecords) -> DocumentRecords {
        records.sort_canonical();
        records
    }

    #[test]
    fn old_frame_size_snapshots_are_read_without_guessing_mixed_keys() {
        let records = head_records();
        let mut snapshot = build(&records);
        let frame = snapshot.get_mut(sid(10)).unwrap();
        let width = frame.path(&[keys::frame::WIDTH]);
        let height = frame.path(&[keys::frame::HEIGHT]);
        let size = frame.path(&[keys::frame::SIZE]);
        let floats = |value| FiniteF32::new(value).unwrap();
        frame
            .insert_property(
                size,
                SemanticValue::Floats(vec![floats(320.), floats(240.)]),
            )
            .unwrap();
        assert!(frame_payload(&Reader { object: frame }).is_err());
        frame.remove_property(&width).unwrap();
        frame.remove_property(&height).unwrap();
        assert_eq!(
            document_records(&snapshot, sid(1)).unwrap().records,
            canonical(records)
        );
    }

    #[test]
    fn shadow_offset_reads_longhands_and_old_bundle_only_alone() {
        use keys::modifier as m;
        let px = |v: f32| SemanticValue::Floats(vec![FiniteF32::new(v).unwrap()]);
        let mut object = SemanticObject::new(
            sid(60),
            ObjectKind::Modifier(ModifierKind::Shadow),
            Origin::Document(sid(1)),
        );
        let color = SemanticValue::Typed(TypedValue::Color(
            Color::new(ColorSpace::Srgb, 0.0, 0.0, 0.0, 0.5).unwrap(),
        ));
        for (key, value) in [
            (m::SHADOW_COLOR, color),
            (m::SHADOW_BLUR, px(4.0)),
            (m::SHADOW_OFFSET_X, px(2.0)),
            (m::SHADOW_OFFSET_Y, px(-3.0)),
        ] {
            object.insert_property(object.path(&[key]), value).unwrap();
        }
        let offset = |object: &SemanticObject| match modifier_payload(
            &Reader { object },
            ModifierKind::Shadow,
        ) {
            Ok(ModifierPayload::Shadow { offset, .. }) => Ok((offset.x(), offset.y())),
            Ok(other) => panic!("{other:?}"),
            Err(error) => Err(error),
        };
        assert_eq!(offset(&object), Ok((2.0, -3.0)));
        // 옛 묶음 경로와 longhand가 섞이면 거부합니다.
        let packed = object.path(&[m::SHADOW_OFFSET]);
        object
            .insert_property(
                packed,
                SemanticValue::Floats(vec![FiniteF32::ONE, FiniteF32::ONE]),
            )
            .unwrap();
        assert!(offset(&object).is_err());
        for key in m::SHADOW_OFFSET_LONGHANDS {
            object.remove_property(&object.path(&[key])).unwrap();
        }
        assert_eq!(offset(&object), Ok((1.0, 1.0)));
    }

    #[test]
    fn old_radius_snapshot_restores_only_without_longhands() {
        let records = head_records();
        let mut snapshot = build(&records);
        let object = snapshot.get_mut(sid(10)).unwrap();
        let packed = object.path(&[keys::frame::CORNER_RADIUS]);
        let mut radii = Vec::new();
        for key in keys::frame::BORDER_RADIUS_LONGHANDS {
            let path = object.path(&[key]);
            let SemanticValue::Floats(value) = object.remove_property(&path).unwrap() else {
                panic!("Frame 반경은 한 길이 값");
            };
            radii.extend(value);
            assert!(frame_payload(&Reader { object }).is_err());
        }
        object
            .insert_property(packed, SemanticValue::Floats(radii))
            .unwrap();
        assert_eq!(
            frame_payload(&Reader { object }).unwrap().corner_radius,
            CornerRadius::ZERO
        );
        let path = object.path(&[keys::frame::BORDER_BOTTOM_RIGHT_RADIUS]);
        object
            .insert_property(path, SemanticValue::Floats(vec![FiniteF32::ZERO]))
            .unwrap();
        assert!(frame_payload(&Reader { object }).is_err());
    }

    #[test]
    fn projection_round_trips_records() {
        let head = head_records();
        let snapshot = build(&head);
        assert_eq!(documents(&snapshot), vec![sid(1)]);
        let projection = document_records(&snapshot, sid(1)).unwrap();
        assert!(projection.instances.is_empty());
        assert_eq!(projection.records, canonical(head));
        // random valid document: 모든 node·modifier 종류와 payload 필드.
        for seed in 0..200 {
            let records = random_valid_document(seed).to_records().unwrap();
            let snapshot = build(&records);
            let id = records.definition.id;
            let projection = document_records(&snapshot, id).unwrap();
            assert_eq!(projection.records, canonical(records), "seed {seed}");
            // 다시 build하면 같은 snapshot.
            assert_eq!(build(&projection.records), snapshot, "seed {seed}");
        }
    }

    #[test]
    fn document_instances_project_sparse_overrides_and_slot_references_losslessly() {
        let mut records = head_records();
        let mut instance = ComponentInstancePayload::new(ComponentRef::new(sid(300)));
        instance.set_property_override(
            OverrideKey::direct(ComponentPropertyId::new(sid(310))),
            TypedValue::Boolean(false),
        );
        instance.set_property_override(
            OverrideKey::nested(vec![sid(303)], ComponentPropertyId::new(sid(311))),
            TypedValue::String("Nested".into()),
        );
        instance.set_slot_override(
            ComponentSlotId::new(sid(330)),
            SlotOverride::new(vec![sid(11)]).unwrap(),
        );
        let parent = records.nodes[0].id;
        records.nodes.push(NodeRecord {
            id: sid(400),
            parent: Some(parent),
            order_key: stellar_core_types::order_key::OrderKey::from_bytes(vec![0xf0]).unwrap(),
            visible: true,
            locked: false,
            display_name: "Placed".into(),
            payload: NodePayload::ComponentInstance(instance),
        });
        let snapshot = build(&records);
        let projection = document_records(&snapshot, records.definition.id).unwrap();
        assert_eq!(projection.instances, [sid(400)]);
        assert_eq!(projection.records, canonical(records));
        assert_eq!(build(&projection.records), snapshot);
    }

    #[test]
    fn missing_or_malformed_objects_are_typed_errors() {
        let snapshot = build(&head_records());
        assert_eq!(
            document_records(&snapshot, sid(99)),
            Err(ProjectionError {
                object: sid(99),
                reason: ProjectionFailure::MissingDocument
            })
        );
        // 문서 목록이 없는 페이지를 가리킴.
        let mut broken = snapshot.clone();
        broken.remove(sid(2));
        assert_eq!(
            document_records(&broken, sid(1)).unwrap_err().reason,
            ProjectionFailure::MissingDefinition { id: sid(2) }
        );
        // rect 크기 형식 오류.
        let mut broken = snapshot.clone();
        let mut rect = broken.remove(sid(11)).unwrap();
        rect.set_property(rect.path(&[shape_keys::WIDTH]), SemanticValue::Bool(true))
            .unwrap();
        broken.insert(rect).unwrap();
        assert_eq!(
            document_records(&broken, sid(1)).unwrap_err(),
            ProjectionError {
                object: sid(11),
                reason: ProjectionFailure::BadProperty {
                    key: shape_keys::WIDTH
                }
            }
        );
        // 음수 크기는 고쳐 넣지 않고 거부.
        let mut broken = snapshot.clone();
        let mut rect = broken.remove(sid(11)).unwrap();
        rect.set_property(
            rect.path(&[shape_keys::WIDTH]),
            SemanticValue::Floats(vec![FiniteF32::new(-1.0).unwrap()]),
        )
        .unwrap();
        broken.insert(rect).unwrap();
        assert!(matches!(
            document_records(&broken, sid(1)).unwrap_err().reason,
            ProjectionFailure::InvalidValue { .. }
        ));
    }

    /// head 문서에 명시 layout longhand를 더합니다(ADR-0086): root Frame의 container 값, 자식의 item 값.
    pub(crate) fn with_layout(mut records: DocumentRecords) -> DocumentRecords {
        use stellar_document::layout_record::LayoutLonghand as L;
        let record = |node: u64, pairs: &[(L, &str)]| {
            let mut record = LayoutRecord::new(sid(node));
            for (longhand, text) in pairs {
                record
                    .set(*longhand, longhand.parse(text).unwrap())
                    .unwrap();
            }
            record
        };
        records.layout_records = vec![
            record(
                10,
                &[
                    (L::Display, "grid"),
                    (L::ColumnGap, "8px"),
                    (L::GridTemplateColumns, "1fr minmax(0px, 100px)"),
                ],
            ),
            record(11, &[(L::FlexGrow, "1"), (L::Left, "4px")]),
            record(12, &[(L::AlignSelf, "center")]),
        ];
        records.sort_canonical();
        records
    }

    #[test]
    fn layout_records_project_per_longhand_and_round_trip() {
        let head = with_layout(head_records());
        stellar_document::validate::validate_document(&head).unwrap();
        let snapshot = build(&head);
        let root = snapshot.get(sid(10)).unwrap();
        assert_eq!(
            root.property(&root.path(&[keys::layout_container::DISPLAY])),
            Some(&SemanticValue::Text("grid".into()))
        );
        let projected = document_records(&snapshot, sid(1)).unwrap().records;
        assert_eq!(projected, head);

        // longhand 하나만 바꾸면 그 경로 하나만 다릅니다.
        let mut working = head.clone();
        working.layout_records[0]
            .set(
                stellar_document::layout_record::LayoutLonghand::ColumnGap,
                stellar_document::layout_record::LayoutLonghand::ColumnGap
                    .parse("12px")
                    .unwrap(),
            )
            .unwrap();
        let changes = crate::diff::diff(&snapshot, &build(&working));
        let paths: Vec<_> = changes
            .changes()
            .iter()
            .filter_map(|c| c.property_path().cloned())
            .collect();
        assert_eq!(
            paths,
            vec![stellar_core_types::property::PropertyPath::with_keys(
                sid(10),
                vec![keys::layout_container::COLUMN_GAP]
            )]
        );
        // longhand를 지우면(미설정) 그 경로가 사라집니다.
        let mut unset = head.clone();
        unset.layout_records.retain(|r| r.node != sid(12));
        let restored = document_records(&build(&unset), sid(1)).unwrap().records;
        assert_eq!(restored, unset);
    }

    #[test]
    fn layout_projection_rejects_orphans_and_non_canonical_text() {
        let mut orphan = with_layout(head_records());
        orphan.layout_records[0].node = sid(99);
        let mut builder = crate::build::SnapshotBuilder::new();
        assert_eq!(
            builder.add_document(&orphan),
            Err(crate::snapshot::SnapshotError::LayoutOwnerMissing { node: sid(99) })
        );
        // 정규 표기가 아닌 값(`8.0px`)과 문자열이 아닌 값은 고쳐 넣지 않고 거부합니다.
        let snapshot = build(&with_layout(head_records()));
        for bad in [
            SemanticValue::Text("8.0px".into()),
            SemanticValue::Text("var(--gap)".into()),
            SemanticValue::Uint(8),
        ] {
            let mut broken = snapshot.clone();
            let mut root = broken.remove(sid(10)).unwrap();
            root.set_property(root.path(&[keys::layout_container::COLUMN_GAP]), bad)
                .unwrap();
            broken.insert(root).unwrap();
            assert!(document_records(&broken, sid(1)).is_err());
        }
    }
}
