#!/bin/bash
# scripts/deploy.sh
# Deployment script for Astro Photo Gallery
# Syncs build artifacts, content, and admin interface to production.
#
# Usage:
#   npm run deploy              # Full deployment
#   npm run deploy -- --checksum   # Force checksum for albums (slower but thorough)
#
# Configuration:
#   - Copy .env.example to .env and fill in your deployment settings
#   - Supports SSH key (recommended) or password authentication

# --- Parse Arguments ---
FORCE_CHECKSUM=""
PARALLEL_MODE=""

for arg in "$@"; do
    case $arg in
        --checksum)
            FORCE_CHECKSUM="--checksum"
            ;;
        --parallel)
            PARALLEL_MODE="true"
            ;;
    esac
done

# --- Load Environment Variables ---
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
ENV_FILE="$PROJECT_ROOT/.env"

if [[ -f "$ENV_FILE" ]]; then
    set -a
    source <(grep -v '^#' "$ENV_FILE" | grep -v '^$' | sed 's/^/export /')
    set +a
    echo -e "${DIM:-}Loaded configuration from .env${NC:-}"
fi

# --- Configuration ---
REMOTE_USER="${DEPLOY_REMOTE_USER:-klukacin}"
REMOTE_HOST="${DEPLOY_REMOTE_HOST:-kristijan.lukacin.com}"
REMOTE_ROOT="${DEPLOY_REMOTE_ROOT:-/home/klukacin/public_html}"
SSH_PORT="${DEPLOY_SSH_PORT:-22}"
SSH_KEY="${DEPLOY_SSH_KEY:-}"
SSH_PASSWORD="${DEPLOY_SSH_PASSWORD:-}"

# --- Permission Configuration ---
CHMOD_DIRS="${DEPLOY_CHMOD_DIRS:-775}"
CHMOD_FILES="${DEPLOY_CHMOD_FILES:-664}"
CHMOD_PRIVATE="${DEPLOY_CHMOD_PRIVATE:-660}"
CHOWN="${DEPLOY_CHOWN:-}"

# SSH multiplexing - reuse single connection
CONTROL_PATH="/tmp/deploy-ssh-$$"
SSH_BASE_OPTS="-o ControlMaster=auto -o ControlPath=$CONTROL_PATH -o ControlPersist=60"

# Build SSH options
if [[ -n "$SSH_KEY" ]] && [[ -f "${SSH_KEY/#\~/$HOME}" ]]; then
    SSH_KEY_EXPANDED="${SSH_KEY/#\~/$HOME}"
    SSH_OPTS="$SSH_BASE_OPTS -i $SSH_KEY_EXPANDED"
    AUTH_METHOD="SSH key ($SSH_KEY)"
elif [[ -n "$SSH_PASSWORD" ]]; then
    if command -v sshpass &> /dev/null; then
        SSH_CMD_PREFIX="sshpass -p '$SSH_PASSWORD'"
        SSH_OPTS="$SSH_BASE_OPTS"
        AUTH_METHOD="SSH password (via sshpass)"
    else
        echo -e "\033[1;33mWarning: sshpass not installed.\033[0m"
        SSH_OPTS="$SSH_BASE_OPTS"
        AUTH_METHOD="SSH password (interactive)"
    fi
else
    SSH_OPTS="$SSH_BASE_OPTS"
    AUTH_METHOD="default SSH agent"
fi

# Colors
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
RED='\033[0;31m'
DIM='\033[2m'
NC='\033[0m'

# Cleanup on exit
cleanup() {
    ssh -O exit -o ControlPath=$CONTROL_PATH ${REMOTE_USER}@${REMOTE_HOST} 2>/dev/null
}
trap cleanup EXIT

# Start timer
SECONDS=0

echo -e "${GREEN}Starting Deployment to ${REMOTE_HOST}...${NC}"
echo -e "${DIM}  User: ${REMOTE_USER}${NC}"
echo -e "${DIM}  Path: ${REMOTE_ROOT}${NC}"
echo -e "${DIM}  Auth: ${AUTH_METHOD}${NC}"
echo -e "${DIM}  Chmod: dirs=${CHMOD_DIRS}, files=${CHMOD_FILES}, private=${CHMOD_PRIVATE}${NC}"
[[ -n "$CHOWN" ]] && echo -e "${DIM}  Chown: ${CHOWN}${NC}"
[[ -n "$FORCE_CHECKSUM" ]] && echo -e "${YELLOW}  Mode: Checksum sync (slower but thorough)${NC}"
[[ -n "$PARALLEL_MODE" ]] && echo -e "${YELLOW}  Mode: Parallel sync (5 workers)${NC}"
echo ""

# 1. Sanitize Folder Names
echo -e "${YELLOW}[1/7] Sanitizing folder names to lowercase...${NC}"
node scripts/sanitize-folders.mjs || { echo "Folder sanitization failed."; exit 1; }

# 2. Build Project
echo -e "${YELLOW}[2/7] Building project...${NC}"
npm run build || { echo "Build failed."; exit 1; }

# 3. Fix Paths
echo -e "${YELLOW}[3/7] Fixing server paths for production...${NC}"
node scripts/fix-server-paths.mjs || { echo "Path fix failed."; exit 1; }

# 4. Create Remote Directories
echo -e "${YELLOW}[4/7] Preparing remote directories...${NC}"
ssh $SSH_OPTS -p $SSH_PORT ${REMOTE_USER}@${REMOTE_HOST} \
    "mkdir -p ${REMOTE_ROOT}/client ${REMOTE_ROOT}/server ${REMOTE_ROOT}/src/content/albums ${REMOTE_ROOT}/public"

# 5. Sync Files
echo -e "${YELLOW}[5/7] Syncing files...${NC}"

# Sync static directories
echo -e "  ${DIM}→ Syncing client assets...${NC}"
rsync -av --delete --chmod=D${CHMOD_DIRS},F${CHMOD_FILES} \
    -e "ssh $SSH_OPTS -p $SSH_PORT" \
    dist/client/ "${REMOTE_USER}@${REMOTE_HOST}:${REMOTE_ROOT}/client/"

echo -e "  ${DIM}→ Syncing public assets...${NC}"
rsync -av --delete --chmod=D${CHMOD_DIRS},F${CHMOD_FILES} \
    -e "ssh $SSH_OPTS -p $SSH_PORT" \
    public/ "${REMOTE_USER}@${REMOTE_HOST}:${REMOTE_ROOT}/public/"

# Sync albums
ALBUMS_LOCAL="src/content/albums"
ALBUMS_REMOTE="${REMOTE_ROOT}/src/content/albums"
MAX_CONCURRENT=5

if [[ -n "$PARALLEL_MODE" ]]; then
    echo -e "  ${DIM}→ Syncing albums (parallel mode, $MAX_CONCURRENT workers)...${NC}"

    # Sync a single album directory (called in background)
    sync_album() {
        local name="$1"
        local src="$ALBUMS_LOCAL/$name"
        local dest="${REMOTE_USER}@${REMOTE_HOST}:$ALBUMS_REMOTE/$name"

        if [[ -d "$src" ]]; then
            # Directory: sync contents with --delete
            rsync -av --delete --chmod=D${CHMOD_DIRS},F${CHMOD_FILES} $FORCE_CHECKSUM \
                -e "ssh $SSH_OPTS -p $SSH_PORT" \
                "$src/" "$dest/"
        elif [[ -f "$src" ]]; then
            # File: sync to remote directory (no --delete for single files)
            rsync -av --chmod=D${CHMOD_DIRS},F${CHMOD_FILES} $FORCE_CHECKSUM \
                -e "ssh $SSH_OPTS -p $SSH_PORT" \
                "$src" "${REMOTE_USER}@${REMOTE_HOST}:$ALBUMS_REMOTE/"
        fi
    }

    # Export for background subshells
    export -f sync_album
    export ALBUMS_LOCAL ALBUMS_REMOTE REMOTE_USER REMOTE_HOST SSH_OPTS SSH_PORT
    export CHMOD_DIRS CHMOD_FILES FORCE_CHECKSUM

    # Gather top-level items (files and directories)
    mapfile -t ITEMS < <(find "$ALBUMS_LOCAL" -mindepth 1 -maxdepth 1 -printf "%f\n")
    echo -e "  ${DIM}  Found ${#ITEMS[@]} top-level items to sync${NC}"

    # Sync in parallel with worker limit
    active=0
    for item in "${ITEMS[@]}"; do
        echo -e "  ${DIM}  Starting: $item${NC}"
        sync_album "$item" &
        ((active++))
        if (( active >= MAX_CONCURRENT )); then
            wait -n  # Wait for any one background job to finish (Bash 5+)
            ((active--))
        fi
    done
    wait

    # Final cleanup: remove stale remote items
    echo -e "  ${DIM}→ Cleaning up stale remote items...${NC}"
    REMOTE_ITEMS=$(ssh $SSH_OPTS -p $SSH_PORT "${REMOTE_USER}@${REMOTE_HOST}" \
        "find '$ALBUMS_REMOTE' -mindepth 1 -maxdepth 1 -printf '%f\n'" 2>/dev/null)

    while IFS= read -r rem_item; do
        [[ -z "$rem_item" ]] && continue
        if [ ! -e "$ALBUMS_LOCAL/$rem_item" ]; then
            echo -e "    ${DIM}Removing stale: $rem_item${NC}"
            ssh $SSH_OPTS -p $SSH_PORT "${REMOTE_USER}@${REMOTE_HOST}" \
                "rm -rf '$ALBUMS_REMOTE/$rem_item'"
        fi
    done <<< "$REMOTE_ITEMS"
else
    # Sequential mode (default)
    echo -e "  ${DIM}→ Syncing albums...${NC}"
    rsync -av --progress --delete --chmod=D${CHMOD_DIRS},F${CHMOD_FILES} $FORCE_CHECKSUM \
        -e "ssh $SSH_OPTS -p $SSH_PORT" \
        src/content/albums/ "${REMOTE_USER}@${REMOTE_HOST}:${REMOTE_ROOT}/src/content/albums/"
fi

# Sync server and config files
echo -e "  ${DIM}→ Syncing server files...${NC}"
rsync -av --delete --checksum --chmod=D${CHMOD_DIRS},F${CHMOD_FILES} \
    -e "ssh $SSH_OPTS -p $SSH_PORT" \
    dist/server/ "${REMOTE_USER}@${REMOTE_HOST}:${REMOTE_ROOT}/server/"

rsync -av --checksum --chmod=D${CHMOD_DIRS},F${CHMOD_FILES} \
    -e "ssh $SSH_OPTS -p $SSH_PORT" \
    package.json ecosystem.config.cjs "${REMOTE_USER}@${REMOTE_HOST}:${REMOTE_ROOT}/"

rsync -av --checksum --chmod=D${CHMOD_DIRS},F${CHMOD_FILES} \
    -e "ssh $SSH_OPTS -p $SSH_PORT" \
    scripts/ "${REMOTE_USER}@${REMOTE_HOST}:${REMOTE_ROOT}/scripts/"

echo -e "  ${GREEN}✓ Sync complete${NC}"

# 6. Post-Deployment Setup
echo -e "${YELLOW}[6/7] Configuring remote environment...${NC}"
ssh $SSH_OPTS -p $SSH_PORT ${REMOTE_USER}@${REMOTE_HOST} "bash -s" <<EOF
    cd ${REMOTE_ROOT}

    # Sanitize remote folder names to lowercase
    echo "  -> Sanitizing album folder names..."
    find src/content/albums -depth -type d -name '*[A-Z]*' 2>/dev/null | while read dir; do
        parent=\$(dirname "\$dir")
        name=\$(basename "\$dir")
        lower=\$(echo "\$name" | tr '[:upper:]' '[:lower:]')
        if [ "\$name" != "\$lower" ]; then
            temp="\$parent/__temp_\$\$_\$lower"
            target="\$parent/\$lower"
            if [ ! -e "\$target" ] || [ "\$dir" = "\$target" ]; then
                mv "\$dir" "\$temp" 2>/dev/null && mv "\$temp" "\$target" 2>/dev/null && \
                echo "    Renamed: \$name -> \$lower"
            fi
        fi
    done

    # Symlink assets
    rm -rf client/home && ln -s ../public/home client/home
    rm -rf client/images && ln -s ../public/images client/images

    # Symlink albums for direct Apache serving
    [ ! -L "albums" ] && ln -s src/content/albums albums

    # Set private file permissions
    echo "  -> Setting private file permissions (${CHMOD_PRIVATE})..."
    find src/content/albums -name "index.md" -exec chmod ${CHMOD_PRIVATE} {} \;
    find src/content/albums -name "body.md" -exec chmod ${CHMOD_PRIVATE} {} \;
    find . -name ".htaccess" -exec chmod ${CHMOD_PRIVATE} {} \;

    # Optional ownership
    if [[ -n "${CHOWN}" ]]; then
        echo "  -> Setting ownership to ${CHOWN}..."
        chown -R ${CHOWN} src/content/albums public client server 2>/dev/null || true
    fi

    # Install dependencies
    echo "  -> Installing dependencies..."
    npm install --production --no-audit --no-fund
EOF

# 7. Restart Server
echo -e "${YELLOW}[7/7] Restarting gallery server...${NC}"
ssh $SSH_OPTS -p $SSH_PORT ${REMOTE_USER}@${REMOTE_HOST} "cd ${REMOTE_ROOT} && pm2 restart ecosystem.config.cjs"

# Done
ELAPSED=$SECONDS
MINS=$((ELAPSED / 60))
SECS=$((ELAPSED % 60))

echo ""
echo -e "${GREEN}═══════════════════════════════════════════${NC}"
echo -e "${GREEN}  Deployment complete! Gallery restarted.${NC}"
echo -e "${CYAN}  Total time: ${MINS}m ${SECS}s${NC}"
echo -e "${GREEN}═══════════════════════════════════════════${NC}"
