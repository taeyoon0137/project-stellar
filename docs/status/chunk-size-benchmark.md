# chunk 목표 크기 측정 (V.6.3)

<!-- 이 파일은 cargo xtask bench-chunk-size 가 생성합니다. 직접 수정하지 마세요. -->

- 데이터셋: 결정적 pseudo-random 8MiB, 단일 지점 수정은 중앙 chunk 16바이트.
- 측정 환경: DEV-MAC 로컬 1회 실행 — 절대값은 참고용이며 후보 간 상대 비교가 목적입니다.
- **본 보고서는 결론을 포함하지 않습니다.** 임계치 확정은 ADR(V.6.4)에서 수행합니다.

## raw 결과

| 후보 | chunk 수 | 저장(ms) | 로드(ms) | 재기록 bytes | Git delta bytes |
|---|---|---|---|---|---|
| 64KiB | 128 | 2434.7 | 339.7 | 65536 | 74679 |
| 256KiB | 32 | 1324.1 | 336.0 | 262144 | 265343 |
| 1MiB | 8 | 1103.2 | 333.2 | 1048576 | 1050389 |

## 해석 지침(ADR 입력)

- 저장·로드 시간: 작은 chunk는 파일 수·fsync 횟수가 늘고, 큰 chunk는 단일 변경의 재기록 비용이 늘어납니다.
- 재기록 bytes: 단일 지점 수정 시 다시 쓰는 payload 총량 — chunk 크기에 비례합니다.
- Git delta: content-addressed 파일 교체로 인해 두 번째 commit이 object store에 추가한 크기입니다.
