# 작업 보고서: V.10.4 전체 영속 의미의 텍스트화와 다중 파일 배치

## WBS

- WBS: V.10.4
- 작업명: 모든 node·modifier·component·token·resource를 텍스트화하고 문서·페이지·View·큰 Frame 파일 참조와 이름/StableId 규칙 완성
- 상태: 부분 완료 (`[~]`) — 2026-11-23 plan-revision 계획서 판정: 남은 범위가 있음(아래 본문). 보고 당시 기록: 부분 완료 (`[~]`) — 텍스트화·배치·거부 규칙·Git 병합·CLI 검증은 통과했습니다. headless MCP 읽기 비교 테스트는 만들지 않았고, 선행 V.10.3의 상태는 XXIV.1.3 판정(ledger-verification lane)에 따릅니다.
- 구현 커밋: `58709614`(v4 node·modifier 문법), `df80100f`(catalog), `ae935137`(다중 파일 원본·generation 변환), `07f81702`(전환 준비 corpus). 결정: [ADR-0077](../adr/ADR-0077-text-source-full-semantics-and-migration-registry.md). 명세: [`text-source-v4.md`](../spec/text-source-v4.md).

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일

- `crates/project/src/document_io/source_v4/{project,slug,token}.rs`(신규), `component_source.rs`(모든 node 종류의 정의 tree, `from_entry`).
- `crates/ops/src/text_source.rs`: `export_project`, `check_project`. `tools/stellar-cli/src/text_source.rs`: `text-source export-project|check-project`.
- 테스트: `crates/project/tests/{text_project,text_switch_corpus}.rs`(신규), `tools/stellar-cli/tests/text_project.rs`(신규).

## 구현 요약

- 배치: `design/document.stellar.yaml`, `pages/<page>/<view>.stellar.yaml`, 큰 Frame `pages/<page>/<view>/<frame>.stellar.yaml`(`$ref`), `components/`, `tokens/`, `assets/catalog.stellar.yaml`. 이름은 ASCII slug(겹치면 StableId 접미사), 정체성은 파일 안의 StableId입니다.
- 분할: 직계 자식 2,000개 초과 Frame은 항상, 파일 1 MiB 초과면 하위 tree가 가장 큰 Frame(64 KiB 이상)부터 나눕니다.
- 읽기: 참조하지 않은 파일·두 번 참조·모르는 경로·symlink·부모가 다른 파일에 있는 node를 거부합니다.
- 손실 저장 없음: 다시 읽은 의미(정렬한 문서 레코드·library bytes·token·image record)가 같을 때만 씁니다. generation 변환은 chunk pointer 전체를 소비해야 합니다.
- 거부: 컴포넌트 Property·Slot·상태 머신, 리터럴이 아닌 token 표현식, 리터럴로 쓰지 않는 값 타입, root 문서 둘 이상, 알 수 없는 chunk.
- token: `stellar-bindings`에 crate 정책상 의존할 수 없어 core 값 기반 `TokenSetSource`로 파일 의미를 정의했습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-project --test text_project` | 8 passed: random 120개·golden 12개·rich 3개 문서(+컴포넌트·token·이미지·Text style 표) 전체 왕복과 semantic snapshot 동일, 같은 bytes 재생성, 2,000 자식·1 MiB 분할, 분할 불가 1 MiB 초과 거부, 미참조·중복·모르는 파일·field·버전 거부, 미지원 의미 쓰기 거부, 디스크 쓰기·symlink 거부, 시스템 git merge(같은 Frame의 width와 background) 충돌 없음 |
| `cargo test -p stellar-project --test text_switch_corpus` | 2 passed: 저장 generation 72개와 커밋된 창 session fixture를 텍스트로 옮겼다가 다시 읽어 semantic snapshot 동일 |
| `cargo test -p stellar-cli --test text_project` | 2 passed: export-project→check-project 같은 hash, 재export 같은 bytes, 기존 폴더·프로젝트 안 출력 거부, 원본 프로젝트 digest 불변, `stellar query` 값 = 텍스트 값 |
| `cargo test --workspace --locked --no-fail-fast` | 4,168 passed, 1 failed(`stellar-local-ipc` 타이밍 테스트, 단독 통과, 변경과 무관) |

## 차이

- headless MCP 읽기 비교: MCP headless는 신뢰 등록된 프로젝트의 ops snapshot을 읽습니다(`crates/mcp-core`). 텍스트 원본과 MCP 값을 비교하는 통합 테스트는 만들지 않았습니다(MCP 테스트 crate가 CLI·ops를 직접 쓰지 않고 신뢰 저장소 준비가 필요). CLI `query`와의 비교로 대신했습니다.
- 컴포넌트 Property·Slot·상태 머신과 표현식 token은 표현식 텍스트 문법이 정해진 뒤 씁니다. 지금은 거부합니다.
- 여러 파일 저장의 원자성(journal)은 V.10.5입니다. `write_new`는 새 폴더에만 파일별 원자 쓰기를 합니다.

## 테스트

- 이 행의 테스트는 모두 통과했습니다. workspace 전체에서는 변경과 무관한 `stellar-local-ipc` 타이밍 테스트 1건이 병렬 실행에서 실패했습니다(단독 통과). 남은 경고 수: clippy `-D warnings` 0.

## 성능

- 측정하지 않았습니다. 열기·저장 예산 판정은 기본 저장 전환(V.10.7)에서 실측합니다.

## 남은 위험

- registry 이름이 공개 포맷 동결 전에 바뀌면 v4 파일 키도 바뀝니다. 그 사이의 v4 파일은 다시 export합니다.
- registry 순서에서 이웃한 줄을 두 브랜치가 바꾸면 Git 텍스트 충돌입니다(merge driver XXIV.2.1 이후 해결).
- 큰 문서의 export·check 성능은 측정하지 않았습니다(V.10.7에서 실측).
