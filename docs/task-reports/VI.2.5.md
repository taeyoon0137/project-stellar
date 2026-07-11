# 작업 보고서: VI.2.5 ReferenceIndex

- WBS: VI.2.5
- 작업명: node 삭제 전 참조 inbound 목록 조회 `ReferenceIndex` 인터페이스
- 상태: 완료 (`[x]`)
- 선행: III.2.3, VI.2.2

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/reference_index.rs`(신규) — `ReferenceIndex`
  (add/remove/remove_all_from/inbound/outbound/has_inbound/edge_count).
- `crates/document/src/lib.rs` — 모듈 등록.
- `docs/task-reports/VI.2.5.md`(신규), 계획서(VI.2.5 `[ ]`→`[x]`).

## 구현 요약

- **양방향 index**: from→to(outbound)와 to→from(inbound)을 `BTreeMap<StableId,
  BTreeSet>`으로 동기 유지합니다(결정론적 순서). 중복 edge는 멱등, 해제는 양방향
  동시 갱신, `remove_all_from`은 node 삭제 준비용으로 outbound만 정리합니다.
- **정책 비포함**: index는 사실만 보고합니다 — 참조가 남은 node의 삭제
  허용·차단·정리는 command 단계(Phase VII)에서 결정합니다. VI.9.4가
  Boolean/Mask/Clip modifier 의존성을 이 index에 등록합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document` | 47 passed, 0 failed |
| `cargo fmt -p stellar-document -- --check` | 통과 |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| inbound/outbound fixture 정확(3-edge 구성)(`inbound_and_outbound_fixtures_are_accurate`) | 통과 |
| 중복 멱등, 양방향 해제, outbound 전체 정리 시 타 참조 유지 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- edge에 참조 위치(PropertyPath)·종류 label은 아직 없습니다. modifier 등록
  (VI.9.4)에서 필요해지면 edge payload를 확장합니다.
