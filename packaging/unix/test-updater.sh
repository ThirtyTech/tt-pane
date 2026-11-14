#!/bin/sh
# Exercises the embedded macOS/Linux restart helper (src/update_unix.sh) with
# disposable executables: no network, no real install.
#
#   packaging/unix/test-updater.sh
set -eu
export TTPANE_UPDATER_TEST=1

here=$(cd "$(dirname "$0")" && pwd)
helper="$here/../../src/update_unix.sh"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

fail() {
    echo "test-updater: $*" >&2
    exit 1
}

# A fake tt-pane. A working one (status 0) records its arguments; a broken
# one exits with that status at once.
fixture() {
    if [ "$2" = 0 ]; then
        printf '#!/bin/sh\nprintf "%%s\\n" "$@" > "$0.launched"\n' > "$1"
    else
        printf '#!/bin/sh\nexit %s\n' "$2" > "$1"
    fi
    chmod 755 "$1"
}

# Sets up <case>/install/tt-pane (new) with its backup (old) and a running
# "tt-pane" (a sleep), then runs the helper and waits for it.
run_case() {
    case_dir="$work/$1"
    mkdir -p "$case_dir/root" "$case_dir/install/.tt-pane-backup-x"
    target="$case_dir/install/tt-pane"
    backup="$case_dir/install/.tt-pane-backup-x/tt-pane"
    fixture "$target" "$2"
    fixture "$backup" 0
    sleep 1 &
    running=$!
    /bin/sh "$helper" "$case_dir/root" "$running" "$target" "$case_dir/my config.toml" \
        "$case_dir/install/.tt-pane-backup-x" "$backup" binary &
    helper_pid=$!
    tries=0
    until [ -e "$case_dir/root/ready" ]; do
        tries=$((tries + 1))
        [ "$tries" -le 100 ] || fail "$1: helper never became ready"
        sleep 0.1
    done
    # The new version must not start before the running one exits.
    [ ! -e "$target.launched" ] || fail "$1: started before the old version exited"
    wait "$running"
    status=0
    wait "$helper_pid" || status=$?
}

run_case success 0
[ "$status" = 0 ] || fail "success: helper exited with $status"
[ "$(cat "$target.launched")" = "--config
$case_dir/my config.toml" ] || fail "success: wrong arguments: $(cat "$target.launched")"
[ ! -e "$case_dir/install/.tt-pane-backup-x" ] || fail "success: backup kept"
[ ! -e "$case_dir/root" ] || fail "success: staging directory kept"

run_case broken 3
[ "$status" = 1 ] || fail "broken: helper exited with $status"
# The restored old version is back at the install path and was started.
[ -e "$target.launched" ] || fail "broken: restored version was not started"
! grep -q 'exit' "$target" || fail "broken: previous version was not restored"
grep -q 'previous version was restored' "$case_dir/root/error.txt" || fail "broken: no error report"
[ ! -e "$case_dir/install/.tt-pane-backup-x" ] || fail "broken: backup directory kept"

echo "test-updater: ok"
