#!/bin/bash
cd "$(dirname "$0")/.."

PID_FILE=".admin.pid"
LOG_FILE=".admin.log"

if [ -f "$PID_FILE" ] && kill -0 $(cat "$PID_FILE") 2>/dev/null; then
  echo "Admin server already running (PID: $(cat $PID_FILE))"
  exit 1
fi

npm run admin > "$LOG_FILE" 2>&1 &
PID=$!
echo $PID > "$PID_FILE"

# Give the server a moment to start, then verify it actually survived
# (e.g. it dies instantly if port 4444 is already in use)
sleep 2
if ! kill -0 "$PID" 2>/dev/null; then
  echo "Admin server failed to start. Last log lines ($LOG_FILE):"
  tail -n 20 "$LOG_FILE"
  rm -f "$PID_FILE"
  exit 1
fi

echo "Admin server started (PID: $PID) - http://localhost:4444 (log: $LOG_FILE)"
