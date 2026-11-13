# ADR-0007: core-types 식별자·참조·공통 기반 타입

## Status

Accepted (소급, 2026-11-14). 이 ADR은 이미 구현된 결정을 소급 기록합니다. 아래 커밋은
보호 경로 `crates/core-types/`(I.3.5)를 변경했지만 같은 diff에 ADR이 없었습니다. 새 결정은
추가하지 않습니다.

## Context

- 기술 아키텍처 §12.1은 모든 영속 객체에 128비트 UUIDv7 `StableId`를 요구하고, 런타임에는
  별도의 밀집 `EntityId`를 허용합니다. §12.8은 StableId ↔ EntityId 양방향 인덱스와 Arena 기반
  저장을 요구합니다. §3.1은 결정론을 요구합니다.
- `crates/core-types/`는 공개 API와 numeric registry를 담는 보호 경로입니다. 보호 경로 정책(I.3.5)
  도입 후 다음 커밋이 ADR 없이 이 경로를 변경했습니다.

| WBS | 커밋 | 내용 |
|---|---|---|
| II.2.1 | `a6eae75` | core-types 골격(공개 모듈 선언만, 외부 의존성 없음) |
| II.3.1 | `64c7ee4` | `ErrorCode`·`StellarError` |
| II.3.3 | `b9812b3` | `MonotonicClock`·`ManualClock` |
| II.3.4 | `92d1ffb` | revision newtype 3종 |
| III.1.1 | `4aea091` | `StableId`(UUIDv7) |
| III.1.2 | `06db4e4` | 결정적 `StableIdGenerator` |
| III.1.3 | `3f9c532` | `RuntimeEntityId`(generational) |
| III.1.4 | `8ead9f6` | `IdRegistry`(StableId ↔ RuntimeEntityId) |
| III.1.5 | `1c85096` | 복제용 `IdRemapTable` |
| III.2.1 | `754d726` | `PropertyKey`·이름 registry |
| III.2.2 | `fe36d01` | `PropertyPath` |
| III.2.3 | `a07b064` | 강한 타입 참조 4종 |
| III.2.4 | `e04f9b4` | `UnresolvedRef` |
| III.6.1 | `3994790` | `RuntimeEntityAllocator::live_slot` 추가 |
| V.9.1 | `5d7f08f` | `WindowSessionId` 추가 |

## Decision

공통 원칙:

- `stellar-core-types`는 외부 의존성 없이 표준 라이브러리만 사용합니다(II.2.1). 제품 crate 의존
  그래프에서 core-types는 어떤 제품 crate에도 의존하지 않습니다(II.2.8).
- 서로 다른 의미의 식별자는 별도 newtype으로 분리해 혼용을 컴파일 단계에서 거부합니다. 혼용
  거부는 rustdoc `compile_fail` doctest로 고정합니다(II.3.4, III.1.3, III.2.3).

오류·시간·revision(Phase II.3):

- `ErrorCode`는 `#[repr(u32)]`이며 안정 숫자 `Internal=1`, `InvalidInput=2`, `NotFound=3`,
  `AlreadyExists=4`, `PermissionDenied=5`, `InternalPanic=6`을 가집니다. 숫자는 재사용·변경하지
  않습니다. `StellarError`는 사용자 메시지와 개발자 context를 분리합니다(II.3.1).
- 성능·히스토리 순서는 wall clock이 아니라 `MonotonicClock::now_nanos()` 추상화를 사용합니다.
  테스트는 `ManualClock`으로 결정론을 유지합니다(II.3.3).
- `DocumentRevision`·`SceneRevision`·`StorageGeneration`은 각각 `u64`를 감싸는 별도 타입입니다
  (II.3.4).

영속 식별자(Phase III.1):

- `StableId`는 16바이트 UUIDv7(RFC 9562) 레이아웃입니다. 문자열은 소문자 8-4-4-4-12 형식으로
  출력하고, 파싱은 대소문자를 모두 허용합니다. 바이트 순서 `Ord`가 시간 순서와 같습니다(III.1.1).
- `StableId::new_v7(unix_ts_ms, rand_a, rand_b)`는 입력 기반 생성자입니다. core-types는 시스템
  시계·엔트로피를 직접 읽지 않습니다(III.1.1).
- `StableIdGenerator`는 주입된 `MonotonicClock`에서 timestamp(ms)를, seed 기반 내부 SplitMix64에서
  random 필드를 가져옵니다. 같은 seed와 같은 시각 수열이면 같은 ID 수열을 만듭니다(III.1.2).
- `IdRemapTable`은 복제 그래프의 old → new `StableId` 매핑을 보관합니다. 테이블에 없는 id(외부
  참조)는 원본을 유지합니다(III.1.5, §12.1 "복제 그래프 전체에 새 ID 발급").
- `WindowSessionId`는 창 편집 세션을 식별하는 `StableId` 기반 newtype입니다. 복구 스냅샷
  키(V.9)와 창별 undo/redo(Phase VII)가 사용합니다(V.9.1).

런타임 식별자(Phase III.1, III.6):

- `RuntimeEntityId { slot: u32, generation: u32 }`는 런타임 전용 핸들입니다. 필드는 `pub(crate)`이며
  직렬화하지 않고 FFI에 노출하지 않습니다(III.1.3).
- `RuntimeEntityAllocator`는 해제 시 generation을 올려 옛 핸들을 무효화하고 slot을 재사용합니다
  (III.1.3).
- `live_slot(id) -> Option<usize>`는 generation 일치만 검사하는 O(1) 접근자입니다. 반환 slot은
  in-process 저장 인덱싱 전용이며 generation은 노출하지 않습니다. `RuntimeEntityId`가 유일한
  런타임 핸들입니다(III.6.1).
- `IdRegistry`는 양방향 lookup을 유지합니다. 중복 `StableId`와 중복 `RuntimeEntityId` 삽입을 모두
  오류로 거부하고, 실패 시 registry를 바꾸지 않습니다(III.1.4).

속성 경로와 참조(Phase III.2):

- `PropertyKey(u32)`는 문자열이 아닌 고정 numeric 식별자입니다. 이름 변경은 키를 유지합니다
  (III.2.1).
- `PropertyPath { root: StableId, keys: Vec<PropertyKey>, tail: Option<PathSegment> }`는
  `to_canonical_bytes()`로 모호성 없는 바이트열을 만듭니다(III.2.2).
- `NodeRef`·`TokenRef`·`ComponentRef`·`AssetRef`는 서로 변환되지 않는 별도 newtype입니다(III.2.3).
- `UnresolvedRef`는 원래 id, 기대 참조 종류(`ExpectedRefType`), 발견 위치(`PropertyPath`)를 보존합니다
  (III.2.4).

정하지 않은 사항:

- `PropertyPath::to_canonical_bytes()`는 길이·키에 big-endian `u32`를 씁니다. 저장 canonical
  규칙(V.1.1, ADR-0010)은 little-endian입니다. 현재 이 바이트열은 core-types 밖에서 사용되지
  않습니다. 디스크 표현으로 쓸지, 저장 시 canonical encoder로 다시 인코딩할지는 정하지 않았습니다.

## Alternatives

- 식별자를 원시 `[u8; 16]`·`u64`로 공유: 혼용 오류를 런타임에서만 발견합니다. 미선택.
- 외부 UUID·난수 crate 사용: core-types 의존성 0 정책(II.2.1)과 결정적 생성기 주입(III.1.2)에
  맞지 않습니다. 미선택.
- 런타임 핸들로 `Box`·포인터 사용: §12.8의 Arena 배치와 stale 핸들 검출(generation)을 제공하지
  못합니다. 미선택.

## Consequences

- 식별자 혼용은 컴파일 오류가 되고, compile_fail doctest가 이 계약을 회귀로부터 보호합니다.
- 같은 seed와 시각 입력이면 ID 수열이 재현되어 fixture를 결정론적으로 만들 수 있습니다.
- core-types는 시스템 시계·엔트로피를 읽지 않으므로, 실제 ID 생성 시 시각과 seed는 상위 계층이
  주입해야 합니다.

## Compatibility

- 파일 포맷: `StableId` 16바이트 레이아웃과 문자열 형식, `ErrorCode` 숫자, `PropertyKey` 숫자는
  계약입니다. 변경하려면 이 ADR을 supersede합니다.
- 공개 API: `RuntimeEntityId` 내부 필드는 공개 API가 아닙니다. 직렬화·FFI에 나타나지 않습니다.
- FFI: 이 ADR의 타입은 C ABI에 직접 노출되지 않습니다(ADR-0009).

## Performance

- `live_slot`과 registry lookup은 O(1)입니다. 성능 예산 파일(`config/performance-budgets.yaml`)은
  변경하지 않습니다.

## Security

- `StableIdGenerator`의 SplitMix64는 결정성을 위한 것이며 암호학적 난수가 아닙니다. 식별자는
  비밀값이 아니며 권한 판단에 쓰지 않습니다.
- `StableId` 파싱은 잘못된 길이·문자를 구조화 오류로 거부합니다.

## Migration

- 이미 구현된 상태를 기록하므로 데이터 마이그레이션은 없습니다.
- 위 숫자·레이아웃 계약을 바꾸려면 새 ADR과 schema migration(ADR-0011)을 함께 정의합니다.
