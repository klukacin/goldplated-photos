#!/bin/bash
cd "$(dirname "$0")/.."

PID_FILE=".admin.pid"

if [ -f "$PID_FILE" ] && kill -0 $(cat "$PID_FILE") 2>/dev/null; then
  echo "Admin server already running (PID: $(cat $PID_FILE))"
  exit 1
fi

npm run admin > /dev/null 2>&1 &
echo $! > "$PID_FILE"
echo "Admin server started (PID: $!) - http://localhost:4444"
