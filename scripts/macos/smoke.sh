#!/usr/bin/env bash
# macOS 셸 smoke/lifecycle/canvas/text input/input/welcome test (II.5.2, II.5.3, II.5.4, XII.5.1, XII.3.2, XII.3.4, XVI.9.5,
# XXIV.5.3~XXIV.5.6).
# Stellar 앱을 Debug로 빌드한 뒤:
#  - --smoke: FFI로 코어 버전을 읽고 종료 코드 0
#  - --lifecycle-check: runtime create/destroy가 1:1인지 확인하고 종료 코드 0
#  - --canvas-check: drawable pixel == point × scale 및 코어 round-trip 확인
#  - --text-input-check: 네이티브 UI 텍스트 컨트롤의 undo/IME(marked text) smoke
#  - --input-check: mouse·trackpad·keyboard NSEvent → 공통 InputEvent 변환과 연속 move coalescing
#  - --update-check: 업데이트 구성·채널 격리·appcast 서명·승인 gate(Sparkle, loopback fixture)
#  - --<영역>-ui-check: UI 영역별 화면 밖 검사(shell-frame, tools-menus, sidebar, inspector, git-panel, agent-panel, welcome)
#  - --shell-integration-check: 실제 앱 구성(메뉴, 시작 창, 코어로 프로젝트 열기, 프로젝트 창 영역·도구, 복원·닫기)
#  - --project-open-check: Open Project 폴더 선택·코어 열기와 잘못된 폴더별 안내 대응(XII.6.2)
#  - --window-session-check: 저장 프로젝트 fixture로 창 2개 open/close, 창 controller당 코어 창 session 하나(XII.2.2)
#  - --session-commands-check: 메뉴 Undo·Redo·Save Now가 그 창의 코어 창 session에만 가는지(XII.6.4)
#  - --key-mapping-check: 코어 공유 command ID와 macOS 관례 단축키 ⌘Z·⇧⌘Z·⌘S 대응(XII.6.5)
#  - UI 구조 검사: UI/Tools는 코어 FFI를 모르고, UI/Shell은 stellar_ 함수를 직접 부르지 않음(CoreRuntime 경유)
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

# 로컬에서 xcode-select가 CommandLineTools를 가리키면 Xcode를 직접 지정합니다.
# CI 러너처럼 이미 Xcode가 선택된 환경에서는 DEVELOPER_DIR를 덮어쓰지 않습니다.
if ! xcodebuild -version >/dev/null 2>&1; then
  if [ -d /Applications/Xcode-beta.app ]; then
    export DEVELOPER_DIR=/Applications/Xcode-beta.app/Contents/Developer
  elif [ -d /Applications/Xcode.app ]; then
    export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
  fi
fi

DERIVED="$REPO_ROOT/apps/macos/build"

# Swift package(Sparkle, ADR-0017)는 Package.resolved 고정 버전만 쓰고 자격 증명 조회는 netrc로 합니다
# (공개 package라 인증 불필요, 키체인 접근 대화상자 방지).
xcodebuild \
  -project "$REPO_ROOT/apps/macos/Stellar.xcodeproj" \
  -scheme Stellar \
  -configuration Debug \
  -derivedDataPath "$DERIVED" \
  -packageAuthorizationProvider netrc \
  -disableAutomaticPackageResolution \
  CODE_SIGNING_ALLOWED=NO \
  build

APP="$DERIVED/Build/Products/Debug/Stellar.app/Contents/MacOS/Stellar"

OUTPUT="$("$APP" --smoke)"
echo "$OUTPUT"
if ! printf '%s\n' "$OUTPUT" | grep -q "stellar core version:"; then
  echo "smoke 실패: 버전 문자열을 읽지 못했습니다." >&2
  exit 1
fi

# lifecycle: create/destroy 1:1 (set -e가 비0 종료를 실패로 처리).
"$APP" --lifecycle-check

# canvas: drawable pixel == point × scale + 코어 round-trip.
"$APP" --canvas-check
STELLAR_TEST_CANVAS_APP="$APP" cargo test --locked --manifest-path "$REPO_ROOT/Cargo.toml" -p stellar-runtime --lib canvas::tests::saved_frames_open_in_window_painter_order -- --exact --nocapture

# 네이티브 UI 텍스트 컨트롤은 코어 텍스트 엔진(FFI)을 직접·간접으로 호출하지 않습니다(XII.5.1 구조 검사).
# - 대상: 고정 목록 + `Native*.swift`로 추가되는 네이티브 UI 파일 전부.
# - 목록의 파일이 없으면(이름 변경·삭제) 검사가 조용히 통과하지 않도록 실패합니다.
# - 직접 호출(`stellar_`)뿐 아니라 셸 FFI wrapper(`CoreRuntime`, `RuntimeHost`)와 runtime handle 전달도 금지합니다.
# - grep 종료 코드 0(발견)과 2 이상(읽기 오류)은 모두 실패로 처리합니다.
SOURCES_DIR="$REPO_ROOT/apps/macos/Stellar"
NATIVE_UI_SOURCES=(NativeTextControls.swift NativeFieldsPanel.swift)
for path in "$SOURCES_DIR"/Native*.swift; do
  [ -e "$path" ] || continue
  name="$(basename "$path")"
  case " ${NATIVE_UI_SOURCES[*]} " in
    *" $name "*) ;;
    *) NATIVE_UI_SOURCES+=("$name") ;;
  esac
done
CORE_CALL_PATTERN='stellar_|CoreRuntime|RuntimeHost|runtimeHandle'
for source in "${NATIVE_UI_SOURCES[@]}"; do
  path="$SOURCES_DIR/$source"
  if [ ! -f "$path" ]; then
    echo "text 구조 실패: 검사 대상 $source 가 없습니다(이름을 바꿨다면 smoke.sh 목록을 갱신하세요)." >&2
    exit 1
  fi
  status=0
  grep -nE "$CORE_CALL_PATTERN" "$path" || status=$?
  case "$status" in
    0)
      echo "text 구조 실패: $source 가 코어 FFI를 호출합니다." >&2
      exit 1
      ;;
    1) ;;
    *)
      echo "text 구조 실패: $source 검사 중 grep 오류(종료 $status)." >&2
      exit 1
      ;;
  esac
done
echo "text 구조 ok: ${NATIVE_UI_SOURCES[*]}"

# text input: NSTextView/NSTextField/NSSearchField의 native undo/redo + IME marked text.
"$APP" --text-input-check

# input: mouse·trackpad·keyboard NSEvent → 공통 InputEvent(coordinate·modifier·double click 등, XII.3.2)
#        + 10,000 drag → 최신 1개 coalescing(XII.3.4).
"$APP" --input-check

# update: Info.plist 업데이트 구성, Stable/Beta appcast 채널 격리, appcast 서명 검증, 승인 전 다운로드 금지
#         (XVI.9.5, ADR-0017; 실제 Sparkle updater와 127.0.0.1 fixture 서버, 외부 네트워크 없음).
"$APP" --update-check

# UI 영역 검사: 각 영역(docs/design/ui/*.md)을 화면 밖 창으로 만들어 레이아웃·접근성·명령·snapshot 반영을 확인합니다.
AREA_UI_CHECKS=(shell-frame tools-menus sidebar inspector git-panel agent-panel welcome)
for area in "${AREA_UI_CHECKS[@]}"; do
  "$APP" "--${area}-ui-check"
done

# 셸 통합: 앱이 실제로 쓰는 구성을 화면 밖에서 검사합니다(실제 코어 runtime으로 임시 fixture 프로젝트를 엽니다).
"$APP" --shell-integration-check

# 신뢰 등록 smoke는 검사 전용 HOME만 사용합니다.
(
  trust_home="$(mktemp -d)"
  trap 'rm -rf "$trust_home"' EXIT
  HOME="$trust_home" "$APP" --project-trust-check "$trust_home"
)

# Open Project(XII.6.2): 폴더 선택 → 코어 열기·안전 probe → 창, 잘못된 폴더 fixture별 실패 코드 → 안내 대응.
"$APP" --project-open-check
(
  clone_home="$(mktemp -d)"
  trap 'rm -rf "$clone_home"' EXIT
  HOME="$clone_home" GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 GIT_ALLOW_PROTOCOL=file \
    GIT_AUTHOR_NAME=Fixture GIT_COMMITTER_NAME=Fixture GIT_AUTHOR_EMAIL=fixture@example.invalid \
    GIT_COMMITTER_EMAIL=fixture@example.invalid "$APP" --project-clone-check "$clone_home"
)

# 창 session(XII.2.2): 저장 프로젝트 fixture를 복사한 두 프로젝트를 열고 닫으며 창 controller마다 코어 창 session handle
# 하나를 소유하는지, 닫으면 그 창 session만 닫히는지 확인합니다.
WINDOW_SESSION_FIXTURE="$REPO_ROOT/crates/runtime/tests/fixtures/window-session-project"
cargo build --manifest-path "$REPO_ROOT/Cargo.toml" --locked -p stellar-cli
cli_target="${CARGO_TARGET_DIR:-$REPO_ROOT/target}"
python3 "$REPO_ROOT/scripts/macos/canvas-move-check.py" "$APP" "$cli_target/debug/stellar" "$WINDOW_SESSION_FIXTURE"
(
  frame_home="$(mktemp -d)"
  trap 'rm -rf "$frame_home"' EXIT
  unset GIT_AUTHOR_NAME GIT_AUTHOR_EMAIL GIT_COMMITTER_NAME GIT_COMMITTER_EMAIL EMAIL
  HOME="$frame_home" XDG_CONFIG_HOME="$frame_home" GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 \
    "$APP" --frame-insert-check "$frame_home" --expect-missing-git-identity
)

"$APP" --window-session-check "$WINDOW_SESSION_FIXTURE"

# 창 session 명령(XII.6.4): 메뉴 Undo·Redo·Save Now를 그 창의 창 session에 보내고, 다른 창 history·디스크는 그대로인지,
# Stale 저장은 쓰지 않고 alert 한 번인지 확인합니다.
"$APP" --session-commands-check "$WINDOW_SESSION_FIXTURE"

# 정상 AppKit 닫기에서 실제 Rust 편집 저장·실패·timeout·late callback 보존.
"$APP" --window-close-check "$WINDOW_SESSION_FIXTURE"

# 단축키 대응(XII.6.5): command ID는 코어가 공유 이름으로 알려 주고, 단축키는 macOS 관례(⌘Z·⇧⌘Z·⌘S)로 메뉴에 둡니다.
# 키 조합 → 메뉴 항목 → 셸 명령 → 코어 ID → 창 session 도착을 확인합니다.
# 실제 활성 앱의 nil target 메뉴 검사: 아래 명령 끝에 --native-menus를 더합니다(활성화 실패는 비0 종료).
"$APP" --key-mapping-check "$WINDOW_SESSION_FIXTURE"

# UI 구조 검사(grep 종료 코드 0 = 발견 = 실패, 2 이상 = 읽기 오류 = 실패).
# - UI/Tools(메뉴·도구 명령)는 코어 FFI와 runtime handle을 모릅니다. 명령은 ToolsMenuModel로만 나갑니다.
# - UI/Shell은 stellar_ 함수를 직접 부르지 않고 CoreRuntime(FFI 헬퍼)을 거칩니다.
check_ui_structure() {
  local label="$1" pattern="$2" dir="$3"
  if [ ! -d "$dir" ]; then
    echo "UI 구조 실패: $dir 가 없습니다." >&2
    exit 1
  fi
  local status=0
  grep -rnE "$pattern" "$dir" --include='*.swift' || status=$?
  case "$status" in
    0)
      echo "UI 구조 실패: $label" >&2
      exit 1
      ;;
    1) ;;
    *)
      echo "UI 구조 실패: $dir 검사 중 grep 오류(종료 $status)." >&2
      exit 1
      ;;
  esac
}
check_ui_structure "UI/Tools가 코어 FFI·runtime을 참조합니다." 'stellar_|CoreRuntime|RuntimeHost|runtimeHandle' "$SOURCES_DIR/UI/Tools"
check_ui_structure "UI/Shell이 stellar_ 함수를 직접 호출합니다(CoreRuntime 경유)." '\bstellar_[a-z_]+\(' "$SOURCES_DIR/UI/Shell"
echo "UI 구조 ok: UI/Tools FFI 없음, UI/Shell FFI는 CoreRuntime 경유"

echo "smoke OK"
