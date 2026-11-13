# 작업 보고서: XV.8.3 다중 버전 경고 경로·예상 추가 용량·URL 분리

- WBS: XV.8.3
- 작업명: duplicate library version warning에 dependency paths와 예상 추가 용량 포함, repository URL은 표시 UI에만 local source로 전달하고 telemetry에서 제외
- 상태: 완료 (`[x]`)
- 선행: XV.2.5, XV.3.5(이 lane에서 완료)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/dependencies/src/duplicate_report.rs`(신규) — `DuplicateVersionReport`,
  `DuplicateVersionDetail`, `AdditionalSize`, `DuplicateWarningUi`/`DuplicateVersionUi`,
  `DuplicateWarningTelemetry`, `duplicate_version_reports`.
- `crates/dependencies/src/lib.rs` — 모듈 등록.
- `docs/task-reports/XV.8.3.md`(신규), 계획서(XV.8.3 `[ ]`→`[x]`).

## 구현 요약

- XV.2.5 경고의 버전마다 dependency path, repository keyed identifier, privacy cache index(XV.3.5)의
  content hash·크기를 붙입니다.
- **예상 추가 용량**: 같은 content hash 버전은 공용 cache에서 중복 제거되므로(아키텍처 30.3) 고유 내용만
  세어 "고유 내용 크기 합 − 최대 고유 내용 크기"(한 버전만 쓸 때보다 더 드는 용량)로 계산합니다. index에
  없는 버전이 있으면 `LowerBound { bytes, unknown_versions }`입니다.
- **출력 분리**: `ui_view(&LocalSourceConfig)`만 local source config에서 repository URL을 찾아 버전별로
  전달합니다. `telemetry()`는 정적 이벤트 이름과 버전 수·경로 수·byte 수·크기 미상 수만 담고 URL·경로·
  이름·library id·commit을 제외합니다(아키텍처 39장). 보고서 자체도 URL 대신 keyed identifier만 가집니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-dependencies` | 55 passed, 0 failed; doctest 1 passed |
| `cargo clippy -p stellar-dependencies --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-dependencies -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| warning fixture(a→c v1 1000B, b→c v2 3000B, x→c v3 = v1과 같은 내용): 경고 1건, 버전별 경로·크기 정확, 예상 추가 용량 `Known(1000)`(`warning_fixture_has_paths_and_expected_additional_size`) | 통과 |
| UI view가 버전별로 local source URL 전달, telemetry 값 정확하고 Debug에 URL·경로·library id·commit 없음, 보고서 Debug에 URL 없음 | 통과 |
| index 누락 버전 → `LowerBound{1000, 1}`, 다른 keyer(key 불일치)로는 크기 미상(`missing_index_entry_gives_lower_bound`) | 통과 |

## 차이(작업 지시와 다른 점)

- "예상 추가 용량"의 정의(고유 내용 합 − 최대 고유 내용)를 명시적으로 정했습니다. 계획 행은 계산식을
  정하지 않습니다.

## 남은 위험 / 한계

- 크기는 cache index 기록값입니다. 아직 materialize되지 않은 버전(XV.3.3 차단)은 크기 미상으로 하한만
  보고합니다.
- Token 불일치·Component 호환성 경고는 library snapshot(XV.5, 차단) 이후 범위입니다.

## 상태 정정(통합 리뷰, 2026-11-14)

- `[x]` → `[~]`: 엄격 선행 사슬에 `[~]` 행(XV.1.1 등)이 있어 함께 `[~]`로 둡니다. 이 행 자체의 검증은 dependencies crate 안에서 통과했습니다.

## 상태 복원(deps_wiring lane, 2026-11-14)

- `[~]` → `[x]`: 엄격 선행 XV.2.5·XV.3.5 `[x]`.
- 실제 파서 경로 검증: 경고 상세(경로·예상 추가 용량·UI/telemetry 분리)는 파서와 무관하며 기존 warning fixture가 계속 통과합니다(`warning_fixture_has_paths_and_expected_additional_size`).
- 공통 근거: `Fix: stellar.yaml 의존성 선언을 실제 매니페스트 파서에 연결`(리뷰 P2), `Fix: stellar.lock.yaml entry 형식을 IV.1.6 모델 하나로 통합`(리뷰 P3, ADR-0016), `Fix: resolver library 매니페스트 읽기를 실제 stellar.yaml 파서로 연결`.
- 실행(`DEV-MAC`): `cargo test -p stellar-project` lib 261 passed·통합 17 passed, `cargo test -p stellar-dependencies` 57 passed·doctest 1 passed, `cargo clippy -p stellar-project -p stellar-dependencies --all-targets -- -D warnings` 경고 0, `cargo fmt -p stellar-project -p stellar-dependencies -- --check` 통과.
