# 저장된 단일 View의 고정 Frame 웹 내보내기 검증

- 날짜: 2026-11-14
- 범위: `stellar export-web <프로젝트> <문서 id> <View id>` → 독립 HTML+CSS
- checkout: `project-stellar-takeover-web-frame`, branch `stellar-takeover-web-frame`
- 상태: 아래 제한 부분집합의 제품 경로 구현·검증. 전체 코드 내보내기 WBS 완료로 변경하지 않음.

## 변경

- `crates/ops/src/web_frame.rs`: 열린 저장 revision에서 명시한 View를 선택하고 실제 Frame/Fragment 값만 투영.
  미지원 node·layout·modifier·clip·record·profile·숫자 범위는 node ID 진단과 HTML 없음으로 실패.
- `crates/codegen/src/web/frame.rs`: 기존 core 값의 고정 Frame CSS formatter. border-box·부모 border
  좌표 보정·matrix/origin·개별 radius·색공간/RGBA·clip·stacking을 명시. 범용 IR·저장 스키마 없음.
- `tools/stellar-cli/src/web_frame.rs` 및 명령 registry·도움말: 일반 CLI 공개 명령과 결과 envelope 연결.
- `crates/ops/src/test_support/web_frame.rs`, CLI 테스트와 `tests/fixtures/web/`: 실제 저장 두 View,
  HTML golden, 실패·바이트 불변·Chrome DOM 검증.
- [지원 명세](../spec/web-frame-export.md), CLI 안내, ADR-0049 후속 경계 기록.

기존 `export-accessibility`의 구현·HTML golden, property bundle·variant·longhand 스키마,
`property_model.md`, `frame.rs`, 기본 저장·text-source 경로는 변경하지 않았습니다.

## 실행 검증

모든 Cargo 실행은 `CARGO_TARGET_DIR="$PWD/target/web-frame"`를 사용했습니다.

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-codegen -p stellar-ops -p stellar-cli` | 160 passed, 0 failed. Chrome 테스트 1개는 이 실행에서 ignored이며 아래에서 별도 실행 |
| `STELLAR_CHROME='/Applications/Google Chrome.app/Contents/MacOS/Google Chrome' STELLAR_WEB_EVIDENCE_DIR="$PWD/target/web-frame/evidence" cargo test -p stellar-cli --test web_frame -- --include-ignored --nocapture` | Chrome 포함 5 passed, 0 failed. 두 View 모두 DOM probe `ok: true` |
| `cargo clippy -p stellar-codegen -p stellar-ops -p stellar-cli --all-targets -- -D warnings` | 통과 |
| `cargo xtask validate-crate-graph` | 통과, 추가 crate/의존 간선 없음 |
| 변경 Rust 파일의 `rustfmt --edition 2021 --check --config skip_children=true` | 통과 |
| `./target/web-frame/debug/stellar --help` | 실제 실행 결과로 help golden 갱신, 기존 headless help 테스트 통과 |
| `git diff --check` | 통과 |
| `git status --short`, `ls -l CLAUDE.md` | 전용 작업 파일만 변경, CLAUDE.md → AGENTS.md 유지 |

초기 테스트에서 새 명령이 반영되기 전 help golden 불일치와 Text fixture의 금지된 payload-kind 교체가
실패했습니다. 실제 help 재생성과 fixture의 leaf 삭제·삽입으로 수정한 뒤 위 테스트를 통과했습니다.

Chrome 버전은 **Google Chrome 157.0.8103.51**, DPR 1, `--window-size=1000,800`로 실행했습니다.
검증 스크립트는 테스트 HTML에만 주입했고 제품 HTML에는 script가 없습니다.

| 실제 DOM 항목 | 관측 결과 `(x, y, width, height)` |
|---|---|
| View A root: 저장 tx=700, ty=-900, size=640×400, border=4 | `(0, 0, 640, 400)` |
| Fragment 두 단계 아래 card: 32,48; border=3 | `(32, 48, 240, 160)` |
| card 안 bar: 저장 10,12; border=1 | `(42, 60, 80, 24)` |
| View B root: tx=-500, ty=800, scale=1.25×1.5, size=240×160 | `(0, 0, 300, 240)` |
| View B 90° 회전 | `(100, 37.5, 25, 60)` |
| View B 좌우 반전 | `(187.5, 150, 75, 45)` |
| View B shear | `(25, 150, 65, 51)` |

computed style로 네 변의 border width/color, 각 radius, `box-sizing`, `transform-origin`, matrix,
sRGB/P3·alpha와 확장 RGB `color(display-p3 1.2 -0.1 0.3 / 0.25)` 보존을 확인했습니다.
`elementFromPoint`로 OrderKey 순서의 겹침과 border box 안쪽/바깥쪽 clipping을 확인했습니다.
Fragment 자체는 DOM에 없고 숨겨진 Fragment의 Frame은 `display: none`입니다.

반복 CLI stdout/stderr가 byte 동일하며 다른 View가 섞이지 않습니다. 성공·실패 후 프로젝트 파일과
Git index·refs·objects의 해시, HEAD, status가 동일합니다. 열린 snapshot은 다음 저장 후에도 같은
revision을 읽고, root 이동만 ±f32::MAX로 바꿔 저장해도 출력 HTML은 동일합니다.

전체 로그와 실행 증거는 전용 target의 `test.log`, `clippy.log`, `crate-graph.log`, `browser.log`,
`evidence/view-{a,b}.html`, `evidence/view-{a,b}.dom.html`, `evidence/view-{a,b}.computed.json`,
`evidence/chrome-version.txt`에 있습니다. target 산출물은 Git에 추가하지 않습니다.

## 한계와 다음 단계

- 저장된 고정 Free Frame·Fragment만 지원합니다. Text·반응형 layout·control·modifier는 오류이며,
  해석되지 않는 project record/profile도 선택 root 진단으로 보수적으로 실패합니다.
- 단색의 색공간·RGBA 선언을 보존하지만 브라우저 합성·gamut mapping·래스터 픽셀까지 Stellar와 같다는
  보장은 없습니다. 부분 투명 색은 경고합니다. 숫자와 정수 px border 범위는 명세에 고정했습니다.
- 다른 브라우저·DPR·zoom의 적합성, 전체 앱 내보내기, Text 스타일 저장, 실제 capability pin 연결,
  ADR-0059 기본 텍스트 원본 전환은 검증·구현 범위 밖입니다.
- 다음 단계는 실제 layout/Text 영속 record와 capability 해석 경로가 마련된 뒤 해당 입력을 연결하고,
  각 부분집합의 브라우저 fixture를 추가하는 것입니다. 현재 스키마를 추측해 선행 정의하지 않습니다.
- 원격 push·release·서명·추가 작업자 실행을 하지 않았고 다른 checkout에 쓰지 않았습니다.
