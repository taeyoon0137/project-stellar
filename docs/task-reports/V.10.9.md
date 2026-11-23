# 작업 보고서: V.10.9 텍스트 asset catalog

## WBS

- WBS: V.10.9
- 등록: 2026-11-23 plan-revision이 임시 번호 TS-ASSET를 이 ID로 계획서에 등록하고 보고서를 `docs/task-reports/pending-wbs/TS-ASSET.md`에서 옮김
- 작업명: 글꼴·이미지·문서 style 표의 텍스트 asset catalog와 참조 전용 문서
- 상태: 부분 완료 (`[~]`) — 2026-11-23 plan-revision 계획서 판정: 계획 1.1의 "올바른 커밋"이 없음: 구현 커밋이 `Feat:`·`Fix:` 제목이라 `Task(<ID>)` 제목 커밋이 없고, 원장이 근거 커밋을 채우지 못합니다; 선행 V.10.4(`[~]`), V.10.8(`[~]`)가 `[x]`가 아님(계획 1.2 규칙 4). 보고 당시 기록: 완료 (`[x]`)
- 선행: V.10.4 `[~]`, V.10.8 `[~]`
- 구현 커밋: `df80100f`(catalog), `ae935137`(다중 파일 원본에서 문서는 catalog를 참조만 함). 결정: [ADR-0077](../adr/ADR-0077-text-source-full-semantics-and-migration-registry.md) 결정 1·3.

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일

- `crates/project/src/document_io/source_v4/catalog.rs`(신규), `source_v4/styles.rs`(Text style 표 전체 문법).
- `docs/spec/text-source-v4.md`의 "asset catalog" 절.

## 구현 요약

- `design/assets/catalog.stellar.yaml` 하나가 이미지 AssetRecord(출처 `none`·`project`·`external <지문> <이름>`), 등록 글꼴(ProjectFontRecord 필드·라이선스·글꼴 파일 AssetRecord), 문서별 Text style 표(typography 전체, 문단·문자 style 전체, OpenType feature·variation, 글꼴 file hash 참조)를 담습니다.
- 같은 글꼴을 여러 문서가 등록해도 catalog 항목은 하나이고 문서는 file hash로 참조합니다. 다중 파일 원본의 문서·View는 style 표를 담지 않습니다.
- 읽은 값은 binary record로 다시 인코딩해 원본 bytes와 같을 때만 받습니다. 보존 unknown field, 고정 face family, 문서마다 다른 같은 hash record는 거부합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-project --lib catalog` | 3 passed: 이미지(외부 출처 포함)·글꼴·두 문서 표 정확 왕복, 참조 없는 글꼴·순서·모르는 field·버전 거부, 같은 hash 다른 record 거부 |
| `cargo test -p stellar-project --test text_project` | catalog를 포함한 다중 파일 corpus 왕복 통과 |

## 테스트

- 위 명령 표의 테스트가 모두 통과했습니다. 남은 경고 수: clippy `-D warnings` 0.

## 성능

- 측정하지 않았습니다. 열기·저장 예산 판정은 기본 저장 전환(V.10.7)에서 실측합니다.

## 차이

- 작업 지시와 다른 점은 없습니다.

## 남은 위험

- 고정 face를 지정한 font-family와 보존 unknown field가 있는 record는 쓰기를 거부합니다. 그런 문서는 generation에 남습니다.
