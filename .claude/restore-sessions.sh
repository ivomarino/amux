#!/bin/bash
# Restore amux sessions on service startup
# This script restores any lingering tmux sessions after amux-server restarts

set -e

# Wait for amux-server to initialize
sleep 2

# List any sessions that should be restored
# For now, this is a placeholder - the sessions are managed by amux-server itself
exit 0
