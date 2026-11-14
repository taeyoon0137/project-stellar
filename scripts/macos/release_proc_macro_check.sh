#!/usr/bin/env bash
# Xcode 밖 release cargo 빌드의 proc-macro 회귀 검사(XVI.9.1 후속, 리뷰 MF-1 후속).
#
# MACOSX_DEPLOYMENT_TARGET이 12.0 이상이면 host 산출물(proc-macro dylib)이 chained fixups로 링크됩니다. 여기에 release
# profile 기본 strip(debuginfo)이 걸리면 dylib가 깨져 새 target에서 rustc가 proc-macro를 읽지 못합니다(E0463). Xcode phase는
# 환경 변수로 이 strip을 끄지만, 터미널·CI처럼 Xcode 밖에서 같은 변수를 설정한 release 빌드는 루트 Cargo.toml의
# [profile.release.build-override] strip 설정만 믿을 수 있습니다. 이 검사는 profile override 환경 변수(CARGO_PROFILE_*)를
# 모두 지운 채, 비어 있는 임시 CARGO_TARGET_DIR에서 앱의 release staticlib을 빌드해 그 설정만으로 충분한지 확인합니다.
#
# 사용법: scripts/macos/release_proc_macro_check.sh [MACOSX_DEPLOYMENT_TARGET]   (기본 26.0, 앱의 최소 macOS)
# 확인: cargo build -p stellar-ffi --release --locked --target aarch64-apple-darwin 종료 0, arm64 staticlib 생성.
# 종료 코드: 0 성공, 64 사용법 오류, 그 외 실패.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

usage() {
  echo "사용법: $0 [MACOSX_DEPLOYMENT_TARGET]" >&2
  exit 64
}

[ "$#" -le 1 ] || usage
DEPLOYMENT_TARGET="${1:-26.0}"
[[ "$DEPLOYMENT_TARGET" =~ ^[0-9]+\.[0-9]+$ ]] || usage

fail() {
  echo "✖ release proc-macro 검사 실패: $*" >&2
  exit 1
}

export PATH="$HOME/.cargo/bin:$PATH"
command -v cargo >/dev/null 2>&1 || fail "cargo not found in PATH"

# 환경 변수로 profile을 바꾸면 Cargo.toml 설정을 검사하지 못하므로 CARGO_PROFILE_*를 모두 지웁니다.
while IFS= read -r name; do
  unset "$name"
done < <(env | awk -F= '/^CARGO_PROFILE_/ { print $1 }')

RUST_TARGET="aarch64-apple-darwin"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
TARGET_DIR="$WORK/target"
LOG="$WORK/cargo.log"

if ! (cd "$REPO_ROOT" && MACOSX_DEPLOYMENT_TARGET="$DEPLOYMENT_TARGET" CARGO_TARGET_DIR="$TARGET_DIR" \
  cargo build -p stellar-ffi --release --locked --target "$RUST_TARGET") >"$LOG" 2>&1; then
  tail -n 40 "$LOG" >&2
  fail "새 target에서 release cargo 빌드 실패(MACOSX_DEPLOYMENT_TARGET=$DEPLOYMENT_TARGET)"
fi

CORE_LIB="$TARGET_DIR/$RUST_TARGET/release/libstellar_ffi.a"
[ -f "$CORE_LIB" ] || fail "staticlib 없음: $CORE_LIB"
LIB_ARCHS="$(lipo -archs "$CORE_LIB")"
[ "$LIB_ARCHS" = "arm64" ] || fail "Rust 코어 아키텍처: $LIB_ARCHS"
echo "✓ release proc-macro: Xcode 밖 새 target에서 성공, MACOSX_DEPLOYMENT_TARGET=$DEPLOYMENT_TARGET, staticlib $LIB_ARCHS"
