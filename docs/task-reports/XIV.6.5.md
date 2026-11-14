# 작업 보고서: XIV.6.5 실제 Index provider의 semantic 재검증

## WBS

- WBS: XIV.6.5
- 작업명: runtime stage 적용 뒤 실제 Index snapshot과 의미 후보 비교
- 상태: `[x]`
- 선행: XIV.6.4 (`[x]`, `b0e99ea9`), XIV.2.5 (`[x]`)

## 변경 파일

- `crates/runtime/src/staging.rs`: `apply_stage_candidate`, 구조화된 적용/검증/복원 결과, 공용 임시 경로 검사. 기존 prepare·delta·blob·loader를 재사용합니다.
- `crates/runtime/src/staging/tests.rs`: 실제 적용 성공, semantic 불일치 복원, 검증 도중 외부 writer 경합.
- 구현 계획과 이 보고서. UI/FFI 연결이나 새 worker는 없습니다.

## 명령과 종료 코드

전용 `CARGO_TARGET_DIR="$PWD/target/semantic-index-publish"`를 사용했습니다.

| 명령 | 결과 |
|---|---|
| `cargo check -p stellar-runtime --no-default-features` | 0 |
| `cargo test -p stellar-runtime staging::tests --no-default-features` | 0 — 6개 통과 |
| `cargo clippy -p stellar-runtime --all-targets --no-default-features -- -D warnings` | 0·경고 0 |
| `cargo fmt -p stellar-runtime` | 0 |

## 환경과 증거

- `DEV-MAC`, macOS 27.2, Apple Git 2.54.0. 임시 GitFixture에서만 실행했습니다.
- 코드의 Git 실행은 모두 기존 GatedGitRunner 경로입니다. actual index를 Index provider로 읽어 별도 임시 tree에 materialize하고 기존 SavedProjectLoader와 semantic 변환기로 확인합니다.

## 테스트

- 같은 chunk의 A/B 중 A만 적용한 실제 index의 semantic snapshot이 CandidateIndexSnapshot과 같습니다. unrelated staged README도 유지됩니다.
- 후보 tree와 기대 semantic을 의도적으로 어긋나게 한 fixture는 실제 publish 뒤 ReopenMismatch를 검출하고 원래 index tree로 복원합니다.
- publish 증표 발급 이후 semantic 검증 중 별도 Git writer가 index를 바꾼 fixture는 IndexChanged와 rollback StaleBase를 함께 반환합니다. 외부 OID를 덮지 않습니다.
- 검증 전후 실제 index가 바뀌었는지도 확인합니다. 검증 materialization temp는 정리되며, 작업 트리 밖의 parent와 실제 worktree root를 요구합니다.

## 성능

- 실제 index materialization·loader 재열기 비용이 추가됩니다. REF 성능 측정은 하지 않았습니다.

## 차이

- 현재 index를 무조건 덮는 복원 대신 성공 증표의 전체 bytes가 유지될 때만 복원합니다. 미확정 publish에는 증표가 없어 역쓰지 않습니다. 이는 사용자의 경합 보존 요구를 우선한 완료 범위입니다.

## 남은 위험

- 실제 index 적용/검증 Git 명령은 취소 가능하지만 기존 delta/blob 준비는 단계 사이에서 취소를 관찰하며 개별 Git 명령 timeout을 따릅니다. 큰 준비 작업의 즉시 취소·전체 operation deadline은 아직 보장하지 않습니다.
- 검증 실패 뒤 취소·경합·I/O 실패로 복원을 확정할 수 없으면 오류에 rollback 결과를 함께 보냅니다. 성공으로 보고하지 않습니다.
- macOS의 검증된 Git/index 범위만 지원합니다. 실제 앱 QA·UI/FFI·CI·REF 검증은 수행하지 않았습니다.
