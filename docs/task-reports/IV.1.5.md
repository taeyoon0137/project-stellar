# 작업 보고서: IV.1.5 canonical YAML writer

- WBS: IV.1.5
- 작업명: `YamlValue` → canonical 텍스트 writer(결정론·고정 포맷)
- 상태: 완료 (`[x]`)
- 선행: IV.1.1(`YamlValue`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/project/src/yaml/writer.rs` (신규: `write_canonical` + 포맷·결정론·round-trip 테스트)
- `crates/project/src/yaml/mod.rs` (`writer` 모듈 공개·재노출)
- `docs/task-reports/IV.1.5.md`(신규), 계획서(IV.1.5 `[ ]`→`[x]`)

## 구현 요약

- `write_canonical`은 고정 규칙으로 기록합니다: 매핑 **키 순서 유지**(재정렬 없음), 중첩마다
  **공백 2칸** 들여쓰기, **LF**만 사용, **trailing newline**으로 끝, 주석 미보존.
- 빈 컬렉션은 `[]`/`{}` flow로, 비어 있지 않으면 block 스타일(시퀀스 항목은 `-` 줄 뒤 +2 들여쓴
  내용)로 기록합니다.
- 스칼라 인용 정책: **안전한 plain 식별자**(영문 시작, 영문/숫자/`_`/`-`, 불리언·널 키워드
  아님)만 무인용, 그 외(숫자처럼 보임·키워드·특수문자·공백·빈 문자열)는 이스케이프한 더블쿼트.
  덕분에 제한 loader(IV.1.2)로 다시 읽으면 동일한 `YamlValue`가 됩니다.

## 적대적/회귀 검증(ultracode)

- **결정론**: 동일 모델 100회 기록 byte-identical(완료 기준).
- **포맷 golden**: 중첩 매핑·시퀀스·빈 컬렉션을 가진 모델의 정확한 바이트 출력 고정.
- **write→load round-trip**: sample 트리·minimal/full 매니페스트가 텍스트를 거쳐 동일 트리/모델로
  복원됨.
- **까다로운 스칼라 round-trip**: 빈값·공백·콜론·해시·백슬래시·따옴표·개행(`\n`)·탭·CR·키워드
  (true/yes/on/null/~)·숫자(1/010/26.0)·대괄호·중괄호·한글·emoji·`<<`·alias/anchor/tag-like를
  매핑 값·시퀀스 항목·매핑 키 위치에서 각각 round-trip(이스케이프 정확성 확인).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-project` (41 unit) | 0 |
| `cargo fmt -p stellar-project -- --check` | 0 |
| `cargo clippy -p stellar-project --all-targets -- -D warnings` | 0 |

## 차이(작업 지시와 다른 점)

- "주석 보존 없음"은 모델에 주석이 없으므로 자연히 충족됩니다.
- 시퀀스-of-매핑은 `-`를 단독 줄에 두고 내용을 +2 들여쓰는 스타일로 기록합니다(결정론적이며
  loader가 정확히 재파싱함을 round-trip으로 확인).
- 인용 정책은 round-trip·interop 안전을 위해 보수적으로 인용합니다(숫자·키워드도 인용). 가독성
  보다 명확성·안전을 우선했습니다.

## 남은 위험 / 한계

- 매핑 키가 리터럴 `"<<"`인 경우 loader가 merge-key로 거부하므로 writer로 기록해도 재로드되지
  않습니다(매니페스트에서 `<<` 키는 비현실적이라 영향 없음).
- 매우 긴 스칼라의 줄바꿈(folded/literal block) 스타일은 지원하지 않습니다. 매니페스트 값은
  짧아 문제 없으며, 필요 시 후속에서 추가합니다.
