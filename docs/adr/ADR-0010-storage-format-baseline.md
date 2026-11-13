# ADR-0010: 저장 포맷 기준선(canonical 인코딩·chunk·generation manifest)

## Status

Accepted (소급, 2026-11-14). 이 ADR은 이미 구현된 결정을 소급 기록합니다. 아래 커밋은
보호 경로 `crates/storage/`(I.3.5)를 변경했지만 같은 diff에 ADR이 없었습니다. 새 결정은
추가하지 않습니다.

## Context

- 기술 아키텍처 §21.1은 논리 식별(불변 `ChunkId`)과 물리 저장(`ContentHash`)을 분리하고, manifest가
  `ChunkId → ContentHash`를 매핑하는 혼합 모델을 선택했습니다(§21.4). §21.2는 세대 기반 트랜잭션
  저장을, §36은 버전 기록과 호환성 정책을 요구합니다.
- content hash 알고리즘과 문자열 형식은 ADR-0004(V.1.5 `16b86fd`)가 정했습니다. 이 ADR은 반복하지
  않고 `sha256:<hex64>` 형식을 그대로 사용합니다.
- canonical primitive 규칙 R1~R6과 golden fixture `docs/fixtures/storage/canonical-primitives.yaml`은
  V.1.1(`ba3b980`, 보호 경로 변경 없음)이 정의했습니다.
- ADR 없이 `crates/storage/`를 변경한 포맷 관련 커밋은 다음과 같습니다. 저장 트랜잭션·GC·마이그레이션
  커밋은 ADR-0011이 다룹니다.

| WBS | 커밋 | 내용 |
|---|---|---|
| II.2.5 | `45b4666` | storage crate 골격(core-types만 직접 의존) |
| V.1.2 | `9ec19ed` | canonical encoder |
| V.1.3 | `cade17a` | canonical decoder, tagged field 형식 |
| V.1.4 | `0cda7d5` | 저장 float 유한성 정책 |
| V.2.1 | `a001acf` | `ChunkId` |
| V.2.2 | `3c1a3c9` | chunk envelope header |
| V.2.3 | `78f587b` | chunk object 경로 |
| V.2.4 | `ec7e6c9` | immutable chunk write |
| V.2.5 | `1f27059` | atomic 파일 쓰기 |
| V.2.6 | `3268f49` | chunk read 전체 검증 |
| V.2.7 | `54c336f` | chunk kind registry와 unknown 정책 |
| V.3.1 | `fd58bef` | `ChunkPointer` |
| V.3.2 | `ad8ef4b` | 중복 ChunkId pointer validator |
| V.3.3 | `27ab661` | chunk write planner |
| V.3.4 | `4abd543` | chunk remap manifest |
| V.4.1 | `47b43da` | generation manifest |
| V.4.2 | `405d21b` | manifest content hash |
| V.4.3 | `5f8b789` | current pointer 파일 |
| V.4.4 | `2c32bff` | 프로젝트 열기 loader |
| V.4.5 | `707f4d1` | 정상 세대 fallback scanner |
| V.4.6 | `5020eb0` | 읽기 전용 recovery open |
| V.6.1 | `ada4951` | 초기 chunk kind 6종 |
| V.6.2 | `e3d8560` | ViewTree subtree 참조 스키마 |
| V.6.6 | `4095eaa` | 지연 로딩 loader |

## Decision

canonical 인코딩(V.1):

- R1 little-endian, R2 고정 정수 폭(bool 1바이트), R3 `u32` LE 길이 프리픽스 UTF-8·바이트열,
  R4 정렬된 map key, R5 유한 float bits, R6 unknown field 정책을 따릅니다. 외부 의존성 없이
  구현합니다(V.1.2).
- R1의 예외: `StableId`·`ChunkId`의 16바이트 raw 필드(manifest의 project id·root document id,
  `ChunkPointer`의 chunk_id, ViewTree의 node `StableId`·subtree `ChunkId`)는 정수로 다루지 않고
  UUID 바이트 순서(big-endian) 그대로 기록합니다(V.3.1, V.4.1, V.6.2).
- map은 `u32` LE entry 수 + key 바이트 순서로 정렬된 entry입니다. 입력 삽입 순서는 결과에 영향이
  없고 중복 key는 거부합니다. decoder는 key가 엄격히 증가해야 하며 중복·역순을 거부합니다
  (V.1.2, V.1.3).
- tagged field 형식은 `u32` LE tag + `u8` flags(bit0 = mandatory) + 길이 프리픽스 payload이고,
  레코드는 `u32` count + tag 오름차순입니다. 모르는 mandatory tag는 거부하고 모르는 optional tag는
  건너뜁니다. 정의되지 않은 flags bit는 거부합니다(V.1.3).
- decoder는 panic 없이 오류를 반환하며 길이 overflow·절단·bool 0/1 외 값·비 UTF-8·trailing bytes를
  거부합니다(V.1.3).
- `f32`는 core `FiniteF32`(ADR-0008) 규칙을 그대로 쓰고, `f64`는 같은 규칙을 storage에서
  적용합니다. NaN·±Infinity는 encode에서 거부하고 `-0.0`은 `+0.0`으로 정규화합니다. decoder는 NaN·
  ±Infinity·`-0.0` bit 패턴을 자동 수정 없이 거부합니다(V.1.4).

chunk(V.2, V.6.1):

- `ChunkId`는 `StableId`를 감싸는 newtype입니다. 내용이 바뀌어도 유지되는 논리 식별자이며 원시
  `StableId`와 혼용할 수 없습니다(V.2.1, compile_fail doctest).
- envelope header(format version 1)는 8바이트 magic `STARCHNK`(`CHUNK_MAGIC`) + format version
  `u32` LE + kind `u32` LE + 비압축 길이 `u64` LE + payload hash(ADR-0004 문자열, 길이 프리픽스)
  입니다. 압축 필드는 없으며 압축을 도입하면 format version을 올립니다(V.2.2).
- object 경로는 `design/objects/<prefix>/<algorithm>-<hex64>.starchunk`입니다(확장자 상수
  `CHUNK_EXTENSION`). prefix는 digest hex 앞 2자입니다. 경로는 `ContentHash`에서만 생성되어 임의
  문자열 경로가 없습니다. 파일명은 `:` 대신 `-`를 씁니다(`:`는 Windows 파일명 금지 문자이며, 코어는
  ADR-0006에 따라 플랫폼 중립을 유지합니다)(V.2.3).
- chunk 파일은 불변입니다. 대상이 없으면 기록하고, 전체 bytes가 같으면 재사용하며, 다르면
  `CorruptExisting`으로 거부하고 덮어쓰지 않습니다(V.2.4).
- 파일 쓰기는 같은 디렉터리 temp 생성 → 기록 → flush → `sync_all` → atomic rename → 부모 디렉터리
  sync(POSIX만) 순서로 고정합니다. rename이 대상을 바꾸는 유일한 단계입니다. 각 단계 직전에 fault
  injection observer를 둡니다(V.2.5).
- chunk 읽기는 envelope → 기대 kind 일치 → payload 길이 일치 → envelope hash와 요청 주소 일치 →
  payload 재해시 일치 순서로 전부 검증하고 파일을 수정하지 않습니다. payload hash가 header를
  덮지 않으므로 호출자가 기대 kind를 넘깁니다(V.2.6).
- kind code의 최상위 bit(bit 31, `0x8000_0000`)가 required 여부입니다. reader는 모르는 kind라도
  code만으로 optional이면 건너뛰고 required이면 오류를 냅니다(V.2.7).
- 초기 kind는 ProjectMetadata `0x80000001`, DocumentManifest `0x80000002`, ViewTree `0x80000003`,
  ComponentLibrary `0x80000004`, TokenSet `0x80000005`(required), AssetMetadata `0x00000006`
  (optional)입니다(V.6.1).
- ViewTree payload는 view root node `StableId` + subtree count(`u32` LE) + (subtree root node
  `StableId`, subtree `ChunkId`) 목록입니다. 노드 내용은 subtree chunk가 담습니다. schema version은
  payload가 아니라 `ChunkPointer`가 보관합니다(V.6.2).

ChunkId ↔ ContentHash 매핑(V.3):

- `ChunkPointer`는 chunk_id 16바이트 + content hash 문자열(길이 프리픽스) + kind `u32` LE + schema
  version `u32` LE입니다. 정렬은 (chunk_id, content_hash, kind, schema_version) 사전식입니다(V.3.1).
- 한 manifest에서 ChunkId는 정확히 한 번 나타납니다. 다른 hash를 가리키는 중복과 완전 중복을 모두
  거부하고, 서로 다른 ChunkId가 같은 hash를 공유하는 것(dedup)은 허용합니다(V.3.2).
- write planner는 `Clean`(base pointer 재사용)·`Dirty`(새 payload)를 받습니다. hash가 base와 같은
  Dirty는 기록하지 않습니다. 결과 pointer는 chunk_id 오름차순입니다(V.3.3).
- `ChunkRemapManifest`는 `from → to` 재매핑입니다. 자기 재매핑·중복 source를 거부하고, chain의 최종
  대상이 cycle 없이 살아 있는 ChunkId여야 합니다(V.3.4).

generation manifest와 열기(V.4, V.6.6):

- manifest(format version 1)는 magic `STARMFST` + format version + generation `u64`(1부터) + parent
  flag와 parent `u64` + project id 16바이트 + schema version `u32` + chunk pointer 목록(chunk_id
  오름차순 강제) + root document id 목록(순서 보존, 중복 거부)입니다. parent는 generation보다
  작아야 합니다. encode·decode 양쪽에서 검증하며 재인코딩은 byte-identical합니다(V.4.1).
- manifest hash는 canonical encode 결과 전체 bytes의 ADR-0004 hash입니다(V.4.2).
- manifest 파일은 `design/generations/<algorithm>-<hex64>.starmanifest`이며 chunk와 같은 불변성
  규칙을 따릅니다(V.4.4).
- current pointer는 `design/current`에 manifest hash 문자열 + LF(총 72바이트) 하나만 담습니다.
  atomic write로만 교체합니다(V.4.3).
- 열기는 current pointer → manifest hash 일치·구조 검증 → project id 일치 → 모든 chunk 파일 존재
  확인 순서입니다. chunk 내용은 읽기 시점에 검증합니다(V.4.4).
- current 로드가 실패하면 `design/generations/`를 hash 사전식으로 순회해 완전 검증을 통과한 세대
  중 generation 최대를 고르고 `ReadOnlyRecovery`로 엽니다. 열기는 어떤 경로에서도 디스크에 쓰지
  않으며 current pointer를 바꾸지 않습니다(V.4.5, V.4.6).
- 지연 로딩은 열기에서 DocumentManifest kind payload만 전체 검증으로 읽고, 나머지는 요청 시
  읽습니다. 모든 payload 읽기는 계측 가능한 단일 경로를 지납니다(V.6.6).

정하지 않은 사항:

- chunk 분할 임계치(V.6.4)와 작은 chunk 병합 기준(V.6.5)은 정하지 않았습니다. 분할 임계치 제안은
  ADR-0012(Proposed)에 있으며 제품 소유자 승인 전입니다.
- §21.2는 current pointer를 `current.stellarref`라고 부르지만 구현(V.4.3)은 `design/current`를
  씁니다. 이 ADR은 구현 상태를 기록할 뿐 둘 중 어느 쪽으로 맞출지 정하지 않습니다.
- ViewTree 외 kind의 payload 스키마와 압축 도입 여부는 정하지 않았습니다.

## Alternatives

- 기술 아키텍처 §21.4가 단일 바이너리, 객체별 파일, 안정 경로 가변 chunk, 순수 ContentHash chunk,
  단일 데이터베이스를 비교해 탈락시켰습니다. 이 ADR은 그 판단을 반복하지 않습니다.
- registry 기반 requiredness: 구버전 reader가 신규 kind의 처리 방식을 알 수 없습니다. code bit
  방식을 선택했습니다(V.2.7). 미선택.
- 파일명에 `:` 사용: Windows 파일명에서 금지됩니다. 미선택.
- 열기에서 전체 payload 검증: 지연 로딩(§21.1)과 충돌합니다. 미선택.

## Consequences

- 같은 문서 모델은 플랫폼과 무관하게 같은 bytes와 같은 hash를 만듭니다.
- 저장 산출물은 content-addressed 새 파일 추가이며, 기존 파일은 덮어쓰지 않습니다. 손상된 기존
  파일은 자동 수정하지 않고 오류로 드러납니다.
- 열기와 fallback은 읽기 전용이므로, 복구 확정은 사용자 결정과 저장 트랜잭션(ADR-0011)에 맡깁니다.
- 지원하지 않는 format version은 거부되므로, 포맷 변경 시 reader 갱신이 먼저 필요합니다.

## Compatibility

- 파일 포맷: envelope·manifest format version 1, kind code, 경로 형식, current pointer 형식, ViewTree
  payload 형식은 이 ADR로 고정됩니다. 변경하려면 format version을 올리고 새 ADR을 둡니다.
- optional kind는 구버전 reader가 건너뛰므로 추가가 호환됩니다. required kind 추가는 구버전 reader가
  거부합니다.
- 공개 API·FFI: 영향 없음.

## Performance

- 열기 비용은 manifest 크기와 chunk 파일 존재 확인에 비례하며 payload 전체 읽기를 포함하지 않습니다.
- 내부 SHA-256(ADR-0004)과 파일별 `sync_all`이 저장 시간에 포함됩니다. chunk 크기 측정(V.6.3)은
  ADR-0012의 입력입니다. 성능 예산 파일은 변경하지 않습니다.

## Security

- 경로는 `ContentHash` 타입에서만 생성되어 path traversal이 타입상 불가능합니다.
- 모든 decoder는 길이 overflow·절단·잉여 bytes를 거부하고 panic하지 않습니다. chunk 읽기는 길이와
  hash를 검증합니다(§37.2 "청크 길이·참조·해시 검증"). §37.2의 깊이·개수 제한과 fuzzing은 이
  ADR의 범위가 아닙니다.
- hash는 무결성 검증용이며 비밀값이 아닙니다.

## Migration

- 이미 구현된 상태를 기록하므로 데이터 마이그레이션은 없습니다.
- 스키마 변경은 ADR-0011의 migration 경로로만 수행합니다. format version 변경은 새 ADR과 별도
  WBS가 필요합니다.
