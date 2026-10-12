#!/bin/sh
# Install a checksum-verified local Eevee package; no downloads or profile edits.
set -eu
LC_ALL=C
export LC_ALL

fail() {
    printf 'ev installer: %s\n' "$*" >&2
    exit 1
}

usage() {
    cat <<'USAGE'
Usage: sh install.sh --archive PATH --checksums PATH [--install-dir PATH]

Install a prebuilt ev package for this Mac or Linux machine from local files.
Both --archive and --checksums are required; there is no default download URL.
--checksums must name the package's SHA256SUMS file. --install-dir must be an
absolute directory, and defaults to EV_INSTALL_DIR or $HOME/.local/bin.
Rerun with another verified package to upgrade an installer-owned executable.
No sudo, shell profile changes, or Eevee cache/data changes are performed.
USAGE
}

archive=
checksums=
install_dir=${EV_INSTALL_DIR:-}
archive_set=0
checksums_set=0
dir_set=0
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
        -h | --help) usage; exit 0 ;;
        *) usage >&2; fail "unknown argument: $1" ;;
    esac
done
[ "$archive_set" -eq 1 ] && [ "$checksums_set" -eq 1 ] || {
    usage >&2
    fail 'provide both --archive PATH and --checksums PATH'
}

for prerequisite in uname tar mktemp awk mkdir cp chmod mv rm rmdir wc dd cat; do
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
[ "${archive##*/}" = "$asset" ] || fail "this machine requires archive basename $asset"
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
case "$archive" in /*) ;; *) archive=$PWD/$archive ;; esac
case "$checksums" in /*) ;; *) checksums=$PWD/$checksums ;; esac
[ -f "$archive" ] && [ -r "$archive" ] || fail "archive is not a readable regular file: $archive"
[ -f "$checksums" ] && [ -r "$checksums" ] || fail "checksums are not a readable regular file: $checksums"
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
cleanup() {
    status=$?
    trap - 0
    trap '' 1 2 3 15
    set +e
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

# Snapshot inputs before verification. dd caps reads even if an input grows.
dd if="$archive" of="$temp_dir/$asset" bs=1048576 count=65 2>/dev/null || fail 'cannot read archive'
archive_bytes=$(wc -c < "$temp_dir/$asset")
[ "$archive_bytes" -gt 0 ] && [ "$archive_bytes" -le 67108864 ] || fail 'archive must be nonempty and at most 64 MiB'
dd if="$checksums" of="$temp_dir/SHA256SUMS" bs=1048576 count=2 2>/dev/null || fail 'cannot read checksums'
checksum_bytes=$(wc -c < "$temp_dir/SHA256SUMS")
[ "$checksum_bytes" -gt 0 ] && [ "$checksum_bytes" -le 1048576 ] || fail 'SHA256SUMS must be nonempty and at most 1 MiB'
expected=$(awk -v wanted="$asset" '
    {
        hash = substr($0, 1, 64)
        filename = substr($0, 67)
        if (length(hash) != 64 || hash !~ /^[0-9a-fA-F]+$/ || substr($0, 65, 2) != "  " ||
            filename !~ /^ev-((darwin|linux)-(arm64|x86_64)\.tar\.gz|windows-x86_64\.zip)$/ || seen[filename]++) {
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

check_directory create
mkdir "$lock" 2>/dev/null || fail "installation is locked at $lock; remove only a stale lock after confirming no installer is running"
locked=1

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
        owner_hash=$(awk '
            NR == 1 { if ($0 != "ev-native-install-v1") invalid = 1 }
            NR == 2 { if ($0 !~ /^version=ev [^[:cntrl:]]+$/ || length($0) > 264) invalid = 1 }
            NR == 3 { hash = substr($0, 8); if (substr($0, 1, 7) != "sha256=" || length(hash) != 64 || hash !~ /^[0-9a-f]+$/) invalid = 1 }
            END { if (invalid || NR != 3) exit 1; print hash }
        ' "$metadata/.native-install") || fail 'existing ownership marker is invalid; refusing replacement'
        current_hash=$(hash_file "$binary") || fail 'cannot hash existing executable'
        [ "$current_hash" = "$owner_hash" ] || fail 'existing executable no longer matches its ownership marker; refusing replacement'
        owned=1
    else
        [ ! -e "$metadata" ] || fail "orphaned metadata exists at $metadata; refusing to overwrite it"
        owned=0
    fi
}
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
printf 'ev-native-install-v1\nversion=%s\nsha256=%s\n' "$version" "$new_hash" > "$stage/metadata/.native-install" || fail 'cannot write ownership marker'
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
printf 'Installed %s to %s\n' "$version" "$binary"
case ":${PATH:-}:" in
    *":$install_dir:"*) ;;
    *)
        printf 'Add this directory to your shell PATH (no profile was modified):\n  %s\n' "$install_dir"
        printf 'Then start a new shell and run ev. Rerun this installer with a verified local package to upgrade.\n'
        ;;
esac
