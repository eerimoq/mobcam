#!/usr/bin/env bash
#
# Set up the system tests on the Linux machine the iPhone or iPad is connected
# to, a BELABOX in particular.
#
# Run it on that machine, from a clone of the repository:
#
#     ./tests/belabox/setup.sh
#
# It installs FFmpeg and a Python the tests run on, puts the user in the groups
# that may read the camera and the microphone, and creates the virtual
# environment the tests are started from.
#
# The tests record the camera and the microphone with the FFmpeg in
# /opt/mobcam/ffmpeg, which crates/virtualcam/belabox/install.sh builds, as that
# is the one that encodes video in the RK3588 hardware and reads ALSA. The
# FFmpeg of the machine reads what came out.

set -euo pipefail

PYTHON=python3.11
VENV=.venv
PACKAGES=(
    alsa-utils
    ffmpeg
    python3.11
    python3.11-venv
    rsync
    v4l-utils
)
USER_GROUPS=(audio video)
MOBCAM_FFMPEG=/opt/mobcam/ffmpeg/bin/ffmpeg
sudo=

step()
{
    echo
    echo "==> $1"
}

die()
{
    echo "error: $1" >&2
    exit 1
}

check_machine()
{
    [ "$(uname -s)" = Linux ] || die "this only runs on Linux"
    [ -f tests/test.py ] || die "run this from the root of a clone of the repository"
    if [ "$(id -u)" -ne 0 ] ; then
        sudo=sudo
        if ! sudo -n true 2>/dev/null && ! sudo -v ; then
            die "sudo is needed to install"
        fi
    fi
    if [ ! -x $MOBCAM_FFMPEG ] ; then
        die "no $MOBCAM_FFMPEG; run ./crates/virtualcam/belabox/install.sh first"
    fi
}

install_packages()
{
    step "Installing ${PACKAGES[*]}."
    $sudo apt-get update
    $sudo DEBIAN_FRONTEND=noninteractive apt-get install -y "${PACKAGES[@]}"
}

add_user_to_groups()
{
    local user
    local added

    user=$(id -un)
    added=()
    for group in "${USER_GROUPS[@]}" ; do
        if ! id -nG "$user" | tr ' ' '\n' | grep -qx "$group" ; then
            $sudo usermod -aG "$group" "$user"
            added+=("$group")
        fi
    done
    if [ ${#added[@]} -gt 0 ] ; then
        step "Added $user to ${added[*]}. Log out and in again for it to take."
    fi
}

ensure_localhost()
{
    # The remote control assistant the tests drive Moblin through talks to
    # itself over localhost, which a BELABOX does not have in /etc/hosts.
    if getent hosts localhost >/dev/null ; then
        return
    fi
    step "Adding localhost to /etc/hosts."
    echo "127.0.0.1 localhost" | $sudo tee -a /etc/hosts >/dev/null
}

create_virtual_environment()
{
    step "Creating $VENV."
    $PYTHON -m venv $VENV
    $VENV/bin/pip install --upgrade pip
    $VENV/bin/pip install -r scripts/requirements.txt
}

create_configuration()
{
    if [ -f tests/config.toml ] ; then
        return
    fi
    step "Creating tests/config.toml from tests/config.example.toml."
    cp tests/config.example.toml tests/config.toml
}

summary()
{
    step "Set up."
    cat <<EOF
Edit tests/config.toml to match this machine and the device, then import the
settings printed by

    make test-generate-device-settings-stdout

into Moblin, connect the iPhone or iPad with a USB cable, unlock it and tap
Trust. Run the tests with

    make test TEST_ARGS="--device <name>"
EOF
}

main()
{
    check_machine
    install_packages
    add_user_to_groups
    ensure_localhost
    create_virtual_environment
    create_configuration
    summary
}

main "$@"
