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

## 통합 재검증 (2026-11-14)

- `169903b0`·`636034ba`·`b0e99ea9`·`63c2fc96`의 이력과 `8bc056b3`의 Git 호환성 검사를 통합했습니다. 기준은 `bc5b2db2`를 포함한 Text 원본 왕복 통합본 `d84af06b`입니다.
- `RUST_TEST_THREADS=1 cargo test --locked --offline -p stellar-git-porcelain -p stellar-runtime --no-default-features`: 호환성 병합 전 475개 통과(문서 테스트 14개 포함).
- 호환성 병합 후 `RUST_TEST_THREADS=1 cargo test --locked --offline -p stellar-git-porcelain index_publish::`: Apple Git 2.54.0과 Fork Git 2.50.1에서 각각 20개 통과. Fork는 `STELLAR_INDEX_PUBLISH_TEST_GIT`로 선택했습니다.
- `change_between_snapshot_and_lock_is_rejected_including_flags_only`를 같은 compiled test binary에서 각 Git으로 10회씩 실행해 모두 통과했습니다. readiness·FD 소유 증명·receipt·잠금 안 bytes 재검사를 유지합니다.
- 새 sparse 회귀는 `git config --worktree index.sparse true`의 유효값을 적용 전에 확인합니다. 지원하지 않는 sparse index는 거부하며 index bytes·유효 설정값·잠금 정리를 확인했습니다. local 설정만 바꾸면 기존 worktree 설정이 우선하므로, 사전 값 확인 없이 설정 변경 여부를 판단하지 않습니다.
- 원장은 병합 커밋이 HEAD 이력에 들어간 뒤 `cargo xtask generate-ledger`로 생성했습니다. 병합 전에는 신규 WBS의 커밋 근거를 찾지 못해 생성이 거부됐으며, validator를 우회하거나 원장을 직접 수정하지 않았습니다.

### 통합 재개 검증

- 부모 통합본 `d27cad14`의 입력·inverse 512-byte 상한, XIV.6.6 원본 불변성 검증, `f324b85b`의 앱·CLI 공용 저장 잠금을 `19713932`로 병합했습니다. 위 ADR-0059 호환 범위는 유지합니다.
- `CARGO_BUILD_JOBS=3 RUST_TEST_THREADS=1 CARGO_NET_OFFLINE=true cargo test --locked --offline -p stellar-git-porcelain -p stellar-runtime --no-default-features`: 종료 0, 483개 통과(문서 테스트 14개 포함). Apple Git 2.54.0의 index publish 23개가 포함됩니다. Fork Git 2.50.1을 지정한 `cargo test --locked --offline -p stellar-git-porcelain index_publish::`도 종료 0, 23개 통과입니다.
- 같은 환경의 `bash scripts/macos/smoke.sh`: 종료 0. 실제 Swift compile/link와 앱 실행, window-session 68개, session-commands 56개, window-close 98개, key-mapping 101개 및 기본 셸 검사를 통과했습니다. 활성 창의 native 메뉴·트랙패드 사용자 QA는 포함하지 않습니다.
- 이전 전체 Rust 검사는 `stellar-local-ipc`의 `pending_handshakes_are_capped_and_resume_when_one_finishes`에서 timeout으로 실패했습니다. `crates/local-ipc/src/listener.rs`의 이 테스트만 수정해 인증된 pending 상태·fd readiness로 cap, EOF 후 슬롯 해제, 실제 backlog 인증과 WELCOME을 확인합니다. 두 번째 backlog 연결로 accept 상한도 확인하고 watchdog으로 테스트 대기를 제한합니다. 제품 인증·기본 timeout은 그대로입니다. 수정 후 단독 실행과 compiled binary 10회 반복 모두 통과했습니다.
- 수정 후 `CARGO_BUILD_JOBS=3 RUST_TEST_THREADS=1 CARGO_NET_OFFLINE=true cargo xtask check-core`: 종료 0. workspace fmt·전체 clippy(경고 0), workspace 테스트 3,916 passed·10 ignored(문서 테스트 포함), 별도 doctest 54 passed입니다. 이전에 실패한 IPC 테스트도 전체 실행에서 통과했습니다.
- `generate-ledger`, `validate-task-status`, `sync-plan --check`, `validate-crate-graph`, 변경 경로의 `validate-adr`, XIV.6.2~6.6의 `verify-task` 및 보고서 검사는 종료 0입니다. 추가 확인한 `verify-task XXIV.4.5`는 기존 선행 XXIV.4.4 미완료에 따른 `missing-dependency`로 종료 65였습니다. 기존 `[~]`와 `reported_done`을 유지하며 완료로 올리지 않습니다.

## 차이

- 현재 index를 무조건 덮는 복원 대신 성공 증표의 전체 bytes가 유지될 때만 복원합니다. 미확정 publish에는 증표가 없어 역쓰지 않습니다. 이는 사용자의 경합 보존 요구를 우선한 완료 범위입니다.

## 남은 위험

- 실제 index 적용/검증 Git 명령은 취소 가능하지만 기존 delta/blob 준비는 단계 사이에서 취소를 관찰하며 개별 Git 명령 timeout을 따릅니다. 큰 준비 작업의 즉시 취소·전체 operation deadline은 아직 보장하지 않습니다.
- 검증 실패 뒤 취소·경합·I/O 실패로 복원을 확정할 수 없으면 오류에 rollback 결과를 함께 보냅니다. 성공으로 보고하지 않습니다.
- macOS의 검증된 Git/index 범위만 지원합니다. 실제 앱 QA·UI/FFI·CI·REF 검증은 수행하지 않았습니다.
- 현재 runtime 의미 후보는 기존 `design/current`·generation·objects 호환 경로입니다. ADR-0059의 텍스트 원본 의미 선택 stage와 기본 저장 전환은 포함하지 않습니다. `.stellar/cache/`를 stage 원본으로 사용하지 않으며, 텍스트 후보 직렬화·재검증 연결은 V.10의 후속 검증 대상입니다.
