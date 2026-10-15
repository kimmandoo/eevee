#!/bin/sh
# Install, update, or remove verified native Eevee releases without profile edits.
set -eu
# Remember whether the user's terminal speaks UTF-8 before forcing the C locale.
user_locale=${LC_ALL:-${LC_CTYPE:-${LANG:-}}}
LC_ALL=C
export LC_ALL

animation_pid=
download_pid=
terminal_active=0

fail() {
    if [ -n "$animation_pid" ]; then stop_animation; fi
    printf 'ev installer: %s\n' "$*" >&2
    exit 1
}

usage() {
    cat <<'USAGE'
Usage: sh install.sh [--version vX.Y.Z] [--install-dir PATH]
       sh install.sh --archive PATH --checksums PATH [--install-dir PATH]
       sh install.sh --uninstall [--install-dir PATH]

Download the latest release for this Mac or Linux machine, or select an exact
release with --version / EV_VERSION. HTTPS downloads require curl.
Online installation also downloads nine hash-pinned original sprites to the user
cache. A failed sprite download stops publication; rerun resumes completed files.
Offline installation requires both --archive and --checksums (SHA256SUMS);
it makes no network requests and does not populate a missing sprite cache.
--install-dir must be absolute; it defaults to EV_INSTALL_DIR or $HOME/.local/bin.
Rerun to update an installer-owned executable. --uninstall removes only verified
owned executable/metadata, keeping all task, provider, and cache data.
No sudo, shell profile changes, or task/provider data changes are performed.
USAGE
}

archive=
checksums=
install_dir=${EV_INSTALL_DIR:-}
archive_set=0
checksums_set=0
dir_set=0
selected_tag=${EV_VERSION:-}
version_set=0
uninstall=0
while [ "$#" -gt 0 ]; do
    case "$1" in
        --archive)
            [ "$#" -ge 2 ] && [ -n "$2" ] || { usage >&2; fail '--archive requires a path'; }
            [ "$archive_set" -eq 0 ] || fail 'provide --archive only once'
            archive=$2; archive_set=1; shift 2 ;;
        --checksums)
            [ "$#" -ge 2 ] && [ -n "$2" ] || { usage >&2; fail '--checksums requires a path'; }
            [ "$checksums_set" -eq 0 ] || fail 'provide --checksums only once'
            checksums=$2; checksums_set=1; shift 2 ;;
        --install-dir)
            [ "$#" -ge 2 ] && [ -n "$2" ] || { usage >&2; fail '--install-dir requires a path'; }
            [ "$dir_set" -eq 0 ] || fail 'provide --install-dir only once'
            install_dir=$2; dir_set=1; shift 2 ;;
        --version)
            [ "$#" -ge 2 ] && [ -n "$2" ] || { usage >&2; fail '--version requires vX.Y.Z'; }
            [ "$version_set" -eq 0 ] || fail 'provide --version only once'
            selected_tag=$2; version_set=1; shift 2 ;;
        --uninstall)
            [ "$uninstall" -eq 0 ] || fail 'provide --uninstall only once'
            uninstall=1; shift ;;
        -h | --help) usage; exit 0 ;;
        *) usage >&2; fail "unknown argument: $1" ;;
    esac
done
[ "$archive_set" -eq "$checksums_set" ] || fail 'provide both --archive PATH and --checksums PATH'
if [ "$uninstall" -eq 1 ]; then
    [ "$archive_set" -eq 0 ] && [ "$version_set" -eq 0 ] || fail '--uninstall cannot be combined with archive or version options'
    selected_tag=
fi
valid_tag() {
    printf '%s\n' "$1" | awk '
        NR != 1 || $0 !~ /^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$/ || length($0) > 64 { invalid = 1 }
        END { exit (invalid || NR != 1) }'
}
[ -z "$selected_tag" ] || valid_tag "$selected_tag" || fail 'release tag must be vX.Y.Z with canonical nonnegative integers'

for prerequisite in uname tar mktemp awk mkdir cp chmod mv rm rmdir wc dd cat sleep; do
    command -v "$prerequisite" >/dev/null 2>&1 || fail "missing prerequisite: $prerequisite"
done
case "$(uname -s)" in
    Darwin) os=darwin ;;
    Linux) os=linux ;;
    *) fail 'only macOS and Linux native packages are supported' ;;
esac
machine=$(uname -m)
case "$machine" in
    arm64 | aarch64) arch=arm64 ;;
    x86_64) arch=x86_64 ;;
    *) fail "unsupported architecture: $machine (expected arm64/aarch64 or x86_64)" ;;
esac
asset="ev-$os-$arch.tar.gz"
if command -v sha256sum >/dev/null 2>&1; then
    hash_tool=sha256sum
elif command -v shasum >/dev/null 2>&1; then
    hash_tool=shasum
else
    fail 'install sha256sum (Linux) or shasum (macOS) for SHA-256 verification'
fi
hash_file() {
    case "$hash_tool" in
        sha256sum) sha256sum < "$1" ;;
        shasum) shasum -a 256 < "$1" ;;
    esac | awk 'NR == 1 && length($1) == 64 && $1 ~ /^[0-9a-fA-F]+$/ { hash = tolower($1) }
        END { if (NR != 1 || hash == "") exit 1; print hash }'
}

if [ -z "$install_dir" ]; then
    [ -n "${HOME:-}" ] || fail 'HOME is unset; provide --install-dir or EV_INSTALL_DIR'
    install_dir=$HOME/.local/bin
fi
case "$install_dir" in
    /*) ;;
    *) fail '--install-dir / EV_INSTALL_DIR must be an absolute path' ;;
esac
case "$install_dir" in
    *'
'*) fail 'installation directories containing newlines are not supported' ;;
esac
# Normalize repeated slashes, but never resolve dot components through a link.
remaining=${install_dir#/}
normalized=
while [ -n "$remaining" ]; do
    component=${remaining%%/*}
    case "$remaining" in */*) remaining=${remaining#*/} ;; *) remaining= ;; esac
    case "$component" in
        '') continue ;;
        . | ..) fail 'installation directory must not contain . or .. components' ;;
    esac
    normalized=$normalized/$component
done
install_dir=${normalized:-/}
binary=$install_dir/ev
metadata=$install_dir/.ev-install
lock=$install_dir/.ev-install.lock

# Walk every parent rather than relying on mkdir -p, which follows symlinks.
check_directory() {
    walk=
    remainder=${install_dir#/}
    while [ -n "$remainder" ]; do
        part=${remainder%%/*}
        case "$remainder" in */*) remainder=${remainder#*/} ;; *) remainder= ;; esac
        walk=$walk/$part
        [ ! -L "$walk" ] || fail "refusing symlink in installation path: $walk"
        if [ -e "$walk" ]; then
            [ -d "$walk" ] || fail "installation path component is not a directory: $walk"
        elif [ "$1" = create ]; then
            mkdir "$walk" || fail "cannot create $walk; choose a writable --install-dir"
            [ -d "$walk" ] && [ ! -L "$walk" ] || fail "unsafe installation directory: $walk"
        fi
    done
}
check_directory inspect
case "${TMPDIR:-/tmp}" in /*) ;; *) fail 'TMPDIR must be an absolute directory path' ;; esac

umask 077
temp_dir=
stage=
locked=0
owned=0
publish_started=0
metadata_attempted=0
binary_attempted=0
published=0
stop_animation() {
    if [ -n "$animation_pid" ]; then
        kill "$animation_pid" 2>/dev/null || :
        wait "$animation_pid" 2>/dev/null || :
        animation_pid=
    fi
    if [ "$terminal_active" -eq 1 ]; then
        printf '\r\033[7A\033[J\033[?25h' >&2
        terminal_active=0
    fi
}
terminal_size() {
    size=$(stty size <&2 2>/dev/null) || return 1
    case "$size" in *[!0-9\ ]* | '' | \ *) return 1 ;; esac
    rows=${size%% *}
    columns=${size##* }
    [ "$rows" -ge 12 ] && [ "$columns" -ge 20 ]
}
ball_lines() {
    # Original 16x16 Poke Ball pixel art drawn with half blocks: red top,
    # thin black band, white bottom, and a button that lights up when $1 is 1.
    case $ball_style in
        color)
            ball_0='\033[0m \033[0m \033[0m \033[0;38;5;236m▄\033[0;38;5;236m▄\033[0;38;5;236;48;5;196m▀\033[0;38;5;236;48;5;196m▀\033[0;38;5;236;48;5;196m▀\033[0;38;5;236;48;5;196m▀\033[0;38;5;236;48;5;196m▀\033[0;38;5;236;48;5;196m▀\033[0;38;5;236m▄\033[0;38;5;236m▄\033[0m \033[0m \033[0m \033[0m'
            ball_1='\033[0m \033[0;38;5;236m▄\033[0;38;5;236;48;5;196m▀\033[0;38;5;196;48;5;210m▀\033[0;38;5;196;48;5;210m▀\033[0;38;5;196m█\033[0;38;5;196m█\033[0;38;5;196m█\033[0;38;5;196m█\033[0;38;5;196m█\033[0;38;5;196m█\033[0;38;5;196m█\033[0;38;5;196m█\033[0;38;5;236;48;5;124m▀\033[0;38;5;236m▄\033[0m \033[0m'
            ball_2='\033[0;38;5;236m▄\033[0;38;5;236;48;5;196m▀\033[0;38;5;196m█\033[0;38;5;210;48;5;196m▀\033[0;38;5;196m█\033[0;38;5;196m█\033[0;38;5;196;48;5;236m▀\033[0;38;5;196;48;5;236m▀\033[0;38;5;196;48;5;236m▀\033[0;38;5;196;48;5;236m▀\033[0;38;5;196m█\033[0;38;5;196m█\033[0;38;5;196m█\033[0;38;5;124;48;5;196m▀\033[0;38;5;236;48;5;124m▀\033[0;38;5;236m▄\033[0m'
            ball_3='\033[0;38;5;236m█\033[0;38;5;196;48;5;236m▀\033[0;38;5;196;48;5;236m▀\033[0;38;5;196;48;5;236m▀\033[0;38;5;196;48;5;236m▀\033[0;38;5;236m█\033[0;38;5;231m█\033[0;38;5;231m█\033[0;38;5;231m█\033[0;38;5;231m█\033[0;38;5;236m█\033[0;38;5;196;48;5;236m▀\033[0;38;5;196;48;5;236m▀\033[0;38;5;124;48;5;236m▀\033[0;38;5;124;48;5;236m▀\033[0;38;5;236m█\033[0m'
            ball_4='\033[0;38;5;236m█\033[0;38;5;236;48;5;231m▀\033[0;38;5;236;48;5;231m▀\033[0;38;5;236;48;5;231m▀\033[0;38;5;236;48;5;231m▀\033[0;38;5;236m█\033[0;38;5;231m█\033[0;38;5;231m█\033[0;38;5;231m█\033[0;38;5;231m█\033[0;38;5;236m█\033[0;38;5;236;48;5;231m▀\033[0;38;5;236;48;5;231m▀\033[0;38;5;236;48;5;231m▀\033[0;38;5;236;48;5;250m▀\033[0;38;5;236m█\033[0m'
            ball_5='\033[0;38;5;236m▀\033[0;38;5;231;48;5;236m▀\033[0;38;5;231m█\033[0;38;5;231m█\033[0;38;5;231m█\033[0;38;5;231m█\033[0;38;5;236;48;5;231m▀\033[0;38;5;236;48;5;231m▀\033[0;38;5;236;48;5;231m▀\033[0;38;5;236;48;5;231m▀\033[0;38;5;231m█\033[0;38;5;231m█\033[0;38;5;231m█\033[0;38;5;231;48;5;250m▀\033[0;38;5;250;48;5;236m▀\033[0;38;5;236m▀\033[0m'
            ball_6='\033[0m \033[0;38;5;236m▀\033[0;38;5;231;48;5;236m▀\033[0;38;5;231m█\033[0;38;5;231m█\033[0;38;5;231m█\033[0;38;5;231m█\033[0;38;5;231m█\033[0;38;5;231m█\033[0;38;5;231m█\033[0;38;5;231m█\033[0;38;5;231;48;5;250m▀\033[0;38;5;250m█\033[0;38;5;250;48;5;236m▀\033[0;38;5;236m▀\033[0m \033[0m'
            ball_7='\033[0m \033[0m \033[0m \033[0;38;5;236m▀\033[0;38;5;236m▀\033[0;38;5;231;48;5;236m▀\033[0;38;5;231;48;5;236m▀\033[0;38;5;231;48;5;236m▀\033[0;38;5;231;48;5;236m▀\033[0;38;5;250;48;5;236m▀\033[0;38;5;250;48;5;236m▀\033[0;38;5;236m▀\033[0;38;5;236m▀\033[0m \033[0m \033[0m \033[0m'
            if [ "$1" -eq 1 ]; then
                ball_3='\033[0;38;5;236m█\033[0;38;5;196;48;5;236m▀\033[0;38;5;196;48;5;236m▀\033[0;38;5;196;48;5;236m▀\033[0;38;5;196;48;5;236m▀\033[0;38;5;236m█\033[0;38;5;203m█\033[0;38;5;203m█\033[0;38;5;203m█\033[0;38;5;203m█\033[0;38;5;236m█\033[0;38;5;196;48;5;236m▀\033[0;38;5;196;48;5;236m▀\033[0;38;5;124;48;5;236m▀\033[0;38;5;124;48;5;236m▀\033[0;38;5;236m█\033[0m'
                ball_4='\033[0;38;5;236m█\033[0;38;5;236;48;5;231m▀\033[0;38;5;236;48;5;231m▀\033[0;38;5;236;48;5;231m▀\033[0;38;5;236;48;5;231m▀\033[0;38;5;236m█\033[0;38;5;203m█\033[0;38;5;203m█\033[0;38;5;203m█\033[0;38;5;203m█\033[0;38;5;236m█\033[0;38;5;236;48;5;231m▀\033[0;38;5;236;48;5;231m▀\033[0;38;5;236;48;5;231m▀\033[0;38;5;236;48;5;250m▀\033[0;38;5;236m█\033[0m'
            fi ;;
        mono)
            ball_0='   ▄▄▀▀▀▀▀▀▄▄'
            ball_1=' ▄▀░░░░░░░░░░▀▄'
            ball_2='▄▀░░░░▄▄▄▄░░░░▀▄'
            ball_3='█▄▄▄▄█    █▄▄▄▄█'
            ball_4='█▀▀▀▀█    █▀▀▀▀█'
            ball_5='▀▄    ▀▀▀▀    ▄▀'
            ball_6=' ▀▄          ▄▀'
            ball_7='   ▀▀▄▄▄▄▄▄▀▀'
            if [ "$1" -eq 1 ]; then
                ball_3='█▄▄▄▄█▒▒▒▒█▄▄▄▄█'
                ball_4='█▀▀▀▀█▒▒▒▒█▀▀▀▀█'
            fi ;;
        *)
            ball_0="     ______"
            ball_1="   .'      '."
            ball_2="  /          \\\\"
            ball_3=" |=====(o)====|"
            ball_4=" |            |"
            ball_5="  \\\\          /"
            ball_6="   '.______.'"
            ball_7=""
            if [ "$1" -eq 1 ]; then ball_3=" |=====(O)====|"; fi
            if [ -z "${NO_COLOR+x}" ]; then
                ball_0="\\033[91m$ball_0" ball_2="$ball_2\\033[0m" ball_4="\\033[97m$ball_4" ball_6="$ball_6\\033[0m"
            fi ;;
    esac
}
phase() {
    stop_animation
    if [ -t 2 ] && [ "${TERM:-dumb}" != dumb ] && [ -z "${CI:-}" ] && terminal_size; then
        # Keep the phase outside the redraw area: resizing can reflow ordinary
        # text without moving a long animated line across the screen.
        printf '%.*s\n' "$((columns - 1))" "$1" >&2
        terminal_active=1
        printf '\033[?25l\n\n\n\n\n\n\n' >&2
        case $user_locale in
            *[Uu][Tt][Ff]-8* | *[Uu][Tt][Ff]8*) if [ -z "${NO_COLOR+x}" ]; then ball_style=color; else ball_style=mono; fi ;;
            *) ball_style=ascii ;;
        esac
        (
            tick_pid=
            tick=0
            trap 'trap "" 1 2 3 15; if [ -n "$tick_pid" ]; then kill "$tick_pid" 2>/dev/null || :; wait "$tick_pid" 2>/dev/null || :; fi; exit 0' 1 2 3 15
            while :; do
                # Recheck the real terminal, not a stale COLUMNS environment
                # variable. A narrow resize clears the ball until it fits again.
                if terminal_size; then
                    # Shake like a caught ball: left, centre, right, then rest
                    # while the button lights up once.
                    flash=0
                    case $((tick % 8)) in
                        1) pad= ;;
                        3) pad='  ' ;;
                        5) pad=' '; flash=1 ;;
                        *) pad=' ' ;;
                    esac
                    ball_lines "$flash"
                    printf '\r\033[7A\033[J%s%b\n%s%b\n%s%b\n%s%b\n%s%b\n%s%b\n%s%b\n%s%b\r' \
                        "$pad" "$ball_0" "$pad" "$ball_1" "$pad" "$ball_2" "$pad" "$ball_3" \
                        "$pad" "$ball_4" "$pad" "$ball_5" "$pad" "$ball_6" "$pad" "$ball_7" >&2
                else
                    printf '\r\033[7A\033[J\n\n\n\n\n\n\n' >&2
                fi
                tick=$((tick + 1))
                sleep 0.16 &
                tick_pid=$!
                wait "$tick_pid" || :
                tick_pid=
            done
        ) &
        animation_pid=$!
    else
        printf 'ev installer: %s\n' "$1" >&2
    fi
}
cleanup() {
    status=$?
    trap - 0
    trap '' 1 2 3 15
    set +e
    if [ -n "$download_pid" ]; then
        kill "$download_pid" 2>/dev/null || :
        wait "$download_pid" 2>/dev/null || :
        download_pid=
    fi
    stop_animation
    rollback_failed=0
    if [ "$publish_started" -eq 1 ] && [ "$published" -eq 0 ]; then
        if [ "$owned" -eq 1 ]; then
            if [ "$binary_attempted" -eq 1 ]; then
                mv -f "$stage/old-ev" "$binary" || rollback_failed=1
            fi
            if [ -d "$stage/old-metadata" ]; then
                if [ "$metadata_attempted" -eq 1 ]; then
                    rm -rf "$metadata" || rollback_failed=1
                fi
                mv "$stage/old-metadata" "$metadata" || rollback_failed=1
            fi
        else
            if [ "$binary_attempted" -eq 1 ]; then
                rm -f "$binary" || rollback_failed=1
            fi
            if [ "$metadata_attempted" -eq 1 ] && [ ! -d "$stage/metadata" ]; then
                rm -rf "$metadata" || rollback_failed=1
            fi
        fi
    fi
    if [ "$rollback_failed" -eq 1 ]; then
        printf 'ev installer: rollback failed; preserved recovery files at %s\n' "$stage" >&2
        status=1
    elif [ -n "$stage" ]; then
        rm -rf "$stage"
    fi
    [ -z "$temp_dir" ] || rm -rf "$temp_dir"
    if [ "$locked" -eq 1 ]; then rmdir "$lock"; fi
    exit "$status"
}
trap cleanup 0
trap 'exit 1' 1 2 3 15
temp_dir=$(mktemp -d "${TMPDIR:-/tmp}/ev-install.XXXXXXXX") || fail 'cannot create private temporary directory'

check_ownership() {
    check_directory inspect
    [ ! -L "$binary" ] && [ ! -L "$metadata" ] || fail 'refusing a symlink executable or metadata directory'
    if [ -e "$binary" ]; then
        [ -f "$binary" ] || fail "executable target is not a regular file: $binary"
        [ -d "$metadata" ] || fail "refusing to replace unowned executable: $binary"
        for item in "$metadata"/* "$metadata"/.[!.]* "$metadata"/..?*; do
            [ -e "$item" ] || [ -L "$item" ] || continue
            case "${item##*/}" in
                LICENSE | THIRD_PARTY_NOTICES | .native-install) ;;
                *) fail "unrecognized existing metadata; refusing to replace $item" ;;
            esac
            [ -f "$item" ] && [ ! -L "$item" ] || fail "unsafe existing metadata file: $item"
        done
        for member in LICENSE THIRD_PARTY_NOTICES .native-install; do
            [ -f "$metadata/$member" ] && [ ! -L "$metadata/$member" ] || fail "missing or unsafe ownership metadata: $member"
        done
        owner_fields=$(awk '
            NR == 1 { if ($0 != "ev-native-install-v2") invalid = 1 }
            NR == 2 { if ($0 !~ /^version=ev [^[:cntrl:]]+$/ || length($0) > 264) invalid = 1 }
            NR == 3 { hash = substr($0, 8); if (substr($0, 1, 7) != "sha256=" || length(hash) != 64 || hash !~ /^[0-9a-f]+$/) invalid = 1 }
            NR == 4 { license = substr($0, 16); if (substr($0, 1, 15) != "license-sha256=" || length(license) != 64 || license !~ /^[0-9a-f]+$/) invalid = 1 }
            NR == 5 { notices = substr($0, 16); if (substr($0, 1, 15) != "notices-sha256=" || length(notices) != 64 || notices !~ /^[0-9a-f]+$/) invalid = 1 }
            END {
                if (invalid || NR != 5) exit 1
                print hash, license, notices
            }
        ' "$metadata/.native-install") || fail 'existing ownership marker is invalid; refusing replacement'
        # Fields have been restricted to lowercase hashes.
        set -- $owner_fields
        owner_hash=$1
        license_hash=$(hash_file "$metadata/LICENSE") || fail 'cannot hash existing LICENSE'
        notices_hash=$(hash_file "$metadata/THIRD_PARTY_NOTICES") || fail 'cannot hash existing THIRD_PARTY_NOTICES'
        [ "$license_hash" = "$2" ] && [ "$notices_hash" = "$3" ] || fail 'existing metadata no longer matches its ownership marker; refusing replacement'
        current_hash=$(hash_file "$binary") || fail 'cannot hash existing executable'
        [ "$current_hash" = "$owner_hash" ] || fail 'existing executable no longer matches its ownership marker; refusing replacement'
        owned=1
    else
        [ ! -e "$metadata" ] || fail "orphaned metadata exists at $metadata; refusing to overwrite it"
        owned=0
    fi
}

if [ "$uninstall" -eq 1 ]; then
    [ -d "$install_dir" ] || fail "no owned installation exists at $install_dir"
    mkdir "$lock" 2>/dev/null || fail "installation is locked at $lock; remove only a stale lock after confirming no installer is running"
    locked=1
    check_ownership
    [ "$owned" -eq 1 ] || fail "no owned executable exists at $binary"
    phase 'Removing verified Eevee installation'
    stage=$(mktemp -d "$install_dir/.ev-stage.XXXXXXXX") || fail 'cannot stage removal on destination filesystem'
    cp -p "$binary" "$stage/old-ev" || fail 'cannot preserve executable for rollback'
    check_ownership
    publish_started=1
    mv "$metadata" "$stage/old-metadata" || fail 'cannot preserve metadata for rollback'
    binary_attempted=1
    rm -f "$binary" || fail 'cannot remove executable'
    published=1
    stop_animation
    printf 'Uninstalled %s; task, provider, and cache data were retained.\n' "$binary"
    exit 0
fi

# curl runs as a tracked child. File limits also bound chunked responses on
# curl versions whose --max-filesize only checks Content-Length.
download() {
    destination=$1
    maximum=$2
    shift 2
    (
        ulimit -f "$((maximum / 512))" || exit 1
        exec curl --disable --fail --silent --show-error --location \
            --proto '=https' --proto-redir '=https' --tlsv1.2 \
            --connect-timeout 15 --max-time 180 --max-redirs 8 \
            --max-filesize "$maximum" --output "$destination" "$@"
    ) > "$temp_dir/effective-url" &
    download_pid=$!
    if wait "$download_pid"; then
        download_pid=
    else
        download_pid=
        fail 'HTTPS download failed; no release was installed (check connectivity and the requested release)'
    fi
    downloaded_bytes=$(wc -c < "$destination")
    [ "$downloaded_bytes" -gt 0 ] && [ "$downloaded_bytes" -le "$maximum" ] || fail 'HTTPS response was empty or exceeded its size limit'
}
if [ "$archive_set" -eq 0 ]; then
    command -v curl >/dev/null 2>&1 || fail 'install curl for HTTPS downloads, or supply --archive and --checksums'
    if [ -z "$selected_tag" ]; then
        phase 'Resolving latest Eevee release'
        download "$temp_dir/latest-headers" 1048576 --head --write-out '%{url_effective}' \
            https://github.com/kimmandoo/eevee/releases/latest
        latest_url=$(cat "$temp_dir/effective-url")
        case "$latest_url" in
            https://github.com/kimmandoo/eevee/releases/tag/*)
                selected_tag=${latest_url#https://github.com/kimmandoo/eevee/releases/tag/} ;;
            *) fail 'latest release did not resolve to an exact GitHub release tag' ;;
        esac
        valid_tag "$selected_tag" || fail 'latest release has an unsupported tag; expected vX.Y.Z'
    fi
    release_url=https://github.com/kimmandoo/eevee/releases/download/$selected_tag
    phase "Downloading Eevee $selected_tag for $os/$arch"
    download "$temp_dir/SHA256SUMS" 1048576 "$release_url/SHA256SUMS"
    download "$temp_dir/$asset" 67108864 "$release_url/$asset"
else
    [ "${archive##*/}" = "$asset" ] || fail "this machine requires archive basename $asset"
    case "$archive" in /*) ;; *) archive=$PWD/$archive ;; esac
    case "$checksums" in /*) ;; *) checksums=$PWD/$checksums ;; esac
    [ -f "$archive" ] && [ -r "$archive" ] || fail "archive is not a readable regular file: $archive"
    [ -f "$checksums" ] && [ -r "$checksums" ] || fail "checksums are not a readable regular file: $checksums"
    # Snapshot inputs before verification. dd caps reads even if an input grows.
    dd if="$archive" of="$temp_dir/$asset" bs=1048576 count=65 2>/dev/null || fail 'cannot read archive'
    archive_bytes=$(wc -c < "$temp_dir/$asset")
    [ "$archive_bytes" -gt 0 ] && [ "$archive_bytes" -le 67108864 ] || fail 'archive must be nonempty and at most 64 MiB'
    dd if="$checksums" of="$temp_dir/SHA256SUMS" bs=1048576 count=2 2>/dev/null || fail 'cannot read checksums'
fi
checksum_bytes=$(wc -c < "$temp_dir/SHA256SUMS")
[ "$checksum_bytes" -gt 0 ] && [ "$checksum_bytes" -le 1048576 ] || fail 'SHA256SUMS must be nonempty and at most 1 MiB'
phase 'Verifying release checksum and archive'
expected=$(awk -v wanted="$asset" '
    {
        hash = substr($0, 1, 64)
        filename = substr($0, 67)
        if (length(hash) != 64 || hash !~ /^[0-9a-fA-F]+$/ || substr($0, 65, 2) != "  " ||
            filename !~ /^(ev-((darwin|linux)-(arm64|x86_64)\.tar\.gz|windows-x86_64\.zip)|install\.(sh|ps1))$/ || seen[filename]++) {
            invalid = 1; exit
        }
        count++
        if (filename == wanted) selected = tolower(hash)
    }
    END { if (invalid || count < 1 || selected == "") exit 1; print selected }
' "$temp_dir/SHA256SUMS") || fail "malformed SHA256SUMS or missing unique checksum for $asset"
actual=$(hash_file "$temp_dir/$asset") || fail 'cannot compute archive SHA-256'
[ "$actual" = "$expected" ] || fail "SHA-256 mismatch for $asset; nothing was installed"

# Inspect before extraction. Bound listing output as well as compressed inputs.
(ulimit -f 1024 && tar -tzf "$temp_dir/$asset" > "$temp_dir/entries") || fail 'cannot safely list archive'
awk '
    ($0 == "ev" || $0 == "LICENSE" || $0 == "THIRD_PARTY_NOTICES") && !seen[$0]++ { count++; next }
    { invalid = 1 }
    END { if (invalid || count != 3) exit 1 }
' "$temp_dir/entries" || fail 'archive must contain exactly ev, LICENSE, and THIRD_PARTY_NOTICES at its root, without duplicates'
(ulimit -f 1024 && tar -tvzf "$temp_dir/$asset" > "$temp_dir/details") || fail 'cannot safely inspect archive'
# GNU and BSD tar expose logical sizes in different columns. Fail closed on
# other layouts: POSIX tar has no portable machine-readable size listing.
awk '
    {
        name = $NF
        if (substr($0, 1, 1) != "-" ||
            (name != "ev" && name != "LICENSE" && name != "THIRD_PARTY_NOTICES") || seen[name]++) {
            invalid = 1; next
        }
        if (NF == 6 && $3 ~ /^[0-9]+$/ && $4 ~ /^[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]$/) size = $3
        else if (NF == 9 && $2 ~ /^[0-9]+$/ && $5 ~ /^[0-9]+$/) size = $5
        else { invalid = 1; next }
        total += size
        if (size > 100663296 || total > 100663296) invalid = 1
    }
    END { if (invalid || NR != 3) exit 1 }
' "$temp_dir/details" || fail 'archive has links, nonregular files, over 96 MiB of expanded files, or an unsupported tar size listing'
mkdir "$temp_dir/extracted" || fail 'cannot prepare extraction directory'
tar -xzf "$temp_dir/$asset" -C "$temp_dir/extracted" ev LICENSE THIRD_PARTY_NOTICES || fail 'archive extraction failed'
total=0
for member in ev LICENSE THIRD_PARTY_NOTICES; do
    file=$temp_dir/extracted/$member
    [ -f "$file" ] && [ ! -L "$file" ] || fail "extracted $member is not a regular file"
    bytes=$(wc -c < "$file")
    total=$((total + bytes))
    [ "$total" -le 100663296 ] || fail 'expanded files exceed 96 MiB'
done


phase 'Staging verified Eevee installation'
check_directory create
mkdir "$lock" 2>/dev/null || fail "installation is locked at $lock; remove only a stale lock after confirming no installer is running"
locked=1
check_ownership
stage=$(mktemp -d "$install_dir/.ev-stage.XXXXXXXX") || fail 'cannot stage installation on destination filesystem'
mkdir "$stage/metadata" || fail 'cannot stage metadata'
cp "$temp_dir/extracted/ev" "$stage/ev" || fail 'cannot stage executable'
cp "$temp_dir/extracted/LICENSE" "$stage/metadata/LICENSE" || fail 'cannot stage LICENSE'
cp "$temp_dir/extracted/THIRD_PARTY_NOTICES" "$stage/metadata/THIRD_PARTY_NOTICES" || fail 'cannot stage THIRD_PARTY_NOTICES'
chmod 755 "$stage/ev" "$stage/metadata" || fail 'cannot set executable or metadata permissions'
chmod 644 "$stage/metadata/LICENSE" "$stage/metadata/THIRD_PARTY_NOTICES" || fail 'cannot set license permissions'
new_hash=$(hash_file "$stage/ev") || fail 'cannot hash staged executable'
"$stage/ev" --version > "$temp_dir/version" || fail 'staged ev --version failed; prior installation is unchanged'
after_version_hash=$(hash_file "$stage/ev") || fail 'cannot verify staged executable after --version'
[ "$after_version_hash" = "$new_hash" ] || fail 'staged executable changed during --version; prior installation is unchanged'
awk 'NR != 1 || $0 !~ /^ev [^[:cntrl:]]+$/ || length($0) > 256 { invalid = 1 }
    END { if (invalid || NR != 1) exit 1 }' "$temp_dir/version" || fail 'staged ev --version must return one nonempty ev version line'
version=$(cat "$temp_dir/version")
if [ -n "$selected_tag" ]; then
    [ "$version" = "ev ${selected_tag#v}" ] || fail "staged ev --version does not match $selected_tag; prior installation is unchanged"
fi

if [ "$archive_set" -eq 0 ]; then
    phase 'Downloading and verifying all nine original sprites'
    # Use the verified release's pinned downloader and platform cache defaults,
    # exactly as normal startup does. Never run an unverified PATH executable.
    "$stage/ev" --download-artwork > "$temp_dir/artwork-output" 2> "$temp_dir/artwork-error" &
    download_pid=$!
    if wait "$download_pid"; then
        download_pid=
        stop_animation
        cat "$temp_dir/artwork-output"
        cat "$temp_dir/artwork-error" >&2
    else
        download_pid=
        stop_animation
        cat "$temp_dir/artwork-output"
        cat "$temp_dir/artwork-error" >&2
        fail 'sprite setup failed; prior installation is unchanged. Completed sprites remain cached. Rerun to resume, or use --archive and --checksums for an explicit offline installation'
    fi
    after_sprite_hash=$(hash_file "$stage/ev") || fail 'cannot verify staged executable after sprite setup'
    [ "$after_sprite_hash" = "$new_hash" ] || fail 'staged executable changed during sprite setup; prior installation is unchanged'
else
    stop_animation
    printf 'Offline install: sprite cache unchanged. Missing images? Run ev --download-artwork when online, or provide --assets-dir.\n'
fi
phase 'Publishing verified Eevee installation'
license_hash=$(hash_file "$stage/metadata/LICENSE") || fail 'cannot hash staged LICENSE'
notices_hash=$(hash_file "$stage/metadata/THIRD_PARTY_NOTICES") || fail 'cannot hash staged THIRD_PARTY_NOTICES'
printf 'ev-native-install-v2\nversion=%s\nsha256=%s\nlicense-sha256=%s\nnotices-sha256=%s\n' \
    "$version" "$new_hash" "$license_hash" "$notices_hash" > "$stage/metadata/.native-install" || fail 'cannot write ownership marker'
chmod 644 "$stage/metadata/.native-install" || fail 'cannot set ownership marker permissions'

# Revalidate immediately before publishing; serialize cooperating installers.
# Shell cannot eliminate races with unrelated processes rewriting these paths.
check_ownership
if [ "$owned" -eq 1 ]; then
    cp -p "$binary" "$stage/old-ev" || fail 'cannot preserve previous executable for rollback'
fi
publish_started=1
if [ "$owned" -eq 1 ]; then
    mv "$metadata" "$stage/old-metadata" || fail 'cannot preserve previous metadata for rollback'
fi
metadata_attempted=1
mv "$stage/metadata" "$metadata" || fail 'cannot publish metadata'
binary_attempted=1
mv -f "$stage/ev" "$binary" || fail 'cannot publish executable'
published=1
stop_animation
printf 'Installed %s to %s\n' "$version" "$binary"
case ":${PATH:-}:" in
    *":$install_dir:"*) ;;
    *)
        path_literal=$(printf '%s\n' "$install_dir" | awk '{
            printf "\047"
            for (i = 1; i <= length($0); i++) {
                c = substr($0, i, 1)
                if (c == "\047") printf "%c%c%c%c", 39, 92, 39, 39
                else printf "%s", c
            }
            printf "\047"
        }')
        printf 'For this shell, add the command directory to PATH:\n  export PATH=%s:"$PATH"\n' "$path_literal"
        printf 'No profile was modified. Run ev update for future releases, or ev uninstall to remove the command.\n'
        ;;
esac
