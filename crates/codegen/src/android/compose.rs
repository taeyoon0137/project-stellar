//! layout·control semantics·접근성 → Jetpack Compose API (XXII.4.2).
//!
//! Fragment가 아닌 IR node마다 composable 호출 하나를 만듭니다(Fragment 자식은 부모 content에 바로 놓임). 보이지 않는
//! node(`visible = false`)는 만들지 않습니다(layout에서도 제외).
//!
//! # layout
//!
//! | 공통 layout | Compose |
//! |---|---|
//! | Free | `Box`, 자식 `align`(끝 anchor) + `offset` |
//! | Stack 세로·가로 | `Column(verticalArrangement, horizontalAlignment)`·`Row(horizontalArrangement, verticalAlignment)` |
//! | Stack wrap | `FlowColumn`·`FlowRow`(`@OptIn(ExperimentalLayoutApi::class)`) |
//! | main 정렬 + gap | `Arrangement.spacedBy(gap, 정렬)`, gap 0이면 `Arrangement.Top/Start/Center/Bottom/End`, Space*는 `Arrangement.Space*`(gap은 근사) |
//! | cross 정렬 | `Alignment.Start/CenterHorizontally/End`·`Top/CenterVertically/Bottom`, Stretch는 자식 `fillMaxWidth()`/`fillMaxHeight()` |
//! | Grid | `LazyVerticalGrid(GridCells.Fixed(n) 또는 FixedSize)`, 자식 `item(span = { GridItemSpan(k) })` |
//! | Overlay | `Box(contentAlignment = Alignment.…)` |
//! | Constraint | 대응 없음 — `Box`로 두고 노트(ConstraintLayout은 별도 의존성) |
//! | sizing | Fixed `width`/`height`/`size`, Fill은 Stack main 축 `weight(1f)`·그 밖 `fillMax*()`, Percentage `fillMax*(비율)`, Hug는 기본 |
//! | flex grow | Stack main 축 `weight(grow)`(basis 차이는 근사 노트) |
//! | limits·aspect·padding | `widthIn`/`heightIn`, `aspectRatio`, `padding(all)`·`absolutePadding` |
//! | transform | 이동만 `offset`, 그 밖은 노트 |
//!
//! modifier 순서는 부모 scope(`weight`·`align`·`offset`) → 크기 → 상호작용·semantics → padding입니다.
//!
//! # control semantics·접근성
//!
//! | role | Compose |
//! |---|---|
//! | button | `clickable(enabled, role = Role.Button, onClick)`, pressed는 `toggleableState` |
//! | radio | `selectable(selected, enabled, role = Role.RadioButton, onClick)` |
//! | radioGroup | `selectableGroup()` |
//! | list | `collectionInfo = CollectionInfo(rowCount, 1)`, 선택 항목이 있으면 `selectableGroup()` |
//! | listItem | `collectionItemInfo`, 선택 가능하면 `selectable(selected, onClick)` |
//! | textField | `BasicTextField(value = 상태, onValueChange = { 상태 = it }, enabled, readOnly)` |
//! | dialog | `Dialog(onDismissRequest) { … }`, 이름은 `paneTitle` |
//! | image | `role = Role.Image` |
//! | group(이름 있음) | `contentDescription`, `isTraversalGroup = true` |
//! | 숨김 subtree | `clearAndSetSemantics { }` |
//! | label·value | `contentDescription`·`stateDescription`(자기 텍스트는 `BasicText`가 제공) |
//! | disabled | control의 `enabled = !식`, 그 밖은 `disabled()` |
//! | selected·focused | `selected`·`focused`, focusable(native 아님) `focusable()` |
//! | 읽기 순서(시각 순서와 다름) | 부모 `isTraversalGroup = true`, 자식 `traversalIndex`(Compose UI 1.5 이상) |
//!
//! 상태 변수는 `var 이름 by remember { mutableStateOf(초기값) }`, semantic action은 `onAction("<StableId>")`입니다.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use stellar_accessibility::node::{
    AccessibilityBinding, AccessibilityRole, ActionKind, ActionTarget, StateKind,
};
use stellar_core_types::ids::StableId;
use stellar_core_types::typed_value::TypedValue;
use stellar_core_types::value::Transform2D;
use stellar_layout::free::{AxisAnchors, Offset};
use stellar_layout::geometry::Axis;
use stellar_layout::grid::{GridLayout, GridPlacement, TrackSize};
use stellar_layout::input::{ContainerLayout, SizingMode};
use stellar_layout::overlay::OverlayAlignment;
use stellar_layout::stack::{
    CrossAlignment, MainAlignment, StackDirection, StackLayout, StackWrap,
};
use stellar_target_profile::capability::ElementKind;

use crate::android::kotlin::{double, dp, float, identifiers, string_literal};
use crate::android::{ComposeFacet, ComposeNote, ComposeNoteKind, LayoutFacet};
use crate::facet::Facet;
use crate::ir::{CodegenIr, IrAccessibilityEntry, IrLayout, IrNode, ResolvedBinding};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KotlinType {
    String,
    Boolean,
    Double,
}

/// 부모 content scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    /// root 또는 Dialog content(scope 없음).
    None,
    /// Box(Free).
    Free,
    /// Box(Overlay/Constraint).
    Box,
    /// Column/Row(Flow 포함), main 축.
    Stack(Axis, CrossAlignment),
    /// LazyVerticalGrid item.
    Grid,
}

/// 생성할 composable 호출.
#[derive(Debug, Clone, Default)]
struct Call {
    name: &'static str,
    args: Vec<(String, String)>,
    modifier: Vec<String>,
    children: Vec<usize>,
    /// 자식을 `item(...) { }`으로 감쌀 때의 span(Grid).
    grid_spans: Vec<u16>,
    /// content lambda가 없는 호출.
    leaf: bool,
}

struct Mapper<'a> {
    ir: &'a CodegenIr,
    ui_version: (u16, u16),
    states: BTreeMap<StableId, (String, KotlinType)>,
    calls: Vec<Call>,
    notes: Vec<ComposeNote>,
    uses_action: bool,
    uses_flow: bool,
    /// node → traversalIndex(읽기 순서가 시각 순서와 다른 부모의 자식).
    traversal_index: BTreeMap<u32, u32>,
    /// isTraversalGroup가 필요한 node.
    traversal_group: Vec<bool>,
    has_semantics: Vec<bool>,
}

fn axis_main_is_vertical(axis: Axis) -> bool {
    axis == Axis::Vertical
}

impl<'a> Mapper<'a> {
    fn note(&mut self, node: StableId, kind: ComposeNoteKind) {
        self.notes.push(ComposeNote { node, kind });
    }

    fn layout_note(&mut self, node: StableId, approximated: bool, facet: LayoutFacet) {
        let facet = ComposeFacet::Layout(facet);
        self.note(
            node,
            if approximated {
                ComposeNoteKind::Approximated(facet)
            } else {
                ComposeNoteKind::NoEquivalent(facet)
            },
        );
    }

    fn state_name(&self, id: StableId, ty: KotlinType) -> Option<String> {
        self.states
            .get(&id)
            .filter(|(_, t)| *t == ty)
            .map(|(name, _)| name.clone())
    }

    fn text_expr(
        &mut self,
        node: StableId,
        binding: &AccessibilityBinding,
        facet: Facet,
    ) -> Option<String> {
        match self.ir.resolve_binding(binding) {
            ResolvedBinding::Constant(TypedValue::String(text)) => Some(string_literal(text)),
            ResolvedBinding::Property(property) => match &property.value {
                TypedValue::String(text) => Some(string_literal(text)),
                _ => {
                    self.note(node, ComposeNoteKind::UnresolvedBinding(facet));
                    None
                }
            },
            ResolvedBinding::NodeText {
                text: Some(text), ..
            } => Some(string_literal(text)),
            ResolvedBinding::State(state) => match self.state_name(state.id, KotlinType::String) {
                Some(name) => Some(name),
                None => {
                    self.note(node, ComposeNoteKind::UnresolvedBinding(facet));
                    None
                }
            },
            ResolvedBinding::ComponentProperty(_) => {
                self.note(node, ComposeNoteKind::ComponentProperty(facet));
                None
            }
            _ => {
                self.note(node, ComposeNoteKind::UnresolvedBinding(facet));
                None
            }
        }
    }

    fn flag_expr(
        &mut self,
        node: StableId,
        binding: &AccessibilityBinding,
        facet: Facet,
    ) -> Option<String> {
        match self.ir.resolve_binding(binding) {
            ResolvedBinding::Constant(TypedValue::Boolean(value)) => Some(value.to_string()),
            ResolvedBinding::Property(property) => match property.value {
                TypedValue::Boolean(value) => Some(value.to_string()),
                _ => {
                    self.note(node, ComposeNoteKind::UnresolvedBinding(facet));
                    None
                }
            },
            ResolvedBinding::State(state) => match self.state_name(state.id, KotlinType::Boolean) {
                Some(name) => Some(name),
                None => {
                    self.note(node, ComposeNoteKind::UnresolvedBinding(facet));
                    None
                }
            },
            ResolvedBinding::SelectionMatches { state, item } => {
                match self.state_name(state.id, KotlinType::String) {
                    Some(name) => Some(format!("{name} == {}", string_literal(&item.to_string()))),
                    None => {
                        self.note(node, ComposeNoteKind::UnresolvedBinding(facet));
                        None
                    }
                }
            }
            ResolvedBinding::ComponentProperty(_) => {
                self.note(node, ComposeNoteKind::ComponentProperty(facet));
                None
            }
            _ => {
                self.note(node, ComposeNoteKind::UnresolvedBinding(facet));
                None
            }
        }
    }

    fn supports_traversal(&self) -> bool {
        self.ui_version >= (1, 5)
    }

    /// 읽기 순서가 시각 순서와 다른 entry의 부모·자식 표시.
    fn plan_reading_order(&mut self) {
        let ir = self.ir;
        for (entry_index, entry) in ir.accessibility.iter().enumerate() {
            let children: Vec<u32> = ir
                .accessibility_children(entry_index as u32)
                .into_iter()
                .map(|child| ir.accessibility[child as usize].node)
                .collect();
            if children.is_empty() {
                continue;
            }
            let mut visual = children.clone();
            visual.sort_unstable();
            if visual == children {
                continue;
            }
            let parent_id = ir.nodes[entry.node as usize].id;
            if !self.supports_traversal() {
                self.note(
                    parent_id,
                    ComposeNoteKind::Unavailable(ComposeFacet::Semantics(Facet::ReadingOrder)),
                );
                continue;
            }
            self.traversal_group[entry.node as usize] = true;
            for (position, child) in children.iter().enumerate() {
                self.traversal_index.insert(*child, position as u32);
                // 접근성 부모가 시각 조상이 아니면 traversal 묶음 밖이라 근사입니다.
                let mut ancestor = ir.nodes[*child as usize].parent;
                let mut inside = false;
                while let Some(p) = ancestor {
                    if p == entry.node {
                        inside = true;
                        break;
                    }
                    ancestor = ir.nodes[p as usize].parent;
                }
                if !inside {
                    let id = ir.nodes[*child as usize].id;
                    self.note(
                        id,
                        ComposeNoteKind::Approximated(ComposeFacet::Semantics(Facet::ReadingOrder)),
                    );
                }
            }
        }
    }

    fn main_arrangement(&mut self, node: StableId, stack: &StackLayout) -> String {
        let vertical = axis_main_is_vertical(stack.axis);
        let gap = stack.gap.get();
        let (start, center, end) = if vertical {
            ("Top", "CenterVertically", "Bottom")
        } else {
            ("Start", "CenterHorizontally", "End")
        };
        let spaced =
            |alignment: &str| format!("Arrangement.spacedBy({}, Alignment.{alignment})", dp(gap));
        match stack.main_alignment {
            MainAlignment::Start if gap > 0.0 => spaced(start),
            MainAlignment::Center if gap > 0.0 => spaced(center),
            MainAlignment::End if gap > 0.0 => spaced(end),
            MainAlignment::Start => format!("Arrangement.{start}"),
            MainAlignment::Center => "Arrangement.Center".to_string(),
            MainAlignment::End => format!("Arrangement.{end}"),
            MainAlignment::SpaceBetween
            | MainAlignment::SpaceAround
            | MainAlignment::SpaceEvenly => {
                if gap > 0.0 {
                    self.layout_note(node, true, LayoutFacet::Gap);
                }
                match stack.main_alignment {
                    MainAlignment::SpaceBetween => "Arrangement.SpaceBetween",
                    MainAlignment::SpaceAround => "Arrangement.SpaceAround",
                    _ => "Arrangement.SpaceEvenly",
                }
                .to_string()
            }
        }
    }

    fn cross_alignment(stack: &StackLayout) -> String {
        let vertical = axis_main_is_vertical(stack.axis);
        let name = match (stack.cross_alignment, vertical) {
            (CrossAlignment::Start | CrossAlignment::Stretch, true) => "Start",
            (CrossAlignment::Center, true) => "CenterHorizontally",
            (CrossAlignment::End, true) => "End",
            (CrossAlignment::Start | CrossAlignment::Stretch, false) => "Top",
            (CrossAlignment::Center, false) => "CenterVertically",
            (CrossAlignment::End, false) => "Bottom",
        };
        format!("Alignment.{name}")
    }

    fn overlay_alignment(horizontal: OverlayAlignment, vertical: OverlayAlignment) -> String {
        let v = match vertical {
            OverlayAlignment::Start => "Top",
            OverlayAlignment::Center => "Center",
            OverlayAlignment::End => "Bottom",
        };
        let h = match horizontal {
            OverlayAlignment::Start => "Start",
            OverlayAlignment::Center => "",
            OverlayAlignment::End => "End",
        };
        let name = match (v, h) {
            ("Center", "") => "Center".to_string(),
            ("Center", h) => format!("Center{h}"),
            (v, "") => format!("{v}Center"),
            (v, h) => format!("{v}{h}"),
        };
        format!("Alignment.{name}")
    }

    fn grid_cells(&mut self, node: StableId, grid: &GridLayout) -> String {
        let n = grid.columns.len();
        let first = grid.columns.first().copied();
        let uniform_fraction = grid
            .columns
            .iter()
            .all(|track| matches!(track, TrackSize::Fraction(_)) && Some(*track) == first);
        let uniform_fixed = grid
            .columns
            .iter()
            .all(|track| matches!(track, TrackSize::Fixed(_)) && Some(*track) == first);
        if !grid.rows.is_empty() {
            self.layout_note(node, true, LayoutFacet::GridTracks);
        }
        if uniform_fixed {
            if let Some(TrackSize::Fixed(length)) = first {
                return format!("GridCells.FixedSize({})", dp(length.get()));
            }
        }
        if !uniform_fraction {
            self.layout_note(node, true, LayoutFacet::GridTracks);
        }
        format!("GridCells.Fixed({n})")
    }

    /// 축 하나의 sizing modifier(부모 scope 반영).
    fn axis_size(
        &mut self,
        node: &IrNode,
        layout: &IrLayout,
        axis: Axis,
        scope: Scope,
    ) -> Option<String> {
        let (sizing, fill_name) = match axis {
            Axis::Horizontal => (layout.width, "fillMaxWidth"),
            Axis::Vertical => (layout.height, "fillMaxHeight"),
        };
        let main = matches!(scope, Scope::Stack(main_axis, _) if main_axis == axis);
        let stretch =
            matches!(scope, Scope::Stack(main_axis, CrossAlignment::Stretch) if main_axis != axis);
        match sizing {
            SizingMode::Fixed(_) => None,
            SizingMode::Fill if main => {
                let grow = layout.flex.grow.get();
                let weight = if grow > 0.0 { grow } else { 1.0 };
                Some(format!("weight({})", float(weight)))
            }
            SizingMode::Fill => Some(format!("{fill_name}()")),
            SizingMode::Percentage(percentage) => {
                if main {
                    self.layout_note(node.id, true, LayoutFacet::Sizing);
                }
                Some(format!("{fill_name}({})", float(percentage.get() / 100.0)))
            }
            SizingMode::HugContent if main && layout.flex.grow.get() > 0.0 => {
                self.layout_note(node.id, true, LayoutFacet::Flex);
                Some(format!("weight({})", float(layout.flex.grow.get())))
            }
            SizingMode::HugContent if stretch => Some(format!("{fill_name}()")),
            SizingMode::HugContent => None,
        }
    }

    fn size_modifiers(&mut self, node: &IrNode, layout: &IrLayout, scope: Scope) -> Vec<String> {
        let mut out = Vec::new();
        match (layout.width, layout.height) {
            (SizingMode::Fixed(w), SizingMode::Fixed(h)) => {
                out.push(format!("size({}, {})", dp(w.get()), dp(h.get())))
            }
            (SizingMode::Fixed(w), _) => out.push(format!("width({})", dp(w.get()))),
            (_, SizingMode::Fixed(h)) => out.push(format!("height({})", dp(h.get()))),
            _ => {}
        }
        if let Some(modifier) = self.axis_size(node, layout, Axis::Horizontal, scope) {
            out.push(modifier);
        }
        if let Some(modifier) = self.axis_size(node, layout, Axis::Vertical, scope) {
            out.push(modifier);
        }
        // weight는 부모 scope modifier이므로 앞쪽에 둡니다.
        out.sort_by_key(|m| !m.starts_with("weight("));
        let limits = layout.limits;
        let pair = |min: Option<stellar_core_types::value::Length>,
                    max: Option<stellar_core_types::value::Length>| {
            let mut args = Vec::new();
            if let Some(min) = min {
                args.push(format!("min = {}", dp(min.get())));
            }
            if let Some(max) = max {
                args.push(format!("max = {}", dp(max.get())));
            }
            args
        };
        let width = pair(limits.min_width, limits.max_width);
        if !width.is_empty() {
            out.push(format!("widthIn({})", width.join(", ")));
        }
        let height = pair(limits.min_height, limits.max_height);
        if !height.is_empty() {
            out.push(format!("heightIn({})", height.join(", ")));
        }
        if let Some(ratio) = layout.aspect_ratio {
            out.push(format!(
                "aspectRatio({})",
                float(ratio.width.get() / ratio.height.get())
            ));
        }
        out
    }

    fn position_modifiers(
        &mut self,
        node: &IrNode,
        layout: &IrLayout,
        scope: Scope,
    ) -> Vec<String> {
        let mut out = Vec::new();
        let (mut x, mut y) = (0.0f32, 0.0f32);
        if scope == Scope::Free {
            let axis_offset = |anchors: &AxisAnchors, mapper: &mut Mapper<'_>| -> (f32, bool) {
                let length = |offset: Option<Offset>, mapper: &mut Mapper<'_>| match offset {
                    Some(Offset::Length(length)) => Some(length.get()),
                    Some(Offset::Percentage(_)) => {
                        mapper.layout_note(node.id, false, LayoutFacet::Position);
                        None
                    }
                    None => None,
                };
                match (anchors.start, anchors.end) {
                    (Some(_), Some(_)) => {
                        mapper.layout_note(node.id, true, LayoutFacet::Position);
                        (length(anchors.start, mapper).unwrap_or(0.0), false)
                    }
                    (Some(_), None) => (length(anchors.start, mapper).unwrap_or(0.0), false),
                    (None, Some(_)) => (-length(anchors.end, mapper).unwrap_or(0.0), true),
                    (None, None) => (0.0, false),
                }
            };
            let (dx, end_x) = axis_offset(&layout.position.horizontal, self);
            let (dy, end_y) = axis_offset(&layout.position.vertical, self);
            x += dx;
            y += dy;
            let alignment = match (end_y, end_x) {
                (false, false) => None,
                (false, true) => Some("TopEnd"),
                (true, false) => Some("BottomStart"),
                (true, true) => Some("BottomEnd"),
            };
            if let Some(alignment) = alignment {
                out.push(format!("align(Alignment.{alignment})"));
            }
        }
        if layout.transform != Transform2D::identity() {
            let (a, b, c, d, tx, ty) = layout.transform.components();
            if (a, b, c, d) == (1.0, 0.0, 0.0, 1.0) {
                x += tx;
                y += ty;
            } else {
                self.layout_note(node.id, false, LayoutFacet::Transform);
            }
        }
        if x != 0.0 || y != 0.0 {
            out.push(format!("offset(x = {}, y = {})", dp(x), dp(y)));
        }
        out
    }

    fn padding_modifier(layout: &IrLayout) -> Option<String> {
        let (top, right, bottom, left) = layout.padding.components();
        if top == 0.0 && right == 0.0 && bottom == 0.0 && left == 0.0 {
            return None;
        }
        if top == right && right == bottom && bottom == left {
            return Some(format!("padding({})", dp(top)));
        }
        Some(format!(
            "absolutePadding(left = {}, top = {}, right = {}, bottom = {})",
            dp(left),
            dp(top),
            dp(right),
            dp(bottom)
        ))
    }

    /// node의 접근성·control 의미 → (상호작용 modifier, semantics 문장, 추가 인자).
    fn semantics(
        &mut self,
        index: u32,
        entry: Option<&IrAccessibilityEntry>,
        call_name: &'static str,
    ) -> (Vec<String>, Vec<String>, Vec<(String, String)>) {
        let ir = self.ir;
        let node = &ir.nodes[index as usize];
        let id = node.id;
        let mut interaction = Vec::new();
        let mut statements = Vec::new();
        let mut args = Vec::new();
        let parent_has = node.parent.is_none_or(|p| self.has_semantics[p as usize]);
        let Some(entry) = entry else {
            if !self.has_semantics[index as usize] && parent_has {
                interaction.push("clearAndSetSemantics { }".to_string());
            }
            return (interaction, statements, args);
        };
        let semantics = &entry.semantics;
        let role = semantics.role;
        // disabled.
        let disabled = semantics
            .states
            .get(&StateKind::Disabled)
            .and_then(|binding| self.flag_expr(id, binding, Facet::State(StateKind::Disabled)));
        let enabled_arg = disabled.as_ref().map(|expr| match expr.as_str() {
            "true" => "false".to_string(),
            "false" => "true".to_string(),
            other => format!("!({other})"),
        });
        // 기본 동작 문장(activate + select).
        let mut on_click: Vec<String> = Vec::new();
        let mut dismiss: Option<String> = None;
        for (action, target) in &semantics.actions {
            match (action, target) {
                (ActionKind::Activate, ActionTarget::SemanticAction(action_id)) => {
                    self.uses_action = true;
                    on_click.push(format!(
                        "onAction({})",
                        string_literal(&action_id.to_string())
                    ));
                }
                (ActionKind::Select, ActionTarget::State(state)) => {
                    match self.state_name(*state, KotlinType::String) {
                        Some(name) => {
                            on_click.push(format!("{name} = {}", string_literal(&id.to_string())))
                        }
                        None => self.note(
                            id,
                            ComposeNoteKind::UnresolvedBinding(Facet::Action(*action)),
                        ),
                    }
                }
                (ActionKind::Dismiss, ActionTarget::SemanticAction(action_id)) => {
                    self.uses_action = true;
                    dismiss = Some(format!(
                        "onAction({})",
                        string_literal(&action_id.to_string())
                    ));
                }
                (ActionKind::Focus | ActionKind::SetValue, _) => {}
                _ => self.note(
                    id,
                    ComposeNoteKind::NoEquivalent(ComposeFacet::Semantics(Facet::Action(*action))),
                ),
            }
        }
        let click_body = if on_click.is_empty() {
            "{ }".to_string()
        } else {
            format!("{{ {} }}", on_click.join("; "))
        };
        let mut native_focus = false;
        let mut selected_handled = false;
        match role {
            AccessibilityRole::Button => {
                let mut parts = Vec::new();
                if let Some(enabled) = &enabled_arg {
                    parts.push(format!("enabled = {enabled}"));
                }
                parts.push("role = Role.Button".to_string());
                parts.push(format!("onClick = {click_body}"));
                interaction.push(format!("clickable({})", parts.join(", ")));
                native_focus = true;
                if let Some(pressed) = semantics.states.get(&StateKind::Pressed) {
                    if let Some(expr) =
                        self.flag_expr(id, pressed, Facet::State(StateKind::Pressed))
                    {
                        statements.push(format!("toggleableState = ToggleableState({expr})"));
                    }
                }
            }
            AccessibilityRole::Radio | AccessibilityRole::ListItem
                if semantics.actions.contains_key(&ActionKind::Select) =>
            {
                let state = if role == AccessibilityRole::Radio {
                    StateKind::Checked
                } else {
                    StateKind::Selected
                };
                let selected = semantics
                    .states
                    .get(&state)
                    .and_then(|binding| self.flag_expr(id, binding, Facet::State(state)))
                    .unwrap_or_else(|| "false".to_string());
                let mut parts = vec![format!("selected = {selected}")];
                if let Some(enabled) = &enabled_arg {
                    parts.push(format!("enabled = {enabled}"));
                }
                if role == AccessibilityRole::Radio {
                    parts.push("role = Role.RadioButton".to_string());
                }
                parts.push(format!("onClick = {click_body}"));
                interaction.push(format!("selectable({})", parts.join(", ")));
                native_focus = true;
                selected_handled = true;
            }
            AccessibilityRole::RadioGroup => interaction.push("selectableGroup()".to_string()),
            AccessibilityRole::TextField => {
                native_focus = true;
                if let Some(enabled) = &enabled_arg {
                    args.push(("enabled".to_string(), enabled.clone()));
                }
                if let Some(editable) = semantics.states.get(&StateKind::Editable) {
                    if let Some(expr) =
                        self.flag_expr(id, editable, Facet::State(StateKind::Editable))
                    {
                        match expr.as_str() {
                            "true" => {}
                            "false" => args.push(("readOnly".to_string(), "true".to_string())),
                            other => args.push(("readOnly".to_string(), format!("!({other})"))),
                        }
                    }
                }
            }
            _ => {}
        }
        if role == AccessibilityRole::List {
            let items = ir
                .accessibility_children(node.semantics.unwrap_or_default())
                .into_iter()
                .filter(|child| {
                    ir.accessibility[*child as usize].semantics.role == AccessibilityRole::ListItem
                })
                .count();
            let selectable = ir
                .accessibility_children(node.semantics.unwrap_or_default())
                .into_iter()
                .any(|child| {
                    ir.accessibility[child as usize]
                        .semantics
                        .actions
                        .contains_key(&ActionKind::Select)
                });
            if selectable {
                interaction.push("selectableGroup()".to_string());
            }
            statements.push(format!(
                "collectionInfo = CollectionInfo(rowCount = {items}, columnCount = 1)"
            ));
        }
        if role == AccessibilityRole::ListItem {
            let row = entry
                .parent
                .map(|parent| {
                    ir.accessibility_children(parent)
                        .into_iter()
                        .filter(|child| {
                            ir.accessibility[*child as usize].semantics.role
                                == AccessibilityRole::ListItem
                        })
                        .position(|child| ir.accessibility[child as usize].node == index)
                        .unwrap_or(0)
                })
                .unwrap_or(0);
            statements.push(format!(
                "collectionItemInfo = CollectionItemInfo(rowIndex = {row}, rowSpan = 1, columnIndex = 0, columnSpan = 1)"
            ));
        }
        // 이름.
        if let Some(label) = &semantics.label {
            let own_text = matches!(label, AccessibilityBinding::NodeText(target) if *target == id);
            if !(own_text && call_name == "BasicText") {
                if let Some(expr) = self.text_expr(id, label, Facet::Label) {
                    if role == AccessibilityRole::Dialog {
                        statements.push(format!("paneTitle = {expr}"));
                    } else {
                        statements.push(format!("contentDescription = {expr}"));
                    }
                }
            }
        }
        if role == AccessibilityRole::Image {
            statements.push("role = Role.Image".to_string());
        }
        if let Some(value) = &semantics.value {
            if role != AccessibilityRole::TextField {
                if let Some(expr) = self.text_expr(id, value, Facet::Value) {
                    statements.push(format!("stateDescription = {expr}"));
                }
            }
        }
        for (state, binding) in &semantics.states {
            let facet = Facet::State(*state);
            match state {
                StateKind::Disabled => {
                    if enabled_arg.is_none()
                        || !(native_focus || role == AccessibilityRole::TextField)
                    {
                        if let Some(expr) = &disabled {
                            match expr.as_str() {
                                "false" => {}
                                "true" => statements.push("disabled()".to_string()),
                                other => statements.push(format!("if ({other}) disabled()")),
                            }
                        }
                    }
                }
                StateKind::Selected | StateKind::Checked if !selected_handled => {
                    if let Some(expr) = self.flag_expr(id, binding, facet) {
                        statements.push(format!("selected = {expr}"));
                    }
                }
                StateKind::Focused => {
                    if let Some(expr) = self.flag_expr(id, binding, facet) {
                        statements.push(format!("focused = {expr}"));
                    }
                }
                StateKind::Focusable => {
                    if !native_focus && self.ir.static_flag(binding) == Some(true) {
                        interaction.push("focusable()".to_string());
                    }
                }
                StateKind::Modal => {
                    if role != AccessibilityRole::Dialog {
                        self.note(
                            id,
                            ComposeNoteKind::NoEquivalent(ComposeFacet::Semantics(facet)),
                        );
                    }
                }
                StateKind::Multiselectable => self.note(
                    id,
                    ComposeNoteKind::NoEquivalent(ComposeFacet::Semantics(facet)),
                ),
                _ => {}
            }
        }
        if role == AccessibilityRole::Group
            && semantics.label.is_some()
            && self.supports_traversal()
        {
            self.traversal_group[index as usize] = true;
        }
        if role == AccessibilityRole::Dialog {
            if let Some(dismiss) = dismiss {
                args.push(("onDismissRequest".to_string(), format!("{{ {dismiss} }}")));
            } else {
                args.push(("onDismissRequest".to_string(), "{ }".to_string()));
            }
        } else if dismiss.is_some() {
            self.note(
                id,
                ComposeNoteKind::NoEquivalent(ComposeFacet::Semantics(Facet::Action(
                    ActionKind::Dismiss,
                ))),
            );
        }
        (interaction, statements, args)
    }

    /// node 하나의 호출을 만들고 index를 돌려줍니다(보이지 않으면 `None`).
    fn map_node(&mut self, index: u32, scope: Scope) -> Option<usize> {
        let ir = self.ir;
        let node = &ir.nodes[index as usize];
        let layout = node.layout.as_ref()?;
        if !layout.visible && node.parent.is_some() {
            return None;
        }
        let entry = ir.semantics_of(index);
        let role = entry.map(|e| e.semantics.role);
        let id = node.id;

        // 호출 종류와 content scope.
        let mut call = Call::default();
        let mut child_scope = Scope::Box;
        let leaf_content = node.children.is_empty();
        if role == Some(AccessibilityRole::TextField) {
            call.name = "BasicTextField";
            call.leaf = true;
            if !leaf_content {
                self.layout_note(id, true, LayoutFacet::Content);
            }
            let value = entry
                .and_then(|e| e.semantics.value.as_ref())
                .and_then(|binding| match ir.resolve_binding(binding) {
                    ResolvedBinding::State(state) => self.state_name(state.id, KotlinType::String),
                    _ => None,
                });
            match value {
                Some(name) => {
                    call.args.push(("value".to_string(), name.clone()));
                    call.args
                        .push(("onValueChange".to_string(), format!("{{ {name} = it }}")));
                }
                None => {
                    self.note(id, ComposeNoteKind::UnresolvedBinding(Facet::Value));
                    call.args.push(("value".to_string(), "\"\"".to_string()));
                    call.args
                        .push(("onValueChange".to_string(), "{ }".to_string()));
                }
            }
        } else if node.element == ElementKind::TEXT {
            call.name = "BasicText";
            call.leaf = true;
            let text = match &node.text {
                Some(text) => string_literal(text),
                None => {
                    self.layout_note(id, true, LayoutFacet::Content);
                    "\"\"".to_string()
                }
            };
            call.args.push(("text".to_string(), text));
        } else {
            match &layout.container {
                ContainerLayout::Free => {
                    call.name = "Box";
                    child_scope = Scope::Free;
                }
                ContainerLayout::Stack(stack) => {
                    let vertical = axis_main_is_vertical(stack.axis);
                    let wrap = stack.wrap == StackWrap::Wrap;
                    if wrap {
                        self.uses_flow = true;
                        if stack.line_alignment != stellar_layout::stack::LineAlignment::Start {
                            self.layout_note(id, true, LayoutFacet::Wrap);
                        }
                    }
                    call.name = match (vertical, wrap) {
                        (true, false) => "Column",
                        (false, false) => "Row",
                        (true, true) => "FlowColumn",
                        (false, true) => "FlowRow",
                    };
                    let main = self.main_arrangement(id, stack);
                    let (main_name, cross_name) = if vertical {
                        ("verticalArrangement", "horizontalAlignment")
                    } else {
                        ("horizontalArrangement", "verticalAlignment")
                    };
                    call.args.push((main_name.to_string(), main));
                    if wrap {
                        let cross_arrangement = if stack.cross_gap.get() > 0.0 {
                            format!("Arrangement.spacedBy({})", dp(stack.cross_gap.get()))
                        } else if vertical {
                            "Arrangement.Start".to_string()
                        } else {
                            "Arrangement.Top".to_string()
                        };
                        let cross_arg = if vertical {
                            "horizontalArrangement"
                        } else {
                            "verticalArrangement"
                        };
                        call.args.push((cross_arg.to_string(), cross_arrangement));
                        if stack.cross_alignment != CrossAlignment::Start {
                            self.layout_note(id, true, LayoutFacet::Wrap);
                        }
                    } else {
                        call.args
                            .push((cross_name.to_string(), Mapper::cross_alignment(stack)));
                    }
                    if stack.direction == StackDirection::Reverse {
                        self.layout_note(id, true, LayoutFacet::Reverse);
                    }
                    child_scope = Scope::Stack(stack.axis, stack.cross_alignment);
                }
                ContainerLayout::Grid(grid) => {
                    call.name = "LazyVerticalGrid";
                    let cells = self.grid_cells(id, grid);
                    call.args.push(("columns".to_string(), cells));
                    if grid.column_gap.get() > 0.0 {
                        call.args.push((
                            "horizontalArrangement".to_string(),
                            format!("Arrangement.spacedBy({})", dp(grid.column_gap.get())),
                        ));
                    }
                    if grid.row_gap.get() > 0.0 {
                        call.args.push((
                            "verticalArrangement".to_string(),
                            format!("Arrangement.spacedBy({})", dp(grid.row_gap.get())),
                        ));
                    }
                    child_scope = Scope::Grid;
                }
                ContainerLayout::Overlay(overlay) => {
                    call.name = "Box";
                    let alignment = Mapper::overlay_alignment(overlay.horizontal, overlay.vertical);
                    if alignment != "Alignment.TopStart" {
                        call.args.push(("contentAlignment".to_string(), alignment));
                    }
                }
                ContainerLayout::Constraint => {
                    call.name = "Box";
                    self.layout_note(id, false, LayoutFacet::Constraint);
                }
            }
            if node.element == ElementKind::IMAGE {
                self.layout_note(id, false, LayoutFacet::Content);
            }
            call.leaf = node.children.is_empty();
        }

        // modifier: 부모 scope → 크기 → 상호작용·semantics → padding.
        let (interaction, mut statements, extra_args) = self.semantics(index, entry, call.name);
        let dialog = role == Some(AccessibilityRole::Dialog);
        // Dialog content에는 부모 scope가 없습니다.
        let own_scope = if dialog { Scope::None } else { scope };
        let mut modifier = self.position_modifiers(node, layout, own_scope);
        modifier.extend(self.size_modifiers(node, layout, own_scope));
        if self.traversal_group[index as usize] {
            statements.insert(0, "isTraversalGroup = true".to_string());
        }
        if let Some(position) = self.traversal_index.get(&index) {
            statements.push(format!("traversalIndex = {}", float(*position as f32)));
        }
        modifier.extend(interaction);
        if !statements.is_empty() {
            modifier.push(format!("semantics {{\n{}\n}}", statements.join("\n")));
        }
        if let Some(padding) = Mapper::padding_modifier(layout) {
            modifier.push(padding);
        }
        call.modifier = modifier;
        let (dialog_args, own_args): (Vec<_>, Vec<_>) = extra_args
            .into_iter()
            .partition(|(name, _)| name == "onDismissRequest");
        call.args.extend(own_args);

        let call_index = self.calls.len();
        self.calls.push(call);

        // 자식(Fragment 펼침, visual order).
        let mut children = Vec::new();
        let mut stack: Vec<u32> = node.children.iter().rev().copied().collect();
        while let Some(child) = stack.pop() {
            let child_node = &ir.nodes[child as usize];
            if child_node.element == ElementKind::FRAGMENT {
                stack.extend(child_node.children.iter().rev().copied());
                continue;
            }
            children.push(child);
        }
        if let Some(ContainerLayout::Stack(stack)) = node.layout.as_ref().map(|l| &l.container) {
            if stack.direction == StackDirection::Reverse {
                children.reverse();
            }
        }
        let mut grid_cursor = 0u16;
        let columns = match &layout.container {
            ContainerLayout::Grid(grid) => grid.columns.len() as u16,
            _ => 0,
        };
        let mut spans = Vec::new();
        let mut child_calls = Vec::new();
        for child in children {
            if child_scope == Scope::Grid {
                if let Some(child_layout) = ir.nodes[child as usize].layout.as_ref() {
                    let span = child_layout.grid.column.span().max(1);
                    if let Some(start) = child_layout.grid.column.start_index() {
                        if start as u16 != grid_cursor % columns.max(1) {
                            self.layout_note(
                                ir.nodes[child as usize].id,
                                true,
                                LayoutFacet::GridPlacement,
                            );
                        }
                    }
                    if !matches!(child_layout.grid.row, GridPlacement::Auto { span: 1 })
                        && child_layout.grid.row.start_index()
                            != Some((grid_cursor / columns.max(1)) as usize)
                    {
                        self.layout_note(
                            ir.nodes[child as usize].id,
                            true,
                            LayoutFacet::GridPlacement,
                        );
                    }
                    grid_cursor = grid_cursor.saturating_add(span);
                    if let Some(call) = self.map_node(child, child_scope) {
                        child_calls.push(call);
                        spans.push(span);
                    }
                    continue;
                }
            }
            if let Some(call) = self.map_node(child, child_scope) {
                child_calls.push(call);
                spans.push(1);
            }
        }
        self.calls[call_index].children = child_calls;
        if child_scope == Scope::Grid {
            self.calls[call_index].grid_spans = spans;
        }
        // 자식 없는 Box는 content 없는 `Box(modifier)` overload만 있어 contentAlignment를 쓰지 않고 modifier를 반드시
        // 넘깁니다. Row·Column·Flow·Grid는 content lambda가 필수라 빈 lambda를 씁니다.
        let call = &mut self.calls[call_index];
        if call.children.is_empty() {
            if call.name == "Box" {
                call.leaf = true;
                call.args.retain(|(name, _)| name != "contentAlignment");
            } else if is_content_container(call.name) {
                call.leaf = false;
            }
        }

        if dialog {
            let wrapper = Call {
                name: "Dialog",
                args: dialog_args,
                children: vec![call_index],
                ..Call::default()
            };
            let wrapper_index = self.calls.len();
            self.calls.push(wrapper);
            return Some(wrapper_index);
        }
        Some(call_index)
    }
}

fn is_content_container(name: &str) -> bool {
    matches!(
        name,
        "Row" | "Column" | "FlowRow" | "FlowColumn" | "LazyVerticalGrid" | "Dialog"
    )
}

/// 매핑 결과(본문 소스와 사용 정보).
pub(crate) struct ComposeBody {
    /// `@Composable` 함수 본문(들여쓰기 4칸 기준).
    pub body: String,
    /// `onAction` 매개변수 필요 여부.
    pub uses_action: bool,
    /// `@OptIn(ExperimentalLayoutApi::class)` 필요 여부.
    pub uses_flow: bool,
    /// 노트.
    pub notes: Vec<ComposeNote>,
}

/// IR을 composable 함수 본문으로 매핑합니다.
pub(crate) fn map_compose(ir: &CodegenIr, ui_version: (u16, u16)) -> ComposeBody {
    let typed: Vec<(StableId, &str, Option<KotlinType>)> = ir
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
    let states: BTreeMap<StableId, (String, KotlinType)> = typed
        .iter()
        .zip(names)
        .filter_map(|((id, _, ty), name)| ty.map(|ty| (*id, (name, ty))))
        .collect();
    let mut mapper = Mapper {
        ir,
        ui_version,
        states,
        calls: Vec::new(),
        notes: Vec::new(),
        uses_action: false,
        uses_flow: false,
        traversal_index: BTreeMap::new(),
        traversal_group: vec![false; ir.nodes.len()],
        has_semantics: ir.semantic_subtrees(),
    };
    mapper.plan_reading_order();
    let root = mapper.map_node(0, Scope::None);
    let mut body = String::new();
    for state in &ir.states {
        let Some((name, ty)) = mapper.states.get(&state.id) else {
            continue;
        };
        let initial = match (&state.initial, ty) {
            (TypedValue::String(text), KotlinType::String) => string_literal(text),
            (TypedValue::Boolean(value), KotlinType::Boolean) => value.to_string(),
            (TypedValue::Number(value), KotlinType::Double) => double(value.get()),
            _ => continue,
        };
        let _ = writeln!(
            body,
            "    var {name} by remember {{ mutableStateOf({initial}) }}"
        );
    }
    if let Some(root) = root {
        write_tree(&mut body, &mapper.calls, root);
    }
    let mut notes = mapper.notes;
    let order = |id: StableId| ir.node_index(id).unwrap_or(u32::MAX);
    notes.sort_by_key(|note| (order(note.node), note.kind));
    notes.dedup();
    ComposeBody {
        body,
        uses_action: mapper.uses_action,
        uses_flow: mapper.uses_flow,
        notes,
    }
}

/// 호출 tree를 씁니다(재귀 없이 stack으로).
fn write_tree(out: &mut String, calls: &[Call], root: usize) {
    enum Step {
        Open(usize, usize, Option<u16>),
        Close(usize, bool),
    }
    let mut stack = vec![Step::Open(root, 1, None)];
    while let Some(step) = stack.pop() {
        match step {
            Step::Close(depth, item) => {
                let _ = writeln!(out, "{}}}", "    ".repeat(depth));
                if item {
                    let _ = writeln!(out, "{}}}", "    ".repeat(depth - 1));
                }
            }
            Step::Open(index, outer_depth, span) => {
                let call = &calls[index];
                if let Some(span) = span {
                    let indent = "    ".repeat(outer_depth);
                    if span > 1 {
                        let _ = writeln!(out, "{indent}item(span = {{ GridItemSpan({span}) }}) {{");
                    } else {
                        let _ = writeln!(out, "{indent}item {{");
                    }
                }
                let depth = if span.is_some() {
                    outer_depth + 1
                } else {
                    outer_depth
                };
                let indent = "    ".repeat(depth);
                let mut args: Vec<String> = Vec::new();
                if !call.modifier.is_empty() {
                    let inner = "    ".repeat(depth + 2);
                    let mut chain = String::from("modifier = Modifier");
                    for modifier in &call.modifier {
                        let lines: Vec<&str> = modifier.lines().collect();
                        let _ = write!(chain, "\n{inner}.{}", lines[0]);
                        if lines.len() > 1 {
                            for line in &lines[1..lines.len() - 1] {
                                let _ = write!(chain, "\n{inner}    {line}");
                            }
                            let _ = write!(chain, "\n{inner}{}", lines[lines.len() - 1]);
                        }
                    }
                    args.push(chain);
                }
                if call.name == "Box" && call.leaf && call.modifier.is_empty() {
                    args.push("modifier = Modifier".to_string());
                }
                args.extend(
                    call.args
                        .iter()
                        .map(|(name, value)| format!("{name} = {value}")),
                );
                let _ = write!(out, "{indent}{}", call.name);
                if !args.is_empty() {
                    out.push_str("(\n");
                    for arg in &args {
                        let _ = writeln!(out, "{}{arg},", "    ".repeat(depth + 1));
                    }
                    let _ = write!(out, "{indent})");
                } else if call.leaf {
                    out.push_str("()");
                }
                if call.leaf {
                    out.push('\n');
                    if span.is_some() {
                        let _ = writeln!(out, "{}}}", "    ".repeat(outer_depth));
                    }
                    continue;
                }
                out.push_str(" {\n");
                stack.push(Step::Close(depth, span.is_some()));
                for (position, child) in call.children.iter().enumerate().rev() {
                    let child_span = if call.grid_spans.is_empty() {
                        None
                    } else {
                        call.grid_spans.get(position).copied()
                    };
                    stack.push(Step::Open(*child, depth + 1, child_span));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_alignment_names_follow_compose_alignment_constants() {
        use OverlayAlignment::{Center, End, Start};
        let cases = [
            ((Start, Start), "Alignment.TopStart"),
            ((Center, Start), "Alignment.TopCenter"),
            ((End, Start), "Alignment.TopEnd"),
            ((Start, Center), "Alignment.CenterStart"),
            ((Center, Center), "Alignment.Center"),
            ((End, Center), "Alignment.CenterEnd"),
            ((Start, End), "Alignment.BottomStart"),
            ((Center, End), "Alignment.BottomCenter"),
            ((End, End), "Alignment.BottomEnd"),
        ];
        for ((horizontal, vertical), expected) in cases {
            assert_eq!(Mapper::overlay_alignment(horizontal, vertical), expected);
        }
    }
}
