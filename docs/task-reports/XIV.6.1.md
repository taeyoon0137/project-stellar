# 작업 보고서: XIV.6.1 후보 bytes의 Git blob 기록

## WBS

- WBS: XIV.6.1
- 작업명: temp candidate file bytes를 시스템 Git blob으로 기록하고 반환 OID 검증
- 상태: `[x]` — 기존 candidate delta에서 Git blob 기록까지 검증했습니다.
- 선행: XIV.5.6, XIV.1.3 (`[x]`)

## 변경 파일

- `crates/git-porcelain/src/blob_write.rs`, `crates/git-porcelain/src/lib.rs`: 기존 GatedGitRunner·run_ok_with_input으로 blob을 기록합니다. `WrittenCandidate`는 base index와 mode/path/OID·삭제 목록을 유지합니다.
- `crates/git-porcelain/src/blob_write/candidate_tests.rs`: 실제 임시 Git 프로젝트 pipeline 검증.
- `crates/runtime/src/staging/tests.rs`: 기존 A/B 선택 후보의 blob bytes 및 동일 후보의 기록 명령 0 검증.
- 이 보고서와 후속 계획·생성 원장. 새 의존성·실행기·thread·worker·FFI API는 없습니다.

## 명령과 종료 코드

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-git-porcelain blob_write::tests -- --nocapture` (구현 전) | 101 — write_blob·OID parser 부재의 컴파일 실패, 3 unused import 경고 |
| 같은 명령 (구현 후) | 0 — 3개 통과 |
| `cargo test -p stellar-git-porcelain blob_write:: -- --nocapture` | 0 — primitive·pipeline 7개 통과 |
| `cargo test -p stellar-runtime staging::tests --no-default-features` (최초 연결) | 101 — 파일 digest가 새 Git object까지 불변으로 기대한 fixture 오류 1건 |
| 같은 명령 (fixture 수정 후) | 0 — 3개 통과; 기존 파일 전체 불변, 신규 경로는 Git objects만 허용 |
| `cargo fmt -p stellar-git-porcelain -p stellar-runtime -- --check` | 0 |
| `cargo xtask check-core` | 0 — workspace fmt·clippy·test·doc-test 통과, 경고 0 |
| `cargo xtask validate-task-report docs/task-reports/XIV.6.1.md` | 0 |
| `cargo xtask generate-ledger` | 0 — Task 커밋 `ba98f47c` 이후 실행, 해당 작업만 갱신 |
| `cargo xtask validate-task-status` / `cargo xtask sync-plan` / `cargo xtask verify-task XIV.6.1` | 모두 0 |
| `git diff --check` | 0 |

## 환경과 증거

- `DEV-MAC`: macOS 27.2 (26B123), Rust/Cargo 1.96.0, Apple Git 2.54.0.
- 기존 GitFixture 및 동일 runner의 환경 사본을 사용한 자동 테스트 임시 프로젝트에서만 object를 기록했습니다. 셸 명령 문자열·새 실행기·전역/계정 설정 변경은 없습니다.
- check-core test 단계 3849 passed·8 ignored·0 failed(이 단계 doctest 포함), 별도 doc-test 54 passed·0 failed. 로그는 `target/xtask-logs/`에 있습니다. 독립 코드 리뷰에서 새 확정 결함 없음.
- CI·LAB·REF·실제 앱 QA 검증은 실행하지 않았습니다.

## 테스트

- 빈 bytes·NUL·비UTF8 binary bytes의 실제 `hash-object -w --stdin` 결과를 `cat-file blob`으로 비교합니다. 각 호출의 실제 프로세스 증가량은 정확히 1입니다.
- OID parser는 SHA-1/256 소문자 hex 한 줄만 허용하며 null·잘못된 길이·추가 레코드·누락 LF를 거부합니다. 오류에는 출력 앞 128 bytes만 담습니다.
- untrusted 요청은 프로세스 증가 0입니다.
- 정상 저장 프로젝트에서 A만 포함한 3개 delta가 정확히 3개 blob 기록으로 연결됩니다. 반환 path/mode와 blob bytes가 delta와 같습니다. 기존 index 및 WorkingTree 파일 bytes는 모두 유지됩니다.
- 동일 후보·삭제만 있는 후보는 blob 기록 0입니다. 삭제 path와 mode·symlink 정보는 후속 단계용 값으로 그대로 전달합니다. 이 단계는 실제 index stage가 아닙니다.
- SHA-256 실제 임시 저장소에서도 OID 길이와 cat-file bytes를 확인합니다.
- 동일 후보를 3개 thread가 동시에 기록해도 명령 증가량이 정확히 12(각 4개)이고 결과 OID/mode/path가 같으며, actual index bytes와 WorkingTree bytes/mode가 유지됩니다.
- fixture 내 잘못된 object directory로 실제 Git 비영 종료를 유발해 구조화된 GitOpError와 원본 불변을 확인합니다.


- 기본 제외 8개는 20 GiB stream, 200k/1m 문서 tier, 50k 복원 성능, nightly fuzz, 전체 시스템 폰트 scan, hit-test 성능, `product_serializer_splits_views_over_approved_threshold`입니다. 마지막 항목은 기존 document_io serializer와 V.6.4 split planner 미연결 결함으로 제외되어 있습니다. 이 결함이나 성능 gate를 해결했다고 주장하지 않습니다.

## 성능

- Upsert당 hash-object 1회이며 삭제와 unchanged에는 기록 명령을 실행하지 않습니다. 파일별 Git 프로세스 비용은 아직 benchmark하지 않았습니다.
- 변경 bytes의 메모리 상한은 선행 CandidateDelta에서 적용합니다. REF 성능 예산 증거는 없습니다.

## 차이

- stdin에는 filter가 적용되지 않으며 경로를 전달하지 않습니다. 이는 [Git hash-object 공식 문서](https://git-scm.com/docs/git-hash-object)의 stdin 규칙을 따릅니다.
- primitive API의 테스트 선행 실패를 확인한 뒤 구현했습니다. pipeline과 runtime fixture 검증은 이어서 추가했습니다.
- 원장은 해당 Task 커밋 뒤에만 생성합니다.

## 남은 위험

- 실패 전에 기록한 미참조 object는 남을 수 있습니다. 다른 사용자의 object와 구분 없이 삭제하지 않으며 Git GC에 맡깁니다. 부분 WrittenCandidate는 반환하지 않습니다.
- `WrittenCandidate`는 index 설치 성공이나 승인을 나타내지 않습니다. 실제 index 비교·적용·복원과 stage 이후 semantic 검증은 XIV.6.2~XIV.6.6에 남습니다.
- 준비 이후 index 경합과 index flags/확장 metadata의 보존이 해결되지 않았으므로 실제 사용자 index 적용을 이 API에 연결하지 않았습니다.
