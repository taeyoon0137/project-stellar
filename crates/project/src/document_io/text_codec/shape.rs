//! View v2의 도형 geometry와 순서 있는 Transform·Fill·Stroke 원본을 보존합니다.
//! 기하 속성과 modifier 파라미터는 값 하나를 키 하나·한 줄로 씁니다(ADR-0061 결정 2).

use super::*;
use stellar_core_types::value::{Angle, FiniteF32, Rect, Vec2};
use stellar_document::shape::{
    EllipseParams, LineCap, LineParams, PolygonParams, RectangleParams, ShapeGeometry,
};

fn modifier(entry: &ModifierEntry) -> Result<YamlValue, TextSourceError> {
    let common = |kind: &str| {
        vec![
            (scalar("id"), scalar(entry.id.stable_id())),
            (scalar("order_key"), order_key(&entry.order)),
            (scalar("enabled"), scalar(entry.enabled)),
            (scalar("kind"), scalar(kind)),
        ]
    };
    let fields = match &entry.payload {
        ModifierPayload::Transform(transform) => {
            let (a, b, c, d, x, y) = transform.components();
            let mut fields = common("transform");
            fields.push((scalar("matrix"), floats(&[a, b, c, d, x, y])));
            fields
        }
        ModifierPayload::Fill { color: fill } => {
            let mut fields = common("fill");
            fields.push((scalar("color"), color_line(Some(*fill))));
            fields
        }
        ModifierPayload::Stroke { color: ink, width } => {
            let mut fields = common("stroke");
            fields.push((scalar("color"), color_line(Some(*ink))));
            fields.push((scalar("width"), scalar(width.get())));
            fields
        }
        _ => {
            return Err(TextSourceError::Unsupported(format!(
                "Shape modifier: {:?}",
                entry.kind
            )))
        }
    };
    Ok(YamlValue::Mapping(fields))
}

pub(super) fn encode(
    geometry: &ShapeGeometry,
    stack: Option<&ModifierStack>,
) -> Result<YamlValue, TextSourceError> {
    let modifiers = (
        "modifiers",
        YamlValue::Sequence(
            stack
                .into_iter()
                .flat_map(ModifierStack::entries)
                .map(modifier)
                .collect::<Result<_, _>>()?,
        ),
    );
    Ok(match geometry {
        ShapeGeometry::Rectangle(rectangle) => {
            let (tl, tr, br, bl) = rectangle.corner_radius.components();
            mapping([
                ("border-bottom-left-radius", scalar(bl)),
                ("border-bottom-right-radius", scalar(br)),
                ("border-top-left-radius", scalar(tl)),
                ("border-top-right-radius", scalar(tr)),
                ("height", scalar(rectangle.size.height())),
                ("width", scalar(rectangle.size.width())),
                modifiers,
            ])
        }
        ShapeGeometry::Ellipse(ellipse) => {
            EllipseParams::new(ellipse.bounds).map_err(invalid)?;
            let [height, x, y, width] = bounds_fields(ellipse.bounds);
            mapping([height, x, y, width, modifiers])
        }
        ShapeGeometry::Line(line) => mapping([
            (
                "cap",
                scalar(match line.cap {
                    LineCap::Butt => "butt",
                    LineCap::Round => "round",
                    LineCap::Square => "square",
                }),
            ),
            ("end-x", scalar(line.end.x())),
            ("end-y", scalar(line.end.y())),
            ("start-x", scalar(line.start.x())),
            ("start-y", scalar(line.start.y())),
            modifiers,
        ]),
        ShapeGeometry::Polygon(polygon) => {
            let [height, x, y, width] = bounds_fields(polygon.bounds());
            mapping([
                ("corner_radius", scalar(polygon.corner_radius())),
                height,
                x,
                y,
                ("rotation_degrees", scalar(polygon.rotation_degrees())),
                ("sides", scalar(polygon.sides())),
                width,
                modifiers,
            ])
        }
    })
}

/// 로컬 경계의 크기와 왼쪽 위 좌표(이름 오름차순).
fn bounds_fields(bounds: Rect) -> [(&'static str, YamlValue); 4] {
    [
        ("height", scalar(bounds.size().height())),
        ("origin-x", scalar(bounds.origin().x())),
        ("origin-y", scalar(bounds.origin().y())),
        ("width", scalar(bounds.size().width())),
    ]
}

fn parse_modifier(entry: &YamlValue) -> Result<ModifierEntry, TextSourceError> {
    const COMMON: [&str; 4] = ["id", "order_key", "enabled", "kind"];
    let with = |params: &[&'static str]| -> Vec<&'static str> {
        COMMON
            .iter()
            .copied()
            .chain(params.iter().copied())
            .collect()
    };
    let payload = match text(field(entry, "kind")?)? {
        "transform" => {
            fields(entry, &with(&["matrix"]))?;
            let [a, b, c, d, tx, ty] = parse_floats(field(entry, "matrix")?)?;
            ModifierPayload::Transform(Transform2D::new(a, b, c, d, tx, ty).map_err(invalid)?)
        }
        "fill" => {
            fields(entry, &with(&["color"]))?;
            ModifierPayload::Fill {
                color: parse_color_line(field(entry, "color")?)?,
            }
        }
        "stroke" => {
            fields(entry, &with(&["color", "width"]))?;
            ModifierPayload::Stroke {
                color: parse_color_line(field(entry, "color")?)?,
                width: FiniteF32::new(finite(field(entry, "width")?)?).map_err(invalid)?,
            }
        }
        kind => {
            return Err(TextSourceError::Unsupported(format!(
                "Shape modifier: {kind}"
            )))
        }
    };
    Ok(ModifierEntry {
        id: ModifierId::new(parse(field(entry, "id")?)?),
        order: parse_order_key(field(entry, "order_key")?)?,
        enabled: parse(field(entry, "enabled")?)?,
        kind: payload.kind(),
        payload,
    })
}

pub(super) fn decode(
    node: &YamlValue,
    kind: ShapeKind,
    stacks: &mut Vec<ModifierStack>,
) -> Result<NodeRecord, TextSourceError> {
    let value = field(node, "payload")?;
    let length = |key| finite(field(value, key)?);
    let size = || Size2::new(length("width")?, length("height")?).map_err(invalid);
    let point = |x, y| -> Result<Vec2, TextSourceError> {
        Vec2::new(length(x)?, length(y)?).map_err(invalid)
    };
    let bounds = || -> Result<Rect, TextSourceError> {
        Ok(Rect::new(point("origin-x", "origin-y")?, size()?))
    };
    let geometry = match kind {
        ShapeKind::Rectangle => {
            fields(
                value,
                &[
                    "border-bottom-left-radius",
                    "border-bottom-right-radius",
                    "border-top-left-radius",
                    "border-top-right-radius",
                    "height",
                    "width",
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
            fields(
                value,
                &["height", "origin-x", "origin-y", "width", "modifiers"],
            )?;
            ShapeGeometry::Ellipse(EllipseParams::new(bounds()?).map_err(invalid)?)
        }
        ShapeKind::Line => {
            fields(
                value,
                &["cap", "end-x", "end-y", "start-x", "start-y", "modifiers"],
            )?;
            ShapeGeometry::Line(LineParams {
                start: point("start-x", "start-y")?,
                end: point("end-x", "end-y")?,
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
                    "corner_radius",
                    "height",
                    "origin-x",
                    "origin-y",
                    "rotation_degrees",
                    "sides",
                    "width",
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
        stack.insert(parse_modifier(entry)?).map_err(invalid)?;
    }
    if !entries.is_empty() {
        stacks.push(stack);
    }
    Ok(record)
}
