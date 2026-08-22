#!/bin/bash
cd "$(dirname "$0")/.."

PID_FILE=".admin.pid"
PORT=4444

# Find process by port and kill it
PID_BY_PORT=$(lsof -i :$PORT -t 2>/dev/null)

if [ -n "$PID_BY_PORT" ]; then
  kill $PID_BY_PORT 2>/dev/null
  echo "Admin server stopped (port $PORT)"
else
  echo "Admin server not running"
fi

rm -f "$PID_FILE"
