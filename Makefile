PYTHON ?= python3

.PHONY: validate test generate-goal install-skills check runtime-build runtime-test runtime-test-postgres runtime-lint runtime-check runtime-compose web-install web-dev web-check ui-dev ui-demo ui-open

validate:
	LIMINA_TELEMETRY_INTERNAL=1 $(PYTHON) scripts/kb_validate.py

test:
	LIMINA_TELEMETRY_INTERNAL=1 $(PYTHON) -m unittest discover -s tests

generate-goal:
	$(PYTHON) scripts/generate_goal.py --write

install-skills:
	bash scripts/install_skills.sh

check: test validate

# ─── Runtime (Rust) ───────────────────────────────────────────────────────

runtime-build:
	cargo build --release --locked --bin limina

runtime-test:
	cargo test --workspace --locked

# The same suites against PostgreSQL, in a throwaway container on port 55432.
runtime-test-postgres:
	docker rm -f limina-test-postgres >/dev/null 2>&1 || true
	docker run -d --name limina-test-postgres -e POSTGRES_USER=limina -e POSTGRES_PASSWORD=limina \
	  -e POSTGRES_DB=limina -p 127.0.0.1:55432:5432 postgres:17-alpine >/dev/null
	until docker exec limina-test-postgres pg_isready -U limina -d limina >/dev/null 2>&1; do sleep 1; done
	LIMINA_TEST_POSTGRES_URL=postgres://limina:limina@127.0.0.1:55432/limina \
	  cargo test --workspace --locked -- --test-threads=1; status=$$?; \
	  docker rm -f limina-test-postgres >/dev/null; exit $$status

runtime-lint:
	cargo fmt --all --check
	cargo clippy --workspace --all-targets --locked -- -D warnings

runtime-compose:
	OPENAI_API_KEY=test ANTHROPIC_API_KEY=test LIMINA_API_TOKEN=test docker compose config >/dev/null
	OPENAI_API_KEY=test ANTHROPIC_API_KEY=test LIMINA_API_TOKEN=test docker compose -f compose.cloud.yaml config >/dev/null

runtime-check: runtime-lint runtime-test runtime-compose validate

# ─── Web console ──────────────────────────────────────────────────────────

web-install:
	cd web && pnpm install --frozen-lockfile

# Hot-reloading UI on http://127.0.0.1:5173, proxied to a runtime on :7433 (see ui-dev).
web-dev:
	cd web && pnpm dev

web-check:
	cd web && pnpm typecheck && pnpm test && pnpm build

# Local runtime with a scripted engine: exercises the UI without provider credentials.
# LIMINA_DEV_IMPORT=<project folder> also imports a file-based project as development data.
# LIMINA_DEV_* lines in .env apply here (nothing else from .env does).
ui-dev:
	@set -a; [ -f .env ] && eval "$$(grep -E '^LIMINA_DEV_[A-Z_]+=' .env)"; set +a; \
	  cargo run -p limina-cli -- dev

# Seed the scripted demo project into the docker compose deployment (run before `up`).
ui-demo:
	docker compose run --rm --no-deps \
	  -e LIMINA_DEV_SEED=1 -e LIMINA_DEV_PACE=0.6 -e LIMINA_DEV_DATA=/tmp/limina-seed -e LIMINA_DEV_PORT=7439 \
	  -e LIMINA_DEV_DATABASE_URL=sqlite:////var/lib/limina/runtime.db limina \
	  sh -c "limina db upgrade && limina dev"

# Open the deployed console signed in with the admin token from .env (the token stays in the URL fragment).
ui-open:
	@port=$$(grep '^LIMINA_PORT=' .env | cut -d= -f2-); \
	open "http://127.0.0.1:$${port:-$${LIMINA_PORT:-7433}}/#token=$$(grep '^LIMINA_ADMIN_API_TOKEN=' .env | cut -d= -f2-)"
