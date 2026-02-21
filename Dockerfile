# Sage V2 - Rust-based AI agent
# Multi-stage build with cargo-chef for optimal layer caching

# Stage 1: Chef - Install cargo-chef
FROM docker.io/rust:1.83-bookworm AS chef

# Install Rust nightly first (required for edition2024 in cargo-chef deps)
RUN rustup toolchain install nightly \
    && rustup default nightly

RUN cargo install cargo-chef

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    libpq-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Stage 2: Planner - Analyze dependencies
FROM chef AS planner

COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/

RUN cargo chef prepare --recipe-path recipe.json

# Stage 3: Builder - Build dependencies (cached) then source
FROM chef AS builder

# Copy recipe and build dependencies only (this layer is cached!)
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json

# Now copy source and build (only recompiles our code)
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/

RUN cargo build --release

# Stage 4: Build marmotd (optional MLS messaging sidecar)
FROM chef AS marmotd-builder
ARG MARMOTD_VERSION=0.3.2
RUN git clone --depth 1 --branch marmotd-v${MARMOTD_VERSION} https://github.com/sledtools/pika.git /marmotd-src
WORKDIR /marmotd-src
RUN cargo build -p marmotd --release

# Stage 5: Runtime
FROM docker.io/debian:bookworm-slim

# Install runtime dependencies and comprehensive CLI toolset
RUN apt-get update && apt-get install -y --no-install-recommends \
    # Runtime libs
    libssl3 \
    libpq5 \
    ca-certificates \
    gnupg \
    # Core utilities
    curl \
    wget \
    # Text processing
    jq \
    yq \
    sed \
    gawk \
    grep \
    # File utilities
    file \
    tree \
    zip \
    unzip \
    tar \
    gzip \
    bzip2 \
    xz-utils \
    # Network tools
    netcat-openbsd \
    dnsutils \
    iputils-ping \
    openssh-client \
    # Development tools
    git \
    make \
    build-essential \
    # Data processing
    sqlite3 \
    csvtool \
    # System utilities
    procps \
    htop \
    less \
    vim-tiny \
    nano \
    # Image processing (lightweight)
    imagemagick \
    # PDF tools
    poppler-utils \
    && rm -rf /var/lib/apt/lists/*

# Install Node.js 20.x (for JavaScript execution)
RUN curl -fsSL https://deb.nodesource.com/setup_20.x | bash - \
    && apt-get install -y nodejs \
    && rm -rf /var/lib/apt/lists/*

# Install GitHub CLI
RUN curl -fsSL https://cli.github.com/packages/githubcli-archive-keyring.gpg | dd of=/usr/share/keyrings/githubcli-archive-keyring.gpg \
    && chmod go+r /usr/share/keyrings/githubcli-archive-keyring.gpg \
    && echo "deb [arch=$(dpkg --print-architecture) signed-by=/usr/share/keyrings/githubcli-archive-keyring.gpg] https://cli.github.com/packages stable main" | tee /etc/apt/sources.list.d/github-cli.list > /dev/null \
    && apt-get update && apt-get install -y gh \
    && rm -rf /var/lib/apt/lists/*

# Install just (command runner)
RUN curl --proto '=https' --tlsv1.2 -sSf https://just.systems/install.sh | bash -s -- --to /usr/local/bin

# Install Rust toolchain (for development)
ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:$PATH
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable --profile minimal \
    && rustup component add rustfmt clippy \
    && chmod -R a+w $RUSTUP_HOME $CARGO_HOME

# Install additional dev tools (ripgrep, fd, fzf, tmux)
RUN apt-get update && apt-get install -y --no-install-recommends \
    ripgrep \
    fd-find \
    fzf \
    tmux \
    && rm -rf /var/lib/apt/lists/* \
    && ln -s $(which fdfind) /usr/local/bin/fd

# Install Python 3.11 with useful packages
RUN apt-get update && apt-get install -y --no-install-recommends \
    python3 \
    python3-pip \
    python3-venv \
    && rm -rf /var/lib/apt/lists/* \
    && pip3 install --no-cache-dir --break-system-packages \
    requests \
    httpx \
    beautifulsoup4 \
    lxml \
    pandas \
    pyyaml \
    toml \
    rich

# Copy marmotd binary (MLS messaging sidecar, built from pika source)
COPY --from=marmotd-builder /marmotd-src/target/release/marmotd /usr/local/bin/marmotd

# Create non-root user
RUN useradd -m -u 1000 sage

WORKDIR /app

# Copy the binary from builder
COPY --from=builder /app/target/release/sage /app/sage

# Copy migrations for diesel
COPY --from=builder /app/crates/sage-core/migrations /app/migrations

# Create workspace and marmot state directories
RUN mkdir -p /workspace /data/marmot-state && chown sage:sage /workspace /data/marmot-state

# Run as non-root user
USER sage

# Environment defaults (can be overridden)
ENV RUST_LOG=info
ENV DATABASE_URL=postgres://sage:sage@postgres:5432/sage
ENV MAPLE_API_URL=http://host.docker.internal:8089/v1
ENV SIGNAL_CLI_HOST=signal-cli
ENV SIGNAL_CLI_PORT=7583
ENV SAGE_WORKSPACE=/workspace
ENV HEALTH_PORT=8080

# Expose health check port
EXPOSE 8080

# Health check using curl
HEALTHCHECK --interval=30s --timeout=5s --start-period=30s --retries=3 \
    CMD curl -f http://localhost:8080/health || exit 1

# Run sage
CMD ["/app/sage"]
