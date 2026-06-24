#!/usr/bin/env bash
#
# check-commit-subjects.sh (I.4.4)
#   기준 commit 이후 모든 non-merge commit subject를 `cargo xtask validate-commit-message`로
#   검사합니다. 예외 prefix는 없습니다(Task(<WBS-ID>): <제목>만 허용).
#
#   범위 결정:
#     - PR: origin/<base>..HEAD
#     - BASELINE_REF 설정 시: <BASELINE_REF>..HEAD
#     - 그 외: 직전 commit 1개
#
set -euo pipefail

if [ -n "${GITHUB_BASE_REF:-}" ]; then
  git fetch --no-tags --depth=200 origin "$GITHUB_BASE_REF" >/dev/null 2>&1 || true
  range="origin/${GITHUB_BASE_REF}..HEAD"
elif [ -n "${BASELINE_REF:-}" ]; then
  range="${BASELINE_REF}..HEAD"
else
  range="HEAD~1..HEAD"
fi

rc=0
while IFS= read -r subject; do
  [ -n "$subject" ] || continue
  if ! cargo xtask validate-commit-message "$subject" >/dev/null 2>&1; then
    echo "✖ 잘못된 commit subject: $subject" >&2
    rc=1
  fi
done < <(git log --no-merges --format='%s' "$range")

if [ "$rc" -eq 0 ]; then
  echo "✓ commit subject 검사 통과 ($range)"
fi
exit "$rc"
