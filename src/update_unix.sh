# Embedded in tt-pane, staged locally and run by /bin/sh on macOS and Linux:
#
#   update.sh <root> <pid> <target> <config> <backup-dir> <backup> <bundle|binary>
#
# tt-pane has already swapped the new version in at <target> and kept the old
# one at <backup>. This waits for tt-pane (<pid>) to exit, starts the new
# version with --config <config>, and if it does not start puts the old version
# back and starts that instead.
root=$1 pid=$2 target=$3 config=$4 backup_dir=$5 backup=$6 kind=$7

launch() {
    if [ "$kind" = bundle ]; then
        # Through Launch Services, so macOS treats it as the app (Accessibility).
        /usr/bin/open -n "$target" --args --config "$config"
        return
    fi
    nohup "$target" --config "$config" </dev/null >/dev/null 2>&1 &
    child=$!
    sleep 3
    # Still running, or exited successfully (another instance took over).
    kill -0 "$child" 2>/dev/null && return 0
    wait "$child"
}

report() {
    printf '%s\n' "$1" > "$root/error.txt"
    message="$1 Details: $root"
    # packaging/unix/test-updater.sh runs without a desktop to show it on.
    [ -z "${TTPANE_UPDATER_TEST:-}" ] || return 0
    if [ "$kind" = bundle ] || [ "$(uname -s)" = Darwin ]; then
        /usr/bin/osascript -e 'on run argv' -e 'display alert "tt-pane update failed" message (item 1 of argv)' -e 'end run' "$message" >/dev/null 2>&1
    elif command -v notify-send >/dev/null 2>&1; then
        notify-send "tt-pane update failed" "$message" >/dev/null 2>&1
    fi
}

: > "$root/ready" || exit 1

tries=0
while kill -0 "$pid" 2>/dev/null; do
    tries=$((tries + 1))
    if [ "$tries" -gt 600 ]; then
        # The new version is in place; it starts the next time tt-pane does.
        rm -rf "$backup_dir"
        printf '%s\n' "tt-pane did not exit; the update applies on its next start." > "$root/error.txt"
        exit 1
    fi
    sleep 0.1
done

if launch; then
    rm -rf "$backup_dir" "$root"
    exit 0
fi

if [ -e "$backup" ] && mv "$target" "$backup_dir/failed" && mv "$backup" "$target"; then
    rm -rf "$backup_dir"
    if launch; then
        report "The new version did not start, so the previous version was restored."
    else
        report "The new version did not start and neither did the restored previous version."
    fi
else
    report "The new version did not start and the previous version could not be restored from $backup."
fi
exit 1
