# recallweave — build, test, and demo automation.
#
# Targets:
#   make build        release build of the Rust engine
#   make test         Rust tests + viewer tests
#   make viewer       install deps and build the TypeScript viewer
#   make demo         run the demo script and regenerate the tapestry SVG
#   make verify       build + test + clippy + fmt-check + viewer build/test
#   make fmt          format Rust sources
#   make clean        remove build artifacts and the demo store

CARGO ?= cargo
NPM   ?= npm
NODE  ?= node

.PHONY: all build test viewer viewer-build viewer-test demo verify fmt fmt-check clippy clean

all: build

build:
	$(CARGO) build --release

test:
	$(CARGO) test
	$(MAKE) viewer-test

viewer: viewer-build

viewer-build:
	cd viewer && $(NPM) install && $(NPM) run build

viewer-test: viewer-build
	cd viewer && $(NPM) test

demo: build viewer-build
	sh examples/demo.sh
	$(NODE) viewer/dist/cli.js examples/memory-pack.json --svg docs/assets/tapestry-demo.svg

fmt:
	$(CARGO) fmt

fmt-check:
	$(CARGO) fmt --check

clippy:
	$(CARGO) clippy --all-targets -- -D warnings

verify: build clippy fmt-check test
	@echo "all checks passed"

clean:
	$(CARGO) clean
	rm -rf examples/demo-store viewer/dist viewer/node_modules
