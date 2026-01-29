# Sage V2 - Rust-based AI agent
# Multi-stage build for smaller final image

# Stage 1: Build
FROM docker.io/rust:1.83-bookworm AS builder

# Install Rust nightly (required for edition2024 in DSRs)
RUN rustup toolchain install nightly \
    && rustup default nightly

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    libpq-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy manifests
COPY Cargo.toml Cargo.lock ./
COPY crates/sage-core/Cargo.toml crates/sage-core/
COPY crates/sage-tools/Cargo.toml crates/sage-tools/

# Copy source
COPY crates/ crates/

# Build the actual binary
RUN cargo build --release

# Stage 2: Runtime
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

# Run sage
CMD ["/app/sage"]
