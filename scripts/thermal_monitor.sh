#!/usr/bin/env bash
set -u

interval=${1:-60}
output=${2:-a220091-thermal.csv}
printf 'timestamp_utc,cpu_temperature_c,cpu_frequency_khz,load_1m,mem_available_bytes\n' > "$output"
while true; do
    timestamp=$(date -u +%Y-%m-%dT%H:%M:%SZ)
    temperature=$(for zone in /sys/class/thermal/thermal_zone*/temp; do [ -r "$zone" ] && awk '{printf "%.1f", $1/1000; exit}' "$zone"; done)
    frequency=$(awk '/^cpu MHz/ {printf "%.0f", $4 * 1000; exit}' /proc/cpuinfo 2>/dev/null)
    load=$(awk '{print $1}' /proc/loadavg 2>/dev/null)
    memory=$(awk '/MemAvailable/ {print $2 * 1024; exit}' /proc/meminfo 2>/dev/null)
    printf '%s,%s,%s,%s,%s\n' "$timestamp" "${temperature:-unavailable}" "${frequency:-unavailable}" "${load:-unavailable}" "${memory:-unavailable}" >> "$output"
    sleep "$interval"
done
