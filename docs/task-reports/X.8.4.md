# 작업 보고서: X.8.4 missing font 진단

- WBS: X.8.4
- 작업명: missing font diagnostic에 requested face, source, affected Text IDs 포함(텍스트 내용 미포함)
- 상태: 완료 (`[x]`)
- 선행: X.8.3(fallback chain, 이 lane에서 `[x]`), II.3.2(구조화 로그·민감 타입, `[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증. `planning/task-status.yaml`의 required_environments는 `DEV-MAC`만 지정).

## 변경 파일 목록

- `crates/text/src/missing_font.rs`(신규) — `TextFontUsage`, `MissingFontDiagnostic`, `collect_missing_fonts`, 단위 테스트.
- `crates/text/src/lib.rs` — `pub mod missing_font;` 선언.
- `docs/task-reports/X.8.4.md`(신규), 계획서(X.8.4 `[ ]`→`[x]`).

## 구현 요약

- **입력 `TextFontUsage { text_id, chain }`**: Text node 영속 ID와 그 Text의 fallback chain만 받습니다. Text 내용 필드가
  없어 진단 경로에 내용이 들어올 수 없습니다.
- **`collect_missing_fonts(usages, registry)`**: 각 chain을 registry로 해석해(X.8.3 `missing()`) 등록되지 않은 face마다
  진단 하나를 만듭니다. primary뿐 아니라 fallback 항목의 누락도 보고합니다(누락된 fallback이 glyph·줄바꿈 결과를
  바꿀 수 있기 때문). 같은 face라도 요청 출처가 다르면 별도 진단입니다.
- **`MissingFontDiagnostic`**: `requested()`(file hash, face index, variation axes), `source()`, `affected_text_ids()`
  (오름차순·중복 없음), 안정 코드 `CODE = "missing-font"`를 제공합니다. 진단 순서는 (face key, 출처, axes)
  오름차순이라 입력 순서와 무관하게 같습니다.
- **Display**: `missing-font: face sha256:<hex>#<index> (출처 <source>[, axes <tag>=<값>...]) 누락, 영향 Text <n>개: <id>...`
  형식으로 hash·index·axis·출처 이름·StableId만 씁니다.

## 실행한 명령 / 검증 (`DEV-MAC`, Apple Silicon arm64)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-text` | 종료 코드 0 |
| `cargo test -p stellar-text` | 종료 코드 0 (30 unit passed, 0 failed) |
| `cargo clippy -p stellar-text --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-text -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| **fixture 정확**(`missing_font_fixture_is_exact`): Text 5개(중복 입력 1개 포함)에서 A(Project)→`[1,2,4]`, A(System)→`[5]`, C(System, wght 700)→`[2]` 세 진단과 face index·axes 일치, 모든 face 등록된 Text 3은 미포함 | 통과 |
| **텍스트 내용 미포함**(`diagnostics_never_contain_text_content`): fixture Text 내용 문자열과 그 일부가 모든 진단의 Display·Debug 출력에 없음 | 통과 |
| Display 형식 고정(axes 있음·없음 두 경우) | 통과 |
| 입력 순서를 뒤집어도 같은 결과 | 통과 |
| 모든 face 등록 시·빈 입력 시 진단 0개 | 통과 |

## 차이(작업 지시와 다른 점)

- 진단 타입은 `stellar-diagnostics`의 `TelemetryEvent`/`Sensitive`(II.3.2)를 직접 쓰지 않고 `text` crate 안에 두었습니다.
  crate 의존 정책상 `stellar-text`는 `stellar-core-types`만 의존할 수 있기 때문입니다. 대신 진단에 민감 데이터(텍스트
  내용·경로)를 담을 필드를 아예 두지 않아 II.3.2의 비민감 경계를 지키고, 상위 계층이 `CODE`로 telemetry 이벤트에
  연결할 수 있게 했습니다.

## 남은 위험 / 한계

- 진단 결과를 문서 진단 index·UI에 연결하는 일(XXV.2.1)과 missing face에서 fallback chain 적용 후 줄바꿈 변경을
  진단하는 일(XIII.6.7)은 후속 작업입니다.
- Text node 목록과 각 Text의 chain을 문서에서 모으는 일은 호출자(문서·레이아웃 계층) 책임입니다.
