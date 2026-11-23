//! Component Context 통합 fixture(XXX.7.13): 테마 Context가 10단계 nested component를 중간 Property 없이 통과합니다.
//!
//! - scene View root가 theme(파랑)과 외부 라이브러리 key brand(초록)를 공급합니다.
//! - L1 → L2 → … → L10이 차례로 다음 단계 인스턴스를 품습니다. 중간 컴포넌트는 Property도 requirement도 없습니다.
//! - L3 root가 theme(노랑), L5 root가 theme(빨강)으로 중간 단계에서 가립니다(shadowing).
//! - L3에는 density만 공개하는 slot `content`가 있고, L2가 그 slot에 작성한 콘텐츠는 공개하지 않은 theme을 L3 내부(노랑)
//!   대신 instance 바깥 값(파랑)으로, 공개한 density를 L3 내부 provision(3)으로 해석합니다.
//! - L10 Frame이 theme·brand를 읽습니다(L10만 requirement 선언).

mod context_common;

use context_common::*;
use stellar_core_types::ids::StableId;
use stellar_core_types::typed_value::TypedValue;
use stellar_core_types::value::FiniteF32;
use stellar_document::component::definition::ComponentLibrary;
use stellar_document::context::fixtures::*;
use stellar_document::context::key::{ContextKey, ContextScope};
use stellar_document::context::provision::{ContextOwner, ContextProvision};
use stellar_document::context::query::inspect_binding;
use stellar_document::context::resolve::{ContextOrigin, ContextResolver, ContextSite, NoInputs};
use stellar_document::context::set::{ContextKeys, ContextSet};
use stellar_document::context::validate::{validate_context, ContextConsumer, ContextSeverity};
use stellar_document::model::DocumentModel;
use stellar_project::document_io::load::{load_document, MemoryChunks};
use stellar_project::document_io::serialize::serialize_document;
use stellar_project::document_io::source_v4::context as context_source;
use stellar_semantic_diff::build::SnapshotBuilder;
use stellar_semantic_diff::codec::snapshot_to_bytes;

const DOC: u64 = 1;
const VIEW: u64 = 3;
const ROOT: u64 = 10;
const TOP: u64 = 11;
const THEME: u64 = 900;
const BRAND: u64 = 901;
const DENSITY: u64 = 902;
const LIBRARY: u64 = 77;
const SLOT: u64 = 3_500;
const CONTENT: u64 = 2_500;
const LEAF_FRAME: u64 = 10_500;

/// 단계 i(1~10)의 컴포넌트·root·다음 단계 인스턴스 id.
fn component(i: u64) -> u64 {
    i * 1_000
}
fn root(i: u64) -> u64 {
    i * 1_000 + 1
}
fn nested(i: u64) -> u64 {
    i * 1_000 + 2
}
fn host(i: u64) -> u64 {
    i * 1_000 + 3
}

fn world() -> (DocumentModel, ComponentLibrary, ContextSet) {
    let mut library = ComponentLibrary::new();
    for i in (1..=10u64).rev() {
        let mut builder = ComponentBuilder::new(sid(component(i)), &format!("L{i}"), sid(root(i)));
        if i == 10 {
            builder = builder.frame(sid(LEAF_FRAME), sid(root(10)));
        } else {
            let mut slots = Vec::new();
            if i == 2 {
                // L2가 L3 인스턴스의 slot에 콘텐츠를 작성합니다.
                slots.push((sid(SLOT), vec![sid(CONTENT)]));
            }
            builder = builder.instance(
                sid(nested(i)),
                sid(root(i)),
                instance_of(sid(component(i + 1)), &slots),
            );
            if i == 2 {
                builder = builder.frame(sid(CONTENT), sid(nested(2)));
            }
        }
        if i == 3 {
            builder = builder
                .frame(sid(host(3)), sid(root(3)))
                .slot(sid(SLOT), sid(host(3)));
        }
        builder.register(&mut library);
    }
    let mut model = SceneBuilder::new(sid(DOC), sid(2), sid(VIEW), sid(ROOT))
        .instance(sid(TOP), sid(ROOT), instance_of(sid(component(1)), &[]))
        .build();
    let mut set = ContextSet::new();
    set.insert_key(
        ContextKey::new(
            sid(THEME),
            "theme",
            ContextScope::Document(sid(DOC)),
            color_type(),
            literal(THEME, rgb(1.0, 1.0, 1.0)),
        )
        .unwrap(),
    )
    .unwrap();
    set.insert_key(
        ContextKey::new(
            sid(DENSITY),
            "density",
            ContextScope::Document(sid(DOC)),
            number_type(),
            number(DENSITY, 1.0),
        )
        .unwrap(),
    )
    .unwrap();
    let provisions = [
        (
            60_001,
            ContextOwner::View(sid(VIEW)),
            ROOT,
            THEME,
            literal(60_001, rgb(0.0, 0.0, 1.0)),
        ),
        (
            60_002,
            ContextOwner::View(sid(VIEW)),
            ROOT,
            BRAND,
            literal(60_002, rgb(0.0, 1.0, 0.0)),
        ),
        (
            60_003,
            ContextOwner::Component(sid(component(3))),
            root(3),
            THEME,
            literal(60_003, rgb(1.0, 1.0, 0.0)),
        ),
        (
            60_004,
            ContextOwner::Component(sid(component(5))),
            root(5),
            THEME,
            literal(60_004, rgb(1.0, 0.0, 0.0)),
        ),
        (
            60_005,
            ContextOwner::Component(sid(component(3))),
            host(3),
            DENSITY,
            number(60_005, 3.0),
        ),
    ];
    for (id, owner, provider, key, value) in provisions {
        set.insert_provision(
            ContextProvision::new(sid(id), owner, sid(provider), sid(key), value).unwrap(),
        )
        .unwrap();
    }
    set.add_slot_key(sid(SLOT), sid(DENSITY));
    set.add_requirement(sid(component(10)), sid(THEME));
    set.add_requirement(sid(component(10)), sid(BRAND));
    set.add_requirement(sid(component(2)), sid(THEME));
    set.add_requirement(sid(component(2)), sid(DENSITY));
    model.replace_context(set);
    let mut external = ContextSet::new();
    external
        .insert_key(
            ContextKey::new(
                sid(BRAND),
                "brand",
                ContextScope::Library(sid(LIBRARY)),
                color_type(),
                literal(BRAND, rgb(0.5, 0.5, 0.5)),
            )
            .unwrap(),
        )
        .unwrap();
    (model, library, external)
}

fn leaf_site() -> ContextSite {
    let mut chain = vec![sid(TOP)];
    chain.extend((1..=9).map(|i| sid(nested(i))));
    ContextSite::virtual_node(chain, sid(LEAF_FRAME))
}

fn content_site() -> ContextSite {
    ContextSite::virtual_node(vec![sid(TOP), sid(nested(1))], sid(CONTENT))
}

fn consumers() -> Vec<ContextConsumer> {
    let read = |binding: u64, site: ContextSite, key: u64, expected| ContextConsumer {
        binding: sid(binding),
        site,
        expected,
        source: context_read(binding, sid(key)),
    };
    vec![
        read(20_001, leaf_site(), THEME, color_type()),
        read(20_002, leaf_site(), BRAND, color_type()),
        read(20_003, content_site(), THEME, color_type()),
        read(20_004, content_site(), DENSITY, number_type()),
    ]
}

fn snapshot_bytes(model: &DocumentModel, library: &ComponentLibrary) -> Vec<u8> {
    let mut builder = SnapshotBuilder::new();
    builder.add_document(&model.to_records().unwrap()).unwrap();
    builder.add_component_library(sid(DOC), library).unwrap();
    builder
        .add_context(sid(DOC), &model.context().live(model))
        .unwrap();
    snapshot_to_bytes(&builder.finish())
}

#[test]
fn theme_passes_ten_nested_components_with_shadowing_library_key_and_slots() {
    let (model, library, external) = world();
    let keys = ContextKeys::new(model.context()).with(&external);
    let consumers = consumers();

    // validator 0 오류(중간 컴포넌트에 Property·requirement 없음).
    let diagnostics = validate_context(&model, &library, &keys, &consumers, &NoInputs);
    let errors: Vec<_> = diagnostics
        .iter()
        .filter(|d| {
            matches!(
                d.severity(),
                ContextSeverity::Error | ContextSeverity::Orphan | ContextSeverity::Warning
            )
        })
        .collect();
    assert!(errors.is_empty(), "{errors:?}");

    // 해석: 10단계 아래 leaf는 가장 가까운 L5 theme(빨강), 외부 라이브러리 key brand는 View root(초록).
    let resolver = ContextResolver::new(&model, &library);
    let theme = resolver.resolve(sid(THEME), &leaf_site()).unwrap();
    assert_eq!(theme.provision(), Some(sid(60_004)));
    assert_eq!(theme.shadowed, vec![sid(60_003), sid(60_001)]);
    assert_eq!(theme.path.len(), 1 + 1 + 5 * 2);
    let brand = resolver.resolve(sid(BRAND), &leaf_site()).unwrap();
    assert_eq!(brand.provision(), Some(sid(60_002)));
    // slot 콘텐츠: 공개하지 않은 theme은 L3 내부(노랑) 대신 instance 바깥(파랑), 공개한 density는 L3 내부(3).
    let slot_theme = resolver.resolve(sid(THEME), &content_site()).unwrap();
    assert_eq!(slot_theme.provision(), Some(sid(60_001)));
    assert_eq!(slot_theme.skipped_slots, vec![sid(SLOT)]);
    let slot_density = resolver.resolve(sid(DENSITY), &content_site()).unwrap();
    assert_eq!(slot_density.provision(), Some(sid(60_005)));
    assert_eq!(slot_density.via_slots, vec![sid(SLOT)]);

    // binding graph 값과 provenance(provision 경로 포함).
    let graph = context_graph(&model, &library, &[&external], &consumers);
    let evaluator = evaluator(&graph, &consumers);
    let red = rgb(1.0, 0.0, 0.0);
    let blue = rgb(0.0, 0.0, 1.0);
    let green = rgb(0.0, 1.0, 0.0);
    assert_eq!(value(&evaluator, &consumers[0]), Some(red));
    assert_eq!(value(&evaluator, &consumers[1]), Some(green));
    assert_eq!(value(&evaluator, &consumers[2]), Some(blue));
    assert_eq!(
        value(&evaluator, &consumers[3]),
        Some(TypedValue::Number(FiniteF32::new(3.0).unwrap()))
    );
    let trace = evaluator
        .context_provenance(&consumer_node(&consumers[0]), &graph)
        .unwrap();
    assert_eq!(trace.providers(), vec![sid(root(5))]);
    let info = inspect_binding(&resolver, &keys, &consumers[0], &NoInputs).unwrap();
    assert_eq!(
        info[0].origin,
        ContextOrigin::Provision {
            provision: sid(60_004),
            provider: sid(root(5)),
            instances: [TOP, nested(1), nested(2), nested(3), nested(4)]
                .iter()
                .map(|n| sid(*n))
                .collect(),
        }
    );
    assert_eq!(info[0].path.first(), Some(&leaf_site()));
    assert_eq!(info[0].path.last().map(|s| s.node), Some(sid(root(5))));

    // semantic snapshot byte-identical: 같은 입력, chunk cache·텍스트 원본을 거친 Context에서 같은 bytes.
    let first = snapshot_bytes(&model, &library);
    assert_eq!(snapshot_bytes(&model, &library), first);
    let serialized = serialize_document(&model).unwrap();
    let mut chunks = MemoryChunks::new();
    chunks.insert_document(&serialized);
    let loaded = load_document(&serialized.manifest.payload, &mut chunks)
        .unwrap()
        .model;
    assert_eq!(snapshot_bytes(&loaded, &library), first);
    let text = context_source::encode(model.context()).unwrap();
    let mut from_text = loaded;
    from_text.replace_context(context_source::decode(&text).unwrap());
    assert_eq!(snapshot_bytes(&from_text, &library), first);
    assert_ne!(sid(THEME), StableId::NIL);
}
