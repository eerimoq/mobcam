#!/usr/bin/env bash

set -euo pipefail

PYTHON=python3.11
VENV=.venv
INSTALL=./scripts/belabox/install.sh
PACKAGES=(
    alsa-utils
    python3.11
    python3.11-venv
    v4l-utils
)
USER_GROUPS=(audio video)
FFMPEG_PREFIX=/opt/mobcam/ffmpeg
MOBCAM_FFMPEG=$FFMPEG_PREFIX/bin/ffmpeg
SERVICE=mobcam-virtualcam.service

die()
{
    echo "error: $1" >&2
    exit 1
}

run_sudo()
{
    echo "running: sudo $*" >&2
    sudo "$@"
}

check_machine()
{
    if [ ! -x $MOBCAM_FFMPEG ] ; then
        die "no $MOBCAM_FFMPEG; run $INSTALL first"
    fi
}

missing_packages()
{
    local package

    for package in "${PACKAGES[@]}" ; do
        if ! dpkg-query -W -f '${Status}' "$package" 2>/dev/null | grep -q '^install ok installed$' ; then
            return 0
        fi
    done
    return 1
}

install_packages()
{
    if ! missing_packages ; then
        return
    fi
    run_sudo apt-get update
    run_sudo DEBIAN_FRONTEND=noninteractive apt-get install -y "${PACKAGES[@]}"
}

add_user_to_groups()
{
    local user
    local added

    user=$(id -un)
    added=()
    for group in "${USER_GROUPS[@]}" ; do
        if ! id -nG "$user" | tr ' ' '\n' | grep -qx "$group" ; then
            run_sudo usermod -aG "$group" "$user"
            added+=("$group")
        fi
    done
    if [ ${#added[@]} -gt 0 ] ; then
        die "added $user to ${added[*]}; log in again for it to take, then run this again"
    fi
}

create_virtual_environment()
{
    if [ ! -d $VENV ] ; then
        $PYTHON -m venv $VENV
        $VENV/bin/pip install --upgrade pip
    fi
    $VENV/bin/pip install --quiet -r scripts/requirements.txt
}

stop_service()
{
    if systemctl is-active --quiet $SERVICE ; then
        run_sudo systemctl stop $SERVICE
    fi
}

build()
{
    . "$HOME/.cargo/env"
    export PKG_CONFIG_PATH=$FFMPEG_PREFIX/lib/pkgconfig
    export RUSTFLAGS="${RUSTFLAGS:-} -C link-arg=-Wl,-rpath,$FFMPEG_PREFIX/lib"
    cargo build --locked --release --package mobcam-virtualcam
}

run_tests()
{
    export PATH=$FFMPEG_PREFIX/bin:$PATH
    . $VENV/bin/activate
    make test-virtualcam TEST_ARGS="$*"
}

main()
{
    check_machine
    install_packages
    add_user_to_groups
    create_virtual_environment
    stop_service
    build
    run_tests "$@"
}

main "$@"
