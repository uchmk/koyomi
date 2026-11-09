#!/usr/bin/env bash
# The ACL tests: they check who can write what koyomi runs from, on the real permissions of this
# machine, so they are kept out of scripts/verify.sh (its `ALL OK` does not cover them). Run this
# on a machine where they are meant to pass, and report its result separately.
#
# KOYOMI_TEST_ACL turns the reload's access check on in the tests (see engine::ACL_TEST_ENV).
# Last line: `ACL OK: ...` only when every test passed; anything else is a failure, not a pass.
set -o pipefail
cd "$(dirname "$0")/.." || exit 1
out=$(KOYOMI_TEST_ACL=1 cargo test -q -p koyomi --all-features 2>&1)
rc=$?
if [ $rc -ne 0 ]; then
    echo "$out" | grep -E "panicked|^    engine::|test result" | head -40
    echo "ACL FAILED: $(echo "$out" | grep 'test result' | tail -1)"
    exit 1
fi
echo "ACL OK: $(echo "$out" | grep 'test result' | tail -1)"
