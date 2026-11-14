# 작업 보고서: XVIII.4.2 터미널 화면 buffer·scrollback·선택·clipboard 모델

- WBS: XVIII.4.2
- 작업명: terminal screen buffer, scrollback, selection, clipboard 모델 구현
- 상태: 완료 (`[x]`)
- 선행: XVIII.4.1 `[x]`(이 lane에서 완료, `c302b80`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/terminal/src/emulator/scrollback.rs`(신규) — `Scrollback`, `ScrollbackLimits`, `HistoryLine`, 상한 상수.
- `crates/terminal/src/emulator/selection.rs`(신규) — `Selection`, `SelectionMode`, `BufferPoint`, `WORD_SEPARATORS`.
- `crates/terminal/src/emulator/clipboard.rs`(신규) — `encode_paste`, `PasteData`, `PasteError`, `CopiedText`, `ClipboardAccess`, base64 해독.
- `crates/terminal/src/emulator/screen.rs` — `TerminalConfig`, `BufferLine`, 기록 연결, 대체 화면(47·1047·1049), ED 3, 보기 위치, 선택 API, OSC 52, `TerminalEvent::ClipboardWrite`, `Modes::alt_screen`.
- `crates/terminal/src/emulator/mod.rs` — 모듈·재노출, `Terminal::with_config`·`screen_mut`·`copy_selection`·`paste`.
- `crates/terminal/src/emulator/buffer_tests.rs`(신규, 테스트 전용).
- `docs/task-reports/XVIII.4.2.md`(신규), 계획서(XVIII.4.2 `[ ]`→`[x]`).

## 구현 요약

- **scrollback**: 주 화면에서 맨 위 줄부터 시작하는 scroll region이 올라갈 때(LF·IND·SU)와 크기
  축소로 위에서 빠지는 줄을 기록합니다. 화면 중간 region과 대체 화면의 줄은 기록하지 않습니다(xterm
  동작). ED 3은 기록을 지우고, RIS는 화면만 초기화합니다.
- **두 상한**: 줄 수(기본 10,000, 설정 최대 1,000,000)와 메모리(기본 64 MiB, 설정 최대 1 GiB).
  메모리는 줄 heap(셀 16 B·결합 문자)에 더해 ring buffer의 **할당된 칸 전체**를 셉니다. ring buffer는
  칸만으로 메모리 상한의 절반을 넘지 않게 직접 늘리고, 기록을 비워도 들어가지 않는 줄은 저장하지
  않습니다. 줄 끝 기본 빈 칸은 저장하지 않아 빈 줄은 칸 크기(40 B)만 씁니다. 상한을 줄이면 줄 수 →
  남는 칸 반환 → 메모리 순으로 바로 맞춥니다.
- **줄 번호와 보기 위치**: 기록 첫 줄부터 센 절대 번호로 화면·기록 줄을 함께 찾습니다
  (`buffer_line`, `first_line`, `screen_top_line`, `last_line`). `scroll_display`로 기록을 거슬러
  보고, 거슬러 보는 중 출력이 쌓이면 같은 내용을 계속 보이게 위치를 옮깁니다. 대체 화면에서는 기록이
  보이지 않고 위치가 움직이지 않습니다. 붙여넣기는 현재 화면으로 돌아갑니다.
- **대체 화면**: 47·1047은 화면만, 1049는 cursor 저장·복원까지 합니다. 들어갈 때 빈 화면, 나올 때 주
  화면을 그대로 되돌립니다. 대체 화면 중 크기를 줄이면 보관한 주 화면도 1049가 저장한 cursor 줄이
  보이게 맞추고, 위에서 빠진 주 화면 줄은 기록에 넣습니다.
- **선택**: Simple(흐름)·Word(공백과 `()[]{}<>'"\`|;,`로 끊음)·Line·Block(직사각형). 넓은 문자 뒤
  칸에서 시작해도 문자 전체를 넣습니다. 복사 문자열은 자동 줄바꿈 줄을 개행 없이 잇고(이어지는
  공백 보존), 그 밖의 줄 끝 공백을 뺍니다. 기록에서 버려진 줄은 선택에서 잘라 내고, 전부 버려지면
  선택이 사라집니다. 대체 화면 전환·크기 변경·RIS는 선택을 지웁니다. 그리기용 줄별 열 범위
  (`selected_columns`)를 제공합니다.
- **clipboard 모델**: 복사는 32 MiB에서 문자 경계로 자르고 `truncated`로 알립니다. 붙여넣기는
  `\r\n`·`\n`을 `\r`로 바꾸고 tab 외 제어 문자(C0·DEL·C1, ESC 포함)를 지워 bracketed paste 끝 표시
  주입을 막으며, bracketed paste mode면 `ESC [200~`·`ESC [201~`로 감쌉니다. 줄 수를 알려 UI가 여러
  줄 붙여넣기를 확인받을 수 있습니다. OSC 52는 기본 거부이고, 쓰기 허용이어도 `ClipboardWrite`
  알림만 내며 읽기 요청(`?`)은 항상 거부합니다. 시스템 clipboard 접근은 앱 셸(XVIII.4.3) 몫입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-terminal` | unit 110 passed, 0 failed / doctest 2 passed |
| `cargo test -p stellar-terminal` 10회 반복 | 실패 0, 반복 뒤 `pgrep -fl "sleep 300"` 결과 없음 |
| `cargo test -p stellar-terminal one_million` | 2 passed(6.36 s, debug) |
| `cargo fmt -p stellar-terminal -- --check` | 통과 |
| `cargo clippy -p stellar-terminal --all-targets -- -D warnings` | 경고 0 |
| `./scripts/macos/smoke.sh` | 실행하지 않음(앱 셸 변경 없음, UI 연결은 XVIII.4.3) |

| 검사 | 결과 |
|---|---|
| **100만 줄 memory cap**: 기록 직접 넣기 100만 줄(40열 내용) 1,024줄마다·마지막에 측정한 최대 메모리 ≤ 16 MiB, 버린 줄 + 남은 줄 = 1,000,000, 최신 줄 유지; 빈 줄 100만 개는 128 MiB 안에 모두 저장, 100만 1번째에서 줄 수로 버림(`one_million_lines_keep_memory_cap`) | 통과 |
| **100만 줄 memory cap(emulator 경유)**: 24×80 터미널에 출력 100만 줄, 상한 1,000,000줄·32 MiB: 측정 최대 33,554,032 B ≤ 33,554,432 B, 남은 기록 52,251줄·버린 줄 947,726줄(=1,000,000 − 화면 23줄), 최신 줄·화면 끝 줄 일치; 기본 설정은 10,000줄에서 멈춤(`one_million_output_lines_keep_scrollback_cap`) | 통과 |
| 4096열 줄도 1 MiB 상한 유지, 상한 축소 즉시 반영, 한 줄도 못 담는 상한·칸까지 더하면 넘는 상한은 저장 안 함, 상한을 줄여도 칸 때문에 줄을 더 버리지 않음(`wide_lines_respect_byte_cap_and_shrinking_limits`) | 통과 |
| 줄 끝 기본 빈 칸 미저장·배경색 빈 칸 저장(`trailing_default_blanks_are_not_stored`), 줄 수 상한·절대 번호·0줄 상한(`line_limit_evicts_oldest_and_keeps_absolute_numbers`), 상한 검증(`limits_are_validated`) | 통과 |
| 올라간 줄 순서 기록·절대 번호 조회·SU 기록·ED 3(`scrolled_lines_enter_scrollback_in_order`) | 통과 |
| 중간 region·대체 화면 줄 미기록, 1049 cursor 저장·복원, 47·1047 전환(`region_and_alternate_screen_lines_are_not_recorded`) | 통과 |
| 보기 위치 거슬러 보기·출력 중 고정·끝 정지·붙여넣기 때 복귀(`viewport_scrolls_back_and_stays_anchored`), 크기 축소 기록·대체 화면 중 축소(`shrinking_moves_top_rows_into_scrollback`) | 통과 |
| 선택 4방식 문자열·역방향·그리기 열 범위(`selection_modes_produce_expected_text`), 자동 줄바꿈 잇기·넓은 문자·기록과 화면에 걸친 선택(`wrapped_wide_and_history_spanning_selection`) | 통과 |
| 버려진 줄 따라 선택 자르기·사라짐, 대체 화면·크기 변경 때 선택 지움(`selection_follows_eviction_and_is_cleared_on_screen_switch`) | 통과 |
| 복사 상한·한글 문자 경계 자르기(`copy_is_bounded_and_marks_truncation`), 붙여넣기 개행·제어 문자 제거·bracketed(`paste_normalizes_newlines_and_strips_controls`, `paste_follows_bracketed_mode`), base64(`base64_decoding`), OSC 52 정책(`osc52_clipboard_policy`), 선택 순서·자르기(`ordering_and_clamping`) | 통과 |

## 리뷰 후 수정(`Fix:` 커밋)

- **할당자 계측으로 memory cap 독립 확인**: 위 100만 줄 검사는 crate가 스스로 세는
  `Scrollback::memory_bytes`만 봅니다. 그 계산과 따로, 테스트 바이너리 전용 계측 할당자(System 위임,
  thread별)로 같은 조건(24×80, 상한 1,000,000줄·32 MiB, 출력 100만 줄)의 heap을 재는 통합 테스트
  `crates/terminal/tests/scrollback_heap_measurement.rs`를 추가했습니다. realloc은 두 블록이 공존한다고
  보고 peak를 셉니다(보수적).

| 측정(`DEV-MAC`, `cargo test -p stellar-terminal --test scrollback_heap_measurement -- --nocapture`) | 값 |
|---|---|
| 상한 | 33,554,432 B |
| 자체 계산(`memory_bytes`, 끝) | 33,554,032 B |
| 요청 크기 heap peak(측정 중 늘어난 양) | 33,555,904 B(상한 + 1,472 B: 기록으로 옮기는 줄 대신 화면에 새로 만드는 빈 줄 등 scrollback 밖 일시 할당) |
| 요청 크기 heap 끝 | 33,554,032 B(자체 계산과 같음) |
| 실제 블록 크기 peak(macOS `malloc_size`, 크기 등급 올림 포함) | 36,064,000 B(상한의 107.5%) |
| 남은 기록 | 52,251줄 |

| 검사 | 결과 |
|---|---|
| 요청 크기 peak ≤ 상한 + 64 KiB, 끝 heap과 자체 계산 차이 ≤ 64 KiB, 실제 블록 peak ≤ 상한 + 1/8(`one_million_output_lines_stay_within_cap_by_allocator_measurement`) | 통과 |
| 검사가 실제로 잡는지: ring buffer 칸을 상한 계산에서 빼도록 임시로 고치면 요청 크기 peak 36,177,280 B로 실패(확인 뒤 되돌림) | 실패 확인 |

## 차이(작업 지시와 다른 점)

- "100만 line memory cap"은 줄 수 상한을 최대 100만 줄로 설정해 100만 줄 이상을 넣어도 메모리 상한
  (설정 값)을 넘지 않는 것으로 해석해 검사했습니다. 기본 상한(10,000줄, 64 MiB)은 이 작업에서 정한
  값이며 설정 UI는 앱 셸 작업 범위입니다.

## 남은 위험 / 한계

- 크기 변경 때 줄을 다시 흘리지 않고, 화면이 커질 때 기록 줄을 화면으로 되돌리지 않습니다.
- 메모리 상한은 요청 크기(byte)로 셉니다. 실제 사용량에는 할당자의 크기 등급 올림이 더해지며, 위
  측정(37자 줄)에서 상한의 7.5%였습니다. 줄 폭에 따라 이 비율이 달라질 수 있고 다른 폭은 재지 않았습니다.
  할당자 관리 정보(zone metadata)는 `malloc_size`에도 들어가지 않습니다.
- 화면 격자 자체(최대 4096×4096 셀)는 scrollback 상한에 들어가지 않습니다. 크기 상한
  (`TerminalSize`)이 그 상한입니다.
- 선택은 출력이 선택 영역을 덮어써도 지워지지 않습니다(내용이 바뀐 줄의 새 문자열이 복사됨).
- 시스템 clipboard 읽기·쓰기, 여러 줄 붙여넣기 확인 UI, OSC 52 허용 설정 UI는 XVIII.4.3에서 연결합니다.
