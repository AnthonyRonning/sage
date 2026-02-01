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

# Stage 4: Runtime
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

# Create non-root user
RUN useradd -m -u 1000 sage

WORKDIR /app

# Copy the binary from builder
COPY --from=builder /app/target/release/sage /app/sage

# Copy migrations for diesel
COPY --from=builder /app/crates/sage-core/migrations /app/migrations

# Create workspace directory
RUN mkdir -p /workspace && chown sage:sage /workspace

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
