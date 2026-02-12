#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────
# core_agent_lines.sh — Count lines of code in OxiBot
# Usage: ./core_agent_lines.sh [--detail]
# ─────────────────────────────────────────────────────────────

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CRATES_DIR="$SCRIPT_DIR/crates"
BRIDGE_DIR="$SCRIPT_DIR/bridge"

BOLD="\033[1m"
DIM="\033[2m"
CYAN="\033[36m"
GREEN="\033[32m"
YELLOW="\033[33m"
RESET="\033[0m"

detail=false
[[ "${1:-}" == "--detail" ]] && detail=true

echo -e "${BOLD}🦀 OxiBot — Lines of Code${RESET}"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

total_rust=0

# Count Rust LOC per crate
for crate_dir in "$CRATES_DIR"/*/; do
    crate_name=$(basename "$crate_dir")
    if [[ -d "$crate_dir/src" ]]; then
        loc=$(find "$crate_dir/src" -name '*.rs' -exec cat {} + 2>/dev/null | wc -l)
        total_rust=$((total_rust + loc))
        printf "  ${CYAN}%-22s${RESET} %'6d lines\n" "$crate_name" "$loc"

        if $detail; then
            find "$crate_dir/src" -name '*.rs' | sort | while read -r f; do
                file_loc=$(wc -l < "$f")
                rel=$(echo "$f" | sed "s|$SCRIPT_DIR/||")
                printf "    ${DIM}%-40s %5d${RESET}\n" "$rel" "$file_loc"
            done
        fi
    fi
done

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
printf "  ${GREEN}%-22s${RESET} %'6d lines\n" "Total Rust" "$total_rust"

# Count TypeScript LOC (bridge)
total_ts=0
if [[ -d "$BRIDGE_DIR/src" ]]; then
    total_ts=$(find "$BRIDGE_DIR/src" -name '*.ts' -exec cat {} + 2>/dev/null | wc -l)
    printf "  ${YELLOW}%-22s${RESET} %'6d lines\n" "Total TypeScript" "$total_ts"

    if $detail; then
        find "$BRIDGE_DIR/src" -name '*.ts' | sort | while read -r f; do
            file_loc=$(wc -l < "$f")
            rel=$(echo "$f" | sed "s|$SCRIPT_DIR/||")
            printf "    ${DIM}%-40s %5d${RESET}\n" "$rel" "$file_loc"
        done
    fi
fi

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
grand_total=$((total_rust + total_ts))
printf "  ${BOLD}%-22s %'6d lines${RESET}\n" "GRAND TOTAL" "$grand_total"

# Count test lines
test_lines=$(find "$CRATES_DIR" -name '*.rs' -exec grep -c '#\[test\]\|#\[tokio::test\]' {} + 2>/dev/null | awk -F: '{s+=$NF} END {print s}')
echo ""
printf "  ${DIM}Test functions:        %6d${RESET}\n" "${test_lines:-0}"

# Count skills
skill_count=0
if [[ -d "$CRATES_DIR/oxibot-agent/skills" ]]; then
    skill_count=$(find "$CRATES_DIR/oxibot-agent/skills" -name '*.md' | wc -l)
fi
printf "  ${DIM}Bundled skills:        %6d${RESET}\n" "$skill_count"
echo ""
