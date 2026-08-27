MYPY_DIRS += scripts/build.py
MYPY_DIRS += scripts/ci.py

PYTHON_DIRS += $(MYPY_DIRS)
PYTHON_DIRS += tests

CODE_DIRS += $(PYTHON_DIRS)
CODE_DIRS += crates/virtualcam/belabox/install.sh
CODE_DIRS += tests/belabox/setup.sh

CONFIG_DIR = .config
PYTHON = python
BELABOX = user@belabox.local

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
	$(PYTHON) -m tests.test $(TEST_ARGS)

test-remote:
	rsync -a --delete \
	    --exclude .git \
	    --exclude .venv \
	    --exclude .deps \
	    --exclude target \
	    --exclude release \
	    --exclude logs \
	    --exclude tests/files \
	    ./ $(BELABOX):mobcam/
	status=0 ; \
	ssh $(BELABOX) 'cd mobcam && make test PYTHON=.venv/bin/python TEST_ARGS="$(TEST_ARGS)"' || status=$$? ; \
	rsync -a $(BELABOX):mobcam/logs/ logs/ ; \
	rsync -a $(BELABOX):mobcam/tests/files/ tests/files/ ; \
	exit $$status

test-generate-device-settings-clipboard:
	$(PYTHON) -m tests.generate_device_settings $(TEST_ARGS)

test-generate-device-settings-stdout:
	$(PYTHON) -m tests.generate_device_settings --force-stdout $(TEST_ARGS)

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
	mypy --config-file $(CONFIG_DIR)/mypy.ini $(MYPY_DIRS)

spell-check:
	codespell --config $(CONFIG_DIR)/codespellrc $(CODE_DIRS)
