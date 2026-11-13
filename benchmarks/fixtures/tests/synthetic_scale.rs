//! XVI.3.1 검증: 규모별 synthetic fixture가 seed로 재현되고(같은 seed content hash 동일), 실제
//! 문서 모델·validator·serializer·loader를 통과합니다.
//!
//! 1만 tier와 축소 프로젝트 변형은 CI에서 실행합니다. 20만·100만 tier는 같은 생성 경로의 대형
//! 실행이라 `#[ignore]`이며 `cargo test -p stellar-bench-fixtures --release -- --ignored`로
//! 필요할 때 실행합니다.

use std::collections::BTreeSet;
use std::path::PathBuf;

use stellar_bench_fixtures::split::{fixture_split_policy, summarize_split};
use stellar_bench_fixtures::synthetic::{
    generate_synthetic, ScaleTier, SyntheticSpec, SyntheticSummary,
};
use stellar_bench_fixtures::GENERATOR_VERSION;
use stellar_core_types::ids::StableId;
use stellar_document::validate::MAX_NODE_DEPTH;
use stellar_project::document_io::load::{load_document, MemoryChunks};
use stellar_storage::chunk::budget::ChunkBudget;
use stellar_storage::chunk::split::SplitPolicy;

fn budget_policy() -> SplitPolicy {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../config/performance-budgets.yaml");
    let budget = ChunkBudget::from_budget_file(&path).expect("성능 예산 파일");
    fixture_split_policy(&budget)
}

fn summary_only(spec: &SyntheticSpec) -> SyntheticSummary {
    generate_synthetic(spec, |_| Ok(())).expect("fixture 생성")
}

#[test]
fn nodes_10k_tier_is_reproducible_valid_and_round_trips() {
    let spec = SyntheticSpec::tier(ScaleTier::Nodes10k, 1);
    let policy = budget_policy();
    let mut checked = 0usize;
    let first = generate_synthetic(&spec, |document| {
        let model = &document.built.model;
        // 정확히 1만 node, depth 상한 이내.
        assert_eq!(model.node_count(), 10_000);
        assert_eq!(document.stats.nodes, 10_000);
        assert_eq!(document.stats.kinds.total(), 10_000);
        assert!(document.stats.max_depth <= MAX_NODE_DEPTH);
        assert_eq!(document.stats.views, 20);
        // 모든 종류와 modifier·참조가 실제로 포함됩니다.
        let kinds = document.stats.kinds;
        for count in [
            kinds.frame,
            kinds.fragment,
            kinds.shape,
            kinds.path,
            kinds.text,
            kinds.image,
        ] {
            assert!(count > 0, "종류가 빠졌습니다: {kinds:?}");
        }
        assert!(document.stats.modifier_stacks > 0);
        assert!(document.stats.node_references > 0);

        // chunk model → loader round-trip이 같은 영속 레코드를 만듭니다.
        let mut chunks = MemoryChunks::new();
        chunks.insert_document(&document.built.serialized);
        let loaded =
            load_document(&document.built.serialized.manifest.payload, &mut chunks).expect("로드");
        assert!(loaded.diagnostics.unresolved.is_empty());
        assert_eq!(
            loaded.model.to_records().unwrap(),
            model.to_records().unwrap()
        );

        // V.6.4 split planner: View subtree가 임계치 아래라 oversized chunk가 없습니다.
        let split = summarize_split(model, &policy).expect("split 계획");
        assert_eq!(split.views, 20);
        assert_eq!(split.oversized_chunks, 0);
        assert_eq!(
            split.planned_chunks, split.views,
            "분할이 필요한 View가 없습니다"
        );
        assert!(split.max_view_bytes <= policy.threshold_bytes);
        checked += 1;
        Ok(())
    })
    .expect("fixture 생성");
    assert_eq!(checked, 1);
    assert_eq!(first.nodes, 10_000);
    assert_eq!(first.generator_version, GENERATOR_VERSION);
    assert_eq!(first.name, "nodes-10k");

    // 같은 seed → 같은 content hash(별도 실행에서 전체 재생성).
    let second = summary_only(&spec);
    assert_eq!(first.content_hash, second.content_hash);
    assert_eq!(first, second);

    // 다른 seed → 다른 내용.
    let other = summary_only(&SyntheticSpec::tier(ScaleTier::Nodes10k, 2));
    assert_ne!(first.content_hash, other.content_hash);
    assert_eq!(other.nodes, 10_000);
}

#[test]
fn project_smoke_variant_keeps_ids_unique_across_documents() {
    // 프로젝트 tier와 같은 경로(여러 문서, 공유 id 발급기)의 CI 축소 변형.
    let spec = SyntheticSpec::custom("project-smoke", 7, vec![3_000, 1_500, 1_500, 777]);
    let mut ids: BTreeSet<StableId> = BTreeSet::new();
    let mut nodes = 0usize;
    let summary = generate_synthetic(&spec, |document| {
        let records = document.built.model.to_records().unwrap();
        assert!(ids.insert(records.definition.id));
        for page in &records.definition.pages {
            assert!(ids.insert(page.id));
            for view in &page.views {
                assert!(ids.insert(view.id));
            }
        }
        for node in &records.nodes {
            assert!(ids.insert(node.id), "문서 사이 node id 중복");
        }
        nodes += records.nodes.len();
        Ok(())
    })
    .expect("fixture 생성");
    assert_eq!(nodes, 6_777);
    assert_eq!(summary.nodes, 6_777);
    assert_eq!(summary.documents, 4);
    assert_eq!(
        summary
            .document_stats
            .iter()
            .map(|s| s.nodes)
            .collect::<Vec<_>>(),
        vec![3_000, 1_500, 1_500, 777]
    );
    assert_eq!(summary.content_hash, summary_only(&spec).content_hash);

    // 이름·규모 매개변수가 hash에 들어갑니다.
    let renamed = SyntheticSpec::custom("project-smoke-2", 7, spec.document_nodes.clone());
    assert_ne!(summary_only(&renamed).content_hash, summary.content_hash);
    let resized = SyntheticSpec::custom("project-smoke", 7, vec![3_000, 1_500, 1_500, 778]);
    assert_ne!(summary_only(&resized).content_hash, summary.content_hash);
}

#[test]
#[ignore = "대형 tier: --release -- --ignored로 실행"]
fn nodes_200k_tier_is_reproducible() {
    let spec = SyntheticSpec::tier(ScaleTier::Nodes200k, 1);
    let policy = budget_policy();
    let first = generate_synthetic(&spec, |document| {
        assert_eq!(document.built.model.node_count(), 200_000);
        let split = summarize_split(&document.built.model, &policy).expect("split 계획");
        assert_eq!(split.oversized_chunks, 0);
        Ok(())
    })
    .expect("fixture 생성");
    assert_eq!(first.nodes, 200_000);
    assert_eq!(first.content_hash, summary_only(&spec).content_hash);
}

#[test]
#[ignore = "대형 tier: --release -- --ignored로 실행"]
fn project_1m_tier_is_reproducible() {
    let spec = SyntheticSpec::tier(ScaleTier::Project1M, 1);
    let first = summary_only(&spec);
    assert_eq!(first.nodes, 1_000_000);
    assert_eq!(first.documents, 9);
    assert_eq!(first.content_hash, summary_only(&spec).content_hash);
}
