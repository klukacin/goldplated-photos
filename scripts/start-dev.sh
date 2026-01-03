#!/bin/bash
cd "$(dirname "$0")/.."

PID_FILE=".dev.pid"

if [ -f "$PID_FILE" ] && kill -0 $(cat "$PID_FILE") 2>/dev/null; then
  echo "Dev server already running (PID: $(cat $PID_FILE))"
  exit 1
fi

npm run dev > /dev/null 2>&1 &
echo $! > "$PID_FILE"
echo "Dev server started (PID: $!) - http://localhost:4321"
