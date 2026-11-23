//! modifier stack → CSS 고정 적용 순서 선형화 계획 (IX.11.18, 속성 모델 3장·9.1절·9.2절).
//!
//! 파일을 쓰지 않는 core 계획 함수입니다. [`render_plan`]이 계획을 읽어 HTML 요소 구조(wrapper·node·overlay·clip
//! wrapper와 선언)로 냅니다. 그 DOM 배치는 기준 browser로 잰 결과 fixture로 확인합니다(ADR-0121 결정 8). 문서 node를
//! 이 계획으로 잇는 Web 파일 출력 연결은 XXII.2 몫입니다.
//!
//! - CSS는 한 요소에 `transform` → paint(`box-shadow`, background, border, 내용) → `filter` 목록 → `clip-path` →
//!   `mask` → `opacity` → `mix-blend-mode` 순서로 효과를 적용합니다.
//! - 활성 modifier를 stack 순서로 읽어, 이 순서를 거스르지 않는 가장 긴 앞 구간을 node 요소에 담고, 거스르는
//!   지점마다 바깥 wrapper를 하나씩 만듭니다. wrapper가 생기면 `approximated`(구조 변경)로 진단합니다.
//! - 이어지는 Shadow·Blur는 filter 목록 하나로 합칩니다. Frame Shadow는 앞에 filter 계열이 없을 때만 `box-shadow`입니다.
//! - Boolean·StrokeExpand·Corner(`stellar-only`)가 있으면 마지막 그런 modifier까지 path로 굽습니다. Frame은 굽지 못해
//!   그 node subtree를 래스터화합니다(XX.4.1).
//! - Frame 칠하기 층과 clip은 9.2절 규칙으로 overlay·clip wrapper를 만듭니다.
//! - 모든 요소가 같은 층의 stacking context를 만듭니다(3장, 14장 E3).
//!
//! 문서 crate에 의존하지 않으므로 입력은 지역 타입([`PlanInput`])입니다. 같은 입력은 언제나 같은 계획입니다.

use crate::text::num;

/// 계획에 쓰는 modifier 종류.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PlanModifier {
    /// Transform modifier(node border box 좌상단 원점 기준 행렬 `a b c d e f`).
    Transform([f32; 6]),
    /// Boolean(path 굽기).
    Boolean,
    /// StrokeExpand(path 굽기).
    StrokeExpand,
    /// Corner(path 굽기).
    Corner,
    /// Mask.
    Mask,
    /// Clip.
    Clip,
    /// Fill.
    Fill,
    /// Stroke.
    Stroke,
    /// Shadow.
    Shadow,
    /// Blur.
    Blur,
    /// Blend.
    Blend,
    /// Opacity.
    Opacity,
}

impl PlanModifier {
    fn is_stellar_only(self) -> bool {
        matches!(
            self,
            PlanModifier::Boolean | PlanModifier::StrokeExpand | PlanModifier::Corner
        )
    }
}

/// node 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanNodeKind {
    /// Frame.
    Frame,
    /// Shape·Path(SVG 기하).
    Shape,
    /// Text.
    Text,
    /// Image.
    Image,
    /// ComponentInstance.
    Instance,
}

/// 계획 입력.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanInput {
    /// node 종류.
    pub kind: PlanNodeKind,
    /// stack 순서의 (modifier, 활성).
    pub modifiers: Vec<(PlanModifier, bool)>,
    /// node가 `position: absolute`로 나가는지(Free 자식).
    pub absolute: bool,
    /// `transform-origin`의 x·y CSS 값 텍스트(예: `50%`, `0px`).
    pub transform_origin: [String; 2],
    /// Frame 테두리 두께(4변 같음, 없으면 0).
    pub frame_border: f32,
    /// Frame `border-*-radius` 4값(좌상, 우상, 우하, 좌하).
    pub frame_radius: [f32; 4],
    /// Frame 배경이 있는지.
    pub frame_background: bool,
    /// Frame clip이 Path인지.
    pub frame_path_clip: bool,
}

/// CSS 적용 단계(작을수록 먼저).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Stage {
    /// `transform`.
    Transform,
    /// paint(`box-shadow`, background, border, 내용).
    Paint,
    /// `filter` 목록.
    Filter,
    /// `clip-path`.
    ClipPath,
    /// `mask`.
    Mask,
    /// `opacity`.
    Opacity,
    /// `mix-blend-mode`.
    Blend,
}

/// 요소의 역할.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElementRole {
    /// node 자신의 요소(상자·item longhand).
    Node,
    /// 9.1절 3번: 순서를 거스르는 지점의 바깥 wrapper.
    Wrapper,
    /// 9.2절 1번: border 위 Fill·Stroke를 칠하는 첫 자식 overlay.
    Overlay,
    /// 9.2절 2번: container longhand와 자식을 갖고 `clip-path`를 쓰는 안쪽 wrapper.
    ClipWrapper,
}

/// 요소 하나와 그 선언.
#[derive(Debug, Clone, PartialEq)]
pub struct PlannedElement {
    /// 역할.
    pub role: ElementRole,
    /// 이 요소에 담은 modifier의 stack index.
    pub modifiers: Vec<usize>,
    /// CSS 선언(이름, 값). 값의 세부(색·반경 등)는 출력기가 채우는 자리 표시(`<…>`)를 둘 수 있습니다.
    pub declarations: Vec<(&'static str, String)>,
}

/// 계획 진단.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanDiagnostic {
    /// code.
    pub code: &'static str,
    /// 설명.
    pub message: String,
}

/// 선형화 계획.
#[derive(Debug, Clone, PartialEq)]
pub struct ModifierPlan {
    /// 바깥 → 안 순서의 요소(마지막 wrapper가 맨 앞, node 요소 다음에 overlay·clip wrapper).
    pub elements: Vec<PlannedElement>,
    /// path로 굽는 마지막 stack index(그 index까지 굽기 결과에 포함).
    pub bake_through: Option<usize>,
    /// 래스터화해야 하는지(굽지 못하는 node의 stellar-only modifier).
    pub rasterize: bool,
    /// 진단(`approximated` 구조 변경 등).
    pub diagnostics: Vec<PlanDiagnostic>,
}

fn stage(modifier: PlanModifier, kind: PlanNodeKind, filter_seen: bool) -> Stage {
    match modifier {
        PlanModifier::Transform(_) => Stage::Transform,
        PlanModifier::Fill | PlanModifier::Stroke => Stage::Paint,
        PlanModifier::Shadow if kind == PlanNodeKind::Frame && !filter_seen => Stage::Paint,
        PlanModifier::Shadow | PlanModifier::Blur => Stage::Filter,
        PlanModifier::Clip => Stage::ClipPath,
        PlanModifier::Mask => Stage::Mask,
        PlanModifier::Opacity => Stage::Opacity,
        PlanModifier::Blend => Stage::Blend,
        PlanModifier::Boolean | PlanModifier::StrokeExpand | PlanModifier::Corner => Stage::Paint,
    }
}

/// 한 요소에 두 번 둘 수 없는 단계(목록이 아닌 단일 값).
fn single(stage: Stage) -> bool {
    matches!(
        stage,
        Stage::ClipPath | Stage::Mask | Stage::Opacity | Stage::Blend
    )
}

fn is_zero(value: &str) -> bool {
    matches!(value, "0" | "0px" | "0%")
}

fn negate(origin: &str) -> String {
    match origin.strip_prefix('-') {
        Some(rest) => rest.to_string(),
        None if is_zero(origin) => origin.to_string(),
        None => format!("-{origin}"),
    }
}

/// Transform modifier 행렬을 `transform-origin` 반대 이동으로 감쌉니다(속성 모델 9장 Transform 행).
fn conjugated_transform(matrix: [f32; 6], origin: &[String; 2]) -> String {
    let m = matrix.map(num).join(", ");
    let identity = origin.iter().all(|o| is_zero(o));
    if identity {
        format!("matrix({m})")
    } else {
        format!(
            "translate({}, {}) matrix({m}) translate({}, {})",
            negate(&origin[0]),
            negate(&origin[1]),
            origin[0],
            origin[1]
        )
    }
}

fn declarations_for(
    input: &PlanInput,
    indices: &[usize],
    overlay: bool,
) -> Vec<(&'static str, String)> {
    let mut transforms = Vec::new();
    let mut filters = Vec::new();
    let mut out = Vec::new();
    let mut filter_seen = false;
    for &i in indices {
        let modifier = input.modifiers[i].0;
        match modifier {
            PlanModifier::Transform(matrix) => {
                transforms.push(conjugated_transform(matrix, &input.transform_origin))
            }
            PlanModifier::Fill if overlay || input.kind != PlanNodeKind::Frame => {
                out.push(("background-color", "<fill>".to_string()))
            }
            PlanModifier::Fill => {
                out.push(("background-image", "linear-gradient(<fill>, <fill>)".into()))
            }
            PlanModifier::Stroke if input.kind == PlanNodeKind::Frame => {
                out.push(("box-shadow", "inset 0 0 0 <stroke-width> <stroke>".into()))
            }
            PlanModifier::Stroke => {
                out.push(("stroke", "<stroke>".into()));
                out.push(("stroke-width", "<stroke-width>".into()));
            }
            PlanModifier::Shadow if input.kind == PlanNodeKind::Frame && !filter_seen => {
                out.push(("box-shadow", "<x> <y> <blur> <shadow>".into()))
            }
            PlanModifier::Shadow => {
                filter_seen = true;
                filters.push("drop-shadow(<x> <y> <blur> <shadow>)".to_string());
            }
            PlanModifier::Blur => {
                filter_seen = true;
                filters.push("blur(<blur>)".to_string());
            }
            PlanModifier::Clip => out.push(("clip-path", "url(#<clip>)".into())),
            PlanModifier::Mask => out.push(("mask-image", "url(#<mask>)".into())),
            PlanModifier::Opacity => out.push(("opacity", "<opacity>".into())),
            PlanModifier::Blend => {
                out.push(("mix-blend-mode", "<blend>".into()));
                out.push(("isolation", "isolate".into()));
            }
            PlanModifier::Boolean | PlanModifier::StrokeExpand | PlanModifier::Corner => {}
        }
    }
    let mut head = Vec::new();
    if !transforms.is_empty() {
        head.push(("transform", transforms.join(" ")));
    }
    if !filters.is_empty() {
        out.push(("filter", filters.join(" ")));
    }
    head.extend(out);
    head
}

fn stacking(absolute: bool) -> Vec<(&'static str, String)> {
    if absolute {
        vec![("z-index", "0".into())]
    } else {
        vec![("position", "relative".into()), ("z-index", "0".into())]
    }
}

/// stack을 CSS 고정 순서로 선형화합니다.
pub fn plan_modifiers(input: &PlanInput) -> ModifierPlan {
    let mut diagnostics = Vec::new();
    let active: Vec<usize> = (0..input.modifiers.len())
        .filter(|&i| input.modifiers[i].1)
        .collect();
    // 4번: stellar-only까지 굽기.
    let bake_through = active
        .iter()
        .copied()
        .filter(|&i| input.modifiers[i].0.is_stellar_only())
        .max();
    let mut rasterize = false;
    if let Some(last) = bake_through {
        if input.kind == PlanNodeKind::Shape {
            diagnostics.push(PlanDiagnostic {
                code: "approximated-path-bake",
                message: format!("stack index {last}까지를 path로 굽습니다"),
            });
        } else {
            rasterize = true;
            diagnostics.push(PlanDiagnostic {
                code: "approximated-rasterize",
                message: format!(
                    "path로 굽지 못하는 node라 stack index {last}까지를 래스터화합니다"
                ),
            });
        }
    }
    let remaining: Vec<usize> = active
        .into_iter()
        .filter(|&i| bake_through.is_none_or(|b| i > b))
        .collect();
    // 1·3번: 단조 구간으로 나눕니다.
    let mut segments: Vec<Vec<usize>> = vec![Vec::new()];
    let mut last_stage: Option<Stage> = None;
    let mut filter_seen = false;
    for &i in &remaining {
        let s = stage(input.modifiers[i].0, input.kind, filter_seen);
        let breaks = match last_stage {
            Some(prev) => s < prev || (s == prev && single(s)),
            None => false,
        };
        if breaks {
            segments.push(Vec::new());
            filter_seen = false;
        }
        let s = stage(input.modifiers[i].0, input.kind, filter_seen);
        if s == Stage::Filter {
            filter_seen = true;
        }
        last_stage = Some(s);
        segments.last_mut().expect("구간").push(i);
    }
    if segments.len() > 1 {
        diagnostics.push(PlanDiagnostic {
            code: "approximated-structure",
            message: format!(
                "modifier 순서가 CSS 고정 순서를 거슬러 wrapper {}개를 만듭니다",
                segments.len() - 1
            ),
        });
    }
    let first = segments.remove(0);
    let mut elements = Vec::new();
    // 바깥 wrapper(마지막 구간이 가장 바깥).
    for segment in segments.iter().rev() {
        let mut declarations = stacking(false);
        declarations.extend(declarations_for(input, segment, false));
        elements.push(PlannedElement {
            role: ElementRole::Wrapper,
            modifiers: segment.clone(),
            declarations,
        });
    }
    // node 요소와 9.2절 분리.
    let frame = input.kind == PlanNodeKind::Frame;
    let paint_on_border: Vec<usize> = first
        .iter()
        .copied()
        .filter(|&i| {
            matches!(
                input.modifiers[i].0,
                PlanModifier::Fill | PlanModifier::Stroke
            )
        })
        .collect();
    let overlay = frame && input.frame_border > 0.0 && !paint_on_border.is_empty();
    let node_modifiers: Vec<usize> = first
        .iter()
        .copied()
        .filter(|i| !overlay || !paint_on_border.contains(i))
        .collect();
    let mut node_declarations = stacking(input.absolute);
    node_declarations.extend(declarations_for(input, &node_modifiers, false));
    let self_paint = input.frame_background
        || input.frame_border > 0.0
        || first.iter().any(|&i| {
            matches!(
                input.modifiers[i].0,
                PlanModifier::Fill | PlanModifier::Stroke | PlanModifier::Shadow
            )
        });
    let clip_wrapper = frame && input.frame_path_clip && self_paint;
    elements.push(PlannedElement {
        role: ElementRole::Node,
        modifiers: node_modifiers,
        declarations: node_declarations,
    });
    if overlay {
        let inset = format!("-{}", crate::text::num(input.frame_border));
        let mut declarations = vec![
            ("position", "absolute".to_string()),
            ("z-index", "0".to_string()),
            ("inset", format!("{inset}px")),
        ];
        let [tl, tr, br, bl] = input.frame_radius.map(|r| format!("{}px", num(r)));
        declarations.push(("border-radius", format!("{tl} {tr} {br} {bl}")));
        declarations.extend(declarations_for(input, &paint_on_border, true));
        elements.push(PlannedElement {
            role: ElementRole::Overlay,
            modifiers: paint_on_border,
            declarations,
        });
        diagnostics.push(PlanDiagnostic {
            code: "approximated-structure",
            message: "border가 있는 Frame의 Fill·Stroke를 overlay 요소에 칠합니다".into(),
        });
    }
    if clip_wrapper {
        elements.push(PlannedElement {
            role: ElementRole::ClipWrapper,
            modifiers: Vec::new(),
            declarations: vec![
                ("position", "relative".into()),
                ("z-index", "0".into()),
                ("clip-path", "url(#<frame-clip>)".into()),
            ],
        });
        diagnostics.push(PlanDiagnostic {
            code: "approximated-structure",
            message: "자기 칠하기가 있는 Path clip Frame을 clip wrapper로 나눕니다".into(),
        });
    }
    if remaining
        .iter()
        .any(|&i| input.modifiers[i].0 == PlanModifier::Blend)
    {
        diagnostics.push(PlanDiagnostic {
            code: "blend-scope",
            message: "모든 요소가 stacking context를 만들어 Blend 합성 범위가 부모 stacking context 안으로 제한됩니다"
                .into(),
        });
    }
    ModifierPlan {
        elements,
        bake_through,
        rasterize,
        diagnostics,
    }
}

/// 계획 출력 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanRenderError {
    /// 계획이 래스터화를 요구합니다(요소 구조로 낼 수 없음, XX.4.1).
    Rasterize,
    /// 자리 표시(`<이름>`)의 값이 없습니다.
    MissingValue(String),
}

impl std::fmt::Display for PlanRenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlanRenderError::Rasterize => f.write_str("래스터화가 필요한 계획입니다"),
            PlanRenderError::MissingValue(name) => write!(f, "자리 표시 <{name}>의 값이 없습니다"),
        }
    }
}

impl std::error::Error for PlanRenderError {}

/// 출력한 요소 위치.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanSlots {
    /// 가장 바깥 요소(부모에 붙인 요소).
    pub outer: u32,
    /// node 자신의 요소.
    pub node: u32,
    /// node 내용·자식을 붙일 요소(clip wrapper가 있으면 그것, 없으면 node 요소).
    pub content: u32,
}

/// 자리 표시(`<이름>`)를 값으로 바꿉니다.
fn resolve(
    value: &str,
    values: &dyn Fn(&str) -> Option<String>,
) -> Result<String, PlanRenderError> {
    let mut out = String::new();
    let mut rest = value;
    while let Some(start) = rest.find('<') {
        let Some(end) = rest[start..].find('>') else {
            break;
        };
        let name = &rest[start + 1..start + end];
        out.push_str(&rest[..start]);
        out.push_str(&values(name).ok_or_else(|| PlanRenderError::MissingValue(name.to_string()))?);
        rest = &rest[start + end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

fn style_text(
    declarations: &[(&'static str, String)],
    values: &dyn Fn(&str) -> Option<String>,
) -> Result<String, PlanRenderError> {
    let mut parts = Vec::with_capacity(declarations.len());
    for (name, value) in declarations {
        parts.push(format!("{name}: {};", resolve(value, values)?));
    }
    Ok(parts.join(" "))
}

/// 계획을 HTML 요소 구조로 냅니다(IX.11.18 계획 출력기, ADR-0121 결정 8).
///
/// 바깥 wrapper(마지막 구간이 가장 바깥) → node 요소(`tag`) → 9.2절 overlay(node의 첫 자식) → clip wrapper(node의
/// 자식, 내용은 그 안) 순서로 `document`에 요소를 만들고 `parent`(없으면 root)에 붙입니다. 요소 선언은 계획의 선언
/// 그대로이고, node 요소에는 그 앞에 `node_declarations`(상자·layout longhand)를 둡니다. 선언 값의 자리 표시
/// (`<fill>` 등)는 `values`가 채우며 값이 없으면 오류입니다. 같은 계획과 입력은 같은 요소 구조입니다.
pub fn render_plan(
    plan: &ModifierPlan,
    document: &mut crate::web::html::HtmlDocument,
    parent: Option<u32>,
    tag: &'static str,
    node_declarations: &[(&'static str, String)],
    values: &dyn Fn(&str) -> Option<String>,
) -> Result<PlanSlots, PlanRenderError> {
    if plan.rasterize {
        return Err(PlanRenderError::Rasterize);
    }
    let mut attach = parent;
    let mut outer = None;
    let mut node = None;
    let mut content = None;
    for element in &plan.elements {
        let (element_tag, declarations) = match element.role {
            ElementRole::Node => {
                let mut all = node_declarations.to_vec();
                all.extend(element.declarations.iter().cloned());
                (tag, all)
            }
            _ => ("div", element.declarations.clone()),
        };
        let index = document.add(element_tag, None);
        document.set_attr(index, "style", style_text(&declarations, values)?);
        match element.role {
            ElementRole::Wrapper => {}
            ElementRole::Node => {
                node = Some(index);
                content = Some(index);
            }
            ElementRole::Overlay => {}
            ElementRole::ClipWrapper => content = Some(index),
        }
        match attach {
            Some(parent) => document.append(parent, index),
            None => document.push_root(index),
        }
        outer.get_or_insert(index);
        // wrapper는 다음 요소를 감싸고, overlay·clip wrapper는 node의 자식입니다.
        attach = match element.role {
            ElementRole::Wrapper | ElementRole::Node => Some(index),
            ElementRole::Overlay | ElementRole::ClipWrapper => node,
        };
    }
    let node = node.expect("계획에는 node 요소가 하나 있음");
    Ok(PlanSlots {
        outer: outer.unwrap_or(node),
        node,
        content: content.unwrap_or(node),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(kind: PlanNodeKind, modifiers: &[PlanModifier]) -> PlanInput {
        PlanInput {
            kind,
            modifiers: modifiers.iter().map(|m| (*m, true)).collect(),
            absolute: false,
            transform_origin: ["0px".into(), "0px".into()],
            frame_border: 0.0,
            frame_radius: [0.0; 4],
            frame_background: false,
            frame_path_clip: false,
        }
    }

    fn roles(plan: &ModifierPlan) -> Vec<ElementRole> {
        plan.elements.iter().map(|e| e.role).collect()
    }

    fn names(element: &PlannedElement) -> Vec<&'static str> {
        element.declarations.iter().map(|(n, _)| *n).collect()
    }

    #[test]
    fn css_order_stack_fits_one_element() {
        use PlanModifier as M;
        let plan = plan_modifiers(&input(
            PlanNodeKind::Shape,
            &[M::Fill, M::Blur, M::Shadow, M::Clip, M::Opacity, M::Blend],
        ));
        assert_eq!(roles(&plan), [ElementRole::Node]);
        assert_eq!(
            names(&plan.elements[0]),
            [
                "position",
                "z-index",
                "background-color",
                "clip-path",
                "opacity",
                "mix-blend-mode",
                "isolation",
                "filter"
            ]
        );
        let filter = &plan.elements[0].declarations.last().unwrap().1;
        assert_eq!(
            filter, "blur(<blur>) drop-shadow(<x> <y> <blur> <shadow>)",
            "Blur 뒤 Shadow는 drop-shadow"
        );
        assert!(plan
            .diagnostics
            .iter()
            .all(|d| d.code != "approximated-structure"));
        assert!(plan.diagnostics.iter().any(|d| d.code == "blend-scope"));
    }

    #[test]
    fn order_violations_create_wrappers_and_same_stack_same_structure() {
        use PlanModifier as M;
        // Opacity 뒤 Fill은 순서를 거스릅니다. [Fill, Opacity]는 CSS 순서라 wrapper 하나에, 다음 Opacity는 또 바깥에.
        let stack = input(
            PlanNodeKind::Shape,
            &[M::Opacity, M::Fill, M::Opacity, M::Opacity],
        );
        let plan = plan_modifiers(&stack);
        assert_eq!(
            roles(&plan),
            [
                ElementRole::Wrapper,
                ElementRole::Wrapper,
                ElementRole::Node
            ]
        );
        assert_eq!(plan.elements[2].modifiers, [0]);
        assert_eq!(plan.elements[1].modifiers, [1, 2]);
        assert_eq!(plan.elements[0].modifiers, [3], "마지막 구간이 가장 바깥");
        assert!(plan
            .diagnostics
            .iter()
            .any(|d| d.code == "approximated-structure"));
        assert_eq!(plan_modifiers(&stack), plan, "같은 stack 같은 구조");
        // 꺼진 modifier는 출력하지 않습니다.
        let mut off = stack.clone();
        off.modifiers[1].1 = false;
        off.modifiers[2].1 = false;
        off.modifiers[3].1 = false;
        assert_eq!(roles(&plan_modifiers(&off)), [ElementRole::Node]);
    }

    #[test]
    fn frame_shadow_is_box_shadow_only_before_filters() {
        use PlanModifier as M;
        let plan = plan_modifiers(&input(PlanNodeKind::Frame, &[M::Shadow, M::Blur]));
        assert!(names(&plan.elements[0]).contains(&"box-shadow"));
        let plan = plan_modifiers(&input(PlanNodeKind::Frame, &[M::Blur, M::Shadow]));
        assert_eq!(
            plan.elements[0].declarations.last().unwrap().1,
            "blur(<blur>) drop-shadow(<x> <y> <blur> <shadow>)"
        );
    }

    #[test]
    fn stellar_only_modifiers_bake_paths_or_rasterize() {
        use PlanModifier as M;
        let plan = plan_modifiers(&input(
            PlanNodeKind::Shape,
            &[M::Fill, M::Boolean, M::Corner, M::Opacity],
        ));
        assert_eq!(plan.bake_through, Some(2));
        assert!(!plan.rasterize);
        assert_eq!(plan.elements[0].modifiers, [3], "굽기 뒤 구간만 CSS");
        let frame = plan_modifiers(&input(PlanNodeKind::Frame, &[M::StrokeExpand]));
        assert!(frame.rasterize);
    }

    #[test]
    fn free_frame_and_flow_sibling_stacking_declarations() {
        let mut free = input(PlanNodeKind::Frame, &[]);
        free.absolute = true;
        assert_eq!(
            plan_modifiers(&free).elements[0].declarations,
            [("z-index", "0".to_string())]
        );
        let flow = input(PlanNodeKind::Text, &[]);
        assert_eq!(
            plan_modifiers(&flow).elements[0].declarations,
            [
                ("position", "relative".to_string()),
                ("z-index", "0".to_string())
            ]
        );
    }

    #[test]
    fn bordered_frame_paints_fill_and_stroke_on_an_overlay_with_the_same_radius() {
        use PlanModifier as M;
        let mut frame = input(PlanNodeKind::Frame, &[M::Fill, M::Stroke]);
        frame.frame_border = 2.0;
        frame.frame_radius = [4.0, 4.0, 8.0, 8.0];
        let plan = plan_modifiers(&frame);
        assert_eq!(roles(&plan), [ElementRole::Node, ElementRole::Overlay]);
        let overlay = &plan.elements[1];
        assert_eq!(overlay.modifiers, [0, 1]);
        assert!(overlay
            .declarations
            .contains(&("inset", "-2px".to_string())));
        assert!(overlay
            .declarations
            .contains(&("border-radius", "4px 4px 8px 8px".to_string())));
        assert!(overlay.declarations.contains(&(
            "box-shadow",
            "inset 0 0 0 <stroke-width> <stroke>".to_string()
        )));
        // border가 없으면 overlay가 없습니다.
        frame.frame_border = 0.0;
        assert_eq!(roles(&plan_modifiers(&frame)), [ElementRole::Node]);
    }

    #[test]
    fn path_clip_frame_with_own_paint_splits_a_clip_wrapper() {
        let mut frame = input(PlanNodeKind::Frame, &[]);
        frame.frame_path_clip = true;
        frame.frame_background = true;
        let plan = plan_modifiers(&frame);
        assert_eq!(roles(&plan), [ElementRole::Node, ElementRole::ClipWrapper]);
        frame.frame_background = false;
        assert_eq!(roles(&plan_modifiers(&frame)), [ElementRole::Node]);
    }

    #[test]
    fn transform_modifier_is_conjugated_by_transform_origin() {
        let mut node = input(
            PlanNodeKind::Shape,
            &[PlanModifier::Transform([0.0, 1.0, -1.0, 0.0, 10.0, 0.0])],
        );
        node.transform_origin = ["50%".into(), "50%".into()];
        let plan = plan_modifiers(&node);
        assert_eq!(
            plan.elements[0].declarations[2],
            (
                "transform",
                "translate(-50%, -50%) matrix(0, 1, -1, 0, 10, 0) translate(50%, 50%)".to_string()
            )
        );
        node.transform_origin = ["0px".into(), "0px".into()];
        assert_eq!(
            plan_modifiers(&node).elements[0].declarations[2].1,
            "matrix(0, 1, -1, 0, 10, 0)"
        );
    }

    fn values(name: &str) -> Option<String> {
        Some(
            match name {
                "fill" => "rgb(0 128 255)",
                "stroke" => "rgb(255 0 0)",
                "stroke-width" => "2px",
                "opacity" => "0.5",
                "frame-clip" => "frame-clip-a",
                _ => return None,
            }
            .to_string(),
        )
    }

    fn decl(pairs: &[(&'static str, &str)]) -> Vec<(&'static str, String)> {
        pairs.iter().map(|(n, v)| (*n, v.to_string())).collect()
    }

    /// DOM 배치 fixture 페이지: flex 컨테이너 안에 (A) border·Fill·Stroke가 있는 flex item이자 flex 컨테이너 Frame,
    /// (B) absolute Frame, (C) 뒤 흐름 형제. A의 overlay containing block과 B·C stacking을 브라우저로 잽니다.
    fn dom_page() -> String {
        use crate::web::html::HtmlDocument;
        use PlanModifier as M;
        let mut doc = HtmlDocument::new();
        let root = doc.add("div", None);
        doc.set_attr(root, "data-probe", "root");
        doc.set_attr(
            root,
            "style",
            "display: flex; align-items: flex-start; width: 320px; height: 200px; position: relative; z-index: 0;",
        );
        doc.push_root(root);
        let mut a = input(PlanNodeKind::Frame, &[M::Fill, M::Stroke]);
        a.frame_border = 4.0;
        a.frame_radius = [8.0, 8.0, 16.0, 16.0];
        let plan_a = plan_modifiers(&a);
        let slots = render_plan(
            &plan_a,
            &mut doc,
            Some(root),
            "div",
            &decl(&[
                ("display", "flex"),
                ("box-sizing", "border-box"),
                ("width", "120px"),
                ("height", "80px"),
                ("padding", "10px"),
                ("border", "4px solid rgb(0 0 0)"),
                ("border-radius", "8px 8px 16px 16px"),
            ]),
            &values,
        )
        .expect("A");
        doc.set_attr(slots.node, "data-probe", "frame-a");
        let overlay = doc.element(slots.node).children[0].clone();
        if let crate::web::html::HtmlChild::Element(overlay) = overlay {
            doc.set_attr(overlay, "data-probe", "overlay-a");
        }
        // 자식 Frame도 같은 규칙(stacking 선언)으로 냅니다.
        let child = render_plan(
            &plan_modifiers(&input(PlanNodeKind::Frame, &[])),
            &mut doc,
            Some(slots.content),
            "div",
            &decl(&[("width", "20px"), ("height", "20px")]),
            &values,
        )
        .expect("A 자식");
        doc.set_attr(child.node, "data-probe", "content-a");
        let mut b = input(PlanNodeKind::Frame, &[M::Fill]);
        b.absolute = true;
        let slots = render_plan(
            &plan_modifiers(&b),
            &mut doc,
            Some(root),
            "div",
            &decl(&[
                ("position", "absolute"),
                ("left", "150px"),
                ("top", "10px"),
                ("width", "40px"),
                ("height", "40px"),
            ]),
            &values,
        )
        .expect("B");
        doc.set_attr(slots.node, "data-probe", "frame-b");
        let c = plan_modifiers(&input(PlanNodeKind::Frame, &[M::Fill]));
        let slots = render_plan(
            &c,
            &mut doc,
            Some(root),
            "div",
            &decl(&[("width", "100px"), ("height", "60px")]),
            &values,
        )
        .expect("C");
        doc.set_attr(slots.node, "data-probe", "sibling-c");
        doc.to_html()
    }

    fn fnv(text: &str) -> u64 {
        text.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
        })
    }

    #[test]
    fn plan_renders_into_the_dom_structure_it_describes() {
        use PlanModifier as M;
        // wrapper → node → overlay·clip wrapper 순서와 자리 표시 치환.
        let mut frame = input(PlanNodeKind::Frame, &[M::Opacity, M::Fill, M::Stroke]);
        frame.frame_border = 2.0;
        frame.frame_path_clip = true;
        let plan = plan_modifiers(&frame);
        let mut doc = crate::web::html::HtmlDocument::new();
        let slots = render_plan(&plan, &mut doc, None, "section", &[], &values).expect("render");
        let html = doc.to_html();
        assert_eq!(doc.element(slots.outer).tag, "div");
        assert_eq!(doc.element(slots.node).tag, "section");
        assert_ne!(slots.content, slots.node, "clip wrapper가 내용 부모");
        // Opacity 뒤 Fill은 순서를 거슬러 바깥 wrapper가 칠합니다. node 요소는 opacity, 내용 부모는 clip wrapper입니다.
        assert!(html.starts_with("<div style=\"position: relative; z-index: 0; background-image: linear-gradient(rgb(0 128 255), rgb(0 128 255));"), "{html}");
        assert!(html.contains("<section style=\"position: relative; z-index: 0; opacity: 0.5;\">"));
        assert!(html.contains("clip-path: url(#frame-clip-a);"));
        assert!(!html.contains("<fill>") && !html.contains("&lt;fill&gt;"));
        let missing = plan_modifiers(&input(PlanNodeKind::Shape, &[M::Blur]));
        assert_eq!(
            render_plan(&missing, &mut doc, None, "div", &[], &values),
            Err(PlanRenderError::MissingValue("blur".into()))
        );
        let raster = plan_modifiers(&input(PlanNodeKind::Frame, &[M::Boolean]));
        assert_eq!(
            render_plan(&raster, &mut doc, None, "div", &[], &values),
            Err(PlanRenderError::Rasterize)
        );
    }

    #[test]
    fn rendered_plan_dom_placement_matches_the_browser_measurement() {
        // IX.11.18 남은 검증: 계획 출력기가 만든 DOM을 기준 browser(Chrome 157, headless, DPR 1)로 잰 결과를
        // 커밋했습니다(`tests/fixtures/modifier_plan_dom.expected.txt`, 생성 방법은 ADR-0121 결정 8). 페이지 bytes가 잰
        // 페이지와 같은지 hash로 확인하고, 잰 값으로 불변식을 검사합니다.
        let page = dom_page();
        crate::fixture::check_golden("modifier_plan_dom.html", &page);
        let expected = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/modifier_plan_dom.expected.txt"),
        )
        .expect("측정 결과");
        let mut lines = expected.lines();
        assert_eq!(
            lines.next(),
            Some(format!("page-fnv64 {:016x}", fnv(&page)).as_str()),
            "측정한 페이지와 지금 출력이 다릅니다. 측정을 다시 하세요"
        );
        let mut probes = std::collections::BTreeMap::new();
        let mut hits = std::collections::BTreeMap::new();
        for line in lines {
            let parts: Vec<&str> = line.split(' ').collect();
            match parts.as_slice() {
                ["hit", x, y, probe] => {
                    hits.insert(format!("{x},{y}"), *probe);
                }
                [probe, rest @ ..] => {
                    probes.insert(*probe, rest.join(" "));
                }
                [] => {}
            }
        }
        let field = |probe: &str, key: &str| -> String {
            probes[probe]
                .split(' ')
                .find_map(|kv| kv.strip_prefix(&format!("{key}=")))
                .unwrap_or_else(|| panic!("{probe} {key}"))
                .to_string()
        };
        // flex item인 flex 컨테이너 Frame의 overlay containing block은 그 Frame입니다: offsetParent가 Frame이고, overlay
        // 상자가 Frame border box와 같으며, 바깥 radius가 같습니다.
        assert_eq!(field("overlay-a", "offsetParent"), "frame-a");
        assert_eq!(field("overlay-a", "rect"), field("frame-a", "rect"));
        assert_eq!(field("overlay-a", "radius"), field("frame-a", "radius"));
        // 내용은 padding·border 안쪽이고, overlay는 내용보다 먼저(아래) 칠합니다(내용 위 hit test가 내용).
        assert_eq!(field("content-a", "rect"), "14,14,20,20");
        assert_eq!(hits.get("24,24"), Some(&"content-a"));
        // overlay는 border 위까지 덮습니다(border 위 hit test가 overlay).
        assert_eq!(hits.get("2,40"), Some(&"overlay-a"));
        // stacking: 모든 요소가 z-index 0 stacking context라 DOM 순서대로 그립니다. absolute B 위를 뒤 형제 C가 덮습니다.
        assert_eq!(field("frame-b", "zIndex"), "0");
        assert_eq!(field("sibling-c", "position"), "relative");
        assert_eq!(field("sibling-c", "zIndex"), "0");
        assert_eq!(field("sibling-c", "rect"), "120,0,100,60");
        assert_eq!(hits.get("170,30"), Some(&"sibling-c"));
    }
}
