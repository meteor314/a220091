#!/usr/bin/env bash
set -u

echo "timestamp_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
echo "cpu_model=$(lscpu 2>/dev/null | awk -F: '/Model name/ {gsub(/^ +/, "", $2); print $2; exit}')"
echo "logical_cpus=$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo unavailable)"
echo "cpu_cores=$(lscpu 2>/dev/null | awk -F: '/Core\(s\) per socket/ {gsub(/^ +/, "", $2); print $2; exit}')"
echo "sockets=$(lscpu 2>/dev/null | awk -F: '/Socket\(s\)/ {gsub(/^ +/, "", $2); print $2; exit}')"
echo "ram_bytes=$(awk '/MemTotal/ {print $2 * 1024; exit}' /proc/meminfo 2>/dev/null || echo unavailable)"
echo "kernel=$(uname -srmo 2>/dev/null || echo unavailable)"
if [ -r /etc/os-release ]; then . /etc/os-release; echo "os=${PRETTY_NAME:-unknown}"; else echo "os=unavailable"; fi
echo "rustc=$(rustc --version 2>/dev/null || echo unavailable)"
echo "docker=$(docker --version 2>/dev/null || echo unavailable)"
echo "git_commit=$(git rev-parse HEAD 2>/dev/null || echo unavailable)"
echo "temperature_c=$(for zone in /sys/class/thermal/thermal_zone*/temp; do [ -r "$zone" ] && awk '{printf "%.1f", $1/1000; exit}' "$zone"; done)"
echo "frequency_khz=$(awk '/^cpu MHz/ {printf "%.0f", $4 * 1000; exit}' /proc/cpuinfo 2>/dev/null || echo unavailable)"
