#!/bin/sh
# Runs the installer from the latest release, so this URL never changes between releases.
set -eu

main() {
    installer=$(curl --proto '=https' --tlsv1.2 -fsSL https://github.com/younes-io/ttyl/releases/latest/download/ttyl-installer.sh)
    printf '%s\n' "$installer" | sh -s -- "$@"
}

main "$@"
