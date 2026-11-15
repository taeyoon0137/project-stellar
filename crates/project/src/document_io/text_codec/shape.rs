//! 단일 View의 도형 geometry와 순서 있는 Transform·Fill·Stroke 원본을 보존합니다.

use super::*;
use stellar_core_types::value::{Angle, Rect, Vec2};
use stellar_document::shape::{
    EllipseParams, LineCap, LineParams, PolygonParams, RectangleParams, ShapeGeometry,
};

pub(super) fn encode(
    geometry: &ShapeGeometry,
    stack: Option<&ModifierStack>,
) -> Result<YamlValue, TextSourceError> {
    let mut modifiers = Vec::new();
    for entry in stack.into_iter().flat_map(ModifierStack::entries) {
        let (kind, value) = match &entry.payload {
            ModifierPayload::Transform(transform) => {
                let (a, b, c, d, x, y) = transform.components();
                ("transform", floats(&[a, b, c, d, x, y]))
            }
            ModifierPayload::Fill { color: fill } => ("fill", color(*fill)),
            ModifierPayload::Stroke { color: ink, width } => (
                "stroke",
                mapping([("color", color(*ink)), ("width", scalar(width.get()))]),
            ),
            _ => {
                return Err(TextSourceError::Unsupported(format!(
                    "Shape modifier: {:?}",
                    entry.kind
                )))
            }
        };
        modifiers.push(mapping([
            ("id", scalar(entry.id.stable_id())),
            ("order_key", order_key(&entry.order)),
            ("enabled", scalar(entry.enabled)),
            ("kind", scalar(kind)),
            ("value", value),
        ]));
    }
    let modifiers = ("modifiers", YamlValue::Sequence(modifiers));
    Ok(match geometry {
        ShapeGeometry::Rectangle(rectangle) => {
            let (tl, tr, br, bl) = rectangle.corner_radius.components();
            mapping([
                ("width", scalar(rectangle.size.width())),
                ("height", scalar(rectangle.size.height())),
                ("border-top-left-radius", scalar(tl)),
                ("border-top-right-radius", scalar(tr)),
                ("border-bottom-right-radius", scalar(br)),
                ("border-bottom-left-radius", scalar(bl)),
                modifiers,
            ])
        }
        ShapeGeometry::Ellipse(ellipse) => {
            EllipseParams::new(ellipse.bounds).map_err(invalid)?;
            let [origin, width, height] = bounds_fields(ellipse.bounds);
            mapping([origin, width, height, modifiers])
        }
        ShapeGeometry::Line(line) => mapping([
            ("start", point(line.start)),
            ("end", point(line.end)),
            (
                "cap",
                scalar(match line.cap {
                    LineCap::Butt => "butt",
                    LineCap::Round => "round",
                    LineCap::Square => "square",
                }),
            ),
            modifiers,
        ]),
        ShapeGeometry::Polygon(polygon) => {
            let [origin, width, height] = bounds_fields(polygon.bounds());
            mapping([
                origin,
                width,
                height,
                ("sides", scalar(polygon.sides())),
                ("rotation_degrees", scalar(polygon.rotation_degrees())),
                ("corner_radius", scalar(polygon.corner_radius())),
                modifiers,
            ])
        }
    })
}

fn point(value: Vec2) -> YamlValue {
    floats(&[value.x(), value.y()])
}

fn bounds_fields(bounds: Rect) -> [(&'static str, YamlValue); 3] {
    [
        ("origin", point(bounds.origin())),
        ("width", scalar(bounds.size().width())),
        ("height", scalar(bounds.size().height())),
    ]
}

fn parse_point(value: &YamlValue) -> Result<Vec2, TextSourceError> {
    let [x, y] = parse_floats(value)?;
    Vec2::new(x, y).map_err(invalid)
}

pub(super) fn decode(
    node: &YamlValue,
    kind: ShapeKind,
    stacks: &mut Vec<ModifierStack>,
) -> Result<NodeRecord, TextSourceError> {
    let value = field(node, "payload")?;
    let length = |key| parse::<f32>(field(value, key)?);
    let size = || Size2::new(length("width")?, length("height")?).map_err(invalid);
    let bounds = || -> Result<Rect, TextSourceError> {
        Ok(Rect::new(parse_point(field(value, "origin")?)?, size()?))
    };
    let geometry = match kind {
        ShapeKind::Rectangle => {
            fields(
                value,
                &[
                    "width",
                    "height",
                    "border-top-left-radius",
                    "border-top-right-radius",
                    "border-bottom-right-radius",
                    "border-bottom-left-radius",
                    "modifiers",
                ],
            )?;
            ShapeGeometry::Rectangle(RectangleParams {
                size: size()?,
                corner_radius: CornerRadius::new(
                    length("border-top-left-radius")?,
                    length("border-top-right-radius")?,
                    length("border-bottom-right-radius")?,
                    length("border-bottom-left-radius")?,
                )
                .map_err(invalid)?,
            })
        }
        ShapeKind::Ellipse => {
            fields(value, &["origin", "width", "height", "modifiers"])?;
            ShapeGeometry::Ellipse(EllipseParams::new(bounds()?).map_err(invalid)?)
        }
        ShapeKind::Line => {
            fields(value, &["start", "end", "cap", "modifiers"])?;
            ShapeGeometry::Line(LineParams {
                start: parse_point(field(value, "start")?)?,
                end: parse_point(field(value, "end")?)?,
                cap: match text(field(value, "cap")?)? {
                    "butt" => LineCap::Butt,
                    "round" => LineCap::Round,
                    "square" => LineCap::Square,
                    other => {
                        return Err(TextSourceError::Unsupported(format!("Line cap: {other}")))
                    }
                },
            })
        }
        ShapeKind::Polygon => {
            fields(
                value,
                &[
                    "origin",
                    "width",
                    "height",
                    "sides",
                    "rotation_degrees",
                    "corner_radius",
                    "modifiers",
                ],
            )?;
            ShapeGeometry::Polygon(
                PolygonParams::new(
                    bounds()?,
                    parse(field(value, "sides")?)?,
                    Angle::new(length("rotation_degrees")?).map_err(invalid)?,
                    length("corner_radius")?,
                )
                .map_err(invalid)?,
            )
        }
    };
    let record = parse_node_fields(node, NodePayload::Shape(geometry))?;
    let entries = field(value, "modifiers")?
        .as_sequence()
        .ok_or_else(|| invalid("도형 modifiers는 sequence여야 합니다"))?;
    let mut stack = ModifierStack::new(record.id);
    for entry in entries {
        fields(entry, &["id", "order_key", "enabled", "kind", "value"])?;
        let value = field(entry, "value")?;
        let payload = match text(field(entry, "kind")?)? {
            "transform" => {
                let [a, b, c, d, tx, ty] = parse_floats(value)?;
                ModifierPayload::Transform(Transform2D::new(a, b, c, d, tx, ty).map_err(invalid)?)
            }
            "fill" => ModifierPayload::Fill {
                color: parse_color(value)?,
            },
            "stroke" => {
                fields(value, &["color", "width"])?;
                ModifierPayload::Stroke {
                    color: parse_color(field(value, "color")?)?,
                    width: FiniteF32::new(parse(field(value, "width")?)?).map_err(invalid)?,
                }
            }
            kind => {
                return Err(TextSourceError::Unsupported(format!(
                    "Shape modifier: {kind}"
                )))
            }
        };
        stack
            .insert(ModifierEntry {
                id: ModifierId::new(parse(field(entry, "id")?)?),
                order: parse_order_key(field(entry, "order_key")?)?,
                enabled: parse(field(entry, "enabled")?)?,
                kind: payload.kind(),
                payload,
            })
            .map_err(invalid)?;
    }
    if !entries.is_empty() {
        stacks.push(stack);
    }
    Ok(record)
}
