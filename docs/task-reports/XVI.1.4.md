# 작업 보고서: XVI.1.4 trace dimension privacy

- WBS: XVI.1.4
- 작업명: interaction kind, document size bucket, visible node bucket, cache state만 dimension으로 허용
- 상태: 완료 (`[x]`)
- 결정: ADR-0084 결정 8
- 선행: II.3.2(`[x]`), XVI.1.3(이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증). 측정 장비는 개발 장비(Apple M3 Max, 36GB)이고 기준 장비(M1 16GB)가 아닙니다.

## 변경 파일 목록

- `crates/diagnostics/src/interaction.rs` — `TraceDimensions`, bucket, `TRACE_LINE_KEYS`.
- `docs/task-reports/XVI.1.4.md`(신규).

## 구현 요약

- dimension은 닫힌 enum 네 개입니다. 문자열 field가 없어 object 이름·text·path를 담을 수 없습니다(compile_fail doctest).
- bucket 경계는 아키텍처 §5.3 보장 규모(문서 200k, 가시 20k)를 기준으로 합니다.
- trace 줄은 허용 key만 쓰고, 값은 숫자이거나 고정 이름입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-diagnostics` | lib 25 passed, doc 3 passed(compile_fail 포함), 0 failed |
| 최종 검증(`check-core` 등) | lane 결과 보고에 기록 |

## 테스트

- `trace_line_uses_only_allowed_keys`: 모든 key가 허용 목록 안이고 값 문자 집합 제한, 고정 줄 golden.
- `buckets_follow_the_guaranteed_scale`.
- `TraceDimensions` compile_fail doctest(문자열 field 추가 불가).

## 성능

- 해당 없음.

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 없음.

## 2026-11-23 interaction-trace 후속 (ADR-0096 결정 3)

- `InteractionKind`를 아키텍처 §38.1 시나리오 13개와 1:1로 맞췄습니다. `token_edit`은 `token_propagation`, `undo_redo`는
  `undo`·`redo`로 나눴고 `component_update`·`variant_change`를 더했습니다. 단일 속성 변경 `property_edit`은 남깁니다.
- 값은 여전히 닫힌 enum의 고정 이름(소문자·밑줄)입니다. `kind_names_are_unique_fixed_names`가 이름이 서로 다르고 허용
  문자만 쓰는지 검사합니다. 기존 privacy schema test(`trace_line_uses_only_allowed_keys`, compile_fail doctest)도
  통과합니다.
- `InteractionTrace::gpu`(기록한 GPU 시각 읽기)를 더했습니다. 앱 연결 지점 probe가 병합된 event에 GPU 시간을 복사할 때
  씁니다(XVI.1.3).

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-diagnostics` | 통과(이 lane 최종 검증 절은 XVI.2.5 보고서) |

### 근거 커밋

- `d25a69c9` Feat: 상호작용 trace T0\~T5 ID·stage·구간·dimension 추가 (XVI.1.1\~XVI.1.4)
- 이 커밋(`Task(XVI.1.4)`): 시나리오와 1:1인 dimension 종류와 검사.
