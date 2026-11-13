# ADR-0012: chunk 분할 임계치(V.6.4) 제안

## Status

Proposed (2026-11-14). 제품 소유자 승인 대기 중입니다. 이 ADR은 V.6.3 측정 자료를 바탕으로 한
권고이며 아직 결정이 아닙니다. 승인 전에는 V.6.4를 착수하지 않고 `config/performance-budgets.yaml`도
바꾸지 않습니다.

## Context

- 기술 아키텍처 §21.1은 Frame·Section·Component 등 의미 경계로 chunk를 나누고, "정확한 목표 청크 크기와
  분할 임계치는 실제 문서 벤치마크 후 결정"한다고 정합니다. 청크 분할·병합은 문서 의미 변경이 아니라
  저장 최적화입니다.
- 구현 계획 V.6.4는 승인된 임계치를 `config/performance-budgets.yaml`에 기록하고 split planner를
  구현하라고 정하며, 선행 조건은 V.6.3과 ADR 승인입니다.
- V.6.3 harness(`cargo xtask bench-chunk-size`)의 결과가 `docs/status/chunk-size-benchmark.md`에
  있습니다. 이 보고서는 결론을 담지 않습니다.

측정 조건(보고서와 harness 코드 기준):

- 데이터셋: 결정적 pseudo-random 8MiB(비압축성 합성 데이터). 의미 경계가 없는 고정 크기 분할입니다.
- 환경: `DEV-MAC` 로컬 1회 실행입니다.
- 저장(ms): 데이터셋 전체를 첫 generation으로 저장하는 V.5 `run_save` 시간입니다. 단일 수정 후 재저장
  시간은 따로 재지 않았습니다.
- 로드(ms): open과 모든 chunk 전체 검증 읽기 시간입니다.
- Git delta: 중앙 chunk 16바이트 수정 후 두 번째 commit이 `.git/objects`에 더한 크기입니다(pack 전
  loose object 기준).

| 후보 | chunk 수 | 저장(ms) | 로드(ms) | 재기록 bytes | Git delta bytes |
|---|---|---|---|---|---|
| 64KiB | 128 | 2434.7 | 339.7 | 65536 | 74679 |
| 256KiB | 32 | 1324.1 | 336.0 | 262144 | 265343 |
| 1MiB | 8 | 1103.2 | 333.2 | 1048576 | 1050389 |

수치 관찰:

- 로드 시간은 세 후보가 거의 같습니다(333.2~339.7ms).
- 전체 저장 시간은 64KiB가 1MiB의 약 2.2배이고, 256KiB는 약 1.2배입니다. chunk 수(파일 수·fsync
  횟수)가 늘수록 커집니다.
- 단일 수정의 재기록량과 Git 증가량은 chunk 크기에 거의 비례합니다. 256KiB는 64KiB의 약 3.6배, 1MiB의
  약 1/4입니다.

## Decision

제안(미승인):

- chunk 분할 임계치를 **256KiB(262,144 bytes)**로 둡니다. 기준 값은 envelope의 비압축 payload 길이
  (ADR-0010)입니다. 의미 경계로 나눈 chunk의 payload가 이 값을 넘으면 split planner가 분할 대상으로
  봅니다.
- 승인되면 V.6.4가 이 값을 `config/performance-budgets.yaml`에 기록합니다. key 이름과 위치는 V.6.4에서
  정합니다.

이 ADR이 정하지 않는 사항:

- 작은 인접 chunk의 병합 기준(V.6.5)은 정하지 않습니다.
- 의미 경계가 임계치보다 큰 단일 노드를 어떻게 나눌지는 split planner 설계(V.6.4)에서 정합니다.

## Alternatives

- **64KiB**: 단일 수정의 Git 증가량이 가장 작습니다(74,679 bytes). 하지만 전체 저장 시간이 가장 길고
  (2434.7ms) chunk 파일 수가 256KiB의 4배입니다. 파일 수가 많으면 Git·파일 시스템·백신 비용이 커집니다
  (§21.4). 저장 처리량보다 Git 이력 크기를 우선하면 이 후보가 맞습니다. 권고하지 않음.
- **1MiB**: 전체 저장이 가장 빠릅니다(1103.2ms). 하지만 16바이트 수정에도 약 1MiB를 다시 쓰고 Git
  증가량이 64KiB의 약 14배입니다. 프로젝트 원본이 Git에 commit되므로(§3.3) commit마다 이 증가가
  이력에 누적됩니다. 권고하지 않음.
- **256KiB(제안)**: 전체 저장 시간은 1MiB의 약 1.2배에 그치고, 단일 수정 비용은 1MiB의 약 1/4입니다.
  측정한 세 후보 중 양쪽 비용의 균형점입니다.

## Consequences

- 승인 시: 임계치 이하로 유지되는 chunk에서는 단일 편집의 재기록·Git 증가량이 256KiB 수준으로
  제한되고, 같은 데이터의 파일 수는 64KiB 후보의 1/4입니다.
- 근거의 한계: 1회 측정, 단일 환경, 비압축성 합성 데이터, loose object 기준이며, 증분 저장 시간은
  직접 재지 않았습니다. 실제 문서 분포와 다를 수 있습니다. 승인 전에 실제 문서 형태의 fixture로 다시
  재야 할지는 제품 소유자가 정합니다.
- 미승인 상태가 이어지면 V.6.4와 그 후속(V.6.5, XVI.3.1)이 계속 차단됩니다.

## Compatibility

- 파일 포맷: 임계치는 포맷 필드가 아니라 저장 최적화 정책입니다(§21.1). 바꿔도 기존 chunk를 그대로
  읽을 수 있습니다. 분할·병합으로 ChunkId가 바뀌는 경우는 remap manifest(V.3.4)로 추적합니다.
- 공개 API·FFI: 영향 없음.

## Performance

- 승인되면 `config/performance-budgets.yaml`에 새 항목이 생깁니다(V.6.4 작업, 같은 diff에 ADR 필요).
- 위 표의 상대 비교가 근거이며, 절대값은 측정 환경에 따라 달라집니다.

## Security

- 영향 없음. 임계치는 payload 크기만 비교합니다.

## Migration

- 기존 저장 데이터는 임계치 없이 기록되었습니다. 세대와 chunk 파일은 불변이므로(ADR-0010) 기존
  세대는 바뀌지 않고, 분할은 이후 저장이 만드는 새 세대에만 반영됩니다.
- 임계치를 다시 바꾸려면 이 ADR을 supersede하고 측정 자료를 갱신합니다.
