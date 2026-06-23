# 작업 보고서: I.1.10 Bootstrap CI

- WBS: I.1.10
- 작업명: `bootstrap-macos`·`bootstrap-windows` job 추가 (pinned toolchain, xtask 검증)
- 상태: `[~]` (워크플로 작성 완료, macOS 단계 로컬 동등 검증. 실제 CI 실행·Windows job·job URL·artifact checksum은 CI에서 확보 — 미완료)
- 선행: I.1.9 (`[x]`)

## 필수 실행 환경

- `CI-MAC`, `CI-WIN` (둘 다 실제 GitHub Actions runner 필요).

## 변경 파일 목록

- `.github/workflows/bootstrap.yml` (신규)
- `docs/task-reports/I.1.10.md` (신규)
- `docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md` (상태 열 갱신: I.1.10 `[ ]` → `[~]`)

## 워크플로 구성

- job 2개: `bootstrap-macos`(macos-14), `bootstrap-windows`(windows-2022). 각 10단계.
- 단계: checkout → `rustup show` → `rustc -Vv` → `cargo -V` → `cargo metadata` → `cargo check -p xtask --locked` → `cargo xtask help` → fmt --check → clippy → test.
- production crate/앱 build 미포함. Windows job은 compile/headless 증거로만 표시.

## 실행한 검증

| 검사 | 결과 |
|---|---|
| YAML 문법(`ruby -ryaml`) | OK, job 2개 각 10단계 |
| macOS 단계 로컬 동등 실행 | 전부 통과(I.1.7~I.1.9 보고서 참조) |

## 미완료(연기) — `[~]` 사유

- 실제 GitHub Actions 양 runner 성공 증거(job URL, log artifact checksum), Rust release=1.96.0 일치, fixture-branch 실패 검증은 push 후 CI에서만 확보 가능. 확보 후 `[x]`로 승격.

## 남은 경고 수

- 0

## 작업 지시와 다른 점

- 없음(증거 단계만 CI로 연기).
