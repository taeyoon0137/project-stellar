# 차단 보고서: V.10.3 기존 프로젝트의 명시적 텍스트 export와 CLI 검사

## WBS

- WBS: V.10.3
- 작업명: 기존 프로젝트의 명시적 텍스트 export와 CLI 검사
- 상태: `[!]`
- 선행: V.10.2 `[x]`, XXIV.1.3 `[~]`

## 실패 선행조건

- XXIV.1.3이 `[~]`입니다. 계획 1.2 규칙 4는 선행이 모두 `[x]`가 아니면 진척을 올리지 않습니다.
- 2026-11-14 구현 때 선행 미완료를 알면서 `[~]`로 올렸고, 2026-11-15 검토에서 `[!]`로 고쳤습니다.

## 재현 명령

- `rg -n '^\| (XXIV\.1\.3|V\.10\.3) ' docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md` — XXIV.1.3 `[~]`.
- `cargo xtask verify-task V.10.3` — 선행 미완료로 missing-dependency(65)입니다.
- `cargo test -p stellar-cli --test text_source` — 이미 있는 export·check 경로의 검사입니다. 2026-11-15 DEV-MAC의
  `cargo test --workspace`(종료 0)에 포함되어 6개 통과했습니다.

## 필요한 결정과 환경

- `DEV-MAC`. 형식·배치 결정은 [ADR-0061](../adr/ADR-0061-text-source-format-and-layout.md)(2026-11-15 Accepted)을 따릅니다.

## 변경되지 않은 파일

- 선행 완료 전에 들어온 구현은 유지합니다: `crates/ops/src/text_source.rs`의 export·check와 기존 generation 전체 pointer 집합
  비교, `tools/stellar-cli/src/text_source.rs`의 `text-source export|check`(`0a26ffe3`). 2026-11-15 검토에서 쓰기 버전을 2 하나로
  재정의하고(`fa9cf642`) 프로젝트 루트 안으로의 export를 거부하게 고쳤습니다(`a74cf26f`).
- 기본 열기·저장은 바꾸지 않았습니다. 텍스트 파일 편집 명령(apply)은 A4에서 삭제했고 V.10.6 범위입니다.

## 재개 조건

- XXIV.1.3이 `[x]`가 된 뒤 검증 열(실제 CLI export→check→재export 거부, 기존 generation bytes 보존, unknown optional·미사용
  chunk 거부, `cargo xtask check-core`)을 다시 실행해 보고서를 완료 형식으로 씁니다.
