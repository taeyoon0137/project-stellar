#!/usr/bin/env bash
# macOS Release 빌드와 배포 구성 검사(XVI.9.1, XVI.9.2).
#
# 서명하지 않은 Release 앱을 빌드하고 다음을 확인합니다(하나라도 어긋나면 실패):
#  - Xcode Release 설정: ARCHS=arm64, ONLY_ACTIVE_ARCH=NO, MACOSX_DEPLOYMENT_TARGET=26.0,
#    PRODUCT_BUNDLE_IDENTIFIER=xyz.taeyoon.stellar
#  - 실행 파일과 Rust 코어 staticlib이 arm64 전용(lipo)
#  - 실행 파일 LC_BUILD_VERSION: platform MACOS, minos 26.0
#  - Info.plist: CFBundleIdentifier, LSMinimumSystemVersion=26.0, CFBundleExecutable
#  - Release 실행 파일 --smoke(FFI로 코어 버전 읽기)
#  - 번들 구조·링크·rpath·서명 구조·재배치 실행: scripts/macos/bundle_check.sh --adhoc-sign(XVI.9.2)
#
# 산출물: apps/macos/build/Build/Products/Release/StellarMac.app (build/는 git ignore)
# 종료 코드: 0 성공, 그 외 빌드·검사 실패.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
EXPECTED_BUNDLE_ID="xyz.taeyoon.stellar"
EXPECTED_MIN_OS="26.0"
EXPECTED_ARCHS="arm64"
RUST_TARGET="aarch64-apple-darwin"

# --- Xcode 선택(smoke.sh와 같은 규칙) ---
if ! xcodebuild -version >/dev/null 2>&1; then
  if [ -d /Applications/Xcode-beta.app ]; then
    export DEVELOPER_DIR=/Applications/Xcode-beta.app/Contents/Developer
  elif [ -d /Applications/Xcode.app ]; then
    export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
  fi
fi

fail() {
  echo "✖ release 구성 실패: $*" >&2
  exit 1
}

PROJECT="$REPO_ROOT/apps/macos/StellarMac.xcodeproj"
DERIVED="$REPO_ROOT/apps/macos/build"

# --- Xcode Release 설정 고정값 ---
SETTINGS="$(xcodebuild -project "$PROJECT" -scheme StellarMac -configuration Release -showBuildSettings 2>/dev/null)"
setting() {
  # 'KEY = value' 줄에서 값만 꺼냅니다(정확히 일치하는 key).
  printf '%s\n' "$SETTINGS" | awk -v key="$1" '$1 == key && $2 == "=" { sub(/^[^=]*= /, ""); print; exit }'
}
[ "$(setting ARCHS)" = "$EXPECTED_ARCHS" ] || fail "ARCHS=$(setting ARCHS)"
[ "$(setting ONLY_ACTIVE_ARCH)" = "NO" ] || fail "ONLY_ACTIVE_ARCH=$(setting ONLY_ACTIVE_ARCH)"
[ "$(setting MACOSX_DEPLOYMENT_TARGET)" = "$EXPECTED_MIN_OS" ] \
  || fail "MACOSX_DEPLOYMENT_TARGET=$(setting MACOSX_DEPLOYMENT_TARGET)"
[ "$(setting PRODUCT_BUNDLE_IDENTIFIER)" = "$EXPECTED_BUNDLE_ID" ] \
  || fail "PRODUCT_BUNDLE_IDENTIFIER=$(setting PRODUCT_BUNDLE_IDENTIFIER)"
echo "✓ Xcode Release 설정: ARCHS=arm64, ONLY_ACTIVE_ARCH=NO, macOS $EXPECTED_MIN_OS, $EXPECTED_BUNDLE_ID"

# --- 빌드(서명 없음; Xcode build phase가 arm64 release staticlib을 --locked로 빌드) ---
xcodebuild \
  -project "$PROJECT" \
  -scheme StellarMac \
  -configuration Release \
  -derivedDataPath "$DERIVED" \
  CODE_SIGNING_ALLOWED=NO \
  build

APP="$DERIVED/Build/Products/Release/StellarMac.app"
BIN="$APP/Contents/MacOS/StellarMac"
CORE_LIB="$REPO_ROOT/target/$RUST_TARGET/release/libstellar_ffi.a"
[ -x "$BIN" ] || fail "실행 파일 없음: $BIN"
[ -f "$CORE_LIB" ] || fail "Rust 코어 staticlib 없음: $CORE_LIB"

# --- 아키텍처: arm64 전용 ---
BIN_ARCHS="$(lipo -archs "$BIN")"
[ "$BIN_ARCHS" = "$EXPECTED_ARCHS" ] || fail "실행 파일 아키텍처: $BIN_ARCHS"
LIB_ARCHS="$(lipo -archs "$CORE_LIB")"
[ "$LIB_ARCHS" = "$EXPECTED_ARCHS" ] || fail "Rust 코어 아키텍처: $LIB_ARCHS"
echo "✓ 아키텍처: 실행 파일 $BIN_ARCHS, Rust 코어 $LIB_ARCHS"

# --- 최소 OS: LC_BUILD_VERSION ---
BUILD_VERSION="$(xcrun vtool -show-build "$BIN")"
printf '%s\n' "$BUILD_VERSION" | grep -Eq "platform[[:space:]]+MACOS" || fail "LC_BUILD_VERSION platform: $BUILD_VERSION"
printf '%s\n' "$BUILD_VERSION" | grep -Eq "minos[[:space:]]+$EXPECTED_MIN_OS$" || fail "LC_BUILD_VERSION minos: $BUILD_VERSION"
echo "✓ LC_BUILD_VERSION: platform MACOS, minos $EXPECTED_MIN_OS"

# --- Info.plist ---
plist() {
  /usr/libexec/PlistBuddy -c "Print :$1" "$APP/Contents/Info.plist"
}
[ "$(plist CFBundleIdentifier)" = "$EXPECTED_BUNDLE_ID" ] || fail "CFBundleIdentifier=$(plist CFBundleIdentifier)"
[ "$(plist LSMinimumSystemVersion)" = "$EXPECTED_MIN_OS" ] || fail "LSMinimumSystemVersion=$(plist LSMinimumSystemVersion)"
[ "$(plist CFBundleExecutable)" = "StellarMac" ] || fail "CFBundleExecutable=$(plist CFBundleExecutable)"
echo "✓ Info.plist: $EXPECTED_BUNDLE_ID, LSMinimumSystemVersion $EXPECTED_MIN_OS"

# --- Release 실행 smoke ---
"$BIN" --smoke

# --- 번들 구조·rpath·서명 구조·재배치 실행(ad-hoc 서명 사본, 원본은 서명하지 않은 상태 유지) ---
"$REPO_ROOT/scripts/macos/bundle_check.sh" --adhoc-sign "$APP"

echo "✓ Release 구성 검사 통과: $APP"
