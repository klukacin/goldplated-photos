#!/bin/bash
# scripts/deploy.sh
# Deployment script for Astro Photo Gallery
# Syncs build artifacts, content, and admin interface to production.
#
# Usage:
#   npm run deploy              # Normal deploy (checksum for code only)
#   npm run deploy -- --checksum   # Force checksum for albums too (slower but thorough)
#
# Configuration:
#   - Copy .env.example to .env and fill in your deployment settings
#   - Supports SSH key (recommended) or password authentication

# --- Parse Arguments ---
FORCE_CHECKSUM=""
for arg in "$@"; do
    case $arg in
        --checksum)
            FORCE_CHECKSUM="--checksum"
            shift
            ;;
    esac
done

# --- Load Environment Variables ---
# Load from .env file if it exists (from project root)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
ENV_FILE="$PROJECT_ROOT/.env"

if [[ -f "$ENV_FILE" ]]; then
    # Export variables from .env, skipping comments and empty lines
    set -a
    source <(grep -v '^#' "$ENV_FILE" | grep -v '^$' | sed 's/^/export /')
    set +a
    echo -e "${DIM:-}Loaded configuration from .env${NC:-}"
fi

# --- Configuration (from env or defaults) ---
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

# SSH multiplexing - reuse single connection for all operations
CONTROL_PATH="/tmp/deploy-ssh-$$"
SSH_BASE_OPTS="-o ControlMaster=auto -o ControlPath=$CONTROL_PATH -o ControlPersist=60"

# Build SSH options based on authentication method
if [[ -n "$SSH_KEY" ]] && [[ -f "${SSH_KEY/#\~/$HOME}" ]]; then
    # Expand ~ to $HOME for the check
    SSH_KEY_EXPANDED="${SSH_KEY/#\~/$HOME}"
    SSH_OPTS="$SSH_BASE_OPTS -i $SSH_KEY_EXPANDED"
    AUTH_METHOD="SSH key ($SSH_KEY)"
elif [[ -n "$SSH_PASSWORD" ]]; then
    # Password authentication requires sshpass
    if command -v sshpass &> /dev/null; then
        SSH_CMD_PREFIX="sshpass -p '$SSH_PASSWORD'"
        SSH_OPTS="$SSH_BASE_OPTS"
        AUTH_METHOD="SSH password (via sshpass)"
    else
        echo -e "\033[1;33mWarning: sshpass not installed. Install with 'brew install sshpass' or use SSH key instead.\033[0m"
        echo -e "\033[1;33mFalling back to interactive password prompts.\033[0m"
        SSH_OPTS="$SSH_BASE_OPTS"
        AUTH_METHOD="SSH password (interactive)"
    fi
else
    SSH_OPTS="$SSH_BASE_OPTS"
    AUTH_METHOD="default SSH agent"
fi

# Colors and ANSI codes
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
DIM='\033[2m'
BOLD='\033[1m'
NC='\033[0m' # No Color

# Cursor control
SAVE_CURSOR='\033[s'
RESTORE_CURSOR='\033[u'
CLEAR_LINE='\033[K'
HIDE_CURSOR='\033[?25l'
SHOW_CURSOR='\033[?25h'

# Temp directory for worker status
DEPLOY_TMP="/tmp/deploy-$$"
mkdir -p "$DEPLOY_TMP"

# Cleanup on exit
cleanup() {
    printf "${SHOW_CURSOR}"
    rm -rf "$DEPLOY_TMP"
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
if [[ -n "$CHOWN" ]]; then
    echo -e "${DIM}  Chown: ${CHOWN}${NC}"
fi
if [[ -n "$FORCE_CHECKSUM" ]]; then
    echo -e "${YELLOW}  Mode: Checksum sync for albums (slower but thorough)${NC}"
fi
echo ""

# 1. Sanitize Folder Names (lowercase)
echo -e "${YELLOW}[1/7] Sanitizing folder names to lowercase...${NC}"
node scripts/sanitize-folders.mjs
if [ $? -ne 0 ]; then
    echo "Folder sanitization failed. Aborting."
    exit 1
fi

# 2. Build Project
echo -e "${YELLOW}[2/7] Building project...${NC}"
npm run build
if [ $? -ne 0 ]; then
    echo "Build failed. Aborting."
    exit 1
fi

# 3. Fix Paths in Entry File
echo -e "${YELLOW}[3/7] Fixing server paths for production...${NC}"
node scripts/fix-server-paths.mjs
if [ $? -ne 0 ]; then
    echo "Path fix failed. Aborting."
    exit 1
fi

# 4. Create Remote Directory Structure
echo -e "${YELLOW}[4/7] Preparing remote directories...${NC}"
ssh $SSH_OPTS -p $SSH_PORT ${REMOTE_USER}@${REMOTE_HOST} "mkdir -p ${REMOTE_ROOT}/client ${REMOTE_ROOT}/server ${REMOTE_ROOT}/src/content/albums ${REMOTE_ROOT}/public"

# 5. Sync Files with Live Progress Display

# Format bytes to human readable (e.g., 1500000 -> 1.5MB)
format_bytes() {
    local bytes=$1
    if [[ -z "$bytes" ]] || [[ "$bytes" == "0" ]]; then
        echo ""
        return
    fi
    if [[ $bytes -ge 1073741824 ]]; then
        printf "%.1fGB" $(echo "scale=1; $bytes/1073741824" | bc)
    elif [[ $bytes -ge 1048576 ]]; then
        printf "%.1fMB" $(echo "scale=1; $bytes/1048576" | bc)
    elif [[ $bytes -ge 1024 ]]; then
        printf "%.0fKB" $(echo "scale=0; $bytes/1024" | bc)
    else
        printf "%dB" "$bytes"
    fi
}

# Sync with status tracking - captures filename and speed
sync_with_status() {
    local worker_id=$1
    local name=$2
    local src=$3
    local dest=$4
    local opts=$5

    local status_file="$DEPLOY_TMP/worker-$worker_id"
    local file_file="$DEPLOY_TMP/worker-$worker_id-file"
    local progress_file="$DEPLOY_TMP/worker-$worker_id-progress"

    echo "running|$name|||" > "$status_file"
    echo "" > "$file_file"
    echo "||" > "$progress_file"

    # Run rsync with --progress to get speed info
    # Use process substitution to avoid subshell issues with "done" status
    # Permissions: configurable via DEPLOY_CHMOD_* env vars
    local rsync_exit=0
    rsync -av --progress --partial --chmod=D${CHMOD_DIRS},F${CHMOD_FILES} $opts \
        -e "ssh $SSH_OPTS -p $SSH_PORT" \
        "$src" "${REMOTE_USER}@${REMOTE_HOST}:${dest}" > >(
            while IFS= read -r line; do
                # Capture filename (line starting with non-space, not system message)
                if [[ "$line" =~ ^[^[:space:]] ]] && [[ ! "$line" =~ (sending|sent|total|building|deleting|\./$|^$) ]]; then
                    echo "$line" > "$file_file"
                # Capture progress: bytes, percentage, speed (e.g., "  1234567 45%  12.34MB/s  0:00:01")
                elif [[ "$line" =~ ^[[:space:]]*([0-9,]+)[[:space:]]+([0-9]+)%[[:space:]]+([0-9.]+[KMG]?B/s) ]]; then
                    local bytes="${BASH_REMATCH[1]//,/}"  # Remove commas
                    local pct="${BASH_REMATCH[2]}"
                    local speed="${BASH_REMATCH[3]}"
                    echo "$bytes|$pct|$speed" > "$progress_file"
                fi
            done
        ) 2>&1
    rsync_exit=$?

    # Check rsync exit code and report status
    if [[ $rsync_exit -eq 0 ]]; then
        echo "done|$name|||" > "$status_file"
    else
        echo "failed|$name|exit=$rsync_exit||" > "$status_file"
        # Write error to a log file for later review
        echo "$(date '+%H:%M:%S') FAILED: $name (src=$src, exit=$rsync_exit)" >> "$DEPLOY_TMP/errors.log"
    fi
}

# Simple sync without live progress (for sequential syncs)
sync_simple() {
    local src=$1
    local dest=$2
    local opts=$3
    echo -e "  ${DIM}→ Syncing $src...${NC}"
    if ! rsync -av --partial --chmod=D${CHMOD_DIRS},F${CHMOD_FILES} $opts -e "ssh $SSH_OPTS -p $SSH_PORT" "$src" "${REMOTE_USER}@${REMOTE_HOST}:${dest}" > /dev/null 2>&1; then
        echo "$(date '+%H:%M:%S') FAILED: $src" >> "$DEPLOY_TMP/errors.log"
        return 1
    fi
}

echo -e "${YELLOW}[5/7] Syncing files (parallel)...${NC}"
echo ""

# Collect all sync tasks
declare -a SYNC_TASKS
declare -a SYNC_NAMES
declare -a COLLECTION_FOLDERS  # Collection folders to sync sequentially (root files only)

# Add static directories
SYNC_TASKS+=("dist/client/|${REMOTE_ROOT}/client/|--delete")
SYNC_NAMES+=("client")
SYNC_TASKS+=("public/|${REMOTE_ROOT}/public/|--delete")
SYNC_NAMES+=("public")

# Add album directories - recursively expand collections at any depth
add_album_tasks() {
    local base_path=$1
    local remote_base=$2
    local display_base=$3

    for dir in "$base_path"/*/; do
        [[ ! -d "$dir" ]] && continue
        local name=$(basename "$dir")

        # Check if this folder has subfolders (is a collection)
        shopt -s nullglob
        local subdirs=("$dir"*/)
        shopt -u nullglob

        if [[ ${#subdirs[@]} -gt 0 ]]; then
            # This is a collection folder (has subfolders)
            # Recurse into subfolders first - they'll be synced in parallel
            add_album_tasks "$dir" "$remote_base/$name" "$display_base/$name"
            # Track collection folder for sequential sync later (root files only)
            COLLECTION_FOLDERS+=("$dir|${REMOTE_ROOT}/$remote_base/$name/")
        else
            # Leaf folder (actual album) - sync with --delete
            SYNC_TASKS+=("$dir|${REMOTE_ROOT}/$remote_base/$name/|--delete $FORCE_CHECKSUM")
            SYNC_NAMES+=("${display_base#/}/$name")
        fi
    done
}

# Start recursive expansion from albums root
add_album_tasks "src/content/albums" "src/content/albums" ""

TOTAL_TASKS=${#SYNC_TASKS[@]}
MAX_WORKERS=4
COMPLETED=0

# Initialize worker status
for ((i=0; i<MAX_WORKERS; i++)); do
    echo "idle|||0||" > "$DEPLOY_TMP/worker-$i"
done

# Task queue (index into SYNC_TASKS)
NEXT_TASK=0

# Worker PIDs
declare -a WORKER_PIDS
for ((i=0; i<MAX_WORKERS; i++)); do
    WORKER_PIDS[$i]=0
done

# Start monitor in background - redraws status every 0.3s
printf "${HIDE_CURSOR}"

# Reserve lines for display (MAX_WORKERS * 2 lines + 1 for queue)
for ((i=0; i<MAX_WORKERS*2+2; i++)); do
    echo ""
done

# Move cursor back up to start of display area
printf "\033[${MAX_WORKERS}A\033[$((MAX_WORKERS+2))A"

# Save position for redrawing
DISPLAY_START_LINE=$(tput lines)

# Function to draw current status
draw_status() {
    # Move to display area (go up from current position)
    printf "\033[$((MAX_WORKERS*2+2))A"

    for ((i=0; i<MAX_WORKERS; i++)); do
        local status_file="$DEPLOY_TMP/worker-$i"
        local file_file="$DEPLOY_TMP/worker-$i-file"
        local progress_file="$DEPLOY_TMP/worker-$i-progress"

        if [[ -f "$status_file" ]]; then
            IFS='|' read -r state name _ _ _ < "$status_file"
            local current_file=""
            local bytes="" pct="" speed=""
            [[ -f "$file_file" ]] && current_file=$(cat "$file_file" 2>/dev/null)
            if [[ -f "$progress_file" ]]; then
                IFS='|' read -r bytes pct speed < "$progress_file"
            fi

            # Line 1: Name and status
            printf "\r${CLEAR_LINE}"
            if [[ "$state" == "done" ]]; then
                printf "  ${GREEN}✓ %-20s done${NC}" "$name"
            elif [[ "$state" == "failed" ]]; then
                printf "  \033[1;31m✗ %-20s FAILED${NC}" "$name"
            elif [[ "$state" == "running" ]]; then
                printf "  ${CYAN}● %-20s syncing...${NC}" "$name"
            else
                printf "  ${DIM}○ %-20s waiting${NC}" "slot-$i"
            fi
            echo ""

            # Line 2: Current file with size, progress, and speed
            printf "\r${CLEAR_LINE}"
            if [[ -n "$current_file" ]] && [[ "$state" == "running" ]]; then
                # Truncate long filenames
                if [[ ${#current_file} -gt 35 ]]; then
                    current_file="...${current_file: -32}"
                fi
                # Build progress info string
                local progress_info=""
                if [[ -n "$bytes" ]] && [[ "$bytes" != "0" ]]; then
                    local formatted_bytes=$(format_bytes "$bytes")
                    progress_info="${formatted_bytes}"
                fi
                if [[ -n "$pct" ]]; then
                    progress_info="${progress_info} ${pct}%"
                fi
                if [[ -n "$speed" ]]; then
                    progress_info="${progress_info} ${speed}"
                fi
                printf "  ${DIM}  → %-35s %s${NC}" "$current_file" "$progress_info"
            fi
            echo ""
        else
            echo ""
            echo ""
        fi
    done

    # Queue status line
    printf "\r${CLEAR_LINE}"
    local remaining=$((TOTAL_TASKS - NEXT_TASK))
    if [[ $remaining -gt 0 ]]; then
        # Show next few in queue
        local queue_preview=""
        for ((q=NEXT_TASK; q<NEXT_TASK+3 && q<TOTAL_TASKS; q++)); do
            [[ -n "$queue_preview" ]] && queue_preview+=", "
            queue_preview+="${SYNC_NAMES[$q]}"
        done
        [[ $remaining -gt 3 ]] && queue_preview+="..."
        printf "  ${DIM}[queue]      %d waiting: %s${NC}" "$remaining" "$queue_preview"
    else
        printf "  ${DIM}[queue]      all tasks started${NC}"
    fi
    echo ""
    echo ""
}

# Main worker loop
while [[ $COMPLETED -lt $TOTAL_TASKS ]]; do
    # Check for finished workers and start new tasks
    for ((i=0; i<MAX_WORKERS; i++)); do
        pid=${WORKER_PIDS[$i]}

        # If worker finished (or never started)
        if [[ $pid -eq 0 ]] || ! kill -0 $pid 2>/dev/null; then
            # Count completion if it was running
            if [[ $pid -ne 0 ]]; then
                ((COMPLETED++))
            fi

            # Start next task if available
            if [[ $NEXT_TASK -lt $TOTAL_TASKS ]]; then
                IFS='|' read -r src dest opts <<< "${SYNC_TASKS[$NEXT_TASK]}"
                name="${SYNC_NAMES[$NEXT_TASK]}"

                sync_with_status $i "$name" "$src" "$dest" "$opts" &
                WORKER_PIDS[$i]=$!
                ((NEXT_TASK++))
            else
                WORKER_PIDS[$i]=0
                echo "idle|||0||" > "$DEPLOY_TMP/worker-$i"
            fi
        fi
    done

    draw_status
    sleep 0.3
done

# Final status draw
draw_status

printf "${SHOW_CURSOR}"
echo ""

# Check for failed tasks
FAILED_COUNT=0
if [[ -f "$DEPLOY_TMP/errors.log" ]]; then
    FAILED_COUNT=$(wc -l < "$DEPLOY_TMP/errors.log" | tr -d ' ')
fi

if [[ $FAILED_COUNT -gt 0 ]]; then
    echo -e "  \033[1;31m✗ $FAILED_COUNT sync tasks FAILED${NC}"
    echo -e "  ${DIM}Failed tasks:${NC}"
    cat "$DEPLOY_TMP/errors.log" | while read line; do
        echo -e "    \033[1;31m$line${NC}"
    done
    echo ""
    echo -e "  ${YELLOW}Warning: Some files may not have synced. Run deploy again or fix manually.${NC}"
else
    echo -e "  ${GREEN}✓ All $TOTAL_TASKS sync tasks completed${NC}"
fi
echo ""

# PHASE 2: Sync collection folder root files (index.md, body.md, .htaccess)
# Simple approach: just sync root files, no --delete (can only ADD, never break)
if [[ ${#COLLECTION_FOLDERS[@]} -gt 0 ]]; then
    echo -e "  ${DIM}Syncing ${#COLLECTION_FOLDERS[@]} collection roots...${NC}"
    PHASE2_ERRORS=0
    for entry in "${COLLECTION_FOLDERS[@]}"; do
        IFS='|' read -r src dest <<< "$entry"
        # --exclude='*/' skips subdirectories, syncs only root files
        # NO --delete here - orphans cleaned in Phase 3 verification
        if ! rsync -av --exclude='*/' \
            $SSH_OPTS -e "ssh -p $SSH_PORT" \
            "$src" "${REMOTE_USER}@${REMOTE_HOST}:$dest" > /dev/null 2>&1; then
            ((PHASE2_ERRORS++))
            echo "$(date '+%H:%M:%S') FAILED: collection root $src" >> "$DEPLOY_TMP/errors.log"
        fi
    done
    if [[ $PHASE2_ERRORS -gt 0 ]]; then
        echo -e "  ${RED}✗ $PHASE2_ERRORS collection syncs failed${NC}"
    fi
fi

# Sequential syncs for small files (no fancy display needed)
# Use --checksum for code files to ensure content changes are detected
# (timestamps can be unreliable after builds)
echo -e "  ${DIM}Syncing small files...${NC}"
sync_simple "dist/server/" "${REMOTE_ROOT}/server/" "--delete --checksum"
sync_simple "dist/client/" "${REMOTE_ROOT}/" "--checksum"
sync_simple "package.json" "${REMOTE_ROOT}/" "--checksum"
sync_simple "ecosystem.config.cjs" "${REMOTE_ROOT}/" "--checksum"
sync_simple "scripts/" "${REMOTE_ROOT}/scripts/" "--checksum"

# PHASE 3: Final verification pass
# Single rsync of entire albums tree with --delete to catch:
# - Any files missed by parallel sync
# - Orphaned files in collection folders
# - Any corruption or inconsistencies
# This is fast because most files are already synced (timestamp match)
echo -e "  ${DIM}Running verification pass...${NC}"
if rsync -av --delete \
    $SSH_OPTS -e "ssh -p $SSH_PORT" \
    "src/content/albums/" \
    "${REMOTE_USER}@${REMOTE_HOST}:${REMOTE_ROOT}/src/content/albums/" \
    > /dev/null 2>&1; then
    echo -e "  ${GREEN}✓ Verification complete${NC}"
else
    echo -e "  ${RED}✗ Verification FAILED - server may be inconsistent${NC}"
    echo "$(date '+%H:%M:%S') CRITICAL: Verification rsync failed" >> "$DEPLOY_TMP/errors.log"
fi

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
            # Use temp name to handle case-only renames
            temp="\$parent/__temp_\$\$_\$lower"
            target="\$parent/\$lower"
            if [ ! -e "\$target" ] || [ "\$dir" = "\$target" ]; then
                mv "\$dir" "\$temp" 2>/dev/null && mv "\$temp" "\$target" 2>/dev/null && \
                echo "    Renamed: \$name -> \$lower"
            fi
        fi
    done

    # Symlink assets so they are served correctly
    rm -rf client/home
    ln -s ../public/home client/home
    
    rm -rf client/images
    ln -s ../public/images client/images

    # 2. Symlink 'albums' for direct Apache serving (Performance)
    # This bypasses Node.js for image serving, relying on .htaccess to protect index.md
    if [ ! -L "albums" ]; then
        ln -s src/content/albums albums
    fi

    # 3. Set sensitive files to private permissions (not world-readable)
    # index.md files contain passwords, body.md may have private content
    echo "  -> Setting private file permissions (${CHMOD_PRIVATE})..."
    find src/content/albums -name "index.md" -exec chmod ${CHMOD_PRIVATE} {} \;
    find src/content/albums -name "body.md" -exec chmod ${CHMOD_PRIVATE} {} \;
    find . -name ".htaccess" -exec chmod ${CHMOD_PRIVATE} {} \;

    # 4. Optional: Set ownership if DEPLOY_CHOWN is configured
    if [[ -n "${CHOWN}" ]]; then
        echo "  -> Setting ownership to ${CHOWN}..."
        chown -R ${CHOWN} src/content/albums public client server 2>/dev/null || echo "    (chown skipped - may require sudo)"
    fi

    # Install Dependencies
    echo "  -> Installing dependencies..."
    npm install --production --no-audit --no-fund
EOF

# 7. Restart Server
echo -e "${YELLOW}[7/7] Restarting gallery server...${NC}"
ssh $SSH_OPTS -p $SSH_PORT ${REMOTE_USER}@${REMOTE_HOST} "cd ${REMOTE_ROOT} && pm2 restart ecosystem.config.cjs"

# Calculate elapsed time
ELAPSED=$SECONDS
MINS=$((ELAPSED / 60))
SECS=$((ELAPSED % 60))

echo ""
echo -e "${GREEN}═══════════════════════════════════════════${NC}"
echo -e "${GREEN}  Deployment complete! Gallery restarted.${NC}"
echo -e "${CYAN}  Total time: ${MINS}m ${SECS}s${NC}"
echo -e "${GREEN}═══════════════════════════════════════════${NC}"
