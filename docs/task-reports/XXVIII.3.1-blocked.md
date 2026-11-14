# 차단 보고서: XXVIII.3.1 공개 format version 동결과 호환성 governance

## WBS

- WBS: XXVIII.3.1
- 작업명: 공개 format version을 동결하고 이후 incompatible change는 major format bump와 migration을 요구하도록
  governance를 적용
- 상태: `[!]` — 사용자 결정과 작업 범위 차단(계획 §1.1 정의). 착수하지 않았습니다.
- 선행: XXVIII.2.4(기준 commit에서 `[x]`). 명시 선행은 충족했지만 아래 조건 때문에 진행할 수 없습니다.

## 실패 선행조건

- **공개 version 동결은 product owner 결정입니다.** 어떤 형식 버전(chunk·manifest format version 1, storage
  스키마 `LATEST_SCHEMA` = 2, 문서 chunk 스키마 1, `stellar.yaml`·`stellar.lock.yaml` 스키마 1)을 "공개 버전"으로
  묶어 동결할지, major·minor 번호 체계를 어디에 기록할지 정한 ADR이 없습니다. 호환성 명세
  (`docs/spec/stellar-format-compatibility.md`) §8의 "정하지 않은 사항" 1·2·5번(최소 reader·writer 버전 기록
  위치, 열기 경로의 판정 적용 시점, 공개 형식 버전 동결)이 그대로 남아 있습니다. 아키텍처 결정은 ADR로 기록해야
  하는데 ADR 경로(`docs/adr/`)는 이 lane의 허용 경로(`docs/spec/**`) 밖입니다.
- **검증 기준이 CI 변경을 요구합니다.** "breaking fixture without bump CI 실패"는 형식 변경을 감지해 버전 bump가
  없으면 실패하는 검사(xtask 또는 CI step)가 필요합니다. `tools/xtask`와 CI 파일은 이 lane에서 수정할 수 없습니다.
- **공개 전 조건이 충족되지 않았습니다.** 기술 아키텍처 §35의 공개 전 조건 중 형식 안정화 보고서(XXVIII.1.1 `[ ]`,
  두 major app version 사용 데이터 필요), conformance runner(XXVIII.2.2 `[ ]`), corpus 라이선스(XXVIII.2.1 `[~]`,
  라이선스 미결정)가 없습니다.
- 계획 §1.2 규칙에 따라 코드와 CI는 수정하지 않았습니다.

## 재현 명령

- `grep -n "| XXVIII.1.1 \|| XXVIII.2.1 \|| XXVIII.2.2 \|| XXVIII.3.1 " docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md`:
  관련 행 상태 확인.
- `ls docs/adr/`: 공개 형식 버전 동결 ADR이 없음을 확인.
- 차단이 풀린 뒤 검증 후보:
  - `cargo xtask test-migrations`와 golden corpus(`docs/spec/conformance/manifest.yaml`)를 기준으로, 형식 상수
    (`CHUNK_FORMAT_VERSION`, `MANIFEST_FORMAT_VERSION`, `LATEST_SCHEMA` 등)나 golden bytes가 바뀌었는데 버전 bump와
    migration fixture가 없으면 실패하는 검사.
  - 일부러 golden bytes만 바꾼 breaking fixture로 그 검사가 실패하는지 확인.

## 필요한 결정과 환경

- product owner 결정: 공개 형식 버전의 범위와 번호 체계, 동결 시점, major bump 기준, 최소 reader·writer 버전 기록
  위치(ADR-0011 "정하지 않은 사항")를 ADR로 확정.
- product owner 결정: corpus·명세 공개 라이선스(XXVIII.2.1).
- 작업 범위: `tools/xtask` 검사와 CI step을 수정할 수 있는 lane.
- 환경: `DEV-MAC`이면 충분합니다(순수 Rust·문서 검사).

## 변경되지 않은 파일

- `docs/adr/`(동결 ADR 없음), `tools/xtask/`, `.github/workflows/`, `docs/spec/stellar-format-compatibility.md`의
  §8 "정하지 않은 사항".

## 재개 조건

- 위 결정이 Accepted ADR로 기록되고, 형식 변경 감지 검사를 xtask·CI에 넣을 수 있는 작업 범위가 주어지면 재개합니다.
- 그 검사가 breaking fixture(버전 bump 없는 golden bytes 변경)에서 실패하고 정상 상태에서 통과하면 `[x]`입니다.
