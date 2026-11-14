#!/usr/bin/env bash
# Xcode "Build Rust core (cargo)" build phase 회귀 검사(XVI.9.1 후속, 리뷰 MF-1).
#
# Xcode는 build setting을 환경 변수로 script phase에 넘깁니다(MACOSX_DEPLOYMENT_TARGET=26.0 등). 이미 채워진 Rust
# target에서는 host 산출물(build script·proc-macro)을 다시 링크하지 않으므로, 이 환경에서만 나는 실패가 가려집니다.
# 그래서 이 검사는 project.pbxproj의 phase 스크립트를 그대로 꺼내 Stellar target의 build setting을 환경으로 넣고,
# 비어 있는 임시 CARGO_TARGET_DIR에서 실행합니다(Xcode처럼 현재 환경 위에 build setting을 얹습니다).
#
# 사용법: scripts/macos/rust_core_phase_check.sh [Release|Debug]   (기본 Release)
# 확인: phase 종료 0, staticlib 생성, Release는 arm64 전용 staticlib.
# 종료 코드: 0 성공, 64 사용법 오류, 그 외 실패.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

if [ "$#" -gt 1 ]; then
  echo "사용법: $0 [Release|Debug]" >&2
  exit 64
fi
CONFIGURATION="${1:-Release}"
case "$CONFIGURATION" in
  Release | Debug) ;;
  *) echo "사용법: $0 [Release|Debug]" >&2; exit 64 ;;
esac

# --- Xcode 선택(smoke.sh와 같은 규칙) ---
if ! xcodebuild -version >/dev/null 2>&1; then
  if [ -d /Applications/Xcode-beta.app ]; then
    export DEVELOPER_DIR=/Applications/Xcode-beta.app/Contents/Developer
  elif [ -d /Applications/Xcode.app ]; then
    export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
  fi
fi

fail() {
  echo "✖ Rust 코어 phase 검사 실패: $*" >&2
  exit 1
}

PROJECT="$REPO_ROOT/apps/macos/Stellar.xcodeproj"
PBXPROJ="$PROJECT/project.pbxproj"
TARGET_NAME="Stellar"
# project.pbxproj의 "Build Rust core (cargo)" PBXShellScriptBuildPhase object ID입니다. 이름이 다르면 ID가 바뀐 것이므로
# 조용히 다른 phase를 검사하지 않고 실패합니다.
PHASE_ID="AA0000000000000000000010"
PHASE_NAME="$(plutil -extract "objects.$PHASE_ID.name" raw -o - "$PBXPROJ" 2>/dev/null || true)"
[ "$PHASE_NAME" = "Build Rust core (cargo)" ] || fail "phase object $PHASE_ID 이름이 다름: '$PHASE_NAME'"
PHASE_SCRIPT="$(plutil -extract "objects.$PHASE_ID.shellScript" raw -o - "$PBXPROJ")"
PHASE_SHELL="$(plutil -extract "objects.$PHASE_ID.shellPath" raw -o - "$PBXPROJ")"

# --- phase 환경: Stellar target의 build setting(release_build.sh와 같은 package 옵션) ---
SETTINGS="$(xcodebuild -project "$PROJECT" -scheme Stellar -configuration "$CONFIGURATION" \
  -packageAuthorizationProvider netrc -disableAutomaticPackageResolution \
  -derivedDataPath "$REPO_ROOT/apps/macos/build" -showBuildSettings 2>/dev/null)"
PHASE_ENV=()
IN_TARGET=0
while IFS= read -r line; do
  case "$line" in
    "Build settings for action build and target $TARGET_NAME:") IN_TARGET=1; continue ;;
    "Build settings for "*) IN_TARGET=0; continue ;;
  esac
  [ "$IN_TARGET" -eq 1 ] || continue
  # '    KEY = value' 줄만 씁니다. 환경 변수 이름이 될 수 없는 key는 건너뜁니다.
  case "$line" in "    "*" = "*) ;; *) continue ;; esac
  line="${line#    }"
  key="${line%% = *}"
  case "$key" in "" | [0-9]* | *[!A-Za-z0-9_]*) continue ;; esac
  PHASE_ENV+=("$key=${line#* = }")
done <<<"$SETTINGS"

DEPLOYMENT_TARGET=""
for entry in ${PHASE_ENV[@]+"${PHASE_ENV[@]}"}; do
  case "$entry" in MACOSX_DEPLOYMENT_TARGET=*) DEPLOYMENT_TARGET="${entry#*=}" ;; esac
done
# build setting을 읽지 못하면 검사가 Xcode 환경 없이 통과할 수 있으므로 실패합니다.
[ -n "$DEPLOYMENT_TARGET" ] || fail "$TARGET_NAME $CONFIGURATION build setting에서 MACOSX_DEPLOYMENT_TARGET을 읽지 못함"

# --- 비어 있는 target 디렉터리에서 phase 실행 ---
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
TARGET_DIR="$WORK/target"
LOG="$WORK/phase.log"
if ! env "${PHASE_ENV[@]}" CARGO_TARGET_DIR="$TARGET_DIR" "$PHASE_SHELL" -c "$PHASE_SCRIPT" >"$LOG" 2>&1; then
  tail -n 40 "$LOG" >&2
  fail "새 target에서 phase 스크립트 실패($CONFIGURATION, MACOSX_DEPLOYMENT_TARGET=$DEPLOYMENT_TARGET)"
fi

if [ "$CONFIGURATION" = "Release" ]; then
  CORE_LIB="$TARGET_DIR/aarch64-apple-darwin/release/libstellar_ffi.a"
else
  CORE_LIB="$TARGET_DIR/debug/libstellar_ffi.a"
fi
[ -f "$CORE_LIB" ] || fail "staticlib 없음: $CORE_LIB"
LIB_ARCHS="$(lipo -archs "$CORE_LIB")"
if [ "$CONFIGURATION" = "Release" ] && [ "$LIB_ARCHS" != "arm64" ]; then
  fail "Release Rust 코어 아키텍처: $LIB_ARCHS"
fi
echo "✓ Rust 코어 phase($CONFIGURATION): 새 target에서 성공, MACOSX_DEPLOYMENT_TARGET=$DEPLOYMENT_TARGET, staticlib $LIB_ARCHS"
