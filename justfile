code_dirs := "scripts tests"
config_dir := ".config"
python := "python"
belabox := "user@belabox.local"

default:
    @just --list

build:
    {{python}} scripts/build.py deps
    {{python}} scripts/build.py build

package:
    {{python}} scripts/build.py package --installer

install:
    {{python}} scripts/build.py install

clean:
    {{python}} scripts/build.py clean

unit-test:
    cargo test --workspace

test-obs-plugin *args:
    {{python}} -m tests.test_obs_plugin {{args}}

test-virtualcam *args:
    {{python}} -m tests.test_virtualcam {{args}}

test-virtualcam-belabox *args:
    #!/usr/bin/env bash
    set -u
    rsync \
        --archive \
        --delete \
        --exclude .git \
        --exclude .venv \
        --exclude .deps \
        --exclude target \
        --exclude release \
        --exclude logs \
        --exclude tests/files \
        ./ \
        {{belabox}}:mobcam/
    status=0
    ssh -t {{belabox}} 'cd mobcam && ./scripts/belabox/test.sh {{args}}' || status=$?
    rsync --archive {{belabox}}:mobcam/logs/ logs/
    rsync --archive {{belabox}}:mobcam/tests/files/ tests/files/
    exit $status

test-generate-device-settings-clipboard *args:
    {{python}} -m tests.generate_device_settings {{args}}

test-generate-device-settings-stdout *args:
    {{python}} -m tests.generate_device_settings --force-stdout {{args}}

style:
    cargo fmt -- --config-path {{config_dir}}/rustfmt.toml
    isort --settings-path {{config_dir}}/isort.cfg {{code_dirs}}
    ruff format --config {{config_dir}}/ruff.toml {{code_dirs}}

style-check:
    cargo fmt --check -- --config-path {{config_dir}}/rustfmt.toml
    isort --settings-path {{config_dir}}/isort.cfg {{code_dirs}} --check
    ruff format --config {{config_dir}}/ruff.toml {{code_dirs}} --check

lint:
    cargo clippy --workspace --all-targets -- --deny warnings
    ruff check --config {{config_dir}}/ruff.toml {{code_dirs}}
    mypy --config-file {{config_dir}}/mypy.ini {{code_dirs}}

spell-check:
    codespell --config {{config_dir}}/codespellrc {{code_dirs}}
