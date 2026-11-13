#!/usr/bin/env bash
# macOS Release 빌드와 코드 서명(XVI.9.3).
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
# 순서: 저장소 서명 자료 부재 확인 → scripts/macos/release_build.sh(Release 빌드·구성·번들 검사) → 서명 →
#       codesign --verify --deep --strict → Developer ID 체인·timestamp 확인 → scripts/macos/bundle_check.sh(서명본)
#
# 산출물: apps/macos/build/Build/Products/Release/StellarMac.app (build/는 git ignore)
#
# 종료 코드: 0 성공, 64 사용법 오류, 69 서명 identity 없음/모호, 그 외 빌드·검증 실패.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

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

# --- 서명 자료는 저장소에 두지 않습니다(XVI.9.3): 추적 파일에 인증서·키·profile이 있으면 중단 ---
if git -C "$REPO_ROOT" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  TRACKED_SECRETS="$(git -C "$REPO_ROOT" ls-files \
    | grep -Ei '\.(p12|pfx|cer|crt|der|pem|key|p8|keychain|keychain-db|mobileprovision|provisionprofile)$' || true)"
  if [ -n "$TRACKED_SECRETS" ]; then
    echo "✖ 저장소에 서명 자료로 보이는 파일이 추적되고 있습니다:" >&2
    printf '  %s\n' "$TRACKED_SECRETS" >&2
    exit 1
  fi
fi

# --- 빌드와 배포 구성 검사: 서명하지 않은 Release(arm64 전용·macOS 26·번들 ID, 번들 구조) ---
"$REPO_ROOT/scripts/macos/release_build.sh"

APP="$REPO_ROOT/apps/macos/build/Build/Products/Release/StellarMac.app"
BIN="$APP/Contents/MacOS/StellarMac"

# --- 서명: Hardened Runtime(공증 필수) ---
# Rust 코어는 staticlib으로 실행 파일에 링크되어 중첩 서명 대상이 없습니다(bundle_check.sh가 확인).
codesign --force --options runtime "$TIMESTAMP_FLAG" --sign "$IDENTITY" "$APP"

# --- 검증 ---
codesign --verify --deep --strict --verbose=2 "$APP"

# pipefail에서 `grep -q` 조기 종료가 SIGPIPE 실패로 보이지 않도록 출력을 먼저 캡처합니다.
SIGNATURE_INFO="$(codesign -dvv "$APP" 2>&1)"
if ! printf '%s\n' "$SIGNATURE_INFO" | grep -q "flags=.*runtime"; then
  echo "✖ Hardened Runtime 플래그가 없습니다." >&2
  exit 1
fi
if [ "$MODE" = "developer-id" ]; then
  # 배포 서명: Developer ID Application 인증서 체인, Apple 보안 timestamp, team 식별자.
  if ! printf '%s\n' "$SIGNATURE_INFO" | grep -q "^Authority=Developer ID Application:"; then
    echo "✖ Developer ID Application 인증서로 서명되지 않았습니다." >&2
    exit 1
  fi
  if ! printf '%s\n' "$SIGNATURE_INFO" | grep -q "^Authority=Developer ID Certification Authority"; then
    echo "✖ Developer ID 인증서 체인이 없습니다." >&2
    exit 1
  fi
  if ! printf '%s\n' "$SIGNATURE_INFO" | grep -q "^Timestamp="; then
    echo "✖ Apple 보안 timestamp가 없습니다." >&2
    exit 1
  fi
  if printf '%s\n' "$SIGNATURE_INFO" | grep -q "^TeamIdentifier=not set"; then
    echo "✖ TeamIdentifier가 없습니다." >&2
    exit 1
  fi
fi

# 서명된 번들의 구조·rpath·서명 구조와 재배치 실행 smoke(XVI.9.2 검사를 서명본에 그대로 적용).
"$REPO_ROOT/scripts/macos/bundle_check.sh" "$APP"

# 서명된 앱이 실제로 실행되는지 smoke(FFI로 코어 버전 읽기).
"$BIN" --smoke

echo "✓ 서명 완료($MODE): $APP"
if [ "$MODE" = "developer-id" ]; then
  echo "  다음 단계: scripts/macos/notarize.sh 로 공증과 stapling을 진행합니다."
else
  echo "  주의: Apple Development 서명은 로컬 검증용이며 배포할 수 없습니다."
fi
