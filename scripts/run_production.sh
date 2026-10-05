#!/usr/bin/env bash
set -u

log_dir=${A220091_LOG_DIR:-logs}
mkdir -p "$log_dir"
log_file=${A220091_LOG_FILE:-"$log_dir/a220091-$(date -u +%Y%m%dT%H%M%SZ).log"}
export A220091_LOG_FILE="$log_file"
{
    echo "startup timestamp_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    echo "git_commit=$(git rev-parse HEAD 2>/dev/null || echo unavailable)"
    echo "command=$0 $*"
    "$@"
    status=$?
    echo "completion timestamp_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ) exit_code=$status"
    exit "$status"
} 2>&1 | tee -a "$log_file"
