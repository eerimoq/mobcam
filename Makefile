PYTHON_DIRS += scripts/build.py
PYTHON_DIRS += scripts/ci.py

CODE_DIRS += $(PYTHON_DIRS)
CODE_DIRS += crates/virtualcam/belabox/install.sh

CONFIG_DIR = .config

default:

build:
	python scripts/build.py deps
	python scripts/build.py build

package:
	python scripts/build.py package --installer

install:
	python scripts/build.py install

clean:
	python scripts/build.py clean

unit-test:
	cargo test --workspace

test:
	python -m tests.test $(TEST_ARGS)

test-generate-device-settings-clipboard:
	python -m tests.generate_device_settings $(TEST_ARGS)

test-generate-device-settings-stdout:
	python -m tests.generate_device_settings --force-stdout $(TEST_ARGS)

style:
	cargo fmt -- --config-path $(CONFIG_DIR)/rustfmt.toml
	isort --settings-path $(CONFIG_DIR)/isort.cfg $(PYTHON_DIRS)
	ruff format --config $(CONFIG_DIR)/ruff.toml $(PYTHON_DIRS)

style-check:
	cargo fmt --check -- --config-path $(CONFIG_DIR)/rustfmt.toml
	isort --settings-path $(CONFIG_DIR)/isort.cfg $(PYTHON_DIRS) --check
	ruff format --config $(CONFIG_DIR)/ruff.toml $(PYTHON_DIRS) --check

lint:
	cargo clippy --workspace --all-targets -- --deny warnings
	ruff check --config $(CONFIG_DIR)/ruff.toml $(PYTHON_DIRS)
	mypy --config-file $(CONFIG_DIR)/mypy.ini $(PYTHON_DIRS)

spell-check:
	codespell --config $(CONFIG_DIR)/codespellrc $(CODE_DIRS)
