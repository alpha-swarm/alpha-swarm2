```rust
/// # safe function
///
/// This function performs a safe operation.

struct Helper {
    /// A helper field used for synchronization.
    helper: String,
}

impl Helper {
    fn new(helper: String) -> Self {
        Helper { helper }
    }
}
```