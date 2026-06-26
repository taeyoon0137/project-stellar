#!/usr/bin/env bash
# macOS 셸 smoke/lifecycle/canvas test (II.5.2, II.5.3, II.5.4).
# StellarMac 앱을 Debug로 빌드한 뒤:
#  - --smoke: FFI로 코어 버전을 읽고 종료 코드 0
#  - --lifecycle-check: runtime create/destroy가 1:1인지 확인하고 종료 코드 0
#  - --canvas-check: drawable pixel == point × scale 및 코어 round-trip 확인
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

echo "smoke OK"
