# 작업 보고서 XIV.7.3 선택 Unstage의 실제 Git 적용

## WBS

- WBS: XIV.7.3
- 작업명: 승인한 Unstage 후보를 실제 index에 기록
- 상태: `[x]`
- 선행: XIV.7.2, XIV.6.6

## 변경 파일

- `crates/runtime/src/staging.rs`와 `crates/ops/src/candidate.rs`에서 기존 inverse·dependency 검사와 index 적용·재검증·조건부 복원을 연결했습니다.
- `crates/runtime/src/app/git_names.rs`, `crates/ffi/src/git_names.rs`, 생성 헤더와 macOS Git 패널에서 표시 세대와 이름 변경의 양방향 값을 확인합니다. 새 C action 2가 선택한 이름의 Stage를 해제합니다.
- `crates/runtime/src/staging/tests.rs`, `ShellGitStageCheck.swift`, `scripts/macos/git-stage-check.py`가 실제 Git 결과를 검사합니다. 제품 사용법은 `docs/status/macos-git-name-stage.md`에 있습니다.

## 필수 실행 환경과 테스트

DEV-MAC에서 시스템 Git과 임시 저장소를 사용했습니다. 두 속성을 함께 Stage하고 작업 파일을 추가 편집한 뒤 이름만 Unstage했습니다. 재열린 index에는 HEAD 이름과 staged 잠금이 남고 다른 staged 파일도 보존됩니다. 잘못된 미리보기와 이동한 HEAD를 거부하고 index 바이트가 유지되는지 확인했습니다. Working Tree의 generation 포인터가 해석되지 않는 경우에도 해당 파일에 쓰지 않습니다.

AppKit 검사에서는 CLI로 실제 편집한 저장소를 열고 이름을 Stage한 뒤 창을 다시 열어 체크를 해제합니다. 두 시점의 index를 각각 `git checkout-index`로 내보내 이름과 잠금을 디스크에서 다시 읽었습니다. 작업 파일의 이름과 잠금 편집은 유지됐습니다.

## 실행한 명령과 결과

- `RUST_TEST_THREADS=1 cargo test --locked --offline -p stellar-runtime --lib staging::` — 9개 통과.
- `./scripts/macos/smoke.sh` — 통과. 실제 창의 Stage·재열기·Unstage와 시스템 Git의 디스크 검증이 포함됩니다.
- `cargo clippy --locked --offline -p stellar-runtime -p stellar-ffi --all-targets -- -D warnings`, `cargo fmt --all -- --check`, `git diff --check` — 통과.

## 성능

대규모 저장소에서의 응답 시간은 측정하지 않았습니다. Unstage 준비 시 HEAD와 Index를 열며, 적용 전후에는 기준 HEAD를 다시 확인합니다.

## 차이와 남은 위험

- 검증된 제품 UI는 커밋된 프로젝트에서 저장된 노드 이름 한 건을 다룹니다. Working Tree 밖의 별도 임시 폴더를 사용합니다. unborn HEAD는 진단합니다.
- 실제 index 쓰기는 기존 macOS 적용 범위와 512바이트 입력 한도를 따릅니다. 지원 범위 밖의 문서·필수 의존성·Git 형식에는 기존 거부 정책을 적용합니다.
- 새 generation의 의미 상태를 기준으로 복원합니다. 기존 Git에 남은 immutable object·manifest를 보존하므로, 의미 변경이 사라진 뒤에도 일반 Git diff에 generation 파일이 남을 수 있습니다. 파일 단위 전체 Unstage는 별도 동작입니다.
- HEAD와 index는 서로 다른 Git 자원입니다. 적용 전후의 HEAD 관찰과 기존 index 소유 확인·조건부 복원을 사용합니다. 외부 writer와의 원자적 다중 ref 트랜잭션은 보장하지 않습니다.
