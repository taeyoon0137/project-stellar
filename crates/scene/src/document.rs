//! 문서 → 증분 scene source 변환 (XI.2.1, XI.2.4, XI.2.5, XII.10.30, ADR-0084 결정 2·3·5·7).
//!
//! [`document_scene_source`]는 View root 아래 문서 node를 `crate::source::SceneSource`로 바꿉니다. node 종류별
//! 입력은 캔버스 행과 같은 함수([`super::frame_input`], [`super::shape::shape_source`], [`super::image::image_input`])로
//! 만듭니다. [`scene_edits`]·[`scene_invalidations`]는 command change set(VII.1.4)을 source 편집과 scene 무효화로
//! 바꿉니다.
//!
//! # 규칙
//!
//! - 숨긴 node도 넣습니다(scene은 숨김을 `Visibility::Hidden`으로 남겨 표시 전환이 구조 변경이 되지 않음).
//! - Free가 아닌 layout Frame(`layout_mode`)은 배치 결과 연결 전이라 subtree를 넣지 않습니다
//!   ([`SkipReason::LayoutNotEvaluated`]). Path clip Frame과 modifier가 있는 Frame도 같은 이유로 뺍니다.
//! - ComponentInstance는 문서에 복제하지 않고 expansion view(IX.2.2)의 가상 subtree를 펼칩니다. 인스턴스 key는
//!   Fragment(scene node 없음)이고 그 아래 정의 root의 가상 투영이 옵니다. 가상 node key는
//!   `SceneKey::virtual_node(인스턴스 chain, 정의 node)`, scene `source`는 바깥 인스턴스입니다(hit test는 인스턴스를
//!   고름). slot에 투영한 인스턴스 소유 node는 자기 StableId를 씁니다.
//! - View Variant(IX.7.6)는 해석한 override 묶음(`VariantOverrideSet`)을 받아 적용합니다. 지금은 scene 입력에 바로 닿는
//!   표시 여부(`node_keys::VISIBLE`) patch만 적용하고, 나머지 property·구조 patch는 문서 계층의 값 쓰기 함수가 생길
//!   때까지 [`DocumentSceneSource::unapplied_variant_patches`]에 셉니다(ADR-0084 결정 5).
//! - Boolean modifier(XII.10.30)를 가진 Shape·Path는 결과 host입니다. host의 다른 modifier는 Boolean 항목을 뺀
//!   stack으로 평가합니다. operand node는 source에 넣지 않고(결과에 흡수), host local 좌표로 옮긴 geometry를 문서
//!   pre-order(OrderKey 순) 순서로 host에 둡니다. operand의 geometry 변경과 operand 조상의 변경은 host의 geometry
//!   dirty가 됩니다([`SceneDependents`]). host가 skip되어 결과를 남기지 못하면 operand는 흡수된 채 그리지 않고
//!   [`SkipReason::BooleanHostSkipped`]로 기록합니다(ADR-0084 결정 7).

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::boolean::{BooleanOp as SceneBooleanOp, BooleanOperand};
use crate::builder::{FragmentInput, NodeInput, PathInput, StrokeInput, TextInput};
use crate::dirty::{InvalidationClasses, InvalidationOrigin, SceneDependents, SceneInvalidation};
use crate::geometry::{FillRule, PathGeometry};
use crate::source::{
    BooleanSource, SceneKey, SceneSource, SourceEdit, SourceError, SourceKind, SourceNode,
};
use crate::stack::Paint;
use stellar_commands::change::{node_keys, ChangeSet};
use stellar_core_types::ids::StableId;
use stellar_core_types::typed_value::TypedValue;
use stellar_core_types::value::{Rect, Size2, Transform2D, Vec2};
use stellar_document::component::definition::ComponentLibrary;
use stellar_document::component::expansion::{
    ExpandedNodeId, InstanceExpansion, SceneIndex, ScenePayloads,
};
use stellar_document::component::instance::{frame_instance_transform, placed_free_frame};
use stellar_document::component::variant_override::VariantOverrideSet;
use stellar_document::frame::Clipping;
use stellar_document::model::DocumentModel;
use stellar_document::modifier::{BooleanOp, ModifierKind, ModifierPayload, ModifierStack};
use stellar_document::modifier_metadata::{InvalidationClass, InvalidationSet};
use stellar_document::path::WindingRule;
use stellar_document::record_codec::{decode_payload, encode_payload};
use stellar_document::records::NodePayload;
use stellar_document::shape::ShapeModifierStyle;
use stellar_document::text::TextPayload;

/// 문서 node payload → scene builder 입력.
pub mod inputs;

use inputs::{frame_input, image_input, scene_command, shape_source, shape_style};

/// source에 넣지 않은 node의 이유.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SkipReason {
    /// layout Frame(`layout_mode`)이라 배치 결과 연결 전입니다.
    LayoutNotEvaluated,
    /// Path clip Frame.
    PathClip,
    /// 평가하지 않는 modifier stack.
    UnsupportedModifiers,
    /// text layout hook이 없거나 layout에 실패했습니다.
    Text,
    /// payload를 읽지 못했습니다.
    Payload,
    /// ComponentInstance expansion 실패(미해결·재귀·중첩 상한).
    Instance,
    /// 값이 scene 규칙을 벗어났습니다(비유한 등).
    InvalidValue,
    /// Boolean operand인데 결과를 그리는 host가 scene에 없습니다(host가 skip됨). operand는 결과에 흡수되므로 단독으로
    /// 그리지 않습니다(ADR-0084 결정 7).
    BooleanHostSkipped,
}

/// Text node를 scene 입력으로 바꾸는 hook(글꼴·layout은 text 엔진이 맡음).
pub type TextHook<'a> =
    dyn FnMut(StableId, &TextPayload, Option<&ModifierStack>, bool) -> Option<TextInput> + 'a;

/// 변환 옵션.
#[derive(Default)]
pub struct SceneSourceOptions<'a> {
    /// ComponentInstance 정의 집합.
    pub library: Option<&'a ComponentLibrary>,
    /// Text hook.
    pub text: Option<&'a mut TextHook<'a>>,
    /// 해석한 View Variant override(IX.7.6).
    pub variant: Option<&'a VariantOverrideSet>,
}

/// 변환 결과.
#[derive(Debug, Clone, PartialEq)]
pub struct DocumentSceneSource {
    /// scene source tree.
    pub source: SceneSource,
    /// 넣지 않은 node와 이유(StableId 순).
    pub skipped: BTreeMap<SceneKey, SkipReason>,
    /// Boolean operand → host 목록.
    pub operand_hosts: BTreeMap<StableId, Vec<StableId>>,
    /// 적용하지 않은 View Variant patch 수.
    pub unapplied_variant_patches: usize,
    /// View root.
    root: StableId,
    /// root subtree pre-order 위치(Boolean operand 순서). 구조 변경 때만 다시 계산합니다.
    preorder: HashMap<StableId, usize>,
    /// Boolean operand의 조상 → 그 operand를 읽는 host(정렬·중복 없음). 조상 이동이 host를 dirty로 만드는 데
    /// 씁니다. dirty node마다 operand 전체를 훑지 않도록 조상 쪽으로 뒤집어 둡니다.
    ancestor_hosts: HashMap<StableId, Vec<StableId>>,
    /// instance 펼침에서 나온 skip 기록 → 그 instance. 가상 key는 문서 node id가 아니라서, 인스턴스를 다시 변환할 때
    /// 이 표로 이전 기록을 지웁니다.
    skip_owner: BTreeMap<SceneKey, StableId>,
}

impl SceneDependents for DocumentSceneSource {
    fn dependents_of(&self, node: StableId) -> Vec<StableId> {
        self.operand_hosts.get(&node).cloned().unwrap_or_default()
    }

    fn descendant_dependents(&self, node: StableId) -> Vec<StableId> {
        self.ancestor_hosts.get(&node).cloned().unwrap_or_default()
    }
}

impl DocumentSceneSource {
    /// View root.
    pub const fn root(&self) -> StableId {
        self.root
    }
}

/// 변환 상태.
struct Converter<'a, 'o> {
    model: &'a DocumentModel,
    options: &'o mut SceneSourceOptions<'a>,
    /// Boolean operand → host.
    operand_hosts: BTreeMap<StableId, Vec<StableId>>,
    /// root subtree pre-order 위치.
    preorder: HashMap<StableId, usize>,
    /// variant 표시 여부 patch.
    visibility: HashMap<StableId, bool>,
    /// instance expansion용 payload 색인(필요할 때만 만듦).
    payloads: Option<ScenePayloads>,
    skipped: BTreeMap<SceneKey, SkipReason>,
    /// instance 펼침 skip 기록의 instance.
    skip_owner: BTreeMap<SceneKey, StableId>,
}

/// 문서 View root subtree를 scene source로 바꿉니다.
pub fn document_scene_source<'a>(
    model: &'a DocumentModel,
    root: StableId,
    mut options: SceneSourceOptions<'a>,
) -> Result<DocumentSceneSource, SourceError> {
    let mut converter = Converter::new(model, root, &mut options);
    let unapplied = converter.unapplied_variant_patches();
    let mut nodes = Vec::new();
    let roots: Vec<SceneKey> = converter.subtree(root, &mut nodes).into_iter().collect();
    let source = SceneSource::from_nodes(roots, nodes)?;
    let mut built = DocumentSceneSource {
        source,
        skipped: converter.skipped,
        operand_hosts: converter.operand_hosts,
        unapplied_variant_patches: unapplied,
        root,
        preorder: converter.preorder,
        ancestor_hosts: HashMap::new(),
        skip_owner: converter.skip_owner,
    };
    refresh_operands(model, &mut built);
    Ok(built)
}

/// operand 조상 표와 host 없는 operand의 skip 기록을 현재 source에 맞춥니다.
///
/// host가 scene에 Boolean 결과를 남기지 못하면(host 자신이나 조상이 skip됨) operand는 흡수된 채 아무것도 그리지
/// 않습니다. 그 operand를 [`SkipReason::BooleanHostSkipped`]로 기록해 빠진 이유가 드러나게 합니다(ADR-0084 결정 7).
fn refresh_operands(model: &DocumentModel, current: &mut DocumentSceneSource) {
    let mut ancestor_hosts: HashMap<StableId, Vec<StableId>> = HashMap::new();
    for (operand, hosts) in &current.operand_hosts {
        let mut parent = parent_of(model, *operand);
        while let Some(id) = parent {
            ancestor_hosts
                .entry(id)
                .or_default()
                .extend(hosts.iter().copied());
            parent = parent_of(model, id);
        }
    }
    for hosts in ancestor_hosts.values_mut() {
        hosts.sort_unstable();
        hosts.dedup();
    }
    current.ancestor_hosts = ancestor_hosts;
    current
        .skipped
        .retain(|_, reason| *reason != SkipReason::BooleanHostSkipped);
    for (operand, hosts) in &current.operand_hosts {
        let drawn = hosts.iter().any(|host| {
            current
                .source
                .get(SceneKey::node(*host))
                .is_some_and(|node| node.boolean.is_some())
        });
        if !drawn {
            current
                .skipped
                .entry(SceneKey::node(*operand))
                .or_insert(SkipReason::BooleanHostSkipped);
        }
    }
}

impl<'a, 'o> Converter<'a, 'o> {
    fn new(
        model: &'a DocumentModel,
        root: StableId,
        options: &'o mut SceneSourceOptions<'a>,
    ) -> Converter<'a, 'o> {
        let mut preorder = HashMap::new();
        let mut operand_hosts: BTreeMap<StableId, Vec<StableId>> = BTreeMap::new();
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            preorder.insert(id, preorder.len());
            for (_, operands) in boolean_modifiers(model.modifier_stack(id)) {
                for operand in operands {
                    operand_hosts.entry(operand).or_default().push(id);
                }
            }
            stack.extend(model.children_in_order(id).into_iter().rev());
        }
        Converter::with_index(model, options, preorder, operand_hosts)
    }

    /// 이미 계산한 pre-order·operand 색인으로 만듭니다(구조가 바뀌지 않은 편집).
    fn with_index(
        model: &'a DocumentModel,
        options: &'o mut SceneSourceOptions<'a>,
        preorder: HashMap<StableId, usize>,
        operand_hosts: BTreeMap<StableId, Vec<StableId>>,
    ) -> Converter<'a, 'o> {
        let mut visibility = HashMap::new();
        if let Some(variant) = options.variant {
            for (path, value) in variant.property_patches() {
                if path.keys() == [node_keys::VISIBLE] {
                    if let TypedValue::Boolean(visible) = value {
                        visibility.insert(path.root(), *visible);
                    }
                }
            }
        }
        Converter {
            model,
            options,
            operand_hosts,
            preorder,
            visibility,
            payloads: None,
            skipped: BTreeMap::new(),
            skip_owner: BTreeMap::new(),
        }
    }

    fn unapplied_variant_patches(&self) -> usize {
        self.options.variant.map_or(0, |variant| {
            let properties = variant
                .property_patches()
                .filter(|(path, value)| {
                    !(path.keys() == [node_keys::VISIBLE]
                        && matches!(value, TypedValue::Boolean(_)))
                })
                .count();
            properties + variant.structure_patches().count() + variant.slot_changes().count()
        })
    }

    fn skip(&mut self, key: SceneKey, reason: SkipReason) {
        self.skipped.insert(key, reason);
    }

    /// `id` subtree를 `out`에 쌓고 넣었으면 key를 돌려줍니다.
    fn subtree(&mut self, id: StableId, out: &mut Vec<(SceneKey, SourceNode)>) -> Option<SceneKey> {
        let key = SceneKey::node(id);
        if self.operand_hosts.contains_key(&id) {
            // operand는 결과에 흡수되어 단독 표시하지 않습니다.
            return None;
        }
        let model = self.model;
        let fields = model.runtime_id(id).and_then(|r| model.columns().get(r))?;
        let visible = self.visibility.get(&id).copied().unwrap_or(fields.visible);
        let locked = fields.locked;
        let stack = model.modifier_stack(id);
        let Some(payload) = model.payload(id) else {
            self.skip(key, SkipReason::Payload);
            return None;
        };
        let kind = match payload {
            NodePayload::ComponentInstance(instance) => {
                let placement = frame_instance_transform(stack);
                return self.instance(id, visible, locked, placement, instance, out);
            }
            NodePayload::Frame(frame) => {
                if frame.layout_mode.is_some() {
                    self.skip(key, SkipReason::LayoutNotEvaluated);
                    return None;
                }
                if matches!(frame.clipping, Clipping::Path(_)) {
                    self.skip(key, SkipReason::PathClip);
                    return None;
                }
                if stack.is_some_and(|s| !s.entries().is_empty()) {
                    self.skip(key, SkipReason::UnsupportedModifiers);
                    return None;
                }
                let Some(mut input) = frame_input(id, frame, locked) else {
                    self.skip(key, SkipReason::InvalidValue);
                    return None;
                };
                input.node.visible = visible;
                SourceKind::Frame(input)
            }
            NodePayload::Fragment(_) => SourceKind::Fragment(FragmentInput {
                source: id,
                visible,
                locked,
            }),
            other => match self.leaf(id, other, stack, visible, locked) {
                Ok(node) => {
                    out.push((key, node));
                    return Some(key);
                }
                Err(reason) => {
                    self.skip(key, reason);
                    return None;
                }
            },
        };
        let children = model
            .children_in_order(id)
            .into_iter()
            .filter_map(|child| self.subtree(child, out))
            .collect();
        out.push((key, SourceNode::with_children(kind, children)));
        Some(key)
    }

    /// Shape·Path·Text·Image leaf.
    fn leaf(
        &mut self,
        id: StableId,
        payload: &NodePayload,
        stack: Option<&ModifierStack>,
        visible: bool,
        locked: bool,
    ) -> Result<SourceNode, SkipReason> {
        let booleans = boolean_modifiers(stack);
        let filtered = booleans.first().map(|_| without_booleans(id, stack));
        let stack = filtered.as_ref().map_or(stack, Some);
        let mut node = match payload {
            NodePayload::Shape(shape) => {
                let source = shape_source(id, shape, stack, locked)
                    .ok_or(SkipReason::UnsupportedModifiers)?;
                let mut input = source.input;
                input.node.visible = visible;
                SourceNode::leaf(SourceKind::Shape(input))
            }
            NodePayload::Path(path) => {
                let style = shape_style(stack).ok_or(SkipReason::UnsupportedModifiers)?;
                let input = path_input(id, path.data(), &style, visible, locked)
                    .ok_or(SkipReason::InvalidValue)?;
                SourceNode::leaf(SourceKind::Path(input))
            }
            NodePayload::Image(image) => {
                let mut input = image_input(id, image, stack, locked)
                    .ok_or(SkipReason::UnsupportedModifiers)?;
                input.node.visible = visible;
                SourceNode::leaf(SourceKind::Image(input))
            }
            NodePayload::Text(text) => {
                let hook = self.options.text.as_mut().ok_or(SkipReason::Text)?;
                let mut input = hook(id, text, stack, locked).ok_or(SkipReason::Text)?;
                input.node.visible = visible;
                SourceNode::leaf(SourceKind::Text(input))
            }
            _ => return Err(SkipReason::Payload),
        };
        if let Some((op, operands)) = booleans.into_iter().next() {
            if !matches!(node.kind, SourceKind::Shape(_) | SourceKind::Path(_)) {
                return Err(SkipReason::UnsupportedModifiers);
            }
            node.boolean = Some(self.boolean_source(id, op, operands));
        }
        Ok(node)
    }

    /// host local 좌표로 옮긴 operand geometry(문서 pre-order 순).
    fn boolean_source(
        &mut self,
        host: StableId,
        op: BooleanOp,
        operands: Vec<StableId>,
    ) -> BooleanSource {
        let model = self.model;
        let mut ordered: Vec<StableId> = operands
            .into_iter()
            .filter(|id| self.preorder.contains_key(id))
            .collect();
        ordered.sort_by_key(|id| self.preorder[id]);
        ordered.dedup();
        let host_world = world_transform(model, host);
        let to_host = host_world.and_then(|world| world.inverse());
        let mut out = Vec::new();
        for operand in ordered {
            let Some((geometry, fill_rule, local)) = operand_geometry(model, operand) else {
                self.skip(SceneKey::node(operand), SkipReason::UnsupportedModifiers);
                continue;
            };
            let transform = world_transform_with(model, operand, local)
                .zip(to_host)
                .and_then(|(world, inverse)| world.then(&inverse).ok());
            let Some(transform) = transform else {
                self.skip(SceneKey::node(operand), SkipReason::InvalidValue);
                continue;
            };
            out.push(BooleanOperand {
                geometry,
                fill_rule,
                transform,
            });
        }
        BooleanSource {
            op: scene_boolean_op(op),
            operands: out,
        }
    }

    /// ComponentInstance의 가상 subtree. 인스턴스 key는 Fragment입니다.
    fn instance(
        &mut self,
        id: StableId,
        visible: bool,
        locked: bool,
        placement: Option<Transform2D>,
        _payload: &stellar_document::component::instance::ComponentInstancePayload,
        out: &mut Vec<(SceneKey, SourceNode)>,
    ) -> Option<SceneKey> {
        let key = SceneKey::node(id);
        let Some(library) = self.options.library else {
            self.skip(key, SkipReason::Instance);
            return None;
        };
        let Some(placement) = placement else {
            self.skip(key, SkipReason::UnsupportedModifiers);
            return None;
        };
        let model = self.model;
        let payloads: &ScenePayloads = self.payloads.get_or_insert_with(|| scene_payloads(model));
        let index = SceneIndex::build(model.columns(), payloads);
        let Ok(expansion) = InstanceExpansion::new(library, &index, id) else {
            self.skip(key, SkipReason::Instance);
            return None;
        };
        let mut virtual_nodes = Vec::new();
        let mut skipped = Vec::new();
        let root = expand(
            &expansion,
            &expansion.root(),
            id,
            Some(placement),
            &mut virtual_nodes,
            &mut skipped,
        );
        for (k, reason) in skipped {
            self.skip(k, reason);
            self.skip_owner.insert(k, id);
        }
        out.extend(virtual_nodes);
        let children = root.into_iter().collect();
        out.push((
            key,
            SourceNode::with_children(
                SourceKind::Fragment(FragmentInput {
                    source: id,
                    visible,
                    locked,
                }),
                children,
            ),
        ));
        Some(key)
    }
}

/// expansion node 하나와 자식을 source node로 바꿉니다(IX.9 fixture처럼 문서 모델 없이 쓰는 경로 포함).
pub fn expand(
    expansion: &InstanceExpansion<'_>,
    id: &ExpandedNodeId,
    instance: StableId,
    placement: Option<Transform2D>,
    out: &mut Vec<(SceneKey, SourceNode)>,
    skipped: &mut Vec<(SceneKey, SkipReason)>,
) -> Option<SceneKey> {
    let key = match id {
        ExpandedNodeId::Virtual { instances, source } => SceneKey::virtual_node(instances, *source),
        ExpandedNodeId::Owned(node) => SceneKey::node(*node),
    };
    let Ok(node) = expansion.node(id) else {
        skipped.push((key, SkipReason::Instance));
        return None;
    };
    // 가상 node는 바깥 인스턴스를, 소유 node는 자기를 scene source로 씁니다.
    let source = match id {
        ExpandedNodeId::Virtual { .. } => instance,
        ExpandedNodeId::Owned(node) => *node,
    };
    let payload = node
        .payload()
        .and_then(|bytes| decode_payload(node.kind(), bytes).ok());
    let Some(payload) = payload else {
        skipped.push((key, SkipReason::Payload));
        return None;
    };
    let kind = match &payload {
        NodePayload::Frame(frame) => {
            // 정의 root는 정의의 캔버스 원점을 인스턴스 배치로 바꿉니다(캔버스 행과 같은 규칙).
            let frame = match node.definition_node() {
                Some(definition) => match placed_free_frame(definition, placement) {
                    Some(frame) => frame,
                    None => {
                        skipped.push((key, SkipReason::LayoutNotEvaluated));
                        return None;
                    }
                },
                None => frame.clone(),
            };
            let Some(mut input) = frame_input(source, &frame, node.locked()) else {
                skipped.push((key, SkipReason::InvalidValue));
                return None;
            };
            input.node.visible = node.visible();
            SourceKind::Frame(input)
        }
        NodePayload::Fragment(_) => SourceKind::Fragment(FragmentInput {
            source,
            visible: node.visible(),
            locked: node.locked(),
        }),
        NodePayload::Shape(shape) => match shape_source(source, shape, None, node.locked()) {
            Some(shape) => {
                let mut input = shape.input;
                input.node.visible = node.visible();
                out.push((key, SourceNode::leaf(SourceKind::Shape(input))));
                return Some(key);
            }
            None => {
                skipped.push((key, SkipReason::InvalidValue));
                return None;
            }
        },
        NodePayload::Image(image) => match image_input(source, image, None, node.locked()) {
            Some(mut input) => {
                input.node.visible = node.visible();
                out.push((key, SourceNode::leaf(SourceKind::Image(input))));
                return Some(key);
            }
            None => {
                skipped.push((key, SkipReason::UnsupportedModifiers));
                return None;
            }
        },
        NodePayload::Path(path) => {
            let style = ShapeModifierStyle {
                transform: Transform2D::identity(),
                last_transform: None,
                fill: None,
                stroke: None,
            };
            match path_input(source, path.data(), &style, node.visible(), node.locked()) {
                Some(input) => {
                    out.push((key, SourceNode::leaf(SourceKind::Path(input))));
                    return Some(key);
                }
                None => {
                    skipped.push((key, SkipReason::InvalidValue));
                    return None;
                }
            }
        }
        NodePayload::Text(_) => {
            skipped.push((key, SkipReason::Text));
            return None;
        }
        NodePayload::ComponentInstance(_) => SourceKind::Fragment(FragmentInput {
            source,
            visible: node.visible(),
            locked: node.locked(),
        }),
    };
    let children = expansion
        .children(id)
        .unwrap_or_default()
        .iter()
        .filter_map(|child| expand(expansion, child, instance, None, out, skipped))
        .collect();
    out.push((key, SourceNode::with_children(kind, children)));
    Some(key)
}

/// expansion이 소유 node payload를 읽을 수 있게 모든 node payload를 색인합니다.
fn scene_payloads(model: &DocumentModel) -> ScenePayloads {
    let mut payloads = ScenePayloads::new();
    for handle in model.columns().iter_live() {
        if let Some(fields) = model.columns().get(handle) {
            if let Some(payload) = model.payload(fields.stable_id) {
                payloads.insert(fields.stable_id, encode_payload(payload));
            }
        }
    }
    payloads
}

/// 활성 Boolean modifier(연산, operand) 목록.
fn boolean_modifiers(stack: Option<&ModifierStack>) -> Vec<(BooleanOp, Vec<StableId>)> {
    stack
        .into_iter()
        .flat_map(ModifierStack::entries)
        .filter(|m| m.enabled)
        .filter_map(|m| match &m.payload {
            ModifierPayload::Boolean { op, operands } => {
                Some((*op, operands.iter().map(|r| r.id()).collect()))
            }
            _ => None,
        })
        .collect()
}

/// Boolean 항목을 뺀 stack.
fn without_booleans(node: StableId, stack: Option<&ModifierStack>) -> ModifierStack {
    let mut filtered = ModifierStack::new(node);
    for entry in stack.into_iter().flat_map(ModifierStack::entries) {
        if entry.kind != ModifierKind::Boolean {
            // 원래 stack의 항목이라 id·순서가 겹치지 않습니다.
            let _ = filtered.insert(entry.clone());
        }
    }
    filtered
}

fn scene_boolean_op(op: BooleanOp) -> SceneBooleanOp {
    match op {
        BooleanOp::Union => SceneBooleanOp::Union,
        BooleanOp::Subtract => SceneBooleanOp::Subtract,
        BooleanOp::Intersect => SceneBooleanOp::Intersect,
        BooleanOp::Exclude => SceneBooleanOp::Exclude,
    }
}

fn path_input(
    id: StableId,
    data: &stellar_document::path::PathData,
    style: &ShapeModifierStyle,
    visible: bool,
    locked: bool,
) -> Option<PathInput> {
    let geometry =
        PathGeometry::new(data.commands().iter().copied().map(scene_command).collect()).ok()?;
    let mut node = NodeInput::new(id, style.transform);
    node.visible = visible;
    node.locked = locked;
    if let Some(color) = style.fill {
        node.paint.fills.push(Paint::Solid(color));
    }
    if let Some(stroke) = style.stroke {
        node.paint.strokes.push(StrokeInput {
            paint: Paint::Solid(stroke.color),
            width: stroke.thickness(),
        });
    }
    let bounds = match crate::bounds::path_control_bounds(&geometry) {
        Some(aabb) => Rect::new(
            Vec2::new(aabb.min_x(), aabb.min_y()).ok()?,
            Size2::new(aabb.width(), aabb.height()).ok()?,
        ),
        None => Rect::new(Vec2::ZERO, Size2::new(0.0, 0.0).ok()?),
    };
    Some(PathInput {
        node,
        layout_rect: bounds,
        geometry,
        fill_rule: match data.winding() {
            WindingRule::NonZero => FillRule::NonZero,
            WindingRule::EvenOdd => FillRule::EvenOdd,
        },
    })
}

/// operand node의 local geometry·채우기 규칙·local transform.
fn operand_geometry(
    model: &DocumentModel,
    id: StableId,
) -> Option<(PathGeometry, FillRule, Transform2D)> {
    let stack = model.modifier_stack(id);
    match model.payload(id)? {
        NodePayload::Shape(shape) => {
            let source = shape_source(id, shape, stack, false)?;
            Some((
                source.input.geometry,
                FillRule::NonZero,
                source.input.node.transform,
            ))
        }
        NodePayload::Path(path) => {
            let style = shape_style(stack)?;
            let input = path_input(id, path.data(), &style, true, false)?;
            Some((input.geometry, input.fill_rule, input.node.transform))
        }
        _ => None,
    }
}

/// node 자신의 local transform(Frame transform, 도형·path·image의 modifier Transform).
fn local_transform(model: &DocumentModel, id: StableId) -> Option<Transform2D> {
    let stack = model.modifier_stack(id);
    match model.payload(id)? {
        NodePayload::Frame(frame) => Some(frame.transform),
        NodePayload::Fragment(_) => Some(Transform2D::identity()),
        NodePayload::ComponentInstance(_) => frame_instance_transform(stack),
        NodePayload::Shape(_)
        | NodePayload::Path(_)
        | NodePayload::Image(_)
        | NodePayload::Text(_) => {
            let filtered = without_booleans(id, stack);
            shape_style(Some(&filtered)).map(|style| style.transform)
        }
    }
}

/// 조상 local transform을 합성한 world transform.
fn world_transform(model: &DocumentModel, id: StableId) -> Option<Transform2D> {
    world_transform_with(model, id, local_transform(model, id)?)
}

fn world_transform_with(
    model: &DocumentModel,
    id: StableId,
    local: Transform2D,
) -> Option<Transform2D> {
    let mut world = local;
    let mut current = parent_of(model, id);
    while let Some(parent) = current {
        world = world.then(&local_transform(model, parent)?).ok()?;
        current = parent_of(model, parent);
    }
    Some(world)
}

fn parent_of(model: &DocumentModel, id: StableId) -> Option<StableId> {
    let fields = model.runtime_id(id).and_then(|r| model.columns().get(r))?;
    let parent = fields.parent?;
    model.columns().get(parent).map(|p| p.stable_id)
}

/// document 무효화 class를 scene 값으로 옮깁니다.
fn classes(set: InvalidationSet) -> InvalidationClasses {
    InvalidationClasses {
        geometry: set.contains(InvalidationClass::Geometry),
        layout: set.contains(InvalidationClass::Layout),
        paint: set.contains(InvalidationClass::Paint),
    }
}

/// command change set의 scene 무효화(XI.2.1).
///
/// node별 무효화 class를 그대로 옮기고, 잠금 변경(문서 class는 비어 있음)은 hit target 전환, 삽입·삭제는 부모의 구조
/// 변경으로 봅니다.
pub fn scene_invalidations(model: &DocumentModel, changes: &ChangeSet) -> Vec<SceneInvalidation> {
    let mut out = Vec::new();
    for node in changes.changed_nodes() {
        let set = changes
            .node_invalidation(node)
            .unwrap_or(InvalidationSet::EMPTY);
        out.push(SceneInvalidation::Node {
            node,
            origin: InvalidationOrigin::Property,
            classes: classes(set),
        });
    }
    for path in changes.changed_properties() {
        if path.keys() == [node_keys::LOCKED] {
            out.push(SceneInvalidation::HitTarget { node: path.root() });
        }
    }
    for node in changes.inserted_nodes() {
        if let Some(parent) = parent_of(model, node) {
            out.push(SceneInvalidation::Structure { node: parent });
        }
    }
    out
}

/// change set을 source 편집으로 바꿉니다(XI.2.5).
///
/// 바뀐 node, 삽입·삭제된 node의 부모, 바뀐 operand의 Boolean host를 subtree 단위로 다시 변환하고, 이전 source와
/// 다른 node만 Upsert, 사라진 node는 Remove합니다. 편집은 `current.source`에도 적용해 다음 change set의 비교 기준을
/// 맞추고, operand·skip 정보도 갱신합니다. 같은 편집을 `IncrementalScene::update`에 넘깁니다.
pub fn scene_edits<'a>(
    model: &'a DocumentModel,
    current: &mut DocumentSceneSource,
    changes: &ChangeSet,
    mut options: SceneSourceOptions<'a>,
) -> Result<Vec<SourceEdit>, SourceError> {
    let root = current.root;
    let mut affected: BTreeSet<StableId> = changes.changed_nodes().into_iter().collect();
    for node in changes.inserted_nodes() {
        if let Some(parent) = parent_of(model, node) {
            affected.insert(parent);
        }
    }
    let removed: BTreeSet<StableId> = changes.removed_nodes().into_iter().collect();
    for node in &removed {
        if let Some(parent) = current.source.parent(SceneKey::node(*node)) {
            affected.insert(parent.id());
        }
    }
    // operand나 host 자신·조상이 바뀌면(조상 transform 포함) host local 좌표의 operand 값이 달라질 수 있습니다.
    // 지운 operand도 host 결과에서 빠져야 합니다.
    let mut hosts_to_refresh = Vec::new();
    for (operand, hosts) in &current.operand_hosts {
        let touched = removed.contains(operand)
            || std::iter::once(*operand)
                .chain(hosts.iter().copied())
                .any(|node| self_or_ancestor_in(model, node, &affected));
        if touched {
            hosts_to_refresh.extend(hosts.iter().copied());
        }
    }
    affected.extend(hosts_to_refresh);
    let old_operands: BTreeSet<StableId> = current.operand_hosts.keys().copied().collect();
    // 구조가 바뀌면 pre-order·operand 색인을 다시 만들고, 아니면 바뀐 node의 Boolean operand만 다시 읽습니다
    // (전체 tree를 걷지 않음).
    let structural = !changes.inserted_nodes().is_empty() || !removed.is_empty();
    let mut converter = if structural {
        Converter::new(model, root, &mut options)
    } else {
        let mut operand_hosts = std::mem::take(&mut current.operand_hosts);
        for hosts in operand_hosts.values_mut() {
            hosts.retain(|host| !affected.contains(host));
        }
        for host in &affected {
            for (_, operands) in boolean_modifiers(model.modifier_stack(*host)) {
                for operand in operands {
                    operand_hosts.entry(operand).or_default().push(*host);
                }
            }
        }
        operand_hosts.retain(|_, hosts| !hosts.is_empty());
        Converter::with_index(
            model,
            &mut options,
            std::mem::take(&mut current.preorder),
            operand_hosts,
        )
    };
    // operand 여부가 바뀐 node(Boolean 참조 추가·삭제)는 부모 목록에서 빠지거나 다시 들어갑니다.
    for operand in
        old_operands.symmetric_difference(&converter.operand_hosts.keys().copied().collect())
    {
        if let Some(parent) = parent_of(model, *operand) {
            affected.insert(parent);
        }
    }
    // source에 들어가는지가 바뀐 subtree root(지원하지 않는 modifier 추가 등)는 부모 목록도 바뀌므로 부모를 다시
    // 변환합니다. 부모를 더하면 처음부터 다시 셉니다(깊이만큼 반복).
    let (tops, edits_and_keys) = loop {
        let tops = subtree_tops(model, &affected, &converter.preorder);
        let mut edits = Vec::new();
        let mut keep: HashSet<SceneKey> = HashSet::new();
        let mut old_keys: Vec<SceneKey> = Vec::new();
        let mut parents_to_add = Vec::new();
        for top in &tops {
            collect_keys(&current.source, SceneKey::node(*top), &mut old_keys);
            let mut nodes = Vec::new();
            let key = converter.subtree(*top, &mut nodes);
            let was_present = current.source.get(SceneKey::node(*top)).is_some();
            if key.is_some() != was_present && *top != root {
                if let Some(parent) = parent_of(model, *top) {
                    parents_to_add.push(parent);
                }
            }
            if key.is_none() && *top == root {
                continue;
            }
            for (key, node) in nodes {
                keep.insert(key);
                if current.source.get(key) != Some(&node) {
                    edits.push(SourceEdit::upsert(key, node));
                }
            }
        }
        let before = affected.len();
        affected.extend(parents_to_add);
        if affected.len() == before {
            break (tops, (edits, keep, old_keys));
        }
    };
    let (mut edits, keep, old_keys) = edits_and_keys;
    for key in old_keys {
        if !keep.contains(&key) {
            edits.push(SourceEdit::Remove(key));
        }
    }
    for node in removed.iter().copied() {
        let key = SceneKey::node(node);
        if current.source.get(key).is_some()
            && !keep.contains(&key)
            && !edits.contains(&SourceEdit::Remove(key))
        {
            edits.push(SourceEdit::Remove(key));
        }
    }
    current.operand_hosts = converter.operand_hosts;
    current.preorder = converter.preorder;
    // 다시 변환한 subtree의 이전 skip 기록은 이번 결과로 바꿉니다(다시 들어간 node가 skip으로 남지 않게).
    let mut reconverted: HashSet<StableId> = HashSet::new();
    for top in &tops {
        let mut stack = vec![*top];
        while let Some(id) = stack.pop() {
            reconverted.insert(id);
            stack.extend(model.children_in_order(id));
        }
    }
    // instance 펼침에서 나온 기록(가상 key)은 그 instance를 다시 변환하거나 지웠을 때 지웁니다.
    let stale = |key: &SceneKey, owner: &BTreeMap<SceneKey, StableId>| {
        let gone = |id: &StableId| reconverted.contains(id) || removed.contains(id);
        gone(&key.id()) || owner.get(key).is_some_and(gone)
    };
    let owner = std::mem::take(&mut current.skip_owner);
    current.skipped.retain(|key, _| !stale(key, &owner));
    current.skip_owner = owner
        .iter()
        .filter(|(key, _)| !stale(key, &owner))
        .map(|(key, id)| (*key, *id))
        .collect();
    current.skipped.extend(converter.skipped);
    current.skip_owner.extend(converter.skip_owner);
    current.source.apply(&edits)?;
    refresh_operands(model, current);
    Ok(edits)
}

/// 조상이 `affected`에 없는 `affected` node(View root subtree 안). 조상이 함께 바뀐 node는 조상 subtree 변환에 들어갑니다.
fn subtree_tops(
    model: &DocumentModel,
    affected: &BTreeSet<StableId>,
    preorder: &HashMap<StableId, usize>,
) -> Vec<StableId> {
    affected
        .iter()
        .copied()
        .filter(|id| {
            let mut current = parent_of(model, *id);
            while let Some(parent) = current {
                if affected.contains(&parent) {
                    return false;
                }
                current = parent_of(model, parent);
            }
            preorder.contains_key(id)
        })
        .collect()
}

/// `node` 자신이나 조상이 `set`에 있는지.
fn self_or_ancestor_in(model: &DocumentModel, node: StableId, set: &BTreeSet<StableId>) -> bool {
    let mut current = Some(node);
    while let Some(id) = current {
        if set.contains(&id) {
            return true;
        }
        current = parent_of(model, id);
    }
    false
}

fn collect_keys(source: &SceneSource, key: SceneKey, out: &mut Vec<SceneKey>) {
    let Some(node) = source.get(key) else {
        return;
    };
    out.push(key);
    for child in &node.children {
        collect_keys(source, *child, out);
    }
}

#[cfg(test)]
mod tests;
