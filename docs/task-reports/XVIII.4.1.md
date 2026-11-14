# 작업 보고서: XVIII.4.1 terminal parser/emulator와 VT sequence corpus

- WBS: XVIII.4.1
- 작업명: 승인된 terminal parser/emulator dependency를 등록하고 VT sequence corpus를 테스트
- 상태: 완료 (`[x]`)
- 선행: I.3.2 `[x]`, XVIII.2.1 `[x]`, XVIII.3.1 `[!]`(Windows 보류, ADR-0006. 작업 지시에 따라 충족으로 봄)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + macOS PTY 통합 테스트, 로컬 검증).

## 변경 파일 목록

- `crates/terminal/src/vt/mod.rs`(신규) — VT 해석 모듈, 외부 dependency를 쓰지 않는 이유.
- `crates/terminal/src/vt/parser.rs`(신규) — `Parser`, `Perform`, `Params`/`ParamGroups`, 상한 상수.
- `crates/terminal/src/vt/width.rs`(신규) — `char_width`, `unicode_version`.
- `crates/terminal/src/vt/width_table.rs`(신규, 생성 파일) — 셀 폭 0·2 범위 표(Unicode 17.0.0).
- `crates/terminal/tools/gen_width.py`(신규) — 위 표 생성기.
- `crates/terminal/src/emulator/mod.rs`(신규) — `Terminal`(parser + 화면), `TERM_NAME`, `COLORTERM`.
- `crates/terminal/src/emulator/cell.rs`(신규) — `Cell`, `Style`, `Color`, `Attrs`, `CellWidth`.
- `crates/terminal/src/emulator/screen.rs`(신규) — `Screen`, `Row`, `Modes`, `CursorState`, `TerminalEvent`.
- `crates/terminal/src/emulator/corpus.rs`(신규, 테스트 전용) — VT sequence corpus.
- `crates/terminal/src/registry.rs` — child 환경 `TERM`·`COLORTERM` 기본값.
- `crates/terminal/src/lib.rs` — `vt`, `emulator` 모듈.
- `docs/task-reports/XVIII.4.1.md`(신규), 계획서(XVIII.4.1 `[ ]`→`[x]`).

## 구현 요약

- **parser**(`vt::parser`): 공개된 DEC ANSI 호환 parser 상태 기계(ground·escape·CSI·DCS·OSC·
  SOS/PM/APC 14개 상태)를 crate 안에서 구현했습니다. 동작은 `print`(UTF-8 해독, 잘못된 입력은 최대
  부분마다 U+FFFD), `execute`(C0), `csi_dispatch`(`;` group과 `:` subparameter, private marker),
  `esc_dispatch`, `osc_dispatch`(BEL·ST 종료), DCS `hook`/`put`/`unhook`으로 나눕니다. CAN·SUB는
  sequence를 버리고, ESC는 진행 중인 sequence를 끊습니다. 입력을 어떻게 나눠 넣어도 결과가 같습니다.
- **상한**: 매개변수 32개(값은 `u16` 포화), 중간 byte 2개(넘으면 `ignored` 표시로 실행 안 함), OSC
  8 KiB(넘으면 그 OSC 버림), 응답 4 KiB, 알림 256개, 제목 256자, 셀당 결합 문자 4개, REP 반복은 화면
  크기까지. 8-bit C1은 UTF-8 연속 byte와 겹치므로 해석하지 않습니다.
- **셀 폭**(`vt::width`): `tools/gen_width.py`가 Python `unicodedata`(Unicode 17.0.0)에서 폭 0(Mn·Me·
  Cf 일부, Hangul 중성·종성, skin tone modifier)과 폭 2(East Asian Width W·F) 범위 표를 생성합니다.
  재생성 결과가 커밋한 표와 같음을 확인했습니다(아래).
- **emulator**(`emulator`): `Terminal::feed`가 화면 격자를 갱신합니다. cursor 이동 전체(CUP·HVP·CUU·
  CUD·CUF·CUB·CNL·CPL·CHA·HPA·HPR·VPA·VPR·HT·CHT·CBT·HTS·TBC·IND·NEL·RI), DECSC/DECRC·SCOSC/SCORC,
  DECOM, DECAWM과 마지막 열 지연 줄바꿈(`xenl`), SGR(16·밝은 16·256·24-bit, `;`·`:` 형식, 속성
  켜기·끄기), ED·EL·ECH·ICH·DCH·IL·DL·SU·SD·REP, DECSTBM, IRM·LNM, 지운 칸의 현재 배경색(`bce`),
  넓은 문자 짝 보정, 결합 문자(NFD 한글 포함), DEC Special Graphics(SO/SI 포함), DSR 5·6·`?6`, DA1·DA2,
  DECSCUSR, mode 기록(DECCKM·DECKPAM·DECTCEM·bracketed paste·focus·mouse·synchronized output),
  RIS·DECSTR·DECALN, 크기 변경(줄 다시 흘리기 없음)을 지원합니다. BEL·OSC 0/2 제목·OSC 7 작업
  디렉터리는 `TerminalEvent`로 내며, OSC 7 경로는 XVIII.4.5 header의 현재 cwd 관찰값으로 쓸 수
  있게 절대 경로 문법을 검사합니다.
- **child가 보는 `TERM`**: registry가 profile에 지정이 없으면 `TERM=xterm-256color`,
  `COLORTERM=truecolor`를 넣습니다(앱 프로세스에서 상속한 값보다 우선, profile 지정은 유지). 지원
  sequence가 macOS terminfo `xterm-256color`의 기본 기능과 맞는지 실제 `tput` 출력으로 확인했습니다.
- 출력 내용을 제품 상태로 해석하지 않고, CLI별 출력 parser를 두지 않습니다(기술 아키텍처 31.9).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-terminal` | unit 91 passed, 0 failed / doctest 2 passed |
| `cargo test -p stellar-terminal` 10회 반복 | 실패 0, 반복 뒤 `pgrep -fl "sleep 300"` 결과 없음 |
| `cargo fmt -p stellar-terminal -- --check` | 통과 |
| `cargo clippy -p stellar-terminal --all-targets -- -D warnings` | 경고 0 |
| `python3 crates/terminal/tools/gen_width.py \| diff - crates/terminal/src/vt/width_table.rs` | 차이 없음 |
| `./scripts/macos/smoke.sh` | 실행하지 않음(앱 셸 변경 없음, UI 연결은 XVIII.4.3) |

| 검사 | 결과 |
|---|---|
| **cursor** fixture: 절대·상대 이동과 가장자리 정지, 0·생략 매개변수=1, tab stop 설정·해제, DECSC/DECRC·SCOSC/SCORC(위치·pen·문자 집합), DECOM 좌표와 CPR(`cursor_positioning_and_relative_moves`, `save_restore_and_origin_mode`) | 통과 |
| **color** fixture: 16·밝은 16·256·24-bit(`;`, `:`, 색 공간 칸 있음·없음), 속성 9종 켜기·끄기, 4:0·4:2·21, 39·49, 범위 밖·잘린 색 무시(`sgr_colors_and_attributes`) | 통과 |
| **erase** fixture: EL 0·1·2, ED 0·1·2(cursor 유지), ECH, 지운 칸 배경색(bce)(`erase_in_line_display_and_characters`) | 통과 |
| 줄바꿈·지연 줄바꿈·DECAWM 끔·스크롤·LNM(`text_crlf_autowrap_and_scrolling`), ICH·DCH·IL·DL·SU·SD·DECSTBM·RI·IRM·REP·DECALN(`insert_delete_scroll_and_regions`) | 통과 |
| 넓은 문자 줄바꿈·반쪽 덮어쓰기·지우기 경계, NFD 한글, 결합 악센트, emoji(`wide_and_combining_characters`), DEC 선 문자(`dec_special_graphics_line_drawing`) | 통과 |
| DA1·DA2·DSR 응답, BEL·제목(BEL·ST)·OSC 7 알림과 잘못된 OSC 7 거부, 응답·알림·제목 상한(`replies_events_and_bounds`), mode 기록(`modes_are_tracked`), RIS·DECSTR(`full_and_soft_reset`), 크기 변경(`resize_keeps_cursor_row_and_cuts_wide_cells`) | 통과 |
| 위 모든 corpus 입력: 한 번에 넣은 결과 = 한 byte씩 넣은 결과(`run` 도우미) | 통과 |
| 512 KiB 치우친 무작위 입력·무작위 chunk·무작위 resize: panic 없음, cursor 범위 안, 응답·알림 상한, 넓은 문자 짝 유지(`random_input_never_panics_and_stays_bounded`) | 통과 |
| 실제 PTY + `TERM=xterm-256color` `tput cup/setaf/setab/sgr0/el/bold` 출력 해석(`pty_output_with_xterm_256color_terminfo`) | 통과 |
| parser 상태 기계: 매개변수·subparameter·marker·포화, 상한 초과 ignored, C0 실행·CAN 중단·ESC 재시작, OSC BEL/ST·상한·CAN, DCS passthrough, SOS/PM/APC 버림, UTF-8 해독과 U+FFFD, 한 byte씩 넣기 동일(`vt::parser::tests` 7개) | 통과 |
| 셀 폭 표: 한글·CJK·전각·emoji 2, 결합 부호·NFD 중성/종성·ZWJ·VS16 0, 표 정렬·서로소(`vt::width::tests` 2개) | 통과 |
| registry 시작 요청 환경에 `COLORTERM`·`TERM` 기본값, profile 지정 `TERM` 유지(`trusted_start_builds_request_and_close_terminates_child`, `missing_shell_is_reported_without_spawning`) | 통과 |

## 차이(작업 지시와 다른 점)

- **외부 parser/emulator dependency를 등록하지 않고 crate 안에서 구현했습니다.** 이유:
  ADR-0019 결정 2가 `stellar-terminal`의 의존을 `core-types`만 허용하고 crate 테스트
  (`terminal_state_has_no_persistence_dependency`)가 이를 고정합니다. 공개된 상태 기계 명세와
  Unicode 데이터만으로 구현할 수 있고, 입력 크기 비례 상한을 crate가 직접 보장해야 했습니다. 그래서
  `config/dependencies.yaml`·`Cargo.lock`·ADR 변경이 없으며, 새 결정이 아니라 기존 ADR-0019를 따른
  것이라 ADR을 추가하지 않았습니다. "등록할 dependency"가 없으므로 승인 목록의 direct dependency
  일치 조건(I.3.2)은 그대로 유지됩니다.
- 셀 폭 표는 생성 파일이므로 생성기(`crates/terminal/tools/gen_width.py`)를 함께 두었습니다. 표는
  Python 3.15.2의 `unicodedata`(Unicode 17.0.0)로 만들었습니다.
- 작업 행에 없던 `TERM`·`COLORTERM` 기본값을 registry에 넣었습니다. emulator가 해석하는 sequence
  집합을 child에 알리는 값이라 이 작업의 일부로 봤습니다.

## 남은 위험 / 한계

- 셀 폭은 code point 단위입니다. emoji ZWJ 연쇄·국기 같은 grapheme cluster는 실제 표시 폭보다 넓게
  잡힐 수 있습니다(일반 터미널과 같은 한계).
- 크기 변경 때 줄을 다시 흘리지 않습니다. alternate screen·scrollback·ED 3은 XVIII.4.2에서 다룹니다.
- OSC 4·10·11·12 색 질의, OSC 8 hyperlink, OSC 52 clipboard, DCS 기능(DECRQSS·sixel)은 해석하지
  않습니다(무시). OSC 52 정책은 XVIII.4.2 clipboard 모델에서 정합니다.
- 실제 화면 표시와 입력 연결(XVIII.4.3)은 이 작업 범위가 아닙니다. `Terminal::feed`는 main thread가
  아니라 출력 reader 쪽에서 불러야 합니다(모듈 문서).
