#!/usr/bin/env bash
# Install git hooks from scripts/hooks/ into .git/hooks/
# Usage: ./scripts/install-hooks.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
HOOKS_DIR="$PROJECT_ROOT/.git/hooks"
SOURCE_DIR="$SCRIPT_DIR/hooks"

if [ ! -d "$SOURCE_DIR" ]; then
    echo "❌ No hooks directory found at $SOURCE_DIR"
    exit 1
fi

echo "🔧 Installing git hooks from $SOURCE_DIR ..."

for hook in "$SOURCE_DIR"/*; do
    hook_name="$(basename "$hook")"
    cp "$hook" "$HOOKS_DIR/$hook_name"
    chmod +x "$HOOKS_DIR/$hook_name"
    echo "  ✅ Installed $hook_name"
done

echo ""
echo "🎉 Hooks installed. They will run automatically on the next commit."
