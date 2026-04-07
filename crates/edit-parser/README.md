# Edit Parser for Rust Crate

This crate parses text and identifies code edits in a specific format. It is used to modify source files within the context of this platform. The supported edit formats are as follows:

## Modify an Existing File
To modify an existing file, use the following format:
```
<<<EDIT path/to/file.rs
--- OLD
the exact lines to replace  (include enough context to be unique)
--- NEW
the replacement lines