#!/usr/bin/env bash
# Stellar.app을 안쪽 코드부터 서명합니다(XVI.9.2·XVI.9.3, ADR-0017).
#
# 사용법:
#   scripts/macos/codesign_bundle.sh IDENTITY TIMESTAMP_FLAG APP [APP_ENTITLEMENTS]
#     IDENTITY          codesign identity(SHA-1 해시·이름, ad-hoc은 "-")
#     TIMESTAMP_FLAG    --timestamp(배포·공증) 또는 --timestamp=none(ad-hoc·로컬 검증)
#     APP_ENTITLEMENTS  앱 실행 파일 서명에만 붙일 entitlements(선택). 배포 서명에는 쓰지 않습니다.
#
# 서명 순서(Sparkle 배포 문서와 같은 순서, 모두 Hardened Runtime):
#   Sparkle.framework/Versions/B/XPCServices/Installer.xpc
#   Sparkle.framework/Versions/B/XPCServices/Downloader.xpc (entitlements 보존)
#   Sparkle.framework/Versions/B/Autoupdate
#   Sparkle.framework/Versions/B/Updater.app
#   Sparkle.framework
#   Stellar.app
# 공증은 모든 중첩 코드가 같은 Developer ID·Hardened Runtime·secure timestamp로 서명되어야 통과합니다.
# `codesign --deep`으로 서명하지 않습니다(중첩 코드별 옵션을 잃음).
#
# 종료 코드: 0 성공, 64 사용법 오류, 그 외 codesign 실패.
set -euo pipefail

if [ "$#" -lt 3 ] || [ "$#" -gt 4 ] || [ ! -d "$3" ]; then
  echo "사용법: $0 IDENTITY TIMESTAMP_FLAG Stellar.app [APP_ENTITLEMENTS]" >&2
  exit 64
fi
IDENTITY="$1"
TIMESTAMP_FLAG="$2"
APP="$3"
APP_ENTITLEMENTS="${4:-}"
if [ -n "$APP_ENTITLEMENTS" ] && [ ! -f "$APP_ENTITLEMENTS" ]; then
  echo "entitlements 파일 없음: $APP_ENTITLEMENTS" >&2
  exit 64
fi
case "$TIMESTAMP_FLAG" in
  --timestamp | --timestamp=none) ;;
  *) echo "TIMESTAMP_FLAG는 --timestamp 또는 --timestamp=none: $TIMESTAMP_FLAG" >&2; exit 64 ;;
esac

sign() {
  codesign --force --options runtime "$TIMESTAMP_FLAG" --sign "$IDENTITY" "$@"
}

SPARKLE="$APP/Contents/Frameworks/Sparkle.framework"
if [ -d "$SPARKLE" ]; then
  VERSION_DIR="$SPARKLE/Versions/B"
  for nested in \
    "$VERSION_DIR/XPCServices/Installer.xpc" \
    "$VERSION_DIR/XPCServices/Downloader.xpc" \
    "$VERSION_DIR/Autoupdate" \
    "$VERSION_DIR/Updater.app"; do
    [ -e "$nested" ] || { echo "Sparkle 중첩 코드 없음: $nested" >&2; exit 1; }
  done
  sign "$VERSION_DIR/XPCServices/Installer.xpc"
  sign --preserve-metadata=entitlements "$VERSION_DIR/XPCServices/Downloader.xpc"
  sign "$VERSION_DIR/Autoupdate"
  sign "$VERSION_DIR/Updater.app"
  sign "$SPARKLE"
fi
if [ -n "$APP_ENTITLEMENTS" ]; then
  sign --entitlements "$APP_ENTITLEMENTS" "$APP"
else
  sign "$APP"
fi
