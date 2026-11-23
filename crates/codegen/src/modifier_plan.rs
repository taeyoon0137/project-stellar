//! modifier stack → CSS 고정 적용 순서 선형화 계획 (IX.11.18, 속성 모델 3장·9.1절·9.2절).
//!
//! 파일을 쓰지 않는 core 계획 함수입니다. 실제 HTML·CSS 출력은 XXII.2가 이 계획을 읽어 합니다.
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
}
