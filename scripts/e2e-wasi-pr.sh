#!/usr/bin/env bash
# E2E: WASI agent-worker → blobstore workspace → git diff → PR
#
# Prerequisites:
#   - wash dev running (agent-worker on :8000)
#   - Ollama running on csatapaci
#   - gh CLI authenticated
#   - Git repo clean
#
# Usage: ./scripts/e2e-wasi-pr.sh

set -euo pipefail

REPO_DIR="$(cd "$(dirname "$0")/.." && pwd)"
OLLAMA_URL="http://100.81.10.8:11434"
MODEL="qwen2.5-coder:14b"
COMPONENT_URL="http://localhost:8000"
BRANCH="agent/wasi-e2e-$(date +%s)"
TARGET_FILE="crates/orchestrator/src/runner.rs"

echo "=== WASI E2E: Agent → Blobstore → Diff → PR ==="
echo "Repo: $REPO_DIR"
echo "Target: $TARGET_FILE"
echo "Model: $MODEL"
echo ""

# 1. Read the target file
echo "1. Reading target file..."
FILE_CONTENT=$(cat "$REPO_DIR/$TARGET_FILE")
echo "   $(echo "$FILE_CONTENT" | wc -l | tr -d ' ') lines"

# 2. Send task to WASI agent-worker with workspace
echo "2. Sending task to WASI agent-worker..."
ESCAPED_CONTENT=$(python3 -c "import json,sys; print(json.dumps(sys.stdin.read()))" <<< "$FILE_CONTENT")

RESPONSE=$(curl -s -X POST "$COMPONENT_URL/" \
  -H 'Content-Type: application/json' \
  -d "{
    \"task\": \"Add a doc comment to the discover_source_files function in $TARGET_FILE explaining what file extensions it scans for (rs, ts, js, go, py, md, toml, json, yaml, yml) and what directories it skips (dotfiles, target, node_modules)\",
    \"model\": \"$MODEL\",
    \"ollama_url\": \"$OLLAMA_URL\",
    \"workspace_id\": \"pr-workspace-$(date +%s)\",
    \"files\": [{\"path\": \"$TARGET_FILE\", \"content\": $ESCAPED_CONTENT}]
  }")

echo "   Response received"

# 3. Check result
STATUS=$(echo "$RESPONSE" | python3 -c "import json,sys; print(json.load(sys.stdin).get('status','error'))")
EDITS=$(echo "$RESPONSE" | python3 -c "import json,sys; print(json.load(sys.stdin).get('edits',0))")
DIFF=$(echo "$RESPONSE" | python3 -c "import json,sys; print(json.load(sys.stdin).get('diff',''))")
RAW=$(echo "$RESPONSE" | python3 -c "import json,sys; print(json.load(sys.stdin).get('raw_response',''))")

echo "   Status: $STATUS"
echo "   Edits: $EDITS"

if [ "$STATUS" != "ok" ] || [ "$EDITS" = "0" ]; then
  echo "ERROR: Agent failed to produce edits"
  echo "Response: $RESPONSE"
  exit 1
fi

echo ""
echo "3. Diff from virt-git (in-memory, via blobstore):"
echo "---"
echo "$DIFF"
echo "---"
echo ""

# 4. Apply the raw LLM edit to the actual file
echo "4. Applying edit to git repo..."
cd "$REPO_DIR"

# Parse the edit blocks and apply
python3 << 'PYEOF'
import re, sys, json

raw = json.loads(sys.argv[1]) if len(sys.argv) > 1 else ""
# Parse from response
response = json.loads(open("/dev/stdin").read())
raw = response.get("raw_response", "")

# Find <<<EDIT ... >>> blocks
pattern = r'<<<EDIT\s+(\S+)\s*\n---\s*OLD\s*\n(.*?)\n---\s*NEW\s*\n(.*?)\n>>>'
matches = re.findall(pattern, raw, re.DOTALL)

for path, old, new in matches:
    try:
        with open(path, 'r') as f:
            content = f.read()
        if old.strip() in content:
            content = content.replace(old.strip(), new.strip(), 1)
            with open(path, 'w') as f:
                f.write(content)
            print(f"   Applied edit to {path}")
        else:
            print(f"   WARNING: OLD block not found in {path}, trying fuzzy match...")
            # Try line-by-line trimmed match
            old_lines = [l.strip() for l in old.strip().split('\n')]
            content_lines = content.split('\n')
            for i in range(len(content_lines) - len(old_lines) + 1):
                window = [l.strip() for l in content_lines[i:i+len(old_lines)]]
                if window == old_lines:
                    new_lines = new.strip().split('\n')
                    content_lines[i:i+len(old_lines)] = new_lines
                    with open(path, 'w') as f:
                        f.write('\n'.join(content_lines))
                    print(f"   Applied edit to {path} (fuzzy match at line {i+1})")
                    break
            else:
                print(f"   FAILED: Could not match OLD block in {path}")
    except Exception as e:
        print(f"   ERROR: {e}")
PYEOF
echo "$RESPONSE" | python3 << 'PYEOF'
import re, sys, json

response = json.loads(sys.stdin.read())
raw = response.get("raw_response", "")

pattern = r'<<<EDIT\s+(\S+)\s*\n---\s*OLD\s*\n(.*?)\n---\s*NEW\s*\n(.*?)\n>>>'
matches = re.findall(pattern, raw, re.DOTALL)

for path, old, new in matches:
    try:
        with open(path, 'r') as f:
            content = f.read()
        if old.strip() in content:
            content = content.replace(old.strip(), new.strip(), 1)
            with open(path, 'w') as f:
                f.write(content)
            print(f"   Applied edit to {path}")
        else:
            # Fuzzy match
            old_lines = [l.strip() for l in old.strip().split('\n')]
            content_lines = content.split('\n')
            for i in range(len(content_lines) - len(old_lines) + 1):
                window = [l.strip() for l in content_lines[i:i+len(old_lines)]]
                if window == old_lines:
                    new_lines = new.strip().split('\n')
                    content_lines[i:i+len(old_lines)] = new_lines
                    with open(path, 'w') as f:
                        f.write('\n'.join(content_lines))
                    print(f"   Applied (fuzzy) to {path}")
                    break
            else:
                print(f"   SKIP: no match in {path}")
    except Exception as e:
        print(f"   ERROR: {e}")
PYEOF

# 5. Format
echo "5. Running cargo fmt..."
cargo fmt -- "$TARGET_FILE" 2>/dev/null || true

# 6. Check if there are actual changes
if git diff --quiet; then
  echo "ERROR: No changes detected after applying edits"
  exit 1
fi

echo ""
echo "6. Git diff:"
git diff --stat
echo ""

# 7. Create branch, commit, push
echo "7. Creating branch and committing..."
git checkout -b "$BRANCH"
git add "$TARGET_FILE"
git commit -m "docs: add doc comment to discover_source_files

Generated by alpha-swarm WASI agent-worker via:
- qwen2.5-coder:14b on Ollama (csatapaci)
- virt-git workspace with wasi:blobstore
- Edit parsed from <<<EDIT>>> protocol

Co-Authored-By: alpha-swarm <agent@alpha-swarm.local>"

git push origin "$BRANCH"

# 8. Create PR
echo ""
echo "8. Creating PR..."
PR_URL=$(gh pr create \
  --title "docs: add doc comment to discover_source_files" \
  --body "## Summary

Generated by **alpha-swarm WASI agent-worker** E2E pipeline:

1. WASI component (\`agent-worker.wasm\`, 432KB) received task via HTTP
2. Called Ollama (\`qwen2.5-coder:14b\`) via \`wasi:http/outgoing-handler\`
3. Edit parsed by \`edit-parser\` (WASI-portable)
4. Applied to \`VirtWorkspace\` (virt-git, in-memory content-addressed store)
5. Workspace backed by \`wasi:blobstore\` → NATS Object Store
6. Diff generated from content-addressed tree comparison
7. Applied to repo, committed, PR created

### Diff
\`\`\`diff
${DIFF}
\`\`\`

🤖 Generated with [alpha-swarm](https://github.com/alpha-swarm/alpha-swarm2)" \
  --base main \
  --head "$BRANCH" 2>&1)

echo "   PR created: $PR_URL"

# 9. Switch back to main
git checkout main

echo ""
echo "=== E2E COMPLETE ==="
echo "Branch: $BRANCH"
echo "PR: $PR_URL"
