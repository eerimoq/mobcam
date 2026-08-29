CODE_DIRS += scripts
CODE_DIRS += tests

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

test-obs-plugin:
	$(PYTHON) -m tests.test_obs_plugin $(TEST_ARGS)

test-virtualcam:
	$(PYTHON) -m tests.test_virtualcam $(TEST_ARGS)

test-virtualcam-belabox:
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
		$(BELABOX):mobcam/
	status=0 ; \
	ssh -t $(BELABOX) 'cd mobcam && ./scripts/belabox/test.sh $(TEST_ARGS)' || status=$$? ; \
	rsync -a $(BELABOX):mobcam/logs/ logs/ ; \
	rsync -a $(BELABOX):mobcam/tests/files/ tests/files/ ; \
	exit $$status

test-generate-device-settings-clipboard:
	$(PYTHON) -m tests.generate_device_settings $(TEST_ARGS)

test-generate-device-settings-stdout:
	$(PYTHON) -m tests.generate_device_settings --force-stdout $(TEST_ARGS)

style:
	cargo fmt -- --config-path $(CONFIG_DIR)/rustfmt.toml
	isort --settings-path $(CONFIG_DIR)/isort.cfg $(CODE_DIRS)
	ruff format --config $(CONFIG_DIR)/ruff.toml $(CODE_DIRS)

style-check:
	cargo fmt --check -- --config-path $(CONFIG_DIR)/rustfmt.toml
	isort --settings-path $(CONFIG_DIR)/isort.cfg $(CODE_DIRS) --check
	ruff format --config $(CONFIG_DIR)/ruff.toml $(CODE_DIRS) --check

lint:
	cargo clippy --workspace --all-targets -- --deny warnings
	ruff check --config $(CONFIG_DIR)/ruff.toml $(CODE_DIRS)
	mypy --config-file $(CONFIG_DIR)/mypy.ini $(CODE_DIRS)

spell-check:
	codespell --config $(CONFIG_DIR)/codespellrc $(CODE_DIRS)
