/// Greets a person by name.
///
/// # Arguments
///
/// * `name` - A string slice that holds the name of the person to greet.
///
/// # Returns
///
/// A `String` that contains the greeting message.
pub fn greet(name: &str) -> String {
    format!("Hello, {}!", name)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_greet() {
        assert_eq!(greet("world"), "Hello, world!");
    }
}