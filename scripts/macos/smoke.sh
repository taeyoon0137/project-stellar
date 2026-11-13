#!/usr/bin/env bash
# macOS 셸 smoke/lifecycle/canvas/text input/input test (II.5.2, II.5.3, II.5.4, XII.5.1, XII.3.2, XII.3.4, XVI.9.5).
# StellarMac 앱을 Debug로 빌드한 뒤:
#  - --smoke: FFI로 코어 버전을 읽고 종료 코드 0
#  - --lifecycle-check: runtime create/destroy가 1:1인지 확인하고 종료 코드 0
#  - --canvas-check: drawable pixel == point × scale 및 코어 round-trip 확인
#  - --text-input-check: 네이티브 UI 텍스트 컨트롤의 undo/IME(marked text) smoke
#  - --input-check: mouse·trackpad·keyboard NSEvent → 공통 InputEvent 변환과 연속 move coalescing
#  - --update-check: 업데이트 채널 격리·승인 gate
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

xcodebuild \
  -project "$REPO_ROOT/apps/macos/StellarMac.xcodeproj" \
  -scheme StellarMac \
  -configuration Debug \
  -derivedDataPath "$DERIVED" \
  CODE_SIGNING_ALLOWED=NO \
  build

APP="$DERIVED/Build/Products/Debug/StellarMac.app/Contents/MacOS/StellarMac"

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

# 네이티브 UI 텍스트 컨트롤은 코어 텍스트 엔진(FFI)을 직접·간접으로 호출하지 않습니다(XII.5.1 구조 검사).
# - 대상: 고정 목록 + `Native*.swift`로 추가되는 네이티브 UI 파일 전부.
# - 목록의 파일이 없으면(이름 변경·삭제) 검사가 조용히 통과하지 않도록 실패합니다.
# - 직접 호출(`stellar_`)뿐 아니라 셸 FFI wrapper(`CoreRuntime`, `RuntimeHost`)와 runtime handle 전달도 금지합니다.
# - grep 종료 코드 0(발견)과 2 이상(읽기 오류)은 모두 실패로 처리합니다.
SOURCES_DIR="$REPO_ROOT/apps/macos/StellarMac"
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

# update: Stable/Beta feed 채널 격리, background 확인, 승인 전 다운로드 금지(XVI.9.5).
"$APP" --update-check

echo "smoke OK"
