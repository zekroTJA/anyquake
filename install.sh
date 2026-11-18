#!/usr/bin/env bash
# Installs or updates AnyQuake from the latest GitHub release.
#
#   curl -fsSL https://raw.githubusercontent.com/zekroTJA/anyquake/main/install.sh | bash
#
# macOS: installs AnyQuake.app into /Applications and resets its
# Accessibility permission, which macOS invalidates on every update.
# Linux: installs the .deb (apt) or .rpm (dnf, yum, zypper) package.
#
# Environment:
#   ANYQUAKE_VERSION  version to install, e.g. 0.5.0 (default: latest release)

set -euo pipefail

REPO="zekroTJA/anyquake"
BUNDLE_ID="de.anyquake.AnyQuake"
APP_DIR="/Applications/AnyQuake.app"

info() {
    printf '\033[1;34m==>\033[0m %s\n' "$*"
}

warn() {
    printf '\033[1;33mwarning:\033[0m %s\n' "$*" >&2
}

die() {
    printf '\033[1;31merror:\033[0m %s\n' "$*" >&2
    exit 1
}

need() {
    command -v "$1" >/dev/null 2>&1 || die "'$1' is required but not installed"
}

# Runs a command as root, through sudo unless already root.
as_root() {
    if [ "$(id -u)" -eq 0 ]; then
        "$@"
    else
        need sudo
        sudo "$@"
    fi
}

# Prints the download URLs of all assets of the requested release.
release_asset_urls() {
    local api
    if [ -n "${ANYQUAKE_VERSION:-}" ]; then
        api="https://api.github.com/repos/$REPO/releases/tags/v${ANYQUAKE_VERSION#v}"
    else
        api="https://api.github.com/repos/$REPO/releases/latest"
    fi
    curl -fsSL -H "Accept: application/vnd.github+json" "$api" |
        grep -o '"browser_download_url": *"[^"]*"' |
        sed 's/.*"\(https[^"]*\)"$/\1/'
}

# Prints the first URL matching an extended regex, or fails.
pick_asset() {
    local urls=$1 pattern=$2 url
    url=$(printf '%s\n' "$urls" | grep -E "$pattern" | head -n 1) ||
        die "no release asset matching '$pattern' found"
    printf '%s\n' "$url"
}

download() {
    local url=$1 dest=$2
    info "Downloading ${url##*/}"
    curl -fL --progress-bar -o "$dest" "$url"
}

install_macos() {
    need hdiutil
    local urls url dmg mount updating=0
    urls=$(release_asset_urls) || die "could not fetch the release from GitHub"
    url=$(pick_asset "$urls" '\.dmg$')

    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT
    dmg="$tmp/${url##*/}"
    download "$url" "$dmg"

    if [ -d "$APP_DIR" ]; then
        updating=1
        info "Stopping the running AnyQuake"
        "$APP_DIR/Contents/MacOS/anyquake" quit >/dev/null 2>&1 || true
        # Give it a moment to exit before replacing the bundle.
        for _ in 1 2 3 4 5; do
            pgrep -f "$APP_DIR/Contents/MacOS/anyquake" >/dev/null || break
            sleep 1
        done
    fi

    info "Installing to $APP_DIR"
    mount="$tmp/mount"
    mkdir "$mount"
    hdiutil attach -nobrowse -readonly -quiet -mountpoint "$mount" "$dmg"
    local copy=(ditto "$mount/AnyQuake.app" "$APP_DIR.new")
    local swap=(sh -c "rm -rf '$APP_DIR' && mv '$APP_DIR.new' '$APP_DIR'")
    if [ -w /Applications ] && { [ ! -e "$APP_DIR" ] || [ -w "$APP_DIR" ]; }; then
        rm -rf "$APP_DIR.new"
        "${copy[@]}" && "${swap[@]}"
    else
        as_root rm -rf "$APP_DIR.new"
        as_root "${copy[@]}" && as_root "${swap[@]}"
    fi
    hdiutil detach -quiet "$mount" || true

    # Downloads with curl aren't quarantined, but make sure Gatekeeper
    # doesn't ask for "Open Anyway".
    xattr -dr com.apple.quarantine "$APP_DIR" 2>/dev/null || true

    if [ "$updating" -eq 1 ]; then
        # The app is signed ad-hoc, so the old Accessibility entry no longer
        # matches the new build even though it still shows as enabled.
        info "Removing the outdated Accessibility permission"
        if ! tccutil reset Accessibility "$BUNDLE_ID" >/dev/null 2>&1; then
            warn "could not reset the permission automatically; remove the AnyQuake entry"
            warn "in System Settings → Privacy & Security → Accessibility with '−' first"
        fi
    fi

    cat <<'EOF'

AnyQuake is installed. To finish, grant it the Accessibility permission:

  1. System Settings → Privacy & Security → Accessibility opens now.
  2. Click '+', select AnyQuake in Applications and click 'Open'.
  3. Make sure the switch next to AnyQuake is on.
  4. Start AnyQuake from Applications.

EOF
    open "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility" ||
        true
}

install_linux() {
    local arch deb_arch rpm_arch kind installer
    arch=$(uname -m)
    case "$arch" in
    x86_64 | amd64) deb_arch=amd64 rpm_arch=x86_64 ;;
    aarch64 | arm64) deb_arch=arm64 rpm_arch=aarch64 ;;
    *) die "unsupported architecture: $arch" ;;
    esac

    if command -v apt-get >/dev/null 2>&1; then
        kind=deb installer=apt-get
    elif command -v dnf >/dev/null 2>&1; then
        kind=rpm installer=dnf
    elif command -v zypper >/dev/null 2>&1; then
        kind=rpm installer=zypper
    elif command -v yum >/dev/null 2>&1; then
        kind=rpm installer=yum
    else
        die "no supported package manager found (apt-get, dnf, zypper, yum); download a package from https://github.com/$REPO/releases"
    fi

    local urls url pkg
    urls=$(release_asset_urls) || die "could not fetch the release from GitHub"
    if [ "$kind" = deb ]; then
        url=$(pick_asset "$urls" "_${deb_arch}\.deb$")
    else
        url=$(pick_asset "$urls" "\.${rpm_arch}\.rpm$")
    fi

    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT
    # Readable for apt's unprivileged "_apt" download user.
    chmod 755 "$tmp"
    pkg="$tmp/${url##*/}"
    download "$url" "$pkg"
    chmod 644 "$pkg"

    # Stop a running instance so the new version is used after restart.
    command -v anyquake >/dev/null 2>&1 && anyquake quit >/dev/null 2>&1 || true

    info "Installing ${pkg##*/} with $installer"
    case "$installer" in
    apt-get) as_root apt-get install -y "$pkg" ;;
    dnf) as_root dnf install -y "$pkg" ;;
    yum) as_root yum install -y "$pkg" ;;
    zypper) as_root zypper --non-interactive install --allow-unsigned-rpm "$pkg" ;;
    esac

    printf '\nAnyQuake is installed. Start it with "anyquake" or from your application menu.\n'
}

main() {
    need curl
    case "$(uname -s)" in
    Darwin) install_macos ;;
    Linux) install_linux ;;
    *) die "unsupported operating system: $(uname -s) (on Windows, use the installer from https://github.com/$REPO/releases)" ;;
    esac
}

# Everything runs from here, so a partially downloaded script does nothing.
main "$@"
