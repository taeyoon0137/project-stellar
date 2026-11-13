#!/usr/bin/env bash
# macOS 셸 smoke/lifecycle/canvas/text input test (II.5.2, II.5.3, II.5.4, XII.5.1).
# StellarMac 앱을 Debug로 빌드한 뒤:
#  - --smoke: FFI로 코어 버전을 읽고 종료 코드 0
#  - --lifecycle-check: runtime create/destroy가 1:1인지 확인하고 종료 코드 0
#  - --canvas-check: drawable pixel == point × scale 및 코어 round-trip 확인
#  - --text-input-check: 네이티브 UI 텍스트 컨트롤의 undo/IME(marked text) smoke
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

# 네이티브 UI 텍스트 컨트롤은 코어 텍스트 엔진(FFI)을 호출하지 않습니다(XII.5.1 구조 검사).
for source in NativeTextControls.swift NativeFieldsPanel.swift; do
  if grep -n "stellar_" "$REPO_ROOT/apps/macos/StellarMac/$source"; then
    echo "text 구조 실패: $source 가 코어 FFI를 호출합니다." >&2
    exit 1
  fi
done

# text input: NSTextView/NSTextField/NSSearchField의 native undo/redo + IME marked text.
"$APP" --text-input-check

echo "smoke OK"
