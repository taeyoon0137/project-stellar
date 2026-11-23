//! AppKit view tree: Control contract·layout·상태 (XXII.3.1·XXII.3.2, ADR-0107).
//!
//! 출력은 `@MainActor final class <prefix>View: NSView`입니다. 상태 변수는 `didSet`에서 접근성을 다시 적용하는 속성,
//! semantic action은 생성자에서 받는 `onAction`입니다. node마다 `makeNode<StableId 16진>() -> NSView`를 두고,
//! XXII.3.3 접근성 함수(`<prefix>Accessibility.apply…`)를 `applyAccessibility()`에서 부릅니다.
//!
//! | Control contract | AppKit |
//! |---|---|
//! | Button | 자식 layout view + `NSClickGestureRecognizer`(비활성 상태면 동작하지 않음). `NSButton`은 디자인한 자식 view를 담지 못함 |
//! | TextInput | `NSTextField(string:)` + `placeholderString`, 입력 확정 시 상태 쓰기 |
//! | Modal | 그 자리에 내용 container(노트) |
//! | List·SingleSelectionGroup | 자식 layout container, 의미는 접근성 함수 |
//!
//! layout 규칙은 [`crate::apple::layout::APPKIT_RULES`]입니다.

use std::fmt::Write as _;

use stellar_accessibility::node::{ActionKind, ActionTarget, StateKind};
use stellar_core_types::typed_value::TypedValue;
use stellar_layout::style::{
    BoxSizing, ContentAlignment, FlexBasis, FlexWrap, ItemAlignment, LayoutStyle,
    LengthPercentageAuto, MaxSize, Position, Size,
};
use stellar_target_profile::builtin::properties as p;
use stellar_target_profile::capability::{CapabilityStatus, CapabilitySubject};

use crate::apple::accessibility::{AccessibilityHook, StateNames, SwiftType};
use crate::apple::layout::{appkit_stack_alignment, approximated, unsupported, LayoutFinding};
use crate::apple::native::{
    grow_is_basis_zero, item_edge, justify_alignment, justify_edge, self_alignment, size, style,
    track_kind, Analysis, Axis, Container, Control, Edge, GridPlan, NodeInfo, TrackKind, ViewKind,
};
use crate::apple::swift::{number, string_literal};
use crate::apple::swiftui::initial_literal;
use crate::apple::{AppleNote, AppleNoteKind, AppleNoteSubject, ControlNote};
use crate::facet::Facet;
use crate::ir::snapshot::id_hex;
use crate::ir::{CodegenIr, ResolvedBinding};

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
    /// 노트.
    pub notes: &'a mut Vec<AppleNote>,
}

fn px(value: f32) -> String {
    number(value)
}

fn anchor(axis: Axis) -> &'static str {
    match axis {
        Axis::Horizontal => "widthAnchor",
        Axis::Vertical => "heightAnchor",
    }
}

fn orientation(axis: Axis) -> &'static str {
    match axis {
        Axis::Horizontal => ".horizontal",
        Axis::Vertical => ".vertical",
    }
}

fn placement(alignment: ItemAlignment, horizontal: bool) -> &'static str {
    match (item_edge(alignment), horizontal) {
        _ if matches!(alignment, ItemAlignment::Normal | ItemAlignment::Stretch) => ".fill",
        (Edge::Start, true) => ".leading",
        (Edge::Start, false) => ".top",
        (Edge::Center, _) => ".center",
        (Edge::End, true) => ".trailing",
        (Edge::End, false) => ".bottom",
    }
}

/// 함수 본문 줄 모음.
#[derive(Default)]
struct Body {
    lines: Vec<String>,
    padded: bool,
}

impl Body {
    fn push(&mut self, line: impl Into<String>) {
        self.lines.push(line.into());
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

    fn make(&self, index: u32) -> String {
        format!("makeNode{}", id_hex(self.ir.nodes[index as usize].id))
    }

    fn style(&self, index: u32) -> &LayoutStyle {
        style(self.ir, index).expect("출력 node는 layout이 있음")
    }

    fn info(&self, index: u32) -> &NodeInfo {
        &self.analysis.nodes[index as usize]
    }

    /// 크기 제약(채움은 부모가 냅니다).
    fn size_constraints(&mut self, index: u32, body: &mut Body) {
        let s = self.style(index).clone();
        let info = self.info(index).clone();
        let (top, right, bottom, left) = s.padding.components();
        let extra = |axis: Axis| -> f32 {
            if s.box_sizing == BoxSizing::ContentBox {
                match axis {
                    Axis::Horizontal => left + right,
                    Axis::Vertical => top + bottom,
                }
            } else {
                0.0
            }
        };
        let main_basis = info
            .parent
            .and_then(|parent| match self.info(parent).container {
                Container::Flex { main, .. } if s.flex_grow.get() == 0.0 => Some(main),
                _ => None,
            });
        for (axis, name, min_name, max_name) in [
            (Axis::Horizontal, "width", "min-width", "max-width"),
            (Axis::Vertical, "height", "min-height", "max-height"),
        ] {
            let mut value = size(&s, axis);
            if main_basis == Some(axis) {
                match s.flex_basis {
                    FlexBasis::Length(length) => value = Size::Length(length),
                    FlexBasis::Percentage(_) => self.finding(
                        index,
                        unsupported(
                            "flex-basis",
                            "percent-size",
                            "AppKit 제약으로 부모 기준 백분율 크기를 내지 않습니다",
                        ),
                    ),
                    _ => {}
                }
            }
            if !info.fill.axes[axis.index()] {
                match value {
                    Size::Length(length) => body.push(format!(
                        "view.{}.constraint(equalToConstant: {}).isActive = true",
                        anchor(axis),
                        px(length.get() + extra(axis))
                    )),
                    Size::Auto | Size::MaxContent => {}
                    Size::FitContent => self.finding(
                        index,
                        approximated(
                            name,
                            "fit-content",
                            "fit-content는 Auto Layout 고유 크기로 냅니다",
                        ),
                    ),
                    Size::MinContent | Size::Percentage(_) => self.finding(
                        index,
                        unsupported(
                            name,
                            "intrinsic-size",
                            "AppKit 제약으로 백분율·min-content 크기를 내지 않습니다",
                        ),
                    ),
                }
            }
            let (min, max) = match axis {
                Axis::Horizontal => (s.min_width, s.max_width),
                Axis::Vertical => (s.min_height, s.max_height),
            };
            match min {
                Size::Auto => {}
                Size::Length(length) => body.push(format!(
                    "view.{}.constraint(greaterThanOrEqualToConstant: {}).isActive = true",
                    anchor(axis),
                    px(length.get() + extra(axis))
                )),
                _ => self.finding(
                    index,
                    unsupported(
                        min_name,
                        "intrinsic-limit",
                        "AppKit 제약으로 백분율·intrinsic 최소 크기를 내지 않습니다",
                    ),
                ),
            }
            match max {
                MaxSize::None => {}
                MaxSize::Length(length) => body.push(format!(
                    "view.{}.constraint(lessThanOrEqualToConstant: {}).isActive = true",
                    anchor(axis),
                    px(length.get() + extra(axis))
                )),
                _ => self.finding(
                    index,
                    unsupported(
                        max_name,
                        "intrinsic-limit",
                        "AppKit 제약으로 백분율·intrinsic 최대 크기를 내지 않습니다",
                    ),
                ),
            }
        }
        if let Some(ratio) = s.aspect_ratio {
            body.push(format!(
                "view.widthAnchor.constraint(equalTo: view.heightAnchor, multiplier: {} / {}).isActive = true",
                px(ratio.width.get()),
                px(ratio.height.get())
            ));
        }
        if s.position == Position::Relative
            && [s.top, s.right, s.bottom, s.left]
                .iter()
                .any(|inset| *inset != LengthPercentageAuto::Auto)
        {
            self.finding(
                index,
                unsupported(
                    "position",
                    "relative-offset",
                    "AppKit view에는 배치 뒤 시각 이동 API가 없어 relative inset을 내지 않습니다",
                ),
            );
        }
        if let Some(layout) = &self.ir.nodes[index as usize].layout {
            if layout.transform != stellar_core_types::value::Transform2D::identity() {
                self.finding(
                    index,
                    unsupported(
                        "transform",
                        "transform",
                        "AppKit layer transform은 layout 때 다시 쓰일 수 있어 내지 않습니다",
                    ),
                );
            }
        }
    }

    /// 자식 cross 채움 제약.
    fn fill_constraint(&self, parent: u32, child: &str, axis: Axis) -> String {
        let (top, right, bottom, left) = self.style(parent).padding.components();
        let inset = match axis {
            Axis::Horizontal => left + right,
            Axis::Vertical => top + bottom,
        };
        format!(
            "{child}.{a}.constraint(equalTo: view.{a}, constant: {}).isActive = true",
            px(-inset),
            a = anchor(axis)
        )
    }

    fn stack(&mut self, index: u32, body: &mut Body) {
        let info = self.info(index).clone();
        let s = self.style(index).clone();
        let (main, reverse) = match info.container {
            Container::Flex { main, reverse } => (main, reverse),
            _ => (Axis::Vertical, false),
        };
        let block = !matches!(info.container, Container::Flex { .. });
        body.push("let view = NSStackView()");
        body.push(format!("view.orientation = {}", orientation(main)));
        let gap = if block {
            0.0
        } else {
            match main {
                Axis::Horizontal => s.column_gap.get(),
                Axis::Vertical => s.row_gap.get(),
            }
        };
        let baseline = !block && s.align_items == ItemAlignment::Baseline;
        let alignment = if block {
            ".leading"
        } else {
            appkit_stack_alignment(
                main,
                item_edge(s.align_items),
                baseline && main == Axis::Horizontal,
            )
        };
        body.push(format!("view.alignment = {alignment}"));
        body.push(format!("view.spacing = {}", px(gap)));
        let (top, right, bottom, left) = s.padding.components();
        if [top, right, bottom, left].iter().any(|v| *v != 0.0) {
            body.push(format!(
                "view.edgeInsets = NSEdgeInsets(top: {}, left: {}, bottom: {}, right: {})",
                px(top),
                px(left),
                px(bottom),
                px(right)
            ));
        }
        let mut children = info.flow.clone();
        if !block {
            if reverse {
                children.reverse();
                self.finding(
                    index,
                    approximated(
                        "flex-direction",
                        "reverse-reading-order",
                        "reverse 방향은 자식 순서를 뒤집어 내므로 보조 기술 읽기 순서도 뒤집힙니다",
                    ),
                );
            }
            if s.flex_wrap != FlexWrap::NoWrap {
                self.finding(
                    index,
                    unsupported(
                        "flex-wrap",
                        "wrap",
                        "NSStackView는 줄바꿈 배치가 없어 한 줄로 냅니다",
                    ),
                );
            }
        }
        let growing: Vec<u32> = children
            .iter()
            .copied()
            .filter(|c| !block && self.info(*c).fill.axes[main.index()])
            .collect();
        let mut gravity = None;
        if !growing.is_empty() {
            let equal = growing.len() == children.len()
                && growing.iter().all(|c| {
                    let cs = self.style(*c);
                    cs.flex_grow == self.style(growing[0]).flex_grow && grow_is_basis_zero(cs, main)
                });
            if equal && growing.len() > 1 {
                body.push("view.distribution = .fillEqually");
            } else {
                body.push("view.distribution = .fill");
                if growing.len() > 1 {
                    self.finding(
                        index,
                        approximated(
                            "flex-grow",
                            "grow-distribution",
                            "NSStackView .fill은 남는 공간을 hugging priority가 가장 낮은 view 하나에 줍니다",
                        ),
                    );
                }
            }
        } else if !block {
            match s.justify_content {
                ContentAlignment::SpaceBetween => body.push("view.distribution = .equalSpacing"),
                ContentAlignment::SpaceAround | ContentAlignment::SpaceEvenly => {
                    self.finding(
                        index,
                        approximated(
                            "justify-content",
                            "spread",
                            "space-around·space-evenly는 NSStackView .equalCentering으로 근사합니다",
                        ),
                    );
                    body.push("view.distribution = .equalCentering");
                }
                value => {
                    body.push("view.distribution = .gravityAreas");
                    gravity = Some(match justify_edge(value, reverse) {
                        Edge::Start => ".leading",
                        Edge::Center => ".center",
                        Edge::End => ".trailing",
                    });
                }
            }
        }
        let stack_edge = item_edge(s.align_items);
        for (position, child) in children.iter().enumerate() {
            let name = format!("child{}", position + 1);
            body.push(format!("let {name} = {}()", self.make(*child)));
            match gravity {
                Some(gravity) => body.push(format!("view.addView({name}, in: {gravity})")),
                None => body.push(format!("view.addArrangedSubview({name})")),
            }
            let child_info = self.info(*child).clone();
            let cross = main.cross();
            if child_info.fill.axes[cross.index()] {
                body.push(self.fill_constraint(index, &name, cross));
            } else if !block {
                let own = item_edge(self_alignment(&s, self.style(*child)));
                if own != stack_edge {
                    self.finding(
                        *child,
                        approximated(
                            "align-self",
                            "align-self",
                            "NSStackView에는 item별 정렬이 없어 부모 정렬로 냅니다",
                        ),
                    );
                }
            }
            if growing.contains(child) {
                body.push(format!(
                    "{name}.setContentHuggingPriority(.init(1), for: {})",
                    orientation(main)
                ));
            }
            if !block {
                let shrink = self.style(*child).flex_shrink.get();
                if shrink == 0.0 {
                    body.push(format!(
                        "{name}.setContentCompressionResistancePriority(.required, for: {})",
                        orientation(main)
                    ));
                } else if shrink != 1.0 {
                    self.finding(
                        *child,
                        approximated(
                            "flex-shrink",
                            "shrink-ratio",
                            "AppKit 압축은 비율을 받지 않아 flex-shrink 1처럼 냅니다",
                        ),
                    );
                }
            }
        }
    }

    fn grid(&mut self, index: u32, plan: &GridPlan, body: &mut Body) {
        let s = self.style(index).clone();
        if let Some(reason) = plan.fallback {
            self.finding(
                index,
                unsupported("grid-column-start", "grid-placement", reason),
            );
        }
        let rows = plan
            .cells
            .iter()
            .map(|cell| cell.row as usize + 1)
            .max()
            .unwrap_or(0);
        let columns = plan.columns.len();
        let mut names = Vec::new();
        for (position, cell) in plan.cells.iter().enumerate() {
            let name = format!("cell{}", position + 1);
            body.push(format!("let {name} = {}()", self.make(cell.child)));
            names.push(name);
        }
        let mut grid_rows = Vec::new();
        for row in 0..rows {
            let mut items = vec!["NSGridCell.emptyContentView".to_string(); columns];
            for (cell, name) in plan.cells.iter().zip(&names) {
                if cell.row as usize == row {
                    items[cell.column as usize] = name.clone();
                }
            }
            grid_rows.push(format!("[{}]", items.join(", ")));
        }
        body.push(format!(
            "let grid = NSGridView(views: [{}])",
            grid_rows.join(", ")
        ));
        body.push(format!("grid.rowSpacing = {}", px(s.row_gap.get())));
        body.push(format!("grid.columnSpacing = {}", px(s.column_gap.get())));
        body.push(format!(
            "grid.xPlacement = {}",
            placement(s.justify_items, true)
        ));
        body.push(format!(
            "grid.yPlacement = {}",
            placement(s.align_items, false)
        ));
        let mut flexible = false;
        for (column, track) in plan.columns.iter().enumerate() {
            match track_kind(track) {
                TrackKind::Fixed(width) => {
                    body.push(format!("grid.column(at: {column}).width = {}", px(width)))
                }
                TrackKind::Content => {}
                _ => flexible = true,
            }
        }
        for (row, track) in plan.rows.iter().enumerate() {
            match track_kind(track) {
                TrackKind::Fixed(height) => {
                    body.push(format!("grid.row(at: {row}).height = {}", px(height)))
                }
                TrackKind::Content => {}
                _ => flexible = true,
            }
        }
        if flexible {
            self.finding(
                index,
                approximated(
                    "grid-template-columns",
                    "flexible-track",
                    "NSGridView에는 fr·백분율 track이 없어 콘텐츠 크기로 냅니다",
                ),
            );
        }
        for (cell, name) in plan.cells.iter().zip(&names) {
            if cell.column_span > 1 {
                body.push(format!(
                    "grid.mergeCells(inHorizontalRange: NSRange(location: {}, length: {}), verticalRange: NSRange(location: {}, length: 1))",
                    cell.column, cell.column_span, cell.row
                ));
            }
            let cs = self.style(cell.child);
            let horizontal = justify_alignment(&s, cs);
            let vertical = self_alignment(&s, cs);
            if horizontal != s.justify_items {
                body.push(format!(
                    "grid.cell(for: {name})?.xPlacement = {}",
                    placement(horizontal, true)
                ));
            }
            if vertical != s.align_items {
                body.push(format!(
                    "grid.cell(for: {name})?.yPlacement = {}",
                    placement(vertical, false)
                ));
            }
        }
        if s.justify_content != ContentAlignment::Normal
            || s.align_content != ContentAlignment::Normal
        {
            self.finding(
                index,
                approximated(
                    "align-content",
                    "grid-content-alignment",
                    "NSGridView는 track 묶음 정렬이 없어 시작으로 냅니다",
                ),
            );
        }
        body.push("let view = grid");
    }

    fn padding_wrap(&self, index: u32, body: &mut Body) {
        let (top, right, bottom, left) = self.style(index).padding.components();
        if [top, right, bottom, left].iter().any(|v| *v != 0.0) {
            body.padded = true;
            body.push(format!(
                "let padded = pad(view, top: {}, left: {}, bottom: {}, right: {})",
                px(top),
                px(left),
                px(bottom),
                px(right)
            ));
        }
    }

    fn sf_symbol(&self, index: u32) -> Option<String> {
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

    fn disabled_guard(&self, index: u32) -> Option<String> {
        let entry = self.ir.semantics_of(index)?;
        let binding = entry.semantics.states.get(&StateKind::Disabled)?;
        match self.ir.resolve_binding(binding) {
            ResolvedBinding::State(state) => match self.states.get(&state.id) {
                Some((name, SwiftType::Bool)) => Some(format!("self.{name}")),
                _ => None,
            },
            ResolvedBinding::Constant(TypedValue::Boolean(true)) => Some("true".to_string()),
            _ => None,
        }
    }

    fn button(&mut self, index: u32, body: &mut Body) {
        let node_id = self.ir.nodes[index as usize].id;
        let mut statements = Vec::new();
        if let Some(entry) = self.ir.semantics_of(index) {
            for (action, target) in &entry.semantics.actions {
                match (action, target) {
                    (ActionKind::Activate, ActionTarget::SemanticAction(action_id)) => statements
                        .push(format!(
                            "self.onAction({})",
                            string_literal(&action_id.to_string())
                        )),
                    (ActionKind::Select, ActionTarget::State(state)) => {
                        if let Some((name, SwiftType::String)) = self.states.get(state) {
                            statements.push(format!(
                                "self.{name} = {}",
                                string_literal(&node_id.to_string())
                            ));
                        }
                    }
                    _ => {}
                }
            }
        }
        body.push(format!(
            "let target = {}ActionTarget {{ [unowned self] in",
            self.prefix
        ));
        if let Some(flag) = self.disabled_guard(index) {
            body.push(format!("    guard !({flag}) else {{ return }}"));
        }
        if statements.is_empty() {
            body.push("    _ = self");
        }
        for statement in statements {
            body.push(format!("    {statement}"));
        }
        body.push("}");
        body.push("targets.append(target)");
        body.push(format!(
            "view.addGestureRecognizer(NSClickGestureRecognizer(target: target, action: #selector({}ActionTarget.fire)))",
            self.prefix
        ));
    }

    fn text_input(&mut self, index: u32, body: &mut Body) {
        let mut state = None;
        let mut placeholder = String::new();
        if let Some(entry) = self.ir.semantics_of(index) {
            if let Some(label) = &entry.semantics.label {
                placeholder = self.ir.static_text(label).unwrap_or_default();
            }
            if let Some(value) = &entry.semantics.value {
                if let ResolvedBinding::State(s) = self.ir.resolve_binding(value) {
                    if let Some((name, SwiftType::String)) = self.states.get(&s.id) {
                        state = Some(name.clone());
                    }
                }
            }
        }
        match &state {
            Some(name) => body.push(format!("let view = NSTextField(string: {name})")),
            None => {
                self.note(index, AppleNoteKind::UnresolvedBinding(Facet::Value));
                body.push("let view = NSTextField(string: \"\")");
            }
        }
        body.push(format!(
            "view.placeholderString = {}",
            string_literal(&placeholder)
        ));
        if let Some(name) = state {
            body.push(format!(
                "let target = {}ActionTarget {{ [unowned self, unowned view] in",
                self.prefix
            ));
            body.push(format!("    self.{name} = view.stringValue"));
            body.push("}");
            body.push("targets.append(target)");
            body.push("view.target = target");
            body.push(format!(
                "view.action = #selector({}ActionTarget.fire)",
                self.prefix
            ));
        }
        if !self.info(index).flow.is_empty() || !self.info(index).absolute.is_empty() {
            self.note(
                index,
                AppleNoteKind::Control(ControlNote::ChildrenIgnored(Control::TextInput)),
            );
        }
    }

    fn absolute_children(&mut self, index: u32, body: &mut Body, target: &str) {
        let children = self.info(index).absolute.clone();
        let (ptop, _, _, pleft) = self.style(index).padding.components();
        for (position, child) in children.iter().enumerate() {
            let name = format!("overlay{}", position + 1);
            body.push(format!("let {name} = {}()", self.make(*child)));
            body.push(format!("{target}.addSubview({name})"));
            let s = self.style(*child).clone();
            for (start, end, start_anchor, end_anchor, static_offset, start_name, end_name) in [
                (
                    s.left,
                    s.right,
                    "leadingAnchor",
                    "trailingAnchor",
                    pleft,
                    "left",
                    "right",
                ),
                (
                    s.top,
                    s.bottom,
                    "topAnchor",
                    "bottomAnchor",
                    ptop,
                    "top",
                    "bottom",
                ),
            ] {
                for (value, longhand) in [(start, start_name), (end, end_name)] {
                    if matches!(value, LengthPercentageAuto::Percentage(_)) {
                        self.finding(
                            *child,
                            unsupported(
                                longhand,
                                "percent-inset",
                                "AppKit 상수 제약으로 백분율 inset을 내지 않습니다",
                            ),
                        );
                    }
                }
                let mut wrote = false;
                if let LengthPercentageAuto::Length(length) = start {
                    body.push(format!(
                        "{name}.{start_anchor}.constraint(equalTo: {target}.{start_anchor}, constant: {}).isActive = true",
                        px(length.get())
                    ));
                    wrote = true;
                }
                if let LengthPercentageAuto::Length(length) = end {
                    body.push(format!(
                        "{name}.{end_anchor}.constraint(equalTo: {target}.{end_anchor}, constant: {}).isActive = true",
                        px(-length.get())
                    ));
                    wrote = true;
                }
                if !wrote {
                    self.finding(
                        *child,
                        approximated(
                            "position",
                            "static-position",
                            "inset이 모두 auto인 absolute box의 static position은 부모 content box 시작으로 냅니다",
                        ),
                    );
                    body.push(format!(
                        "{name}.{start_anchor}.constraint(equalTo: {target}.{start_anchor}, constant: {}).isActive = true",
                        px(static_offset)
                    ));
                }
            }
        }
    }

    fn function(&mut self, index: u32) -> (String, bool) {
        let info = self.info(index).clone();
        let mut body = Body::default();
        if let Some(dropped) = info.dropped_control {
            self.note(
                index,
                AppleNoteKind::Control(ControlNote::ContractUnsupported(dropped)),
            );
        }
        let mut leaf = true;
        match info.kind {
            ViewKind::Text => {
                let text = self.ir.nodes[index as usize]
                    .text
                    .clone()
                    .unwrap_or_default();
                body.push(format!(
                    "let view = NSTextField(labelWithString: {})",
                    string_literal(&text)
                ));
            }
            ViewKind::Image => match self.sf_symbol(index) {
                Some(name) => {
                    body.push(format!(
                        "let view = NSImageView(image: NSImage(systemSymbolName: {}, accessibilityDescription: nil) ?? NSImage())",
                        string_literal(&name)
                    ));
                    body.push("view.imageScaling = .scaleProportionallyUpOrDown");
                }
                None => {
                    self.note(
                        index,
                        AppleNoteKind::Control(ControlNote::ImageSourceUnavailable),
                    );
                    body.push("let view = NSView()");
                }
            },
            ViewKind::Shape => {
                self.note(
                    index,
                    AppleNoteKind::Control(ControlNote::ShapeGeometryUnavailable),
                );
                body.push("let view = NSView()");
            }
            ViewKind::Instance => {
                self.note(
                    index,
                    AppleNoteKind::Control(ControlNote::InstanceUnexpanded),
                );
                body.push("let view = NSView()");
            }
            ViewKind::Container => match info.control {
                Some(Control::TextInput) => self.text_input(index, &mut body),
                _ => {
                    if info.control == Some(Control::Modal) {
                        self.note(index, AppleNoteKind::Control(ControlNote::ModalInPlace));
                    }
                    match info.container.clone() {
                        Container::Grid(plan) if !info.flow.is_empty() => {
                            self.grid(index, &plan, &mut body)
                        }
                        _ => {
                            self.stack(index, &mut body);
                            leaf = false;
                        }
                    }
                }
            },
        }
        body.push("view.translatesAutoresizingMaskIntoConstraints = false");
        // stack은 edgeInsets로 padding을 내고, 그 밖의 view는 감싼 view의 제약으로 냅니다.
        let mut target = "view";
        if leaf {
            self.padding_wrap(index, &mut body);
            if body.padded {
                target = "padded";
            }
        }
        if info.control == Some(Control::Button) {
            self.button(index, &mut body);
        }
        self.absolute_children(index, &mut body, target);
        if target == "padded" {
            // 크기 제약은 padding을 포함한 바깥 view에 겁니다.
            body.push("do {");
            body.push("    let view = padded");
            let mut inner = Body::default();
            self.size_constraints(index, &mut inner);
            for line in inner.lines {
                body.push(format!("    {line}"));
            }
            body.push("}");
        } else {
            self.size_constraints(index, &mut body);
        }
        body.push(format!(
            "views[{}] = {target}",
            string_literal(&self.ir.nodes[index as usize].id.to_string())
        ));
        body.push(format!("return {target}"));
        let mut out = format!("    private func {}() -> NSView {{\n", self.make(index));
        for line in &body.lines {
            let _ = writeln!(out, "        {line}");
        }
        out.push_str("    }\n");
        (out, body.padded)
    }

    fn apply_accessibility(&self) -> String {
        let mut out = String::from("    private func applyAccessibility() {\n");
        for (index, hook) in self.hooks.iter().enumerate() {
            let Some(hook) = hook else {
                continue;
            };
            if !self.info(index as u32).emitted {
                continue;
            }
            let id = self.ir.nodes[index].id;
            let mut args = vec!["to: view".to_string()];
            for state in &hook.states {
                if let Some((name, _)) = self.states.get(state) {
                    args.push(format!("{name}: {name}"));
                }
            }
            if hook.action {
                args.push("onAction: onAction".to_string());
            }
            if hook.set_state {
                args.push(
                    "onStateChange: { [unowned self] id, value in self.setState(id, value) }"
                        .to_string(),
                );
            }
            if hook.children {
                args.push("views: views".to_string());
            }
            let _ = writeln!(
                out,
                "        if let view = views[{}] {{",
                string_literal(&id.to_string())
            );
            let _ = writeln!(
                out,
                "            {}Accessibility.apply{}({})",
                self.prefix,
                id_hex(id),
                args.join(", ")
            );
            out.push_str("        }\n");
        }
        out.push_str("    }\n");
        out
    }

    /// 출력 소스.
    pub(crate) fn source(&mut self) -> String {
        let prefix = self.prefix;
        let mut out = String::new();
        let _ = write!(
            out,
            "\n@MainActor\nfinal class {prefix}ActionTarget: NSObject {{\n"
        );
        out.push_str("    private let handler: () -> Void\n\n");
        out.push_str("    init(_ handler: @escaping () -> Void) {\n        self.handler = handler\n    }\n\n");
        out.push_str("    @objc func fire() {\n        handler()\n    }\n}\n");
        let _ = writeln!(out, "\n@MainActor\nfinal class {prefix}View: NSView {{");
        for state in &self.ir.states {
            if let Some((name, ty)) = self.states.get(&state.id) {
                let _ = writeln!(
                    out,
                    "    var {name}: {} = {} {{\n        didSet {{ applyAccessibility() }}\n    }}",
                    ty.name(),
                    initial_literal(&state.initial, *ty)
                );
            }
        }
        out.push_str("    let onAction: (String) -> Void\n");
        out.push_str("    private var views: [String: NSView] = [:]\n");
        let _ = write!(
            out,
            "    private var targets: [{prefix}ActionTarget] = []\n\n"
        );
        out.push_str("    init(onAction: @escaping (String) -> Void = { _ in }) {\n");
        out.push_str("        self.onAction = onAction\n        super.init(frame: .zero)\n");
        if self.info(0).emitted {
            let _ = writeln!(out, "        let root = {}()", self.make(0));
            out.push_str("        addSubview(root)\n");
            out.push_str(
                "        root.leadingAnchor.constraint(equalTo: leadingAnchor).isActive = true\n",
            );
            out.push_str("        root.topAnchor.constraint(equalTo: topAnchor).isActive = true\n");
            if self.info(0).fill.axes[0] {
                out.push_str("        root.trailingAnchor.constraint(equalTo: trailingAnchor).isActive = true\n");
            }
            out.push_str("        root.bottomAnchor.constraint(lessThanOrEqualTo: bottomAnchor).isActive = true\n");
        }
        out.push_str("        applyAccessibility()\n    }\n\n");
        out.push_str("    required init?(coder: NSCoder) {\n        return nil\n    }\n\n");
        out.push_str("    override var isFlipped: Bool { true }\n");
        let setters: Vec<(String, String)> = self
            .ir
            .states
            .iter()
            .filter_map(|state| match self.states.get(&state.id) {
                Some((name, SwiftType::String)) => Some((state.id.to_string(), name.clone())),
                _ => None,
            })
            .collect();
        if self.hooks.iter().flatten().any(|hook| hook.set_state) {
            out.push_str("\n    private func setState(_ id: String, _ value: String) {\n        switch id {\n");
            for (id, name) in &setters {
                let _ = writeln!(
                    out,
                    "        case {}:\n            {name} = value",
                    string_literal(id)
                );
            }
            out.push_str("        default:\n            break\n        }\n    }\n");
        }
        out.push('\n');
        out.push_str(&self.apply_accessibility());
        let mut padded = false;
        for index in 0..self.ir.nodes.len() as u32 {
            if !self.info(index).emitted {
                continue;
            }
            let (code, uses_pad) = self.function(index);
            padded |= uses_pad;
            out.push('\n');
            out.push_str(&code);
        }
        if padded {
            out.push_str("\n    private func pad(_ content: NSView, top: CGFloat, left: CGFloat, bottom: CGFloat, right: CGFloat) -> NSView {\n");
            out.push_str("        let view = NSView()\n");
            out.push_str("        view.translatesAutoresizingMaskIntoConstraints = false\n");
            out.push_str("        view.addSubview(content)\n");
            out.push_str("        NSLayoutConstraint.activate([\n");
            out.push_str("            content.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: left),\n");
            out.push_str("            content.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -right),\n");
            out.push_str("            content.topAnchor.constraint(equalTo: view.topAnchor, constant: top),\n");
            out.push_str("            content.bottomAnchor.constraint(equalTo: view.bottomAnchor, constant: -bottom),\n");
            out.push_str("        ])\n        return view\n    }\n");
        }
        out.push_str("}\n");
        out
    }
}
