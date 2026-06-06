# Add public function is_doc_comment and unit test in code_utils.rs.

- model: qwen2.5-coder:14b
- files: crates/agent-core/src/code_utils.rs
- when: 2026-06-05T14:17:33.928558+00:00

## Agent output

```
<<<EDIT crates/agent-core/src/code_utils.rs
--- OLD
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_replace_exact() {
        let got = fuzzy_replace("let a = 1;\nlet b = 2;\n", "let a = 1;", "let a = 10;");
        assert_eq!(got.as_deref(), Some("let a = 10;\nlet b = 2;\n"));
    }

    #[test]
    fn fuzzy_replace_tolerates_indentation_drift() {
        // OLD has different leading whitespace than the file — exact replacen
        // would miss; the trimmed line search must still match.
        let content = "fn main() {\n    let x = 1;\n}\n";
        let got = fuzzy_replace(content, "let x = 1;", "let x = 2;");
        assert!(got.is_some());
        assert!(got.unwrap().contains("let x = 2;"));
    }

    #[test]
    fn fuzzy_replace_multiline() {
        let content = "a\n  foo();\n  bar();\nz\n";
        let got = fuzzy_replace(content, "foo();\nbar();", "baz();").unwrap();
        assert!(got.contains("baz();"));
        assert!(!got.contains("foo();"));
    }

    #[test]
    fn fuzzy_replace_miss_returns_none() {
        // The critical contract: a non-matching OLD is None, NOT a silent no-op.
        assert_eq!(fuzzy_replace("let a = 1;\n", "nonexistent line", "x"), None);
        assert_eq!(fuzzy_replace("anything", "   ", "x"), None);
    }
}
--- NEW
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_replace_exact() {
        let got = fuzzy_replace("let a = 1;\nlet b = 2;\n", "let a = 1;", "let a = 10;");
        assert_eq!(got.as_deref(), Some("let a = 10;\nlet b = 2;\n"));
    }

    #[test]
    fn fuzzy_replace_tolerates_indentation_drift() {
        // OLD has different leading whitespace than the file — exact replacen
        // would miss; the trimmed line search must still match.
        let content = "fn main() {\n    let x = 1;\n}\n";
        let got = fuzzy_replace(content, "let x = 1;", "let x = 2;");
        assert!(got.is_some());
        assert!(got.unwrap().contains("let x = 2;"));
    }

    #[test]
    fn fuzzy_replace_multiline() {
        let content = "a\n  foo();\n  bar();\nz\n";
        let got = fuzzy_replace(content, "foo();\nbar();", "baz();").unwrap();
        assert!(got.contains("baz();"));
        assert!(!got.contains("foo();"));
    }

    #[test]
    fn fuzzy_replace_miss_returns_none() {
        // The critical contract: a non-matching OLD is None, NOT a silent no-op.
        assert_eq!(fuzzy_replace("let a = 1;\n", "nonexistent line", "x"), None);
        assert_eq!(fuzzy_replace("anything", "   ", "x"), None);
    }

    #[test]
    fn test_is_doc_comment() {
        assert!(is_doc_comment("/// This is a doc comment"));
        assert!(is_doc_comment("//! Another doc comment"));
        assert!(!is_doc_comment("// A regular comment"));
        assert!(!is_doc_comment("fn main()"));
    }
}
>>>
```

## Diff

```diff
<<<EDIT crates/agent-core/src/code_utils.rs
--- OLD
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_replace_exact() {
        let got = fuzzy_replace("let a = 1;\nlet b = 2;\n", "let a = 1;", "let a = 10;");
        assert_eq!(got.as_deref(), Some("let a = 10;\nlet b = 2;\n"));
    }

    #[test]
    fn fuzzy_replace_tolerates_indentation_drift() {
        // OLD has different leading whitespace than the file — exact replacen
        // would miss; the trimmed line search must still match.
        let content = "fn main() {\n    let x = 1;\n}\n";
        let got = fuzzy_replace(content, "let x = 1;", "let x = 2;");
        assert!(got.is_some());
        assert!(got.unwrap().contains("let x = 2;"));
    }

    #[test]
    fn fuzzy_replace_multiline() {
        let content = "a\n  foo();\n  bar();\nz\n";
        let got = fuzzy_replace(content, "foo();\nbar();", "baz();").unwrap();
        assert!(got.contains("baz();"));
        assert!(!got.contains("foo();"));
    }

    #[test]
    fn fuzzy_replace_miss_returns_none() {
        // The critical contract: a non-matching OLD is None, NOT a silent no-op.
        assert_eq!(fuzzy_replace("let a = 1;\n", "nonexistent line", "x"), None);
        assert_eq!(fuzzy_replace("anything", "   ", "x"), None);
    }
}
--- NEW
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_replace_exact() {
        let got = fuzzy_replace("let a = 1;\nlet b = 2;\n", "let a = 1;", "let a = 10;");
        assert_eq!(got.as_deref(), Some("let a = 10;\nlet b = 2;\n"));
    }

    #[test]
    fn fuzzy_replace_tolerates_indentation_drift() {
        // OLD has different leading whitespace than the file — exact replacen
        // would miss; the trimmed line search must still match.
        let content = "fn main() {\n    let x = 1;\n}\n";
        let got = fuzzy_replace(content, "let x = 1;", "let x = 2;");
        assert!(got.is_some());
        assert!(got.unwrap().contains("let x = 2;"));
    }

    #[test]
    fn fuzzy_replace_multiline() {
        let content = "a\n  foo();\n  bar();\nz\n";
        let got = fuzzy_replace(content, "foo();\nbar();", "baz();").unwrap();
        assert!(got.contains("baz();"));
        assert!(!got.contains("foo();"));
    }

    #[test]
    fn fuzzy_replace_miss_returns_none() {
        // The critical contract: a non-matching OLD is None, NOT a silent no-op.
        assert_eq!(fuzzy_replace("let a = 1;\n", "nonexistent line", "x"), None);
        assert_eq!(fuzzy_replace("anything", "   ", "x"), None);
    }

    #[test]
    fn test_is_doc_comment() {
        assert!(is_doc_comment("/// This is a doc comment"));
        assert!(is_doc_comment("//! Another doc comment"));
        assert!(!is_doc_comment("// A regular comment"));
        assert!(!is_doc_comment("fn main()"));
    }
}
>>>
```
