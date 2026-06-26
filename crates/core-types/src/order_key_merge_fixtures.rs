//! OrderKey Semantic Merge fixture (III.5.4).
//!
//! 서로 다른 브랜치가 **독립적으로 삽입**해도 충돌 없이 합쳐지는 시나리오의 **fixture data**
//! 와 그 **스키마(구조) 검증**만 제공합니다. Semantic Merge 알고리즘은 구현하지 않습니다
//! (후속 Phase). 본 fixture는 추후 `semantic-merge`가 소비합니다.
//!
//! 배치 근거: 이 fixture는 [`OrderKey`]·[`StableId`]에 의존합니다. 의존성 그래프상
//! `semantic-merge`는 아직 `core-types`를 직접 의존하지 않으므로, fixture는 `OrderKey`가 있는
//! `core-types`에 재사용 가능한 `pub` 모듈로 둡니다.
//!
//! 핵심 성질: 두 브랜치가 같은 이웃 사이에 삽입하면 [`OrderKey::between`]이 결정론적이라
//! **동일한 키**가 나올 수 있습니다. 이때 충돌은 `StableId` tie-break으로 해소되어, 두 삽입이
//! 모두 결정론적 순서로 유지됩니다(중앙 조정 불필요).

use crate::ids::StableId;
use crate::order_key::OrderKey;

/// 한 자식의 순서 키와 안정 ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderedChild {
    /// 순서 키.
    pub key: OrderKey,
    /// 안정 식별자.
    pub id: StableId,
}

impl OrderedChild {
    /// 키·ID로 자식을 만듭니다.
    pub fn new(key: OrderKey, id: StableId) -> OrderedChild {
        OrderedChild { key, id }
    }
}

/// 독립 삽입 merge fixture.
///
/// `base`에 대해 두 브랜치가 각각 `branch_a_inserts`·`branch_b_inserts`를 독립적으로 삽입한
/// 뒤, 결정론적으로 합쳐진 기대 순서가 `expected_merged`(자식 `StableId` 나열)입니다.
#[derive(Debug, Clone)]
pub struct OrderKeyMergeFixture {
    /// fixture 이름.
    pub name: &'static str,
    /// 공통 base 자식들.
    pub base: Vec<OrderedChild>,
    /// 브랜치 A의 독립 삽입.
    pub branch_a_inserts: Vec<OrderedChild>,
    /// 브랜치 B의 독립 삽입.
    pub branch_b_inserts: Vec<OrderedChild>,
    /// 기대 병합 순서(자식 `StableId`, 시각 순서).
    pub expected_merged: Vec<StableId>,
}

/// fixture 스키마(구조) 위반.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FixtureError {
    /// 두 브랜치 중 하나라도 삽입이 없음(독립 삽입 시나리오가 아님).
    EmptyBranch,
    /// 자식 `StableId`가 중복됨.
    DuplicateId,
    /// 비정규 키(비어 있거나 `0x00`으로 끝남).
    NonCanonicalKey,
    /// `expected_merged`가 전체 자식 ID의 순열이 아님.
    MergedNotPermutation,
    /// `expected_merged` 순서가 결정론적 순서(`(key, id)` 오름차순)와 다름.
    MergedNotDeterministicOrder,
}

impl OrderKeyMergeFixture {
    /// 전체 자식(base + 두 브랜치 삽입)을 모읍니다.
    fn all_children(&self) -> Vec<OrderedChild> {
        let mut all = self.base.clone();
        all.extend(self.branch_a_inserts.iter().cloned());
        all.extend(self.branch_b_inserts.iter().cloned());
        all
    }

    /// fixture가 스키마(구조) 제약을 만족하는지 검증합니다.
    ///
    /// merge 알고리즘을 실행하지 않고, fixture data 자체의 일관성만 확인합니다: 두 브랜치 모두
    /// 삽입 존재, ID 유일, 키 정규형, `expected_merged`가 전체 ID 순열이며 결정론적
    /// `(key, id)` 오름차순과 일치.
    pub fn validate(&self) -> Result<(), FixtureError> {
        if self.branch_a_inserts.is_empty() || self.branch_b_inserts.is_empty() {
            return Err(FixtureError::EmptyBranch);
        }

        let all = self.all_children();

        // 키 정규형(비어 있지 않고 0x00 미종료).
        for child in &all {
            match child.key.as_bytes().last() {
                None | Some(0) => return Err(FixtureError::NonCanonicalKey),
                Some(_) => {}
            }
        }

        // ID 유일성.
        let mut ids: Vec<StableId> = all.iter().map(|c| c.id).collect();
        ids.sort();
        for pair in ids.windows(2) {
            if pair[0] == pair[1] {
                return Err(FixtureError::DuplicateId);
            }
        }

        // expected_merged가 전체 ID의 순열인지(같은 multiset).
        let mut expected_sorted = self.expected_merged.clone();
        expected_sorted.sort();
        if expected_sorted != ids {
            return Err(FixtureError::MergedNotPermutation);
        }

        // 결정론적 순서: (key, id) 오름차순.
        let mut deterministic = all.clone();
        deterministic.sort_by(|a, b| a.key.cmp(&b.key).then(a.id.cmp(&b.id)));
        let deterministic_ids: Vec<StableId> = deterministic.iter().map(|c| c.id).collect();
        if self.expected_merged != deterministic_ids {
            return Err(FixtureError::MergedNotDeterministicOrder);
        }

        Ok(())
    }
}

/// 모든 OrderKey merge fixture를 반환합니다.
pub fn fixtures() -> Vec<OrderKeyMergeFixture> {
    let k0 = OrderKey::from_bytes(vec![0x40]).unwrap();
    let k1 = OrderKey::from_bytes(vec![0xC0]).unwrap();

    let c0 = StableId::new_v7(1, 0, 1);
    let c1 = StableId::new_v7(2, 0, 2);
    let a_id = StableId::new_v7(10, 0, 10);
    let b_id = StableId::new_v7(20, 0, 20);
    // v7는 timestamp 우선이라 a_id(10) < b_id(20).
    debug_assert!(a_id < b_id);

    // fixture 1: 두 브랜치가 같은 이웃(k0, k1) 사이에 독립 삽입 → between이 결정론적이라
    // 동일 키. 충돌은 StableId tie-break으로 해소(a_id < b_id).
    let same_gap_key = OrderKey::between(Some(&k0), Some(&k1));
    let same_gap = OrderKeyMergeFixture {
        name: "same_gap_deterministic_key_tie_break",
        base: vec![
            OrderedChild::new(k0.clone(), c0),
            OrderedChild::new(k1.clone(), c1),
        ],
        branch_a_inserts: vec![OrderedChild::new(same_gap_key.clone(), a_id)],
        branch_b_inserts: vec![OrderedChild::new(same_gap_key, b_id)],
        expected_merged: vec![c0, a_id, b_id, c1],
    };

    // fixture 2: 한 브랜치는 맨 앞, 다른 브랜치는 맨 뒤에 독립 삽입 → 서로 다른 키, 충돌 없음.
    let front_key = OrderKey::between(None, Some(&k0));
    let back_key = OrderKey::between(Some(&k1), None);
    let front_back = OrderKeyMergeFixture {
        name: "disjoint_front_and_back_inserts",
        base: vec![OrderedChild::new(k0, c0), OrderedChild::new(k1, c1)],
        branch_a_inserts: vec![OrderedChild::new(front_key, a_id)],
        branch_b_inserts: vec![OrderedChild::new(back_key, b_id)],
        expected_merged: vec![a_id, c0, c1, b_id],
    };

    vec![same_gap, front_back]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_fixtures_pass_schema_validation() {
        let all = fixtures();
        assert!(!all.is_empty());
        for fixture in &all {
            assert_eq!(fixture.validate(), Ok(()), "fixture: {}", fixture.name);
        }
    }

    #[test]
    fn same_gap_fixture_has_colliding_keys_resolved_by_id() {
        // 두 브랜치 삽입이 동일 키(결정론적 between)인지, 그리고 ID 오름차순으로 풀리는지.
        let fixture = fixtures()
            .into_iter()
            .find(|f| f.name == "same_gap_deterministic_key_tie_break")
            .unwrap();
        assert_eq!(
            fixture.branch_a_inserts[0].key, fixture.branch_b_inserts[0].key,
            "독립 삽입이 결정론적으로 동일 키"
        );
        assert!(fixture.branch_a_inserts[0].id < fixture.branch_b_inserts[0].id);
        // 기대 순서에서 두 충돌 삽입이 ID 오름차순으로 인접.
        assert_eq!(
            fixture.expected_merged,
            vec![
                fixture.base[0].id,
                fixture.branch_a_inserts[0].id,
                fixture.branch_b_inserts[0].id,
                fixture.base[1].id,
            ]
        );
    }

    #[test]
    fn validate_rejects_empty_branch() {
        let fixture = OrderKeyMergeFixture {
            name: "x",
            base: Vec::new(),
            branch_a_inserts: vec![OrderedChild::new(
                OrderKey::from_bytes(vec![0x80]).unwrap(),
                StableId::new_v7(1, 0, 1),
            )],
            branch_b_inserts: Vec::new(),
            expected_merged: vec![StableId::new_v7(1, 0, 1)],
        };
        assert_eq!(fixture.validate(), Err(FixtureError::EmptyBranch));
    }

    #[test]
    fn validate_rejects_wrong_expected_order() {
        // 결정론적 순서를 일부러 뒤집어 검증 실패를 확인.
        let mut fixture = fixtures().into_iter().next().unwrap();
        fixture.expected_merged.reverse();
        assert_eq!(
            fixture.validate(),
            Err(FixtureError::MergedNotDeterministicOrder)
        );
    }

    #[test]
    fn validate_rejects_non_permutation() {
        let mut fixture = fixtures().into_iter().next().unwrap();
        // ID 하나를 엉뚱한 값으로 바꿔 순열 위반.
        fixture.expected_merged[0] = StableId::new_v7(999, 0, 999);
        assert_eq!(fixture.validate(), Err(FixtureError::MergedNotPermutation));
    }
}
