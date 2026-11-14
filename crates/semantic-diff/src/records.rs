//! snapshot → 영속 문서 레코드 역투영 (XIV.5.4).
//!
//! 선택 staging candidate(XIV.5.4)를 문서 validator(VI.10.1)로 검증하고 chunk model로 직렬화(XIV.5.5)하려면
//! semantic snapshot을 다시 [`DocumentRecords`]로 만들어야 합니다. 이 모듈은 [`SnapshotBuilder`]
//! (crate::build)의 문서 투영을 정확히 거꾸로 돌립니다.
//!
//! | 객체 | 레코드 |
//! |---|---|
//! | Document·Page·View | [`DocumentDefinition`](페이지·View 순서는 `DOCUMENT_PAGES`·`PAGE_VIEWS` 목록) |
//! | scene `Node`(ComponentInstance 제외) | [`NodeRecord`](필드별 속성 → typed payload, Path·Text는 canonical payload 해석) |
//! | `Modifier` | [`ModifierStack`](parent = 소유 node, OrderKey 순) |
//!
//! - **결정론·왕복**: `document_records(build(R)) == R`(레코드 canonical 정렬 기준)이 성립합니다. 결과는
//!   [`DocumentRecords::sort_canonical`] 순서입니다.
//! - **typed 검증**: 값은 문서 crate 생성자(크기·radius·crop·polygon 등)를 다시 거치므로 범위를 벗어난 값은
//!   [`ProjectionError`]로 거부하고 고쳐 넣지 않습니다.
//! - scene ComponentInstance node는 [`DocumentRecords`]가 아직 표현하지 못하므로(XIV.3.6 builder 입력도 별도)
//!   레코드에 넣지 않고 [`DocumentProjection::instances`]로 따로 돌려줍니다. 그 무결성은 semantic validator
//!   (XIV.5.3)가 검사합니다.

use std::collections::BTreeMap;
use std::fmt;

use stellar_core_types::color::{Color, ColorSpace};
use stellar_core_types::ids::StableId;
use stellar_core_types::property::PropertyKey;
use stellar_core_types::reference::{AssetRef, NodeRef};
use stellar_core_types::typed_value::TypedValue;
use stellar_core_types::value::{Angle, FiniteF32, Rect, Size2, Transform2D, Vec2};
use stellar_document::definitions::{DocumentDefinition, PageDefinition, ViewDefinition};
use stellar_document::fragment::FragmentPayload;
use stellar_document::frame::{Border, Clipping, CornerRadius, FramePayload};
use stellar_document::image::{CropRect, FitMode, ImagePayload, SamplingMode};
use stellar_document::modifier::{
    BlendMode, BooleanOp, ModifierEntry, ModifierId, ModifierKind, ModifierPayload, ModifierStack,
};
use stellar_document::node_kind::NodeKind;
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
    /// 레코드로 표현하지 못한 scene ComponentInstance node(StableId 오름차순).
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

    fn vec2(&self, key: PropertyKey) -> Result<Vec2, ProjectionError> {
        let [x, y] = self.floats(key)?;
        Vec2::new(x, y).map_err(|e| self.invalid(key, format!("{e:?}")))
    }

    fn rect(&self, key: PropertyKey) -> Result<Rect, ProjectionError> {
        let [x, y, w, h] = self.floats(key)?;
        let origin = Vec2::new(x, y).map_err(|e| self.invalid(key, format!("{e:?}")))?;
        let size = Size2::new(w, h).map_err(|e| self.invalid(key, format!("{e:?}")))?;
        Ok(Rect::new(origin, size))
    }

    fn transform(&self, key: PropertyKey) -> Result<Transform2D, ProjectionError> {
        let [a, b, c, d, tx, ty] = self.floats(key)?;
        Transform2D::new(a, b, c, d, tx, ty).map_err(|e| self.invalid(key, format!("{e:?}")))
    }

    fn corner_radius(&self, key: PropertyKey) -> Result<CornerRadius, ProjectionError> {
        let [tl, tr, br, bl] = self.floats(key)?;
        CornerRadius::new(tl, tr, br, bl).map_err(|e| self.invalid(key, format!("{e:?}")))
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
        NodeKind::ComponentInstance => Err(r.bad(keys::instance::COMPONENT_REF)),
    }
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
        size: r.size(frame::SIZE)?,
        transform: r.transform(frame::TRANSFORM)?,
        clipping,
        background,
        border,
        corner_radius: r.corner_radius(frame::CORNER_RADIUS)?,
        layout_mode,
    })
}

fn shape_payload(r: &Reader<'_>) -> Result<ShapeGeometry, ProjectionError> {
    let kind = ShapeKind::from_tag(r.small(keys::SHAPE_KIND)?)
        .map_err(|e| r.invalid(keys::SHAPE_KIND, format!("{e:?}")))?;
    Ok(match kind {
        ShapeKind::Rectangle => ShapeGeometry::Rectangle(RectangleParams {
            size: r.size(keys::RECT_SIZE)?,
            corner_radius: r.corner_radius(keys::RECT_CORNER_RADIUS)?,
        }),
        ShapeKind::Ellipse => ShapeGeometry::Ellipse(EllipseParams {
            bounds: r.rect(keys::ELLIPSE_BOUNDS)?,
        }),
        ShapeKind::Line => ShapeGeometry::Line(LineParams {
            start: r.vec2(keys::LINE_START)?,
            end: r.vec2(keys::LINE_END)?,
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
                PolygonParams::new(r.rect(keys::POLYGON_BOUNDS)?, sides, rotation, radius)
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
        size: r.size(keys::IMAGE_SIZE)?,
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
            offset: r.vec2(m::SHADOW_OFFSET)?,
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
            ObjectKind::Node(NodeKind::ComponentInstance) => instances.push(object.id()),
            ObjectKind::Node(kind) => {
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
    records.sort_canonical();
    Ok(DocumentProjection { records, instances })
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
        rect.set_property(rect.path(&[keys::RECT_SIZE]), SemanticValue::Bool(true))
            .unwrap();
        broken.insert(rect).unwrap();
        assert_eq!(
            document_records(&broken, sid(1)).unwrap_err(),
            ProjectionError {
                object: sid(11),
                reason: ProjectionFailure::BadProperty {
                    key: keys::RECT_SIZE
                }
            }
        );
        // 음수 크기는 고쳐 넣지 않고 거부.
        let mut broken = snapshot.clone();
        let mut rect = broken.remove(sid(11)).unwrap();
        rect.set_property(
            rect.path(&[keys::RECT_SIZE]),
            SemanticValue::Floats(vec![
                FiniteF32::new(-1.0).unwrap(),
                FiniteF32::new(1.0).unwrap(),
            ]),
        )
        .unwrap();
        broken.insert(rect).unwrap();
        assert!(matches!(
            document_records(&broken, sid(1)).unwrap_err().reason,
            ProjectionFailure::InvalidValue { .. }
        ));
    }
}
