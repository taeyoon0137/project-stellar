# 작업 보고서: XVI.4.8 binding 저장소 역참조·orphan target index

- WBS: XVI.4.8
- 작업명: `BindingStore`에 token → binding(live·orphan) 역참조 index와 orphan target index를 두어 선형 검색을 없앰
- 상태: 완료 (`[x]`)
- 선행: VIII.5.5, VIII.8.2
- 결정 기록: ADR-0108 결정 2

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, release 측정 포함).

## 변경 파일 목록

- `crates/bindings/src/store.rs`(신규) — `BindingStore`를 orphan.rs에서 옮기고 두 index(`token_readers`, `orphan_targets`)와
  조회(`bindings_reading`, `orphan_by_target`), index를 함께 갱신하는 변경 메서드(`insert_live`, `remove_live`,
  `insert_orphan`, `remove_orphan`, `orphan_broken`)를 둠. random 편집 property test.
- `crates/bindings/src/orphan.rs` — `BindingStore` 정의를 지우고 `pub use crate::store::BindingStore`로 같은 경로를 유지.
- `crates/bindings/src/token_commands.rs` — `TokenOrphanPlan::current`가 모든 binding source를 훑는 대신 역참조 index를 씀.
- `crates/bindings/src/fallback.rs` — `StoreTypes::reference_type`의 orphan 선형 검색을 `orphan_by_target`으로 바꿈.
- `crates/bindings/src/orphan_commands.rs` — `live_mut()`(index를 우회하는 가변 참조)를 index를 갱신하는 `insert_live`·
  `remove_live`로 바꿈.
- `crates/bindings/src/store_bench.rs`(신규, test 전용), `crates/bindings/src/lib.rs`(module 등록).

## 구현 요약

- **토큰 역참조**: 토큰 → `(binding ID, live|orphan)` 집합. binding source의 토큰 참조(`referenced_targets` 중 Token)로
  만들며, 같은 ID가 live·orphan 두 쪽에 겹친 경우에도 한쪽을 지울 때 다른 쪽 항목이 남도록 위치를 함께 둡니다.
  `TokenOrphanPlan::current`의 binding 목록은 이 index의 ID 집합입니다(옛 판정 `refers_to`와 같은 결과).
- **orphan target**: target 속성 → orphan ID 집합. `orphan_by_target`은 가장 작은 ID의 orphan을 돌려줘, 옛
  `orphans().find(target)`(ID 순 첫 번째)과 같은 결과입니다.
- **갱신 경로**: 저장소를 바꾸는 방법은 위 메서드뿐입니다. `live_mut()`를 없애 index를 우회하는 변경이 불가능합니다.
  index는 내용에서 결정적으로 정해지므로 `PartialEq`(같은 내용 ⇒ 같은 index)도 그대로 성립합니다.
- 판정 결과는 바꾸지 않았습니다.

## 성능

`cargo test -p stellar-bindings --release store_bench -- --ignored --nocapture`. synthetic fixture(VIII.9.1, 토큰 1,000 +
binding 1,000·10,000, 모두 Number). 다른 작업 12개가 동시에 도는 부하 상태에서 잰 값입니다.

| 측정 | binding 1,000 | binding 10,000 | 비율 |
|---|---:|---:|---:|
| 토큰 Delete validate(index) | 120 µs | 106 µs | 0.88배 |
| 같은 inbound 검사를 전체 순회로(이전 방식) | 121 µs | 229 µs | 1.9배 |
| orphan 1,000개 저장 기본값 해석(index) | 1.34 ms | 0.87 ms | — |
| 같은 해석을 orphan 선형 검색으로(이전 방식) | 2.23 ms | 1.84 ms | — |
| orphan 1,000개 open 전체(`open_with_stored_defaults`) | 3.95 ms | 24.2 ms | 6.1배 |

- 토큰 Delete validate는 binding 10배에서 0.88배로 기준(2배 이하)을 만족합니다. 시험은 2배를 넘으면 실패합니다. 남은
  120 µs는 거의 모두 토큰 쪽 inbound 검사(`TokenRegistry::referrers`, 토큰 1,000개의 default expression을 훑음)입니다.
- orphan 1,000개 fixture는 각 orphan의 저장 기본값이 직전 orphan target 속성을 읽게 해, 기본값 type 검사가 Property
  참조 타입(옛 선형 검색 경로)을 실제로 지나게 했습니다. index와 이전 방식의 해석 결과(진단)는 같습니다.
- open 전체가 binding 수에 비례하는 것은 평가기가 모든 binding을 평가하기 때문입니다(이 행이 바꾸는 조회가 아님).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-bindings` | unit 55 passed, `fallback_golden_corpus` 2, `token_change_benchmark` 3 passed |
| `cargo test -p stellar-bindings --release store_bench -- --ignored --nocapture` | 위 표, 2 passed |
| `cargo test -p stellar-integrity -p stellar-prototype` | 통과(저장소를 쓰는 다른 crate) |
| `cargo clippy -p stellar-bindings -p stellar-integrity -p stellar-prototype --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-bindings -- --check` | 통과 |

## 테스트

| 검사 | 결과 |
|---|---|
| random 편집(seed 60 × 80단계: live 추가, live → orphan, orphan 재연결·삭제, 같은 ID orphan 교체) 뒤 index 조회가 전체 순회 결과와 같고, 같은 내용으로 다시 만든 저장소와 같음(`indexes_match_full_scans_after_random_edits`) | 통과 |
| 기존 VIII.5.5(token_commands), VIII.8.2·VIII.8.3(fallback, golden corpus), VIII.8.4(orphan_commands) test | 통과 |

## 차이(작업 지시와 다른 점)

작업 지시와 다른 점은 아래 "남은 위험 / 한계"에 함께 적었습니다.

## 남은 위험 / 한계

- 토큰 inbound 검사의 토큰 쪽(`TokenRegistry::referrers`)은 토큰 수에 비례합니다. registry가 토큰 역참조를 증분 유지해야
  없앨 수 있고, 이 행의 범위(binding 저장소 조회)와 소유 범위 밖입니다.
