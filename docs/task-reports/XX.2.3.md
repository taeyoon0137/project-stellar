# 작업 보고서: XX.2.3 PNG encoding worker와 temp+atomic rename 저장

- WBS: XX.2.3
- 작업명: PNG encoding을 worker에서 실행하고 temp+atomic rename으로 저장
- 상태: 부분 완료 (`[~]`) — worker 실행·취소·atomic 저장과 cancel/failure 검사는 끝났습니다. 선행 XX.2.1이 `[~]`입니다(계획
  1.2 규칙 4). CLI `stellar export png`와 MCP 도구는 연결하지 않았습니다(아래 한계).
- 선행: V.2.5 (`[x]`), XX.2.1 (`[~]`).
- 결정: ADR-0103 결정 6·8.

## 필수 실행 환경

- `DEV-MAC` (순수 Rust).

## 변경 파일 목록

- `crates/export/src/png/job.rs`(신규): `PngJob`(worker thread `stellar-export-png`), `render_png`, `PngFile`,
  `PngExportError`, `GamutSummary`.
- `crates/export/src/png/mod.rs`: 모듈 등록·재노출.
- `crates/export/src/png/tests/job.rs`(신규).
- `crates/ops/src/export_png.rs`(신규): `PngExport`(start → finish), `PngSaveOutcome`, `PngSaveError`.
- `crates/ops/src/export_png/tests.rs`(신규).
- `crates/ops/src/lib.rs`, `crates/ops/Cargo.toml`, `Cargo.lock`: 모듈 등록, export 의존, scene dev 의존.

## 구현 요약

- **worker**: `PngJob::spawn`이 렌더 → 출력 계약 검사 → encode를 이름 있는 thread에서 실행합니다. 취소는 draw 사이, encode
  행 사이, 압축 64 KiB마다 확인하고 취소된 job은 bytes를 돌려주지 않습니다. 결과가 나온 뒤 취소해도 취소로 봅니다.
- **저장**: `PngExport::finish`가 storage `write_file_atomic_observed`(같은 디렉터리 `O_EXCL` temp → 기록 → flush → fsync →
  rename → 부모 sync)로 요청 출력 경로에 씁니다. 쓰기 단계마다 취소를 확인하고, 취소·실패 시 storage가 이 호출의 temp를
  지우며 최종 경로는 이전 상태입니다. rename 뒤(부모 sync)에는 취소하지 않습니다(이미 게시됨). 출력 디렉터리는 만들지 않습니다.
- **결과**: 경로·byte 수·크기·profile과 gamut 요약(출력 profile로 옮길 때 잘린 색·확장 범위 색, renderer
  `OutputColorPlan::convert` 판정)을 돌려줍니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-ops --offline --lib export_png` | 통과(6) |
| `cargo test -p stellar-export --offline --lib png` | 통과 |

| 검사(테스트) | 결과 |
|---|---|
| 저장 성공: 해석 가능한 PNG, 디렉터리에 최종 파일만 남음, 같은 경로 재내보내기는 원자 교체(`saves_decodable_png_without_leftovers`) | 통과 |
| 저장 전 취소: 파일·temp 없음(`cancel_before_save_writes_nothing`) | 통과 |
| 기록·flush·fsync 단계 중 취소: 이전 파일 bytes 유지, temp 정리(`cancel_during_write_leaves_previous_file`) | 통과 |
| temp 생성·기록·flush·fsync·rename 직전 강제 중단: 최종 경로 없음, temp 정리(`failure_at_any_stage_before_rename_keeps_final_path`) | 통과 |
| 없는 출력 디렉터리는 쓰기 오류, 디렉터리를 만들지 않음(`missing_directory_is_a_write_error`) | 통과 |
| SVG 요청은 worker를 만들지 않고 거부(`non_png_requests_are_rejected_without_writing`, `job_rejects_wrong_format_and_honours_cancel`) | 통과 |
| worker 결과와 호출 thread 결과가 같음, P3 색을 sRGB로 내보내면 잘린 색 1개·node 보고(`worker_job_renders_encodes_and_reports_gamut`) | 통과 |

## 차이(작업 지시와 다른 점)

- 저장(파일 시스템 정책)은 storage를 쓰는 ops에 두고, export crate는 bytes까지 만듭니다(같은 atomic 쓰기 구현을 따로 두지
  않음).

## 남은 위험 / 한계

- **CLI·MCP 미연결**: 저장 문서에서 scene을 만드는 canvas scene source가 앱 runtime에만 있어 ops·CLI가 scene을 만들 수
  없습니다. 동작하지 않는 명령은 등록하지 않았습니다(ADR-0103 결정 8). scene source가 ops가 쓸 수 있는 crate로 옮겨지면
  `stellar export png`와 MCP 도구는 capture → 사전 진단 → `PngExport`만 부르면 됩니다.
- 앱 export 명령·진행 표시(XX.5.1)는 UI 재설계 때 연결합니다.
