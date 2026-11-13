#!/usr/bin/env bash
# macOS Release 빌드와 코드 서명.
#
# 사용법:
#   scripts/macos/sign_release.sh                # Developer ID Application으로 배포용 서명
#   scripts/macos/sign_release.sh --development  # Apple Development로 로컬 검증용 서명(배포 불가)
#
# 서명 identity:
#   - STELLAR_CODESIGN_IDENTITY가 있으면 그 값(SHA-1 해시 또는 identity 이름)을 씁니다.
#   - 없으면 로그인 키체인에서 해당 종류의 identity를 찾고, 정확히 하나일 때만 씁니다.
#   identity·team 정보는 저장소에 기록하지 않습니다(로컬 키체인과 환경 변수로만 전달).
#
# 산출물: apps/macos/build/Build/Products/Release/StellarMac.app (build/는 git ignore)
#
# 종료 코드: 0 성공, 64 사용법 오류, 69 서명 identity 없음/모호, 그 외 빌드·검증 실패.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
EXPECTED_BUNDLE_ID="xyz.taeyoon.stellar"

MODE="developer-id"
case "${1:-}" in
  "") ;;
  --development) MODE="development" ;;
  *) echo "사용법: $0 [--development]" >&2; exit 64 ;;
esac

if [ "$MODE" = "developer-id" ]; then
  IDENTITY_KIND="Developer ID Application"
  # 배포용 서명은 Apple timestamp를 포함해야 공증을 통과합니다.
  TIMESTAMP_FLAG="--timestamp"
else
  IDENTITY_KIND="Apple Development"
  TIMESTAMP_FLAG="--timestamp=none"
fi

# --- 서명 identity 해석 ---
if [ -n "${STELLAR_CODESIGN_IDENTITY:-}" ]; then
  IDENTITY="$STELLAR_CODESIGN_IDENTITY"
else
  MATCHES="$(security find-identity -v -p codesigning | grep -F "\"$IDENTITY_KIND:" || true)"
  COUNT="$(printf '%s' "$MATCHES" | grep -c . || true)"
  if [ "$COUNT" -eq 0 ]; then
    {
      echo "✖ 키체인에 \"$IDENTITY_KIND\" 서명 identity가 없습니다."
      if [ "$MODE" = "developer-id" ]; then
        echo "  Xcode ▸ Settings ▸ Accounts ▸ (Apple ID) ▸ Manage Certificates… ▸ + ▸"
        echo "  \"Developer ID Application\"으로 인증서를 만든 뒤 다시 실행하세요."
      fi
    } >&2
    exit 69
  fi
  if [ "$COUNT" -gt 1 ]; then
    echo "✖ \"$IDENTITY_KIND\" identity가 여러 개입니다. STELLAR_CODESIGN_IDENTITY로 하나를 지정하세요." >&2
    exit 69
  fi
  # 첫 열(번호) 다음의 SHA-1 해시로 지정합니다.
  IDENTITY="$(printf '%s\n' "$MATCHES" | awk '{print $2}')"
fi

# --- Xcode 선택(smoke.sh와 같은 규칙) ---
if ! xcodebuild -version >/dev/null 2>&1; then
  if [ -d /Applications/Xcode-beta.app ]; then
    export DEVELOPER_DIR=/Applications/Xcode-beta.app/Contents/Developer
  elif [ -d /Applications/Xcode.app ]; then
    export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
  fi
fi

# --- 빌드: Rust 코어 release staticlib → Xcode Release(서명은 아래에서 한 번만) ---
cargo build --manifest-path "$REPO_ROOT/Cargo.toml" -p stellar-ffi --release --locked --target aarch64-apple-darwin

DERIVED="$REPO_ROOT/apps/macos/build"
xcodebuild \
  -project "$REPO_ROOT/apps/macos/StellarMac.xcodeproj" \
  -scheme StellarMac \
  -configuration Release \
  -derivedDataPath "$DERIVED" \
  CODE_SIGNING_ALLOWED=NO \
  build

APP="$DERIVED/Build/Products/Release/StellarMac.app"
BIN="$APP/Contents/MacOS/StellarMac"

# --- 서명: Hardened Runtime(공증 필수) ---
# Rust 코어는 staticlib으로 실행 파일에 링크되어 중첩 서명 대상이 없습니다.
codesign --force --options runtime "$TIMESTAMP_FLAG" --sign "$IDENTITY" "$APP"

# --- 검증 ---
codesign --verify --deep --strict --verbose=2 "$APP"

ARCHS="$(lipo -archs "$BIN")"
if [ "$ARCHS" != "arm64" ]; then
  echo "✖ arm64 전용이어야 합니다: $ARCHS" >&2
  exit 1
fi

BUNDLE_ID="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$APP/Contents/Info.plist")"
if [ "$BUNDLE_ID" != "$EXPECTED_BUNDLE_ID" ]; then
  echo "✖ 번들 ID가 $EXPECTED_BUNDLE_ID 이어야 합니다: $BUNDLE_ID" >&2
  exit 1
fi

# pipefail에서 `grep -q` 조기 종료가 SIGPIPE 실패로 보이지 않도록 출력을 먼저 캡처합니다.
SIGNATURE_INFO="$(codesign -dvv "$APP" 2>&1)"
if ! printf '%s\n' "$SIGNATURE_INFO" | grep -q "flags=.*runtime"; then
  echo "✖ Hardened Runtime 플래그가 없습니다." >&2
  exit 1
fi

# 서명된 앱이 실제로 실행되는지 smoke(FFI로 코어 버전 읽기).
"$BIN" --smoke

echo "✓ 서명 완료($MODE): $APP"
if [ "$MODE" = "developer-id" ]; then
  echo "  다음 단계: scripts/macos/notarize.sh 로 공증과 stapling을 진행합니다."
else
  echo "  주의: Apple Development 서명은 로컬 검증용이며 배포할 수 없습니다."
fi
