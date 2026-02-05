# Sage - Personal AI Agent
# Run `just` to see available commands

set dotenv-load

default:
    @just --list

# =============================================================================
# Container Management (Primary)
# =============================================================================

# Build Sage Docker image
build:
    podman build -f Dockerfile -t sage:latest .

# Start all containers (postgres, signal-cli, sage)
start:
    #!/usr/bin/env bash
    set -e
    set -a
    source .env
    set +a
    
    echo "Starting Sage stack..."
    
    # Start PostgreSQL if not running
    if ! podman ps --format '{{{{.Names}}}}' | grep -q '^sage-postgres$'; then
        echo "Starting PostgreSQL..."
        podman run -d --name sage-postgres \
            -e POSTGRES_USER=sage -e POSTGRES_PASSWORD=sage -e POSTGRES_DB=sage \
            -v sage-pgdata:/var/lib/postgresql/data \
            -p 5434:5432 pgvector/pgvector:pg17
        sleep 3
    else
        echo "PostgreSQL already running"
    fi
    
    # Start signal-cli if not running
    if ! podman ps --format '{{{{.Names}}}}' | grep -q '^sage-signal-cli$'; then
        echo "Starting signal-cli..."
        podman run -d --name sage-signal-cli \
            -p 7583:7583 -v signal-cli-data:/var/lib/signal-cli --tmpfs /tmp:exec \
            registry.gitlab.com/packaging/signal-cli/signal-cli-native:latest \
            -v daemon --tcp 0.0.0.0:7583 --send-read-receipts
        sleep 2
    else
        echo "signal-cli already running"
    fi
    
    # Create workspace directory if it doesn't exist
    mkdir -p ~/.sage/workspace
    
    # Remove old sage container and start fresh
    podman rm -f sage 2>/dev/null || true
    echo "Starting Sage..."
    podman run -d --name sage --network host \
        -v ~/.sage/workspace:/workspace:U,z \
        -e DATABASE_URL=postgres://sage:sage@localhost:5434/sage \
        -e MAPLE_API_URL="$MAPLE_API_URL" \
        -e MAPLE_API_KEY="$MAPLE_API_KEY" \
        -e MAPLE_MODEL="$MAPLE_MODEL" \
        -e MAPLE_EMBEDDING_MODEL="$MAPLE_EMBEDDING_MODEL" \
        -e SIGNAL_CLI_HOST=localhost -e SIGNAL_CLI_PORT=7583 \
        -e SIGNAL_PHONE_NUMBER="$SIGNAL_PHONE_NUMBER" \
        -e SIGNAL_ALLOWED_USERS="$SIGNAL_ALLOWED_USERS" \
        -e BRAVE_API_KEY="$BRAVE_API_KEY" \
        -e SAGE_WORKSPACE=/workspace \
        -e RUST_LOG=info \
        sage:latest
    
    sleep 2
    echo ""
    echo "🌿 Sage stack started!"
    echo "   - PostgreSQL: localhost:5434 (data: sage-pgdata volume)"
    echo "   - signal-cli: localhost:7583 (data: signal-cli-data volume)"
    echo "   - Sage: running"
    echo "   - Workspace: ~/.sage/workspace"
    echo ""
    echo "View logs: just logs"
    echo "Stop:      just stop"

# Stop all containers (preserves data volumes)
stop:
    #!/usr/bin/env bash
    echo "Stopping Sage stack..."
    podman rm -f sage 2>/dev/null || true
    podman rm -f sage-signal-cli 2>/dev/null || true
    podman rm -f sage-postgres 2>/dev/null || true
    echo "Containers stopped. Data preserved in volumes (sage-pgdata, signal-cli-data)."

# Restart Sage only (keeps postgres and signal-cli running)
restart:
    #!/usr/bin/env bash
    set -a
    source .env
    set +a
    
    mkdir -p ~/.sage/workspace
    podman rm -f sage 2>/dev/null || true
    podman run -d --name sage --network host \
        -v ~/.sage/workspace:/workspace:U,z \
        -e DATABASE_URL=postgres://sage:sage@localhost:5434/sage \
        -e MAPLE_API_URL="$MAPLE_API_URL" \
        -e MAPLE_API_KEY="$MAPLE_API_KEY" \
        -e MAPLE_MODEL="$MAPLE_MODEL" \
        -e MAPLE_EMBEDDING_MODEL="$MAPLE_EMBEDDING_MODEL" \
        -e SIGNAL_CLI_HOST=localhost -e SIGNAL_CLI_PORT=7583 \
        -e SIGNAL_PHONE_NUMBER="$SIGNAL_PHONE_NUMBER" \
        -e SIGNAL_ALLOWED_USERS="$SIGNAL_ALLOWED_USERS" \
        -e BRAVE_API_KEY="$BRAVE_API_KEY" \
        -e SAGE_WORKSPACE=/workspace \
        -e RUST_LOG=info \
        sage:latest
    echo "Sage restarted"

# View Sage logs
logs:
    podman logs -f sage

# View all container logs
logs-all:
    #!/usr/bin/env bash
    echo "=== PostgreSQL ===" && podman logs --tail 10 sage-postgres
    echo ""
    echo "=== signal-cli ===" && podman logs --tail 10 sage-signal-cli
    echo ""
    echo "=== Sage ===" && podman logs -f sage

# Show container status
status:
    podman ps -a --filter "name=sage" --format "table {{{{.Names}}}}\t{{{{.Status}}}}\t{{{{.Ports}}}}"

# Connect to PostgreSQL
psql:
    podman exec -it sage-postgres psql -U sage -d sage

# Shell into Sage container
shell:
    podman exec -it sage bash

# =============================================================================
# First-Time Setup
# =============================================================================

# Initialize signal-cli data volume (run once after registering signal-cli locally)
signal-init:
    #!/usr/bin/env bash
    set -e
    echo "Copying signal-cli data to Docker volume..."
    podman volume create signal-cli-data 2>/dev/null || true
    podman run --rm \
        -v ~/.local/share/signal-cli/data:/src:ro \
        -v signal-cli-data:/dest \
        docker.io/alpine:latest \
        sh -c "mkdir -p /dest/.local/share/signal-cli/data && cp -a /src/. /dest/.local/share/signal-cli/data/ && chown -R 101:101 /dest/"
    echo "Done! signal-cli data copied to volume."
    echo "Verify with: podman run --rm -v signal-cli-data:/var/lib/signal-cli registry.gitlab.com/packaging/signal-cli/signal-cli-native:latest listAccounts"

# =============================================================================
# Development (Local)
# =============================================================================

# Build Rust agent locally
build-local:
    cargo build --release

# Run Rust agent locally (uses signal-cli subprocess mode)
run:
    cargo run --release

# Run with debug logging
run-debug:
    RUST_LOG=debug cargo run --release

# Check code
check:
    cargo check

# Run tests
test:
    cargo test

# Format code
fmt:
    cargo fmt

# Lint code
lint:
    cargo clippy

# =============================================================================
# Data Management
# =============================================================================

# List all Sage-related volumes
volumes:
    podman volume ls | grep -E "sage|signal"

# DANGER: Delete all data and start fresh
nuke:
    #!/usr/bin/env bash
    echo "⚠️  This will DELETE ALL SAGE DATA including:"
    echo "   - PostgreSQL database (memory, conversations, archival)"
    echo "   - signal-cli registration"
    echo ""
    read -p "Type 'DELETE' to confirm: " confirm
    if [ "$confirm" = "DELETE" ]; then
        just stop
        podman volume rm -f sage-pgdata signal-cli-data 2>/dev/null || true
        echo "All data deleted."
    else
        echo "Aborted."
    fi

# =============================================================================
# Development Setup
# =============================================================================

# Set up git hooks for pre-commit checks
setup-hooks:
    git config core.hooksPath .githooks
    @echo "✅ Git hooks configured. Pre-commit will run fmt, clippy, and tests."

# Run all CI checks (same as pre-commit hook)
ci-check:
    cargo fmt --all -- --check
    cargo clippy --all-targets --all-features -- -D warnings
    cargo test --all-features

# =============================================================================
# GEPA Prompt Optimization
# =============================================================================

# Evaluate current AGENT_INSTRUCTION against training examples (baseline score)
gepa-eval:
    cargo run --release --bin gepa-optimize -- --eval

# Run GEPA optimization loop (Claude as judge, Kimi as program)
# Requires ANTHROPIC_API_KEY env var for Claude judge
gepa-optimize:
    cargo run --release --bin gepa-optimize -- --optimize

# Show current optimized instruction
gepa-show:
    @cat optimized_instructions/latest.txt 2>/dev/null || echo "No optimized instruction found. Run 'just gepa-optimize' first."

# Show GEPA training examples
gepa-examples:
    @echo "GEPA training examples in examples/gepa/trainset.json"
    @echo ""
    @echo "Categories:"
    @grep -o '"category": "[^"]*"' examples/gepa/trainset.json | sort | uniq -c
    @echo ""
    @echo "Total examples: $(grep -c '"id":' examples/gepa/trainset.json)"
