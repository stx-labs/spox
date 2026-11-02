
# The package flags for cargo commands.
CARGO_EXCLUDES := --workspace
CARGO_FLAGS := --locked

build:
	cargo $(CARGO_FLAGS) build --all-targets $(CARGO_EXCLUDES) ${CARGO_BUILD_ARGS}

test:
	cargo $(CARGO_FLAGS) nextest run --features "testing" --lib $(CARGO_EXCLUDES) --no-fail-fast ${CARGO_BUILD_ARGS}

test-build:
	cargo $(CARGO_FLAGS) test build --features "testing" $(CARGO_EXCLUDES) --no-run --locked ${CARGO_BUILD_ARGS}

CARGO_FMT = cargo $(CARGO_FLAGS) fmt --all
CARGO_FMT_OPTIONS = --config group_imports=StdExternalCrate,imports_granularity=Item
CARGO_CLIPPY_BASE = cargo $(CARGO_FLAGS) clippy --workspace --all-targets --all-features --no-deps
CLIPPY_FLAGS = -D warnings

lint:
	$(CARGO_FMT) -- $(CARGO_FMT_OPTIONS) --check
	$(CARGO_CLIPPY_BASE) -- $(CLIPPY_FLAGS)

format:
	$(CARGO_FMT) -- $(CARGO_FMT_OPTIONS)
	$(CARGO_CLIPPY_BASE) --fix -- $(CLIPPY_FLAGS)

clean:
	cargo $(CARGO_FLAGS) clean

.PHONY: build test test-build lint format clean

# ##############################################################################
# NEXTEST
# ##############################################################################

NEXTEST_ARCHIVE_FILE := target/nextest/nextest-archive.tar.zst
NEXTEST_SERIAL_ARCHIVE_FILE := target/nextest/nextest-archive-serial.tar.zst

# Creates nextest archives
nextest-archive:
	cargo $(CARGO_FLAGS) nextest archive --features "testing" $(CARGO_EXCLUDES) --lib --archive-file $(NEXTEST_ARCHIVE_FILE) ${CARGO_BUILD_ARGS}

# Runs nextest archives
nextest-archive-run:
	cargo $(CARGO_FLAGS) nextest run --no-fail-fast --retries 2 --archive-file $(NEXTEST_ARCHIVE_FILE)

nextest-archive-clean:
	rm -f $(NEXTEST_ARCHIVE_FILE) $(NEXTEST_SERIAL_ARCHIVE_FILE)

.PHONY: nextest-archive nextest-archive-run nextest-archive-clean
