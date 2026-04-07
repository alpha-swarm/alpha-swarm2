# Prompt Evaluation — Iteration 1

**Prompt variant**: v1_original
**Date**: 2026-04-07T11:44:07.367326+00:00
**Parse rate**: 15/16 (94%)
**Correct action rate**: 15/16 (94%)

## System Prompt

```
You are a code modification agent. You receive a task description and file contents, then output precise file edits.

RULES:
- Only modify files that need to change
- Do not add unnecessary changes, comments, or formatting
- If the task is unclear or impossible, explain why instead of guessing

OUTPUT FORMAT:
For each file you want to modify, output a block like this:

<<<EDIT path/to/file.rs
--- OLD
the exact lines to replace (include enough context to be unique)
--- NEW
the replacement lines
>>>

For new files:

<<<CREATE path/to/new_file.rs
file contents here
>>>

For deleted files:

<<<DELETE path/to/file.rs
>>>

Output ONLY edit blocks. No explanation before or after unless the task cannot be done.
```

## Results

| Test | Model | Parsed | Correct | Edits | Tokens | Time |
|------|-------|--------|---------|-------|--------|------|
| create_simple_file | deepseek-coder:33b | pass | pass | 1 | 16 | 8025ms |
| create_readme | deepseek-coder:33b | pass | pass | 1 | 64 | 10571ms |
| edit_function | deepseek-coder:33b | pass | pass | 1 | 64 | 11052ms |
| edit_add_error_handling | deepseek-coder:33b | pass | pass | 1 | 96 | 16156ms |
| create_test_file | deepseek-coder:33b | pass | pass | 1 | 143 | 23742ms |
| delete_file | deepseek-coder:33b | pass | pass | 1 | 12 | 2588ms |
| create_with_context | deepseek-coder:33b | pass | pass | 1 | 171 | 30668ms |
| edit_rename | deepseek-coder:33b | pass | pass | 1 | 89 | 18036ms |
| create_simple_file | qwen2.5-coder:7b | FAIL | FAIL | 0 | 15 | 1804ms |
| create_readme | qwen2.5-coder:7b | pass | pass | 1 | 50 | 3085ms |
| edit_function | qwen2.5-coder:7b | pass | pass | 1 | 56 | 2881ms |
| edit_add_error_handling | qwen2.5-coder:7b | pass | pass | 1 | 62 | 3237ms |
| create_test_file | qwen2.5-coder:7b | pass | pass | 1 | 79 | 4133ms |
| delete_file | qwen2.5-coder:7b | pass | pass | 1 | 9 | 613ms |
| create_with_context | qwen2.5-coder:7b | pass | pass | 1 | 88 | 4457ms |
| edit_rename | qwen2.5-coder:7b | pass | pass | 1 | 104 | 5386ms |

## Failed Responses (no edit blocks parsed)

### create_simple_file (qwen2.5-coder:7b)
```
```markdown
CREATE path/to/hello.md
Hello World
>>>
```

