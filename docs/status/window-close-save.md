# 정상 창 닫기 저장 보존 — XIII.1.6 부분 근거

- 기준: `7f046069`, 독립 트리 `project-stellar-takeover-close-save`, 2026-11-14 DEV-MAC.
- 구현 계획의 XIII.1.6은 `[ ]` 유지. 선행 XIII.1.5가 미구현이고 앱 비활성화 flush도 없으므로 전체 구현이나
  선행 충족을 주장하지 않습니다. XIII.1.1은 config 수치 미확정 `[~]`, XIII.1.2는 완료,
  XIII.1.3~.5는 `[ ]`입니다. 이 기록은 정상 창 닫기의 제한된 correctness 증거입니다.

## 구현과 진입점

일반 앱은 `main.swift → AppDelegate → ShellAppCoordinator`를 사용합니다. 닫기 버튼, `performClose`,
메뉴의 Close Project, 프레임의 Close Project가 `ShellWindowController.windowShouldClose`로 들어옵니다.
원래 창 활성화/열기 중복 방지/폴더 선택 취소/하단 도구 막대/user preference 정책은 유지합니다.

- clean 창은 저장 요청 없이 즉시 닫습니다. 선택·viewport와 선택 history만 있는 창도 clean입니다.
- dirty 창과 미완료 명령이 있는 창은 실제 NSWindow와 Rust session을 유지합니다. 미완료 명령 결과를 먼저
  반영하고 dirty일 때만 기존 비동기 Save Now를 요청합니다. 성공 event와 최신 clean 상태를 확인한 뒤 닫습니다.
- 쓰기 오류·stale·접수 실패·상태 조회 실패·timeout에는 원래 창과 편집을 보존하고 실패 sheet를 표시합니다.
  원인을 해결한 후 다시 닫아 재시도할 수 있습니다. stale 강제 덮어쓰기/디스크 재로드 선택 UI(XII.7)는 추가하지 않습니다.
- 같은 창의 중복 닫기·새 편집 명령은 대기 중 추가 접수하지 않습니다. 닫힘·session 교체·취소·timeout 이후
  늦은 완료는 자동 닫기나 재오픈을 하지 않습니다. 기존 열기 2단계의 늦은 callback 폐기 처리는 보존합니다.
- ABI version 2는 기존 88 byte 안의 reserved를 dirty 값으로 사용합니다. 생성기·헤더·offset 검사·ADR을 함께 갱신합니다.
- 현재 runtime은 `WindowHistory`와 `saved_revision`을 사용합니다. XIII.1.2의 별도 `DirtyTrackedHistory`를
  이번 변경에서 연결하지 않았습니다. 기존 `apply`·문서 Undo/Redo의 revision만 저장 대상으로 판단하며,
  preview 캡처와 worker 저장 중 revision 판정은 autosave 선행 구현 범위입니다.

## 변경 파일

- `apps/macos/StellarMac/UI/Shell/ShellProjectWindow.swift`, `ShellAppCoordinator.swift`: 닫기 보류·실패 보존·재시도.
- `crates/runtime/src/window_session.rs`, `app.rs`, `app_window_tests.rs`: O(1) dirty snapshot과 검사.
- `crates/ffi/src/window_session.rs`, `tools/xtask/src/ffi_header.rs`, `crates/ffi/include/stellar_ffi.h`: ABI·Debug 검사 bridge.
- `apps/macos/StellarMac/CoreRuntime.swift`, `UI/Shell/ShellRuntimeEventPump.swift`,
  `UI/Shell/ShellWindowCloseCheck.swift`, `main.swift`, `scripts/macos/smoke.sh`: 실제 AppKit→Rust 통합 검사.
- ADR-0028, 이 보고서, XII.6.4 보고서, UI shell 문서: 계약·증거·한계.

## 검증

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-runtime -p stellar-ffi` | 종료 0. library runtime 88·ffi 70, 통합·doc test 모두 통과 |
| `cargo clippy -p stellar-runtime -p stellar-ffi -p xtask --all-targets -- -D warnings` | 종료 0 |
| `cargo fmt -p stellar-runtime -p stellar-ffi -p xtask -- --check` | 종료 0 |
| `cargo xtask generate-ffi-header` / `cargo xtask generate-ffi-header --check` | 원본으로 재생성, drift 없음 |
| `cargo xtask validate-adr crates/ffi/src/window_session.rs docs/adr/ADR-0028-window-session-shell-surface.md` | 종료 0 |
| `cargo xtask validate-task-status` / `cargo xtask sync-plan --check` | 종료 0. WBS 상태 변경 없음 |
| Debug `xcodebuild … CODE_SIGNING_ALLOWED=NO build` | 종료 0. Swift warning 0. AppIntents 미사용 metadata 경고 1건 |
| `StellarMac --window-close-check crates/runtime/tests/fixtures/window-session-project` | 종료 0, 87 checks |
| `scripts/macos/smoke.sh` | 종료 0, `smoke OK` |
| C ABI `cc … runtime_abi.c … libstellar_ffi.a` 후 실행 | 종료 0, 1,000 create/destroy cycle·0 leaks·session events |
| `git diff --check` / `git status --short` | 공백 오류 없음, 요청 범위의 변경 파일만 확인 |

macOS smoke는 기존 shell 통합 120, project-open 177, window-session 68, session-commands 56,
key-mapping 101 checks와 새 window-close 87 checks를 포함합니다. 기존 열기 중 1·2단계 닫기와
late callback 폐기, 메뉴·도구·하단 막대 및 user preference 보존 검사가 통과했습니다.

새 검사는 실제 `NSWindow.performClose → windowShouldClose → 기존 Save Now C ABI → Rust 저장`을
사용합니다. Debug 편집 bridge와 기존 `ShellSavedProjectFixture`·`ShellCheckAutosave`를 이용합니다.
clean/선택 전용 즉시 닫기, 접수됐지만 아직 미반영인 편집, 중복 닫기, 대기 중 새 Undo 거부,
저장 후 디스크에서 재열어 편집 이름 검증, 실제 objects 경로 쓰기 오류와 복구 후 재시도,
동일 프로젝트 독립 sibling session의 선행 저장과 stale 창의 메모리·디스크 보존을 확인합니다.
닫기 직전 이미 접수된 Save Now의 실패도 닫기를 취소하고 안내 한 번으로 끝나는지 확인합니다.
미완료 편집 timeout과 실제 Save Now 응답 timeout을 각각 검사하고, timeout 뒤 늦은 저장 성공,
명시적 재시도, 취소·실제 닫힘 뒤 late callback도 검사합니다. 일반 성공/clean 경로의 추가 경고는 없습니다.

활성 데스크톱 strict 검사는 다른 StellarMac/xcodebuild 검사 프로세스가 없는 것을 확인한 뒤 단독 실행했습니다.

```sh
apps/macos/build/Build/Products/Debug/StellarMac.app/Contents/MacOS/StellarMac \
  --window-close-check crates/runtime/tests/fixtures/window-session-project --native-close
```

**종료 1: `실제 key 창 활성화 실패`.** 일반 실행의 실제 활성 창 닫기 버튼·화면 표시 유지 검증은 미완료입니다.
화면 밖 AppKit 통합 통과로 대체하지 않습니다. 로그인된 활성 데스크톱에서 위 명령을 단독 재실행해야 합니다.
이 결과는 기존 세션 통합 보고서의 활성화 실패와 같으며 원인은 이번 변경으로 확인하지 못했습니다.

중간 실패도 기록합니다. 최초 FFI 테스트는 헤더 재생성 전 version 불일치로 1건 실패했고 재생성 후 통과했습니다.
첫 Xcode 호출은 존재하지 않는 Xcode-beta 경로 지정으로 실패해 실제 `/Applications/Xcode.app`으로 수정했습니다.
첫 close 검사는 검사 run loop가 timeout 상태를 확인하기 전에 반환해 timeout 항목 2건이 실패했습니다.
검사가 상태 전이를 기한 안에서 기다리도록 수정한 뒤 타깃과 전체 smoke가 통과했습니다. 제품 watchdog을 늘려
통과시키지 않았습니다. 검사 코드의 미사용 결과 Swift 경고 1건도 수정해 최종 빌드에는 없습니다.

C ABI 재실행 명령:

```sh
cc -Icrates/ffi/include crates/ffi/tests/runtime_abi.c target/debug/libstellar_ffi.a \
  -o /tmp/stellar-close-abi -framework CoreFoundation -framework CoreServices -framework Security
/tmp/stellar-close-abi
```

원시 로그는 `/tmp/stellar-close-{rust,clippy,build,native-target,strict,smoke}.log`입니다.
임시 로그 보존을 전제로 하지 않으며 위 명령과 결과가 인계 근거입니다.

## 닫기 도중 다른 시트가 열리는 경우

`651d3d92`에서 저장을 기다리는 동안 같은 창에 다른 시트가 붙는 경우를 추가했습니다.
기존 동작은 저장에 성공해도 시트가 닫기를 거부하면서 원래 닫기 요청을 잃었습니다.
이제 시트가 끝날 때까지 요청을 보관하고, 원래 창과 session의 편집 상태를 다시 확인한 뒤 닫습니다.
`--window-close-check`의 98개 검사와 `scripts/macos/smoke.sh`가 통과했습니다. 활성 key 창 검사는
여전히 위 명령의 활성화 단계에서 종료 코드 1이었습니다.

## 한계와 후속

- 응답 watchdog 10초는 이 correctness slice의 셸 상한입니다. 성능 benchmark 결과나 AutosavePolicy 설정을
  대신하지 않습니다. 실제 autosave 수치는 여전히 미확정이며 config 변경은 없습니다.
- timeout은 저장 취소가 아닙니다. 늦게 저장이 끝날 수 있으나 원래 창은 열린 채 유지됩니다. worker 취소·
  revision별 capture/result 검증·자동 저장 합치기는 XIII.1.3~.5에서 구현해야 합니다.
- Save Now 직렬화·쓰기는 document thread에서 실행하므로 큰 문서에서 다른 코어 요청은 지연될 수 있습니다.
  GUI thread는 기다리지 않습니다. 새로 추가한 정상 닫기 경로에는 blocking 대기·modal run loop가 없습니다.
- 앱 Quit(⌘Q)·종료·업데이트 재시작·앱 비활성화 flush는 이 경로를 거치지 않습니다. 종료 의미를 바꾸지 않았으며
  이들 경로의 미저장 편집 보존을 보장하지 않습니다. 마지막 프로젝트 창 닫기 후 시작 창 표시도 기존대로입니다.
- 세대를 쓰는 Save Now와 `stellar apply`는 OS 사용자 데이터의 프로젝트별 저장 잠금을 공유합니다.
  잠금 경합은 창을 유지하는 저장 실패로 전달되며, 잠금 해제 후 재시도 때 디스크 세대가 달라졌으면 stale을 반환합니다.
  외부 Git 변경의 자동 병합과 다른 OS 사용자 간 쓰기 조정은 별도 범위입니다.
- normal UI의 캔버스 편집 도구는 아직 미연결입니다. Debug bridge는 실제 Rust transaction으로 편집을 만들며
  native close·저장·재로드·오류는 제품 코드로 확인합니다. 시뮬레이터의 가짜 저장 성공을 근거로 쓰지 않습니다.
- 저수준 `NSWindow.close()`와 `stellar_window_session_close`는 명시적 폐기 API로 유지합니다. 정상 사용자 닫기는
  `performClose`를 거쳐야 합니다. 테스트 정리에서만 저수준 close를 사용합니다.
