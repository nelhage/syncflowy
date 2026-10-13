#!/bin/sh
# usage: step.sh <scenario> <prev-label> <new-label>
cd "$(dirname "$0")/../.." && export SYNCFLOWY_TAG=ui-$1
direnv exec . uv run python experiments/ui/ui.py snap "$1" "$3" 2>&1 | grep -v direnv
echo "--- diff $2 -> $3"
direnv exec . uv run python experiments/ui/ui.py diff "$1" "$2" "$3" 2>&1 | grep -v direnv
