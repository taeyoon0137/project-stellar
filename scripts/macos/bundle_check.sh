#!/usr/bin/env bash
# Stellar.app 번들 구조·링크·rpath·코드 서명 구조 검사와 재배치 실행 smoke(XVI.9.2, ADR-0017).
#
# 사용법:
#   scripts/macos/bundle_check.sh --adhoc-sign APP   # 서명 안 된 앱: 임시 사본에 ad-hoc(Hardened Runtime) 서명 후 검사
#   scripts/macos/bundle_check.sh APP                # 이미 서명된 앱(예: Developer ID)을 그대로 검사
#
# 검사(하나라도 어긋나면 실패):
#  1. 번들 구조 allowlist: Contents/{Info.plist, PkgInfo, MacOS/Stellar, Helpers/stellar, _CodeSignature/CodeResources,
#     CodeResources(stapled 공증 ticket), Resources/**, Frameworks/Sparkle.framework/**}만 허용(빌드 중간물·dSYM·
#     헤더·module·staticlib·다른 framework 혼입 금지)
#  2. Mach-O 파일은 실행 파일과 Sparkle.framework의 고정 목록(framework·Autoupdate·Updater·XPC service 2개)뿐이고
#     모두 arm64 slice를 포함(Rust 코어는 staticlib으로 실행 파일에 포함)
#  3. 동적 링크는 시스템 경로(/usr/lib, /System/Library)와 @rpath/Sparkle.framework만, 빌드 경로 의존 없음,
#     @rpath 대상은 번들 안에 존재
#  4. LC_RPATH는 없거나 @executable_path/@loader_path 상대 경로만
#  5. C ABI 헤더(stellar_ffi.h)의 모든 stellar_* 함수가 실행 파일에 정의됨(Rust 코어 포함 확인)
#  6. 코드 서명: codesign --verify --deep --strict, CodeResources 존재, Sealed Resources v2, Info.plist bound,
#     Hardened Runtime flag, 모든 중첩 코드가 Hardened Runtime이고 앱과 같은 TeamIdentifier.
#     이미 서명된 앱은 library validation을 끄는 entitlement(`com.apple.security.cs.disable-library-validation`)가
#     없어야 합니다. ad-hoc 사본은 Team ID가 없어 Hardened Runtime의 library validation이 번들 안 framework load를
#     거부하므로, 임시 사본의 앱 실행 파일에만 이 entitlement를 붙여 서명합니다(배포 서명에는 붙이지 않음).
#  7. 재배치 실행: 번들을 임시 폴더로 복사해 최소 환경(env -i)과 sandbox 읽기 차단(저장소·DerivedData·Xcode·
#     CommandLineTools·~/.cargo·~/.rustup)에서 --smoke, --lifecycle-check, --canvas-check, --update-check
#     (--update-check는 번들 안 Sparkle.framework를 실제로 load해 loopback fixture로 확인)
#
# 종료 코드: 0 성공, 64 사용법 오류, 그 외 검사 실패.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
HEADER="$REPO_ROOT/crates/ffi/include/stellar_ffi.h"
SPARKLE_DIR="Contents/Frameworks/Sparkle.framework"
SPARKLE_VERSION_DIR="$SPARKLE_DIR/Versions/B"
SPARKLE_INSTALL_NAME="@rpath/Sparkle.framework/Versions/B/Sparkle"

ADHOC=0
if [ "${1:-}" = "--adhoc-sign" ]; then
  ADHOC=1
  shift
fi
if [ "$#" -ne 1 ] || [ ! -d "$1" ]; then
  echo "사용법: $0 [--adhoc-sign] Stellar.app" >&2
  exit 64
fi
SOURCE_APP="$(cd "$1" && pwd)"

fail() {
  echo "✖ bundle 검사 실패: $*" >&2
  exit 1
}

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# 서명 검사는 사본에서 합니다(원본 빌드 산출물을 바꾸지 않음).
APP="$WORK/sign/Stellar.app"
mkdir -p "$WORK/sign"
ditto "$SOURCE_APP" "$APP"
BIN="$APP/Contents/MacOS/Stellar"
[ -x "$BIN" ] || fail "실행 파일 없음"

# --- 1. 번들 구조 allowlist ---
while IFS= read -r entry; do
  case "$entry" in
    "$SPARKLE_DIR"/*/Headers | "$SPARKLE_DIR"/*/Headers/* | "$SPARKLE_DIR"/Headers | "$SPARKLE_DIR"/Headers/*)
      fail "Sparkle.framework에 헤더가 남아 있음: $entry" ;;
    "$SPARKLE_DIR"/*/PrivateHeaders* | "$SPARKLE_DIR"/PrivateHeaders* | "$SPARKLE_DIR"/*/Modules* | "$SPARKLE_DIR"/Modules*)
      fail "Sparkle.framework에 개발 전용 항목이 남아 있음: $entry" ;;
  esac
  case "$entry" in
    Contents | Contents/Info.plist | Contents/PkgInfo | Contents/MacOS | Contents/MacOS/Stellar | Contents/Helpers | Contents/Helpers/stellar) ;;
    Contents/_CodeSignature | Contents/_CodeSignature/CodeResources) ;;
    Contents/CodeResources) ;; # stapler가 붙인 공증 ticket(XVI.9.4)
    Contents/Resources | Contents/Resources/*) ;;
    Contents/Frameworks | "$SPARKLE_DIR" | "$SPARKLE_DIR"/*) ;; # 업데이트 framework(ADR-0017)
    *) fail "허용되지 않은 번들 항목: $entry" ;;
  esac
done < <(cd "$APP" && find Contents -mindepth 0 | LC_ALL=C sort)
echo "✓ 번들 구조: allowlist 통과"

# --- 2. Mach-O 목록 고정, 모두 arm64 포함 ---
EXPECTED_MACHO="$(printf '%s\n' \
  "Contents/MacOS/Stellar" \
  "Contents/Helpers/stellar" \
  "$SPARKLE_VERSION_DIR/Autoupdate" \
  "$SPARKLE_VERSION_DIR/Sparkle" \
  "$SPARKLE_VERSION_DIR/Updater.app/Contents/MacOS/Updater" \
  "$SPARKLE_VERSION_DIR/XPCServices/Downloader.xpc/Contents/MacOS/Downloader" \
  "$SPARKLE_VERSION_DIR/XPCServices/Installer.xpc/Contents/MacOS/Installer" | LC_ALL=C sort)"
MACHO_FILES=""
while IFS= read -r path; do
  if file -b "$APP/$path" | head -n 1 | grep -q "^Mach-O"; then
    MACHO_FILES="${MACHO_FILES}${path}"$'\n'
  fi
done < <(cd "$APP" && find Contents -type f | LC_ALL=C sort)
MACHO_FILES="$(printf '%s' "$MACHO_FILES" | LC_ALL=C sort)"
[ "$MACHO_FILES" = "$EXPECTED_MACHO" ] || fail "Mach-O 파일 목록:"$'\n'"$MACHO_FILES"
while IFS= read -r path; do
  lipo -archs "$APP/$path" | tr ' ' '\n' | grep -qx arm64 || fail "arm64 slice 없음: $path"
done <<< "$MACHO_FILES"
[ -x "$APP/Contents/Helpers/stellar" ] && [ ! -L "$APP/Contents/Helpers/stellar" ] || fail "번들 CLI 실행 파일 없음"
[ "$(lipo -archs "$APP/Contents/Helpers/stellar")" = arm64 ] || fail "배포 CLI는 arm64 전용이어야 함"
echo "✓ Mach-O: 앱 + CLI + Sparkle.framework 고정 목록 $(printf '%s\n' "$MACHO_FILES" | grep -c .)개, 모두 arm64 포함"

# --- 3. 동적 링크는 시스템 경로와 번들 안 Sparkle.framework만 ---
[ -f "$APP/$SPARKLE_VERSION_DIR/Sparkle" ] || fail "@rpath 대상 Sparkle.framework 없음"
LINK_COUNT=0
while IFS= read -r path; do
  LINKED="$(otool -arch arm64 -L "$APP/$path" | tail -n +2 | awk '{print $1}')"
  while IFS= read -r lib; do
    if [ "$path" = "Contents/Helpers/stellar" ] && [ "$lib" = "$SPARKLE_INSTALL_NAME" ]; then
      fail "headless CLI가 Sparkle.framework에 의존함"
    fi
    case "$lib" in
      /usr/lib/* | /System/Library/* | "$SPARKLE_INSTALL_NAME") ;;
      *) fail "$path: 허용되지 않은 동적 라이브러리 의존: $lib" ;;
    esac
    LINK_COUNT=$((LINK_COUNT + 1))
  done <<< "$LINKED"
done <<< "$MACHO_FILES"
otool -arch arm64 -L "$BIN" | tail -n +2 | awk '{print $1}' | grep -qx "$SPARKLE_INSTALL_NAME" \
  || fail "실행 파일이 Sparkle.framework를 link하지 않음"
echo "✓ 동적 링크: 시스템 라이브러리와 $SPARKLE_INSTALL_NAME 만(${LINK_COUNT}개 항목)"

# --- 4. LC_RPATH ---
RPATHS="$(otool -l "$BIN" | awk '/cmd LC_RPATH/ { getline; getline; print $2 }')"
if [ -n "$RPATHS" ]; then
  while IFS= read -r rpath; do
    case "$rpath" in
      @executable_path/* | @loader_path/*) ;;
      *) fail "번들 밖 LC_RPATH: $rpath" ;;
    esac
  done <<< "$RPATHS"
fi
while IFS= read -r path; do
  if otool -l "$APP/$path" | grep -qF "$REPO_ROOT"; then
    fail "$path: load command에 빌드 경로($REPO_ROOT)가 있습니다"
  fi
done <<< "$MACHO_FILES"
echo "✓ rpath: $( [ -n "$RPATHS" ] && printf '%s' "$RPATHS" | tr '\n' ' ' || printf '없음')"

# --- 5. Rust 코어 C ABI가 실행 파일에 정적으로 포함 ---
DEFINED="$(nm -gU "$BIN" | awk '{print $NF}')"
ABI_FUNCTIONS="$(xcrun clang -E -P -x c "$HEADER" | grep -oE '\bstellar_[a-z_]+\(' | tr -d '(' | sort -u)"
[ -n "$ABI_FUNCTIONS" ] || fail "헤더에서 C ABI 함수를 찾지 못함"
ABI_COUNT=0
while IFS= read -r symbol; do
  printf '%s\n' "$DEFINED" | grep -qx "_$symbol" || fail "실행 파일에 $symbol 정의 없음"
  ABI_COUNT=$((ABI_COUNT + 1))
done <<< "$ABI_FUNCTIONS"
echo "✓ Rust 코어: C ABI 함수 ${ABI_COUNT}개가 실행 파일에 정의됨(staticlib 포함)"

# --- 6. 코드 서명 구조 ---
if [ "$ADHOC" -eq 1 ]; then
  ADHOC_ENTITLEMENTS="$WORK/adhoc.entitlements"
  /usr/libexec/PlistBuddy -c "Add :com.apple.security.cs.disable-library-validation bool true" "$ADHOC_ENTITLEMENTS" \
    >/dev/null
  "$REPO_ROOT/scripts/macos/codesign_bundle.sh" - --timestamp=none "$APP" "$ADHOC_ENTITLEMENTS"
else
  if codesign -d --entitlements - --xml "$APP" 2>/dev/null | grep -q "disable-library-validation"; then
    fail "서명된 앱이 library validation을 끄는 entitlement를 가짐"
  fi
fi
[ -f "$APP/Contents/_CodeSignature/CodeResources" ] || fail "_CodeSignature/CodeResources 없음(서명되지 않은 번들)"
codesign --verify --deep --strict --verbose=2 "$APP"
SIGNATURE_INFO="$(codesign -dvv "$APP" 2>&1)"
printf '%s\n' "$SIGNATURE_INFO" | grep -q "^Sealed Resources version=2" || fail "Sealed Resources v2 아님"
printf '%s\n' "$SIGNATURE_INFO" | grep -q "^Info.plist entries=" || fail "Info.plist가 서명에 bound되지 않음"
printf '%s\n' "$SIGNATURE_INFO" | grep -q "flags=.*runtime" || fail "Hardened Runtime flag 없음"
printf '%s\n' "$SIGNATURE_INFO" | grep -q "^Identifier=xyz.taeyoon.stellar$" || fail "서명 identifier가 번들 ID가 아님"
APP_TEAM="$(printf '%s\n' "$SIGNATURE_INFO" | awk -F= '/^TeamIdentifier=/ {print $2}')"
for nested in \
  "Contents/Helpers/stellar" \
  "$SPARKLE_VERSION_DIR/XPCServices/Installer.xpc" \
  "$SPARKLE_VERSION_DIR/XPCServices/Downloader.xpc" \
  "$SPARKLE_VERSION_DIR/Autoupdate" \
  "$SPARKLE_VERSION_DIR/Updater.app" \
  "$SPARKLE_DIR"; do
  NESTED_INFO="$(codesign -dvv "$APP/$nested" 2>&1)"
  printf '%s\n' "$NESTED_INFO" | grep -q "flags=.*runtime" || fail "$nested: Hardened Runtime flag 없음"
  NESTED_TEAM="$(printf '%s\n' "$NESTED_INFO" | awk -F= '/^TeamIdentifier=/ {print $2}')"
  [ "$NESTED_TEAM" = "$APP_TEAM" ] || fail "$nested: TeamIdentifier($NESTED_TEAM)가 앱($APP_TEAM)과 다름"
done
# team 식별자는 출력하지 않습니다(저장소·로그 비기록 원칙, XVI.9.3).
echo "✓ 코드 서명 구조: verify --deep --strict, Sealed Resources v2, Info.plist bound, Hardened Runtime," \
  "CLI·Sparkle 중첩 코드 Hardened Runtime·앱과 같은 TeamIdentifier$( [ "$APP_TEAM" = "not set" ] && printf '(ad-hoc)')"

codesign --verify --strict "$APP/Contents/Helpers/stellar" || fail "CLI 서명 검증 실패"
codesign -d --entitlements - --xml "$APP/Contents/Helpers/stellar" > "$WORK/cli.entitlements" 2>/dev/null
[ ! -s "$WORK/cli.entitlements" ] || fail "CLI에 entitlement가 있습니다"
echo "✓ CLI: 독립 strict 서명 검증 통과, entitlement 없음"

# --- 7. 재배치 실행 smoke(개발 도구·빌드 트리 읽기 차단) ---
RUN="$WORK/run"
mkdir -p "$RUN/home"
ditto "$APP" "$RUN/Stellar.app"
DENY="(deny file-read* (subpath \"$REPO_ROOT\") (subpath \"$HOME/Library/Developer\")"
DENY="$DENY (subpath \"/Applications/Xcode.app\") (subpath \"/Applications/Xcode-beta.app\")"
DENY="$DENY (subpath \"/Library/Developer/CommandLineTools\") (subpath \"$HOME/.cargo\") (subpath \"$HOME/.rustup\"))"
PROFILE="(version 1)(allow default)$DENY"
# 차단 profile이 실제로 읽기를 막는지 먼저 확인합니다.
if sandbox-exec -p "$PROFILE" /bin/cat "$REPO_ROOT/Cargo.toml" >/dev/null 2>&1; then
  fail "sandbox 읽기 차단 profile이 동작하지 않습니다"
fi
env -i HOME="$RUN/home" PATH=/usr/bin:/bin TMPDIR="$RUN/home" \
  sandbox-exec -p "$PROFILE" "$RUN/Stellar.app/Contents/Helpers/stellar" --version > "$WORK/cli-version.json"
[ "$(plutil -extract result.name raw -o - "$WORK/cli-version.json")" = stellar-cli ] || fail "재배치 CLI version 실패"
for mode in --smoke --lifecycle-check --canvas-check --update-check; do
  env -i HOME="$RUN/home" PATH=/usr/bin:/bin TMPDIR="$RUN/home" \
    sandbox-exec -p "$PROFILE" "$RUN/Stellar.app/Contents/MacOS/Stellar" "$mode" \
    || fail "재배치 실행 $mode 실패"
done
echo "✓ 재배치 실행: 임시 폴더·최소 환경·개발 경로 읽기 차단에서 CLI --version 및 앱 --smoke/--lifecycle-check/--canvas-check/--update-check 통과"

echo "✓ bundle 검사 통과: $SOURCE_APP$( [ "$ADHOC" -eq 1 ] && printf ' (ad-hoc 서명 사본)')"
