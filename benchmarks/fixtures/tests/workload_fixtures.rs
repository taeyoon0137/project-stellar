//! XVI.3.3 검증: workload fixture마다 목표 병목 특성이 계측값으로 기준 synthetic 문서보다
//! 확연히 큽니다. token-heavy는 토큰 모델(VIII.5)이 없어 만들 수 없음을 명시합니다.

use stellar_bench_fixtures::synthetic::{generate_synthetic, SyntheticSpec};
use stellar_bench_fixtures::workload::{
    constraint_heavy, profile_constraints, profile_document, workload_document, DocumentProfile,
    DocumentWorkload, UNAVAILABLE_WORKLOADS,
};

const NODES: usize = 5_000;

/// 같은 node 수의 기준 synthetic 문서 계측값.
fn baseline(seed: u64) -> DocumentProfile {
    let mut profile = None;
    generate_synthetic(
        &SyntheticSpec::custom("baseline", seed, vec![NODES]),
        |doc| {
            profile = Some(profile_document(&doc.built.model)?);
            Ok(())
        },
    )
    .expect("기준 문서");
    profile.expect("기준 계측값")
}

fn measured(workload: DocumentWorkload, seed: u64) -> DocumentProfile {
    let fixture = workload_document(workload, seed, NODES).expect("workload 생성");
    assert_eq!(fixture.built.model.node_count(), NODES);
    // 재현: 같은 seed는 같은 hash, 다른 seed는 다른 hash.
    assert_eq!(
        workload_document(workload, seed, NODES)
            .unwrap()
            .content_hash,
        fixture.content_hash
    );
    assert_ne!(
        workload_document(workload, seed + 1, NODES)
            .unwrap()
            .content_hash,
        fixture.content_hash
    );
    profile_document(&fixture.built.model).expect("계측")
}

#[test]
fn text_heavy_dominates_text_volume() {
    let base = baseline(1);
    let text = measured(DocumentWorkload::TextHeavy, 1);
    assert_eq!(text.nodes, NODES);
    // node당 문자 수가 기준의 10배 이상, node payload bytes의 80% 이상이 Text.
    assert!(
        text.text_scalars_per_node() >= 10.0 * base.text_scalars_per_node(),
        "text {} vs base {}",
        text.text_scalars_per_node(),
        base.text_scalars_per_node()
    );
    assert!(text.text_byte_share() >= 0.8, "{}", text.text_byte_share());
    // shaping·줄바꿈 부하: Text node당 style run ≥ 4, 문단 ≥ 3.
    assert!(text.style_runs >= 4 * text.text_nodes as u64);
    assert!(text.text_paragraphs >= 3 * text.text_nodes as u64);
}

#[test]
fn image_heavy_dominates_image_count_assets_and_area() {
    let base = baseline(1);
    let image = measured(DocumentWorkload::ImageHeavy, 1);
    assert!(image.image_share() >= 0.75, "{}", image.image_share());
    assert!(image.image_share() >= 5.0 * base.image_share());
    // 이미지마다 서로 다른 asset: 디코딩·GPU 업로드 대상이 기준의 10배 이상.
    assert_eq!(image.distinct_assets, image.image_nodes);
    assert!(image.distinct_assets >= 10 * base.distinct_assets.max(1));
    // 배치 면적: 이미지당 512² px 이상.
    assert!(image.image_pixels >= image.image_nodes as u64 * 512 * 512);
}

#[test]
fn effect_heavy_dominates_effect_modifiers() {
    let base = baseline(1);
    let effect = measured(DocumentWorkload::EffectHeavy, 1);
    // node당 effect modifier ≥ 2이고 기준의 10배 이상.
    assert!(
        effect.effects_per_node() >= 2.0,
        "{}",
        effect.effects_per_node()
    );
    assert!(effect.effects_per_node() >= 10.0 * base.effects_per_node());
    // visual bounds를 바꾸는(geometry 무효화) modifier가 node당 1개 이상.
    assert!(effect.geometry_modifiers >= effect.nodes);
    assert!(effect.modifier_bytes > effect.payload_bytes);
}

#[test]
fn constraint_heavy_is_solved_by_layout_engine_with_long_chains() {
    let fixture = constraint_heavy(1, NODES).expect("fixture 생성");
    let profile = profile_constraints(&fixture).expect("계측");
    assert_eq!(profile.nodes, NODES);
    // root 외 모든 node가 constraint로 배치되고 node당 등식이 2.5개 이상.
    assert_eq!(profile.constrained_nodes, NODES - 1);
    assert!(profile.constraints_per_node() >= 2.5);
    // panel마다 길이 32의 sibling 의존 chain(연결 요소).
    assert!(profile.max_component >= 32);
    assert!(profile.sibling_constraints >= NODES);
    // 레이아웃 엔진이 모두 배치했고 진단(미해결·underdetermined)이 없습니다.
    assert_eq!(profile.laid_out, NODES);
    assert_eq!(profile.diagnostics, 0);
    assert_eq!(
        constraint_heavy(1, NODES).unwrap().content_hash,
        fixture.content_hash
    );
    assert_ne!(
        constraint_heavy(2, NODES).unwrap().content_hash,
        fixture.content_hash
    );
}

#[test]
fn token_heavy_is_reported_as_unavailable() {
    assert_eq!(UNAVAILABLE_WORKLOADS.len(), 1);
    assert_eq!(UNAVAILABLE_WORKLOADS[0].0, "token-heavy");
    let names: Vec<&str> = DocumentWorkload::ALL.iter().map(|w| w.name()).collect();
    assert_eq!(names, vec!["text-heavy", "image-heavy", "effect-heavy"]);
}
