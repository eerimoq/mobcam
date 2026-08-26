#!/usr/bin/env bash
#
# Build and install Mobcam Virtual Camera on a BELABOX.
#
# Run it on the BELABOX, either from a clone of the repository or on its own,
# in which case it clones the repository itself:
#
#     ./crates/virtualcam/belabox/install.sh
#
# It installs the build dependencies and the v4l2loopback module, builds the
# FFmpeg and snd-aloop that BELABOX does not have, builds and installs the
# binary, creates the Mobcam camera and the Mobcam sound card, both loaded at
# boot, and runs mobcam-virtualcam as a service.

set -euo pipefail

REPOSITORY=${MOBCAM_REPOSITORY:-https://github.com/eerimoq/mobcam.git}
CLONE_DIR=${MOBCAM_SOURCE_DIR:-$HOME/mobcam}
CARD=Mobcam
BINARY=/usr/local/bin/mobcam-virtualcam
SERVICE=mobcam-virtualcam.service
SERVICE_FILE=/etc/systemd/system/$SERVICE
MODPROBE_FILE=/etc/modprobe.d/mobcam.conf
MODULES_FILE=/etc/modules-load.d/mobcam.conf
UDEV_FILE=/etc/udev/rules.d/70-mobcam.rules
VIDEO_DEVICE=/dev/mobcam
AUDIO_DEVICE=plughw:CARD=$CARD,DEV=1
FFMPEG_VERSION=7.1.2
FFMPEG_PREFIX=/opt/mobcam/ffmpeg
FFMPEG_MINIMUM=59.37.100
FFMPEG_URL=https://ffmpeg.org/releases/ffmpeg-$FFMPEG_VERSION.tar.xz
BUILD_DIR=${MOBCAM_BUILD_DIR:-$HOME/.cache/mobcam}
KERNEL=$(uname -r)
KERNEL_HEADERS=/lib/modules/$KERNEL/build
ALOOP_VERSION=${KERNEL%%-*}
ALOOP_DIR=/usr/src/snd-aloop-$ALOOP_VERSION
ALOOP_URL=https://git.kernel.org/pub/scm/linux/kernel/git/stable/linux.git/plain/sound/drivers/aloop.c

PACKAGES=(
    build-essential
    curl
    dkms
    git
    libasound2-dev
    libavcodec-dev
    libavutil-dev
    libclang-dev
    pkg-config
    usbmuxd
    v4l2loopback-dkms
    xz-utils
)

audio=yes
service=yes
sudo=

usage()
{
    cat <<EOF
usage: $(basename "$0") [--no-audio] [--no-service] [--help]

Build and install Mobcam Virtual Camera on a BELABOX.

  --no-audio    do not set up the Mobcam sound card, video only
  --no-service  do not run mobcam-virtualcam as a service
  --help        print this text and exit

The source is the clone this script is part of, or a clone of
$REPOSITORY
in $CLONE_DIR. Set MOBCAM_REPOSITORY and MOBCAM_SOURCE_DIR to
change either, and MOBCAM_BUILD_DIR to build FFmpeg somewhere else than
$BUILD_DIR.
EOF
}

step()
{
    echo
    echo "==> $1"
}

warn()
{
    echo "warning: $1" >&2
}

die()
{
    echo "error: $1" >&2
    exit 1
}

parse_arguments()
{
    while [ $# -gt 0 ] ; do
        case $1 in
            --no-audio) audio=no ;;
            --no-service) service=no ;;
            --help) usage ; exit 0 ;;
            *) usage >&2 ; die "unknown argument $1" ;;
        esac
        shift
    done
}

check_machine()
{
    [ "$(uname -s)" = Linux ] || die "this only runs on Linux"
    [ -d "$KERNEL_HEADERS" ] || die "no kernel headers in $KERNEL_HEADERS"
    if [ ! -d /opt/belaUI ] ; then
        warn "no /opt/belaUI; this does not look like a BELABOX, carrying on anyway"
    fi
    if [ "$(id -u)" -ne 0 ] ; then
        sudo=sudo
        if ! sudo -n true 2>/dev/null && ! sudo -v ; then
            die "sudo is needed to install"
        fi
    fi
}

write_file()
{
    $sudo tee "$1" >/dev/null
}

stop_service()
{
    if systemctl is-active --quiet $SERVICE ; then
        step "Stopping $SERVICE."
        $sudo systemctl stop $SERVICE
    fi
}

install_packages()
{
    step "Installing the build dependencies."
    $sudo apt-get update
    $sudo DEBIAN_FRONTEND=noninteractive apt-get install -y "${PACKAGES[@]}"
}

install_rust()
{
    if [ ! -x "$HOME/.cargo/bin/cargo" ] && ! command -v cargo >/dev/null ; then
        step "Installing the Rust toolchain."
        curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs \
            | sh -s -- -y --no-modify-path --default-toolchain none
    fi
    if [ -f "$HOME/.cargo/env" ] ; then
        . "$HOME/.cargo/env"
    fi
    command -v cargo >/dev/null || die "no cargo in the path"
}

ffmpeg_is_new_enough()
{
    PKG_CONFIG_PATH=${1:-} pkg-config --atleast-version=$FFMPEG_MINIMUM libavcodec 2>/dev/null
}

build_ffmpeg()
{
    local directory

    step "Building FFmpeg $FFMPEG_VERSION in $BUILD_DIR."
    directory=$BUILD_DIR/ffmpeg-$FFMPEG_VERSION
    mkdir -p "$BUILD_DIR"
    if [ ! -d "$directory" ] ; then
        curl -fL "$FFMPEG_URL" | tar -xJ -C "$BUILD_DIR"
    fi
    (
        cd "$directory"
        # /tmp is noexec on a BELABOX, which configure cannot work with.
        mkdir -p tmp
        export TMPDIR=$directory/tmp
        ./configure \
            --prefix=$FFMPEG_PREFIX \
            --enable-shared \
            --disable-static \
            --disable-autodetect \
            --disable-programs \
            --disable-doc \
            --disable-avdevice \
            --disable-avformat \
            --disable-avfilter \
            --disable-swscale \
            --disable-swresample \
            --disable-network \
            --disable-everything \
            --enable-decoder=h264,hevc,aac \
            --enable-parser=h264,hevc,aac
        make -j"$(nproc)"
    )
    $sudo make -C "$directory" install
}

setup_ffmpeg()
{
    if ffmpeg_is_new_enough ; then
        return
    fi
    if ! ffmpeg_is_new_enough $FFMPEG_PREFIX/lib/pkgconfig ; then
        step "The FFmpeg of this machine is too old for Mobcam."
        build_ffmpeg
        ffmpeg_is_new_enough $FFMPEG_PREFIX/lib/pkgconfig || die "the FFmpeg build did not take"
    fi
    export PKG_CONFIG_PATH=$FFMPEG_PREFIX/lib/pkgconfig
    export RUSTFLAGS="${RUSTFLAGS:-} -C link-arg=-Wl,-rpath,$FFMPEG_PREFIX/lib"
}

find_source()
{
    local script
    local root

    script=${BASH_SOURCE[0]:-}
    if [ -f "$script" ] ; then
        root=$(cd "$(dirname "$script")/../../.." && pwd)
        if [ -f "$root/crates/virtualcam/Cargo.toml" ] ; then
            source_dir=$root
            return
        fi
    fi

    source_dir=$CLONE_DIR
    if [ -d "$source_dir/.git" ] ; then
        step "Updating the clone in $source_dir."
        git -C "$source_dir" pull --ff-only
    else
        step "Cloning $REPOSITORY into $source_dir."
        git clone "$REPOSITORY" "$source_dir"
    fi
}

build()
{
    step "Building mobcam-virtualcam in $source_dir."
    (cd "$source_dir" && cargo build --locked --release --package mobcam-virtualcam)
}

install_binary()
{
    step "Installing $BINARY."
    $sudo install -m 755 "$source_dir/target/release/mobcam-virtualcam" $BINARY
}

setup_camera()
{
    step "Setting up the $CARD camera."
    write_file $MODPROBE_FILE <<EOF
options v4l2loopback card_label=$CARD exclusive_caps=1
options snd-aloop id=$CARD
EOF
    write_file $UDEV_FILE <<EOF
SUBSYSTEM=="video4linux", ATTR{name}=="$CARD", SYMLINK+="${VIDEO_DEVICE#/dev/}"
EOF
    $sudo udevadm control --reload
    if lsmod | grep -q '^v4l2loopback ' ; then
        $sudo modprobe -r v4l2loopback \
            || die "failed to unload v4l2loopback; something is using the camera"
    fi
    $sudo modprobe v4l2loopback
    $sudo udevadm settle
    [ -e $VIDEO_DEVICE ] || die "no $VIDEO_DEVICE after loading v4l2loopback"
}

build_aloop()
{
    local tag

    step "Building the snd-aloop module for $KERNEL."
    $sudo mkdir -p $ALOOP_DIR
    for tag in "v$ALOOP_VERSION" "v${ALOOP_VERSION%.*}" ; do
        if curl -fsSL "$ALOOP_URL?h=$tag" | write_file $ALOOP_DIR/aloop.c ; then
            break
        fi
        warn "no aloop.c for $tag"
    done
    [ -s $ALOOP_DIR/aloop.c ] || return 1
    write_file $ALOOP_DIR/Makefile <<EOF
obj-m += snd-aloop.o
snd-aloop-objs := aloop.o
EOF
    write_file $ALOOP_DIR/dkms.conf <<EOF
PACKAGE_NAME="snd-aloop"
PACKAGE_VERSION="$ALOOP_VERSION"
BUILT_MODULE_NAME[0]="snd-aloop"
DEST_MODULE_LOCATION[0]="/kernel/sound/drivers"
MAKE[0]="make -C \${kernel_source_dir} M=\${dkms_tree}/\${PACKAGE_NAME}/\${PACKAGE_VERSION}/build modules"
CLEAN="make -C \${kernel_source_dir} M=\${dkms_tree}/\${PACKAGE_NAME}/\${PACKAGE_VERSION}/build clean"
AUTOINSTALL="yes"
EOF
    $sudo dkms add -m snd-aloop -v "$ALOOP_VERSION" >/dev/null 2>&1 || true
    $sudo dkms build -m snd-aloop -v "$ALOOP_VERSION" || return 1
    $sudo dkms install -m snd-aloop -v "$ALOOP_VERSION" --force || return 1
}

setup_microphone()
{
    step "Setting up the $CARD microphone."
    if ! modinfo snd-aloop >/dev/null 2>&1 && ! build_aloop ; then
        audio=no
        warn "failed to build snd-aloop; installing without a microphone"
        return
    fi
    if lsmod | grep -q '^snd_aloop ' ; then
        $sudo modprobe -r snd-aloop || warn "failed to unload snd-aloop; keeping the loaded one"
    fi
    if ! $sudo modprobe snd-aloop ; then
        audio=no
        warn "failed to load snd-aloop; installing without a microphone"
    fi
}

setup_modules_load()
{
    {
        echo v4l2loopback
        if [ $audio = yes ] ; then
            echo snd-aloop
        fi
    } | write_file $MODULES_FILE
}

install_service()
{
    local arguments
    local present

    arguments="--device $VIDEO_DEVICE"
    present="test -e $VIDEO_DEVICE"
    if [ $audio = yes ] ; then
        arguments="$arguments --audio-backend alsa --audio-device $AUDIO_DEVICE"
        present="$present && test -e /proc/asound/$CARD"
    else
        arguments="$arguments --no-audio"
    fi

    step "Installing $SERVICE."
    write_file $SERVICE_FILE <<EOF
[Unit]
Description=Mobcam Virtual Camera
Documentation=https://github.com/eerimoq/mobcam
After=systemd-modules-load.service usbmuxd.service

[Service]
ExecStartPre=/bin/sh -c '$present'
ExecStart=$BINARY $arguments
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
EOF
    $sudo systemctl daemon-reload
    $sudo systemctl enable --now $SERVICE
}

summary()
{
    step "Installed."
    cat <<EOF
$($BINARY --version) is in $BINARY.

The camera is $VIDEO_DEVICE, labelled $CARD.
EOF
    if [ $audio = yes ] ; then
        cat <<EOF
The microphone is the $CARD sound card, which belaUI lists as an audio source
and belacoder reads with alsasrc device="hw:$CARD".
EOF
    else
        echo "There is no microphone; the video keeps going without one."
    fi
    cat <<EOF

Set the stream URL in Moblin to mobcam://localhost:7790, connect the iPhone or
iPad to the BELABOX with a USB cable, unlock it and tap Trust. Then add a
belacoder pipeline reading from $VIDEO_DEVICE to stream it.
EOF
    if [ $service = yes ] ; then
        cat <<EOF

$SERVICE is running. Follow it with

    sudo journalctl -fu $SERVICE
EOF
    else
        cat <<EOF

No service was installed. Start it by hand with

    sudo $BINARY --device $VIDEO_DEVICE
EOF
    fi
}

main()
{
    parse_arguments "$@"
    check_machine
    stop_service
    install_packages
    install_rust
    setup_ffmpeg
    find_source
    build
    install_binary
    setup_camera
    if [ $audio = yes ] ; then
        setup_microphone
    fi
    setup_modules_load
    if [ $service = yes ] ; then
        install_service
    fi
    summary
}

main "$@"
