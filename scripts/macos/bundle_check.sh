#!/usr/bin/env bash
# StellarMac.app 번들 구조·링크·rpath·코드 서명 구조 검사와 재배치 실행 smoke(XVI.9.2).
#
# 사용법:
#   scripts/macos/bundle_check.sh --adhoc-sign APP   # 서명 안 된 앱: 임시 사본에 ad-hoc(Hardened Runtime) 서명 후 검사
#   scripts/macos/bundle_check.sh APP                # 이미 서명된 앱(예: Developer ID)을 그대로 검사
#
# 검사(하나라도 어긋나면 실패):
#  1. 번들 구조 allowlist: Contents/{Info.plist, PkgInfo, MacOS/StellarMac, _CodeSignature/CodeResources,
#     Resources/**}만 허용(빌드 중간물·dSYM·헤더·staticlib 혼입 금지)
#  2. Mach-O 파일은 Contents/MacOS/StellarMac 하나(Rust 코어는 staticlib으로 실행 파일에 포함)
#  3. 동적 링크는 시스템 경로(/usr/lib, /System/Library)만, @rpath·빌드 경로 의존 없음
#  4. LC_RPATH는 없거나 @executable_path/@loader_path 상대 경로만
#  5. C ABI 헤더(stellar_ffi.h)의 모든 stellar_* 함수가 실행 파일에 정의됨(Rust 코어 포함 확인)
#  6. 코드 서명: codesign --verify --deep --strict, CodeResources 존재, Sealed Resources v2, Info.plist bound,
#     Hardened Runtime flag
#  7. 재배치 실행: 번들을 임시 폴더로 복사해 최소 환경(env -i)과 sandbox 읽기 차단(저장소·DerivedData·Xcode·
#     CommandLineTools·~/.cargo·~/.rustup)에서 --smoke, --lifecycle-check, --canvas-check
#
# 종료 코드: 0 성공, 64 사용법 오류, 그 외 검사 실패.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
HEADER="$REPO_ROOT/crates/ffi/include/stellar_ffi.h"

ADHOC=0
if [ "${1:-}" = "--adhoc-sign" ]; then
  ADHOC=1
  shift
fi
if [ "$#" -ne 1 ] || [ ! -d "$1" ]; then
  echo "사용법: $0 [--adhoc-sign] StellarMac.app" >&2
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
APP="$WORK/sign/StellarMac.app"
mkdir -p "$WORK/sign"
ditto "$SOURCE_APP" "$APP"
BIN="$APP/Contents/MacOS/StellarMac"
[ -x "$BIN" ] || fail "실행 파일 없음"

# --- 1. 번들 구조 allowlist ---
while IFS= read -r entry; do
  case "$entry" in
    Contents | Contents/Info.plist | Contents/PkgInfo | Contents/MacOS | Contents/MacOS/StellarMac) ;;
    Contents/_CodeSignature | Contents/_CodeSignature/CodeResources) ;;
    Contents/Resources | Contents/Resources/*) ;;
    *) fail "허용되지 않은 번들 항목: $entry" ;;
  esac
done < <(cd "$APP" && find Contents -mindepth 0 | LC_ALL=C sort)
echo "✓ 번들 구조: allowlist 통과"

# --- 2. Mach-O는 실행 파일 하나 ---
MACHO_FILES="$(cd "$APP" && find Contents -type f -exec file {} + | grep "Mach-O" | cut -d: -f1 || true)"
[ "$MACHO_FILES" = "Contents/MacOS/StellarMac" ] || fail "Mach-O 파일 목록: $MACHO_FILES"
echo "✓ Mach-O: Contents/MacOS/StellarMac 하나(중첩 코드 없음)"

# --- 3. 동적 링크는 시스템 경로만 ---
LINKED="$(otool -L "$BIN" | tail -n +2 | awk '{print $1}')"
while IFS= read -r lib; do
  case "$lib" in
    /usr/lib/* | /System/Library/*) ;;
    *) fail "시스템 밖 동적 라이브러리 의존: $lib" ;;
  esac
done <<< "$LINKED"
echo "✓ 동적 링크: 시스템 라이브러리 $(printf '%s\n' "$LINKED" | grep -c .)개만"

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
if otool -l "$BIN" | grep -qF "$REPO_ROOT"; then
  fail "load command에 빌드 경로($REPO_ROOT)가 있습니다"
fi
echo "✓ rpath: $( [ -n "$RPATHS" ] && printf '%s' "$RPATHS" | tr '\n' ' ' || printf '없음')"

# --- 5. Rust 코어 C ABI가 실행 파일에 정적으로 포함 ---
DEFINED="$(nm -gU "$BIN" | awk '{print $NF}')"
ABI_FUNCTIONS="$(grep -oE '\bstellar_[a-z_]+\(' "$HEADER" | tr -d '(' | sort -u)"
[ -n "$ABI_FUNCTIONS" ] || fail "헤더에서 C ABI 함수를 찾지 못함"
ABI_COUNT=0
while IFS= read -r symbol; do
  printf '%s\n' "$DEFINED" | grep -qx "_$symbol" || fail "실행 파일에 $symbol 정의 없음"
  ABI_COUNT=$((ABI_COUNT + 1))
done <<< "$ABI_FUNCTIONS"
echo "✓ Rust 코어: C ABI 함수 ${ABI_COUNT}개가 실행 파일에 정의됨(staticlib 포함)"

# --- 6. 코드 서명 구조 ---
if [ "$ADHOC" -eq 1 ]; then
  codesign --force --options runtime --timestamp=none --sign - "$APP"
fi
[ -f "$APP/Contents/_CodeSignature/CodeResources" ] || fail "_CodeSignature/CodeResources 없음(서명되지 않은 번들)"
codesign --verify --deep --strict --verbose=2 "$APP"
SIGNATURE_INFO="$(codesign -dvv "$APP" 2>&1)"
printf '%s\n' "$SIGNATURE_INFO" | grep -q "^Sealed Resources version=2" || fail "Sealed Resources v2 아님"
printf '%s\n' "$SIGNATURE_INFO" | grep -q "^Info.plist entries=" || fail "Info.plist가 서명에 bound되지 않음"
printf '%s\n' "$SIGNATURE_INFO" | grep -q "flags=.*runtime" || fail "Hardened Runtime flag 없음"
printf '%s\n' "$SIGNATURE_INFO" | grep -q "^Identifier=xyz.taeyoon.stellar$" || fail "서명 identifier가 번들 ID가 아님"
echo "✓ 코드 서명 구조: verify --deep --strict, Sealed Resources v2, Info.plist bound, Hardened Runtime"

# --- 7. 재배치 실행 smoke(개발 도구·빌드 트리 읽기 차단) ---
RUN="$WORK/run"
mkdir -p "$RUN/home"
ditto "$APP" "$RUN/StellarMac.app"
DENY="(deny file-read* (subpath \"$REPO_ROOT\") (subpath \"$HOME/Library/Developer\")"
DENY="$DENY (subpath \"/Applications/Xcode.app\") (subpath \"/Applications/Xcode-beta.app\")"
DENY="$DENY (subpath \"/Library/Developer/CommandLineTools\") (subpath \"$HOME/.cargo\") (subpath \"$HOME/.rustup\"))"
PROFILE="(version 1)(allow default)$DENY"
# 차단 profile이 실제로 읽기를 막는지 먼저 확인합니다.
if sandbox-exec -p "$PROFILE" /bin/cat "$REPO_ROOT/Cargo.toml" >/dev/null 2>&1; then
  fail "sandbox 읽기 차단 profile이 동작하지 않습니다"
fi
for mode in --smoke --lifecycle-check --canvas-check; do
  env -i HOME="$RUN/home" PATH=/usr/bin:/bin TMPDIR="$RUN/home" \
    sandbox-exec -p "$PROFILE" "$RUN/StellarMac.app/Contents/MacOS/StellarMac" "$mode" \
    || fail "재배치 실행 $mode 실패"
done
echo "✓ 재배치 실행: 임시 폴더·최소 환경·개발 경로 읽기 차단에서 --smoke/--lifecycle-check/--canvas-check 통과"

echo "✓ bundle 검사 통과: $SOURCE_APP$( [ "$ADHOC" -eq 1 ] && printf ' (ad-hoc 서명 사본)')"
