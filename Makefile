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

test:
	cargo test --workspace

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
