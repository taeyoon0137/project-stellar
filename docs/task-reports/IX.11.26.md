# 작업 보고서: IX.11.26 `stellar.yaml` 기본값 세트 선택과 사용자 세트 파일

- WBS: IX.11.26
- 작업명: `stellar.yaml`의 선택 키 `style_defaults: <세트 이름>`과 사용자 세트 파일
  `design/style-defaults/<name>.stellar.yaml` codec(ADR-0082 결정 3·4).
- 상태: 부분 완료 (`[~]`) — 2026-11-23 통합 판정: 이 행의 구현은 끝났지만 선행 IX.11.24이 `[x]`가 아닙니다. 선행이 끝나면 `[x]`. 구현 보고 당시 기록: 완료 (`[x]`) — 완료 기준을 모두 확인했습니다. codec 본체는 앞선 기본값 세트 작업(커밋 `0045ac21`)이 넣었고,
  이번에 빠진 거부 fixture(내장 세트에서 끝나지 않는 사슬, 깊이·크기 상한, symlink)와 저장 경로용 두 단계 쓰기 API를
  더했습니다. 선행 IX.11.24의 상태 판정은 통합 때 따릅니다.
- 결정: ADR-0087(신규), ADR-0086 결정 9, ADR-0082 결정 3·4
- 선행: IX.11.24, V.10.2

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/project/src/manifest_style_defaults.rs`
  - `prepare_selection(root, expected, name)`과 `PreparedSelection::commit(root)`: 저장 경로가 generation 저장 전에
    검사하고 저장 뒤에 한 줄을 쓰는 두 단계 API(ADR-0087 결정 2).
  - `StyleDefaultsFileError::Stale`(`style-defaults-stale`): 연 시점 선택과 지금 선택이 다를 때.
  - `set_active`는 두 단계 API를 그대로 부릅니다(같은 일을 하는 코드 한 벌).
  - test 두 개 추가.

## 구현 요약

- 키가 없으면 `css`이고, `css`를 고르면 키를 지웁니다. 세트 변경과 되돌림은 `stellar.yaml` 한 줄 diff입니다.
- 사용자 세트 파일은 `extends`(필수), longhand 한 줄씩, 선택 구획 `frame`·`text`·`image`·`shape`입니다. 값은
  문서 crate의 longhand parser 하나로 읽습니다.
- 거부: 순환, 없는 세트(내장 세트에서 끝나지 않는 사슬), 16단계 초과, 내장 이름, 이름 형식, 모르는 longhand·구획,
  중복, 문법 오류, `var()`·`calc()`, 1 MiB 초과, 값 자리의 mapping(깊이), symlink 디렉터리·파일, 256개 초과.
- 준비 단계는 파일을 쓰지 않고, commit은 준비 때 읽은 bytes hash가 그대로일 때만 씁니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-project --lib manifest_style_defaults` | 통과(7). 키 없음 = `css`, 키 왕복, 사용자 세트 정규 쓰기 왕복, 모르는 longhand·구획·문법 오류·`var()`·`calc()`·중복 거부, 순환, 내장 세트에서 끝나지 않는 사슬, 16단계는 받고 17단계 거부, 1 MiB 초과, 깊이, symlink, 이름 형식, 한 줄 쓰기, 준비·commit의 연 시점 확인과 hash 확인. |
| `cargo test -p stellar-project` | 통과(lib 430, 통합 test 전부). |
| `cargo test -p stellar-cli --test style_defaults` | 통과(3). `set_active`를 쓰는 CLI 경로가 그대로입니다. |

## 차이(작업 지시와 다른 점)

- codec 자체는 앞선 작업에서 이미 들어왔습니다. 이번 작업은 빠진 완료 기준 fixture와 저장 경로용 두 단계 쓰기
  API를 더했습니다.
- 쓰지 않는(고르지 않은) 사용자 세트의 사슬 오류는 그 세트를 고를 때 드러납니다. 파일 형식 오류는 읽을 때 바로
  드러납니다(ADR-0087 결정 1).

## 남은 위험 / 한계

- 사용자 세트 파일의 외부 편집 반영(ADR-0059)은 앱 runtime 연결 몫입니다. headless 경로는 열 때마다 다시 읽습니다.
- MCP 세션 cache는 사용자 세트 파일 변경을 최신 판정에 넣지 않습니다(ADR-0087 Consequences).
