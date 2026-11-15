#!/bin/bash
cd "$(dirname "$0")/.."

PID_FILE=".dev.pid"
LOG_FILE=".dev.log"

if [ -f "$PID_FILE" ] && kill -0 $(cat "$PID_FILE") 2>/dev/null; then
  echo "Dev server already running (PID: $(cat $PID_FILE))"
  exit 1
fi

npm run dev > "$LOG_FILE" 2>&1 &
PID=$!
echo $PID > "$PID_FILE"

# Give the server a moment to start, then verify it actually survived
# (e.g. it dies instantly if port 4321 is already in use)
sleep 2
if ! kill -0 "$PID" 2>/dev/null; then
  echo "Dev server failed to start. Last log lines ($LOG_FILE):"
  tail -n 20 "$LOG_FILE"
  rm -f "$PID_FILE"
  exit 1
fi

echo "Dev server started (PID: $PID) - http://localhost:4321 (log: $LOG_FILE)"
