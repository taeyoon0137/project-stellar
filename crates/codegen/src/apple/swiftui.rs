//! SwiftUI view tree: Control contract·layout·상태 (XXII.3.1·XXII.3.2, ADR-0107).
//!
//! 출력 view는 `struct <prefix>View: View` 하나입니다. 상태 변수는 `@State private var`, semantic action은 생성자에서
//! 받는 `onAction(<action StableId>)`입니다. 출력 node마다 `private var node<StableId 16진>: some View` 속성을 두어
//! 식 하나가 커지지 않게 합니다(type-check 시간). node에는 XXII.3.3 접근성 `ViewModifier`를 `.modifier(...)`로
//! 붙입니다.
//!
//! | Control contract | SwiftUI |
//! |---|---|
//! | Button | `Button { onAction(activate); 선택 상태 쓰기 } label: { 자식 layout }` + `.buttonStyle(.plain)`(디자인한 모양을 그대로 씀) |
//! | TextInput | `TextField(text: $상태, prompt: Text(verbatim: placeholder)) { … }` + `.textFieldStyle(.plain)` |
//! | Modal | 표시 상태가 IR에 없어 그 자리에 내용 container로 냄(노트) |
//! | List·SingleSelectionGroup | 자식 layout container(SwiftUI `List`·`Picker`는 플랫폼 행 모양과 scroll을 강제해 디자인과 달라짐). 의미는 접근성 modifier |
//!
//! layout 규칙은 XXII.1.4 matrix의 SwiftUI 열([`crate::matrix::dialect`])입니다.

use std::fmt::Write as _;

use stellar_accessibility::node::{ActionKind, ActionTarget};
use stellar_core_types::typed_value::TypedValue;
use stellar_layout::style::{
    ContentAlignment, FlexWrap, ItemAlignment, LayoutStyle, LengthPercentageAuto, MaxSize,
    Position, Size,
};
use stellar_target_profile::builtin::properties as p;
use stellar_target_profile::capability::{CapabilityStatus, CapabilitySubject};
use stellar_target_profile::spec::{ApplePlatform, AppleProfile};

use crate::apple::accessibility::{AccessibilityHook, StateNames, SwiftType};
use crate::apple::layout::{
    swiftui_alignment, swiftui_stack_alignment, swiftui_unit_point, LayoutFinding,
};
use crate::apple::native::{
    explicit_self, grow_is_basis_zero, is_stretch, item_edge, justify_alignment, justify_edge,
    self_alignment, size, style, track_kind, Analysis, Axis, Container, Control, Edge, TrackKind,
    ViewKind,
};
use crate::apple::swift::{number, string_literal};
use crate::apple::{AppleNote, AppleNoteKind, AppleNoteSubject, ControlNote};
use crate::facet::Facet;
use crate::ir::snapshot::id_hex;
use crate::ir::CodegenIr;

/// 생성기 문맥.
pub(crate) struct Generator<'a> {
    /// IR.
    pub ir: &'a CodegenIr,
    /// 구조 분석.
    pub analysis: &'a Analysis,
    /// 상태 이름.
    pub states: &'a StateNames,
    /// 접근성 hook.
    pub hooks: &'a [Option<AccessibilityHook>],
    /// 타입 접두어.
    pub prefix: &'a str,
    /// profile.
    pub apple: AppleProfile,
    /// 노트.
    pub notes: &'a mut Vec<AppleNote>,
}

/// 길이·수 literal(point).
fn px(value: f32) -> String {
    number(value)
}

/// `Grid`를 쓸 수 있는 최소 OS인지.
pub(crate) fn grid_available(apple: AppleProfile) -> bool {
    let major = apple.minimum_os.major();
    match apple.platform {
        ApplePlatform::Ios | ApplePlatform::IpadOs | ApplePlatform::TvOs => major >= 16,
        ApplePlatform::MacOs => major >= 13,
        ApplePlatform::WatchOs => major >= 9,
        ApplePlatform::VisionOs => major >= 1,
    }
}

/// 상태 초기값 Swift literal.
pub(crate) fn initial_literal(value: &TypedValue, ty: SwiftType) -> String {
    match (value, ty) {
        (TypedValue::String(text), _) => string_literal(text),
        (TypedValue::Boolean(flag), _) => flag.to_string(),
        (TypedValue::Number(n), _) => number(n.get()),
        (_, SwiftType::String) => "\"\"".to_string(),
        (_, SwiftType::Bool) => "false".to_string(),
        (_, SwiftType::Double) => "0".to_string(),
    }
}

/// grid의 track 묶음 정렬(`justify-content`·`align-content`).
pub(crate) fn content_edge(value: ContentAlignment) -> Edge {
    match value {
        ContentAlignment::Center => Edge::Center,
        ContentAlignment::End | ContentAlignment::FlexEnd => Edge::End,
        _ => Edge::Start,
    }
}

/// 자식 사이 공간 분배.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Spread {
    /// 없음.
    None,
    /// 사이만.
    Between,
    /// 양끝과 사이.
    Evenly,
}

/// 축 크기 출력.
#[derive(Debug, Clone, Copy, PartialEq)]
enum SizeOut {
    Natural,
    Fixed(f32),
    Fill,
    Ideal,
}

fn edge_pair(main: Axis, main_edge: Edge, cross_edge: Edge) -> (Edge, Edge) {
    match main {
        Axis::Horizontal => (main_edge, cross_edge),
        Axis::Vertical => (cross_edge, main_edge),
    }
}

impl Generator<'_> {
    fn note(&mut self, index: u32, kind: AppleNoteKind) {
        self.notes.push(AppleNote {
            subject: AppleNoteSubject::Node(self.ir.nodes[index as usize].id),
            kind,
        });
    }

    fn finding(&mut self, index: u32, finding: LayoutFinding) {
        self.note(index, AppleNoteKind::Layout(finding));
    }

    fn name(&self, index: u32) -> String {
        format!("node{}", id_hex(self.ir.nodes[index as usize].id))
    }

    fn style(&self, index: u32) -> &LayoutStyle {
        style(self.ir, index).expect("출력 node는 layout이 있음")
    }

    fn info(&self, index: u32) -> &crate::apple::native::NodeInfo {
        &self.analysis.nodes[index as usize]
    }

    /// node 내용의 정렬(frame alignment 성분: 가로, 세로).
    fn content_alignment(&self, index: u32) -> (Edge, Edge) {
        let s = self.style(index);
        match &self.info(index).container {
            Container::Flex { main, reverse } if self.has_children(index) => {
                let main_edge = justify_edge(s.justify_content, *reverse);
                let cross_edge = item_edge(s.align_items);
                edge_pair(*main, main_edge, cross_edge)
            }
            Container::Grid(_) if self.has_children(index) => (
                content_edge(s.justify_content),
                content_edge(s.align_content),
            ),
            _ => (Edge::Start, Edge::Start),
        }
    }

    fn has_children(&self, index: u32) -> bool {
        let info = self.info(index);
        !info.flow.is_empty()
    }

    /// flex 자식이 cross 축 slot frame을 쓰는지(채우지 않고 부모 stack 정렬과 다른 정렬).
    fn slot_edge(&self, parent: u32, child: u32) -> Option<Edge> {
        let Container::Flex { main, .. } = self.info(parent).container else {
            return None;
        };
        let cross = main.cross();
        if self.info(child).fill.axes[cross.index()] {
            return None;
        }
        let parent_style = self.style(parent);
        let child_style = self.style(child);
        let stack = item_edge(parent_style.align_items);
        let own = item_edge(self_alignment(parent_style, child_style));
        (own != stack).then_some(own)
    }

    /// container가 이상 크기를 써야 하는 축(확정이 아닌 축에서 자식을 채움).
    fn needs_fixed(&self, index: u32) -> [bool; 2] {
        let info = self.info(index);
        let s = self.style(index);
        let mut out = [false; 2];
        for axis in [Axis::Horizontal, Axis::Vertical] {
            if info.definite[axis.index()] || info.fill.axes[axis.index()] {
                continue;
            }
            if matches!(size(s, axis), Size::Length(_)) {
                continue;
            }
            out[axis.index()] = info.flow.iter().any(|child| {
                self.info(*child).fill.axes[axis.index()]
                    || self
                        .slot_edge(index, *child)
                        .is_some_and(|_| match info.container {
                            Container::Flex { main, .. } => main.cross() == axis,
                            _ => false,
                        })
            });
        }
        out
    }

    fn size_out(&mut self, index: u32, axis: Axis) -> SizeOut {
        let longhand = match axis {
            Axis::Horizontal => "width",
            Axis::Vertical => "height",
        };
        let info = self.info(index).clone();
        let s = self.style(index).clone();
        if info.fill.axes[axis.index()] {
            return SizeOut::Fill;
        }
        // grow 0의 flex-basis <length>는 main 크기입니다.
        if let Some(parent) = info.parent {
            if let Container::Flex { main, .. } = self.info(parent).container {
                if main == axis && s.flex_grow.get() == 0.0 {
                    match s.flex_basis {
                        stellar_layout::style::FlexBasis::Length(length) => {
                            return SizeOut::Fixed(length.get())
                        }
                        stellar_layout::style::FlexBasis::Percentage(_) => self.finding(
                            index,
                            matrix_finding(
                                "flex-basis",
                                "percent-size",
                                "SwiftUI에는 부모 기준 백분율 크기가 없어 자연 크기로 냅니다",
                            ),
                        ),
                        _ => {}
                    }
                }
            }
        }
        match size(&s, axis) {
            Size::Auto => SizeOut::Natural,
            Size::Length(length) => SizeOut::Fixed(length.get()),
            Size::MaxContent => SizeOut::Ideal,
            Size::FitContent => {
                self.finding(
                    index,
                    matrix_finding(
                        longhand,
                        "fit-content",
                        "fit-content는 SwiftUI 자연 크기(제안 크기 안의 이상 크기)로 냅니다",
                    ),
                );
                SizeOut::Natural
            }
            Size::MinContent => {
                self.finding(
                    index,
                    matrix_finding(
                        longhand,
                        "min-content",
                        "SwiftUI에는 min-content 크기가 없어 자연 크기로 냅니다",
                    ),
                );
                SizeOut::Natural
            }
            Size::Percentage(_) => {
                self.finding(
                    index,
                    matrix_finding(
                        longhand,
                        "percent-size",
                        "SwiftUI에는 부모 기준 백분율 크기가 없어 자연 크기로 냅니다",
                    ),
                );
                SizeOut::Natural
            }
        }
    }

    fn min_max(&mut self, index: u32) -> Vec<String> {
        let s = self.style(index).clone();
        let mut parts = Vec::new();
        for (axis, min_name, max_name) in [
            (Axis::Horizontal, "min-width", "max-width"),
            (Axis::Vertical, "min-height", "max-height"),
        ] {
            let (min, max) = match axis {
                Axis::Horizontal => (s.min_width, s.max_width),
                Axis::Vertical => (s.min_height, s.max_height),
            };
            let label = match axis {
                Axis::Horizontal => "Width",
                Axis::Vertical => "Height",
            };
            match min {
                Size::Auto => {}
                Size::Length(length) => parts.push(format!("min{label}: {}", px(length.get()))),
                _ => self.finding(
                    index,
                    matrix_finding(
                        min_name,
                        "intrinsic-limit",
                        "SwiftUI frame은 백분율·intrinsic 최소 크기를 받지 않아 내지 않습니다",
                    ),
                ),
            }
            match max {
                MaxSize::None => {}
                MaxSize::Length(length) => parts.push(format!("max{label}: {}", px(length.get()))),
                _ => self.finding(
                    index,
                    matrix_finding(
                        max_name,
                        "intrinsic-limit",
                        "SwiftUI frame은 백분율·intrinsic 최대 크기를 받지 않아 내지 않습니다",
                    ),
                ),
            }
        }
        if parts.is_empty() {
            return Vec::new();
        }
        let (h, v) = self.content_alignment(index);
        parts.push(format!("alignment: {}", swiftui_alignment(h, v)));
        vec![format!(".frame({})", parts.join(", "))]
    }

    /// node box의 modifier(내용 바로 뒤에 붙음).
    fn own_modifiers(&mut self, index: u32) -> Vec<String> {
        let s = self.style(index).clone();
        let (h, v) = self.content_alignment(index);
        let alignment = swiftui_alignment(h, v);
        let mut out = Vec::new();
        let width = self.size_out(index, Axis::Horizontal);
        let height = self.size_out(index, Axis::Vertical);
        let mut fixed_size = self.needs_fixed(index);
        if width == SizeOut::Ideal {
            fixed_size[0] = true;
        }
        if height == SizeOut::Ideal {
            fixed_size[1] = true;
        }
        if fixed_size[0] || fixed_size[1] {
            out.push(format!(
                ".fixedSize(horizontal: {}, vertical: {})",
                fixed_size[0], fixed_size[1]
            ));
        }
        if width == SizeOut::Fill || height == SizeOut::Fill {
            let mut parts = Vec::new();
            if width == SizeOut::Fill {
                parts.push("maxWidth: .infinity".to_string());
            }
            if height == SizeOut::Fill {
                parts.push("maxHeight: .infinity".to_string());
            }
            parts.push(format!("alignment: {alignment}"));
            out.push(format!(".frame({})", parts.join(", ")));
        }
        let mut fixed = Vec::new();
        if let SizeOut::Fixed(w) = width {
            fixed.push(format!("width: {}", px(w)));
        }
        if let SizeOut::Fixed(hgt) = height {
            fixed.push(format!("height: {}", px(hgt)));
        }
        let fixed_frame = (!fixed.is_empty()).then(|| {
            fixed.push(format!("alignment: {alignment}"));
            format!(".frame({})", fixed.join(", "))
        });
        let limits = self.min_max(index);
        let border_box = s.box_sizing == stellar_layout::style::BoxSizing::BorderBox;
        if !border_box {
            out.extend(fixed_frame.clone());
            out.extend(limits.clone());
        }
        let (top, right, bottom, left) = s.padding.components();
        if [top, right, bottom, left].iter().any(|v| *v != 0.0) {
            out.push(format!(
                ".padding(EdgeInsets(top: {}, leading: {}, bottom: {}, trailing: {}))",
                px(top),
                px(left),
                px(bottom),
                px(right)
            ));
        }
        if border_box {
            out.extend(fixed_frame);
            out.extend(limits);
        }
        if let Some(ratio) = s.aspect_ratio {
            self.finding(
                index,
                matrix_finding(
                    "aspect-ratio",
                    "aspect-ratio",
                    "SwiftUI aspectRatio(.fit)는 제안 크기 안에 맞추므로 CSS 비율 크기 해석과 다를 수 있습니다",
                ),
            );
            out.push(format!(
                ".aspectRatio({} / {}, contentMode: .fit)",
                px(ratio.width.get()),
                px(ratio.height.get())
            ));
        }
        let absolute = self.info(index).absolute.clone();
        for child in absolute {
            let a = self.absolute_alignment(child);
            out.push(format!(".overlay(alignment: {a}) {{"));
            out.push(format!("    {}", self.name(child)));
            out.push("}".to_string());
        }
        if let Some(layout) = &self.ir.nodes[index as usize].layout {
            let (a, b, c, d, tx, ty) = layout.transform.components();
            if (a, b, c, d) == (1.0, 0.0, 0.0, 1.0) {
                if tx != 0.0 || ty != 0.0 {
                    out.push(format!(".offset(x: {}, y: {})", px(tx), px(ty)));
                }
            } else {
                out.push(format!(
                    ".transformEffect(CGAffineTransform(a: {}, b: {}, c: {}, d: {}, tx: {}, ty: {}))",
                    px(a),
                    px(b),
                    px(c),
                    px(d),
                    px(tx),
                    px(ty)
                ));
            }
        }
        out
    }

    fn absolute_alignment(&self, child: u32) -> &'static str {
        let s = self.style(child);
        let horizontal = match (s.left, s.right) {
            (LengthPercentageAuto::Auto, LengthPercentageAuto::Auto) => Edge::Start,
            (LengthPercentageAuto::Auto, _) => Edge::End,
            _ => Edge::Start,
        };
        let vertical = match (s.top, s.bottom) {
            (LengthPercentageAuto::Auto, LengthPercentageAuto::Auto) => Edge::Start,
            (LengthPercentageAuto::Auto, _) => Edge::End,
            _ => Edge::Start,
        };
        swiftui_alignment(horizontal, vertical)
    }

    /// inset 한 축의 이동량(시작 inset 값, 없으면 끝 inset의 음수).
    fn inset_offset(
        &mut self,
        index: u32,
        start: (LengthPercentageAuto, &'static str),
        end: (LengthPercentageAuto, &'static str),
        absolute: bool,
        static_offset: f32,
    ) -> f32 {
        let percent =
            |value: LengthPercentageAuto| matches!(value, LengthPercentageAuto::Percentage(_));
        for (value, name) in [start, end] {
            if percent(value) {
                self.finding(
                    index,
                    matrix_finding(
                        name,
                        "percent-inset",
                        "SwiftUI offset은 부모 기준 백분율을 받지 않아 0으로 냅니다",
                    ),
                );
            }
        }
        match (start.0, end.0) {
            (LengthPercentageAuto::Length(a), LengthPercentageAuto::Length(_)) => {
                if absolute {
                    self.finding(
                        index,
                        matrix_finding(
                            "position",
                            "absolute-stretch",
                            "양쪽 inset으로 늘리는 absolute 크기는 낼 수 없어 시작 inset만 씁니다",
                        ),
                    );
                }
                a.get()
            }
            (LengthPercentageAuto::Length(a), _) => a.get(),
            (_, LengthPercentageAuto::Length(b)) => -b.get(),
            (LengthPercentageAuto::Auto, LengthPercentageAuto::Auto) if absolute => {
                self.finding(
                    index,
                    matrix_finding(
                        "position",
                        "static-position",
                        "inset이 모두 auto인 absolute box의 static position은 부모 content box 시작으로 냅니다",
                    ),
                );
                static_offset
            }
            _ => 0.0,
        }
    }

    /// 부모 기준 item modifier(own box 뒤).
    fn item_modifiers(&mut self, index: u32) -> Vec<String> {
        let mut out = Vec::new();
        let s = self.style(index).clone();
        let Some(parent) = self.info(index).parent else {
            return out;
        };
        let parent_style = self.style(parent).clone();
        let (ptop, _, _, pleft) = parent_style.padding.components();
        match s.position {
            Position::Relative | Position::Absolute => {
                let absolute = s.position == Position::Absolute;
                let x =
                    self.inset_offset(index, (s.left, "left"), (s.right, "right"), absolute, pleft);
                let y =
                    self.inset_offset(index, (s.top, "top"), (s.bottom, "bottom"), absolute, ptop);
                if x != 0.0 || y != 0.0 {
                    out.push(format!(".offset(x: {}, y: {})", px(x), px(y)));
                }
            }
            Position::Static => {}
        }
        if s.position == Position::Absolute {
            return out;
        }
        match self.info(parent).container.clone() {
            Container::Flex { main, .. } => {
                let cross = main.cross();
                if let Some(edge) = self.slot_edge(parent, index) {
                    let max = match cross {
                        Axis::Horizontal => "maxWidth",
                        Axis::Vertical => "maxHeight",
                    };
                    let (h, v) = edge_pair(main, Edge::Start, edge);
                    out.push(format!(
                        ".frame({max}: .infinity, alignment: {})",
                        swiftui_alignment(h, v)
                    ));
                }
                let own = self_alignment(&parent_style, &s);
                if own == ItemAlignment::Baseline
                    && main == Axis::Vertical
                    && explicit_self(s.align_self).is_some()
                {
                    self.finding(
                        index,
                        matrix_finding(
                            "align-self",
                            "column-baseline",
                            "column flex의 baseline 정렬은 SwiftUI에 없어 시작 정렬로 냅니다",
                        ),
                    );
                }
                if self.info(index).fill.axes[main.index()] {
                    let siblings = self.info(parent).flow.clone();
                    let growing: Vec<&LayoutStyle> = siblings
                        .iter()
                        .map(|c| self.style(*c))
                        .filter(|c| c.flex_grow.get() > 0.0)
                        .collect();
                    // 성장 item이 하나면 남은 공간을 모두 받으므로 분배 차이가 없습니다.
                    let equal = growing.len() <= 1
                        || growing
                            .iter()
                            .all(|c| c.flex_grow == s.flex_grow && grow_is_basis_zero(c, main));
                    if !equal {
                        self.finding(
                            index,
                            matrix_finding(
                                "flex-grow",
                                "grow-distribution",
                                "SwiftUI는 남은 공간을 채우는 형제에게 콘텐츠 크기와 무관하게 균등 분배합니다",
                            ),
                        );
                    }
                }
                let shrink = s.flex_shrink.get();
                if shrink == 0.0 {
                    let (h, v) = match main {
                        Axis::Horizontal => (true, false),
                        Axis::Vertical => (false, true),
                    };
                    out.push(format!(".fixedSize(horizontal: {h}, vertical: {v})"));
                } else if shrink != 1.0 {
                    self.finding(
                        index,
                        matrix_finding(
                            "flex-shrink",
                            "shrink-ratio",
                            "SwiftUI 압축은 비율을 받지 않아 flex-shrink 1처럼 냅니다",
                        ),
                    );
                }
            }
            Container::Grid(_) => {
                let horizontal = justify_alignment(&parent_style, &s);
                let vertical = self_alignment(&parent_style, &s);
                let default = (
                    item_edge(parent_style.justify_items),
                    item_edge(parent_style.align_items),
                );
                let own = (item_edge(horizontal), item_edge(vertical));
                if own != default && !(is_stretch(horizontal) && is_stretch(vertical)) {
                    out.push(format!(
                        ".gridCellAnchor({})",
                        swiftui_unit_point(own.0, own.1)
                    ));
                }
            }
            Container::Block => {}
        }
        out
    }

    fn empty() -> Vec<String> {
        vec![
            "Color.clear".to_string(),
            ".frame(width: 0, height: 0)".to_string(),
        ]
    }

    /// 자식 목록 줄(flex spacer 포함).
    ///
    /// stack `spacing`은 Spacer 양옆에도 붙으므로, spread가 있으면 stack spacing을 0으로 두고 gap을 사이 Spacer의
    /// `minLength`로 냅니다. space-between은 이렇게 하면 CSS와 같습니다(사이 간격 = gap + 남은 공간 몫).
    fn child_lines(&self, children: &[u32], spread: Spread, gap: f32) -> Vec<String> {
        let mut lines = Vec::new();
        let edge = "Spacer(minLength: 0)".to_string();
        let between = format!("Spacer(minLength: {})", px(gap));
        if spread == Spread::Evenly {
            lines.push(edge.clone());
        }
        for (position, child) in children.iter().enumerate() {
            if position > 0 && spread != Spread::None {
                lines.push(between.clone());
            }
            lines.push(self.name(*child));
        }
        if spread == Spread::Evenly && !children.is_empty() {
            lines.push(edge);
        }
        lines
    }

    fn block(head: String, body: Vec<String>) -> Vec<String> {
        let mut out = vec![format!("{head} {{")];
        out.extend(body.into_iter().map(|line| format!("    {line}")));
        out.push("}".to_string());
        out
    }

    fn container_content(&mut self, index: u32) -> Vec<String> {
        let info = self.info(index).clone();
        let s = self.style(index).clone();
        if info.flow.is_empty() {
            return Self::empty();
        }
        match &info.container {
            Container::Block => Self::block(
                "VStack(alignment: .leading, spacing: 0)".to_string(),
                self.child_lines(&info.flow, Spread::None, 0.0),
            ),
            Container::Flex { main, reverse } => {
                let mut children = info.flow.clone();
                if *reverse {
                    children.reverse();
                    self.finding(
                        index,
                        matrix_finding(
                            "flex-direction",
                            "reverse-reading-order",
                            "reverse 방향은 자식 출력 순서를 뒤집어 내므로 보조 기술 읽기 순서도 뒤집힙니다",
                        ),
                    );
                }
                if s.flex_wrap != FlexWrap::NoWrap {
                    self.finding(
                        index,
                        matrix_finding(
                            "flex-wrap",
                            "wrap",
                            "SwiftUI stack은 줄바꿈 배치가 없어 한 줄로 냅니다",
                        ),
                    );
                }
                let gap = match main {
                    Axis::Horizontal => s.column_gap.get(),
                    Axis::Vertical => s.row_gap.get(),
                };
                let baseline = s.align_items == ItemAlignment::Baseline;
                if baseline && *main == Axis::Vertical {
                    self.finding(
                        index,
                        matrix_finding(
                            "align-items",
                            "column-baseline",
                            "column flex의 baseline 정렬은 SwiftUI에 없어 시작 정렬로 냅니다",
                        ),
                    );
                }
                let stack_alignment = swiftui_stack_alignment(
                    *main,
                    item_edge(s.align_items),
                    baseline && *main == Axis::Horizontal,
                );
                let growing = children
                    .iter()
                    .any(|c| self.info(*c).fill.axes[main.index()]);
                let spread = if info.definite[main.index()] && !growing {
                    match s.justify_content {
                        ContentAlignment::SpaceBetween => Spread::Between,
                        ContentAlignment::SpaceEvenly => {
                            if gap != 0.0 {
                                self.finding(
                                    index,
                                    matrix_finding(
                                        "justify-content",
                                        "spread-gap",
                                        "space-evenly와 gap을 함께 쓰면 사이 Spacer만 gap을 최소로 가져 양끝과 사이 간격이 CSS와 다릅니다",
                                    ),
                                );
                            }
                            Spread::Evenly
                        }
                        ContentAlignment::SpaceAround => {
                            self.finding(
                                index,
                                matrix_finding(
                                    "justify-content",
                                    "space-around",
                                    "space-around의 양끝 절반 간격은 Spacer로 낼 수 없어 space-evenly로 냅니다",
                                ),
                            );
                            Spread::Evenly
                        }
                        _ => Spread::None,
                    }
                } else {
                    Spread::None
                };
                let stack = match main {
                    Axis::Horizontal => "HStack",
                    Axis::Vertical => "VStack",
                };
                let spacing = if spread == Spread::None { gap } else { 0.0 };
                Self::block(
                    format!(
                        "{stack}(alignment: {stack_alignment}, spacing: {})",
                        px(spacing)
                    ),
                    self.child_lines(&children, spread, gap),
                )
            }
            Container::Grid(plan) => {
                if let Some(reason) = plan.fallback {
                    self.finding(
                        index,
                        matrix_finding("grid-column-start", "grid-placement", reason),
                    );
                }
                self.grid_tracks_findings(index, plan);
                let rows = plan
                    .cells
                    .iter()
                    .map(|cell| cell.row as usize + 1)
                    .max()
                    .unwrap_or(0);
                let columns = plan.columns.len();
                let mut body = Vec::new();
                let available = grid_available(self.apple);
                if !available {
                    self.finding(
                        index,
                        matrix_finding(
                            "display",
                            "grid-unavailable",
                            "profile 최소 OS에 SwiftUI Grid가 없어 행·열 stack으로 냅니다",
                        ),
                    );
                }
                for row in 0..rows {
                    let mut row_lines = Vec::new();
                    let mut column = 0usize;
                    let row_cells: Vec<_> = plan
                        .cells
                        .iter()
                        .filter(|cell| cell.row as usize == row)
                        .copied()
                        .collect();
                    for cell in row_cells {
                        while column < cell.column as usize {
                            row_lines.extend(self.empty_cell(available));
                            column += 1;
                        }
                        row_lines.extend(self.cell_lines(index, cell, plan, available));
                        column += cell.column_span as usize;
                    }
                    while column < columns {
                        row_lines.extend(self.empty_cell(available));
                        column += 1;
                    }
                    if available {
                        body.extend(Self::block("GridRow".to_string(), row_lines));
                    } else {
                        body.extend(Self::block(
                            format!(
                                "HStack(alignment: .top, spacing: {})",
                                px(s.column_gap.get())
                            ),
                            row_lines,
                        ));
                    }
                }
                let (h, v) = (item_edge(s.justify_items), item_edge(s.align_items));
                if available {
                    Self::block(
                        format!(
                            "Grid(alignment: {}, horizontalSpacing: {}, verticalSpacing: {})",
                            swiftui_alignment(h, v),
                            px(s.column_gap.get()),
                            px(s.row_gap.get())
                        ),
                        body,
                    )
                } else {
                    Self::block(
                        format!(
                            "VStack(alignment: .leading, spacing: {})",
                            px(s.row_gap.get())
                        ),
                        body,
                    )
                }
            }
        }
    }

    fn grid_tracks_findings(&mut self, index: u32, plan: &crate::apple::native::GridPlan) {
        let fr: Vec<f32> = plan
            .columns
            .iter()
            .filter_map(|track| match track_kind(track) {
                TrackKind::Flexible(fr) | TrackKind::FlexibleAuto(fr) => Some(fr),
                _ => None,
            })
            .collect();
        if fr.windows(2).any(|pair| pair[0] != pair[1]) {
            self.finding(
                index,
                matrix_finding(
                    "grid-template-columns",
                    "unequal-fr",
                    "SwiftUI Grid는 유연한 열을 균등하게 나눠 fr 비율을 지키지 못합니다",
                ),
            );
        }
        for track in &plan.columns {
            match track_kind(track) {
                TrackKind::FlexibleAuto(_) => {
                    self.finding(
                        index,
                        matrix_finding(
                            "grid-template-columns",
                            "fr-auto-minimum",
                            "최소 auto인 fr 열은 콘텐츠 최소 크기를 지키지 않고 남은 공간을 나눕니다",
                        ),
                    );
                    break;
                }
                TrackKind::Other => {
                    self.finding(
                        index,
                        matrix_finding(
                            "grid-template-columns",
                            "track-size",
                            "백분율·섞인 minmax() 열은 SwiftUI Grid로 낼 수 없어 콘텐츠 크기로 냅니다",
                        ),
                    );
                    break;
                }
                _ => {}
            }
        }
        if plan.rows.iter().any(|track| {
            matches!(
                track_kind(track),
                TrackKind::Flexible(_) | TrackKind::FlexibleAuto(_) | TrackKind::Other
            )
        }) {
            self.finding(
                index,
                matrix_finding(
                    "grid-template-rows",
                    "row-track",
                    "SwiftUI Grid 행은 콘텐츠 크기라 fr·백분율 행 크기를 지키지 못합니다",
                ),
            );
        }
    }

    fn empty_cell(&self, available: bool) -> Vec<String> {
        if available {
            vec!["Color.clear.gridCellUnsizedAxes([.horizontal, .vertical])".to_string()]
        } else {
            vec!["Color.clear.frame(width: 0, height: 0)".to_string()]
        }
    }

    fn cell_lines(
        &mut self,
        grid: u32,
        cell: crate::apple::native::GridCell,
        plan: &crate::apple::native::GridPlan,
        available: bool,
    ) -> Vec<String> {
        let mut lines = vec![self.name(cell.child)];
        // 고정 크기와 채움은 한 frame에 섞을 수 없어 따로 냅니다.
        let mut fixed = Vec::new();
        let mut flexible = false;
        if cell.column_span == 1 {
            match track_kind(&plan.columns[cell.column as usize]) {
                TrackKind::Fixed(width) => fixed.push(format!("width: {}", px(width))),
                TrackKind::Flexible(_) | TrackKind::FlexibleAuto(_) => flexible = true,
                TrackKind::Content | TrackKind::Other => {
                    if self.info(cell.child).fill.axes[0] {
                        self.finding(
                            cell.child,
                            matrix_finding(
                                "justify-items",
                                "stretch-content-track",
                                "콘텐츠 크기 열 안의 stretch는 열을 넓혀 CSS track 크기와 다를 수 있습니다",
                            ),
                        );
                    }
                }
            }
        }
        if let Some(track) = plan.rows.get(cell.row as usize) {
            if let TrackKind::Fixed(height) = track_kind(track) {
                fixed.push(format!("height: {}", px(height)));
            }
        }
        let (h, v) = (
            item_edge(self.style(grid).justify_items),
            item_edge(self.style(grid).align_items),
        );
        let alignment = swiftui_alignment(h, v);
        if flexible {
            lines.push(format!(
                ".frame(maxWidth: .infinity, alignment: {alignment})"
            ));
        }
        if !fixed.is_empty() {
            lines.push(format!(
                ".frame({}, alignment: {alignment})",
                fixed.join(", ")
            ));
        }
        if cell.column_span > 1 {
            if available {
                lines.push(format!(".gridCellColumns({})", cell.column_span));
            } else {
                self.finding(
                    grid,
                    matrix_finding(
                        "grid-column-end",
                        "grid-span-unavailable",
                        "Grid가 없는 최소 OS에서는 열 span을 한 칸으로 냅니다",
                    ),
                );
            }
        }
        lines
    }

    fn sf_symbol(&mut self, index: u32) -> Option<String> {
        let node = &self.ir.nodes[index as usize];
        let property = node
            .properties
            .iter()
            .find(|property| property.capability == Some(p::APPLE_SF_SYMBOL))?;
        let TypedValue::String(name) = &property.value else {
            return None;
        };
        let unsupported = node.capabilities.iter().any(|finding| {
            finding.subject == CapabilitySubject::Property(p::APPLE_SF_SYMBOL)
                && finding.status == Some(CapabilityStatus::Unsupported)
        });
        (!unsupported).then(|| name.clone())
    }

    /// 버튼 동작 본문(activate는 onAction, select는 선택 상태 쓰기).
    fn button_body(&mut self, index: u32) -> Vec<String> {
        let mut body = Vec::new();
        let node_id = self.ir.nodes[index as usize].id;
        let Some(entry) = self.ir.semantics_of(index) else {
            return body;
        };
        for (action, target) in &entry.semantics.actions {
            match (action, target) {
                (ActionKind::Activate, ActionTarget::SemanticAction(action_id)) => body.push(
                    format!("onAction({})", string_literal(&action_id.to_string())),
                ),
                (ActionKind::Select, ActionTarget::State(state)) => {
                    if let Some((name, SwiftType::String)) = self.states.get(state) {
                        body.push(format!("{name} = {}", string_literal(&node_id.to_string())));
                    }
                }
                _ => {}
            }
        }
        body
    }

    fn text_input(&mut self, index: u32) -> Vec<String> {
        let mut binding = None;
        let mut placeholder = String::new();
        if let Some(entry) = self.ir.semantics_of(index) {
            if let Some(label) = &entry.semantics.label {
                placeholder = self.ir.static_text(label).unwrap_or_default();
            }
            if let Some(value) = &entry.semantics.value {
                if let crate::ir::ResolvedBinding::State(state) = self.ir.resolve_binding(value) {
                    if let Some((name, SwiftType::String)) = self.states.get(&state.id) {
                        binding = Some(format!("${name}"));
                    }
                }
            }
        }
        let binding = binding.unwrap_or_else(|| {
            self.note(index, AppleNoteKind::UnresolvedBinding(Facet::Value));
            ".constant(\"\")".to_string()
        });
        if !self.info(index).flow.is_empty() || !self.info(index).absolute.is_empty() {
            self.note(
                index,
                AppleNoteKind::Control(ControlNote::ChildrenIgnored(Control::TextInput)),
            );
        }
        let placeholder = string_literal(&placeholder);
        vec![
            format!("TextField(text: {binding}, prompt: Text(verbatim: {placeholder})) {{"),
            format!("    Text(verbatim: {placeholder})"),
            "}".to_string(),
            ".textFieldStyle(.plain)".to_string(),
        ]
    }

    fn content(&mut self, index: u32) -> Vec<String> {
        let info = self.info(index).clone();
        if let Some(dropped) = info.dropped_control {
            self.note(
                index,
                AppleNoteKind::Control(ControlNote::ContractUnsupported(dropped)),
            );
        }
        match info.kind {
            ViewKind::Text => {
                let text = self.ir.nodes[index as usize]
                    .text
                    .clone()
                    .unwrap_or_default();
                vec![format!("Text(verbatim: {})", string_literal(&text))]
            }
            ViewKind::Image => match self.sf_symbol(index) {
                Some(name) => vec![
                    format!("Image(systemName: {})", string_literal(&name)),
                    ".resizable()".to_string(),
                    ".scaledToFit()".to_string(),
                ],
                None => {
                    self.note(
                        index,
                        AppleNoteKind::Control(ControlNote::ImageSourceUnavailable),
                    );
                    Self::empty()
                }
            },
            ViewKind::Shape => {
                self.note(
                    index,
                    AppleNoteKind::Control(ControlNote::ShapeGeometryUnavailable),
                );
                Self::empty()
            }
            ViewKind::Instance => {
                self.note(
                    index,
                    AppleNoteKind::Control(ControlNote::InstanceUnexpanded),
                );
                Self::empty()
            }
            ViewKind::Container => match info.control {
                Some(Control::TextInput) => self.text_input(index),
                Some(Control::Modal) => {
                    self.note(index, AppleNoteKind::Control(ControlNote::ModalInPlace));
                    self.container_content(index)
                }
                _ => self.container_content(index),
            },
        }
    }

    fn accessibility(&self, index: u32) -> Option<String> {
        let hook = self.hooks[index as usize].as_ref()?;
        let mut args = Vec::new();
        for state in &hook.states {
            if let Some((name, _)) = self.states.get(state) {
                args.push(format!("{name}: ${name}"));
            }
        }
        if hook.action {
            args.push("onAction: onAction".to_string());
        }
        Some(format!(
            ".modifier({}Accessibility{}({}))",
            self.prefix,
            id_hex(self.ir.nodes[index as usize].id),
            args.join(", ")
        ))
    }

    /// node 속성 식 줄.
    fn node_lines(&mut self, index: u32) -> Vec<String> {
        let mut lines = self.content(index);
        lines.extend(self.own_modifiers(index));
        if self.info(index).control == Some(Control::Button) {
            let body = self.button_body(index);
            let mut button = Self::block("Button".to_string(), body);
            let last = button.pop().expect("닫는 괄호");
            button.push(format!("{last} label: {{"));
            button.extend(lines.into_iter().map(|line| format!("    {line}")));
            button.push("}".to_string());
            button.push(".buttonStyle(.plain)".to_string());
            lines = button;
        }
        lines.extend(self.item_modifiers(index));
        lines.extend(self.accessibility(index));
        lines
    }

    /// root에 한 번 내는 플랫폼 기본 동작 근사(자동 최소 크기, flex-shrink 1 압축).
    fn root_findings(&mut self) {
        let mut shrink = false;
        for index in 0..self.ir.nodes.len() as u32 {
            let info = self.info(index);
            if !info.emitted {
                continue;
            }
            let Some(parent) = info.parent else {
                continue;
            };
            if matches!(self.info(parent).container, Container::Flex { .. })
                && self.style(index).position != Position::Absolute
                && self.style(index).flex_shrink.get() == 1.0
            {
                shrink = true;
            }
        }
        if shrink {
            self.finding(
                0,
                matrix_finding(
                    "flex-shrink",
                    "platform-compression",
                    "넘칠 때 SwiftUI stack의 압축 순서·자동 최소 크기(min-*: auto)는 CSS flex-shrink 1과 다를 수 있습니다",
                ),
            );
        }
    }

    /// `struct <prefix>View`.
    pub(crate) fn source(&mut self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "\nstruct {}View: View {{", self.prefix);
        for state in &self.ir.states {
            if let Some((name, ty)) = self.states.get(&state.id) {
                let _ = writeln!(
                    out,
                    "    @State private var {name}: {} = {}",
                    ty.name(),
                    initial_literal(&state.initial, *ty)
                );
            }
        }
        out.push_str("    let onAction: (String) -> Void\n\n");
        out.push_str("    init(onAction: @escaping (String) -> Void = { _ in }) {\n");
        out.push_str("        self.onAction = onAction\n    }\n\n");
        out.push_str("    var body: some View {\n");
        if self.info(0).emitted {
            let _ = writeln!(out, "        {}", self.name(0));
        } else {
            out.push_str("        EmptyView()\n");
        }
        out.push_str("    }\n");
        if self.info(0).emitted {
            self.root_findings();
        }
        for index in 0..self.ir.nodes.len() as u32 {
            if !self.info(index).emitted {
                continue;
            }
            let lines = self.node_lines(index);
            let _ = writeln!(out, "\n    private var {}: some View {{", self.name(index));
            for line in lines {
                let _ = writeln!(out, "        {line}");
            }
            out.push_str("    }\n");
        }
        out.push_str("}\n");
        out
    }
}

/// SwiftUi dialect 값별 진단. 상태는 XXII.1.4 matrix가 정합니다(ADR-0121 결정 2).
fn matrix_finding(
    longhand: &'static str,
    code: &'static str,
    reason: &'static str,
) -> LayoutFinding {
    crate::matrix::dialect::finding(
        crate::matrix::dialect::Dialect::SwiftUi,
        longhand,
        code,
        reason,
    )
}
