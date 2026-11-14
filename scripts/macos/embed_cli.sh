#!/usr/bin/env bash
# Xcode가 앱 서명 전에 L1 CLI를 실행 가능한 중첩 코드로 배치합니다(ADR-0005 §7).
set -euo pipefail
export PATH="$HOME/.cargo/bin:$PATH"
cd "$SRCROOT/../.."
# rust_core_phase_check/release_build와 같은 Cargo target 디렉터리 규칙을 사용합니다.
target_dir="$(cargo metadata --format-version 1 --no-deps --locked | /usr/bin/plutil -extract target_directory raw -o - -)"
if [[ "$CONFIGURATION" == Release ]]; then
  CARGO_PROFILE_RELEASE_BUILD_OVERRIDE_STRIP=none cargo build --locked --release --target aarch64-apple-darwin -p stellar-cli
  cli="$target_dir/aarch64-apple-darwin/release/stellar"
else
  cargo build --locked -p stellar-cli
  cli="$target_dir/debug/stellar"
fi
helpers="$TARGET_BUILD_DIR/$CONTENTS_FOLDER_PATH/Helpers"
mkdir -p "$helpers"
/usr/bin/install -m 755 "$cli" "$helpers/stellar"
