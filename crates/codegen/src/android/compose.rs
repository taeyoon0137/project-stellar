//! Compose 생성기: 구조·상태·element·layout (XXII.4.1·XXII.4.2, ADR-0107 결정 6).
//!
//! | element | Compose |
//! |---|---|
//! | Frame(block·flex) | `Column`·`Row`(wrap은 `FlowRow`·`FlowColumn`), absolute 자식이 있으면 바깥 `Box` 안에 흐름 container |
//! | Frame(grid) | 행 `Column` 안 칸 `Row`, 칸마다 `Box(weight|width, contentAlignment)` |
//! | Text | `BasicText(text = …)` |
//! | TextInput contract | `BasicTextField(value = state.<상태>, onValueChange = { state.<상태> = it }, decorationBox = placeholder)` |
//! | Image·Shape·펼치지 않은 instance | 원본·기하가 IR에 없어 `Box`(노트) |
//!
//! Material 컴포넌트를 쓰지 않고 foundation만 씁니다. 디자인한 모양(CSS layout)을 Material 기본 모양이 덮지 않게 하기
//! 위해서입니다. import는 실제로 쓴 이름만 이름 순으로 냅니다.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use stellar_core_types::typed_value::TypedValue;
use stellar_layout::style::{
    BoxSizing, ContentAlignment, FlexBasis, FlexWrap, ItemAlignment, LayoutStyle,
    LengthPercentageAuto, MaxSize, Position, Size,
};
use stellar_target_profile::spec::AndroidProfile;

use crate::android::kotlin::{dp, float, identifiers, string_literal};
use crate::android::{AndroidNote, AndroidNoteKind, AndroidNoteSubject};
use crate::apple::layout::{approximated, unsupported};
use crate::apple::native::{
    grow_is_basis_zero, item_edge, justify_alignment, justify_edge, self_alignment, size, style,
    track_kind, Analysis, Axis, Container, Control, Edge, GridCell, GridPlan, NodeInfo, TrackKind,
    ViewKind,
};
use crate::apple::{ControlNote, LayoutFinding};
use crate::facet::Facet;
use crate::ir::snapshot::id_hex;
use crate::ir::{CodegenIr, ResolvedBinding};

/// 이름 → import 경로.
pub(crate) const IMPORTS: &[(&str, &str)] = &[
    ("Alignment", "androidx.compose.ui.Alignment"),
    (
        "Arrangement",
        "androidx.compose.foundation.layout.Arrangement",
    ),
    ("BasicText", "androidx.compose.foundation.text.BasicText"),
    (
        "BasicTextField",
        "androidx.compose.foundation.text.BasicTextField",
    ),
    ("Box", "androidx.compose.foundation.layout.Box"),
    ("BoxScope", "androidx.compose.foundation.layout.BoxScope"),
    ("Color", "androidx.compose.ui.graphics.Color"),
    (
        "ColorSpaces",
        "androidx.compose.ui.graphics.colorspace.ColorSpaces",
    ),
    ("Column", "androidx.compose.foundation.layout.Column"),
    (
        "ColumnScope",
        "androidx.compose.foundation.layout.ColumnScope",
    ),
    ("Composable", "androidx.compose.runtime.Composable"),
    (
        "ExperimentalLayoutApi",
        "androidx.compose.foundation.layout.ExperimentalLayoutApi",
    ),
    (
        "FlowColumn",
        "androidx.compose.foundation.layout.FlowColumn",
    ),
    (
        "FlowColumnScope",
        "androidx.compose.foundation.layout.FlowColumnScope",
    ),
    ("FlowRow", "androidx.compose.foundation.layout.FlowRow"),
    (
        "FlowRowScope",
        "androidx.compose.foundation.layout.FlowRowScope",
    ),
    (
        "IntrinsicSize",
        "androidx.compose.foundation.layout.IntrinsicSize",
    ),
    ("Matrix", "androidx.compose.ui.graphics.Matrix"),
    ("Modifier", "androidx.compose.ui.Modifier"),
    ("Row", "androidx.compose.foundation.layout.Row"),
    ("RowScope", "androidx.compose.foundation.layout.RowScope"),
    ("Spacer", "androidx.compose.foundation.layout.Spacer"),
    (
        "aspectRatio",
        "androidx.compose.foundation.layout.aspectRatio",
    ),
    ("dp", "androidx.compose.ui.unit.dp"),
    (
        "drawWithContent",
        "androidx.compose.ui.draw.drawWithContent",
    ),
    (
        "fillMaxHeight",
        "androidx.compose.foundation.layout.fillMaxHeight",
    ),
    (
        "fillMaxWidth",
        "androidx.compose.foundation.layout.fillMaxWidth",
    ),
    ("getValue", "androidx.compose.runtime.getValue"),
    ("height", "androidx.compose.foundation.layout.height"),
    ("heightIn", "androidx.compose.foundation.layout.heightIn"),
    ("mutableStateOf", "androidx.compose.runtime.mutableStateOf"),
    ("offset", "androidx.compose.foundation.layout.offset"),
    ("padding", "androidx.compose.foundation.layout.padding"),
    ("remember", "androidx.compose.runtime.remember"),
    ("setValue", "androidx.compose.runtime.setValue"),
    ("width", "androidx.compose.foundation.layout.width"),
    ("widthIn", "androidx.compose.foundation.layout.widthIn"),
    (
        "withTransform",
        "androidx.compose.ui.graphics.drawscope.withTransform",
    ),
];

/// Kotlin 상태 타입.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KotlinType {
    /// `String`.
    String,
    /// `Boolean`.
    Boolean,
    /// `Double`.
    Double,
}

/// 부모가 주는 scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    None,
    Row,
    Column,
    FlowRow,
    FlowColumn,
    Box,
}

impl Scope {
    fn receiver(self) -> &'static str {
        match self {
            Scope::None => "",
            Scope::Row => "RowScope.",
            Scope::Column => "ColumnScope.",
            Scope::FlowRow => "FlowRowScope.",
            Scope::FlowColumn => "FlowColumnScope.",
            Scope::Box => "BoxScope.",
        }
    }

    fn import(self) -> Option<&'static str> {
        match self {
            Scope::None => None,
            Scope::Row => Some("RowScope"),
            Scope::Column => Some("ColumnScope"),
            Scope::FlowRow => Some("FlowRowScope"),
            Scope::FlowColumn => Some("FlowColumnScope"),
            Scope::Box => Some("BoxScope"),
        }
    }
}

/// 생성기.
pub(crate) struct Generator<'a> {
    ir: &'a CodegenIr,
    analysis: &'a Analysis,
    android: AndroidProfile,
    prefix: &'a str,
    notes: &'a mut Vec<AndroidNote>,
    /// 상태 변수 → (Kotlin 이름, 타입).
    pub(crate) states:
        std::collections::BTreeMap<stellar_core_types::ids::StableId, (String, KotlinType)>,
    used: BTreeSet<&'static str>,
    scopes: Vec<Scope>,
    flow_used: bool,
}

fn px(value: f32) -> String {
    dp(value)
}

/// Compose UI 버전이 `major.minor` 이상인지.
pub(crate) fn ui_at_least(android: AndroidProfile, major: u16, minor: u16) -> bool {
    let version = android.ui_system_version;
    (version.major(), version.minor()) >= (major, minor)
}

fn horizontal(edge: Edge) -> &'static str {
    match edge {
        Edge::Start => "Alignment.Start",
        Edge::Center => "Alignment.CenterHorizontally",
        Edge::End => "Alignment.End",
    }
}

fn vertical(edge: Edge) -> &'static str {
    match edge {
        Edge::Start => "Alignment.Top",
        Edge::Center => "Alignment.CenterVertically",
        Edge::End => "Alignment.Bottom",
    }
}

fn box_alignment(h: Edge, v: Edge) -> &'static str {
    match (h, v) {
        (Edge::Start, Edge::Start) => "Alignment.TopStart",
        (Edge::Center, Edge::Start) => "Alignment.TopCenter",
        (Edge::End, Edge::Start) => "Alignment.TopEnd",
        (Edge::Start, Edge::Center) => "Alignment.CenterStart",
        (Edge::Center, Edge::Center) => "Alignment.Center",
        (Edge::End, Edge::Center) => "Alignment.CenterEnd",
        (Edge::Start, Edge::End) => "Alignment.BottomStart",
        (Edge::Center, Edge::End) => "Alignment.BottomCenter",
        (Edge::End, Edge::End) => "Alignment.BottomEnd",
    }
}

/// 여러 줄 modifier 식.
fn modifier_expr(parts: &[String]) -> Vec<String> {
    if parts.is_empty() {
        return vec!["Modifier".to_string()];
    }
    if parts.len() == 1 {
        return vec![format!("Modifier.{}", parts[0])];
    }
    let mut out = vec!["Modifier".to_string()];
    out.extend(parts.iter().map(|part| format!("    .{part}")));
    out
}

impl<'a> Generator<'a> {
    pub(crate) fn new(
        ir: &'a CodegenIr,
        analysis: &'a Analysis,
        android: AndroidProfile,
        prefix: &'a str,
        notes: &'a mut Vec<AndroidNote>,
    ) -> Generator<'a> {
        let typed: Vec<_> = ir
            .states
            .iter()
            .map(|state| {
                let ty = match state.initial {
                    TypedValue::String(_) => Some(KotlinType::String),
                    TypedValue::Boolean(_) => Some(KotlinType::Boolean),
                    TypedValue::Number(_) => Some(KotlinType::Double),
                    _ => None,
                };
                (state.id, state.name.as_str(), ty)
            })
            .collect();
        let names = identifiers(typed.iter().map(|(_, name, _)| *name), "state");
        let states = typed
            .iter()
            .zip(names)
            .filter_map(|((id, _, ty), name)| ty.map(|ty| (*id, (name, ty))))
            .collect();
        Generator {
            ir,
            analysis,
            android,
            prefix,
            notes,
            states,
            used: BTreeSet::new(),
            scopes: vec![Scope::None; ir.nodes.len()],
            flow_used: false,
        }
    }

    fn use_name(&mut self, name: &'static str) {
        self.used.insert(name);
    }

    fn note(&mut self, index: u32, kind: AndroidNoteKind) {
        self.notes.push(AndroidNote {
            subject: AndroidNoteSubject::Node(self.ir.nodes[index as usize].id),
            kind,
        });
    }

    fn finding(&mut self, index: u32, finding: LayoutFinding) {
        self.note(index, AndroidNoteKind::Layout(finding));
    }

    fn name(&self, index: u32) -> String {
        format!("Node{}", id_hex(self.ir.nodes[index as usize].id))
    }

    fn call(&self, index: u32) -> String {
        format!("{}(state, onAction)", self.name(index))
    }

    fn style(&self, index: u32) -> &LayoutStyle {
        style(self.ir, index).expect("출력 node는 layout이 있음")
    }

    fn info(&self, index: u32) -> &NodeInfo {
        &self.analysis.nodes[index as usize]
    }

    fn dp(&mut self, value: f32) -> String {
        self.use_name("dp");
        px(value)
    }

    fn flow_container(&self, index: u32) -> bool {
        matches!(self.info(index).container, Container::Flex { .. })
            && self.style(index).flex_wrap != FlexWrap::NoWrap
            && ui_at_least(self.android, 1, 4)
    }

    /// 확정이 아닌 축에서 자식을 늘리는 container(`IntrinsicSize.Max`).
    fn needs_intrinsic(&self, index: u32) -> [bool; 2] {
        let info = self.info(index);
        let s = self.style(index);
        let mut out = [false; 2];
        for axis in [Axis::Horizontal, Axis::Vertical] {
            if info.definite[axis.index()]
                || info.fill.axes[axis.index()]
                || matches!(size(s, axis), Size::Length(_))
            {
                continue;
            }
            // Flow container의 cross 축은 줄마다 정하고, grid는 행 Row가 Intrinsic을 맡습니다.
            if self.flow_container(index) || matches!(info.container, Container::Grid(_)) {
                continue;
            }
            out[axis.index()] = info
                .flow
                .iter()
                .any(|child| self.info(*child).fill.axes[axis.index()]);
        }
        out
    }

    /// 부모 기준 item modifier(체인 맨 앞).
    fn item_modifiers(&mut self, index: u32) -> Vec<String> {
        let mut out = Vec::new();
        let info = self.info(index).clone();
        let s = self.style(index).clone();
        let Some(parent) = info.parent else {
            return out;
        };
        let parent_style = self.style(parent).clone();
        let (ptop, _, _, pleft) = parent_style.padding.components();
        if s.position == Position::Absolute {
            let h = match (s.left, s.right) {
                (LengthPercentageAuto::Auto, LengthPercentageAuto::Auto) => Edge::Start,
                (LengthPercentageAuto::Auto, _) => Edge::End,
                _ => Edge::Start,
            };
            let v = match (s.top, s.bottom) {
                (LengthPercentageAuto::Auto, LengthPercentageAuto::Auto) => Edge::Start,
                (LengthPercentageAuto::Auto, _) => Edge::End,
                _ => Edge::Start,
            };
            self.use_name("Alignment");
            out.push(format!("align({})", box_alignment(h, v)));
        }
        if matches!(s.position, Position::Relative | Position::Absolute) {
            let absolute = s.position == Position::Absolute;
            let x = self.inset(index, (s.left, "left"), (s.right, "right"), absolute, pleft);
            let y = self.inset(index, (s.top, "top"), (s.bottom, "bottom"), absolute, ptop);
            if x != 0.0 || y != 0.0 {
                self.use_name("offset");
                out.push(format!("offset(x = {}, y = {})", self.dp(x), self.dp(y)));
            }
        }
        if absolute_in_natural_box(self, parent) && s.position == Position::Absolute {
            self.finding(
                index,
                approximated(
                    "position",
                    "absolute-in-natural-box",
                    "Compose Box는 모든 자식 크기를 합쳐 크기를 정하므로 자연 크기 부모에서는 absolute 자식이 부모를 키울 수 있습니다",
                ),
            );
        }
        if s.position == Position::Absolute {
            return out;
        }
        match self.info(parent).container.clone() {
            Container::Flex { main, .. } => {
                let cross = main.cross();
                if info.fill.axes[main.index()] {
                    let growing: Vec<LayoutStyle> = self
                        .info(parent)
                        .flow
                        .iter()
                        .map(|c| self.style(*c).clone())
                        .filter(|c| c.flex_grow.get() > 0.0)
                        .collect();
                    let exact =
                        growing.len() <= 1 || growing.iter().all(|c| grow_is_basis_zero(c, main));
                    if !exact {
                        self.finding(
                            index,
                            approximated(
                                "flex-grow",
                                "grow-distribution",
                                "Compose weight는 콘텐츠 크기를 빼지 않고 남은 공간을 비율로 나눕니다",
                            ),
                        );
                    }
                    out.push(format!("weight({})", float(s.flex_grow.get())));
                }
                let own = self_alignment(&parent_style, &s);
                let stack = item_edge(parent_style.align_items);
                if own == ItemAlignment::Baseline {
                    if main == Axis::Horizontal {
                        out.push("alignByBaseline()".to_string());
                    } else {
                        self.finding(
                            index,
                            approximated(
                                "align-self",
                                "column-baseline",
                                "column flex의 baseline 정렬은 Compose에 없어 시작 정렬로 냅니다",
                            ),
                        );
                    }
                } else if !info.fill.axes[cross.index()] && item_edge(own) != stack {
                    self.use_name("Alignment");
                    out.push(format!(
                        "align({})",
                        match cross {
                            Axis::Vertical => vertical(item_edge(own)),
                            Axis::Horizontal => horizontal(item_edge(own)),
                        }
                    ));
                }
                let shrink = s.flex_shrink.get();
                if shrink != 1.0 {
                    self.finding(
                        index,
                        approximated(
                            "flex-shrink",
                            "shrink-ratio",
                            "Compose Row·Column은 item별 압축 비율이 없어 순차 측정으로 냅니다",
                        ),
                    );
                }
            }
            Container::Grid(_) | Container::Block => {}
        }
        out
    }

    fn inset(
        &mut self,
        index: u32,
        start: (LengthPercentageAuto, &'static str),
        end: (LengthPercentageAuto, &'static str),
        absolute: bool,
        static_offset: f32,
    ) -> f32 {
        for (value, name) in [start, end] {
            if matches!(value, LengthPercentageAuto::Percentage(_)) {
                self.finding(
                    index,
                    unsupported(
                        name,
                        "percent-inset",
                        "Compose offset은 부모 기준 백분율을 받지 않아 0으로 냅니다",
                    ),
                );
            }
        }
        match (start.0, end.0) {
            (LengthPercentageAuto::Length(a), LengthPercentageAuto::Length(_)) => {
                if absolute {
                    self.finding(
                        index,
                        approximated(
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
                    approximated(
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

    fn transform_modifiers(&mut self, index: u32) -> Vec<String> {
        let mut out = Vec::new();
        let Some(layout) = &self.ir.nodes[index as usize].layout else {
            return out;
        };
        let (a, b, c, d, tx, ty) = layout.transform.components();
        if tx != 0.0 || ty != 0.0 {
            self.use_name("offset");
            out.push(format!("offset(x = {}, y = {})", self.dp(tx), self.dp(ty)));
        }
        if (a, b, c, d) != (1.0, 0.0, 0.0, 1.0) {
            self.finding(
                index,
                approximated(
                    "transform",
                    "draw-transform",
                    "선형 변환은 그리기 단계(drawWithContent)에만 적용되어 입력 hit test 영역은 바뀌지 않습니다",
                ),
            );
            self.use_name("drawWithContent");
            self.use_name("withTransform");
            self.use_name("Matrix");
            out.push(format!(
                "drawWithContent {{ withTransform({{ transform(Matrix(floatArrayOf({}, {}, 0f, 0f, {}, {}, 0f, 0f, 0f, 0f, 1f, 0f, 0f, 0f, 0f, 1f))) }}) {{ this@drawWithContent.drawContent() }} }}",
                float(a),
                float(b),
                float(c),
                float(d)
            ));
        }
        out
    }

    /// 크기·padding modifier(바깥에서 안쪽 순서).
    fn box_modifiers(&mut self, index: u32, with_padding: bool) -> Vec<String> {
        let mut out = Vec::new();
        let info = self.info(index).clone();
        let s = self.style(index).clone();
        // CSS min·max는 크기를 자르므로 크기 modifier보다 바깥에 둡니다.
        for (axis, min_name, max_name, call) in [
            (Axis::Horizontal, "min-width", "max-width", "widthIn"),
            (Axis::Vertical, "min-height", "max-height", "heightIn"),
        ] {
            let (min, max) = match axis {
                Axis::Horizontal => (s.min_width, s.max_width),
                Axis::Vertical => (s.min_height, s.max_height),
            };
            let mut parts = Vec::new();
            match min {
                Size::Auto => {}
                Size::Length(length) => parts.push(format!("min = {}", self.dp(length.get()))),
                _ => self.finding(
                    index,
                    unsupported(
                        min_name,
                        "intrinsic-limit",
                        "Compose widthIn·heightIn은 백분율·intrinsic 최소 크기를 받지 않아 내지 않습니다",
                    ),
                ),
            }
            match max {
                MaxSize::None => {}
                MaxSize::Length(length) => parts.push(format!("max = {}", self.dp(length.get()))),
                _ => self.finding(
                    index,
                    unsupported(
                        max_name,
                        "intrinsic-limit",
                        "Compose widthIn·heightIn은 백분율·intrinsic 최대 크기를 받지 않아 내지 않습니다",
                    ),
                ),
            }
            if !parts.is_empty() {
                self.use_name(if axis == Axis::Horizontal {
                    "widthIn"
                } else {
                    "heightIn"
                });
                out.push(format!("{call}({})", parts.join(", ")));
            }
        }
        let intrinsic = self.needs_intrinsic(index);
        let main_basis = info
            .parent
            .and_then(|parent| match self.info(parent).container {
                Container::Flex { main, .. } if s.flex_grow.get() == 0.0 => Some(main),
                _ => None,
            });
        let mut sizes = Vec::new();
        for (axis, name, call, fill) in [
            (Axis::Horizontal, "width", "width", "fillMaxWidth"),
            (Axis::Vertical, "height", "height", "fillMaxHeight"),
        ] {
            let weighted = info.parent.is_some_and(|parent| {
                matches!(self.info(parent).container, Container::Flex { main, .. } if main == axis)
            }) && info.fill.axes[axis.index()];
            if weighted {
                continue;
            }
            if info.fill.axes[axis.index()] {
                self.use_name(fill);
                out.push(format!("{fill}()"));
                continue;
            }
            if intrinsic[axis.index()] {
                self.use_name(call);
                self.use_name("IntrinsicSize");
                sizes.push(format!("{call}(IntrinsicSize.Max)"));
                continue;
            }
            let mut value = size(&s, axis);
            if main_basis == Some(axis) {
                match s.flex_basis {
                    FlexBasis::Length(length) => value = Size::Length(length),
                    FlexBasis::Percentage(_) => self.finding(
                        index,
                        unsupported(
                            "flex-basis",
                            "percent-size",
                            "Compose에는 부모 기준 백분율 flex-basis가 없어 자연 크기로 냅니다",
                        ),
                    ),
                    _ => {}
                }
            }
            match value {
                Size::Auto => {}
                Size::Length(length) => {
                    self.use_name(call);
                    let v = self.dp(length.get());
                    sizes.push(format!("{call}({v})"));
                }
                Size::MaxContent | Size::MinContent => {
                    self.use_name(call);
                    self.use_name("IntrinsicSize");
                    sizes.push(format!(
                        "{call}(IntrinsicSize.{})",
                        if value == Size::MaxContent {
                            "Max"
                        } else {
                            "Min"
                        }
                    ));
                }
                Size::FitContent => self.finding(
                    index,
                    approximated(
                        name,
                        "fit-content",
                        "fit-content는 Compose 자연 크기(제약 안 콘텐츠 크기)로 냅니다",
                    ),
                ),
                Size::Percentage(percentage) => {
                    self.finding(
                        index,
                        approximated(
                            name,
                            "percent-size",
                            "Compose fillMax*(비율)은 부모가 준 최대 제약 기준이라 CSS containing block 백분율과 다를 수 있습니다",
                        ),
                    );
                    self.use_name(fill);
                    sizes.push(format!("{fill}({})", float(percentage.get() / 100.0)));
                }
            }
        }
        if let Some(ratio) = s.aspect_ratio {
            self.finding(
                index,
                approximated(
                    "aspect-ratio",
                    "aspect-ratio",
                    "Compose aspectRatio는 제약 안에 맞추므로 CSS 비율 크기 해석과 다를 수 있습니다",
                ),
            );
            self.use_name("aspectRatio");
            sizes.push(format!(
                "aspectRatio({} / {})",
                float(ratio.width.get()),
                float(ratio.height.get())
            ));
        }
        let (top, right, bottom, left) = s.padding.components();
        let padding =
            (with_padding && [top, right, bottom, left].iter().any(|v| *v != 0.0)).then(|| {
                format!(
                    "padding(start = {}, top = {}, end = {}, bottom = {})",
                    px(left),
                    px(top),
                    px(right),
                    px(bottom)
                )
            });
        if padding.is_some() {
            self.use_name("padding");
            self.use_name("dp");
        }
        if s.box_sizing == BoxSizing::ContentBox {
            out.extend(padding);
            out.extend(sizes);
        } else {
            out.extend(sizes);
            out.extend(padding);
        }
        out
    }

    /// node 전체 modifier.
    fn modifiers(&mut self, index: u32, with_padding: bool) -> Vec<String> {
        let mut out = self.item_modifiers(index);
        out.extend(self.transform_modifiers(index));
        out.extend(self.box_modifiers(index, with_padding));
        out
    }

    fn arrangement(&mut self, index: u32, main: Axis, reverse: bool, gap: f32) -> String {
        let s = self.style(index).clone();
        self.use_name("Arrangement");
        let spread = match s.justify_content {
            ContentAlignment::SpaceBetween => Some("SpaceBetween"),
            ContentAlignment::SpaceAround => Some("SpaceAround"),
            ContentAlignment::SpaceEvenly => Some("SpaceEvenly"),
            _ => None,
        };
        if let Some(spread) = spread {
            if gap != 0.0 {
                self.finding(
                    index,
                    approximated(
                        "justify-content",
                        "spread-gap",
                        "Compose Arrangement.Space*는 gap과 함께 쓸 수 없어 gap을 내지 않습니다",
                    ),
                );
            }
            return format!("Arrangement.{spread}");
        }
        let edge = justify_edge(s.justify_content, reverse);
        if gap != 0.0 {
            self.use_name("Alignment");
            let alignment = match main {
                Axis::Horizontal => horizontal(edge),
                Axis::Vertical => vertical(edge),
            };
            return format!("Arrangement.spacedBy({}, {alignment})", self.dp(gap));
        }
        match (main, edge) {
            (Axis::Horizontal, Edge::Start) => "Arrangement.Start",
            (Axis::Horizontal, Edge::Center) => "Arrangement.Center",
            (Axis::Horizontal, Edge::End) => "Arrangement.End",
            (Axis::Vertical, Edge::Start) => "Arrangement.Top",
            (Axis::Vertical, Edge::Center) => "Arrangement.Center",
            (Axis::Vertical, Edge::End) => "Arrangement.Bottom",
        }
        .to_string()
    }

    /// `Call(args) { body }` 줄.
    fn call_block(head: &str, args: Vec<Vec<String>>, body: Vec<String>) -> Vec<String> {
        let mut out = Vec::new();
        let single = args.iter().all(|arg| arg.len() == 1);
        if single && args.len() <= 1 {
            let joined: Vec<String> = args.iter().map(|arg| arg[0].clone()).collect();
            out.push(format!("{head}({})", joined.join(", ")));
        } else {
            out.push(format!("{head}("));
            for arg in args {
                let mut lines = arg.into_iter();
                if let Some(first) = lines.next() {
                    let rest: Vec<String> = lines.collect();
                    if rest.is_empty() {
                        out.push(format!("    {first},"));
                    } else {
                        out.push(format!("    {first}"));
                        let last = rest.len() - 1;
                        for (position, line) in rest.into_iter().enumerate() {
                            if position == last {
                                out.push(format!("    {line},"));
                            } else {
                                out.push(format!("    {line}"));
                            }
                        }
                    }
                }
            }
            out.push(")".to_string());
        }
        if !body.is_empty() {
            let last = out.pop().expect("머리 줄");
            out.push(format!("{last} {{"));
            out.extend(body.into_iter().map(|line| format!("    {line}")));
            out.push("}".to_string());
        }
        out
    }

    fn modifier_arg(parts: &[String]) -> Vec<String> {
        let mut lines = modifier_expr(parts);
        lines[0] = format!("modifier = {}", lines[0]);
        lines
    }

    fn set_scope(&mut self, children: &[u32], scope: Scope) {
        for child in children {
            self.scopes[*child as usize] = scope;
        }
    }

    /// 흐름 container 호출(block·flex).
    fn flow_call(&mut self, index: u32, modifier: Vec<String>) -> Vec<String> {
        let info = self.info(index).clone();
        let s = self.style(index).clone();
        let mut children = info.flow.clone();
        let (head, args, scope) = match info.container {
            Container::Flex { main, reverse } => {
                if reverse {
                    children.reverse();
                    self.finding(
                        index,
                        approximated(
                            "flex-direction",
                            "reverse-reading-order",
                            "reverse 방향은 자식 출력 순서를 뒤집어 내므로 접근성 순회 순서도 뒤집힙니다",
                        ),
                    );
                }
                let flow = s.flex_wrap != FlexWrap::NoWrap;
                if flow && !ui_at_least(self.android, 1, 4) {
                    self.finding(
                        index,
                        unsupported(
                            "flex-wrap",
                            "wrap-unavailable",
                            "profile Compose UI 버전에 FlowRow·FlowColumn이 없어 한 줄로 냅니다",
                        ),
                    );
                }
                if s.flex_wrap == FlexWrap::WrapReverse {
                    self.finding(
                        index,
                        unsupported(
                            "flex-wrap",
                            "wrap-reverse",
                            "Compose Flow에는 줄 순서 반전이 없어 wrap으로 냅니다",
                        ),
                    );
                }
                let flow = self.flow_container(index);
                let (main_gap, cross_gap) = match main {
                    Axis::Horizontal => (s.column_gap.get(), s.row_gap.get()),
                    Axis::Vertical => (s.row_gap.get(), s.column_gap.get()),
                };
                let arrangement = self.arrangement(index, main, reverse, main_gap);
                let edge = item_edge(s.align_items);
                if s.align_items == ItemAlignment::Baseline && main == Axis::Vertical {
                    self.finding(
                        index,
                        approximated(
                            "align-items",
                            "column-baseline",
                            "column flex의 baseline 정렬은 Compose에 없어 시작 정렬로 냅니다",
                        ),
                    );
                }
                self.use_name("Alignment");
                let mut args = vec![Self::modifier_arg(&modifier)];
                if flow {
                    self.flow_used = true;
                    self.use_name("ExperimentalLayoutApi");
                    let cross = if cross_gap != 0.0 {
                        self.use_name("Arrangement");
                        format!("Arrangement.spacedBy({})", self.dp(cross_gap))
                    } else {
                        self.use_name("Arrangement");
                        match main {
                            Axis::Horizontal => "Arrangement.Top".to_string(),
                            Axis::Vertical => "Arrangement.Start".to_string(),
                        }
                    };
                    match main {
                        Axis::Horizontal => {
                            self.use_name("FlowRow");
                            args.push(vec![format!("horizontalArrangement = {arrangement}")]);
                            args.push(vec![format!("verticalArrangement = {cross}")]);
                            ("FlowRow", args, Scope::FlowRow)
                        }
                        Axis::Vertical => {
                            self.use_name("FlowColumn");
                            args.push(vec![format!("verticalArrangement = {arrangement}")]);
                            args.push(vec![format!("horizontalArrangement = {cross}")]);
                            ("FlowColumn", args, Scope::FlowColumn)
                        }
                    }
                } else {
                    match main {
                        Axis::Horizontal => {
                            self.use_name("Row");
                            args.push(vec![format!("horizontalArrangement = {arrangement}")]);
                            args.push(vec![format!("verticalAlignment = {}", vertical(edge))]);
                            ("Row", args, Scope::Row)
                        }
                        Axis::Vertical => {
                            self.use_name("Column");
                            args.push(vec![format!("verticalArrangement = {arrangement}")]);
                            args.push(vec![format!("horizontalAlignment = {}", horizontal(edge))]);
                            ("Column", args, Scope::Column)
                        }
                    }
                }
            }
            _ => {
                self.use_name("Column");
                ("Column", vec![Self::modifier_arg(&modifier)], Scope::Column)
            }
        };
        self.set_scope(&children, scope);
        let body: Vec<String> = children.iter().map(|child| self.call(*child)).collect();
        Self::call_block(head, args, body)
    }

    fn grid_call(&mut self, index: u32, plan: &GridPlan, modifier: Vec<String>) -> Vec<String> {
        let s = self.style(index).clone();
        if let Some(reason) = plan.fallback {
            self.finding(
                index,
                unsupported("grid-column-start", "grid-placement", reason),
            );
        }
        for track in &plan.columns {
            match track_kind(track) {
                TrackKind::FlexibleAuto(_) => {
                    self.finding(
                        index,
                        approximated(
                            "grid-template-columns",
                            "fr-auto-minimum",
                            "최소 auto인 fr 열은 콘텐츠 최소 크기를 지키지 않고 weight로 나눕니다",
                        ),
                    );
                    break;
                }
                TrackKind::Content => {
                    self.finding(
                        index,
                        approximated(
                            "grid-template-columns",
                            "content-track",
                            "콘텐츠 크기 열은 행마다 칸 폭이 따로 정해져 열이 맞춰지지 않을 수 있습니다",
                        ),
                    );
                    break;
                }
                TrackKind::Other => {
                    self.finding(
                        index,
                        unsupported(
                            "grid-template-columns",
                            "track-size",
                            "백분율·섞인 minmax() 열은 Compose로 낼 수 없어 콘텐츠 크기로 냅니다",
                        ),
                    );
                    break;
                }
                TrackKind::Fixed(_) | TrackKind::Flexible(_) => {}
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
                approximated(
                    "grid-template-rows",
                    "row-track",
                    "Compose 행은 콘텐츠 크기라 fr·백분율 행 크기를 지키지 못합니다",
                ),
            );
        }
        let rows = plan
            .cells
            .iter()
            .map(|cell| cell.row as usize + 1)
            .max()
            .unwrap_or(0);
        let column_gap = s.column_gap.get();
        let mut body = Vec::new();
        self.use_name("Row");
        self.use_name("Box");
        self.use_name("Arrangement");
        for row in 0..rows {
            let cells: Vec<GridCell> = plan
                .cells
                .iter()
                .filter(|cell| cell.row as usize == row)
                .copied()
                .collect();
            let mut row_lines = Vec::new();
            let mut column = 0usize;
            let mut stretch_vertical = false;
            for cell in &cells {
                while column < cell.column as usize {
                    row_lines.push(self.empty_cell(plan, column));
                    column += 1;
                }
                let mut parts = self.cell_width(index, plan, *cell, column_gap);
                let child_style = self.style(cell.child).clone();
                let h = justify_alignment(&s, &child_style);
                let v = self_alignment(&s, &child_style);
                if self.info(cell.child).fill.axes[1] {
                    stretch_vertical = true;
                    self.use_name("fillMaxHeight");
                    parts.push("fillMaxHeight()".to_string());
                }
                let alignment = box_alignment(item_edge(h), item_edge(v));
                self.use_name("Alignment");
                self.scopes[cell.child as usize] = Scope::Box;
                row_lines.extend(Self::call_block(
                    "Box",
                    vec![
                        Self::modifier_arg(&parts),
                        vec![format!("contentAlignment = {alignment}")],
                    ],
                    vec![self.call(cell.child)],
                ));
                column += cell.column_span as usize;
            }
            while column < plan.columns.len() {
                row_lines.push(self.empty_cell(plan, column));
                column += 1;
            }
            let mut row_modifier = Vec::new();
            match plan.rows.get(row).map(track_kind) {
                Some(TrackKind::Fixed(height)) => {
                    self.use_name("height");
                    row_modifier.push(format!("height({})", self.dp(height)));
                }
                _ if stretch_vertical => {
                    self.use_name("height");
                    self.use_name("IntrinsicSize");
                    row_modifier.push("height(IntrinsicSize.Max)".to_string());
                }
                _ => {}
            }
            if self.info(index).fill.axes[0] || self.info(index).definite[0] {
                self.use_name("fillMaxWidth");
                row_modifier.push("fillMaxWidth()".to_string());
            }
            let arrangement = if column_gap != 0.0 {
                format!("Arrangement.spacedBy({})", self.dp(column_gap))
            } else {
                "Arrangement.Start".to_string()
            };
            body.extend(Self::call_block(
                "Row",
                vec![
                    Self::modifier_arg(&row_modifier),
                    vec![format!("horizontalArrangement = {arrangement}")],
                ],
                row_lines,
            ));
        }
        self.use_name("Column");
        let row_gap = s.row_gap.get();
        let arrangement = if row_gap != 0.0 {
            format!("Arrangement.spacedBy({})", self.dp(row_gap))
        } else {
            "Arrangement.Top".to_string()
        };
        Self::call_block(
            "Column",
            vec![
                Self::modifier_arg(&modifier),
                vec![format!("verticalArrangement = {arrangement}")],
            ],
            body,
        )
    }

    fn track_width(&mut self, track: TrackKind) -> Option<String> {
        match track {
            TrackKind::Fixed(width) => {
                self.use_name("width");
                Some(format!("width({})", self.dp(width)))
            }
            TrackKind::Flexible(fr) | TrackKind::FlexibleAuto(fr) => {
                Some(format!("weight({})", float(fr)))
            }
            TrackKind::Content | TrackKind::Other => None,
        }
    }

    fn cell_width(&mut self, grid: u32, plan: &GridPlan, cell: GridCell, gap: f32) -> Vec<String> {
        let start = cell.column as usize;
        let end = start + cell.column_span as usize;
        let tracks: Vec<TrackKind> = plan.columns[start..end.min(plan.columns.len())]
            .iter()
            .map(track_kind)
            .collect();
        if tracks.len() == 1 {
            return self.track_width(tracks[0]).into_iter().collect();
        }
        if tracks.iter().all(|t| matches!(t, TrackKind::Fixed(_))) {
            let total: f32 = tracks
                .iter()
                .map(|t| match t {
                    TrackKind::Fixed(w) => *w,
                    _ => 0.0,
                })
                .sum::<f32>()
                + gap * (tracks.len() - 1) as f32;
            self.use_name("width");
            return vec![format!("width({})", self.dp(total))];
        }
        if tracks
            .iter()
            .all(|t| matches!(t, TrackKind::Flexible(_) | TrackKind::FlexibleAuto(_)))
        {
            if gap != 0.0 {
                self.finding(
                    grid,
                    approximated(
                        "grid-column-end",
                        "grid-span-gap",
                        "gap이 있는 fr 열 span은 weight 합으로 내어 gap만큼 폭이 달라집니다",
                    ),
                );
            }
            let total: f32 = tracks
                .iter()
                .map(|t| match t {
                    TrackKind::Flexible(fr) | TrackKind::FlexibleAuto(fr) => *fr,
                    _ => 0.0,
                })
                .sum();
            return vec![format!("weight({})", float(total))];
        }
        self.finding(
            grid,
            approximated(
                "grid-column-end",
                "grid-span-mixed",
                "고정·비율·콘텐츠 track이 섞인 열 span은 콘텐츠 크기로 냅니다",
            ),
        );
        Vec::new()
    }

    fn empty_cell(&mut self, plan: &GridPlan, column: usize) -> String {
        self.use_name("Spacer");
        let parts: Vec<String> = self
            .track_width(track_kind(&plan.columns[column]))
            .into_iter()
            .collect();
        format!("Spacer({})", modifier_expr(&parts).join(""))
    }

    fn text_input(&mut self, index: u32, modifier: Vec<String>) -> Vec<String> {
        self.use_name("BasicTextField");
        self.use_name("BasicText");
        self.use_name("Box");
        let mut state = None;
        let mut placeholder = String::new();
        if let Some(entry) = self.ir.semantics_of(index) {
            if let Some(label) = &entry.semantics.label {
                placeholder = self.ir.static_text(label).unwrap_or_default();
            }
            if let Some(value) = &entry.semantics.value {
                if let ResolvedBinding::State(s) = self.ir.resolve_binding(value) {
                    if let Some((name, KotlinType::String)) = self.states.get(&s.id) {
                        state = Some(name.clone());
                    }
                }
            }
        }
        if !self.info(index).flow.is_empty() || !self.info(index).absolute.is_empty() {
            self.note(
                index,
                AndroidNoteKind::Control(ControlNote::ChildrenIgnored(Control::TextInput)),
            );
        }
        let (value, change, empty) = match state {
            Some(name) => (
                format!("state.{name}"),
                format!("{{ state.{name} = it }}"),
                format!("state.{name}.isEmpty()"),
            ),
            None => {
                self.note(index, AndroidNoteKind::UnresolvedBinding(Facet::Value));
                ("\"\"".to_string(), "{}".to_string(), "true".to_string())
            }
        };
        let placeholder = string_literal(&placeholder);
        let mut lines = vec!["BasicTextField(".to_string()];
        lines.push(format!("    value = {value},"));
        lines.push(format!("    onValueChange = {change},"));
        let modifier = Self::modifier_arg(&modifier);
        let last = modifier.len() - 1;
        for (position, line) in modifier.into_iter().enumerate() {
            lines.push(if position == last {
                format!("    {line},")
            } else {
                format!("    {line}")
            });
        }
        lines.push("    singleLine = true,".to_string());
        lines.push("    decorationBox = { inner ->".to_string());
        lines.push("        Box {".to_string());
        lines.push(format!(
            "            if ({empty}) BasicText(text = {placeholder})"
        ));
        lines.push("            inner()".to_string());
        lines.push("        }".to_string());
        lines.push("    },".to_string());
        lines.push(")".to_string());
        lines
    }

    fn node_body(&mut self, index: u32) -> Vec<String> {
        let info = self.info(index).clone();
        if let Some(dropped) = info.dropped_control {
            self.note(
                index,
                AndroidNoteKind::Control(ControlNote::ContractUnsupported(dropped)),
            );
        }
        if info.control == Some(Control::Modal) {
            self.note(index, AndroidNoteKind::Control(ControlNote::ModalInPlace));
        }
        let has_absolute = !info.absolute.is_empty();
        let container =
            info.kind == ViewKind::Container && info.control != Some(Control::TextInput);
        if container && has_absolute {
            // 바깥 Box가 크기·item modifier를, 안쪽 흐름 container가 padding을 맡습니다(absolute는 padding box 기준).
            let outer = self.modifiers(index, false);
            let mut inner = Vec::new();
            for (axis, fill) in [(0usize, "fillMaxWidth"), (1, "fillMaxHeight")] {
                let s = self.style(index);
                let explicit =
                    matches!(if axis == 0 { s.width } else { s.height }, Size::Length(_));
                if explicit || info.fill.axes[axis] {
                    self.use_name(fill);
                    inner.push(format!("{fill}()"));
                }
            }
            let (top, right, bottom, left) = self.style(index).padding.components();
            if [top, right, bottom, left].iter().any(|v| *v != 0.0) {
                self.use_name("padding");
                self.use_name("dp");
                inner.push(format!(
                    "padding(start = {}, top = {}, end = {}, bottom = {})",
                    px(left),
                    px(top),
                    px(right),
                    px(bottom)
                ));
            }
            let mut body = Vec::new();
            if !info.flow.is_empty() {
                body.extend(self.content(index, inner));
            }
            self.set_scope(&info.absolute, Scope::Box);
            for child in &info.absolute {
                body.push(self.call(*child));
            }
            self.use_name("Box");
            return Self::call_block("Box", vec![Self::modifier_arg(&outer)], body);
        }
        let modifier = self.modifiers(index, true);
        self.content(index, modifier)
    }

    fn content(&mut self, index: u32, modifier: Vec<String>) -> Vec<String> {
        let info = self.info(index).clone();
        match info.kind {
            ViewKind::Text => {
                self.use_name("BasicText");
                let text = self.ir.nodes[index as usize]
                    .text
                    .clone()
                    .unwrap_or_default();
                let mut args = vec![vec![format!("text = {}", string_literal(&text))]];
                if !modifier.is_empty() {
                    args.push(Self::modifier_arg(&modifier));
                }
                Self::call_block("BasicText", args, Vec::new())
            }
            ViewKind::Image | ViewKind::Shape | ViewKind::Instance => {
                let note = match info.kind {
                    ViewKind::Image => ControlNote::ImageSourceUnavailable,
                    ViewKind::Shape => ControlNote::ShapeGeometryUnavailable,
                    _ => ControlNote::InstanceUnexpanded,
                };
                self.note(index, AndroidNoteKind::Control(note));
                self.use_name("Box");
                Self::call_block("Box", vec![Self::modifier_arg(&modifier)], Vec::new())
            }
            ViewKind::Container => {
                if info.control == Some(Control::TextInput) {
                    return self.text_input(index, modifier);
                }
                if info.flow.is_empty() {
                    self.use_name("Box");
                    return Self::call_block(
                        "Box",
                        vec![Self::modifier_arg(&modifier)],
                        Vec::new(),
                    );
                }
                match info.container.clone() {
                    Container::Grid(plan) => self.grid_call(index, &plan, modifier),
                    _ => self.flow_call(index, modifier),
                }
            }
        }
    }

    fn root_findings(&mut self) {
        let shrink = (0..self.ir.nodes.len() as u32).any(|index| {
            let info = self.info(index);
            info.emitted
                && info.parent.is_some_and(|parent| {
                    matches!(self.info(parent).container, Container::Flex { .. })
                })
                && self.style(index).position != Position::Absolute
                && self.style(index).flex_shrink.get() == 1.0
        });
        if shrink {
            self.finding(
                0,
                approximated(
                    "flex-shrink",
                    "platform-compression",
                    "넘칠 때 Compose Row·Column의 순차 측정 압축과 자동 최소 크기(min-*: auto)는 CSS flex-shrink 1과 다를 수 있습니다",
                ),
            );
        }
    }

    fn state_class(&mut self) -> String {
        let prefix = self.prefix;
        let mut out = format!("class {prefix}State {{\n");
        if !self.states.is_empty() {
            self.use_name("getValue");
            self.use_name("setValue");
            self.use_name("mutableStateOf");
        }
        for state in &self.ir.states {
            let Some((name, ty)) = self.states.get(&state.id) else {
                continue;
            };
            let initial = match (&state.initial, ty) {
                (TypedValue::String(text), _) => string_literal(text),
                (TypedValue::Boolean(flag), _) => flag.to_string(),
                (TypedValue::Number(n), _) => {
                    let text = crate::text::num(n.get());
                    if text.contains('.') {
                        text
                    } else {
                        format!("{text}.0")
                    }
                }
                (_, KotlinType::String) => "\"\"".to_string(),
                (_, KotlinType::Boolean) => "false".to_string(),
                (_, KotlinType::Double) => "0.0".to_string(),
            };
            let _ = writeln!(out, "    var {name} by mutableStateOf({initial})");
        }
        out.push_str("}\n");
        out
    }

    /// Kotlin 파일.
    pub(crate) fn source(&mut self, package: &str) -> String {
        let prefix = self.prefix;
        let mut colors_notes = Vec::new();
        let colors = crate::android::color::colors_object(self.ir, prefix, &mut colors_notes);
        if colors.lines().count() > 2 {
            self.use_name("Color");
            self.use_name("ColorSpaces");
        }
        self.notes.extend(colors_notes);
        let state_class = self.state_class();
        self.use_name("Composable");
        self.use_name("remember");
        let mut functions = String::new();
        if self.info(0).emitted {
            self.root_findings();
        }
        for index in 0..self.ir.nodes.len() as u32 {
            if !self.info(index).emitted {
                continue;
            }
            let body = self.node_body(index);
            let scope = self.scopes[index as usize];
            if let Some(name) = scope.import() {
                self.used.insert(name);
            }
            let _ = write!(
                functions,
                "\n@Composable\nprivate fun {}{}(state: {prefix}State, onAction: (String) -> Unit) {{\n",
                scope.receiver(),
                self.name(index)
            );
            for line in body {
                let _ = writeln!(functions, "    {line}");
            }
            functions.push_str("}\n");
        }
        self.use_name("Modifier");
        let mut out = String::new();
        if self.flow_used {
            out.push_str("@file:OptIn(ExperimentalLayoutApi::class)\n\n");
        }
        let _ = writeln!(out, "package {package}\n");
        let mut imports: Vec<&str> = IMPORTS
            .iter()
            .filter(|(name, _)| self.used.contains(name))
            .map(|(_, path)| *path)
            .collect();
        imports.sort_unstable();
        for import in imports {
            let _ = writeln!(out, "import {import}");
        }
        out.push('\n');
        out.push_str(&colors);
        out.push('\n');
        out.push_str(&state_class);
        let _ = write!(
            out,
            "\n@Composable\nfun {prefix}View(\n    state: {prefix}State = remember {{ {prefix}State() }},\n    onAction: (String) -> Unit = {{}},\n) {{\n"
        );
        if self.info(0).emitted {
            let _ = writeln!(out, "    {}", self.call(0));
        }
        out.push_str("}\n");
        out.push_str(&functions);
        out
    }
}

fn absolute_in_natural_box(generator: &Generator<'_>, parent: u32) -> bool {
    let info = generator.info(parent);
    let s = generator.style(parent);
    [Axis::Horizontal, Axis::Vertical]
        .iter()
        .any(|axis| !info.fill.axes[axis.index()] && !matches!(size(s, *axis), Size::Length(_)))
}
