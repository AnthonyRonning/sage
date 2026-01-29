{
  description = "Sage - Personal AI Agent";

  inputs = {
    flake-utils.url = "github:numtide/flake-utils";
    nixpkgs.url = "nixpkgs/nixos-unstable";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };

        # Standalone signal-cli binary for aarch64 (GraalVM native-image with embedded libsignal)
        signal-cli-standalone = pkgs.stdenv.mkDerivation {
          pname = "signal-cli";
          version = "0.13.22";
          
          src = pkgs.fetchurl {
            url = "https://media.projektzentrisch.de/temp/signal-cli/signal-cli_ubuntu2004_arm64.gz";
            sha256 = "sha256-K2jeHAQYCSuOS5A8uGE1pUel5Yu16Q3TSEtVOBKljqs=";
          };
          
          nativeBuildInputs = [ pkgs.gzip pkgs.autoPatchelfHook ];
          buildInputs = [ pkgs.stdenv.cc.cc.lib pkgs.zlib ];
          
          dontUnpack = true;
          
          installPhase = ''
            mkdir -p $out/bin
            gunzip -c $src > $out/bin/signal-cli
            chmod +x $out/bin/signal-cli
          '';
        };
        
        # Use standalone binary on aarch64-linux, nixpkgs signal-cli elsewhere
        signal-cli-pkg = if system == "aarch64-linux" 
          then signal-cli-standalone 
          else pkgs.signal-cli;

        commonInputs = [
          # Rust toolchain
          pkgs.rustc
          pkgs.cargo
          pkgs.rust-analyzer
          pkgs.clippy
          pkgs.rustfmt
          
          # Build dependencies for Rust crates
          pkgs.pkg-config
          pkgs.openssl
          pkgs.postgresql.lib  # libpq for diesel
          pkgs.diesel-cli      # Database migrations
          
          # System tools
          pkgs.jq
          pkgs.just
          
          # Services
          pkgs.postgresql
          pkgs.postgresql.pkgs.pgvector  # Vector similarity search extension
          pkgs.valkey
          signal-cli-pkg
        ];

        linuxOnlyInputs = [
          pkgs.podman
          pkgs.conmon
          pkgs.slirp4netns
          pkgs.fuse-overlayfs
        ];

        darwinOnlyInputs = [
          pkgs.libiconv
        ];

        inputs = commonInputs
          ++ pkgs.lib.optionals pkgs.stdenv.isLinux linuxOnlyInputs
          ++ pkgs.lib.optionals pkgs.stdenv.isDarwin darwinOnlyInputs;

        # Script to set up .env from example
        setupEnvScript = pkgs.writeShellScript "setup-env" ''
          if [ ! -f .env ]; then
            if [ -f .env.example ]; then
              cp .env.example .env
              echo "Created .env from .env.example"
            fi
          fi
        '';

        # Sage PostgreSQL port (5434 to avoid conflicts with 5432/5433)
        pgPort = "5434";
        pgDataDir = ".postgres-sage";

        # Script to start Sage's dedicated PostgreSQL instance
        startPgScript = pkgs.writeShellScript "start-sage-pg" ''
          PGDATA="$PWD/${pgDataDir}"
          PGPORT=${pgPort}
          
          # Check if already running
          if [ -f "$PGDATA/postmaster.pid" ]; then
            PID=$(head -1 "$PGDATA/postmaster.pid" 2>/dev/null)
            if kill -0 "$PID" 2>/dev/null; then
              echo "PostgreSQL already running on port $PGPORT (PID: $PID)"
              exit 0
            else
              rm -f "$PGDATA/postmaster.pid"
            fi
          fi
          
          # Initialize if needed
          if [ ! -d "$PGDATA" ]; then
            echo "Initializing Sage PostgreSQL database..."
            initdb -D "$PGDATA" --no-locale --encoding=UTF8
            
            # Configure for local development
            echo "host all all 127.0.0.1/32 trust" >> "$PGDATA/pg_hba.conf"
            echo "host all all ::1/128 trust" >> "$PGDATA/pg_hba.conf"
          fi
          
          echo "Starting PostgreSQL on port $PGPORT..."
          pg_ctl -D "$PGDATA" -l "$PGDATA/postgresql.log" -o "-p $PGPORT" start
          
          # Wait for startup
          for i in {1..30}; do
            if pg_isready -p $PGPORT -q; then
              break
            fi
            sleep 0.2
          done
          
          # Create sage database and enable pgvector if not exists
          if ! psql -p $PGPORT -lqt | cut -d \| -f 1 | grep -qw sage; then
            echo "Creating 'sage' database with pgvector..."
            createdb -p $PGPORT sage
            psql -p $PGPORT -d sage -c "CREATE EXTENSION IF NOT EXISTS vector;"
          fi
          
          echo "PostgreSQL ready on port $PGPORT"
        '';

        # Script to stop Sage's PostgreSQL
        stopPgScript = pkgs.writeShellScript "stop-sage-pg" ''
          PGDATA="$PWD/${pgDataDir}"
          if [ -f "$PGDATA/postmaster.pid" ]; then
            echo "Stopping Sage PostgreSQL..."
            pg_ctl -D "$PGDATA" stop -m fast
          else
            echo "PostgreSQL not running"
          fi
        '';
      in
      {
        devShells.default = pkgs.mkShell {
          packages = inputs;

          shellHook = ''
            ${pkgs.lib.optionalString pkgs.stdenv.isLinux ''
              # Increase file descriptor limits for PostgreSQL
              ulimit -n 65536 2>/dev/null || true
              
              alias docker='podman'
              export CONTAINERS_CONF=$HOME/.config/containers/containers.conf
              export CONTAINERS_POLICY=$HOME/.config/containers/policy.json
              mkdir -p $HOME/.config/containers
              echo '{"default":[{"type":"insecureAcceptAnything"}]}' > $CONTAINERS_POLICY
              if [ ! -f $CONTAINERS_CONF ]; then
                echo "[engine]
            cgroup_manager = \"cgroupfs\"
            events_logger = \"file\"
            runtime = \"crun\"" > $CONTAINERS_CONF
              fi
              chmod 600 $CONTAINERS_POLICY $CONTAINERS_CONF
            ''}

            ${setupEnvScript}
            
            # Export library paths for Rust crates
            export PKG_CONFIG_PATH="${pkgs.openssl.dev}/lib/pkgconfig:${pkgs.postgresql.lib}/lib/pkgconfig:$PKG_CONFIG_PATH"
            export OPENSSL_DIR="${pkgs.openssl.dev}"
            export OPENSSL_LIB_DIR="${pkgs.openssl.out}/lib"
            export OPENSSL_INCLUDE_DIR="${pkgs.openssl.dev}/include"

            # Sage PostgreSQL helpers
            alias sage-pg-start='${startPgScript}'
            alias sage-pg-stop='${stopPgScript}'
            alias sage-psql='PGPASSWORD=sage psql -h localhost -p ${pgPort} -U sage -d sage'
            
            # Set DATABASE_URL for Sage's dedicated PostgreSQL (containerized with pgvector)
            export DATABASE_URL="postgres://sage:sage@localhost:${pgPort}/sage"
            export SAGE_PG_PORT="${pgPort}"

            echo ""
            echo "🌿 Sage development environment"
            echo ""
            echo "Quick start:"
            echo "  sage-pg-start       - Start Sage PostgreSQL (port ${pgPort})"
            echo "  sage-pg-stop        - Stop Sage PostgreSQL"
            echo "  sage-psql           - Connect to Sage database"
            echo "  cargo build         - Build Rust agent"
            echo "  cargo run           - Run Rust agent"
            echo ""
            echo "Run 'just' to see all available commands."
            echo ""
          '';
        };
      }
    );
}
