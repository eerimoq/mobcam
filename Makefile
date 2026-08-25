PYTHON_DIRS += build.py
PYTHON_DIRS += ci.py

CODE_DIRS += $(PYTHON_DIRS)

default:

style:
	cargo fmt
	isort $(PYTHON_DIRS)
	ruff format $(PYTHON_DIRS)

style-check:
	cargo fmt --check
	isort $(PYTHON_DIRS) --check
	ruff format $(PYTHON_DIRS) --check

lint:
	cargo clippy --workspace --all-targets -- --deny warnings
	ruff check $(PYTHON_DIRS)
	mypy $(PYTHON_DIRS)

test:
	cargo test --workspace

spell-check:
	codespell $(CODE_DIRS)
