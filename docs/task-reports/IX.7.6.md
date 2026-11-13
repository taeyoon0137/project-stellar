# 작업 보고서: IX.7.6 구체 Variant 우선과 없는 조합의 일반 Variant 적용

- WBS: IX.7.6
- 작업명: 구체 variant가 일반 variant 값을 덮어쓰고 존재하지 않는 조합은 추가 override 없이 base+일반 variant만 적용
- 상태: 완료 (`[x]`)
- 선행: IX.7.4(이 lane에서 `[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/view_variant/resolver.rs` — `VariantResolution::effective_overrides`, `resolve_properties`,
  `ResolvedValue`, `ValueSource`, 16장 요구 예제 테스트.
- `docs/task-reports/IX.7.6.md`(신규), 계획서(IX.7.6 `[ ]`→`[x]`).

## 구현 요약

- `resolve_properties(base, resolution)`: Base 속성 값 위에 resolve 결과(IX.7.5 `applied`)를 적용하고 값마다 출처
  (`Base` 또는 값을 정한 Variant)를 기록합니다. 대상별로 가장 구체적인 group이 값을 정하므로 구체 Variant가 일반
  Variant 값을 덮어쓰고, 지정하지 않은 속성은 Base 또는 일반 Variant 값을 상속합니다.
- 정의되지 않은 조합에는 그 조합 전용 override가 없으므로 Base + 할당에 일치하는 일반 Variant만 적용됩니다(별도
  분기 없음 — 일치하지 않는 Variant는 IX.7.4 단계에서 빠짐).
- 충돌 대상(IX.7.5)은 Variant 값을 적용하지 않고 Base 값을 유지합니다.
- `effective_overrides()`는 적용 값을 희소 `VariantOverrideSet` 하나로 합쳐 scene·layout 생성(X.1.2·XI.2.4)이
  Base 위에 한 번 적용할 수 있게 합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-document` | 완료 |
| `cargo test -p stellar-document` | unit 311 passed, 통합 7+1+1+2 passed, 0 failed |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -- --check` | 통과 |

| 검사(`specific_overrides_general_and_missing_combinations_use_general_only`, Base padding 16·color black, compact→padding 8, error→color red, compact+error→padding 4) | 결과 |
|---|---|
| regular + idle → padding 16·color black(모두 Base) | 통과 |
| compact + processing(정의되지 않은 조합) → padding 8(compact), color black(Base) | 통과 |
| regular + error → padding 16(Base), color red(error) | 통과 |
| compact + error → padding 4(조합이 compact를 덮어씀), color red(일반 error 상속), 합친 override set 동일 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 구조 patch(추가·삭제·이동)를 실제 scene에 적용해 최종 tree를 만드는 것은 layout tree view(X.1.2)·scene 생성
  (XI.2.4)의 몫입니다. 이 작업은 적용할 patch 집합까지 결정합니다.
