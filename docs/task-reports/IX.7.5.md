# 작업 보고서: IX.7.5 같은 구체성 Variant 충돌 진단

- WBS: IX.7.5
- 작업명: 동일 specificity variant가 같은 PropertyPath를 다르게 override하면 conflict diagnostic을 생성하고 임의 순서로 적용하지 않음
- 상태: 완료 (`[x]`)
- 선행: IX.7.4(이 lane에서 `[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/view_variant/resolver.rs` — `PatchKey`, `PatchValue`, `VariantConflict`, `VariantResolution`,
  `resolve_variants`, 충돌 fixture 테스트.
- `docs/task-reports/IX.7.5.md`(신규), 계획서(IX.7.5 `[ ]`→`[x]`).

## 구현 요약

- `resolve_variants`는 일치 Variant(IX.7.4 순서)의 patch를 대상별로 모읍니다. 대상은 PropertyPath, 구조 patch
  node, (인스턴스, slot)입니다(IX.7.3 VariantOverrideSet의 세 갈래).
- 대상마다 **가장 구체적인 group**만 값을 정합니다. 그 group의 값이 모두 같으면 적용(`applied`, 값을 정한 Variant
  id 포함), 다르면 `VariantConflict { key, specificity, variants }`를 만들고 그 대상은 **적용하지 않습니다** — id
  정렬 순서로 임의 선택하지 않습니다.
- 더 구체적인 Variant가 같은 대상을 지정하면 아래 group의 모호함은 해소됩니다(기술 아키텍처 16장).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-document` | 완료 |
| `cargo test -p stellar-document` | unit 310 passed, 통합 7+1+1+2 passed, 0 failed |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| compact(1)·error(1)가 padding을 8·12로 지정, compact+error 할당 → 충돌 1건(구체성 1, 두 Variant), 적용 0건(`same_specificity_conflict_is_reported_and_not_applied`) | 통과(모호함 실패) |
| 한쪽만 일치 → 적용, 같은 값 → 충돌 아님, compact+error(2) Variant 추가 → 해소·조합 값 4 적용(같은 테스트) | 통과 |

## 차이(작업 지시와 다른 점)

- PropertyPath뿐 아니라 구조 patch·slot change도 같은 규칙으로 충돌을 판정합니다(같은 node를 같은 구체성에서
  다르게 이동·삭제하는 경우 등).

## 남은 위험 / 한계

- 서로 다른 PropertyPath가 같은 결과 속성에 영향을 주는 경우(예: 부모 레이아웃 vs 자식 크기)는 대상이 달라 충돌로
  보지 않습니다.
