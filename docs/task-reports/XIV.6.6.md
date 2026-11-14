# 작업 보고서: XIV.6.6 실제 stage의 Working Tree·문서 불변성

## WBS

- WBS: XIV.6.6
- 작업명: 실제 stage 성공·거부·복원의 bytes와 메모리 revision 보존
- 상태: `[x]` — 검증된 macOS·Git·index 지원 범위
- 선행: XIV.6.5 (`[x]`, `63c2fc96`)

## 변경 파일

- `crates/runtime/src/staging/tests.rs`: 6종 실패 경계에서 파일 digest·문서 records/revision·실제 index 비교. 실제 적용 테스트는 macOS에 한정합니다.
- `docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md`, 이 보고서. 이 단위는 테스트·문서 변경이며 production 동작은 XIV.6.2~6.5 및 입력 한도 보강 `0a549a33`을 사용합니다.

## 명령과 종료 코드

모든 Cargo 실행은 전용 `CARGO_TARGET_DIR="$PWD/target/semantic-index-publish"`입니다.

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-runtime staging::tests --no-default-features` | 0 — 7개 통과 |
| `cargo test -p stellar-git-porcelain index_publish::` | 0 — Apple Git 21개 통과 |
| 같은 명령, `STELLAR_INDEX_PUBLISH_TEST_GIT=<임시 upstream Git 2.54.0>` | 0 — 21개 통과 |
| `cargo test -p stellar-git-porcelain -p stellar-runtime --no-default-features` | 0 — Git 351 unit + 3 doctest, runtime 91 unit + 22 integration + 11 doctest 통과 |
| `cargo clippy -p stellar-git-porcelain -p stellar-runtime --all-targets --no-default-features -- -D warnings` | 0·경고 0 |
| `cargo fmt -p stellar-git-porcelain -p stellar-runtime` / `git diff --check` | 모두 0 |

## 환경과 증거

- `DEV-MAC`, macOS 27.2, Apple Git 2.54.0 (Apple Git-157). 임시 upstream Git 2.54.0으로도 porcelain 적용 경계를 확인했습니다.
- 새 GitFixture만 사용했습니다. 제품 저장소의 index·worktree·config에는 실험하지 않았으며 다른 checkout에 쓰지 않았습니다. 원격 push·release·서명·추가 작업자 실행은 없습니다.
- 전체 실행 로그는 전용 target의 `final-full-tests.log`, `final-clippy.log`, `final-upstream-tests.log`에 있습니다. 커밋된 테스트가 재현 원본입니다.

## 테스트

- 성공: 같은 chunk의 A/B에서 A만 실제 stage합니다. Index provider를 다시 materialize하고 loader로 열어 CandidateIndexSnapshot과 비교합니다. unrelated staged 파일, Working Tree의 A+B, 메모리 records와 revision 7이 유지됩니다.
- 취소·신뢰 거부·stale 후보·작업 트리 내부 temp·중첩 cwd·semantic mismatch를 각 독립 fixture에서 실행했습니다. 파일 digest와 문서 records/revision 19가 같습니다. 사전 취소/신뢰 거부의 spawn은 0입니다.
- 적용 전 거부는 index bytes 그대로이며 semantic mismatch는 원래 index tree로 복원합니다. 검증 temp는 제거되고 Git lock도 남지 않습니다.
- 별도 writer가 검증 중 변경한 경우의 복원 충돌은 XIV.6.5 테스트로 확인합니다. 외부 index를 덮지 않습니다.

## 성능

- 성능 수치를 주장하지 않습니다. REF·대형 프로젝트·전체 앱 QA는 실행하지 않았습니다.

## 차이

- 문서 상태를 받지 않는 기존 동기 runtime API를 호출하고 전후 records/revision을 비교합니다. 새 worker·UI·FFI·부분 선택 구현을 추가하지 않았습니다.

## 남은 위험

- 실제 적용은 macOS의 Apple/upstream Git 2.54.0, 지원 index 상태, 적용·복원 양방향 입력 예산 512 bytes 안으로 제한합니다. 초과 입력·selected flags·split/sparse·미확정 잠금 소유는 진단합니다. GNU/Linux·다른 Git 버전은 검증 전 거부합니다.
- 입력 승인 뒤 취소/timeout 또는 외부 writer 때문에 결과가 미확정이면 자동 역쓰기하지 않습니다. 정리 불명 lock도 직접 삭제하지 않습니다. 조건부 복원은 tree/mode/flags/REUC 기준이며 과거 stat cache bytes 복제는 아닙니다.
- 기존 delta/blob 준비의 취소는 단계 사이에서 관찰합니다. 전체 operation deadline·준비 중 즉시 취소·전원 장애 복구는 보장하지 않습니다.
- 다음 별도 단위는 XIV.7.3의 선택 unstage 적용 연결과 XIV.13.2 UI/FFI 진입점입니다. 이번 완료는 실제 stage API와 fixture 검증 범위입니다.
