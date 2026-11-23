# 작업 보고서: XII.10.15 스냅 엔진

- WBS: XII.10.15
- 작업명: 스냅 엔진
- 상태: 완료 (`[x]`)
- 결정: ADR-0105 결정 3
- 선행: 계획서 행의 선행 열

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, Apple M3 Max). 다른 작업 여럿이 같은 장비에서 동시에 돌았습니다.

## 변경 파일 목록

- `crates/runtime/src/canvas/snap.rs`(옛 `app/snap.rs`를 옮겨 확장), `crates/runtime/src/canvas/snap/tests.rs`(신규)
- `crates/runtime/src/canvas.rs`(mod 등록), `crates/runtime/src/app.rs`(`app::snap` 재내보내기)

## 구현 요약

- 후보 우선순위: 가이드 → 레이아웃 그리드 → 부모·형제 가장자리·중심 → 같은 간격 → 같은 크기 → 픽셀 그리드. 같은 종류면 가까운 쪽, 그다음 작은 좌표(결정론).
- 후보는 제스처 시작 때 한 번 모읍니다(`collect_in`: 가시 영역 밖 형제 제외, 흐름 자식은 객체 후보 없음). 축마다 정렬 표와 형제 공간 인덱스(`GridIndex`)를 만들고 update는 이분 탐색·띠 질의만 합니다.
- 허용 오차는 화면 pt / 부모 local 1px 화면 배율이라 줌과 무관하게 화면에서 같습니다.
- 기존 이름(`SnapCandidates`, `snap`, `collect`, `tolerance`)과 앞선 test는 그대로 통과합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-runtime --lib -- canvas::snap` | 통과(16): 우선순위, 가이드·그리드, 같은 간격·크기, 픽셀, 상한, 줌 25%·400% 화면 허용 오차 동일, `canvas/snap-candidates-20k` 결정론, 흐름 자식, scene 수집·가시 영역 |
| `cargo test --release -p stellar-runtime --lib -- canvas::snap::tests::measure --ignored --nocapture` | 통과: 2만 후보 5,000 update 중앙값 30.75µs, p99 44.5µs, 최대 53.6µs(예산 p99 0.35ms) |

## 차이(작업 지시와 다른 점)

- 허용 오차 기본값 6pt는 XII.10.2(ADR-0104) 값이 정해질 때까지 잠정값입니다. 엔진은 값을 인자로 받습니다.
- 가이드·그리드 선은 저장 위치(XII.10.18~XII.10.20)가 없어 호출자가 선 목록으로 넘깁니다.
- 측정은 개발 장비·부하 상태 값이며 REF-MAC 판정이 아닙니다.

## 남은 위험 / 한계

- 기준 장비(REF-MAC)에서 다시 재야 합니다.
