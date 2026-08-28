#!/usr/bin/env bash
#
# Build and install Mobcam Virtual Camera on a BELABOX.
#
# Run it on the BELABOX, either from a clone of the repository or on its own,
# in which case it clones the repository itself:
#
#     ./scripts/belabox/install.sh
#
# It installs the build dependencies and the v4l2loopback module, builds the
# snd-aloop that BELABOX does not have and an FFmpeg that decodes and encodes in
# the RK3588 hardware, builds and installs the binary, creates the Mobcam camera
# and the Mobcam sound card, both loaded at boot, adds the belacoder pipeline
# belaUI streams the camera with, and runs mobcam-virtualcam as a service.
#
# The FFmpeg is nyanmisaka's ffmpeg-rockchip, which decodes H.264 and HEVC in
# the video unit of the RK3588 the BELABOX is built around, through the Rockchip
# Media Process Platform (MPP) it also builds. Both are installed under
# /opt/mobcam, leaving the FFmpeg and the MPP of the machine alone.
#
# The ffmpeg and ffprobe command line tools are built along with the libraries,
# with the Rockchip encoders, the V4L2 and ALSA input devices, the AAC encoder
# and the MP4 muxer, so ffmpeg records a camera and a microphone into an MP4
# file, the video in hardware. They are installed in /opt/mobcam, which is not
# in the path, and the ffmpeg of the machine stays the one that runs when ffmpeg
# is typed.

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
MPP_UDEV_FILE=/etc/udev/rules.d/71-mobcam-mpp.rules
VIDEO_DEVICE=/dev/mobcam
CAMERA_BUFFERS=8
SETUP_FILE=/opt/belaUI/setup.json
PIPELINES_DIR=/usr/share/belacoder/pipelines
PIPELINE_TEMPLATE=h265_camlink
PIPELINE=h265_mobcam
AUDIO_DEVICE=plughw:CARD=$CARD,DEV=1
AUDIO_CAPTURE_DEVICE=plughw:CARD=$CARD,DEV=0
FFMPEG_PREFIX=/opt/mobcam/ffmpeg
FFMPEG_CLI=$FFMPEG_PREFIX/bin/ffmpeg
FFMPEG_MINIMUM=59.37.100
FFMPEG_REPOSITORY=${MOBCAM_FFMPEG_REPOSITORY:-https://github.com/nyanmisaka/ffmpeg-rockchip.git}
FFMPEG_BRANCH=${MOBCAM_FFMPEG_BRANCH:-7.1}
MPP_PREFIX=/opt/mobcam/mpp
MPP_REPOSITORY=${MOBCAM_MPP_REPOSITORY:-https://github.com/nyanmisaka/mpp.git}
MPP_BRANCH=${MOBCAM_MPP_BRANCH:-jellyfin-mpp}
BUILD_DIR=${MOBCAM_BUILD_DIR:-$HOME/.cache/mobcam}
TMP_DIR=$BUILD_DIR/tmp
KERNEL=$(uname -r)
KERNEL_HEADERS=/lib/modules/$KERNEL/build
ALOOP_VERSION=${KERNEL%%-*}
ALOOP_DIR=/usr/src/snd-aloop-$ALOOP_VERSION
ALOOP_URL=https://git.kernel.org/pub/scm/linux/kernel/git/stable/linux.git/plain/sound/drivers/aloop.c

PACKAGES=(
    build-essential
    cmake
    curl
    dkms
    git
    libasound2-dev
    libclang-dev
    libdrm-dev
    pkg-config
    usbmuxd
    v4l2loopback-dkms
    xz-utils
)

audio=yes
packages=yes
pipeline=yes
service=yes
sudo=

usage()
{
    cat <<EOF
usage: $(basename "$0") [--no-audio] [--no-packages] [--no-pipeline]
       [--no-service] [--help]

Build and install Mobcam Virtual Camera on a BELABOX.

  --no-audio     do not set up the Mobcam sound card, video only
  --no-packages  do not install the build dependencies, they are already there
  --no-pipeline  do not add the belacoder pipeline belaUI streams with
  --no-service   do not run mobcam-virtualcam as a service
  --help         print this text and exit

The source is the clone this script is part of, or a clone of
$REPOSITORY
in $CLONE_DIR. Set MOBCAM_REPOSITORY and MOBCAM_SOURCE_DIR to
change either, and MOBCAM_BUILD_DIR to build FFmpeg and MPP somewhere else
than $BUILD_DIR. MOBCAM_FFMPEG_REPOSITORY, MOBCAM_FFMPEG_BRANCH,
MOBCAM_MPP_REPOSITORY and MOBCAM_MPP_BRANCH pick which ffmpeg-rockchip and
MPP to build.
EOF
}

step()
{
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
            --no-packages) packages=no ;;
            --no-pipeline) pipeline=no ;;
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
    if [ ! -e /dev/mpp_service ] ; then
        warn "no /dev/mpp_service; this machine has no Rockchip video unit to decode in"
    fi
    if [ "$(id -u)" -ne 0 ] ; then
        sudo=sudo
        if ! sudo -n true 2>/dev/null && ! sudo -v ; then
            die "sudo is needed to install"
        fi
    fi
}

setup_tmp_dir()
{
    # /tmp is mounted noexec on a BELABOX, which neither the rustup installer
    # nor the FFmpeg configure can run from.
    mkdir -p "$TMP_DIR"
    export TMPDIR=$TMP_DIR
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

ffmpeg_supports()
{
    [ -x $FFMPEG_CLI ] || return 1
    $FFMPEG_CLI -hide_banner -loglevel quiet -"$1" 2>/dev/null | grep -qw "$2"
}

ffmpeg_is_installed()
{
    PKG_CONFIG_PATH=$FFMPEG_PREFIX/lib/pkgconfig \
        pkg-config --atleast-version=$FFMPEG_MINIMUM libavcodec 2>/dev/null \
        && ffmpeg_supports decoders h264_rkmpp \
        && ffmpeg_supports encoders h264_rkmpp \
        && ffmpeg_supports encoders aac \
        && ffmpeg_supports devices v4l2 \
        && ffmpeg_supports devices alsa \
        && ffmpeg_supports muxers mp4
}

clone()
{
    local repository
    local branch
    local directory

    repository=$1
    branch=$2
    directory=$3
    mkdir -p "$(dirname "$directory")"
    if [ -d "$directory/.git" ] ; then
        git -C "$directory" fetch --depth 1 origin "$branch"
        git -C "$directory" checkout --quiet --detach FETCH_HEAD
    else
        rm -rf "$directory"
        git clone --depth 1 --branch "$branch" "$repository" "$directory"
    fi
}

build_mpp()
{
    local directory

    step "Building the Rockchip MPP $MPP_BRANCH in $BUILD_DIR."
    directory=$BUILD_DIR/rkmpp
    clone "$MPP_REPOSITORY" "$MPP_BRANCH" "$directory"
    cmake \
        -S "$directory" \
        -B "$directory/build" \
        -DCMAKE_INSTALL_PREFIX=$MPP_PREFIX \
        -DCMAKE_BUILD_TYPE=Release \
        -DBUILD_SHARED_LIBS=ON \
        -DBUILD_TEST=OFF
    make -C "$directory/build" -j"$(nproc)"
    $sudo make -C "$directory/build" install
}

setup_mpp()
{
    # The MPP of the machine is often too old for the decoders, and replacing it
    # would take the hardware encoder of belacoder with it, so this one is its
    # own and only FFmpeg links against it.
    if [ ! -f $MPP_PREFIX/lib/pkgconfig/rockchip_mpp.pc ] ; then
        build_mpp
    fi
}

build_ffmpeg()
{
    local directory

    step "Building ffmpeg-rockchip $FFMPEG_BRANCH in $BUILD_DIR."
    directory=$BUILD_DIR/ffmpeg-rockchip
    clone "$FFMPEG_REPOSITORY" "$FFMPEG_BRANCH" "$directory"
    (
        cd "$directory"
        export PKG_CONFIG_PATH=$MPP_PREFIX/lib/pkgconfig
        ./configure \
            --prefix=$FFMPEG_PREFIX \
            --enable-shared \
            --disable-static \
            --disable-autodetect \
            --disable-doc \
            --disable-network \
            --disable-everything \
            --disable-ffplay \
            --enable-ffmpeg \
            --enable-ffprobe \
            --enable-rpath \
            --enable-gpl \
            --enable-version3 \
            --enable-libdrm \
            --enable-rkmpp \
            --enable-alsa \
            --enable-decoder=h264,hevc,aac,rawvideo,mjpeg,pcm_s16le,h264_rkmpp,hevc_rkmpp \
            --enable-encoder=h264_rkmpp,hevc_rkmpp,aac,wrapped_avframe \
            --enable-parser=h264,hevc,aac,mjpeg \
            --enable-demuxer=mov,h264,hevc \
            --enable-muxer=mp4,null \
            --enable-indev=v4l2,alsa \
            --enable-protocol=file,pipe \
            --enable-filter=format,scale,fps,copy,hwupload,hwdownload,null,anull,aformat,aresample \
            --enable-bsf=extract_extradata,h264_mp4toannexb,hevc_mp4toannexb \
            --extra-ldflags="-Wl,-rpath,$MPP_PREFIX/lib"
        make -j"$(nproc)"
    )
    $sudo make -C "$directory" install
}

setup_ffmpeg()
{
    # The FFmpeg of the machine decodes in software only, so Mobcam brings one
    # with the Rockchip decoders and encoders of the RK3588 whatever the machine
    # has. Only the libraries are used by mobcam-virtualcam; the ffmpeg and
    # ffprobe tools are there to record a camera and a microphone into an MP4
    # file, the video in hardware, and to look at what came out.
    if ! ffmpeg_is_installed ; then
        step "This machine has no FFmpeg that decodes and encodes in the RK3588 hardware."
        setup_mpp
        build_ffmpeg
        ffmpeg_is_installed || die "the FFmpeg build did not take"
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
        root=$(cd "$(dirname "$script")/../.." && pwd)
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
options v4l2loopback card_label=$CARD exclusive_caps=1 max_buffers=$CAMERA_BUFFERS
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

setup_video_unit()
{
    # The machine ships /dev/mpp_service to root alone, which leaves the
    # hardware encoders and decoders of the RK3588 out of reach of everything
    # not running as root, ffmpeg included. This hands it to the video group,
    # the group that may read the camera.
    step "Handing the RK3588 video unit to the video group."
    write_file $MPP_UDEV_FILE <<EOF
KERNEL=="mpp_service", MODE="0660", GROUP="video"
EOF
    $sudo udevadm control --reload
    $sudo udevadm trigger --name-match=mpp_service
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

setup_value()
{
    sed -n "s/.*\"$1\" *: *\"\([^\"]*\)\".*/\1/p" $SETUP_FILE 2>/dev/null
}

pipelines_dir()
{
    local path

    path=$(setup_value belacoder_path)
    case $path in
        "") echo $PIPELINES_DIR ;;
        *) echo "$path/pipeline" ;;
    esac
}

install_pipeline()
{
    local dir
    local template
    local expressions

    dir=$(pipelines_dir)
    template=$dir/$(setup_value hw)/$PIPELINE_TEMPLATE
    if [ ! -f "$template" ] ; then
        warn "no $template to make the pipeline from; skipping it"
        pipeline=no
        return
    fi

    # The template streams a USB camera and its audio, which is the same
    # pipeline as this one but for the two devices it reads from.
    step "Installing the $PIPELINE pipeline."
    expressions=(-e "s|\(v4l2src device=\)[^ ]*|\1$VIDEO_DEVICE|")
    if [ $audio = yes ] ; then
        expressions+=(-e "s|\(alsasrc device=\)[^ ]*|\1hw:$CARD|")
    fi
    $sudo mkdir -p "$dir/custom"
    sed "${expressions[@]}" "$template" | write_file "$dir/custom/$PIPELINE"
}

restart_belaui()
{
    if [ ! -f $SETUP_FILE ] || ! systemctl is-active --quiet belaUI ; then
        return
    fi
    if pgrep -x belacoder >/dev/null ; then
        warn "belaUI is streaming; restart it later to see the pipeline"
        return
    fi
    step "Restarting belaUI, which reads the pipelines when it starts."
    $sudo systemctl restart belaUI
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
Wants=usbmuxd.service

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
    step "Summary."
    echo "Binary:     $BINARY"
    echo "Camera:     $VIDEO_DEVICE"
    if [ $audio = yes ] ; then
        echo "Microphone: $CARD"
    fi
    if [ $pipeline = yes ] ; then
        echo "Pipeline:   $(pipelines_dir)/custom/$PIPELINE"
    fi
    if [ $service = yes ] ; then
        echo "Service:    $SERVICE"
    fi
    echo "Test: sudo $FFMPEG_CLI -f v4l2 -i $VIDEO_DEVICE -f alsa -i \\"
    echo "          $AUDIO_CAPTURE_DEVICE -c:v h264_rkmpp -c:a aac \\"
    echo "          recording.mp4"
}

main()
{
    parse_arguments "$@"
    check_machine
    setup_tmp_dir
    stop_service
    if [ $packages = yes ] ; then
        install_packages
    fi
    install_rust
    setup_ffmpeg
    find_source
    build
    install_binary
    setup_video_unit
    setup_camera
    if [ $audio = yes ] ; then
        setup_microphone
    fi
    setup_modules_load
    if [ $pipeline = yes ] ; then
        install_pipeline
    fi
    if [ $service = yes ] ; then
        install_service
    fi
    if [ $pipeline = yes ] ; then
        restart_belaui
    fi
    summary
}

main "$@"
