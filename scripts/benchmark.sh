#!/usr/bin/env bash
set -euo pipefail

# Checkpoint 5 Benchmark: Run 20 diverse tasks against a test repo.
# Tracks: model selected, pass/fail, quality gate, duration, retry count.
#
# Prerequisites:
#   - Ollama running (local or ALPHA_SWARM_OLLAMA_URL set)
#   - SurrealDB running at 127.0.0.1:8000
#   - cargo build completed
#
# Usage: ./scripts/benchmark.sh

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
OLLAMA_URL="${ALPHA_SWARM_OLLAMA_URL:-http://100.81.10.8:11434}"
RESULTS_FILE="/tmp/alpha-swarm-benchmark.csv"
TEST_REPO="/tmp/alpha-swarm-bench-repo"

cd "$PROJECT_DIR"

echo "=== Alpha-Swarm Checkpoint 5 Benchmark ==="
echo "Ollama: $OLLAMA_URL"
echo "Results: $RESULTS_FILE"
echo ""

# Build CLI
cargo build -p alpha-swarm-cli --release 2>&1 | tail -1
CLI="./target/release/alpha-swarm"

# Create test repo
rm -rf "$TEST_REPO"
mkdir -p "$TEST_REPO/src"

cat > "$TEST_REPO/Cargo.toml" << 'EOF'
[package]
name = "bench-target"
version = "0.1.0"
edition = "2024"
EOF

cat > "$TEST_REPO/src/main.rs" << 'EOF'
mod math;
mod strings;
mod utils;

fn main() {
    println!("sum: {}", math::add(2, 3));
    println!("greeting: {}", strings::greet("world"));
    println!("is_even: {}", utils::is_even(4));
}
EOF

cat > "$TEST_REPO/src/math.rs" << 'EOF'
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

pub fn multiply(a: i32, b: i32) -> i32 {
    a * b
}
EOF

cat > "$TEST_REPO/src/strings.rs" << 'EOF'
pub fn greet(name: &str) -> String {
    format!("hello, {name}")
}

pub fn uppercase(s: &str) -> String {
    s.to_uppercase()
}
EOF

cat > "$TEST_REPO/src/utils.rs" << 'EOF'
pub fn is_even(n: i32) -> bool {
    n % 2 == 0
}

pub fn clamp(value: i32, min: i32, max: i32) -> i32 {
    if value < min { min } else if value > max { max } else { value }
}
EOF

cd "$TEST_REPO" && git init && git add . && git commit -m "init" -q && cd "$PROJECT_DIR"

# CSV header
echo "task_num,complexity,description,model,edits,quality_pass,duration_ms,status" > "$RESULTS_FILE"

# Define 20 tasks
TASKS=(
    "simple|Add a subtract function to math.rs|src/math.rs"
    "simple|Add a divide function to math.rs that returns Option<i32> (None for division by zero)|src/math.rs"
    "simple|Add a farewell function to strings.rs|src/strings.rs"
    "simple|Add a reverse function to strings.rs|src/strings.rs"
    "simple|Add an is_odd function to utils.rs|src/utils.rs"
    "simple|Add an abs function to utils.rs that returns the absolute value|src/utils.rs"
    "simple|Add a max function to math.rs that returns the larger of two i32|src/math.rs"
    "simple|Add a contains_digit function to strings.rs that checks if a string has any digit|src/strings.rs"
    "simple|Add a factorial function to math.rs (iterative, not recursive)|src/math.rs"
    "simple|Add a truncate function to strings.rs that limits string to N chars|src/strings.rs"
    "medium|Add a power function to math.rs that computes base^exp using a loop|src/math.rs"
    "medium|Add a caesar_cipher function to strings.rs that shifts each letter by N|src/strings.rs"
    "medium|Add a fibonacci function to math.rs that returns the Nth fibonacci number|src/math.rs"
    "medium|Add a slugify function to strings.rs that converts to lowercase and replaces spaces with hyphens|src/strings.rs"
    "medium|Add a median function to utils.rs that takes a Vec<i32> and returns the median value|src/utils.rs"
    "medium|Add a is_palindrome function to strings.rs|src/strings.rs"
    "complex|Add a simple expression evaluator to utils.rs that parses and evaluates strings like '2+3' or '10-4'|src/utils.rs"
    "complex|Add a run_length_encode function to strings.rs (e.g. 'aaabbc' -> '3a2b1c')|src/strings.rs"
    "complex|Add a matrix_multiply function to math.rs that multiplies two 2x2 matrices represented as [[i32;2];2]|src/math.rs"
    "complex|Add a merge_sort function to utils.rs that sorts a Vec<i32>|src/utils.rs"
)

PASSED=0
FAILED=0
TOTAL=0

echo "Running ${#TASKS[@]} tasks..."
echo ""

for i in "${!TASKS[@]}"; do
    TASK_NUM=$((i + 1))
    IFS='|' read -r COMPLEXITY DESC FILES <<< "${TASKS[$i]}"

    # Reset repo to clean state
    cd "$TEST_REPO" && git checkout . && git clean -fd -q && cd "$PROJECT_DIR"

    echo -n "[$TASK_NUM/20] ($COMPLEXITY) $DESC ... "

    START_TIME=$(python3 -c 'import time; print(int(time.time()*1000))')

    # Run agent
    OUTPUT=$(ALPHA_SWARM_OLLAMA_URL="$OLLAMA_URL" "$CLI" run \
        --repo "$TEST_REPO" \
        --task "$DESC" \
        --files "$FILES" \
        --complexity "$COMPLEXITY" \
        --no-quality-gate \
        2>&1) || true

    END_TIME=$(python3 -c 'import time; print(int(time.time()*1000))')
    DURATION=$((END_TIME - START_TIME))

    # Parse output
    MODEL=$(echo "$OUTPUT" | grep "^Model:" | sed 's/Model: *\([^ ]*\).*/\1/' || echo "unknown")
    EDITS=$(echo "$OUTPUT" | grep "^Edits:" | sed 's/Edits: *//' || echo "0")
    APPLIED=$(echo "$OUTPUT" | grep "^Applied:" | sed 's/Applied: *//' || echo "false")

    if [ "$APPLIED" = "true" ] && [ "$EDITS" != "0" ]; then
        STATUS="pass"
        PASSED=$((PASSED + 1))
        echo "PASS (${DURATION}ms, ${EDITS} edits, $MODEL)"
    else
        STATUS="fail"
        FAILED=$((FAILED + 1))
        echo "FAIL (${DURATION}ms, $MODEL)"
    fi
    TOTAL=$((TOTAL + 1))

    echo "$TASK_NUM,$COMPLEXITY,$DESC,$MODEL,$EDITS,,$DURATION,$STATUS" >> "$RESULTS_FILE"
done

echo ""
echo "=== Benchmark Results ==="
echo "Total:   $TOTAL"
echo "Passed:  $PASSED ($((PASSED * 100 / TOTAL))%)"
echo "Failed:  $FAILED"
echo ""

# Per-complexity breakdown
for C in simple medium complex; do
    C_TOTAL=$(grep ",$C," "$RESULTS_FILE" | wc -l | tr -d ' ')
    C_PASS=$(grep ",$C,.*,pass$" "$RESULTS_FILE" | wc -l | tr -d ' ')
    if [ "$C_TOTAL" -gt 0 ]; then
        echo "  $C: $C_PASS/$C_TOTAL ($((C_PASS * 100 / C_TOTAL))%)"
    fi
done

echo ""
echo "Results saved to: $RESULTS_FILE"

# Checkpoint 5 criteria
echo ""
echo "=== Checkpoint 5 Criteria ==="
PASS_RATE=$((PASSED * 100 / TOTAL))
echo "  Pass rate >= 70%:  $PASS_RATE% $([ $PASS_RATE -ge 70 ] && echo 'PASS' || echo 'FAIL')"

AVG_DURATION=$(($(awk -F, 'NR>1{sum+=$7}END{print sum}' "$RESULTS_FILE") / TOTAL))
echo "  Avg duration < 5min: ${AVG_DURATION}ms $([ $AVG_DURATION -lt 300000 ] && echo 'PASS' || echo 'FAIL')"
