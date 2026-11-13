# ADR-0011: 저장 트랜잭션·GC 보존·스키마 마이그레이션

## Status

Accepted (소급, 2026-11-14). 이 ADR은 이미 구현된 결정을 소급 기록합니다. 아래 커밋은
보호 경로 `crates/storage/`(I.3.5)를 변경했지만 같은 diff에 ADR이 없었습니다. 새 결정은
추가하지 않습니다.

## Context

- 기술 아키텍처 §21.2는 변경 chunk 기록 → 검증 → 새 generation manifest 기록 → 검증 → current
  pointer 원자적 교체 순서를 요구하고, 저장 중 종료되어도 이전 pointer가 유효해야 한다고 정합니다.
  §21.3은 보수적 mark-and-sweep을, §36은 결정론적·원자적 마이그레이션, 실패 시 원본 불변, 자동
  다운그레이드·자동 commit 금지를 요구합니다.
- 포맷 기준선(chunk, manifest, current pointer, atomic write)은 ADR-0010을 따릅니다.
- ADR 없이 `crates/storage/`를 변경한 커밋은 다음과 같습니다.

| WBS | 커밋 | 내용 |
|---|---|---|
| V.5.1 | `a353a26` | `SavePlan` |
| V.5.2 | `57e5557` | optimistic base check |
| V.5.3 | `32bc7d2` | 저장 단계 실행기 |
| V.5.4 | `8eb8d30` | pointer 전환 commit |
| V.5.5 | `a98dd50` | 저장 취소 token |
| V.5.6 | `3ac8327` | save fault matrix |
| V.7.1 | `5741858` | 보존 generation 정책(`config/storage-gc.yaml`) |
| V.7.2 | `27ff67b` | reachable mark scanner |
| V.7.3 | `6dfbd45` | grace period sweep |
| V.7.4 | `25d736d` | GC scheduler interface |
| V.8.1 | `6973a2a` | 버전 호환성 판정 |
| V.8.2 | `2dbe57d` | migration step registry |
| V.8.3 | `361ccfe` | noop v1→v2 migration |
| V.8.4 | `f023de4` | temp generation migration 검증 |
| V.8.5 | `89cb0e5` | migration Git working tree fixture |

## Decision

저장 트랜잭션(V.5):

- `SavePlan`은 base generation 번호·manifest hash, target generation(base + 1, 첫 저장은 1), dirty
  chunk 목록, 검증을 통과한 새 manifest를 담는 순수 계산 결과이며 디스크를 건드리지 않습니다(V.5.1).
- 첫 단계는 optimistic base check입니다. current pointer가 계획의 base hash와 정확히 같아야 하고,
  첫 저장은 current pointer 파일이 없어야 합니다. 불일치는 `StaleBase`이며 아무것도 기록하지
  않습니다(V.5.2).
- 단계 순서는 base check → dirty chunk 기록 → 기록한 chunk 재읽기 전체 검증 → manifest 기록 →
  pointer 전환으로 고정합니다. 전환 전 산출물은 새 파일 추가뿐이므로 어느 단계에서 실패해도 current
  generation이 그대로 열립니다(V.5.3).
- `commit_save`가 저장 파이프라인에서 current pointer를 바꾸는 유일한 단계입니다. in-memory
  `SavedGenerationState`는 pointer 전환이 성공한 뒤에만 갱신합니다(V.5.4).
- `CancelToken`은 prepare 각 단계 직전과 pointer 전환 직전에 확인합니다. `commit_save` 진입 후에는
  확인하지 않아 "절반 전환"을 만들지 않습니다(V.5.5).
- fault matrix는 22개 주입 지점(prepare 4 + chunk·manifest·pointer 각각의 atomic write 6단계)을
  고정합니다. pointer rename 이후 실패(부모 디렉터리 sync)만 새 세대로 열리고, 나머지 21개 지점은
  base 세대를 유지하며, 실패한 저장은 in-memory 상태를 바꾸지 않아야 합니다(V.5.6).

GC 보존과 정리(V.7):

- 보존 수치는 `config/storage-gc.yaml`(한 줄 `key: value`)에서 읽습니다. 두 key 모두 필수이며
  누락·미지 key·음이 아닌 정수가 아닌 값을 거부합니다. `keep_recent_generations`는 1 이상이어야
  하고(0 거부), `grace_period_seconds`는 0을 포함한 음이 아닌 정수를 모두 받습니다. 현재 값은
  `keep_recent_generations: 2`(current + 직전 정상 세대), `grace_period_seconds: 86400`입니다
  (V.7.1).
- 보존 세대는 current 세대(손상이어도 진단 자료로 보존), 정상 세대 중 최신
  `keep_recent_generations`개, fallback 복구에 선택될 세대(V.4.5)의 합집합입니다(V.7.1).
- mark는 보존 manifest 자신의 hash와 그 manifest들이 가리키는 모든 chunk hash의 합집합입니다. 보존
  manifest를 읽지 못하면 오류이고, sweep은 mark가 성공해야만 진행합니다(V.7.2).
- sweep 대상은 `design/objects/` 아래 chunk 파일과 `design/generations/` 아래 manifest 파일 중
  content-addressed 이름을 가진 파일입니다. unmark여도 mtime 기준 grace period 미만이면 보존합니다.
  삭제·mtime 읽기 실패는 오류가 아니라 이월 목록에 남기고 다음 실행에서 재시도합니다. 현재 시각은
  인자로 주입합니다(V.7.3).
- GC는 활성 저장 guard 0, GC 미실행, 마지막 활동 이후 idle threshold 경과를 모두 만족할 때만
  시작합니다. 생성 시각을 최초 활동으로 보므로 open 직후에는 시작하지 않습니다. 저장은 RAII guard를
  잡습니다(V.7.4).

스키마 버전과 마이그레이션(V.8):

- `FormatVersions`는 `format_version`, `schema_version`, `minimum_reader_version`,
  `minimum_writer_version`(§36)이며 `minimum_reader ≤ minimum_writer ≤ schema` 선언 불변식을
  검증합니다(V.8.1).
- format version은 정확히 일치해야 합니다. reader 스키마 버전 `A`가 `A ≥ minimum_writer`이면 편집
  가능, `minimum_reader ≤ A < minimum_writer`이면 읽기 전용, `A < minimum_reader`이면 거부입니다
  (V.8.1).
- migration step은 `from → to`이며 역방향·제자리 step을 거부하고, 출발 버전당 step은 하나입니다.
  경로가 끊기거나 목표를 지나치면 `NoPath`, 재진입하면 `Cycle`입니다. step이 실패하면 그 지점에서
  멈추고 버전을 갱신하지 않습니다(V.8.2).
- 실행은 current 세대를 읽기 전용으로 로드 → in-memory 변환 → V.5 저장 파이프라인으로 새 generation
  기록입니다. 원본 파일은 수정하지 않습니다. 모든 검증과 manifest 기록이 끝난 뒤에만 pointer를
  전환하므로, 그 전까지 migration 결과는 어떤 reader에게도 보이지 않습니다(V.8.3, V.8.4).
- Git commit을 만들지 않습니다. noop v1→v2 migration 뒤 Git 상태 변화는 `design/current` 수정과 새
  manifest 파일 추가 2건뿐이고 HEAD는 바뀌지 않습니다(V.8.5).
- storage crate는 noop v1→v2를 테스트용 migration step으로 제공합니다(V.8.3). 제품 registry
  (`product_registry()`)와 최신 storage 문서 스키마 버전 `LATEST_SCHEMA = 2`는 storage가 아니라
  `tools/xtask/src/migration_fixtures.rs`에 정의되어 있고(V.8.6), 제품 registry에 등록된 step은
  payload를 바꾸지 않는 이 noop v1→v2뿐입니다.
- 여기서 말하는 스키마 버전은 generation manifest의 `schema_version`(storage 문서 스키마)입니다.
  프로젝트 manifest `stellar.yaml`의 `CURRENT_SCHEMA_VERSION`(현재 1, project crate)과는 별개의
  버전입니다.

정하지 않은 사항:

- 현재 manifest는 `schema_version`만 기록합니다(ADR-0010). `minimum_reader_version`·
  `minimum_writer_version`을 디스크 어디에 기록할지는 정하지 않았습니다. V.8.1은 판정 타입과 규칙만
  구현했습니다.
- GC idle threshold는 호출자가 주입하는 값이며 기본값은 정하지 않았습니다.
- §21.3의 Git index·HEAD가 가리키는 generation, 복구 스냅샷, 열린 문서 Snapshot을 보존 입력으로
  연결하는 일은 V.7 범위 밖이며 정하지 않았습니다.
- 실제 스키마 변경을 담은 migration step은 아직 없습니다.

## Alternatives

- pointer를 먼저 바꾸고 나중에 chunk 기록: 중간 종료 시 깨진 세대가 current가 됩니다(§21.2). 미선택.
- 전환 중 취소 허용: 원자적 교체 하나뿐인 단계를 끊으면 상태 판정이 모호해집니다(V.5.5). 미선택.
- 즉시 삭제(grace period 없음): 진행 중 저장이 만든 아직 참조되지 않은 파일을 지울 수 있습니다. 미선택.
- 보존 수치를 코드 상수로 고정: 정책 조정 시 코드 변경이 필요합니다. config 파일로 분리했습니다. 미선택.
- 제자리(in-place) migration: 실패 시 원본 불변(§36)을 보장하지 못합니다. 미선택.

## Consequences

- 저장·migration 실패는 current 세대를 바꾸지 않으며 fault matrix가 22개 지점에서 이를 고정합니다.
- GC는 보존 manifest를 읽지 못하면 멈추므로 과잉 삭제 대신 정리 지연을 택합니다. 삭제 실패는 다음
  실행으로 이월됩니다.
- migration 결과는 사용자가 검토·commit할 working tree 변경으로 남습니다.

## Compatibility

- 파일 포맷: 새 포맷 요소는 없습니다(ADR-0010의 포맷만 사용). `config/storage-gc.yaml`의 key 이름은
  storage가 읽는 계약입니다.
- 스키마: 현재 최신 storage 문서 스키마 버전은 2입니다(`LATEST_SCHEMA`, V.8.6, noop step). 프로젝트
  manifest 스키마(`CURRENT_SCHEMA_VERSION = 1`)와는 별개이며, 버전 판정 규칙은 위와 같습니다.
- 공개 API·FFI: 영향 없음.

## Performance

- 저장 비용은 dirty chunk 수에 비례합니다(clean·no-op chunk는 기록하지 않음, ADR-0010). 각 파일마다
  `sync_all`이 있습니다.
- GC는 idle 상태에서만 시작해 편집·저장 경로와 겹치지 않습니다. 성능 예산 파일은 변경하지 않습니다.

## Security

- 열기·migration 로드는 읽기 전용이며, 손상 세대를 자동 수정하지 않습니다.
- sweep은 content-addressed 이름이 아닌 파일을 건드리지 않습니다.

## Migration

- 이미 구현된 상태를 기록하므로 데이터 마이그레이션은 없습니다.
- 새 스키마 버전은 migration step 등록과 golden fixture(V.8.6) 기대값 갱신으로 도입하며, 보호 경로
  정책(I.3.5)에 따라 같은 diff에 ADR을 둡니다.
