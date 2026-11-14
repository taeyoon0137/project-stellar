#!/usr/bin/env bash
# macOS Release 빌드와 배포 구성 검사(XVI.9.1, XVI.9.2).
#
# 서명하지 않은 Release 앱을 빌드하고 다음을 확인합니다(하나라도 어긋나면 실패):
#  - Xcode Release 설정: ARCHS=arm64, ONLY_ACTIVE_ARCH=NO, MACOSX_DEPLOYMENT_TARGET=26.0,
#    PRODUCT_BUNDLE_IDENTIFIER=xyz.taeyoon.stellar
#  - 실행 파일과 Rust 코어 staticlib이 arm64 전용(lipo)
#  - 실행 파일 LC_BUILD_VERSION: platform MACOS, minos 26.0
#  - Info.plist: CFBundleIdentifier, LSMinimumSystemVersion=26.0, CFBundleExecutable
#  - Info.plist 업데이트 구성(ADR-0017): Stable/Beta appcast https URL(서로 다름), 서명 appcast·추출 전 검증 요구,
#    자동 다운로드·설치 금지
#  - Release 실행 파일 --smoke(FFI로 코어 버전 읽기)
#  - 번들 구조·링크·rpath·서명 구조·재배치 실행: scripts/macos/bundle_check.sh --adhoc-sign(XVI.9.2)
#
# Swift package(Sparkle)는 Package.resolved 고정 버전만 쓰고(-disableAutomaticPackageResolution),
# 자격 증명 조회는 키체인 대신 netrc로 합니다(공개 package라 인증 불필요, 키체인 접근 대화상자 방지).
#
# Rust 코어 staticlib은 cargo의 실제 target 디렉터리(CARGO_TARGET_DIR·cargo 설정의 build.target-dir 반영, 기본은 저장소
# target/)에서 링크하고 검사합니다. 회귀 검사: scripts/macos/release_target_dir_check.sh
#
# 업데이트 구성 override(설정된 환경 변수만 xcodebuild build setting으로 전달):
#   STELLAR_UPDATE_FEED_STABLE, STELLAR_UPDATE_FEED_BETA  appcast URL(기본값은 project.pbxproj의 공개 릴리스
#                                                         저장소 placeholder)
#   STELLAR_UPDATE_PUBLIC_ED_KEY                          Sparkle EdDSA 공개 키(base64). 비어 있으면 앱은 업데이트를
#                                                         시작하지 않습니다. 키 생성은 사용자 단계입니다(ADR-0017).
#
# 산출물: apps/macos/build/Build/Products/Release/Stellar.app (build/는 git ignore)
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

PROJECT="$REPO_ROOT/apps/macos/Stellar.xcodeproj"
DERIVED="$REPO_ROOT/apps/macos/build"
PACKAGE_FLAGS=(-packageAuthorizationProvider netrc -disableAutomaticPackageResolution)
UPDATE_OVERRIDES=()
for name in STELLAR_UPDATE_FEED_STABLE STELLAR_UPDATE_FEED_BETA STELLAR_UPDATE_PUBLIC_ED_KEY; do
  if [ -n "${!name:-}" ]; then
    UPDATE_OVERRIDES+=("$name=${!name}")
  fi
done

# --- Rust 코어 staticlib 위치: build phase의 cargo가 실제로 쓰는 target 디렉터리 ---
# project.pbxproj의 LIBRARY_SEARCH_PATHS는 저장소 target/을 가리킵니다. CARGO_TARGET_DIR 등으로 cargo의 target이 다르면
# 그곳의 오래된 staticlib이 링크되거나 링크가 실패하므로, cargo가 알려 주는 target 디렉터리를 링크와 검사에 함께 씁니다.
# phase와 같이 저장소 루트에서 ~/.cargo/bin을 앞에 둔 PATH로 cargo를 찾습니다.
RUST_TARGET_DIR="$(cd "$REPO_ROOT" && PATH="$HOME/.cargo/bin:$PATH" \
  && cargo metadata --format-version 1 --no-deps --locked | plutil -extract target_directory raw -o - -)" \
  || fail "cargo target 디렉터리를 읽지 못함"
CORE_LIB_DIR="$RUST_TARGET_DIR/$RUST_TARGET/release"
# 경로 목록 build setting이라 공백이 있는 경로도 한 항목이 되도록 따옴표로 감쌉니다.
LINK_OVERRIDES=("LIBRARY_SEARCH_PATHS=\"$CORE_LIB_DIR\"")

# --- Xcode Release 설정 고정값 ---
SETTINGS="$(xcodebuild -project "$PROJECT" -scheme Stellar -configuration Release "${PACKAGE_FLAGS[@]}" \
  -derivedDataPath "$DERIVED" "${LINK_OVERRIDES[@]}" ${UPDATE_OVERRIDES[@]+"${UPDATE_OVERRIDES[@]}"} \
  -showBuildSettings 2>/dev/null)"
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
  -scheme Stellar \
  -configuration Release \
  -derivedDataPath "$DERIVED" \
  "${PACKAGE_FLAGS[@]}" \
  CODE_SIGNING_ALLOWED=NO \
  "${LINK_OVERRIDES[@]}" \
  ${UPDATE_OVERRIDES[@]+"${UPDATE_OVERRIDES[@]}"} \
  build

APP="$DERIVED/Build/Products/Release/Stellar.app"
BIN="$APP/Contents/MacOS/Stellar"
CORE_LIB="$CORE_LIB_DIR/libstellar_ffi.a"
[ -x "$BIN" ] || fail "실행 파일 없음: $BIN"
[ -f "$CORE_LIB" ] || fail "Rust 코어 staticlib 없음: $CORE_LIB"
echo "✓ Rust 코어 staticlib: $CORE_LIB"

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
[ "$(plist CFBundleDisplayName)" = "Stellar" ] || fail "CFBundleDisplayName=$(plist CFBundleDisplayName)"
[ "$(plist CFBundleExecutable)" = "Stellar" ] || fail "CFBundleExecutable=$(plist CFBundleExecutable)"
echo "✓ Info.plist: $EXPECTED_BUNDLE_ID, LSMinimumSystemVersion $EXPECTED_MIN_OS"

# --- Info.plist 업데이트 구성(ADR-0017) ---
STABLE_FEED="$(plist SUFeedURL)"
BETA_FEED="$(plist StellarUpdateFeedBeta)"
case "$STABLE_FEED" in https://?*) ;; *) fail "SUFeedURL이 https URL이 아님: '$STABLE_FEED'" ;; esac
case "$BETA_FEED" in https://?*) ;; *) fail "StellarUpdateFeedBeta가 https URL이 아님: '$BETA_FEED'" ;; esac
[ "$STABLE_FEED" != "$BETA_FEED" ] || fail "Stable과 Beta appcast URL이 같음"
for key in SURequireSignedFeed SUVerifyUpdateBeforeExtraction SUEnableAutomaticChecks; do
  [ "$(plist "$key")" = "true" ] || fail "$key=$(plist "$key")"
done
for key in SUAllowsAutomaticUpdates SUAutomaticallyUpdate; do
  [ "$(plist "$key")" = "false" ] || fail "$key=$(plist "$key")"
done
PUBLIC_KEY="$(/usr/libexec/PlistBuddy -c "Print :SUPublicEDKey" "$APP/Contents/Info.plist" 2>/dev/null || true)"
if [ -n "$PUBLIC_KEY" ]; then
  KEY_STATE="EdDSA 공개 키 설정됨"
else
  KEY_STATE="EdDSA 공개 키 없음(업데이트 비활성, ADR-0017 사용자 단계)"
fi
echo "✓ 업데이트 구성: stable/beta appcast 분리, 서명 appcast 요구, 자동 설치 금지, $KEY_STATE"

# --- Release 실행 smoke ---
"$BIN" --smoke

# --- 번들 구조·rpath·서명 구조·재배치 실행(ad-hoc 서명 사본, 원본은 서명하지 않은 상태 유지) ---
"$REPO_ROOT/scripts/macos/bundle_check.sh" --adhoc-sign "$APP"

echo "✓ Release 구성 검사 통과: $APP"
