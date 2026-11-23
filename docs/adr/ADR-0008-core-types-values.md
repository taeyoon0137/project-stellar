# ADR-0008: core-types 값·단위·기하·그래픽 값 타입

## Status

Accepted (소급, 2026-11-14). 이 ADR은 이미 구현된 결정을 소급 기록합니다. 아래 커밋은
보호 경로 `crates/core-types/`(I.3.5)를 변경했지만 같은 diff에 ADR이 없었습니다. 새 결정은
추가하지 않습니다.

## Context

- 기술 아키텍처 §3.1은 결정론을, §10.3은 색상 값과 색공간 메타데이터를 함께 저장할 것과
  sRGB·Display P3 초기 지원을 요구합니다. §12.3 Frame 등 문서 payload는 이 값 타입을 공유합니다.
- 이미 ADR이 있는 결정은 반복하지 않습니다.
  - transfer 곡선과 정확도 예산: ADR-0002(III.4.2 `98c8aa6`, 같은 커밋에 ADR 포함).
  - `OrderKey` 알고리즘·정규형·재균형·tie-break: ADR-0003(III.5.2 `a5ce295`에 포함). ADR보다 먼저
    또는 나중에 들어간 III.5.1 `bd09f02`(요구사항 fixture), III.5.3 `1fef7f3`(재균형), III.5.4
    `46f9595`(Semantic Merge fixture)는 ADR-0003의 결정 범위 안의 구현·검증입니다.
- ADR 없이 `crates/core-types/`를 변경한 값 타입 커밋은 다음과 같습니다.

| WBS | 커밋 | 내용 |
|---|---|---|
| III.3.1 | `58aeff4` | `FiniteF32` |
| III.3.2 | `7537c47` | 단위 newtype 5종 |
| III.3.3 | `576ee5e` | `Vec2`·`Size2`·`Rect`·`Insets`·`Transform2D` |
| III.3.4 | `8d0db18` | `UserText`·`NormalizedText`·`Normalizer` |
| III.3.5 | `46965b7` | `TypedValue` tagged union |
| III.4.1 | `3dc62b7` | `ColorSpace`·`Color`(비연관 알파) |
| III.4.3 | `0efd1c2` | `Gradient`·`GradientStop` |
| III.4.4 | `f2628d1` | `Shadow`·`BlurRadius`, `Vec2` 전순서 |
| VI.3.1 | `c9b3145` | `Transform2D::components` |
| VI.3.3 | `488cfc6` | `Transform2D::then`(합성) |
| VI.8.1 | `ec94b15` | `FiniteF32::ONE` 상수 |

## Decision

수치와 단위:

- `FiniteF32`는 NaN·±Infinity를 `NonFiniteError`로 거부하고 `-0.0`을 `+0.0`으로 정규화합니다.
  그래서 `Eq`·`Ord`(전순서)·`Hash`를 직접 구현합니다. 바이트 형식은 little-endian 4바이트이며 복원도
  `new`를 거칩니다(III.3.1). 상수 `ZERO`(III.3.1)와 `ONE`(VI.8.1)을 제공합니다.
- `Length`·`Percentage`·`Angle`·`Duration`·`Scalar`는 `FiniteF32`를 감싸는 별도 newtype입니다.
  `Add`·`Sub`는 같은 단위끼리만 구현하고, 오버플로는 유한 범위(`f32::MAX`/`MIN`)로 saturate합니다
  (III.3.2).
- **단위 계약(개정 2026-11-23, IX.11.1, ADR-0075 결정 2)**: 저장 값의 단위는 CSS 정규 단위입니다.
  `Length`=`px`(Stellar 논리 픽셀 = CSS px), `Angle`=`deg`, `Duration`=`ms`, `Percentage`=`%`(저장 50 =
  50%, 0..1 비율이 아님), `Scalar`=단위 없는 `<number>`입니다. 다른 단위 입력(`rad`, `grad`, `turn`, `s`)은
  parser가 이 단위로 정규화한 뒤 저장합니다. 지금까지 저장·표시한 값이 이미 이 단위라 저장 bytes의 뜻은
  바뀌지 않습니다. 길이·백분율 혼합(`<length-percentage>`)은 아직 값 타입이 아닙니다(속성 모델 15장 O3).

기하:

- 모든 성분은 `FiniteF32`입니다. `Size2`는 음수를 `SizeError::Negative`로 거부합니다(III.3.3).
- `Transform2D`는 아핀 성분 (a, b, c, d, tx, ty)입니다. `inverse`는 행렬식이 0이거나 결과가
  비유한이면 `None`입니다(III.3.3). `components()`는 성분 튜플을 반환합니다(VI.3.1).
- `then(next)`는 `self`를 먼저 적용하고 `next`를 적용하는 합성(`next ∘ self`)이며, 결과가 비유한이면
  `NonFiniteError`입니다(VI.3.3). 반전은 음수 size가 아니라 transform으로 표현합니다.
- `Vec2`는 `(x, y)` 사전식 전순서를 가집니다(III.4.4, `Shadow`·`TypedValue`의 `Ord` 파생을 위해).

문자열:

- 모든 문자열 값은 UTF-8입니다. `UserText`는 입력을 정규화하지 않고 보존합니다. `NormalizedText`는
  생성 시 주입된 `Normalizer`로 정규화합니다(식별자·파일 경로용). core-types는 NFC 테이블을 포함하지
  않으며 실제 NFC 정규화기는 상위 계층이 주입합니다(III.3.4).

`TypedValue`:

- variant 13종과 안정 type tag를 고정합니다: Boolean `0x01`, Number `0x02`, String `0x03`,
  Enum `0x04`, Length `0x05`, Percentage `0x06`, Angle `0x07`, Duration `0x08`, Color `0x09`,
  Gradient `0x0A`, Shadow `0x0B`, Typography `0x0C`, ResourceRef `0x0D`(III.3.5).
- 새 variant는 끝에 추가하고, 기존 tag는 재사용·변경하지 않습니다.
- `from_bytes`는 입력 전체를 정확히 소비해야 하며 미지 tag·절단·비유한 float·비 UTF-8·비정상
  Boolean·trailing bytes를 구조화 오류로 거부합니다.
- `Typography`의 내부 bytes는 현재 STTY v1 record입니다. 이 형식은 확정된 결정이 아닌 현행 구현 기록이며
  ADR-0067 §4(현행 구현 기록, 결정 아님)에 있습니다. 해석할 수 없는 기존 blob은 원래 bytes를 보존합니다. 기본값은 CSS initial
  record입니다(ADR-0024, ADR-0067).

그래픽 값:

- `ColorSpace` tag는 `Srgb=0x01`, `DisplayP3=0x02`입니다. `Color`는 색공간 + 비연관(straight) 알파
  RGBA이며, 알파만 `[0, 1]`로 강제하고 RGB는 확장 범위를 허용합니다. 바이트 형식은 고정 17바이트
  (tag 1 + 채널 4개 × LE 4바이트)입니다(III.4.1). transfer 규칙은 ADR-0002를 따릅니다.
- `GradientStop`은 `StableId` + position + `Color`의 고정 37바이트입니다. `Gradient`는 저장 순서
  (입력 순서)를 보존하고, 시각 순서는 `(position, StableId)`로 정렬해 분리합니다(III.4.3).
- `BlurRadius`는 음수·비유한을 거부하고 0을 허용합니다. `Shadow`는 offset(음수 허용) + blur +
  color의 고정 29바이트입니다(III.4.4).

정하지 않은 사항:

- core-types 값의 `to_bytes`는 float를 little-endian으로 쓰지만, `TypedValue`의 문자열·blob 길이,
  Enum 필드, `Gradient` stop 개수는 big-endian `u32`로 씁니다. 저장 canonical 규칙(V.1.1, ADR-0010)은
  little-endian입니다. 현재 `TypedValue`·`Gradient` 바이트열은 core-types 밖에서 사용되지 않습니다.
  이 형식을 디스크 표현으로 쓸지, 저장 시 canonical encoder로 다시 인코딩할지는 정하지 않았습니다.
- 색역(primary) 변환은 ADR-0002가 후속 작업으로 남긴 상태 그대로입니다.

## Alternatives

- 원시 `f32` 사용: NaN이 전순서·해시·결정론을 깨뜨립니다. 미선택.
- 음수 size로 반전 표현: 크기 불변식과 bounds 계산이 복잡해집니다. 반전은 transform으로 표현합니다
  (VI.3.3). 미선택.
- 연관(premultiplied) 알파 저장: 편집 시 색 정보 손실이 생깁니다. 합성 단계에서 변환합니다. 미선택.
- 단위를 하나의 수치 타입으로 통합: 단위 혼동을 컴파일 단계에서 막지 못합니다. 미선택.

## Consequences

- 모든 값 타입이 전순서·해시를 가져 canonical 비교와 결정론적 정렬에 쓸 수 있습니다.
- 비유한 값은 생성 시점에 거부되어 이후 계층(layout·renderer·storage)이 NaN을 고려하지 않아도
  됩니다. 같은 단위 산술의 오버플로는 saturate되어 오류 대신 경계값이 됩니다.
- 위 "정하지 않은 사항"의 엔디언 차이는 디스크 표현을 결정할 때 해소해야 합니다.

## Compatibility

- 파일 포맷: `TypedValue` tag, `ColorSpace` tag, `Color`·`GradientStop`·`Shadow` 고정 길이는 계약입니다.
  변경하려면 이 ADR을 supersede하고 migration을 정의합니다.
- 공개 API: `Transform2D::components`·`then`, `FiniteF32::ONE`은 추가만 했으며 기존 API를 바꾸지
  않았습니다.
- FFI: 이 ADR의 타입은 C ABI에 직접 노출되지 않습니다(ADR-0009).

## Performance

- 값 타입은 고정 크기 `Copy` 또는 소유 `Vec`·`String`이며 추가 간접 참조가 없습니다. 성능 예산
  파일은 변경하지 않습니다(색상 정확도 예산은 ADR-0002).

## Security

- 디코더는 미지 tag·절단·잉여 bytes를 거부하고, `Gradient`의 큰 stop 개수는 입력 길이로 검증해
  미리 할당하지 않습니다(III.4.3).

## Migration

- 이미 구현된 상태를 기록하므로 데이터 마이그레이션은 없습니다.
- 단위 계약 개정(2026-11-23)은 문서 주석과 이 ADR만 바꾸며 저장 bytes와 golden은 그대로입니다.
- `Typography`를 STTY v1 구조로 정할 때 tag `0x0C`와 외부 envelope는 유지했습니다(III.3.5, ADR-0024).
