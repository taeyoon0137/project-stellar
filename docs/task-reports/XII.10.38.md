# 작업 보고서: XII.10.38 캔버스 클립보드

- WBS: XII.10.38
- 작업명: 캔버스 Copy·Cut·Paste(Stellar 전용 형식과 Text 일반 텍스트)
- 상태: 부분 완료 (`[~]`) — 같은 문서(같은 창·다른 창) 완료, 다른 프로젝트 subtree 붙여넣기는 진단과 함께 거부(규칙 고정)
- 결정: ADR-0117 결정 9
- 선행: 계획서 행의 선행 열

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, Apple M3 Max). 다른 작업 여럿이 같은 장비에서 동시에 돌았습니다.

## 변경 파일 목록

- `crates/runtime/src/canvas/clipboard.rs`(신규), `crates/runtime/src/canvas/clipboard/tests.rs`(신규), `crates/runtime/src/canvas.rs`(mod 등록)
- `crates/runtime/src/canvas/transform.rs`(`FreshIds`가 OS 난수를 256개씩 받아 두게 함)

## 구현 요약

- 형식: Stellar 전용 type `app.stellar.canvas-subtree.v1`(봉투 `CanvasClipboard`: header, 출처 프로젝트·문서·View, root별 원본 id·원래 부모·world 경계·기존 `ClipboardSubtree` bytes, 참조한 Component 정의·asset 목록)과 일반 텍스트(선택 subtree의 Text 내용을 문서 순서로 줄바꿈 이어 붙임). 봉투는 16 MiB·root 4,096개 상한, 남는 bytes 거부, subtree는 closed command codec으로만 해석합니다.
- Copy는 다른 선택의 자손을 뺀 root(선택 순서)를 담고 잠긴 node도 복사합니다. Cut은 `DeleteSubtree`들을 한 항목으로 기록하고, 잠긴 root·자손이 있으면 전체를 거부합니다(`commands::lock`).
- Paste 위치: 선택이 잠기지 않은 Frame 하나이고 복사한 root 자신이 아니면 그 안(Copy 직후 Paste가 자기 안에 넣지 않음) → 같은 View이고 원본 경계가 화면에 보이며 모든 root의 원래 부모가 남아 있고 편집 가능하면 같은 부모·같은 위치 → 아니면 View root 아래 화면 가운데. 가운데 이동은 배치 쓰기 한 곳(`place::translate`)으로 계획합니다(문서를 복제하지 않고 root record만 담은 임시 모델에서 계획). View root가 흐름 컨테이너면 흐름이 자리를 정하므로 옮기지 않습니다.
- 새 ID는 `ClipboardSubtree::plan_with`(IdRemapTable)로 받고, 붙여 넣은 root 전체를 새 선택으로 `apply_and_select` 한 항목에 기록합니다.
- 다른 문서 규칙: `cross-document`로 거부하고, 대상에 없는 Component 정의(`component-missing`)·asset(`asset-missing`) 진단을 함께 냅니다. 같은 문서에서도 참조 진단은 결과에 담습니다.
- 성능: `FreshIds`가 ID마다 `/dev/urandom`을 열던 비용(1,000 node 붙여넣기에서 13ms)을 묶음 받기로 없앴습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-runtime --lib canvas::clipboard` | 통과(6): Copy 직후 Paste가 같은 자리(자기 안 아님), 원래 부모를 지운 뒤 화면 가운데, 두 형식·봉투 왕복·형식 거부, 같은 창 paste의 세 자리 규칙(선택 컨테이너 안·같은 자리·화면 가운데)과 중복 StableId 0·undo 한 번, 다른 창 paste와 다른 프로젝트 거부(history·revision 불변)·참조 진단 코드, Cut 한 항목과 잠긴 root 거부 |
| `cargo test --release -p stellar-runtime --lib canvas::clipboard::tests::measure_large_paste -- --ignored --nocapture` | 통과: 1,001 node subtree 붙여넣기 코어 처리 20 sample p50 2.10ms, 최대 2.66ms(기준 10ms) |
| `cargo test -p stellar-runtime --lib canvas::` | 통과(135, ignored 4) |

## 차이(작업 지시와 다른 점)

- "다른 프로젝트 paste fixture"는 지금 `ClipboardSubtree`가 같은 문서 subtree만 받아(commands, 소유 밖) 거부 규칙과 진단으로 고정했습니다. 참조 재연결과 asset 복사를 하는 다른 문서 붙여넣기는 `ClipboardSubtree` v2와 asset import(XIII.5.2)가 생기면 같은 진단 목록으로 엽니다.
- 측정은 화면 그리기를 뺀 코어 처리(봉투 해석·ID 재발급·기록·새 선택)입니다. 최초 시각 반응(Present)은 셸 연결 뒤 다시 재야 합니다.
- 앞선 셸 경로 `app::node_paste`(단일 선택 subtree)는 소유 밖이라 그대로 두었고 새 UI 연결 때 이 module로 바꿉니다.

## 남은 위험 / 한계

- 일반 텍스트만 있는 pasteboard를 Text node로 붙이는 일은 셸이 Text 생성 경로(XII.10.33)로 합니다.
