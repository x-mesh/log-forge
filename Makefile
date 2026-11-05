CARGO ?= cargo
IMAGE ?= log-forge

.DEFAULT_GOAL := help
.PHONY: help build release test fmt fmt-check lint check run docker clean

help: ## Show targets
	@grep -E '^[a-z-]+:.*## ' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*## "} {printf "  %-10s %s\n", $$1, $$2}'

build: ## Build debug binaries
	$(CARGO) build --locked

release: ## Build release binaries
	$(CARGO) build --locked --release

test: ## Run tests
	$(CARGO) test --locked

fmt: ## Format code
	$(CARGO) fmt --all

fmt-check: ## Check formatting
	$(CARGO) fmt --all -- --check

lint: ## Run Clippy with warnings as errors
	$(CARGO) clippy --locked --all-targets -- -D warnings

check: fmt-check lint test ## Run the CI checks

run: release ## Generate three sample records
	./target/release/log-forge start --output console --total 3 --seed 42 --no-monitor

docker: ## Build the Docker image
	docker build -t $(IMAGE) .

clean: ## Remove build output
	$(CARGO) clean
