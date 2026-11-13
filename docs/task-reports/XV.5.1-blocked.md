# 차단 보고서: XV.5.1 LibrarySnapshot(ComponentDefinition·TokenSet index, LibraryId+commit+StableId resource key)

## WBS

- WBS: XV.5.1
- 작업명: LibrarySnapshot(ComponentDefinition·TokenSet index, LibraryId+commit+StableId resource key)
- 상태: `[!]`
- 선행: XV.2.1(`[x]`), IX.1.1(`[x]`), VIII.5.1(`[ ]`)

## 실패 선행조건

- 엄격 선행 VIII.5.1(`TokenSet`·`TokenDefinition`·`TokenName`·`TokenType` 정의)이 `[ ]`입니다. TokenSet index를 정의할 타입이 아직 없습니다.
- VIII.5.1은 document crate 범위라 이 lane에서 구현할 수 없습니다.

## 재현 명령

- `grep -E '^\| VIII\.5\.1 \|' docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md`

## 필요한 결정과 환경

- VIII.5.1 담당 lane의 완료가 필요합니다.
- 필수 환경: `DEV-MAC`.

## 변경되지 않은 파일

- 기능 코드는 변경하지 않았습니다(계획 §1.2 규칙 4). 이 보고서와 계획서 상태 표기만 바꿨습니다.

## 재개 조건

- VIII.5.1 `[x]` 후 착수합니다. entry 열기와 read lock 보유는 XV.3.6 `open_entry`·`OpenedEntry`를 씁니다.
