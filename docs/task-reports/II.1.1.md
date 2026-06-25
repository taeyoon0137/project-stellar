# 작업 보고서: II.1.1 Rust 도구체인 baseline 승격

- WBS: II.1.1
- 작업명: Rust 1.96.0 공식 baseline 승격 + `docs/adr/ADR-0001-rust-toolchain.md`
- 상태: `[~]` (ADR 작성·헤딩 검증 완료. `[CI-MAC][CI-WIN]` rustc release·commit-hash 동일 검증은 CI에서 확보 — 미완료)
- 선행: G0 (CI 그린 보류 — 사용자 결정에 따라 CI/Windows 연기)

## 필수 실행 환경

- `CI-MAC`, `CI-WIN`.

## 변경 파일 목록

- `docs/adr/ADR-0001-rust-toolchain.md` (신규)
- `docs/task-reports/II.1.1.md` (신규)
- `docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md` (상태 열 갱신: II.1.1 `[ ]` → `[~]`)

## 구현 요약

- baseline·MSRV = Rust `1.96.0`. 단일 출처는 `rust-toolchain.toml` channel, `config/versions.yaml` rust.exact와 일치(validate-versions 강제).
- 업그레이드는 전용 WBS로만(ADR Migration 절차). 자동 변경 금지.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 검사 | 결과 |
|---|---|
| ADR 필수 heading 9개 각 1회 | 통과 |
| Decision에 1.96.0 명시 | 통과 |
| ADR 정책(ADR 동반 시 보호 경로 변경 통과) | 통과 |

## 미완료(연기) — `[~]` 사유

- `[CI-MAC][CI-WIN]`에서 `rustc -Vv` release·commit-hash 동일 검증은 GitHub Actions 양 runner에서만 확인 가능(bootstrap CI가 이미 `rustc -Vv`를 양 플랫폼에서 출력). 확보 후 `[x]`로 승격.
- 선행 G0는 CI 그린이 조건이므로 함께 보류.

## 남은 경고 수

- 0

## 작업 지시와 다른 점

- 없음.

## CI 증거 (승격)

- 런 `28084248458`에서 `CI-MAC`·`CI-WIN`의 `rustc -Vv`가 release `1.96.0`, commit-hash `ac68faa20c58cbccd01ee7208bf3b6e93a7d7f96`로 완전히 동일. ADR-0001 존재·heading 검증 완료.
- 상태 `[~]` → `[x]` 승격.
